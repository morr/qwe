//! **Сшивка пути через дорогу** — переезд, которого нет в OSM.
//!
//! Подъездной путь в OSM сплошь и рядом рвётся ровно на дороге: два way
//! `railway=rail` подходят к её кромкам с двух сторон, а сам переезд
//! (`railway=level_crossing` и кусок пути поперёк асфальта) не замаплен
//! (Тула, 2613, 3122 → 3140: разрыв 18 м поперёк третьестепенной улицы). На
//! карте путь тогда упирается в дорогу скруглённым торцом и продолжается на
//! той стороне с такого же торца. По правилу «чего нет в OSM — генерировать»
//! такие концы сшиваются в один путь.
//!
//! Пара сшивается, только если всё сразу:
//! - оба конца **свободны** — ни одна вершина другого пути к ним не ближе
//!   [`SHARED_EPSILON`] (конец, стоящий на стрелке, — не обрыв);
//! - пути одного вида (`RailKind`, трамвай не сшивается вовсе — он живёт на
//!   улице);
//! - разрыв короче [`STITCH_GAP_MAX`];
//! - концы **смотрят друг на друга**: направление пути у каждого конца (по
//!   точке в [`HEADING_REACH`] позади) отклоняется от направления на другой
//!   конец не больше чем на [`STITCH_ANGLE_MAX`] градусов;
//! - отрезок между концами пересекает осевую проезжей части в одном уровне —
//!   не мост и не арка. Без дороги разрыв остаётся: тупик у платформы или
//!   упор перед воротами — не переезд.
//!
//! Замер по шести с лишним городам кеша (скрипт в отчёте пачки 5a): Тула — 1
//! (та самая пара), Калуга — 3 (заброшенные пути через улицы), Орёл, Белгород,
//! Берлин, Москва, Ростов, Рязань — 0. Без условия дороги кандидатов было бы
//! в Берлине 37: концы путей у тупиков станции, которые смотрят друг на друга
//! через платформу.

use bevy::math::Vec2;

use crate::map::footprint::segments_cross;
use crate::map::grid::Grid;
use crate::map::osm::{MapData, RailKind, RailLine};

/// Самый длинный разрыв, который ещё сшивается, м: двухполосная улица с
/// тротуарами — 15–20 м, четырёхполосная — под 30.
pub const STITCH_GAP_MAX: f32 = 30.0;
/// Насколько конец может смотреть мимо другого конца, градусов.
const STITCH_ANGLE_MAX: f32 = 12.0;
/// Как далеко позади конца берётся точка направления, м: последнее звено
/// OSM бывает в полметра, и его направление — шум.
const HEADING_REACH: f32 = 8.0;
/// Конец ближе этого к вершине другого пути — общий узел, а не обрыв, м.
const SHARED_EPSILON: f32 = 0.5;
/// Ячейка сеток концов и вершин, м.
const CELL: f32 = 32.0;

/// Свободный конец пути: чей, какой (`false` — первая точка), где и куда
/// смотрит наружу.
#[derive(Clone, Copy)]
struct End {
    rail: usize,
    last: bool,
    at: Vec2,
    outward: Vec2,
}

/// Сшить пути, оборванные на дороге (модульная проза); возвращает, сколько
/// пар сшито. Сшитый путь — первый из пары, второй уходит из `map.rails`.
pub(super) fn stitch_rail_gaps(map: &mut MapData) -> usize {
    let mut stitched = 0;
    // по одной паре за проход: сшивок в городе единицы, а после каждой концы
    // и индексы путей другие
    while let Some((a, b)) = best_pair(map) {
        join(&mut map.rails, a, b);
        stitched += 1;
    }
    stitched
}

/// Ближайшая пара концов, которую правило сшивает.
fn best_pair(map: &MapData) -> Option<(End, End)> {
    let ends = free_ends(&map.rails);
    if ends.len() < 2 {
        return None;
    }
    let mut index: Grid<usize> = Grid::new(CELL);
    for (slot, end) in ends.iter().enumerate() {
        index.insert(end.at, end.at, slot);
    }
    let cos_max = STITCH_ANGLE_MAX.to_radians().cos();
    let mut best: Option<(f32, End, End)> = None;
    for (slot, a) in ends.iter().enumerate() {
        for &other in index
            .near(a.at - STITCH_GAP_MAX, a.at + STITCH_GAP_MAX)
            .iter()
        {
            if other <= slot {
                continue;
            }
            let b = ends[other];
            if a.rail == b.rail || map.rails[a.rail].kind != map.rails[b.rail].kind {
                continue;
            }
            let gap = a.at.distance(b.at);
            if !(SHARED_EPSILON..=STITCH_GAP_MAX).contains(&gap) {
                continue;
            }
            let toward = (b.at - a.at) / gap;
            if a.outward.dot(toward) < cos_max || b.outward.dot(-toward) < cos_max {
                continue;
            }
            if best.is_some_and(|(distance, ..)| distance <= gap) {
                continue;
            }
            if crosses_street(map, a.at, b.at) {
                best = Some((gap, *a, b));
            }
        }
    }
    best.map(|(_, a, b)| (a, b))
}

/// Свободные концы всех путей, кроме трамвая и замкнутых колец.
fn free_ends(rails: &[RailLine]) -> Vec<End> {
    let mut vertices: Grid<usize> = Grid::new(CELL);
    for (rail, line) in rails.iter().enumerate() {
        for &point in &line.points {
            vertices.insert(point, point, rail);
        }
    }
    let shared = |rail: usize, at: Vec2| {
        vertices
            .near(at - SHARED_EPSILON, at + SHARED_EPSILON)
            .into_iter()
            .filter(|&other| other != rail)
            .any(|other| {
                rails[other]
                    .points
                    .iter()
                    .any(|point| point.distance(at) < SHARED_EPSILON)
            })
    };
    let mut ends = Vec::new();
    for (rail, line) in rails.iter().enumerate() {
        let points = &line.points;
        // путь на мосту через дорогу не переезжает: его разрыв — не переезд
        if line.kind == RailKind::Tram
            || line.bridge
            || points.len() < 2
            || points[0] == points[points.len() - 1]
        {
            continue;
        }
        for last in [false, true] {
            let at = if last {
                points[points.len() - 1]
            } else {
                points[0]
            };
            if shared(rail, at) {
                continue;
            }
            let behind = point_behind(points, last);
            let Some(outward) = (at - behind).try_normalize() else {
                continue;
            };
            ends.push(End {
                rail,
                last,
                at,
                outward,
            });
        }
    }
    ends
}

/// Точка пути в [`HEADING_REACH`] позади конца (или дальний конец, если путь
/// короче).
fn point_behind(points: &[Vec2], last: bool) -> Vec2 {
    let mut walk: Box<dyn Iterator<Item = &Vec2>> = if last {
        Box::new(points.iter().rev())
    } else {
        Box::new(points.iter())
    };
    let mut previous = *walk.next().expect("путь из двух точек и больше");
    let mut travelled = 0.0;
    for &point in walk {
        travelled += previous.distance(point);
        previous = point;
        if travelled >= HEADING_REACH {
            break;
        }
    }
    previous
}

/// Пересекает ли отрезок `a→b` осевую проезжей части в одном уровне.
fn crosses_street(map: &MapData, a: Vec2, b: Vec2) -> bool {
    map.roads
        .iter()
        .filter(|road| road.is_carriageway() && !road.bridge)
        .any(|road| {
            road.points
                .windows(2)
                .any(|link| segments_cross(a, b, link[0], link[1]))
        })
}

/// Путь `a.rail` продолжается путём `b.rail` через разрыв; второй удаляется.
fn join(rails: &mut Vec<RailLine>, a: End, b: End) {
    let taken = rails.remove(b.rail);
    let host = if a.rail > b.rail { a.rail - 1 } else { a.rail };
    let line = &mut rails[host];
    if !a.last {
        line.points.reverse();
    }
    let mut tail = taken.points;
    if b.last {
        tail.reverse();
    }
    line.points.extend(tail);
    line.service = line.service.or(taken.service);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    /// Улица поперёк оси x = 0 на высоте `y` и два пути с юга и севера,
    /// оборванные в `gap` метрах друг от друга.
    fn crossing(gap: f32, north_x: f32) -> MapData {
        let mut map = MapData::default();
        map.roads.push(fixture::street(
            vec![Vec2::new(-60.0, 0.0), Vec2::new(60.0, 0.0)],
            8.0,
        ));
        map.rails.push(fixture::rail(
            vec![Vec2::new(0.0, -80.0), Vec2::new(0.0, -gap / 2.0)],
            5.0,
        ));
        map.rails.push(fixture::rail(
            vec![Vec2::new(north_x, gap / 2.0), Vec2::new(north_x, 80.0)],
            5.0,
        ));
        map
    }

    #[test]
    fn a_track_broken_across_a_street_is_stitched() {
        let mut map = crossing(18.0, 0.0);
        assert_eq!(stitch_rail_gaps(&mut map), 1);
        assert_eq!(map.rails.len(), 1);
        let points = &map.rails[0].points;
        assert_eq!(points.first(), Some(&Vec2::new(0.0, -80.0)));
        assert_eq!(points.last(), Some(&Vec2::new(0.0, 80.0)));
    }

    #[test]
    fn the_stitch_holds_whichever_way_the_points_run() {
        let mut map = crossing(18.0, 0.0);
        map.rails[0].points.reverse();
        map.rails[1].points.reverse();
        assert_eq!(stitch_rail_gaps(&mut map), 1);
        assert_eq!(map.rails[0].points.len(), 4);
    }

    #[test]
    fn no_street_in_the_gap_leaves_it_open() {
        let mut map = crossing(18.0, 0.0);
        map.roads.clear();
        assert_eq!(stitch_rail_gaps(&mut map), 0);
        assert_eq!(map.rails.len(), 2);
    }

    #[test]
    fn a_bridge_in_the_gap_is_not_a_level_crossing() {
        let mut map = crossing(18.0, 0.0);
        map.roads[0].bridge = true;
        assert_eq!(stitch_rail_gaps(&mut map), 0);
    }

    #[test]
    fn a_gap_too_long_or_ends_looking_past_each_other_stay_open() {
        assert_eq!(stitch_rail_gaps(&mut crossing(36.0, 0.0)), 0);
        // 6 м вбок на 18 м разрыва — 18° мимо
        assert_eq!(stitch_rail_gaps(&mut crossing(18.0, 6.0)), 0);
    }

    #[test]
    fn an_end_on_a_switch_is_not_free() {
        let mut map = crossing(18.0, 0.0);
        // ответвление от южного конца: он — узел стрелки, не обрыв
        map.rails.push(fixture::rail(
            vec![Vec2::new(0.0, -9.0), Vec2::new(-30.0, -40.0)],
            5.0,
        ));
        assert_eq!(stitch_rail_gaps(&mut map), 0);
    }
}
