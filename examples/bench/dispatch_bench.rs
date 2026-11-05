//! Офлайн-замер живого диспетчера заявок: стоимость ОДНОГО прохода
//! `dispatch_pathfinding_requests` по полной очереди — гейт видимости, сборка
//! очереди, сортировка, усечение под бюджет — без окна, камеры и расписания
//! игры. Система прогоняется `run_system_once` по миру с N заявками; команды
//! её не применяются (`run_system_once` не снимает deferred-параметры), так
//! что каждая итерация видит одну и ту же полную очередь.
//!
//! Позиции повторяют шторм: часть мирных — кучей у центра кадра (толпа у
//! портала), остальное — равномерно по карте. Отдельный прогон с применением
//! команд снимает, сколько заявок уезжает за один проход (это уже не замер).
//!
//! Профиль — тот же dev, что и у игры, поэтому цифры сравнимы с кадром.
//!
//! ```text
//! cargo run --example dispatch_bench -- [заявок] [срочных] [итераций]
//! ```

use std::sync::{Arc, RwLock};
use std::time::Instant;

use bevy::ecs::system::{IntoSystem, RunSystemOnce, System};
use bevy::math::{IVec2, Vec2};
use bevy::prelude::World;
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use qwe::camera::Viewport;
use qwe::movement::{PathfindingRequest, SimPosition, UrgentPath, dispatch_pathfinding_requests};
use qwe::navigation::{Backend, Navmesh};
use qwe::settings::{MAP_SIZE, MAX_PATHFINDING_IN_FLIGHT, WANDER_DISPATCH_MAX_ZOOM};

/// Куча у центра кадра — доля мирных, собранных у портала в шторме.
const CLUSTER_SHARE: f32 = 0.25;
/// Радиус той кучи в метрах.
const CLUSTER_RADIUS: f32 = 300.0;

/// Сид раскладки позиций: одна и та же при каждом запуске.
const SEED: u64 = 0xBEEF;

/// (подпись, зум): близкий — шторм у камеры; средний — полгорода в кадре;
/// дальний — выше отсечки мирных (`WANDER_DISPATCH_MAX_ZOOM`), остаются
/// только срочные.
const ZOOMS: [(&str, f32); 3] = [("close 0.1", 0.1), ("mid 0.5", 0.5), ("wide 0.8", 0.8)];

/// Номинальное окно 1920×1080: полуразмеры кадра в метрах считаются от него.
const WINDOW_HALF: Vec2 = Vec2::new(960.0, 540.0);

fn main() {
    let mut args = std::env::args().skip(1);
    let pending: usize = args
        .next()
        .map(|value| value.parse().expect("pending must be a number"))
        .unwrap_or(20000);
    let urgent: usize = args
        .next()
        .map(|value| value.parse().expect("urgent must be a number"))
        .unwrap_or(200);
    let iterations: usize = args
        .next()
        .map(|value| value.parse().expect("iterations must be a number"))
        .unwrap_or(60);
    assert!(urgent < pending, "urgent must be a share of pending");

    // пул нужен диспетчеру для спавна тасков; сами таски с очередью команд
    // не применённого `run_system_once` умирают сразу, содержимое поиска в
    // замер не входит
    AsyncComputeTaskPool::get_or_init(TaskPool::default);

    println!(
        "pending: {pending} (urgent {urgent}, {CLUSTER_SHARE} clustered within {CLUSTER_RADIUS} m of centre), iterations: {iterations}\n"
    );

    for (label, zoom) in ZOOMS {
        let mut world = world_for_zoom(zoom, pending, urgent);
        let timings = measure(&mut world, iterations);
        let dispatched = count_dispatched(&mut world, pending);
        print_row(label, zoom, &timings, dispatched, pending);
    }
}

/// Мир одного зума: бэкенд (настоящая сетка карты, вся проходимая —
/// содержимое поиска в замер не входит), кадр и очередь заявок.
fn world_for_zoom(zoom: f32, pending: usize, urgent: usize) -> World {
    let mut world = World::new();
    world.insert_resource(Backend::from_grid(Arc::new(RwLock::new(Navmesh::default()))));
    world.insert_resource(Viewport {
        centre: MAP_SIZE * 0.5,
        half_extent: WINDOW_HALF * zoom,
        zoom,
    });
    let (urgent_pawns, wanderers) = generate_pawns(pending, urgent);
    let _ = world.spawn_batch(urgent_pawns.into_iter()).collect::<Vec<_>>();
    let _ = world.spawn_batch(wanderers.into_iter()).collect::<Vec<_>>();
    world
}

/// Заявки: первые `urgent` — срочные и равномерно по карте (как демоны и
/// бегущие в шторме), остальные мирные — четверть кучей у центра кадра.
fn generate_pawns(
    pending: usize,
    urgent: usize,
) -> (
    Vec<(SimPosition, PathfindingRequest, UrgentPath)>,
    Vec<(SimPosition, PathfindingRequest)>,
) {
    let mut rng = StdRng::seed_from_u64(SEED);
    let centre = MAP_SIZE * 0.5;
    let uniform = |rng: &mut StdRng| {
        Vec2::new(rng.random_range(0.0..MAP_SIZE.x), rng.random_range(0.0..MAP_SIZE.y))
    };
    let mut urgent_pawns = Vec::with_capacity(urgent);
    for _ in 0..urgent {
        let position = uniform(&mut rng);
        urgent_pawns.push((SimPosition(position), request_at(position), UrgentPath));
    }
    let mut wanderers = Vec::with_capacity(pending - urgent);
    for index in 0..pending - urgent {
        let clustered =
            (index as f32 / (pending - urgent) as f32) < CLUSTER_SHARE;
        let position = if clustered {
            let direction = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU));
            centre + direction * CLUSTER_RADIUS * rng.random_range(0.0f32..1.0).sqrt()
        } else {
            uniform(&mut rng)
        };
        wanderers.push((SimPosition(position), request_at(position)));
    }
    (urgent_pawns, wanderers)
}

/// Заявка с тайлами вне границ сетки: содержимое поиска в замер не входит
/// (поиск асинхронный, его цена лежит на пуле), а такой поиск кончается
/// мгновенной проверкой границ — пул не отбирает ядра у замера.
fn request_at(_: Vec2) -> PathfindingRequest {
    PathfindingRequest {
        start_tile: IVec2::splat(-1),
        end_tile: IVec2::splat(-2),
    }
}

fn measure(world: &mut World, iterations: usize) -> Vec<f64> {
    // тело диспетчера без применения команд: очередь заявок и бюджет от
    // итерации к итерации не меняются, `System::run` здесь не годится — он
    // применяет deferred сразу и разбирает очередь
    let mut system = IntoSystem::into_system(dispatch_pathfinding_requests);
    system.initialize(world);
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..5 {
        system
            .run_without_applying_deferred((), world)
            .expect("диспетчер прогоняется");
    }
    for _ in 0..iterations {
        let started = Instant::now();
        system
            .run_without_applying_deferred((), world)
            .expect("диспетчер прогоняется");
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    samples
}

/// Сколько заявок уезжает за один проход: прогон с применением команд и счёт
/// остатка. Мир после этого не годится для замера — этот вызов и последний.
fn count_dispatched(world: &mut World, pending: usize) -> usize {
    world
        .run_system_once(dispatch_pathfinding_requests)
        .expect("диспетчер прогоняется");
    let mut query = world.query::<&PathfindingRequest>();
    let remaining = query.iter(&world).count();
    pending - remaining
}

fn print_row(label: &str, zoom: f32, samples: &[f64], dispatched: usize, pending: usize) {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in timings"));
    let percentile = |share: f64| {
        let index = ((sorted.len() as f64 - 1.0) * share).round() as usize;
        sorted[index]
    };
    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
    println!(
        "{label:<10} p50 {:>8.1} µs | p95 {:>8.1} | max {:>8.1} | avg {:>8.1} | dispatched {:>4}/{:<5} (budget {}) | zoom {zoom:.2} vs cut {WANDER_DISPATCH_MAX_ZOOM}",
        percentile(0.5),
        percentile(0.95),
        sorted.last().copied().unwrap_or_default(),
        avg,
        dispatched,
        pending,
        MAX_PATHFINDING_IN_FLIGHT,
    );
}
