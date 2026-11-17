//! Приёмка ответа — проверяется значениями, без `App`: отброс устаревшего,
//! спасение застрявших, приход на месте и подрезка пути под уже пройденное
//! раньше держались только целым приложением.
//!
//! Реквизит минимальный: мир с тремя компонентами пешки, счётчик — системный
//! параметр через `SystemState`, бэкенд — сетка с перечисленными
//! непроходимыми тайлами (как у юнит-тестов шага).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use bevy::ecs::system::SystemState;

use super::*;
use crate::human::Human;
use crate::movement::components::{
    Movable, MovableReachedDestinationEvent, NeedsWanderTarget, PreviousSimPosition, SimPosition,
};
use crate::navigation::{Backend, Navmesh};
use crate::spatial::SpatialGrid;

/// Бэкенд с перечисленными непроходимыми тайлами; навмеш возвращается рядом —
/// из него считаются центры тайлов для позиций пешки.
fn backend(blocked: &[IVec2]) -> (Backend, Arc<RwLock<Navmesh>>) {
    let navmesh = Arc::new(RwLock::new(Navmesh::default()));
    for tile in blocked {
        navmesh.write().unwrap().set_passable(tile.x, tile.y, false);
    }
    (Backend::from_grid(navmesh.clone()), navmesh)
}

/// Ответ поиска: путь и цель; длительность приёмке безразлична.
fn result(end_tile: IVec2, path: Option<Vec<Vec2>>) -> PathfindingResult {
    PathfindingResult {
        path,
        end_tile,
        duration: Duration::ZERO,
    }
}

/// Пешка, ждущая ответ на поиск пути к `target`.
fn pathfinding_pawn(target: IVec2, path: &[Vec2]) -> Movable {
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::Pathfinding(target);
    movable.path = path.iter().copied().collect();
    movable
}

/// Мир с одной пешкой: компоненты вынимаются из мира в прогоне, ресурс
/// диагностик обязателен — его просит счётчик приёмки.
fn pawn_world(position: Vec2, movable: Movable) -> (World, Entity) {
    let mut world = World::new();
    let entity = world
        .spawn((movable, SimPosition(position), PreviousSimPosition(position)))
        .id();
    world.insert_resource(bevy::diagnostic::DiagnosticsStore::default());
    (world, entity)
}

/// Прогон приёмки: компоненты вынимаются, ответ применяется, команды
/// применяются обратно в мир; пешка возвращается на руки для проверок.
fn accept(
    world: &mut World,
    entity: Entity,
    backend: &Backend,
    answer: PathfindingResult,
    is_human: bool,
) -> (Movable, SimPosition, PreviousSimPosition) {
    let mut movable = world.entity_mut(entity).take::<Movable>().unwrap();
    let mut sim_position = world.entity_mut(entity).take::<SimPosition>().unwrap();
    let mut previous = world.entity_mut(entity).take::<PreviousSimPosition>().unwrap();

    let mut params = SystemState::<(
        bevy::diagnostic::Diagnostics,
        Commands,
        Option<ResMut<SpatialGrid<Human>>>,
    )>::new(world);
    let (mut diagnostics, mut commands, mut grid) = params.get_mut(world).unwrap();
    let mut tally = AnswerTally::new(&mut diagnostics);
    accept_answer(
        answer,
        &mut tally,
        PawnEdit {
            entity,
            movable: &mut movable,
            sim_position: &mut sim_position,
            previous: &mut previous,
            commands: &mut commands,
        },
        is_human,
        &mut None,
        backend,
        &mut grid,
    );
    drop(tally);
    drop(diagnostics);
    drop(commands);
    drop(grid);
    params.apply(world);
    (movable, sim_position, previous)
}

// --- ответ, с которым нечего делать ---

/// Устаревший ответ — уже запрошена другая цель: пешка не тронута, маркер
/// новой цели не встаёт.
#[test]
fn a_stale_answer_changes_nothing() {
    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(
        Vec2::ZERO,
        pathfinding_pawn(IVec2::new(5, 0), &[Vec2::new(1.0, 0.0)]),
    );

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(9, 9), Some(vec![Vec2::new(2.0, 0.0)])),
        false,
    );

    assert_eq!(
        movable.state,
        MovableState::Pathfinding(IVec2::new(5, 0)),
        "цель пешки не подменена чужим ответом"
    );
    assert_eq!(movable.path.len(), 1, "путь не подменён");
}

/// Ответ пешке, которая уже не ждёт его (состояние ушло вперёд), — тоже
/// нечего применять.
#[test]
fn an_answer_for_a_pawn_that_already_moved_on_is_ignored() {
    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(Vec2::ZERO, Movable::new(1.0));

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(5, 0), Some(vec![Vec2::new(1.0, 0.0)])),
        false,
    );

    assert_eq!(movable.state, MovableState::Idle, "пешка не тронута");
    assert!(movable.path.is_empty());
}

// --- провал поиска ---

/// Пешка стоит на непроходимом: провал — спасение, а не отказ цели. Телепорт
/// поправляет оба конца интерполяции, пешка возвращается в простой.
#[test]
fn a_failed_search_rescues_a_pawn_standing_on_impassable() {
    let (backend, navmesh) = backend(&[IVec2::new(2, 2)]);
    let stuck = navmesh.read().unwrap().tile_center(IVec2::new(2, 2));
    let (mut world, entity) = pawn_world(
        stuck,
        pathfinding_pawn(IVec2::new(5, 0), &[stuck]),
    );

    let (movable, sim_position, previous) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(5, 0), None),
        false,
    );

    assert_ne!(sim_position.0, stuck, "пешка вытащена с непроходимого тайла");
    assert_eq!(previous.0, sim_position.0, "оба конца интерполяции поправлены");
    assert!(backend.walkable().allows(sim_position.0), "новое место проходимо");
    assert_eq!(movable.state, MovableState::Idle, "спасение кончается простоем");
    assert!(movable.path.is_empty());
    assert!(world.get::<NeedsWanderTarget>(entity).is_some());
}

/// Спасённый человек попадает в человеческую сетку по новому месту — сетка
/// держит позиции людей, и расхождение с `SimPosition` ломало бы расталкивание.
#[test]
fn a_rescued_human_is_reinserted_into_the_human_grid() {
    let (backend, navmesh) = backend(&[IVec2::new(2, 2)]);
    let stuck = navmesh.read().unwrap().tile_center(IVec2::new(2, 2));
    let (mut world, entity) = pawn_world(
        stuck,
        pathfinding_pawn(IVec2::new(5, 0), &[stuck]),
    );
    world.insert_resource(SpatialGrid::<Human>::default());

    let (_, sim_position, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(5, 0), None),
        true,
    );

    assert!(
        world
            .resource::<SpatialGrid<Human>>()
            .any_in_cells_around(sim_position.0, 50.0),
        "сетка знает нового места спасённого"
    );
}

/// Пешка на проходимом, цель недостижима — это отказ цели: состояние
/// `PathfindingError`, позиция не тронута, новую цель выберет поведение.
#[test]
fn an_unreachable_target_is_a_pathfinding_error() {
    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(
        Vec2::ZERO,
        pathfinding_pawn(IVec2::new(5, 0), &[Vec2::ZERO]),
    );

    let (movable, sim_position, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(5, 0), None),
        false,
    );

    assert_eq!(
        movable.state,
        MovableState::PathfindingError(IVec2::new(5, 0))
    );
    assert_eq!(sim_position.0, Vec2::ZERO, "спасение не трогает стоящего на месте");
    assert!(world.get::<NeedsWanderTarget>(entity).is_some());
}

// --- приход ---

/// Ответ из одной точки — пешка уже на месте: путь снимается ДО простоя, и
/// событие прихода доезжает (без снятия пути `to_idle` теряет его молча).
#[test]
fn arriving_on_the_spot_publishes_the_arrival_and_clears_the_path() {
    #[derive(Resource, Default)]
    struct Arrived(Option<IVec2>);

    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(
        Vec2::ZERO,
        pathfinding_pawn(IVec2::new(5, 0), &[Vec2::ZERO]),
    );
    world.init_resource::<Arrived>();
    world.add_observer(|ev: On<MovableReachedDestinationEvent>, mut arrived: ResMut<Arrived>| {
        arrived.0 = Some(ev.grid_tile);
    });

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(5, 0), Some(vec![Vec2::ZERO])),
        false,
    );

    assert_eq!(movable.state, MovableState::Idle);
    assert!(movable.path.is_empty());
    assert_eq!(
        world.resource::<Arrived>().0,
        Some(IVec2::new(5, 0)),
        "событие прихода опубликовано"
    );
    assert!(world.get::<NeedsWanderTarget>(entity).is_some());
}

// --- свежий путь ---

/// Свежий ответ при пешке в стартовой точке: путь начинается со следующего
/// waypoint — стартовая точка ответа всегда вырезается.
#[test]
fn a_fresh_reply_starts_with_the_next_waypoint() {
    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(
        Vec2::ZERO,
        pathfinding_pawn(
            IVec2::new(5, 0),
            &[Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0)],
        ),
    );

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(
            IVec2::new(5, 0),
            Some(vec![Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0)]),
        ),
        false,
    );

    assert_eq!(movable.state, MovableState::Moving(IVec2::new(5, 0)));
    assert_eq!(
        movable.path.into_iter().collect::<Vec<_>>(),
        vec![Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0)]
    );
}

/// Пешка ушла вперёд по старому пути: ведущие точки, не дальше которых она
/// уже стоит, срезаются — иначе первый шаг был бы назад.
#[test]
fn the_reply_is_trimmed_to_where_the_pawn_stands() {
    let (backend, _) = backend(&[]);
    let (mut world, entity) = pawn_world(
        Vec2::new(1.5, 0.0),
        pathfinding_pawn(
            IVec2::new(5, 0),
            &[
                Vec2::ZERO,
                Vec2::new(1.0, 0.0),
                Vec2::new(2.0, 0.0),
                Vec2::new(3.0, 0.0),
            ],
        ),
    );

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(
            IVec2::new(5, 0),
            Some(vec![
                Vec2::ZERO,
                Vec2::new(1.0, 0.0),
                Vec2::new(2.0, 0.0),
                Vec2::new(3.0, 0.0),
            ]),
        ),
        false,
    );

    assert_eq!(movable.state, MovableState::Moving(IVec2::new(5, 0)));
    assert_eq!(
        movable.path.into_iter().collect::<Vec<_>>(),
        vec![Vec2::new(2.0, 0.0), Vec2::new(3.0, 0.0)],
        "точка (1,0) позади пешки срезана: 1.5 уже дальше"
    );
}

/// Сколько бы ведущих точек ни было позади, срезается не больше
/// [`crate::settings::REPATH_TRIM_LIMIT`] — предел существует ради срезания
/// углов, а не только подрезки пробега.
#[test]
fn the_trim_stops_at_its_limit() {
    let limit = crate::settings::REPATH_TRIM_LIMIT;
    let (backend, _) = backend(&[]);
    // пешка зашла в старый путь глубоко: все шесть ведущих точек позади неё
    let path: Vec<Vec2> = (0..=8).map(|x| Vec2::new(x as f32, 0.0)).collect();
    let (mut world, entity) = pawn_world(
        Vec2::new(6.5, 0.0),
        pathfinding_pawn(IVec2::new(20, 0), &path),
    );

    let (movable, _, _) = accept(
        &mut world,
        entity,
        &backend,
        result(IVec2::new(20, 0), Some(path)),
        false,
    );

    assert_eq!(
        movable.path.into_iter().collect::<Vec<_>>()[0],
        Vec2::new(limit as f32 + 1.0, 0.0),
        "срезано ровно {limit} точек, хотя позади пешки их больше"
    );
}
