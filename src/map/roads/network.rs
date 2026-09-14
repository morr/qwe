//! Дорожная сеть глазами отрисовки: общие узлы ways и **стежки** — отрезки,
//! которыми висячий торец дороги дотягивается до дороги, до которой OSM его
//! не довёл.
//!
//! Узлы восстанавливаются так же, как в [`super::junctions`]: общая нода двух
//! ways проецируется в одну точку, и совпадения координат хватает. Общий узел
//! нужен сглаживанию — Chaikin его не срезает (`super::chaikin`): на нём
//! кончается поперечная улица, и сдвинутая хордой сквозная дорога оставляла
//! бы торец поперечной висеть в метре от асфальта или торчать за него.
//!
//! **Стежок** — ответ на кривую разметку OSM. Проезд, нарисованный «до
//! тротуара», кончается в паре метров от улицы, в которую он на месте въезжает;
//! два куска одного проезда, разрезанные картографом, расходятся на метр вместо
//! общей ноды. На снимке там сплошной асфальт, у нас — полоса земли поперёк
//! дороги. Стежок — прямой отрезок от висячего торца до осевой ближайшей дороги
//! **впереди** этого торца, не дальше [`STITCH_MAX_GAP`] от её края.
//!
//! Только картинка: навмеш, двери, деревья и машины по-прежнему видят данные
//! OSM как есть (`RoadLine::points` не трогается, как и при сглаживании).

use std::collections::HashMap;

use bevy::prelude::*;

use super::junctions::node_key;
use crate::map::osm::model::{closest_on_segment, point_in_area, polyline_length, put_in_cells};
use crate::map::osm::{MapData, PolyArea, RoadClass, RoadLine};

/// Зазор между висячим торцом и **краем** дороги впереди, м, который стежок
/// ещё закрывает. Шесть метров — это тротуар с газоном между проездом и
/// улицей; дальше разрыв уже похож на задуманный тупик, а не на небрежность.
pub const STITCH_MAX_GAP: f32 = 6.0;
/// Косинус наибольшего угла между направлением торца и стежком: дорога сбоку
/// (параллельный проезд в трёх метрах) не цель — стежок вбок читается как
/// перемычка, которой на месте нет.
const STITCH_MIN_COS: f32 = 0.5;
/// Шаг проверки стежка на здания и воду, м.
const STITCH_PROBE_STEP: f32 = 1.0;
/// Сетка отрезков и препятствий, м.
const CELL: f32 = 32.0;
/// Торец меряет направление по звену не короче этого, м: слипшиеся точки OSM
/// дают случайное направление.
const MIN_TAIL: f32 = 0.5;

/// Общие узлы дорог: через какие узлы прошли две дороги и больше и какие
/// именно. Ключ — квантованная точка.
///
/// Хранятся только общие узлы — их на город единицы тысяч против сотни тысяч
/// вершин, и собираются они сортировкой, а не картой по каждой вершине: карта
/// с вектором на вершину стоила десяток миллисекунд на каждую пересборку
/// дорог.
pub struct RoadNodes {
    shared: HashMap<(i32, i32), Vec<usize>>,
}

impl RoadNodes {
    pub fn new(roads: &[RoadLine]) -> Self {
        let mut visits: Vec<((i32, i32), usize)> = roads
            .iter()
            .enumerate()
            .filter(|(_, road)| road.points.len() >= 2)
            .flat_map(|(index, road)| {
                road.points
                    .iter()
                    .map(move |&point| (node_key(point), index))
            })
            .collect();
        visits.sort_unstable();
        visits.dedup();
        let mut shared = HashMap::new();
        for run in visits.chunk_by(|a, b| a.0 == b.0) {
            if run.len() >= 2 {
                shared.insert(run[0].0, run.iter().map(|visit| visit.1).collect());
            }
        }
        Self { shared }
    }

    /// Через узел проходят две дороги и больше.
    pub fn is_shared(&self, point: Vec2) -> bool {
        self.shared.contains_key(&node_key(point))
    }

    /// Дороги общего узла по возрастанию индекса; у необщего — пусто.
    pub fn roads_at(&self, point: Vec2) -> &[usize] {
        self.shared.get(&node_key(point)).map_or(&[], Vec::as_slice)
    }
}

/// Стежки по дорогам: `ends[i]` — точка, которую надо добавить перед началом и
/// после конца нарисованной осевой `roads[i]`.
pub struct Stitches {
    pub ends: Vec<[Option<Vec2>; 2]>,
    pub count: usize,
}

impl Stitches {
    /// Осевая с пришитыми стежками.
    pub fn apply(&self, road: usize, path: &mut Vec<Vec2>) {
        let [start, end] = self.ends[road];
        if let Some(start) = start {
            path.insert(0, start);
        }
        if let Some(end) = end {
            path.push(end);
        }
    }

    pub fn touches(&self, road: usize) -> bool {
        self.ends[road].iter().any(Option::is_some)
    }
}

/// Может ли дорога участвовать в стежке — с любой стороны. Мост кончается
/// ровным срезом бордюра, арка приколота к стенам дома: их торцы на месте.
fn stitchable(road: &RoadLine) -> bool {
    !road.bridge && !road.passage && road.points.len() >= 2
}

/// Продолжает ли `target` полотно дороги `own`. Улица — только улица: торец
/// проезда на пешеходной дорожке всё равно висит, песочная лента лежит под
/// асфальтом и асфальта не несёт (так и размечают: проезд «до тротуара», общая
/// нода с дорожкой вдоль проспекта). Дорожке годится любая дорога.
fn carries(own: &RoadLine, target: &RoadLine) -> bool {
    own.class == RoadClass::Alley || target.class == RoadClass::Street
}

/// Длина, до которой дорожка между торцами двух проездов — переезд через
/// тротуар, а не дорожка, м ([`driveway_crossings`]).
const CROSSING_MAX_LENGTH: f32 = 20.0;

/// Переезды через тротуар: короткие дорожки (`Alley`), оба торца которых —
/// **торцы** проездов или улиц. Так OSM размечает проезд, пересекающий
/// тротуар: кусок проезда, кусок `footway` поперёк тротуара, снова проезд, и
/// песочная лента под асфальтом рисовала полосу земли поперёк въезда (Тула,
/// ways 4175 → 4176 → 80 у проспекта Ленина). На месте там асфальт проезда.
///
/// Пешеходный переход через улицу сюда не попадает: его торцы — на
/// тротуарах-дорожках, а проезжую часть он пересекает серединой.
///
/// Возвращает индекс дорожки и ширину, с которой её рисовать, — у́жего из
/// двух проездов.
pub fn driveway_crossings(roads: &[RoadLine], nodes: &RoadNodes) -> Vec<(usize, f32)> {
    let street_end_at = |node: Vec2, own: usize| {
        nodes
            .roads_at(node)
            .iter()
            .filter(|&&other| other != own)
            .filter_map(|&other| {
                let road = &roads[other];
                let (first, last) = (road.points[0], road.points[road.points.len() - 1]);
                (road.class == RoadClass::Street
                    && !road.bridge
                    && first != last
                    && (node_key(first) == node_key(node) || node_key(last) == node_key(node)))
                .then_some(road.width)
            })
            .reduce(f32::min)
    };
    roads
        .iter()
        .enumerate()
        .filter(|(_, road)| {
            road.class == RoadClass::Alley && stitchable(road) && !road.points.is_empty()
        })
        .filter_map(|(index, road)| {
            let (first, last) = (road.points[0], road.points[road.points.len() - 1]);
            if first == last || polyline_length(&road.points) > CROSSING_MAX_LENGTH {
                return None;
            }
            let width = street_end_at(first, index)?.min(street_end_at(last, index)?);
            Some((index, width))
        })
        .collect()
}

/// Стежки всех висячих торцов карты. `roads` — дороги **как рисуются**
/// (переезды уже асфальтом), `map` — ради зданий и воды.
pub fn stitches(roads: &[&RoadLine], map: &MapData, nodes: &RoadNodes) -> Stitches {
    let mut ends = vec![[None; 2]; roads.len()];
    let mut segments: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    let mut widest = 0.0_f32;
    for (index, road) in roads.iter().enumerate() {
        if !stitchable(road) {
            continue;
        }
        let half = road.width / 2.0;
        widest = widest.max(half);
        for (segment, pair) in road.points.windows(2).enumerate() {
            let (min, max) = (pair[0].min(pair[1]), pair[0].max(pair[1]));
            put_in_cells(
                &mut segments,
                min - half,
                max + half,
                CELL,
                (index, segment),
            );
        }
    }
    let obstacles = Obstacles::new(map);

    let mut count = 0;
    for (index, road) in roads.iter().enumerate() {
        if !stitchable(road) {
            continue;
        }
        let points = &road.points;
        if points[0] == points[points.len() - 1] {
            continue;
        }
        for (side, (end, tail)) in [
            (points[0], points[1..].iter()),
            (points[points.len() - 1], points[..points.len() - 1].iter()),
        ]
        .into_iter()
        .enumerate()
        {
            // торец держится, если в узле есть другая дорога, годная в цель
            let held = nodes
                .roads_at(end)
                .iter()
                .any(|&other| other != index && carries(road, roads[other]));
            if held {
                continue;
            }
            let tail: Vec<Vec2> = if side == 0 {
                tail.copied().collect()
            } else {
                tail.rev().copied().collect()
            };
            let Some(from) = tail.iter().find(|point| point.distance(end) >= MIN_TAIL) else {
                continue;
            };
            let heading = (end - *from).normalize();
            let reach = road.width / 2.0 + STITCH_MAX_GAP + widest;
            let stitch = stitch_end(roads, index, end, heading, reach, &segments, &obstacles);
            if let Some(point) = stitch {
                ends[index][side] = Some(point);
                count += 1;
            }
        }
    }
    Stitches { ends, count }
}

/// Куда дотянуть торец `end` дороги `own`, смотрящий по `heading`. `None` —
/// впереди ничего нет, торец уже лежит на чужой ленте или стежок прошёл бы
/// сквозь дом или воду.
fn stitch_end(
    roads: &[&RoadLine],
    own: usize,
    end: Vec2,
    heading: Vec2,
    reach: f32,
    segments: &HashMap<(i32, i32), Vec<(usize, usize)>>,
    obstacles: &Obstacles,
) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2, f32)> = None;
    let (low, high) = ((end - reach) / CELL, (end + reach) / CELL);
    for x in low.x.floor() as i32..=high.x.floor() as i32 {
        for y in low.y.floor() as i32..=high.y.floor() as i32 {
            let Some(found) = segments.get(&(x, y)) else {
                continue;
            };
            for &(road, segment) in found {
                let target = roads[road];
                if road == own || !carries(roads[own], target) {
                    continue;
                }
                let half = target.width / 2.0;
                let (a, b) = (target.points[segment], target.points[segment + 1]);
                let nearest = closest_on_segment(end, a, b);
                let distance = nearest.distance(end);
                // торец уже на чужой ленте — зазора нет
                if distance <= half {
                    return None;
                }
                let mut consider = |gap: f32, point: Vec2| {
                    if gap <= STITCH_MAX_GAP && best.is_none_or(|(known, ..)| gap < known) {
                        best = Some((gap, point, half));
                    }
                };
                if (nearest - end).dot(heading) >= STITCH_MIN_COS * distance {
                    consider(distance - half, nearest);
                }
                // луч вперёд из торца: прямое продолжение дороги до осевой
                let along = b - a;
                let denominator = heading.perp_dot(along);
                if denominator.abs() > 1e-6 {
                    let offset = a - end;
                    let ahead = offset.perp_dot(along) / denominator;
                    let at = offset.perp_dot(heading) / denominator;
                    if ahead > 0.0 && (0.0..=1.0).contains(&at) {
                        consider(ahead - half, end + heading * ahead);
                    }
                }
            }
        }
    }
    let (_, point, target_half) = best?;
    let direction = (point - end).normalize_or_zero();
    // Своя лента шире цели — торец отступает, чтобы полудиск не вылез за
    // дальний край цели.
    let pull = (roads[own].width / 2.0 - target_half).max(0.0);
    let stitched = point - direction * pull;
    let length = (stitched - end).dot(direction);
    if length <= 0.1 {
        return None;
    }
    let steps = (length / STITCH_PROBE_STEP).ceil() as usize;
    let blocked = (1..=steps)
        .any(|step| obstacles.covers(end + direction * (length * step as f32 / steps as f32)));
    (!blocked).then_some(stitched)
}

/// Здания и вода — через них стежок не идёт: проезд, упёртый в стену гаража,
/// кончается там по-настоящему.
struct Obstacles<'a> {
    areas: Vec<&'a PolyArea>,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl<'a> Obstacles<'a> {
    fn new(map: &'a MapData) -> Self {
        let areas: Vec<&PolyArea> = map.buildings.iter().chain(&map.water).collect();
        let mut cells = HashMap::new();
        for (index, area) in areas.iter().enumerate() {
            let (min, max) = area.outer.iter().fold(
                (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                |(min, max), point| (min.min(*point), max.max(*point)),
            );
            put_in_cells(&mut cells, min, max, CELL, index);
        }
        Self { areas, cells }
    }

    fn covers(&self, point: Vec2) -> bool {
        let cell = (
            (point.x / CELL).floor() as i32,
            (point.y / CELL).floor() as i32,
        );
        self.cells.get(&cell).is_some_and(|found| {
            found
                .iter()
                .any(|&index| point_in_area(point, self.areas[index]))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{building, street};

    fn stitched(roads: Vec<RoadLine>) -> Stitches {
        stitched_map(MapData { roads, ..default() })
    }

    fn stitched_map(map: MapData) -> Stitches {
        let drawn: Vec<&RoadLine> = map.roads.iter().collect();
        stitches(&drawn, &map, &RoadNodes::new(&map.roads))
    }

    fn footway(points: Vec<Vec2>) -> RoadLine {
        RoadLine {
            class: RoadClass::Alley,
            ..street(points, 3.5)
        }
    }

    #[test]
    fn a_footway_between_two_drive_ends_is_a_driveway_crossing() {
        // проезд, кусок `footway` поперёк тротуара, снова проезд
        let (near, far) = (Vec2::new(0.0, 10.0), Vec2::new(0.0, 20.0));
        let roads = vec![
            street(vec![Vec2::ZERO, near], 5.0),
            footway(vec![near, far]),
            street(vec![far, Vec2::new(0.0, 60.0)], 6.0),
            // тротуар вдоль улицы, через который идёт переезд
            footway(vec![
                Vec2::new(-30.0, 15.0),
                Vec2::new(0.0, 15.0),
                Vec2::new(30.0, 15.0),
            ]),
        ];
        let nodes = RoadNodes::new(&roads);
        assert_eq!(driveway_crossings(&roads, &nodes), vec![(1, 5.0)]);
    }

    #[test]
    fn a_crosswalk_between_two_pavements_stays_a_footway() {
        // переход: торцы на тротуарах-дорожках, улицу пересекает серединой
        let (south, middle, north) = (Vec2::new(0.0, -6.0), Vec2::ZERO, Vec2::new(0.0, 6.0));
        let roads = vec![
            street(
                vec![Vec2::new(-50.0, 0.0), middle, Vec2::new(50.0, 0.0)],
                8.0,
            ),
            footway(vec![south, middle, north]),
            footway(vec![Vec2::new(-50.0, -6.0), south, Vec2::new(50.0, -6.0)]),
            footway(vec![Vec2::new(-50.0, 6.0), north, Vec2::new(50.0, 6.0)]),
        ];
        let nodes = RoadNodes::new(&roads);
        assert!(driveway_crossings(&roads, &nodes).is_empty());
    }

    #[test]
    fn a_drive_stopping_short_of_the_street_reaches_its_centreline() {
        // улица по y = 0 шириной 8 м; проезд идёт к ней сверху и кончается в
        // трёх метрах от её края
        let avenue = street(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        let drive = street(vec![Vec2::new(0.0, 40.0), Vec2::new(0.0, 7.0)], 5.0);
        let found = stitched(vec![avenue, drive]);
        assert_eq!(found.count, 1);
        assert_eq!(found.ends[1][0], None);
        let end = found.ends[1][1].expect("drive stitched");
        assert!(end.distance(Vec2::ZERO) < 1e-3, "{end:?}");
    }

    #[test]
    fn a_drive_already_on_the_street_is_left_alone() {
        let avenue = street(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        let drive = street(vec![Vec2::new(0.0, 40.0), Vec2::new(0.0, 3.0)], 5.0);
        assert_eq!(stitched(vec![avenue, drive]).count, 0);
    }

    #[test]
    fn a_shared_node_is_not_a_loose_end() {
        let avenue = street(
            vec![
                Vec2::new(-50.0, 0.0),
                Vec2::new(0.0, 0.0),
                Vec2::new(50.0, 0.0),
            ],
            8.0,
        );
        let drive = street(vec![Vec2::new(0.0, 40.0), Vec2::ZERO], 5.0);
        let found = stitched(vec![avenue, drive]);
        // торцы проспекта висят, но впереди у них пусто
        assert_eq!(found.count, 0);
    }

    #[test]
    fn a_drive_ending_on_a_footway_still_reaches_the_street() {
        // вдоль проспекта тротуар-дорожка; проезд кончается на ней, в её ноде
        let avenue = street(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        let on_footway = Vec2::new(0.0, 8.0);
        let pavement = footway(vec![
            Vec2::new(-50.0, 8.0),
            on_footway,
            Vec2::new(50.0, 8.0),
        ]);
        let drive = street(vec![Vec2::new(0.0, 40.0), on_footway], 5.0);
        let found = stitched(vec![avenue, pavement, drive]);
        let end = found.ends[2][1].expect("drive stitched past the footway");
        assert!(end.distance(Vec2::ZERO) < 1e-3, "{end:?}");
    }

    #[test]
    fn a_far_dead_end_stays_a_dead_end() {
        let avenue = street(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        let drive = street(
            vec![
                Vec2::new(0.0, 40.0),
                Vec2::new(0.0, 4.0 + STITCH_MAX_GAP + 2.0),
            ],
            5.0,
        );
        assert_eq!(stitched(vec![avenue, drive]).count, 0);
    }

    #[test]
    fn a_parallel_road_beside_the_end_is_not_a_target() {
        // проезд кончается рядом с параллельной улицей — перемычки вбок нет
        let beside = street(vec![Vec2::new(6.0, -50.0), Vec2::new(6.0, 50.0)], 5.0);
        let drive = street(vec![Vec2::new(0.0, -40.0), Vec2::new(0.0, 0.0)], 5.0);
        assert_eq!(stitched(vec![beside, drive]).count, 0);
    }

    #[test]
    fn two_pieces_of_one_drive_close_the_gap_between_them() {
        let first = street(vec![Vec2::new(-40.0, 0.0), Vec2::new(-1.5, 0.0)], 5.0);
        let second = street(vec![Vec2::new(1.5, 0.0), Vec2::new(40.0, 0.0)], 5.0);
        let found = stitched(vec![first, second]);
        assert!(found.ends[0][1].is_some() && found.ends[1][0].is_some());
    }

    #[test]
    fn a_stitch_does_not_cut_through_a_building() {
        let avenue = street(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 8.0);
        let drive = street(vec![Vec2::new(0.0, 40.0), Vec2::new(0.0, 8.0)], 5.0);
        let shed = building(
            vec![
                Vec2::new(-3.0, 5.0),
                Vec2::new(3.0, 5.0),
                Vec2::new(3.0, 7.0),
                Vec2::new(-3.0, 7.0),
            ],
            Vec::new(),
        );
        let map = MapData {
            roads: vec![avenue, drive],
            buildings: vec![shed],
            ..default()
        };
        assert_eq!(stitched_map(map).count, 0);
    }
}
