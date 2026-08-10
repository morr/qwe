//! Спасение застрявших: инвариант «никто не стоит в непроходимом».

use std::sync::{Arc, RwLock};
use std::time::Duration;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};

use super::components::{Movable, MovableState, PathfindingTask, PreviousSimPosition, SimPosition};
use super::pathfinding::listen_for_pathfinding_tasks;
use super::systems::rescue_trapped_entities;
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
    let task = AsyncComputeTaskPool::get().spawn(async move {
        PathfindingResult {
            path,
            end_tile,
            duration: Duration::ZERO,
        }
    });
    let mut entity = app.world_mut().entity_mut(pawn);
    entity.get_mut::<Movable>().expect("Movable").state = MovableState::Pathfinding(end_tile);
    entity.insert(PathfindingTask::new(task));
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

// --- детерминированный диспетчер ---

use super::components::{PathfindingRequest, RequestedAt};
use super::pathfinding::dispatch_pathfinding_requests_deterministic;
use crate::determinism::{DeterministicRun, SimTick};
use crate::settings::{
    PATHFINDING_UNIT_TILES, PATHFINDING_URGENT_UNITS_PER_TICK, PATHFINDING_WANDER_UNITS_PER_TICK,
};

/// Приложение с одним детерминированным диспетчером и пустой (проходимой)
/// сеткой: проверяется бухгалтерия выдачи, а не сам поиск.
fn app_with_deterministic_dispatcher() -> App {
    AsyncComputeTaskPool::get_or_init(TaskPool::default);
    let mut app = app_with(ArcNavmesh(Arc::new(RwLock::new(Navmesh::default()))));
    app.init_resource::<DeterministicRun>()
        .init_resource::<SimTick>()
        .add_systems(Update, dispatch_pathfinding_requests_deterministic);
    app
}

/// Заявка от пешки: `wander` — мирный человек, иначе срочный (демон).
/// `end_tile` задаётся расстоянием в тайлах от старта, чтобы тест мог
/// управлять ценой заявки.
fn spawn_request(app: &mut App, pawn_id: u32, requested_at: u64, wander: bool, tiles: i32) {
    let start_tile = IVec2::new(10, 10);
    let mut entity = app.world_mut().spawn((
        Movable::new(1.0),
        SimPosition(tile_center(start_tile)),
        PreviousSimPosition(tile_center(start_tile)),
        crate::rng::PawnId(pawn_id),
        RequestedAt(requested_at),
        PathfindingRequest {
            start_tile,
            end_tile: start_tile + IVec2::new(tiles, 0),
        },
    ));
    if wander {
        entity.insert(crate::human::Human);
    }
}

/// Сколько заявок уехало: у выданных `PathfindingRequest` снят.
fn dispatched(app: &mut App) -> usize {
    app.world_mut()
        .query::<&PathfindingTask>()
        .iter(app.world())
        .count()
}

/// Кто остался в очереди — по `PawnId`, отсортировано.
fn still_queued(app: &mut App) -> Vec<u32> {
    let mut left: Vec<u32> = app
        .world_mut()
        .query_filtered::<&crate::rng::PawnId, With<PathfindingRequest>>()
        .iter(app.world())
        .map(|pawn_id| pawn_id.0)
        .collect();
    left.sort_unstable();
    left
}

/// Регрессия на саму яму: до бюджета диспетчер выдавал за тик всё, что в
/// очереди, и 16 000 поручений через город останавливали кадр.
#[test]
fn the_dispatcher_never_exceeds_the_wander_rate_in_one_tick() {
    let app = &mut app_with_deterministic_dispatcher();
    let flood = PATHFINDING_WANDER_UNITS_PER_TICK * 4;
    for pawn_id in 0..flood {
        spawn_request(app, pawn_id, 0, true, 0);
    }

    app.update();

    // заявки по одной единице каждая, значит бюджет измеряется в штуках
    assert_eq!(dispatched(app), PATHFINDING_WANDER_UNITS_PER_TICK as usize);
}

#[test]
fn the_dispatcher_never_exceeds_the_urgent_rate_in_one_tick() {
    let app = &mut app_with_deterministic_dispatcher();
    for pawn_id in 0..PATHFINDING_URGENT_UNITS_PER_TICK * 4 {
        spawn_request(app, pawn_id, 0, false, 0);
    }

    app.update();

    assert_eq!(dispatched(app), PATHFINDING_URGENT_UNITS_PER_TICK as usize);
}

/// Смысл двух счётчиков вместо одного с приоритетом: толпа демонов у портала
/// не имеет права остановить город, а очередь города — задержать панику.
#[test]
fn a_full_urgent_queue_does_not_starve_the_wanderers() {
    let app = &mut app_with_deterministic_dispatcher();
    for pawn_id in 0..PATHFINDING_URGENT_UNITS_PER_TICK * 4 {
        spawn_request(app, pawn_id, 0, false, 0);
    }
    for pawn_id in 0..PATHFINDING_WANDER_UNITS_PER_TICK * 4 {
        spawn_request(app, pawn_id, 0, true, 0);
    }

    app.update();

    assert_eq!(
        dispatched(app),
        (PATHFINDING_URGENT_UNITS_PER_TICK + PATHFINDING_WANDER_UNITS_PER_TICK) as usize
    );
}

/// Цена заявки — расстояние: одно поручение через город стоит столько же,
/// сколько десятки прогулок по соседству. Без этого бюджет, пропускающий залп
/// коротких прогулок, пропустил бы и залп поручений.
#[test]
fn a_long_request_costs_more_of_the_budget_than_a_short_one() {
    let app = &mut app_with_deterministic_dispatcher();
    let far = PATHFINDING_UNIT_TILES * 20;
    for pawn_id in 0..PATHFINDING_WANDER_UNITS_PER_TICK {
        spawn_request(app, pawn_id, 0, true, far);
    }

    app.update();

    // каждая стоит 21 единицу, значит в бюджет их влезает во столько же раз
    // меньше — но не ноль
    let expected = PATHFINDING_WANDER_UNITS_PER_TICK as usize / 21;
    assert_eq!(dispatched(app), expected);
    assert!(expected > 0, "дорогая заявка не должна блокировать очередь");
}

/// FIFO: кто подал раньше, тот и уедет раньше. Это и есть детерминированная
/// замена камерному гейту — ожидание честное, а не по взгляду игрока.
#[test]
fn the_oldest_request_goes_first() {
    let app = &mut app_with_deterministic_dispatcher();
    let quota = PATHFINDING_WANDER_UNITS_PER_TICK;
    for pawn_id in 0..quota {
        spawn_request(app, pawn_id, 1, true, 0);
    }
    // самый молодой, но с наименьшим PawnId: без тика в ключе уехал бы первым
    spawn_request(app, 0, 2, true, 0);

    app.update();

    assert_eq!(still_queued(app), vec![0]);
    assert_eq!(
        app.world_mut()
            .query_filtered::<&RequestedAt, With<PathfindingRequest>>()
            .single(app.world())
            .expect("одна заявка в очереди")
            .0,
        2
    );
}

/// Ничья по тику разрешается номером пешки, а не порядком обхода запроса:
/// он зависит от спавнов и смертей и повтор бы разошёлся.
#[test]
fn a_pawn_id_breaks_a_tie_at_the_same_tick() {
    let app = &mut app_with_deterministic_dispatcher();
    let quota = PATHFINDING_WANDER_UNITS_PER_TICK;
    for pawn_id in (0..=quota).rev() {
        spawn_request(app, pawn_id, 7, true, 0);
    }

    app.update();

    assert_eq!(still_queued(app), vec![quota]);
}

/// Вид перед номером: `PawnId` уникален только внутри вида, поэтому демон №5
/// и убегающий человек №5 живут одновременно и попадают в одну срочную
/// очередь. Без вида ключ бы совпал и порядок задавала бы нестабильная
/// сортировка (в отладочной сборке это ловит `debug_assert` в диспетчере).
#[test]
fn a_demon_and_a_human_may_share_a_pawn_id() {
    let app = &mut app_with_deterministic_dispatcher();
    for pawn_id in 0..PATHFINDING_URGENT_UNITS_PER_TICK * 2 {
        spawn_request(app, pawn_id, 0, false, 0);
        // человек с тем же номером, но убегающий — тоже срочный
        let start_tile = IVec2::new(10, 10);
        app.world_mut().spawn((
            Movable::new(1.0),
            SimPosition(tile_center(start_tile)),
            PreviousSimPosition(tile_center(start_tile)),
            crate::rng::PawnId(pawn_id),
            RequestedAt(0),
            PathfindingRequest {
                start_tile,
                end_tile: start_tile,
            },
            crate::human::Human,
            crate::human::HumanFleeTag,
        ));
    }

    app.update();

    assert_eq!(dispatched(app), PATHFINDING_URGENT_UNITS_PER_TICK as usize);
}
