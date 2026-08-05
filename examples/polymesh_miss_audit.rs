//! Почему полигональный поиск отвечает `None`: разбор промахов того же
//! детерминированного набора запросов, что гоняет `polymesh_bench`.
//!
//! Отказ поиска не говорит, что именно не сошлось, а вариантов три, и лечатся
//! они по-разному: конец запроса не садится на меш (цель выбрана по сеточной
//! проходимости, а меш строже — контуры раздуты на радиус агента), либо концы
//! сели, но лежат в разных компонентах связности (двор, замурованный
//! инфляцией), либо оба на месте и в одной компоненте — тогда виноват сам
//! поиск. Счётчик по каждому и есть ответ.
//!
//! ```text
//! cargo run --example polymesh_miss_audit -- [tasks] [agent_radius]
//! cargo run --example polymesh_miss_audit -- 800 1.0
//! ```

use bevy::math::{IVec2, Vec2};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use qwe::city::City;
use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::{
    Navmesh, PolymeshBuild, build_polymesh_from_map, find_passable_tile_near, find_path_polymesh,
    snap_portal_position,
};
use qwe::settings::{HUMAN_WANDER_RANGE, MAP_SIZE};

const CITY: City = City::Tula;
/// Тот же сид, что у `polymesh_bench`: наборы запросов совпадают, и промах
/// здесь — это промах там же, под тем же номером.
const SEED: u64 = 0xC0FFEE;
const WANDER_TO_BUILDING_SHARE: f32 = 0.8;
const MAP_MARGIN: f32 = 4.0;
const JITTER: f32 = 1.0;

/// Куда точка попала относительно меша.
enum Seat {
    /// Стоит на меше — свободна для агента с этим радиусом.
    OnMesh,
    /// Внутри раздутого контура, но свободное место в допуске снапа есть.
    Snapped(f32),
    /// Глубоко в препятствии: снап не достаёт.
    Walled,
}

fn seat(build: &PolymeshBuild, point: Vec2) -> Seat {
    if build.contains(point) {
        return Seat::OnMesh;
    }
    match build.nearest_free_point(point) {
        Some(free) => Seat::Snapped(point.distance(free)),
        None => Seat::Walled,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let tasks: usize = args
        .next()
        .map(|value| value.parse().expect("tasks must be a number"))
        .unwrap_or(800);
    let radius: f32 = args
        .next()
        .map(|value| value.parse().expect("radius must be a number"))
        .unwrap_or(0.4);

    let map = load_map();
    let navmesh = build_navmesh(&map);
    let build = build_polymesh_from_map(&map, radius).expect("build was not cancelled");
    let queries = generate_queries(&map, &navmesh, tasks);
    println!("{} queries, agent radius {radius}\n", queries.len());

    let mut misses = 0;
    let mut start_walled = 0;
    let mut goal_walled = 0;
    let mut start_snapped = 0;
    let mut goal_snapped = 0;
    let mut both_seated = 0;
    // сколько запросов вообще стартуют/целятся не с меша — в том числе среди
    // найденных: снап их спасает, и знать его долю полезнее, чем долю отказов
    let mut off_mesh_starts = 0;
    let mut off_mesh_goals = 0;

    for (index, (from, to)) in queries.iter().enumerate() {
        let start = seat(&build, *from);
        let goal = seat(&build, *to);
        if !matches!(start, Seat::OnMesh) {
            off_mesh_starts += 1;
        }
        if !matches!(goal, Seat::OnMesh) {
            off_mesh_goals += 1;
        }

        if find_path_polymesh(&build, *from, *to).is_some() {
            continue;
        }
        misses += 1;

        let mut reason = Vec::new();
        match start {
            Seat::Walled => {
                start_walled += 1;
                reason.push("start walled in".to_string());
            }
            Seat::Snapped(distance) => {
                start_snapped += 1;
                reason.push(format!("start snapped {distance:.2} m"));
            }
            Seat::OnMesh => {}
        }
        match goal {
            Seat::Walled => {
                goal_walled += 1;
                reason.push("goal walled in".to_string());
            }
            Seat::Snapped(distance) => {
                goal_snapped += 1;
                reason.push(format!("goal snapped {distance:.2} m"));
            }
            Seat::OnMesh => {}
        }
        if reason.is_empty() {
            both_seated += 1;
            reason.push("both on mesh — components apart".to_string());
        }

        println!(
            "[{index:>4}] MISS {:.0} m  {from:?} -> {to:?}  {}",
            from.distance(*to),
            reason.join(", ")
        );
    }

    let percent = |count: usize| count as f32 / queries.len() as f32 * 100.0;
    println!(
        "\nmissed {misses} of {} ({:.1}%)",
        queries.len(),
        percent(misses)
    );
    println!("  start walled in      {start_walled}");
    println!("  start off mesh       {start_snapped}");
    println!("  goal walled in       {goal_walled}");
    println!("  goal off mesh        {goal_snapped}");
    println!("  both seated          {both_seated}");
    println!(
        "off-mesh endpoints over all queries: starts {off_mesh_starts} ({:.1}%), goals {off_mesh_goals} ({:.1}%)",
        percent(off_mesh_starts),
        percent(off_mesh_goals)
    );
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

/// Копия генератора из `polymesh_bench` — тот же поток RNG, те же запросы.
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
        queries.push((tile_center(start) + jitter * JITTER, tile_center(end)));
    }

    queries
}
