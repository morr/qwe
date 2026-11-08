//! Офлайн-замер живого диспетчера заявок: стоимость ОДНОГО прохода
//! `dispatch_pathfinding_requests` по полной очереди — гейт видимости, сборка
//! очереди, сортировка, спавн тасков — без окна, камеры и расписания игры.
//! Тело системы прогоняется `run_without_applying_deferred`: команды не
//! применяются, так что каждая итерация видит одну и ту же полную очередь и
//! полный бюджет (применяющий `System::run` разбрал бы очередь и сдул бы
//! бюджет в ноль за две итерации).
//!
//! Позиции повторяют шторм: часть мирных — кучей у центра кадра (толпа у
//! портала), остальное — равномерно по карте. Поиски — настоящие, на полной
//! карте: насыщенный пул и есть штатное состояние шторма, замер с простаивающим
//! пулом надувает цену спавна (каждый спавн будит поток). Отдельный прогон с
//! применением команд снимает, сколько заявок уезжает за один проход.
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
use qwe::movement::{
    PathfindingRequest, PathfindingTask, QueuedWander, SimPosition, UrgentPath,
    dispatch_pathfinding_requests, on_pathfinding_request_added, on_pathfinding_request_removed,
    on_urgent_path_added, on_urgent_path_removed,
};
use qwe::navigation::{Backend, Navmesh};
use qwe::settings::{MAP_SIZE, MAX_PATHFINDING_IN_FLIGHT, WANDER_DISPATCH_MAX_ZOOM};
use qwe::spatial::SpatialGrid;

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

    // пул нужен диспетчеру для спавна настоящих поисков — они и насыщают его,
    // как в игре; их цена лежит на пуле, в замер основного потока не входит
    AsyncComputeTaskPool::get_or_init(TaskPool::default);

    println!(
        "pending: {pending} (urgent {urgent}, {CLUSTER_SHARE} clustered within {CLUSTER_RADIUS} m of centre), iterations: {iterations}\n"
    );

    for (label, zoom) in ZOOMS {
        let mut world = world_for_zoom(zoom, pending, urgent);
        let timings = measure(&mut world, iterations);
        let (dispatched, flush_us) = count_dispatched(&mut world, pending);
        print_row(label, zoom, &timings, dispatched, flush_us, pending);
    }
}

/// Мир одного зума: бэкенд (настоящая сетка карты, вся проходимая —
/// содержимое поиска в замер не входит), кадр и очередь заявок.
fn world_for_zoom(zoom: f32, pending: usize, urgent: usize) -> World {
    let mut world = World::new();
    world.insert_resource(Backend::from_grid(Arc::new(
        RwLock::new(Navmesh::default()),
    )));
    world.insert_resource(Viewport {
        centre: MAP_SIZE * 0.5,
        half_extent: WINDOW_HALF * zoom,
        zoom,
    });
    // сетка очереди и её observers — как их поднимает MovementPlugin
    world.init_resource::<SpatialGrid<QueuedWander>>();
    world.add_observer(on_pathfinding_request_added);
    world.add_observer(on_pathfinding_request_removed);
    world.add_observer(on_urgent_path_added);
    world.add_observer(on_urgent_path_removed);
    let (urgent_pawns, wanderers) = generate_pawns(pending, urgent);
    let _ = world.spawn_batch(urgent_pawns).collect::<Vec<_>>();
    let _ = world.spawn_batch(wanderers).collect::<Vec<_>>();
    // observers раскладывают заявки по сетке очереди
    world.flush();
    let mut entries = 0;
    world
        .resource::<SpatialGrid<QueuedWander>>()
        .for_each_in_rect(Vec2::ZERO, MAP_SIZE, |_| entries += 1);
    println!(
        "grid entries after flush: {entries} (wanderers {})",
        pending - urgent
    );
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
        Vec2::new(
            rng.random_range(0.0..MAP_SIZE.x),
            rng.random_range(0.0..MAP_SIZE.y),
        )
    };
    let mut urgent_pawns = Vec::with_capacity(urgent);
    for _ in 0..urgent {
        let position = uniform(&mut rng);
        urgent_pawns.push((SimPosition(position), request_at(position), UrgentPath));
    }
    let mut wanderers = Vec::with_capacity(pending - urgent);
    for index in 0..pending - urgent {
        let clustered = (index as f32 / (pending - urgent) as f32) < CLUSTER_SHARE;
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

/// Настоящие тайлы заявки: содержимое поиска не входит в замер (оно лежит на
/// пуле), но поиски должны быть настоящими — насыщенный пул, как в игре в
/// шторме, иначе каждый спавн таска будит простаивающий поток, и цена спавна
/// в замере надувается в разы против реальной.
fn request_at(position: Vec2) -> PathfindingRequest {
    let tile = IVec2::new((position.x / 2.0) as i32, (position.y / 2.0) as i32);
    PathfindingRequest {
        start_tile: tile,
        end_tile: tile + IVec2::new(20, 10),
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
/// остатка. Заодно замер применения команд (флеша): выдача и её снятие —
/// переносы между архетипами, вторая половина цены шторма входа рядом с
/// самим проходом. У пешек бенча архетип из двух-трёх компонент — копировать
/// при переносе в разы меньше, чем у игровой (~20+), поэтому абсолютное
/// значение флеша ниже игрового; сравнимы фазы прогон с прогоном. Мир после
/// этого не годится для замера — этот вызов и последний.
fn count_dispatched(world: &mut World, pending: usize) -> (usize, f64) {
    let started = Instant::now();
    world
        .run_system_once(dispatch_pathfinding_requests)
        .expect("диспетчер прогоняется");
    world.flush();
    let flush_us = started.elapsed().as_secs_f64() * 1_000_000.0;
    // «осталось в очереди» — заявка без таска: живой диспетчер ставит таск
    // поверх живой заявки, снимает пару приёмка
    let mut query =
        world.query_filtered::<&PathfindingRequest, bevy::ecs::query::Without<PathfindingTask>>();
    let remaining = query.iter(world).count();
    (pending - remaining, flush_us)
}

fn print_row(
    label: &str,
    zoom: f32,
    samples: &[f64],
    dispatched: usize,
    flush_us: f64,
    pending: usize,
) {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in timings"));
    let percentile = |share: f64| {
        let index = ((sorted.len() as f64 - 1.0) * share).round() as usize;
        sorted[index]
    };
    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
    println!(
        "{label:<10} p50 {:>8.1} µs | p95 {:>8.1} | max {:>8.1} | avg {:>8.1} | dispatched {:>4}/{:<5} (budget {}) | flush {:>7.1} µs | zoom {zoom:.2} vs cut {WANDER_DISPATCH_MAX_ZOOM}",
        percentile(0.5),
        percentile(0.95),
        sorted.last().copied().unwrap_or_default(),
        avg,
        dispatched,
        pending,
        MAX_PATHFINDING_IN_FLIGHT,
        flush_us,
    );
}
