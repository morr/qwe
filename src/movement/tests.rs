//! Спасение застрявших: инвариант «никто не стоит в непроходимом».

use std::sync::{Arc, RwLock};

use bevy::prelude::*;

use super::components::{Movable, MovableState, PreviousSimPosition, SimPosition};
use super::systems::{rescue_scan_due, rescue_trapped_entities};
use crate::grid::{tile_center, world_to_tile};
use crate::navigation::{ArcNavmesh, Navmesh};
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

/// Приложение без плагинов: скан гоняется вручную в `Update`, как он стоит в
/// `FixedUpdate` живого мира — вместе со своим условием периодичности.
fn app_with(navmesh: ArcNavmesh) -> App {
    let mut app = App::new();
    app.insert_resource(navmesh);
    app.add_systems(Update, rescue_trapped_entities.run_if(rescue_scan_due));
    app
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

#[test]
fn a_pawn_standing_in_an_impassable_tile_is_moved_to_a_passable_one() {
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ));
    let pawn = spawn_pawn(app, tile_center(IVec2::new(105, 105)));

    app.update();

    let tile = world_to_tile(position_of(app, pawn));
    assert!(
        app.world()
            .resource::<ArcNavmesh>()
            .read()
            .is_passable(tile.x, tile.y),
        "пешка осталась в непроходимом: {tile}"
    );
}

/// Спасение — переезд на **ближайший** свободный тайл, а не куда-нибудь:
/// пешку у стены нельзя выкидывать через весь квартал.
#[test]
fn the_rescued_pawn_lands_on_the_nearest_free_tile() {
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ));
    // тайл у самого края квартала: свободный сосед — ровно один шаг на запад
    let pawn = spawn_pawn(app, tile_center(IVec2::new(100, 105)));

    app.update();

    assert_eq!(
        world_to_tile(position_of(app, pawn)),
        IVec2::new(99, 105),
        "переехал не к ближайшему свободному тайлу"
    );
}

/// Стоящего на проходимом скан не трогает — иначе «спасение» дёргало бы
/// каждую секунду всё население.
#[test]
fn a_pawn_on_a_passable_tile_stays_where_it_is() {
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ));
    let position = tile_center(IVec2::new(50, 50));
    let pawn = spawn_pawn(app, position);

    app.update();

    assert_eq!(position_of(app, pawn), position);
}

/// Переезд обязан обновить оба конца интерполяции и сбросить путь: старый
/// путь ведёт из места, где сущности больше нет.
#[test]
fn the_rescue_syncs_the_interpolation_and_drops_the_stale_path() {
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ));
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
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(100 + side, 100 + side),
    ));
    let position = tile_center(IVec2::new(100 + side / 2, 100 + side / 2));
    let pawn = spawn_pawn(app, position);

    app.update();

    assert_eq!(position_of(app, pawn), position);
}

/// Скан идёт не каждый шаг (см. `RESCUE_SCAN_STEPS`): застрявший, появившийся
/// сразу после скана, ждёт следующего.
#[test]
fn the_scan_runs_on_an_interval_not_every_step() {
    let app = &mut app_with(navmesh_with_block(
        IVec2::new(100, 100),
        IVec2::new(110, 110),
    ));
    spawn_pawn(app, tile_center(IVec2::new(105, 105)));
    app.update();

    let position = tile_center(IVec2::new(104, 104));
    let late = spawn_pawn(app, position);
    app.update();

    assert_eq!(position_of(app, late), position);
}
