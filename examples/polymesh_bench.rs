//! Детерминированный прогон поиска пути по полигональному мешу — без Bevy,
//! без пешек и демонов.
//!
//! Нужен ровно затем, что живьём баг ловится плохо: разбухание памяти и
//! зависание случаются на редких маршрутах, а каждый запуск игры даёт свой
//! случайный набор запросов. Здесь набор один и тот же при каждом запуске
//! (сид), каждый запрос логируется ДО и ПОСЛЕ вызова, и рядом печатается RSS
//! процесса — так видно и какой именно маршрут виноват, и в какой момент
//! память шагнула.
//!
//! ```text
//! cargo run --example polymesh_bench -- [tasks] [agent_radius]
//! cargo run --example polymesh_bench -- 500 0.2
//! ```
//!
//! Зависший запрос виден как строка `START` без парной `END`: пример не
//! завершится, и последняя пара координат в логе и есть виновник.

use std::time::Instant;

use bevy::math::{IVec2, Vec2};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use qwe::city::City;
use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::{
    Navmesh, build_polymesh_from_map, find_passable_tile_near, find_path_polymesh,
    snap_portal_position,
};
use qwe::settings::{HUMAN_WANDER_RANGE, MAP_SIZE};

const CITY: City = City::Tula;
const SEED: u64 = 0xC0FFEE;
/// Доля целей «к случайному зданию» — как в `human::pick_wander_targets`.
const WANDER_TO_BUILDING_SHARE: f32 = 0.8;
const MAP_MARGIN: f32 = 4.0;
/// Порог, выше которого запрос считается медленным и печатается всегда.
const SLOW_MS: f32 = 50.0;
/// Множитель сдвига старта с центра тайла: 1 — как в игре, 0 — ровно по
/// центрам. Переключатель для A/B, набор запросов при этом не меняется.
const JITTER: f32 = 1.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let tasks: usize = args
        .next()
        .map(|value| value.parse().expect("tasks must be a number"))
        .unwrap_or(500);
    let radius: f32 = args
        .next()
        .map(|value| value.parse().expect("radius must be a number"))
        .unwrap_or(0.2);

    println!("rss at start: {}", rss());
    let map = load_map();
    let navmesh = build_navmesh(&map);
    println!("rss after navmesh: {}", rss());

    let started = Instant::now();
    let build = build_polymesh_from_map(&map, radius).expect("build was not cancelled");
    let polygons: usize = build.mesh.layers.iter().map(|l| l.polygons.len()).sum();
    let (grid, chunk_size) = build.chunks();
    println!(
        "polymesh built in {:?}: {}x{} chunks of {:.0}x{:.0} m, {polygons} polygons",
        started.elapsed(),
        grid.x,
        grid.y,
        chunk_size.x,
        chunk_size.y
    );
    println!("rss after polymesh: {}", rss());

    let queries = generate_queries(&map, &navmesh, tasks);
    println!("\n{} queries, agent radius {radius}\n", queries.len());

    let mut found = 0;
    let mut missed = 0;
    let mut slowest = (0.0f32, 0usize);
    let mut total = 0.0f32;

    for (index, (from, to)) in queries.iter().enumerate() {
        // ДО вызова: зависший поиск сюда доедет, а до `END` уже нет
        println!(
            "[{index:>4}] START {:.0}m  {from:?} -> {to:?}",
            from.distance(*to)
        );
        let started = Instant::now();
        let path = find_path_polymesh(&build, *from, *to);
        let elapsed = started.elapsed().as_secs_f32() * 1000.0;
        total += elapsed;

        if path.is_some() {
            found += 1;
        } else {
            missed += 1;
        }
        if elapsed > slowest.0 {
            slowest = (elapsed, index);
        }
        // подробная строка только для заметных запросов, иначе лог не читается
        if elapsed > SLOW_MS || path.is_none() || index % 50 == 0 {
            println!(
                "[{index:>4}] END   {elapsed:>8.2} ms  {}  waypoints {}  rss {}",
                if path.is_some() { "found" } else { "MISS " },
                path.as_ref().map(Vec::len).unwrap_or(0),
                rss()
            );
        }
    }

    println!(
        "\nfound {found}, missed {missed} ({:.1}%), avg {:.2} ms, slowest {:.1} ms at #{}",
        missed as f32 / queries.len() as f32 * 100.0,
        total / queries.len() as f32,
        slowest.0,
        slowest.1
    );
    println!("rss at end: {}", rss());
}

/// RSS процесса — на macOS дешевле всего спросить `ps`.
fn rss() -> String {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output();
    match output {
        Ok(output) => {
            let kilobytes: f64 = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or(0.0);
            format!("{:.2} GB", kilobytes / 1_048_576.0)
        }
        Err(_) => "n/a".to_string(),
    }
}

/// Карта — только из кеша, как у `pathfinding_bench`.
fn load_map() -> MapData {
    let path = overpass::cache_path(CITY);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no OSM cache at {}: {error}. run the app once to download it",
            path.display()
        )
    });
    parse::parse(&json, CITY).expect("failed to parse cached OSM json")
}

/// Сеточный navmesh нужен только затем, чтобы цели выбирались ровно так же,
/// как в игре: `find_passable_tile_near` по прорезанной от портала сетке.
fn build_navmesh(map: &MapData) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    let portal =
        snap_portal_position(&navmesh, CITY.portal_hint()).expect("no clear spot for portal");
    navmesh.prune_unreachable(world_to_tile(portal));
    navmesh
}

/// Тот же профиль нагрузки, что у мирного блуждания: 80% — случайное здание
/// города (длинные маршруты), 20% — прогулка в 20–40 м.
fn generate_queries(map: &MapData, navmesh: &Navmesh, count: usize) -> Vec<(Vec2, Vec2)> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut queries = Vec::with_capacity(count);

    while queries.len() < count {
        let start = loop {
            let candidate = IVec2::new(
                rng.random_range(0..navmesh.grid_size.x),
                rng.random_range(0..navmesh.grid_size.y),
            );
            if navmesh.is_passable(candidate.x, candidate.y) {
                break candidate;
            }
        };

        let to_building =
            rng.random_range(0.0..1.0) < WANDER_TO_BUILDING_SHARE && !map.buildings.is_empty();
        let target = if to_building {
            let building = &map.buildings[rng.random_range(0..map.buildings.len())];
            building.outer[rng.random_range(0..building.outer.len())]
        } else {
            let direction = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU));
            let distance = rng.random_range(HUMAN_WANDER_RANGE.0..HUMAN_WANDER_RANGE.1);
            (tile_center(start) + direction * distance)
                .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN)
        };

        let Some(end) = find_passable_tile_near(navmesh, world_to_tile(target)) else {
            continue;
        };
        if end == start {
            continue;
        }
        // старт — не центр тайла: в игре это `SimPosition` пешки, точка
        // произвольная, и попасть она может как угодно близко к препятствию.
        // Розыгрыш идёт всегда, даже когда сдвиг выключен: иначе поток RNG
        // разъедется и наборы запросов у двух прогонов перестанут совпадать
        let jitter = Vec2::new(rng.random_range(-0.9..0.9), rng.random_range(-0.9..0.9));
        queries.push((tile_center(start) + jitter * JITTER, tile_center(end)));
    }

    queries
}
