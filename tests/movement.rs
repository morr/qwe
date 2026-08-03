//! Тесты движения: шаг симуляции в `FixedUpdate` по `SimPosition` и
//! интерполяция `Transform` между шагами. Регрессия на «плавность при
//! ускоренном времени»: пока движение жило в `Update` по `Transform`, кадр при
//! высоком `time_scale` покрывал сразу несколько тайлов, промежуточные
//! проскакивались, а на экране сущность рвано прыгала.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use qwe::grid::{tile_center, world_to_tile};
use qwe::loading::AppState;
use qwe::movement::{
    Movable, MovableReachedDestinationEvent, MovableState, MovableStateMovingTag, MovementPlugin,
    PreviousSimPosition, SimPosition,
};
use qwe::navigation::{ArcNavmesh, NorthstarGrid, PathfindingAlgorithm};
use qwe::settings::DEFAULT_NAVTILE_SIZE;
use qwe::spatial::SpatialPlugin;

/// Шаг симуляции в тестах; при навтайле 2 м (дефолт) скорость 20 м/с даёт
/// ровно один тайл за шаг — удобно считать пересечения.
const FIXED_STEP: f32 = 0.1;
const ONE_TILE_PER_STEP: f32 = DEFAULT_NAVTILE_SIZE / FIXED_STEP;

/// Приложение с реальным `MovementPlugin`: тесты проверяют в том числе
/// расстановку систем по расписаниям (шаг — в `FixedUpdate`, интерполяция —
/// в `AfterFixedMainLoop`), поэтому плагин берётся целиком, а не по системам.
///
/// `frame_delta` подаётся вручную; `max_delta` поднят, иначе `Time<Virtual>`
/// обрежет дельту на 250 мс по умолчанию и ускорение времени в тесте
/// не проявится.
fn test_app(frame_delta: f32, time_scale: f32) -> App {
    let mut app = App::new();
    // `SpatialPlugin` нужен не ради сеток, а ради порядка `SimSet`
    // (`SpatialRebuild → DemonBehavior → HumanBehavior`): движение
    // упорядочено относительно этих сетов, и без них снимок прошлой позиции
    // и шаг по пути встали бы в произвольном порядке
    app.add_plugins((
        MinimalPlugins,
        bevy::state::app::StatesPlugin,
        bevy::input::InputPlugin,
        bevy::diagnostic::DiagnosticsPlugin,
        SpatialPlugin,
        MovementPlugin,
    ))
    .insert_state(AppState::Playing)
    // без рендера у `draw_move_paths` не резолвится `Gizmos`, а падать из-за
    // отладочной отрисовки тест не должен — проверяется симуляция
    .set_error_handler(bevy::ecs::error::warn)
    // ресурсы pathfinding-диспетчера: сам он всё равно не проходит
    // валидацию без камеры и окна, но параметры должны резолвиться
    .init_resource::<ArcNavmesh>()
    .init_resource::<NorthstarGrid>()
    .init_resource::<PathfindingAlgorithm>()
    .insert_resource(Time::<Fixed>::from_seconds(FIXED_STEP as f64))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        frame_delta,
    )));

    let mut virtual_time = Time::<Virtual>::default();
    virtual_time.set_max_delta(Duration::from_secs(10));
    virtual_time.set_relative_speed(time_scale);
    app.insert_resource(virtual_time);

    // первый апдейт только инициализирует время (дельта нулевая) — прогоняем
    // его на пустом мире, чтобы каждый последующий был полноценным кадром
    app.update();

    app
}

/// Сущность на центре `start_tile`, идущая по прямой на восток через
/// `tiles` тайлов. Состояние `Moving` задаётся полем, поэтому тег
/// (обычно его ставит `Movable::to_moving`) вставляется руками.
fn spawn_walker(app: &mut App, start_tile: IVec2, tiles: i32, speed: f32) -> (Entity, IVec2) {
    let end_tile = start_tile + IVec2::new(tiles, 0);
    // путь — мировые точки; сеточный поиск отдаёт их центрами тайлов
    let path: VecDeque<Vec2> = (1..=tiles)
        .map(|step| tile_center(start_tile + IVec2::new(step, 0)))
        .collect();
    let start_world = tile_center(start_tile);

    let entity = app
        .world_mut()
        .spawn((
            Movable {
                speed,
                path,
                state: MovableState::Moving(end_tile),
                last_direction: Vec2::ZERO,
            },
            MovableStateMovingTag,
            Transform::from_translation(start_world.extend(0.0)),
        ))
        .id();

    (entity, end_tile)
}

fn sim_position(app: &App, entity: Entity) -> Vec2 {
    app.world()
        .get::<SimPosition>(entity)
        .expect("sim position")
        .0
}

/// Один фиксированный шаг проходит столько waypoint'ов, на сколько хватает
/// времени, а не ровно один: остаток времени после waypoint'а тратится дальше.
/// Иначе быстрая сущность упиралась бы в первый тайл пути каждый шаг.
#[test]
fn one_fixed_step_walks_through_several_waypoints() {
    let mut app = test_app(FIXED_STEP, 1.0);
    // на порядок больше, чем нужно на все четыре тайла за шаг
    let (entity, end_tile) =
        spawn_walker(&mut app, IVec2::new(10, 10), 4, ONE_TILE_PER_STEP * 10.0);

    app.update();

    assert_eq!(sim_position(&app, entity), tile_center(end_tile));
    assert!(
        app.world()
            .get::<Movable>(entity)
            .expect("movable")
            .path
            .is_empty()
    );
}

/// Придя в конечный тайл, сущность переходит в `Idle`, теряет тег движения и
/// сообщает о прибытии ровно один раз.
#[test]
fn reaching_the_destination_reports_it_once_and_goes_idle() {
    let mut app = test_app(FIXED_STEP, 1.0);
    let (entity, end_tile) =
        spawn_walker(&mut app, IVec2::new(10, 10), 4, ONE_TILE_PER_STEP * 10.0);

    #[derive(Resource, Default)]
    struct Arrivals(Vec<IVec2>);
    app.init_resource::<Arrivals>();
    app.add_observer(
        |event: On<MovableReachedDestinationEvent>, mut arrivals: ResMut<Arrivals>| {
            arrivals.0.push(event.grid_tile);
        },
    );

    // прибытие фиксируется шагом, следующим за опустошением пути
    for _ in 0..5 {
        app.update();
    }

    assert_eq!(app.world().resource::<Arrivals>().0, vec![end_tile]);
    let movable = app.world().get::<Movable>(entity).expect("movable");
    assert_eq!(movable.state, MovableState::Idle);
    assert!(app.world().get::<MovableStateMovingTag>(entity).is_none());
}

/// Ускорение времени добавляет фиксированных шагов, а не удлиняет каждый:
/// за шаг сущность проходит одно и то же расстояние при `time_scale` 1 и 8,
/// и ни один тайл не проскакивается. Это и есть регрессия на «дырки» в
/// пройденном пути при ускоренной симуляции.
#[test]
fn time_scale_adds_steps_instead_of_stretching_them() {
    /// Позиции на конец каждого фиксированного шага.
    #[derive(Resource, Default)]
    struct StepPositions(Vec<Vec2>);

    fn run(time_scale: f32) -> Vec<Vec2> {
        let mut app = test_app(FIXED_STEP, time_scale);
        app.init_resource::<StepPositions>();
        app.add_systems(
            FixedPostUpdate,
            |query: Query<&SimPosition, With<Movable>>, mut positions: ResMut<StepPositions>| {
                positions.0.extend(query.iter().map(|position| position.0));
            },
        );
        // 16 тайлов пути хватает и на восемь шагов ускоренного прогона
        spawn_walker(&mut app, IVec2::new(10, 10), 16, ONE_TILE_PER_STEP);

        app.update();
        app.world().resource::<StepPositions>().0.clone()
    }

    let normal = run(1.0);
    let fast = run(8.0);

    assert_eq!(normal.len(), 1, "один кадр при scale 1 — один шаг");
    assert_eq!(fast.len(), 8, "тот же кадр при scale 8 — восемь шагов");

    // шаг всегда один тайл: длина шага не зависит от масштаба времени
    let start = tile_center(IVec2::new(10, 10));
    for (index, position) in fast.iter().enumerate() {
        let previous = if index == 0 { start } else { fast[index - 1] };
        assert!(
            (position.distance(previous) - DEFAULT_NAVTILE_SIZE).abs() < 1e-3,
            "шаг {index} прошёл {} м вместо {DEFAULT_NAVTILE_SIZE}",
            position.distance(previous)
        );
    }
    assert_eq!(
        normal[0], fast[0],
        "первый шаг совпадает при обоих масштабах"
    );

    // ни один тайл не пропущен: тайлы идут подряд от стартового
    let tiles: Vec<IVec2> = fast
        .iter()
        .map(|&position| world_to_tile(position))
        .collect();
    let expected: Vec<IVec2> = (1..=8).map(|step| IVec2::new(10 + step, 10)).collect();
    assert_eq!(tiles, expected);
}

/// `Transform` — визуальная позиция: между фиксированными шагами он лерпится
/// от прошлой сим-позиции к текущей по `overstep_fraction`. Без этого при
/// нескольких шагах на кадр сущность двигалась бы рывками.
#[test]
fn transform_interpolates_between_fixed_steps() {
    // кадр вдвое короче шага: первый кадр делает шаг, второй — половину пути
    let mut app = test_app(FIXED_STEP / 2.0, 1.0);
    let (entity, _) = spawn_walker(&mut app, IVec2::new(10, 10), 16, ONE_TILE_PER_STEP);
    let start = tile_center(IVec2::new(10, 10));

    // два кадра = один фиксированный шаг, остаток нулевой
    app.update();
    app.update();
    let transform = app.world().get::<Transform>(entity).expect("transform");
    assert_eq!(transform.translation.truncate(), start);
    assert_eq!(
        sim_position(&app, entity),
        start + Vec2::new(DEFAULT_NAVTILE_SIZE, 0.0)
    );

    // кадр без шага: остаток 0.5 — визуально сущность на середине шага
    app.update();
    let transform = app.world().get::<Transform>(entity).expect("transform");
    let expected = start + Vec2::new(DEFAULT_NAVTILE_SIZE / 2.0, 0.0);
    assert!(
        transform.translation.truncate().distance(expected) < 1e-3,
        "ожидалась середина шага {expected}, а не {}",
        transform.translation.truncate()
    );
}

/// Перепрокладка на ходу: заявка на новый путь не останавливает сущность — она
/// продолжает идти по старому, пока ответ не пришёл. Регрессия на «убегающие
/// стоят, пока считается pathfind»: между заявкой и ответом проходит минимум
/// кадр, и на ускоренном времени эта пауза съедала четверть времени бегства.
#[test]
fn repathing_keeps_walking_the_old_path() {
    let mut app = test_app(FIXED_STEP, 1.0);
    let (entity, _) = spawn_walker(&mut app, IVec2::new(10, 10), 16, ONE_TILE_PER_STEP);

    app.update();
    let before_repath = sim_position(&app, entity);

    // поведение просит путь к новой цели, как это делает `flee`
    let new_target = IVec2::new(10, 30);
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands, query: Single<(Entity, &SimPosition, &mut Movable)>| {
                let (entity, position, mut movable) = query.into_inner();
                movable.to_pathfinding(
                    entity,
                    world_to_tile(position.0),
                    new_target,
                    &mut commands,
                );
            },
        )
        .expect("run to_pathfinding once");

    app.update();

    let movable = app.world().get::<Movable>(entity).expect("movable");
    assert_eq!(movable.state, MovableState::Pathfinding(new_target));
    assert!(
        app.world().get::<MovableStateMovingTag>(entity).is_some(),
        "тег движения снят — сущность встала на время расчёта"
    );
    assert!(
        sim_position(&app, entity).x > before_repath.x,
        "сущность не сдвинулась, пока считается новый путь"
    );
}

/// Остановившаяся сущность не дрожит: снимок прошлой позиции снимается со
/// всех сущностей, а не только движущихся, поэтому оба конца интерполяции
/// совпадают и `Transform` стоит ровно на сим-позиции.
#[test]
fn stopped_entity_transform_settles_on_the_sim_position() {
    let mut app = test_app(FIXED_STEP, 1.0);
    let (entity, end_tile) =
        spawn_walker(&mut app, IVec2::new(10, 10), 2, ONE_TILE_PER_STEP * 10.0);

    for _ in 0..5 {
        app.update();
    }

    let destination = tile_center(end_tile);
    assert_eq!(sim_position(&app, entity), destination);
    assert_eq!(
        app.world()
            .get::<PreviousSimPosition>(entity)
            .expect("previous sim position")
            .0,
        destination
    );
    assert_eq!(
        app.world()
            .get::<Transform>(entity)
            .expect("transform")
            .translation
            .truncate(),
        destination
    );
}

/// Путь дожёван, а ответ перепрокладки ещё не пришёл (`Pathfinding`) —
/// сущность докатывает по последнему вектору вместо остановки.
#[test]
fn coasts_along_last_direction_while_repath_is_pending() {
    let mut app = test_app(FIXED_STEP, 1.0);
    let start = tile_center(IVec2::new(10, 10));
    let entity = app
        .world_mut()
        .spawn((
            Movable {
                speed: ONE_TILE_PER_STEP,
                path: VecDeque::new(),
                state: MovableState::Pathfinding(IVec2::new(50, 10)),
                last_direction: Vec2::X,
            },
            MovableStateMovingTag,
            Transform::from_translation(start.extend(0.0)),
        ))
        .id();

    for _ in 0..3 {
        app.update();
    }

    let position = sim_position(&app, entity);
    assert!(
        position.x > start.x + DEFAULT_NAVTILE_SIZE,
        "должна была докатить на восток: {position}"
    );
    assert!(
        app.world().get::<MovableStateMovingTag>(entity).is_some(),
        "докат не снимает тег движения"
    );
}

/// Докат упирается в непроходимый тайл — сущность останавливается и теряет
/// тег движения, сквозь стену не проходит.
#[test]
fn coasting_stops_at_an_impassable_tile() {
    let mut app = test_app(FIXED_STEP, 1.0);
    // стена ровно на пути доката: тайл (12, 10)
    app.world()
        .resource::<ArcNavmesh>()
        .write()
        .set_passable(12, 10, false);

    let start = tile_center(IVec2::new(10, 10));
    let entity = app
        .world_mut()
        .spawn((
            Movable {
                speed: ONE_TILE_PER_STEP,
                path: VecDeque::new(),
                state: MovableState::Pathfinding(IVec2::new(50, 10)),
                last_direction: Vec2::X,
            },
            MovableStateMovingTag,
            Transform::from_translation(start.extend(0.0)),
        ))
        .id();

    for _ in 0..5 {
        app.update();
    }

    let position = sim_position(&app, entity);
    assert!(
        position.x < tile_center(IVec2::new(12, 10)).x - DEFAULT_NAVTILE_SIZE / 2.0,
        "в стену докат не заходит: {position}"
    );
    assert!(
        app.world().get::<MovableStateMovingTag>(entity).is_none(),
        "у стены докат заканчивается и тег снимается"
    );
}
