//! Проверка стартовой области: стоит ли хинт портала города на полигональном
//! меше. Париж ловился именно здесь — центр карты на острове Сите, а остров
//! был дырой водного мультиполигона «La Seine» и целиком уходил в препятствие.
//!
//! Печатает заодно число дыр во входных кольцах и время постройки: дыры идут
//! в то же boolean-объединение, и цена вопроса видна здесь.
//!
//! ```text
//! cargo run --release --example polymesh_start_area -- [radius] [city ...]
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::Vec2;
use qwe::city::City;
use qwe::navigation::{build_polymesh_from_map, find_path_polymesh};

/// Пробы вокруг портала: за мостом с каждой стороны, если портал на острове.
const PROBES: [Vec2; 4] = [
    Vec2::new(1500.0, 0.0),
    Vec2::new(-1500.0, 0.0),
    Vec2::new(0.0, 1000.0),
    Vec2::new(0.0, -1000.0),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let radius: f32 = args
        .first()
        .map(|a| a.parse().expect("radius"))
        .unwrap_or(0.4);
    let cities: Vec<City> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|name| {
                City::ALL
                    .into_iter()
                    .find(|city| city.slug() == name.to_lowercase())
                    .unwrap_or_else(|| panic!("unknown city {name}"))
            })
            .collect()
    } else {
        City::ALL.to_vec()
    };

    for city in cities {
        let map = common::load_map(city);
        let holes: usize = map
            .buildings
            .iter()
            .chain(&map.water)
            .map(|area| area.holes.len())
            .sum();
        let started = Instant::now();
        let build = build_polymesh_from_map(&map, radius).expect("not cancelled");
        let elapsed = started.elapsed();
        let portal = city.portal_hint();
        let empty = build
            .mesh
            .layers
            .iter()
            .filter(|layer| layer.polygons.is_empty())
            .count();
        let polygons: usize = build.mesh.layers.iter().map(|l| l.polygons.len()).sum();
        println!(
            "{city:?}: portal {portal:?} on mesh = {}; {holes} ring holes in input, \
             {polygons} polygons, {empty} empty layers, {} obstacle contours, built in {elapsed:?}",
            build.contains(portal),
            build.obstacles.len(),
        );
        if !build.contains(portal) {
            println!(
                "   nearest free point: {:?}",
                build.nearest_free_point(portal)
            );
        }
        // стоять на меше мало: стартовая область могла оказаться отрезанным
        // карманом. Пробы вокруг портала — четыре точки в километре с
        // небольшим, каждая подсаженная на меш
        for offset in PROBES {
            let target = portal + offset;
            let verdict = match build.nearest_free_point(target) {
                None => "off the mesh".to_string(),
                Some(probe) => match find_path_polymesh(&build, portal, probe) {
                    Some(path) => format!("reachable, {} waypoints", path.len()),
                    None => "NO PATH".to_string(),
                },
            };
            println!("   probe {target:?}: {verdict}");
        }
    }
}
