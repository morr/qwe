//! Сколько путей цепляется за узлы сетки чанков — точки, где сходятся четыре
//! слоя.
//!
//! Живьём это видно как «звезда»: в узле сетки сходятся десятки путей сразу,
//! причём лучи идут во все стороны и проходят сквозь кварталы. Здесь тот же
//! набор запросов, что у `polymesh_bench`, но считается другое: где стоят
//! промежуточные точки пути относительно сетки чанков — в узле (угол четырёх
//! чанков), на шве или внутри чанка, — и во сколько обходится сам путь против
//! плоского меша.
//!
//! ```text
//! cargo run --release --example polymesh_corner_audit -- [tasks] [agent_radius]
//! QWE_POLYMESH_CHUNK_M=99000 cargo run --release --example polymesh_corner_audit
//! ```

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
const WANDER_TO_BUILDING_SHARE: f32 = 0.8;
const MAP_MARGIN: f32 = 4.0;
/// Допуск «точка стоит на линии сетки»: шов квантуется сантиметром.
const EPS: f32 = 0.05;

fn main() {
    let mut args = std::env::args().skip(1);
    let tasks: usize = args
        .next()
        .map(|value| value.parse().expect("tasks must be a number"))
        .unwrap_or(500);
    let radius: f32 = args
        .next()
        .map(|value| value.parse().expect("radius must be a number"))
        .unwrap_or(0.4);

    let map = load_map();
    let navmesh = build_navmesh(&map);
    let build = build_polymesh_from_map(&map, radius).expect("build was not cancelled");
    let (grid, chunk_size) = build.chunks();
    println!(
        "polymesh: {}x{} chunks of {:.0}x{:.0} m, agent radius {radius}",
        grid.x, grid.y, chunk_size.x, chunk_size.y
    );

    let queries = generate_queries(&map, &navmesh, tasks);
    println!("{} queries\n", queries.len());

    let on_line = |value: f32, span: f32| {
        let node = (value / span).round();
        (value - node * span).abs() <= EPS
    };

    let mut paths = 0usize;
    let mut with_node = 0usize;
    let mut waypoints = 0usize;
    let mut node_waypoints = 0usize;
    let mut seam_waypoints = 0usize;
    let mut hot: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    let mut detour = 0.0f64;
    let mut straight = 0.0f64;
    // излом в узле: сколько метров стоит заход в него и свободен ли прямой
    // срез — если свободен, изгиб не от геометрии, а от коридора
    let mut bend_cost = 0.0f64;
    let mut bends = 0usize;
    let mut free_cuts = 0usize;
    // отрезков пути, у которых нашлась точка вне меша
    let mut off_mesh = 0usize;
    let mut deep = 0usize;
    let mut depth = 0.0f32;
    let mut grazing = 0usize;

    for (from, to) in &queries {
        let Some(path) = find_path_polymesh(&build, *from, *to) else {
            continue;
        };
        paths += 1;
        straight += from.distance(*to) as f64;
        detour += path
            .windows(2)
            .map(|w| w[0].distance(w[1]) as f64)
            .sum::<f64>();

        // страховка для сглаживания: срезанный отрезок обязан лежать на меше.
        // Сэмплирование грубее честного прохода по полигонам, но независимо от
        // него — на несглаженном пути даёт ноль, и на сглаженном обязано тоже
        for pair in path.windows(2) {
            let steps = (pair[0].distance(pair[1]) / 0.5).ceil().max(1.0) as usize;
            let worst = (0..=steps)
                .map(|step| pair[0].lerp(pair[1], step as f32 / steps as f32))
                .filter(|point| !build.contains(*point))
                // насколько точка вне меша: сантиметры — это касание кромки
                // (путь идёт вплотную к стене, `point_in_mesh` на границе даёт
                // false), метры — это дыра в проверке отрезка
                // `None` — снап в метр не достал, точка глубоко в препятствии
                .map(|point| {
                    build
                        .nearest_free_point(point)
                        .map(|free| free.distance(point))
                        .unwrap_or(9.99)
                })
                .fold(0.0f32, f32::max);
            if worst > 0.0 {
                off_mesh += 1;
                depth = depth.max(worst);
                if worst > 0.5 {
                    deep += 1;
                } else if worst > 0.05 {
                    grazing += 1;
                }
            }
        }

        let mut hit = false;
        // первая точка — сам старт, последняя — цель: обе не выбраны поиском
        for &point in path.iter().take(path.len().saturating_sub(1)).skip(1) {
            waypoints += 1;
            let (x, y) = (
                on_line(point.x, chunk_size.x),
                on_line(point.y, chunk_size.y),
            );
            if x && y {
                node_waypoints += 1;
                hit = true;
                let index = path.iter().position(|p| *p == point).unwrap_or(0);
                if index > 0 && index + 1 < path.len() {
                    let (before, after) = (path[index - 1], path[index + 1]);
                    let saving = (before.distance(point) + point.distance(after)
                        - before.distance(after)) as f64;
                    bend_cost += saving;
                    bends += 1;
                    // срез свободен, если весь отрезок стоит на меше — тогда
                    // изгиб не от препятствия, а от закрытого коридором чанка
                    let steps = (before.distance(after) / 0.5).ceil().max(1.0) as usize;
                    if (0..=steps)
                        .all(|step| build.contains(before.lerp(after, step as f32 / steps as f32)))
                    {
                        free_cuts += 1;
                    }
                }
                *hot.entry((
                    (point.x / chunk_size.x).round() as i32,
                    (point.y / chunk_size.y).round() as i32,
                ))
                .or_default() += 1;
            } else if x || y {
                seam_waypoints += 1;
            }
        }
        if hit {
            with_node += 1;
        }
    }

    println!(
        "paths found: {paths}\n\
         paths through a chunk grid NODE: {with_node} ({:.1}%)\n\
         waypoints: {waypoints} — {node_waypoints} on a node ({:.1}%), \
         {seam_waypoints} on a seam ({:.1}%)\n\
         path length / straight distance: {:.3}",
        with_node as f32 / paths.max(1) as f32 * 100.0,
        node_waypoints as f32 / waypoints.max(1) as f32 * 100.0,
        seam_waypoints as f32 / waypoints.max(1) as f32 * 100.0,
        detour / straight.max(1.0),
    );

    println!(
        "path segments with an off-mesh sample: {off_mesh} — {deep} deeper than 0.5 m, \
         {grazing} between 0.05 and 0.5 m, the rest touching the edge; worst {depth:.2} m"
    );
    println!(
        "bends at a node: {bends}, cost {:.0} m total ({:.2} m each), \
         straight cut free on the mesh in {free_cuts} of them ({:.1}%)",
        bend_cost,
        bend_cost / bends.max(1) as f64,
        free_cuts as f32 / bends.max(1) as f32 * 100.0,
    );

    let mut hottest: Vec<_> = hot.into_iter().collect();
    hottest.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    println!("\nhottest nodes (grid node -> waypoints):");
    for ((node_x, node_y), count) in hottest.iter().take(10) {
        println!(
            "  ({node_x:>3},{node_y:>3}) at {:>7.0},{:>7.0} m — {count}",
            *node_x as f32 * chunk_size.x,
            *node_y as f32 * chunk_size.y
        );
    }
}

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

fn build_navmesh(map: &MapData) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    let portal =
        snap_portal_position(&navmesh, CITY.portal_hint()).expect("no clear spot for portal");
    navmesh.prune_unreachable(world_to_tile(portal));
    navmesh
}

/// Тот же набор, что у `polymesh_bench`: 80% — к случайному зданию.
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
        let jitter = Vec2::new(rng.random_range(-0.9..0.9), rng.random_range(-0.9..0.9));
        queries.push((tile_center(start) + jitter, tile_center(end)));
    }

    queries
}
