//! **Улица** — цепочка ways, склеенных через узлы-продолжения: OSM режет
//! одну улицу на куски при каждой смене тега (мост, число полос, покрытие,
//! название), а рисовать и выводить сечение надо по улице целиком.
//!
//! В узле продолжением считается **самая соосная пара торцов одного класса**
//! ([`Highway`]): торцы разных ways, сходящиеся в узле, разбираются парами по
//! возрастанию угла излома, и пара с изломом круче [`MAX_BEND`] не
//! склеивается. Так на Т-образном перекрёстке прямая улица остаётся одной, а
//! примыкающая — своей; на крестовине из четырёх торцов одного класса
//! склеиваются обе прямые.
//!
//! Склеиваются только торцы: way, проходящий узел насквозь, в этом узле уже
//! непрерывен сам. Кольцо (тег или форма) и замкнутый way — улица сами по
//! себе, из одного way: у них нет торцов, которые было бы с чем сращивать, а
//! дуги кольца, склеенные с подходом, увели бы улицу по кругу.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::super::junctions::node_key;
use crate::map::osm::{Highway, RoadLine};
use crate::map::shapes::is_ring;

/// Наибольший излом в узле, при котором два торца ещё одна улица, рад. 50° —
/// плавный изгиб проспекта на площади ещё продолжение, поворот под прямым
/// углом — уже нет.
const MAX_BEND: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Направление торца меряется хордой на эту длину, м: первое звено OSM бывает
/// в полметра и смотрит куда угодно.
const ARM_REACH: f32 = 10.0;

/// Way в составе улицы: `reversed` — улица идёт против порядка его точек.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreetWay {
    pub road: usize,
    pub reversed: bool,
}

impl StreetWay {
    /// Точка, в которой way входит в улицу.
    pub fn entry(self, roads: &[RoadLine]) -> Vec2 {
        let points = &roads[self.road].points;
        if self.reversed {
            points[points.len() - 1]
        } else {
            points[0]
        }
    }

    /// Точка, в которой way из улицы выходит, — узел со следующим way.
    pub fn exit(self, roads: &[RoadLine]) -> Vec2 {
        let points = &roads[self.road].points;
        if self.reversed {
            points[0]
        } else {
            points[points.len() - 1]
        }
    }
}

/// Улица: ways по порядку, каждый следующий начинается там, где кончился
/// предыдущий.
#[derive(Debug, Clone, Default)]
pub struct Street {
    pub ways: Vec<StreetWay>,
    /// Цепочка вернулась в свой первый узел — улица-петля без торцов.
    pub closed: bool,
}

/// Дороги карты, склеенные в улицы. Каждая дорога, кроме дорожек, лежит ровно
/// в одной улице; дорожка ([`Highway::Path`]) — ни в одной.
#[derive(Debug, Clone, Default)]
pub struct RoadNetwork {
    pub streets: Vec<Street>,
    /// Дорога → (улица, место в ней).
    place: Vec<Option<(u32, u32)>>,
}

impl RoadNetwork {
    pub fn new(roads: &[RoadLine]) -> Self {
        let partners = pair_ends(roads);
        let mut place: Vec<Option<(u32, u32)>> = vec![None; roads.len()];
        let mut streets = Vec::new();
        for road in 0..roads.len() {
            if place[road].is_some() || !joins_streets(&roads[road]) {
                continue;
            }
            let street = walk(road, &partners);
            let index = streets.len() as u32;
            for (at, way) in street.ways.iter().enumerate() {
                place[way.road] = Some((index, at as u32));
            }
            streets.push(street);
        }
        Self { streets, place }
    }

    /// Собрана ли сеть по этому списку дорог. Карта, собранная тестом руками,
    /// сети не имеет вовсе, и спрашивать её про чужие индексы нельзя.
    pub fn covers(&self, roads: usize) -> bool {
        self.place.len() == roads && roads > 0
    }

    /// Улица дороги и место в ней.
    pub fn street_of(&self, road: usize) -> Option<(usize, usize)> {
        self.place
            .get(road)
            .copied()
            .flatten()
            .map(|(street, at)| (street as usize, at as usize))
    }

    /// Стыки внутри улиц: пары соседних ways и узел между ними. У петли есть
    /// и стык последнего way с первым.
    pub fn joints(&self) -> impl Iterator<Item = (StreetWay, StreetWay)> + '_ {
        self.streets.iter().flat_map(|street| {
            let wrap = (street.closed && street.ways.len() >= 2)
                .then(|| (street.ways[street.ways.len() - 1], street.ways[0]));
            street
                .ways
                .windows(2)
                .map(|pair| (pair[0], pair[1]))
                .chain(wrap)
        })
    }

    /// Улиц из двух ways и больше — то, что склейка вообще дала.
    pub fn glued(&self) -> usize {
        self.streets
            .iter()
            .filter(|street| street.ways.len() >= 2)
            .count()
    }
}

/// Торец way: `end` — последняя точка, иначе первая.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct End {
    road: usize,
    end: bool,
}

/// Дорога склеивается в улицы: у неё есть сечение (не дорожка) и торцы (не
/// кольцо и не замкнутый way).
fn joins_streets(road: &RoadLine) -> bool {
    road.highway != Highway::Path && road.points.len() >= 2
}

fn has_free_ends(road: &RoadLine) -> bool {
    joins_streets(road) && !road.is_roundabout() && !is_ring(&road.points)
}

/// Направление от торца внутрь дороги — хордой на [`ARM_REACH`].
fn arm_direction(points: &[Vec2], end: bool) -> Option<Vec2> {
    let mut walk: Box<dyn Iterator<Item = &Vec2>> = if end {
        Box::new(points.iter().rev())
    } else {
        Box::new(points.iter())
    };
    let origin = *walk.next()?;
    let mut far = origin;
    for &point in walk {
        far = point;
        if far.distance(origin) >= ARM_REACH {
            break;
        }
    }
    (far - origin).try_normalize()
}

/// Партнёр каждого склеенного торца.
fn pair_ends(roads: &[RoadLine]) -> HashMap<End, End> {
    let mut at_node: HashMap<(i32, i32), Vec<(End, Vec2)>> = HashMap::new();
    for (road, line) in roads.iter().enumerate() {
        if !has_free_ends(line) {
            continue;
        }
        for end in [false, true] {
            let Some(direction) = arm_direction(&line.points, end) else {
                continue;
            };
            let point = if end {
                line.points[line.points.len() - 1]
            } else {
                line.points[0]
            };
            at_node
                .entry(node_key(point))
                .or_default()
                .push((End { road, end }, direction));
        }
    }

    let min_dot = -MAX_BEND.cos();
    let mut partners = HashMap::new();
    // узлы по ключу — чтобы склейка не зависела от порядка обхода карты
    let mut nodes: Vec<_> = at_node
        .into_iter()
        .filter(|(_, arms)| arms.len() >= 2)
        .collect();
    nodes.sort_unstable_by_key(|(key, _)| *key);
    for (_, arms) in nodes {
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for i in 0..arms.len() {
            for j in i + 1..arms.len() {
                let ((a, dir_a), (b, dir_b)) = (arms[i], arms[j]);
                if a.road == b.road || !continues(&roads[a.road], a, &roads[b.road], b) {
                    continue;
                }
                let dot = dir_a.dot(dir_b);
                if dot <= min_dot {
                    pairs.push((dot, i, j));
                }
            }
        }
        pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
        let mut taken = vec![false; arms.len()];
        for (_, i, j) in pairs {
            if taken[i] || taken[j] {
                continue;
            }
            taken[i] = true;
            taken[j] = true;
            partners.insert(arms[i].0, arms[j].0);
            partners.insert(arms[j].0, arms[i].0);
        }
    }
    partners
}

/// Два торца в одном узле могут быть одной улицей: тот же класс, та же
/// односторонность, и у односторонних поток проходит узел насквозь — один
/// way в узел входит, другой из него выходит.
fn continues(a: &RoadLine, a_end: End, b: &RoadLine, b_end: End) -> bool {
    a.highway == b.highway && a.oneway == b.oneway && (!a.oneway || a_end.end != b_end.end)
}

/// Улица, в которой лежит `start`: сначала назад до её начала, потом вперёд.
fn walk(start: usize, partners: &HashMap<End, End>) -> Street {
    // way пройден так, что выходит в `exit`; предыдущий — партнёр его входа
    let entry_of = |way: StreetWay| End {
        road: way.road,
        end: way.reversed,
    };
    let exit_of = |way: StreetWay| End {
        road: way.road,
        end: !way.reversed,
    };
    let mut first = StreetWay {
        road: start,
        reversed: false,
    };
    let mut closed = false;
    while let Some(previous) = partners.get(&entry_of(first)) {
        // предыдущий way выходит в этот узел своим торцом `previous`
        let way = StreetWay {
            road: previous.road,
            reversed: !previous.end,
        };
        if way.road == start {
            closed = true;
            break;
        }
        first = way;
    }
    let mut ways = vec![first];
    let mut current = first;
    while let Some(next) = partners.get(&exit_of(current)) {
        let way = StreetWay {
            road: next.road,
            reversed: next.end,
        };
        if way.road == first.road {
            closed = true;
            break;
        }
        ways.push(way);
        current = way;
    }
    Street { ways, closed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;

    fn network(roads: &[RoadLine]) -> RoadNetwork {
        RoadNetwork::new(roads)
    }

    #[test]
    fn two_pieces_of_one_street_are_one_street() {
        let roads = [
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 8.0),
            street(vec![Vec2::new(100.0, 2.0), Vec2::new(50.0, 0.0)], 8.0),
        ];
        let network = network(&roads);
        assert_eq!(network.streets.len(), 1);
        let ways = &network.streets[0].ways;
        assert_eq!(ways.len(), 2);
        // второй way нарисован навстречу — улица проходит его задом наперёд
        assert_eq!(ways[0].exit(&roads), ways[1].entry(&roads));
        assert_eq!(network.joints().count(), 1);
    }

    #[test]
    fn a_side_street_stays_its_own_street() {
        let roads = [
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 8.0),
            street(vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 60.0)], 8.0),
        ];
        let network = network(&roads);
        assert_eq!(network.streets.len(), 2);
        assert_eq!(
            network.street_of(0).unwrap().0,
            network.street_of(1).unwrap().0
        );
        assert_ne!(
            network.street_of(0).unwrap().0,
            network.street_of(2).unwrap().0
        );
    }

    #[test]
    fn a_right_angle_is_not_a_continuation() {
        let roads = [
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 8.0),
            street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 60.0)], 8.0),
        ];
        assert_eq!(network(&roads).streets.len(), 2);
    }

    #[test]
    fn another_class_is_not_a_continuation() {
        let mut service = street(vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)], 5.0);
        service.highway = Highway::Service;
        let roads = [
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 8.0),
            service,
        ];
        assert_eq!(network(&roads).streets.len(), 2);
    }

    #[test]
    fn one_way_pieces_join_only_along_the_flow() {
        let mut along = street(vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        along.oneway = true;
        let mut onward = street(vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)], 8.0);
        onward.oneway = true;
        let mut against = street(vec![Vec2::new(100.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        against.oneway = true;
        assert_eq!(network(&[along.clone(), onward]).streets.len(), 1);
        assert_eq!(network(&[along, against]).streets.len(), 2);
    }

    #[test]
    fn a_crossroads_joins_both_straight_streets() {
        let o = Vec2::new(50.0, 50.0);
        let roads = [
            street(vec![Vec2::new(0.0, 50.0), o], 8.0),
            street(vec![o, Vec2::new(100.0, 50.0)], 8.0),
            street(vec![Vec2::new(50.0, 0.0), o], 8.0),
            street(vec![o, Vec2::new(50.0, 100.0)], 8.0),
        ];
        let network = network(&roads);
        assert_eq!(network.streets.len(), 2);
        assert_eq!(
            network.street_of(0).unwrap().0,
            network.street_of(1).unwrap().0
        );
        assert_eq!(
            network.street_of(2).unwrap().0,
            network.street_of(3).unwrap().0
        );
    }

    #[test]
    fn a_loop_of_two_ways_is_closed() {
        // квартал в обход, оба стыка — посреди прямых сторон
        let roads = [
            street(
                vec![
                    Vec2::new(50.0, 0.0),
                    Vec2::new(100.0, 0.0),
                    Vec2::new(100.0, 50.0),
                    Vec2::new(50.0, 50.0),
                ],
                8.0,
            ),
            street(
                vec![
                    Vec2::new(50.0, 50.0),
                    Vec2::new(0.0, 50.0),
                    Vec2::new(0.0, 0.0),
                    Vec2::new(50.0, 0.0),
                ],
                8.0,
            ),
        ];
        let network = network(&roads);
        assert_eq!(network.streets.len(), 1);
        assert!(network.streets[0].closed);
        assert_eq!(network.joints().count(), 2);
    }
}
