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

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::meshing::Break;
use crate::map::osm::RoadLine;

/// Шаг квантования координат узла, м. Одна нода OSM даёт одну и ту же точку
/// на всех своих ways, квантование лишь страхует от округления.
const NODE_GRID: f32 = 0.05;

/// Запас разрыва за краем поперечной улицы, м: линия кончается чуть раньше,
/// чем начинается перекрёсток, как стоп-линия перед ним.
pub const JUNCTION_MARGIN: f32 = 1.0;

/// Чья дорога прошла через узел и торец ли это её.
struct Visit {
    road: usize,
    end: bool,
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
pub fn marking_breaks(
    roads: &[RoadLine],
    participates: impl Fn(&RoadLine) -> bool,
) -> MarkingBreaks {
    let mut nodes: HashMap<(i32, i32), (Vec2, Vec<Visit>)> = HashMap::new();
    for (index, road) in roads.iter().enumerate() {
        if !participates(road) || road.points.len() < 2 {
            continue;
        }
        let last = road.points.len() - 1;
        let closed = road.points[0] == road.points[last];
        for (vertex, &point) in road.points.iter().enumerate() {
            let end = !closed && (vertex == 0 || vertex == last);
            let key = (
                (point.x / NODE_GRID).round() as i32,
                (point.y / NODE_GRID).round() as i32,
            );
            nodes
                .entry(key)
                .or_insert_with(|| (point, Vec::new()))
                .1
                .push(Visit { road: index, end });
        }
    }

    let mut breaks = vec![Vec::new(); roads.len()];
    let mut junctions = 0;
    for (at, visits) in nodes.into_values() {
        let mut distinct: Vec<usize> = visits.iter().map(|visit| visit.road).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() < 2 {
            // одна дорога: её торец — тупик, прочие вершины — просто изломы
            for visit in visits.iter().filter(|visit| visit.end) {
                breaks[visit.road].push(Break { at, reach: 0.0 });
            }
            continue;
        }
        // два way торцами друг к другу — одна дорога, разрезанная надвое
        if distinct.len() == 2 && visits.len() == 2 && visits.iter().all(|visit| visit.end) {
            continue;
        }
        junctions += 1;
        for visit in &visits {
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
        marking_breaks(roads, |road| road.width >= 8.0)
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
