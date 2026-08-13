//! Офлайн-замер скорости алгоритмов поиска пути на одном и том же наборе задач.
//!
//! Bevy-приложение не поднимается: карта читается из кеша Overpass, navmesh
//! заполняется и прунится ровно так же, как в `OnEnter(AppState::Playing)`,
//! после чего каждый алгоритм прогоняет ОДИН И ТОТ ЖЕ список задач
//! (стартовые тайлы и цели генерируются один раз из сида).
//!
//! Профиль — тот же dev, что и у игры, поэтому цифры сравнимы с телеметрией
//! в окне. Полный прогон всех шести алгоритмов на 1000 задач — около 1.5 мин.
//!
//! ```text
//! cargo run --example pathfinding_bench -- [tasks] [threads] [алгоритмы,через,запятую]
//! cargo run --example pathfinding_bench -- 1000 8 astar,hpa
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bevy::math::{IVec2, Vec2};
use bevy_northstar::prelude::OrdinalGrid;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use qwe::city::City;
use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::MapData;
use qwe::navigation::{
    Navmesh, PathfindingAlgorithm, build_from_navmesh, find_passable_tile_near, find_path,
    find_path_northstar, snap_portal_position,
};
use qwe::settings::{HUMAN_COUNT, HUMAN_WANDER_RANGE, MAP_SIZE};

/// Бенч гоняется по карте города по умолчанию — той же, что видит игра при
/// первом запуске.
const CITY: City = City::Tula;

/// Сид генератора задач: один и тот же набор при каждом запуске.
const SEED: u64 = 0xC0FFEE;
/// Доля целей «к случайному зданию» — как в `human::pick_wander_targets`.
const WANDER_TO_BUILDING_SHARE: f32 = 0.8;
/// Отступ целей блуждания от края карты, м.
const MAP_MARGIN: f32 = 4.0;

const ALL_ALGORITHMS: [PathfindingAlgorithm; 6] = [
    PathfindingAlgorithm::Astar,
    PathfindingAlgorithm::Dijkstra,
    PathfindingAlgorithm::Fringe,
    PathfindingAlgorithm::Bfs,
    PathfindingAlgorithm::Hpa,
    PathfindingAlgorithm::ThetaStar,
];

#[derive(Clone, Copy)]
struct Task {
    start: IVec2,
    end: IVec2,
}

struct Sample {
    duration: Duration,
    /// Длина найденного пути в метрах; `None` — путь не найден.
    length: Option<f32>,
}

struct Report {
    algorithm: PathfindingAlgorithm,
    wall: Duration,
    cpu: Duration,
    found: usize,
    total: usize,
    avg: f64,
    p50: f64,
    p95: f64,
    max: f64,
    path_length: f64,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let task_count: usize = args
        .next()
        .map(|value| value.parse().expect("tasks must be a number"))
        .unwrap_or(HUMAN_COUNT);
    let threads: usize = args
        .next()
        .map(|value| value.parse().expect("threads must be a number"))
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(8)
        });
    let algorithms: Vec<PathfindingAlgorithm> = match args.next() {
        Some(list) => list.split(',').map(parse_algorithm).collect(),
        None => ALL_ALGORITHMS.to_vec(),
    };

    let map = common::load_map(CITY);
    println!(
        "map: {} buildings, {} roads, {} water, {} parks",
        map.buildings.len(),
        map.roads.len(),
        map.water.len(),
        map.parks.len()
    );

    let navmesh = build_navmesh(&map);
    let tasks = Arc::new(generate_tasks(&map, &navmesh, task_count));
    println!(
        "tasks: {} (seed {SEED:#x}), threads: {threads}\n",
        tasks.len()
    );

    let navmesh = Arc::new(navmesh);
    // иерархическая сетка нужна только HPA*/Theta*, её постройка — разовая
    // стартовая цена, в замер прогона она не входит
    let grid = algorithms
        .iter()
        .any(|algorithm| is_hierarchical(*algorithm))
        .then(|| {
            let started = Instant::now();
            let grid = Arc::new(build_from_navmesh(&navmesh));
            println!("northstar grid built in {:?}\n", started.elapsed());
            grid
        });

    let mut reports = Vec::new();
    for algorithm in algorithms {
        let report = run(algorithm, &navmesh, grid.as_ref(), &tasks, threads);
        print_row(&report);
        reports.push(report);
    }

    print_table(&reports, task_count, threads);
}

fn parse_algorithm(name: &str) -> PathfindingAlgorithm {
    match name.trim().to_ascii_lowercase().as_str() {
        "astar" | "a*" => PathfindingAlgorithm::Astar,
        "dijkstra" => PathfindingAlgorithm::Dijkstra,
        "fringe" => PathfindingAlgorithm::Fringe,
        "bfs" => PathfindingAlgorithm::Bfs,
        "hpa" | "hpa*" => PathfindingAlgorithm::Hpa,
        "theta" | "thetastar" | "theta*" => PathfindingAlgorithm::ThetaStar,
        other => panic!("unknown algorithm: {other}"),
    }
}

fn is_hierarchical(algorithm: PathfindingAlgorithm) -> bool {
    matches!(
        algorithm,
        PathfindingAlgorithm::Hpa | PathfindingAlgorithm::ThetaStar
    )
}

/// Та же последовательность, что и в `NavigationPlugin`: заливка, снап портала,
/// прунинг недостижимого.
fn build_navmesh(map: &MapData) -> Navmesh {
    let mut navmesh = Navmesh::default();

    let started = Instant::now();
    navmesh.fill_from_mapdata(map);
    println!("navmesh filled in {:?}", started.elapsed());

    let portal =
        snap_portal_position(&navmesh, CITY.portal_hint()).expect("no clear spot for portal");
    let started = Instant::now();
    let pruned = navmesh.prune_unreachable(world_to_tile(portal));
    println!(
        "navmesh: pruned {pruned} unreachable tiles in {:?}",
        started.elapsed()
    );

    let passable = (0..navmesh.grid_size.x)
        .flat_map(|x| (0..navmesh.grid_size.y).map(move |y| (x, y)))
        .filter(|&(x, y)| navmesh.is_passable(x, y))
        .count();
    println!("navmesh: {passable} passable tiles, portal at {portal:?}");

    navmesh
}

/// Задачи повторяют нагрузку мирного блуждания: старт — случайный проходимый
/// тайл (как спавн человека), цель — 80% случайное здание города, 20% прогулка
/// в 20–40 м. Берутся только задачи с проходимыми концами, чтобы каждый
/// алгоритм делал одинаковую работу, а не отваливался на невалидной цели.
fn generate_tasks(map: &MapData, navmesh: &Navmesh, count: usize) -> Vec<Task> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut tasks = Vec::with_capacity(count);

    while tasks.len() < count {
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
        tasks.push(Task { start, end });
    }

    tasks
}

/// Прогон одного алгоритма по всему списку задач в `threads` потоках.
/// Общий атомарный курсор вместо нарезки на равные куски: длины поисков
/// различаются на порядки, статическая нарезка перекосила бы wall-clock.
fn run(
    algorithm: PathfindingAlgorithm,
    navmesh: &Arc<Navmesh>,
    grid: Option<&Arc<OrdinalGrid>>,
    tasks: &Arc<Vec<Task>>,
    threads: usize,
) -> Report {
    let cursor = AtomicUsize::new(0);
    let started = Instant::now();

    let samples: Vec<Sample> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let cursor = &cursor;
                scope.spawn(move || {
                    let mut local = Vec::new();
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(task) = tasks.get(index) else {
                            break;
                        };
                        let started = Instant::now();
                        let path = if is_hierarchical(algorithm) {
                            find_path_northstar(
                                grid.expect("hierarchical run without a grid"),
                                task.start,
                                task.end,
                                algorithm == PathfindingAlgorithm::ThetaStar,
                            )
                        } else {
                            find_path(navmesh, task.start, task.end, algorithm)
                        };
                        local.push(Sample {
                            duration: started.elapsed(),
                            length: path.as_deref().map(path_length),
                        });
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("worker panicked"))
            .collect()
    });

    let wall = started.elapsed();
    summarize(algorithm, wall, samples)
}

/// Длина пути в метрах — по центрам тайлов, диагональ считается честно.
fn path_length(path: &[IVec2]) -> f32 {
    path.windows(2)
        .map(|pair| {
            let delta = (pair[1] - pair[0]).as_vec2();
            delta.length() * qwe::settings::navtile_size()
        })
        .sum()
}

fn summarize(algorithm: PathfindingAlgorithm, wall: Duration, samples: Vec<Sample>) -> Report {
    let total = samples.len();
    let cpu: Duration = samples.iter().map(|sample| sample.duration).sum();

    let mut milliseconds: Vec<f64> = samples
        .iter()
        .map(|sample| sample.duration.as_secs_f64() * 1000.0)
        .collect();
    milliseconds.sort_unstable_by(|a, b| a.partial_cmp(b).expect("no NaN in durations"));

    let percentile = |share: f64| {
        if milliseconds.is_empty() {
            return 0.0;
        }
        let index = ((milliseconds.len() as f64 - 1.0) * share).round() as usize;
        milliseconds[index]
    };

    let lengths: Vec<f32> = samples.iter().filter_map(|sample| sample.length).collect();
    let path_length = if lengths.is_empty() {
        0.0
    } else {
        lengths.iter().map(|&value| value as f64).sum::<f64>() / lengths.len() as f64
    };

    Report {
        algorithm,
        wall,
        cpu,
        found: lengths.len(),
        total,
        avg: if total == 0 {
            0.0
        } else {
            cpu.as_secs_f64() * 1000.0 / total as f64
        },
        p50: percentile(0.5),
        p95: percentile(0.95),
        max: milliseconds.last().copied().unwrap_or_default(),
        path_length,
    }
}

fn print_row(report: &Report) {
    println!(
        "{:<10} wall {:>8.2} s | cpu {:>9.2} s | avg {:>8.2} ms | p50 {:>8.2} | p95 {:>9.2} | max {:>9.2} | found {:>5}/{:<5} | path {:>7.0} m",
        report.algorithm.label(),
        report.wall.as_secs_f64(),
        report.cpu.as_secs_f64(),
        report.avg,
        report.p50,
        report.p95,
        report.max,
        report.found,
        report.total,
        report.path_length,
    );
}

fn print_table(reports: &[Report], task_count: usize, threads: usize) {
    println!("\n{task_count} tasks, {threads} threads\n");
    println!(
        "| {:<9} | {:>9} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>7} | {:>9} |",
        "algorithm",
        "wall, s",
        "cpu, s",
        "avg, ms",
        "p50, ms",
        "p95, ms",
        "max, ms",
        "found",
        "path, m"
    );
    println!(
        "|{:-<11}|{:->11}|{:->12}|{:->12}|{:->12}|{:->12}|{:->12}|{:->9}|{:->11}|",
        "", "", "", "", "", "", "", "", ""
    );
    for report in reports {
        println!(
            "| {:<9} | {:>9.2} | {:>10.1} | {:>10.2} | {:>10.2} | {:>10.2} | {:>10.2} | {:>7} | {:>9.0} |",
            report.algorithm.label(),
            report.wall.as_secs_f64(),
            report.cpu.as_secs_f64(),
            report.avg,
            report.p50,
            report.p95,
            report.max,
            report.found,
            report.path_length,
        );
    }
}
