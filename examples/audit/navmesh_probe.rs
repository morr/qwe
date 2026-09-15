//! Проходимость сетки в заданных точках — после заливки и после прунинга, ровно
//! как её строит игра. Для вопросов вида «почему внутрь кремля никто не ходит»:
//! точка проходима, но отрезана, или залита уже при заливке — и чем.
//!
//! ```text
//! cargo run --example navmesh_probe -- tula 4680,2454 4630,2400
//! ```

#[path = "../common/mod.rs"]
mod common;

use bevy::math::Vec2;
use qwe::city::City;
use qwe::grid::world_to_tile;
use qwe::map::osm::model::point_in_area;
use qwe::navigation::{Navmesh, build_polymesh_from_map, snap_portal_position};
use qwe::settings::POLYMESH_AGENT_RADIUS_MIN;

/// Арки не дальше этого от первой точки печатаются по центральной линии, м.
const PASSAGE_REACH: f32 = 300.0;
/// Насколько линия арки продолжается за её концы, м.
const OVERSHOOT: f32 = 6.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let slug = args.next().unwrap_or_else(|| "tula".to_string());
    let city = City::ALL
        .into_iter()
        .find(|city| city.slug() == slug)
        .unwrap_or_else(|| panic!("unknown city {slug}"));
    let points: Vec<Vec2> = args
        .map(|arg| {
            let (x, y) = arg.split_once(',').expect("point as x,y");
            Vec2::new(x.parse().expect("x"), y.parse().expect("y"))
        })
        .collect();

    let mut map = common::load_map(city);
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let portal = snap_portal_position(&navmesh, city.portal_hint()).expect("portal");
    let gates = navmesh.open_sealed_fences(&mut map, portal);
    let filled = navmesh.clone();
    let pruned = navmesh.prune_unreachable(world_to_tile(portal));
    println!("{}: {gates} gates, {pruned} tiles pruned", city.slug());

    // радиус агента — дефолт настроек игры (`PolymeshDebug::default`)
    let mesh = build_polymesh_from_map(&map, POLYMESH_AGENT_RADIUS_MIN).expect("polymesh");
    for &point in &points {
        println!(
            "({:.0}, {:.0}): polymesh {}",
            point.x,
            point.y,
            mesh.contains(point)
        );
    }
    // арки у первой точки: по центральной линии, с запасом за концами, — где
    // проход закрыт в сетке (`#`) и в полигональном меше (`#`)
    if let Some(&around) = points.first() {
        for road in map.roads.iter().filter(|road| road.passage) {
            if road
                .points
                .iter()
                .all(|p| p.distance(around) > PASSAGE_REACH)
            {
                continue;
            }
            let (first, last) = (road.points[0], *road.points.last().unwrap());
            let lead = (road.points[1] - first).normalize_or_zero();
            let tail = (last - road.points[road.points.len() - 2]).normalize_or_zero();
            let mut path = vec![first - lead * OVERSHOOT];
            path.extend(&road.points);
            path.push(last + tail * OVERSHOOT);
            let (mut grid, mut poly) = (String::new(), String::new());
            for pair in path.windows(2) {
                let steps = (pair[0].distance(pair[1]) / 0.5).ceil().max(1.0) as usize;
                for step in 0..steps {
                    let p = pair[0].lerp(pair[1], step as f32 / steps as f32);
                    let tile = world_to_tile(p);
                    grid.push(if navmesh.is_passable(tile.x, tile.y) {
                        '.'
                    } else {
                        '#'
                    });
                    poly.push(if mesh.contains(p) { '.' } else { '#' });
                }
            }
            println!(
                "passage ({:.0}, {:.0})->({:.0}, {:.0}) width {:.1}\n  grid {grid}\n  mesh {poly}",
                first.x, first.y, last.x, last.y, road.width
            );
        }
    }

    for point in points {
        let tile = world_to_tile(point);
        let covering: Vec<String> = map
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| point_in_area(point, building))
            .map(|(index, building)| {
                format!("#{index} {:?} {:?}", building.kind, building.building_use)
            })
            .collect();
        let passages = map.roads.iter().filter(|road| road.passage).count();
        println!(
            "({:.0}, {:.0}): filled {}, after prune {}, inside [{}] ({passages} passages in the map)",
            point.x,
            point.y,
            filled.is_passable(tile.x, tile.y),
            navmesh.is_passable(tile.x, tile.y),
            covering.join("; ")
        );
    }
}
