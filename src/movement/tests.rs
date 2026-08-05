//! Спасение застрявших: инвариант «никто не стоит в непроходимом».

use std::sync::{Arc, RwLock};
use std::time::Duration;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};

use super::components::{Movable, MovableState, PathfindingTask, PreviousSimPosition, SimPosition};
use super::systems::{listen_for_pathfinding_tasks, rescue_trapped_entities};
use crate::grid::{tile_center, world_to_tile};
use crate::navigation::{
    ArcNavmesh, Navmesh, NorthstarGrid, PathfindingAlgorithm, PathfindingResult, PolyNavmesh,
    PolymeshDebug,
};
use crate::settings::RESCUE_SEARCH_TILES;

/// Квартал непроходимых тайлов `[min, max]` (включительно) на пустой сетке.
fn navmesh_with_block(min: IVec2, max: IVec2) -> ArcNavmesh {
    let mut navmesh = Navmesh::default();
    for x in min.x..=max.x {
        for y in min.y..=max.y {
            navmesh.set_passable(x, y, false);
        }
    }
    ArcNavmesh(Arc::new(RwLock::new(navmesh)))
}

/// Приложение с непроходимым кварталом в центре карты. Ресурсы — те, из
/// которых собирается `Pathfinder`: полигонального меша в тестах нет, так что
/// «свободно» здесь меряется одной сеткой.
fn app_with(navmesh: ArcNavmesh) -> App {
    let mut app = App::new();
    app.insert_resource(navmesh)
        .init_resource::<PathfindingAlgorithm>()
        .init_resource::<NorthstarGrid>()
        .init_resource::<PolyNavmesh>()
        .init_resource::<PolymeshDebug>();
    app
}

/// Квартал в центре карты — общий для всех проверок.
fn app_with_block() -> App {
    app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ))
}

fn spawn_pawn(app: &mut App, position: Vec2) -> Entity {
    app.world_mut()
        .spawn((
            Movable::new(1.0),
            SimPosition(position),
            PreviousSimPosition(position),
        ))
        .id()
}

fn position_of(app: &App, pawn: Entity) -> Vec2 {
    app.world().get::<SimPosition>(pawn).expect("SimPosition").0
}

fn assert_on_passable_tile(app: &App, pawn: Entity) {
    let tile = world_to_tile(position_of(app, pawn));
    assert!(
        app.world()
            .resource::<ArcNavmesh>()
            .read()
            .is_passable(tile.x, tile.y),
        "пешка осталась в непроходимом: {tile}"
    );
}

// --- разовый скан на входе в мир ---

#[test]
fn the_spawn_scan_moves_a_pawn_out_of_an_impassable_tile() {
    let app = &mut app_with_block();
    app.add_systems(Update, rescue_trapped_entities);
    let pawn = spawn_pawn(app, tile_center(IVec2::new(105, 105)));

    app.update();

    assert_on_passable_tile(app, pawn);
}

/// Спасение — переезд на **ближайший** свободный тайл, а не куда-нибудь:
/// пешку у стены нельзя выкидывать через весь квартал.
#[test]
fn the_rescued_pawn_lands_on_the_nearest_free_tile() {
    let app = &mut app_with_block();
    app.add_systems(Update, rescue_trapped_entities);
    // тайл у самого края квартала: свободный сосед — ровно один шаг на запад
    let pawn = spawn_pawn(app, tile_center(IVec2::new(100, 105)));

    app.update();

    assert_eq!(
        world_to_tile(position_of(app, pawn)),
        IVec2::new(99, 105),
        "переехал не к ближайшему свободному тайлу"
    );
}

/// Стоящего на проходимом скан не трогает.
#[test]
fn a_pawn_on_a_passable_tile_stays_where_it_is() {
    let app = &mut app_with_block();
    app.add_systems(Update, rescue_trapped_entities);
    let position = tile_center(IVec2::new(50, 50));
    let pawn = spawn_pawn(app, position);

    app.update();

    assert_eq!(position_of(app, pawn), position);
}

/// Переезд обязан обновить оба конца интерполяции и сбросить путь: старый
/// путь ведёт из места, где сущности больше нет.
#[test]
fn the_rescue_syncs_the_interpolation_and_drops_the_stale_path() {
    let app = &mut app_with_block();
    app.add_systems(Update, rescue_trapped_entities);
    let pawn = spawn_pawn(app, tile_center(IVec2::new(105, 105)));
    {
        let mut movable = app.world_mut().get_mut::<Movable>(pawn).expect("Movable");
        movable.state = MovableState::Moving(IVec2::new(200, 200));
        movable.path = [tile_center(IVec2::new(200, 200))].into();
    }

    app.update();

    let position = position_of(app, pawn);
    let previous = app
        .world()
        .get::<PreviousSimPosition>(pawn)
        .expect("PreviousSimPosition")
        .0;
    assert_eq!(
        previous, position,
        "интерполяция протянет пешку через город"
    );

    let movable = app.world().get::<Movable>(pawn).expect("Movable");
    assert_eq!(movable.state, MovableState::Idle);
    assert!(movable.path.is_empty(), "остался путь из старого места");
}

/// Кольцевой поиск ограничен `RESCUE_SEARCH_TILES`: из середины квартала
/// заведомо больше радиуса пешка не телепортируется — такой прыжок увёл бы её
/// дальше, чем она вообще могла бы дойти.
#[test]
fn a_pawn_deeper_than_the_search_radius_is_left_alone() {
    let side = RESCUE_SEARCH_TILES * 3;
    let mut app = app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(100 + side, 100 + side),
    ));
    app.add_systems(Update, rescue_trapped_entities);
    let position = tile_center(IVec2::new(100 + side / 2, 100 + side / 2));
    let pawn = spawn_pawn(&mut app, position);

    app.update();

    assert_eq!(position_of(&app, pawn), position);
}

// --- спасение по провалу поиска ---

/// Готовый ответ поиска для сущности: `None` — поиск не нашёл пути.
fn answer(app: &mut App, pawn: Entity, end_tile: IVec2, path: Option<Vec<Vec2>>) {
    AsyncComputeTaskPool::get_or_init(TaskPool::default);
    let start_tile = world_to_tile(position_of(app, pawn));
    let task = AsyncComputeTaskPool::get().spawn(async move {
        PathfindingResult {
            path,
            start_tile,
            end_tile,
            duration: Duration::ZERO,
        }
    });
    let mut entity = app.world_mut().entity_mut(pawn);
    entity.get_mut::<Movable>().expect("Movable").state = MovableState::Pathfinding(end_tile);
    entity.insert(PathfindingTask(task));
}

/// Ответ считается в другом потоке: крутим кадры, пока приёмка его не снимет.
fn run_until_answered(app: &mut App, pawn: Entity) {
    for _ in 0..1000 {
        app.update();
        if app.world().get::<PathfindingTask>(pawn).is_none() {
            return;
        }
    }
    panic!("ответ поиска так и не снят");
}

fn app_with_listener() -> App {
    let mut app = app_with_block();
    app.init_resource::<bevy::diagnostic::DiagnosticsStore>();
    app.add_systems(Update, listen_for_pathfinding_tasks);
    app
}

/// Главный случай: пешка стоит в непроходимом, поиск оттуда пути не находит —
/// и именно провал выводит её наружу.
#[test]
fn a_failed_search_from_an_impassable_tile_rescues_the_pawn() {
    let app = &mut app_with_listener();
    let pawn = spawn_pawn(app, tile_center(IVec2::new(105, 105)));
    answer(app, pawn, IVec2::new(200, 200), None);

    run_until_answered(app, pawn);

    assert_on_passable_tile(app, pawn);
    let movable = app.world().get::<Movable>(pawn).expect("Movable");
    assert_eq!(
        movable.state,
        MovableState::Idle,
        "после спасения цель выбирается заново"
    );
}

/// Провал у пешки на проходимом тайле — это недостижимая цель, а не
/// застревание: позиция не трогается, состояние — `PathfindingError`.
#[test]
fn a_failed_search_from_a_passable_tile_moves_nobody() {
    let app = &mut app_with_listener();
    let position = tile_center(IVec2::new(50, 50));
    let pawn = spawn_pawn(app, position);
    let end_tile = IVec2::new(200, 200);
    answer(app, pawn, end_tile, None);

    run_until_answered(app, pawn);

    assert_eq!(position_of(app, pawn), position);
    let movable = app.world().get::<Movable>(pawn).expect("Movable");
    assert_eq!(movable.state, MovableState::PathfindingError(end_tile));
}

/// Успешный ответ спасение не запускает вовсе — путь принимается как обычно.
#[test]
fn a_successful_answer_is_taken_as_a_path() {
    let app = &mut app_with_listener();
    let position = tile_center(IVec2::new(50, 50));
    let pawn = spawn_pawn(app, position);
    let end_tile = IVec2::new(52, 50);
    let path = vec![
        position,
        tile_center(IVec2::new(51, 50)),
        tile_center(end_tile),
    ];
    answer(app, pawn, end_tile, Some(path));

    run_until_answered(app, pawn);

    assert_eq!(position_of(app, pawn), position);
    let movable = app.world().get::<Movable>(pawn).expect("Movable");
    assert_eq!(movable.state, MovableState::Moving(end_tile));
    assert_eq!(movable.path.len(), 2);
}
