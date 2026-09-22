//! Перекрёстки для разметки: где улицы сходятся и где линии обязаны
//! прерваться.
//!
//! Геометрии стыков у нас нет — Overpass отдаёт `out geom`, и ленты дорог
//! рисуются независимыми, внахлёст. Но общая нода двух ways проецируется в
//! одну и ту же точку с точностью до бита, и по совпадению координат
//! перекрёстки восстанавливаются без единого пересечения отрезков: узел, в
//! котором сходятся две и больше улицы, — перекрёсток; узел, где один way
//! кончается и другой начинается, — стык одной дороги. Заодно это отделяет
//! мост от улицы под ним: общей ноды у них нет, и разметка на обоих идёт
//! насквозь — там, где поиск пересечений отрезков порвал бы обе.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::network::StitchTarget;
use crate::map::meshing::Break;
use crate::map::osm::RoadLine;

/// Шаг квантования координат узла, м. Одна нода OSM даёт одну и ту же точку
/// на всех своих ways, квантование лишь страхует от округления.
const NODE_GRID: f32 = 0.05;

/// Ключ узла — квантованная точка. Один на всех, кто восстанавливает узлы
/// по совпадению координат (`roads/network.rs`, `roads/corners.rs`).
pub(super) fn node_key(point: Vec2) -> (i32, i32) {
    (
        (point.x / NODE_GRID).round() as i32,
        (point.y / NODE_GRID).round() as i32,
    )
}

/// Запас разрыва за краем поперечной улицы, м: линия кончается чуть раньше,
/// чем начинается перекрёсток, как стоп-линия перед ним.
pub const JUNCTION_MARGIN: f32 = 1.0;

/// Чья дорога прошла через узел, какой её вершиной и торец ли это её.
/// `inner` — узел не на вершине, а внутри отрезка `vertex..vertex + 1`: так
/// в чужую ось упирается стежок ([`with_stitches`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Visit {
    pub road: usize,
    pub vertex: usize,
    pub end: bool,
    pub inner: bool,
}

/// Общий узел участвующих дорог и все их проходы через него.
pub(super) struct SharedNode {
    pub at: Vec2,
    pub visits: Vec<Visit>,
}

impl SharedNode {
    /// Перекрёсток: сошлись две дороги и больше, и это не шов — не два way
    /// торцами друг к другу (way разрезан по смене тега).
    pub fn is_junction(&self) -> bool {
        let mut distinct: Vec<usize> = self.visits.iter().map(|visit| visit.road).collect();
        distinct.sort_unstable();
        distinct.dedup();
        distinct.len() >= 2
            && !(distinct.len() == 2
                && self.visits.len() == 2
                && self.visits.iter().all(|visit| visit.end))
    }
}

/// Узлы участвующих дорог по совпадению координат, в порядке ключа — от
/// порядка обхода `HashMap` не зависит ничего, что из них строится.
pub(super) fn shared_nodes(
    roads: &[impl std::borrow::Borrow<RoadLine>],
    participates: impl Fn(&RoadLine) -> bool,
) -> Vec<SharedNode> {
    let mut nodes: HashMap<(i32, i32), SharedNode> = HashMap::new();
    for (index, road) in roads.iter().enumerate() {
        let road = road.borrow();
        if !participates(road) || road.points.len() < 2 {
            continue;
        }
        let last = road.points.len() - 1;
        let closed = road.points[0] == road.points[last];
        for (vertex, &point) in road.points.iter().enumerate() {
            let end = !closed && (vertex == 0 || vertex == last);
            nodes
                .entry(node_key(point))
                .or_insert_with(|| SharedNode {
                    at: point,
                    visits: Vec::new(),
                })
                .visits
                .push(Visit {
                    road: index,
                    vertex,
                    end,
                    inner: false,
                });
        }
    }
    let mut nodes: Vec<((i32, i32), SharedNode)> = nodes.into_iter().collect();
    nodes.sort_unstable_by_key(|(key, _)| *key);
    nodes.into_iter().map(|(_, node)| node).collect()
}

/// Узлы участвующих дорог вместе со **стежками** (`network::stitches`):
/// торец, дотянутый до чужой оси, — такое же примыкание, как общая нода, только
/// OSM её не провёл. Узел стежка стоит на оси цели, в точке `targets[i][side]`;
/// цель проходит его внутри отрезка (`Visit::inner`), а своей ноды у торца
/// больше нет — он не тупик. `targets` короче дорог — у остальных стежков нет.
pub(super) fn with_stitches(
    roads: &[impl std::borrow::Borrow<RoadLine>],
    participates: impl Fn(&RoadLine) -> bool,
    targets: &[[Option<StitchTarget>; 2]],
) -> Vec<SharedNode> {
    let mut nodes = shared_nodes(roads, &participates);
    let takes = |index: usize| {
        let road = roads[index].borrow();
        participates(road) && road.points.len() >= 2
    };
    let mut keys: HashMap<(i32, i32), usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node_key(node.at), index))
        .collect();
    // (дорога, вершина торца, точка торца)
    let mut stitched_ends: Vec<(usize, usize, Vec2)> = Vec::new();
    for (index, ends) in targets.iter().enumerate().take(roads.len()) {
        for (side, target) in ends.iter().enumerate() {
            let Some(target) = target else { continue };
            if !takes(index) || target.road >= roads.len() || !takes(target.road) {
                continue;
            }
            let own = &roads[index].borrow().points;
            let last = own.len() - 1;
            let vertex = if side == 0 { 0 } else { last };
            stitched_ends.push((index, vertex, own[vertex]));
            let points = &roads[target.road].borrow().points;
            let (a, b) = (target.segment, target.segment + 1);
            let tail = points.len() - 1;
            let closed = points[0] == points[tail];
            // точка стежка на вершине цели — обычный проход вершиной
            let onto = if node_key(target.at) == node_key(points[a]) {
                (a, false)
            } else if b < points.len() && node_key(target.at) == node_key(points[b]) {
                (b, false)
            } else {
                (a, true)
            };
            let slot = *keys.entry(node_key(target.at)).or_insert_with(|| {
                nodes.push(SharedNode {
                    at: target.at,
                    visits: Vec::new(),
                });
                nodes.len() - 1
            });
            let visits = &mut nodes[slot].visits;
            visits.push(Visit {
                road: index,
                vertex,
                end: true,
                inner: false,
            });
            let visit = Visit {
                road: target.road,
                vertex: onto.0,
                end: !closed && !onto.1 && (onto.0 == 0 || onto.0 == tail),
                inner: onto.1,
            };
            let known = visits.iter().any(|found| {
                found.road == visit.road
                    && found.vertex == visit.vertex
                    && found.inner == visit.inner
            });
            if !known {
                visits.push(visit);
            }
        }
    }
    // торец со стежком — не тупик: его проход своей нодой уходит
    for node in &mut nodes {
        node.visits.retain(|visit| {
            !stitched_ends.iter().any(|&(road, vertex, at)| {
                visit.road == road && visit.vertex == vertex && !visit.inner && node.at == at
            })
        });
    }
    nodes.retain(|node| !node.visits.is_empty());
    nodes.sort_by_key(|node| node_key(node.at));
    nodes
}

/// Разрывы разметки по дорогам: `breaks[i]` — у `roads[i]`, у дорог вне
/// участников пусто. `junctions` — сколько узлов оказались перекрёстками.
pub struct MarkingBreaks {
    pub breaks: Vec<Vec<Break>>,
    pub junctions: usize,
}

/// Разрывы по общим узлам участвующих дорог. Перекрёсток режет разметку
/// каждой из сошедшихся дорог на полуширину самой широкой из остальных плюс
/// [`JUNCTION_MARGIN`]; тупик — разрыв нулевой длины на торце; стык двух
/// торцов (way разрезан по смене тега) — не разрыв вовсе. Замкнутый way
/// (кольцо одним way) торцов не имеет.
///
/// Стежки (`targets`, [`with_stitches`]) — такие же узлы: торец, дотянутый до
/// чужой оси, рвётся у её кромки, а не у своей последней ноды.
///
/// Это разрывы **асфальта** — по ним гаснут колея и разделительные. Краска
/// рвётся по своим (`roads/node_paint.rs`): главная проходит узел, не теряя
/// линий, и она же снимает с главной эти разрывы, чтобы колея шла сквозь.
pub fn marking_breaks(
    roads: &[RoadLine],
    participates: impl Fn(&RoadLine) -> bool,
    targets: &[[Option<StitchTarget>; 2]],
) -> MarkingBreaks {
    let mut breaks = vec![Vec::new(); roads.len()];
    let mut junctions = 0;
    for node in with_stitches(roads, participates, targets) {
        let SharedNode { at, visits } = &node;
        let at = *at;
        if !node.is_junction() {
            // одна дорога: её торец — тупик, прочие вершины — просто изломы;
            // у шва двух way торцов нет вовсе
            if visits.iter().all(|visit| visit.road == visits[0].road) {
                for visit in visits.iter().filter(|visit| visit.end) {
                    breaks[visit.road].push(Break { at, reach: 0.0 });
                }
            }
            continue;
        }
        junctions += 1;
        for visit in visits {
            let widest_other = visits
                .iter()
                .filter(|other| other.road != visit.road)
                .map(|other| roads[other.road].width / 2.0)
                .fold(0.0_f32, f32::max);
            breaks[visit.road].push(Break {
                at,
                reach: widest_other + JUNCTION_MARGIN,
            });
        }
    }
    MarkingBreaks { breaks, junctions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;

    const NODE: Vec2 = Vec2::new(50.0, 50.0);

    fn breaks_of(roads: &[RoadLine]) -> MarkingBreaks {
        marking_breaks(roads, |road| road.width >= 8.0, &[])
    }

    fn at_node(breaks: &[Break], node: Vec2) -> Break {
        breaks
            .iter()
            .find(|found| found.at == node)
            .copied()
            .unwrap_or_else(|| panic!("no break at {node:?} in {breaks:?}"))
    }

    fn across(x: f32, width: f32) -> RoadLine {
        street(
            vec![Vec2::new(0.0, x), Vec2::new(50.0, x), Vec2::new(100.0, x)],
            width,
        )
    }

    #[test]
    fn a_crossing_breaks_both_streets_by_the_other_half_width() {
        let wide = across(50.0, 16.0);
        let narrow = street(
            vec![Vec2::new(50.0, 0.0), NODE, Vec2::new(50.0, 100.0)],
            8.0,
        );
        let found = breaks_of(&[wide, narrow]);
        assert_eq!(found.junctions, 1);
        assert_eq!(
            at_node(&found.breaks[0], NODE).reach,
            4.0 + JUNCTION_MARGIN,
            "магистраль рвётся на полуширину жилой улицы"
        );
        assert_eq!(at_node(&found.breaks[1], NODE).reach, 8.0 + JUNCTION_MARGIN);
        // торцы обеих — тупики нулевой длины
        for breaks in &found.breaks {
            assert_eq!(breaks.len(), 3);
            assert_eq!(breaks.iter().filter(|found| found.reach == 0.0).count(), 2);
        }
    }

    #[test]
    fn a_t_junction_ends_the_side_street_and_cuts_the_through_one() {
        let through = across(50.0, 12.0);
        let side = street(vec![Vec2::new(50.0, 0.0), NODE], 8.0);
        let found = breaks_of(&[through, side]);
        assert_eq!(found.junctions, 1);
        assert_eq!(at_node(&found.breaks[0], NODE).reach, 4.0 + JUNCTION_MARGIN);
        assert_eq!(at_node(&found.breaks[1], NODE).reach, 6.0 + JUNCTION_MARGIN);
    }

    #[test]
    fn two_ways_meeting_end_to_end_are_one_road() {
        let first = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let second = street(vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)], 8.0);
        let found = breaks_of(&[first, second]);
        assert_eq!(found.junctions, 0);
        // у каждого way разрыв только на дальнем торце — стык не разрыв
        assert_eq!(
            found.breaks[0],
            vec![Break {
                at: Vec2::ZERO,
                reach: 0.0
            }]
        );
        assert_eq!(
            found.breaks[1],
            vec![Break {
                at: Vec2::new(100.0, 0.0),
                reach: 0.0
            }]
        );
    }

    #[test]
    fn a_third_way_at_the_seam_makes_it_a_junction() {
        let first = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let second = street(vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)], 8.0);
        let third = street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 100.0)], 10.0);
        let found = breaks_of(&[first, second, third]);
        assert_eq!(found.junctions, 1);
        assert_eq!(
            at_node(&found.breaks[0], Vec2::new(50.0, 0.0)).reach,
            5.0 + JUNCTION_MARGIN,
            "рвёт самая широкая из остальных"
        );
    }

    #[test]
    fn a_stitched_end_joins_the_street_it_was_stitched_to() {
        let through = across(50.0, 12.0);
        let side = street(vec![Vec2::new(40.0, 0.0), Vec2::new(40.0, 40.0)], 8.0);
        let at = Vec2::new(40.0, 50.0);
        let targets = [
            [None, None],
            [
                None,
                Some(StitchTarget {
                    road: 0,
                    segment: 0,
                    at,
                }),
            ],
        ];
        let found = marking_breaks(&[through, side], |road| road.width >= 8.0, &targets);
        assert_eq!(found.junctions, 1);
        assert_eq!(at_node(&found.breaks[0], at).reach, 4.0 + JUNCTION_MARGIN);
        assert_eq!(at_node(&found.breaks[1], at).reach, 6.0 + JUNCTION_MARGIN);
        assert_eq!(
            found.breaks[1]
                .iter()
                .filter(|found| found.reach == 0.0)
                .count(),
            1,
            "торец со стежком — не тупик: {:?}",
            found.breaks[1]
        );
    }

    #[test]
    fn a_service_road_does_not_break_the_street() {
        let main = across(50.0, 8.0);
        let drive = street(vec![Vec2::new(50.0, 0.0), NODE], 5.0);
        let found = breaks_of(&[main, drive]);
        assert_eq!(found.junctions, 0);
        assert!(found.breaks[0].iter().all(|found| found.reach == 0.0));
        assert!(
            found.breaks[1].is_empty(),
            "проезд не участник — разрывов у него нет"
        );
    }

    #[test]
    fn a_closed_ring_has_no_ends_and_its_entry_is_a_junction() {
        let ring = street(
            vec![
                Vec2::ZERO,
                Vec2::new(20.0, 0.0),
                Vec2::new(20.0, 20.0),
                Vec2::new(0.0, 20.0),
                Vec2::ZERO,
            ],
            12.0,
        );
        let entry = street(vec![Vec2::new(20.0, 50.0), Vec2::new(20.0, 20.0)], 8.0);
        let found = breaks_of(&[ring, entry]);
        assert_eq!(found.junctions, 1);
        assert!(
            found.breaks[0].iter().all(|found| found.reach > 0.0),
            "у замкнутого кольца нет тупиков: {:?}",
            found.breaks[0]
        );
        assert_eq!(
            at_node(&found.breaks[1], Vec2::new(20.0, 20.0)).reach,
            6.0 + JUNCTION_MARGIN
        );
    }

    #[test]
    fn a_bridge_without_a_shared_node_leaves_the_street_below_alone() {
        // мост через улицу: ленты пересекаются, ноды — нет
        let below = across(50.0, 12.0);
        let bridge = street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 100.0)], 12.0);
        let found = breaks_of(&[below, bridge]);
        assert_eq!(found.junctions, 0);
    }
}
