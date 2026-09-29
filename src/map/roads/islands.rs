//! **Островки безопасности и площади полотна** — то, что запрос v15 приносит
//! о проезжей части сверх её осей (`MapData::road_nodes`,
//! `MapData::road_areas`):
//!
//! - **островок-точка** — `traffic_calming=island` узлом или переход с
//!   `crossing:island=yes`: посреди двусторонней улицы в две полосы и больше
//!   встаёт бордюрная линза [`REFUGE_LENGTH`] вдоль оси. Узел стоит на оси, и
//!   лента дороги его не обходит — островок кладётся поверх асфальта и
//!   краски, зебра уходит под него, как на месте;
//! - **контур островка** (`RoadAreaKind::Island`) — бордюрный островок по
//!   контуру, тоже поверх;
//! - **контур полотна** (`RoadAreaKind::Carriageway`) — асфальт под лентами:
//!   площадь, карман, расширение, которых ось не описывает;
//! - **пешеходная площадь** (`RoadAreaKind::Walkway`: `area:highway=footway`,
//!   `highway=pedestrian` + `area=yes`) — заливкой плиткой тротуара в его
//!   слой. Без неё площадь `area:highway` была голой землёй, на которой
//!   обрывался проезд (Тула, у Ленина), а `pedestrian` + `area=yes` — одним
//!   кольцом ленты по контуру с землёй внутри (остров кольца у 2539 2393).
//!
//! **Замкнутая линия `highway=*` + `area=yes` лентой не кладётся**
//! ([`RoadIslands::outlines`]): разбор отдаёт её и площадью, и линией (линия
//! нужна прочим потребителям сети), а нарисованная лентой она — кольцо поверх
//! заливки. Узнаётся по совпадению точек с контуром площади.
//!
//! В Туле островков нет вовсе (`references/osm-coverage.md`, «v15»), контуров
//! полотна — дюжина дворовых, пешеходных площадей — 19; в Берлине, Париже и
//! Нью-Йорке — сотни.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::node_key;
use super::{is_carriageway, lane_count};
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::osm::{MapData, RoadAreaKind, RoadLine, RoadNodeKind};
use crate::map::shapes::{Shape, is_ring, oriented};

/// Длина островка-точки вдоль оси, м: зебра и по метру бордюра с каждой
/// стороны.
const REFUGE_LENGTH: f32 = 8.0;
/// Полуширина островка-точки, м: полтора метра — стоит пешеход с коляской.
const REFUGE_HALF_WIDTH: f32 = 0.9;
/// Звенья каждой стороны линзы.
const REFUGE_STEPS: usize = 8;

/// Что рисуется из данных v15: бордюрные островки — в слой бордюра над
/// асфальтом, контуры полотна — в асфальт улиц, пешеходные площади — в слой
/// тротуаров.
#[derive(Default)]
pub(super) struct RoadIslands {
    pub kerbs: Vec<Shape>,
    pub carriageways: Vec<Shape>,
    pub walkways: Vec<Shape>,
    /// По дороге `drawn`: её ось — контур площади дороги (`highway=*` +
    /// `area=yes`), и лентой она не кладётся.
    pub outlines: Vec<bool>,
    /// Островков-точек, встало на улицу.
    pub refuges: usize,
}

impl RoadIslands {
    /// Островки и площади карты `map` по дорогам `drawn`, нарисованным по
    /// `paths`.
    pub fn new(map: &MapData, drawn: &[&RoadLine], paths: &[impl AsRef<[Vec2]>]) -> Self {
        let mut islands = Self::default();
        // площади по первой точке контура — чтобы узнать их линии
        let mut areas: HashMap<(i32, i32), Vec<&[Vec2]>> = HashMap::new();
        for area in &map.road_areas {
            if area.outline.len() < 3 {
                continue;
            }
            let shape = vec![oriented(&area.outline, true)];
            match area.kind {
                RoadAreaKind::Island => islands.kerbs.push(shape),
                RoadAreaKind::Carriageway => islands.carriageways.push(shape),
                RoadAreaKind::Walkway => islands.walkways.push(shape),
            }
            if area.kind != RoadAreaKind::Island {
                areas
                    .entry(node_key(area.outline[0]))
                    .or_default()
                    .push(&area.outline);
            }
        }
        islands.outlines = drawn
            .iter()
            .map(|road| {
                let points = &road.points;
                // замкнутая линия: последняя точка — первая, до неё — контур
                let Some((_, open)) = points.split_last().filter(|_| is_ring(points)) else {
                    return false;
                };
                areas
                    .get(&node_key(open[0]))
                    .is_some_and(|outlines| outlines.contains(&open))
            })
            .collect();
        // улица, на оси которой стоит узел: островок — только на
        // двусторонней, где ему есть место между встречными полосами
        let mut owners: HashMap<(i32, i32), usize> = HashMap::new();
        for (index, road) in drawn.iter().enumerate() {
            if road.oneway || road.bridge || !is_carriageway(road) || lane_count(road) < 2 {
                continue;
            }
            for &point in &road.points {
                owners.entry(node_key(point)).or_insert(index);
            }
        }
        for node in &map.road_nodes {
            let refuge = matches!(
                node.kind,
                RoadNodeKind::Island | RoadNodeKind::Crossing { island: true, .. }
            );
            if !refuge {
                continue;
            }
            let Some(&road) = owners.get(&node_key(node.pos)) else {
                continue;
            };
            if let Some(lens) = refuge_at(paths[road].as_ref(), node.pos) {
                islands.kerbs.push(vec![lens]);
                islands.refuges += 1;
            }
        }
        islands
    }
}

/// Линза островка на пути `path` в точке `at`: полная полуширина посередине,
/// в ноль к концам. Путь короче островка — ничего.
fn refuge_at(path: &[Vec2], at: Vec2) -> Option<Vec<[f32; 2]>> {
    let (along, total) = arclengths(path);
    let (_, center) = nearest_on_path(path, at)?;
    let half = REFUGE_LENGTH / 2.0;
    if center < half || center > total - half {
        return None;
    }
    let mut left = Vec::with_capacity(REFUGE_STEPS + 1);
    let mut right = Vec::with_capacity(REFUGE_STEPS + 1);
    for step in 0..=REFUGE_STEPS {
        let u = step as f32 / REFUGE_STEPS as f32 * 2.0 - 1.0;
        let (point, direction) = place_on_path(path, &along, center + u * half)?;
        let spread = REFUGE_HALF_WIDTH * (1.0 - u * u).max(0.0).sqrt();
        left.push(point + direction.perp() * spread);
        right.push(point - direction.perp() * spread);
    }
    // концы линзы — одна точка на оси
    right.pop();
    right.remove(0);
    right.reverse();
    let ring: Vec<Vec2> = left.into_iter().chain(right).collect();
    Some(oriented(&ring, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;
    use crate::map::osm::{RoadArea, RoadNode};
    use crate::map::shapes::contour_area;

    fn two_way() -> RoadLine {
        // узел перехода в OSM — вершина улицы
        let mut road = street(
            vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)],
            8.0,
        );
        road.lanes = Some(2);
        road
    }

    #[test]
    fn a_crossing_with_an_island_puts_a_kerbed_lens_on_the_axis() {
        let road = two_way();
        let map = MapData {
            road_nodes: vec![RoadNode {
                pos: Vec2::new(50.0, 0.0),
                kind: RoadNodeKind::Crossing {
                    signals: false,
                    island: true,
                    marked: true,
                },
            }],
            ..MapData::default()
        };
        let paths = [road.points.clone()];
        let islands = RoadIslands::new(&map, &[&road], &paths);
        assert_eq!(islands.refuges, 1);
        let lens = &islands.kerbs[0][0];
        let xs: Vec<f32> = lens.iter().map(|point| point[0]).collect();
        let ys: Vec<f32> = lens.iter().map(|point| point[1]).collect();
        let span = |values: &[f32]| {
            values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - values.iter().copied().fold(f32::INFINITY, f32::min)
        };
        assert!((span(&xs) - REFUGE_LENGTH).abs() < 1e-3, "{xs:?}");
        assert!((span(&ys) - 2.0 * REFUGE_HALF_WIDTH).abs() < 1e-3, "{ys:?}");
        assert!(contour_area(lens) > 0.0);
    }

    #[test]
    fn a_one_way_street_or_a_plain_crossing_gets_no_island() {
        let mut road = two_way();
        road.oneway = true;
        let island = |kind| MapData {
            road_nodes: vec![RoadNode {
                pos: Vec2::new(50.0, 0.0),
                kind,
            }],
            ..MapData::default()
        };
        let paths = [road.points.clone()];
        let map = island(RoadNodeKind::Island);
        assert_eq!(RoadIslands::new(&map, &[&road], &paths).refuges, 0);
        road.oneway = false;
        let plain = island(RoadNodeKind::Crossing {
            signals: true,
            island: false,
            marked: true,
        });
        assert_eq!(RoadIslands::new(&plain, &[&road], &paths).refuges, 0);
    }

    #[test]
    fn areas_split_into_kerbs_and_asphalt() {
        let square = vec![
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let area = |kind| RoadArea {
            outline: square.clone(),
            kind,
        };
        let map = MapData {
            road_areas: vec![
                area(RoadAreaKind::Island),
                area(RoadAreaKind::Carriageway),
                area(RoadAreaKind::Walkway),
            ],
            ..MapData::default()
        };
        let islands = RoadIslands::new(&map, &[], &[] as &[Vec<Vec2>]);
        assert_eq!(islands.kerbs.len(), 1);
        assert_eq!(islands.carriageways.len(), 1);
        assert_eq!(islands.walkways.len(), 1);
        assert_eq!(islands.refuges, 0);
    }

    /// Площадь `highway=pedestrian` + `area=yes` приходит и контуром, и
    /// замкнутой линией; линия лентой не кладётся — иначе кольцо поверх
    /// заливки (остров кольца в Туле). Замкнутая линия с другим контуром и
    /// открытая линия по тем же точкам — обычные дороги.
    #[test]
    fn the_closed_line_of_an_area_is_its_outline_not_a_ribbon() {
        let square = vec![
            Vec2::ZERO,
            Vec2::new(18.0, 0.0),
            Vec2::new(18.0, 18.0),
            Vec2::new(0.0, 18.0),
        ];
        let map = MapData {
            road_areas: vec![RoadArea {
                outline: square.clone(),
                kind: RoadAreaKind::Walkway,
            }],
            ..MapData::default()
        };
        let closed = |points: &[Vec2]| {
            let mut ring = points.to_vec();
            ring.push(points[0]);
            street(ring, 3.5)
        };
        let area_line = closed(&square);
        let mut other = square.clone();
        other[2] = Vec2::new(20.0, 20.0);
        let other_ring = closed(&other);
        let open = street(square.clone(), 3.5);
        let roads = [&area_line, &other_ring, &open];
        let paths: Vec<Vec<Vec2>> = roads.iter().map(|road| road.points.clone()).collect();

        let islands = RoadIslands::new(&map, &roads, &paths);

        assert_eq!(islands.outlines, [true, false, false]);
        assert_eq!(islands.walkways.len(), 1);
    }
}
