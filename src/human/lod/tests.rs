use std::time::Duration;

use bevy::ecs::system::RunSystemOnce;

use super::*;
use crate::human::components::HumanFirstWanderTag;

/// Кадр двора: центр карты, зум внутрь города, полкадра 300 м. Пешки
/// дальше ~360 м (300 · VIEW_MARGIN) — вне кадра с запасом.
fn view(zoom: f32) -> Viewport {
    Viewport {
        centre: crate::settings::MAP_SIZE / 2.0,
        half_extent: Vec2::splat(300.0),
        zoom,
    }
}

/// Двор LOD: поведенческий двор плюс сетки, стиль, сид, телеметрия и кадр.
fn lod_yard(zoom: f32) -> App {
    let mut app = crate::sim_yard::behavior_yard();
    app.init_resource::<SpatialGrid<Human>>()
        .init_resource::<SpatialGrid<Demon>>()
        .init_resource::<HumanStyle>()
        .init_resource::<WorldSeed>()
        .init_resource::<Telemetry>()
        .insert_resource(view(zoom));
    app
}

/// Мирная пешка: `pause_zero` — пауза закончена, цель выбирается первым
/// же срезом; `far` — уже в дальних (маркер и срез по номеру).
fn spawn_pawn(app: &mut App, pawn_id: u32, position: Vec2, pause_zero: bool, far: bool) -> Entity {
    let bundle = (
        Transform::from_translation(position.extend(0.0)),
        Human,
        HumanWanderTag,
        SimPosition(position),
        Movable::new(1.0),
        Pace(0.0),
        WanderHeading(Vec2::X),
        PawnId(pawn_id),
        WanderIndex::ready(),
        WanderPause(if pause_zero {
            Timer::from_seconds(0.0, TimerMode::Once)
        } else {
            WanderPause::default().0
        }),
    );
    if !far {
        return app.world_mut().spawn(bundle).id();
    }
    // дальняя: ярус и срез ставятся по номеру
    let id = app.world_mut().spawn(bundle).id();
    let mut entity = app.world_mut().entity_mut(id);
    match slice_of(pawn_id) {
        0 => entity.insert(FarSlice::<0>),
        1 => entity.insert(FarSlice::<1>),
        2 => entity.insert(FarSlice::<2>),
        3 => entity.insert(FarSlice::<3>),
        4 => entity.insert(FarSlice::<4>),
        5 => entity.insert(FarSlice::<5>),
        6 => entity.insert(FarSlice::<6>),
        _ => entity.insert(FarSlice::<7>),
    }
    .insert(FarTag);
    id
}

/// Демон, видимый и сетке, и запросу позиций.
fn spawn_demon(app: &mut App, position: Vec2) {
    let demon = app.world_mut().spawn((Demon, SimPosition(position))).id();
    app.world_mut()
        .resource_mut::<SpatialGrid<Demon>>()
        .insert(demon, position);
}

/// Есть ли у пешки маркер среза `S`.
macro_rules! has_slice {
    ($app:expr, $entity:expr, $s:literal) => {
        $app.world().get::<FarSlice<$s>>($entity).is_some()
    };
}

/// Свежая пешка вне кадра уходит в дальние первым же проходом — без
/// выдержки: расселение 200k не должно копить очередь заявок, пока
/// выдержка тикает.
#[test]
fn a_fresh_pawn_outside_the_frame_goes_far_without_dwell() {
    let mut app = lod_yard(0.2);
    let far_away = crate::settings::MAP_SIZE / 2.0 + Vec2::X * 3000.0;
    let pawn = spawn_pawn(&mut app, 3, far_away, false, false);

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(
        app.world().get::<FarTag>(pawn).is_some(),
        "пешка обязана уйти в дальние"
    );
    assert_eq!(
        app.world().get::<Visibility>(pawn),
        Some(&Visibility::Hidden),
        "спрайт дальней пешки скрыт"
    );
    assert!(has_slice!(app, pawn, 3), "срез — по номеру пешки");
}

/// Пешка в кадре остаётся ближней, выдержка заново полная.
#[test]
fn a_pawn_inside_the_frame_stays_near() {
    let mut app = lod_yard(0.2);
    let inside = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 4, inside, false, false);

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(app.world().get::<FarTag>(pawn).is_none());
    assert_eq!(
        app.world().get::<LodDwell>(pawn),
        Some(&LodDwell(LodDwell::FULL))
    );
}

/// Городской зум опустошает Near целиком: пешка В КАДРЕ тоже уходит в
/// дальние, без выдержки — на этом зуме мирных не диспетчит никто.
#[test]
fn the_city_zoom_empties_near_entirely() {
    let mut app = lod_yard(0.8); // ≥ WANDER_DISPATCH_MAX_ZOOM
    let inside = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 5, inside, false, false);
    app.world_mut()
        .entity_mut(pawn)
        .insert(LodDwell(LodDwell::FULL));

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(
        app.world().get::<FarTag>(pawn).is_some(),
        "на городском зуме Near пуст — даже пешка в кадре"
    );
}

/// Promote возвращает спрайт, снимает срез и ставит оба конца
/// интерполяции из сим-позиции.
#[test]
fn promote_returns_the_sprite_and_the_slice() {
    let mut app = lod_yard(0.2);
    let inside = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 2, inside, false, true);
    app.world_mut()
        .resource_mut::<SpatialGrid<Human>>()
        .insert(pawn, inside);

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(
        app.world().get::<FarTag>(pawn).is_none(),
        "пешка в кадре — обратно в Near"
    );
    assert_eq!(
        app.world().get::<Visibility>(pawn),
        Some(&Visibility::Visible)
    );
    assert!(!has_slice!(app, pawn, 2));
    assert_eq!(
        app.world().get::<LodDwell>(pawn),
        Some(&LodDwell(LodDwell::FULL))
    );
    // оба конца интерполяции — на сим-позиции: лерп не дёрнет пешку
    // из устаревшего прошлого
    let position = app.world().get::<SimPosition>(pawn).unwrap().0;
    assert_eq!(
        app.world().get::<PreviousSimPosition>(pawn).unwrap().0,
        position
    );
    assert_eq!(
        app.world()
            .get::<Transform>(pawn)
            .unwrap()
            .translation
            .truncate(),
        position
    );
}

/// Demote снимает ждущую заявку: дальние заявок не подают, а оставленная
/// — это очередь диспетчера, которую Фаза 2 обязана опустошить.
#[test]
fn demote_cancels_a_pending_request() {
    use crate::movement::{MovableState, PathfindingRequest, RequestedAt};

    let mut app = lod_yard(0.2);
    let far_away = crate::settings::MAP_SIZE / 2.0 + Vec2::X * 3000.0;
    let pawn = spawn_pawn(&mut app, 6, far_away, false, false);
    let mut entity = app.world_mut().entity_mut(pawn);
    entity.insert((
        PathfindingRequest {
            start_tile: IVec2::new(1, 1),
            end_tile: IVec2::new(2, 2),
        },
        RequestedAt(0),
    ));
    entity.get_mut::<Movable>().unwrap().state = MovableState::Pathfinding(IVec2::new(2, 2));

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(app.world().get::<FarTag>(pawn).is_some());
    assert!(
        app.world().get::<PathfindingRequest>(pawn).is_none(),
        "заявка снята"
    );
    assert!(
        app.world().get::<RequestedAt>(pawn).is_none(),
        "метка тика снята"
    );
    assert!(matches!(
        app.world().get::<Movable>(pawn).unwrap().state,
        MovableState::Idle
    ));
}

/// Promote посреди бегства возвращает срочность: заявка бегства теперь
/// обслуживается вне очереди, как у всякого ближнего беглеца.
#[test]
fn promote_mid_flee_restores_urgency() {
    let mut app = lod_yard(0.2);
    let inside = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 7, inside, false, true);
    app.world_mut()
        .entity_mut(pawn)
        .remove::<HumanWanderTag>()
        .insert(HumanFleeTag);
    app.world_mut()
        .resource_mut::<SpatialGrid<Human>>()
        .insert(pawn, inside);

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(
        app.world().get::<UrgentPath>(pawn).is_some(),
        "срочность возвращена"
    );
    assert!(app.world().get::<FarTag>(pawn).is_none());
}

/// Demote посреди бегства снимает срочность — заявки у дальних нет, и
/// маркер «обслуживать вне очереди» обязан уйти вместе с ней.
#[test]
fn demote_mid_flee_drops_urgency() {
    let mut app = lod_yard(0.2);
    let far_away = crate::settings::MAP_SIZE / 2.0 + Vec2::X * 3000.0;
    let pawn = spawn_pawn(&mut app, 8, far_away, false, false);
    app.world_mut()
        .entity_mut(pawn)
        .remove::<HumanWanderTag>()
        .insert((HumanFleeTag, UrgentPath));

    app.world_mut()
        .run_system_once(assign_lod_tiers)
        .expect("проход");

    assert!(app.world().get::<FarTag>(pawn).is_some());
    assert!(
        app.world().get::<UrgentPath>(pawn).is_none(),
        "срочность снята"
    );
}

/// Ходок, вошедший в кадр ПЕШКОМ при стоячей камере, возвращается
/// сердцебиением прохода: promote не имеет права жить только на движении
/// камеры — у срезной пешки нет другого способа стать ближней.
#[test]
fn a_walker_entering_the_frame_is_promoted_on_a_standing_camera() {
    let mut app = lod_yard(0.2);
    let far_away = crate::settings::MAP_SIZE / 2.0 + Vec2::X * 3000.0;
    let inside = crate::settings::MAP_SIZE / 2.0;
    // ближняя вне кадра: первый проход (кадр «двинулся») demote'ит её
    let pawn = spawn_pawn(&mut app, 3, far_away, false, false);
    app.world_mut()
        .run_system_cached(assign_lod_tiers)
        .expect("проход");
    assert!(app.world().get::<FarTag>(pawn).is_some(), "ушла в дальние");

    // пешка дошла до кадра сама, камера не двигалась: позиция и сетка —
    // от promote; из трёх проходов два — ранние выходы, третий — сердцебиение
    app.world_mut().get_mut::<SimPosition>(pawn).unwrap().0 = inside;
    app.world_mut()
        .resource_mut::<SpatialGrid<Human>>()
        .insert(pawn, inside);
    for _ in 0..3 {
        app.world_mut()
            .run_system_cached(assign_lod_tiers)
            .expect("проход");
    }

    assert!(
        app.world().get::<FarTag>(pawn).is_none(),
        "стоячая камера обязана вернуть пешку в кадре в ближние"
    );
    assert_eq!(
        app.world().get::<Visibility>(pawn),
        Some(&Visibility::Visible)
    );
}

/// Дальнее блуждание: цель — прогулка в конусе, путь — прямая из одной
/// точки, БЕЗ заявки на поиск.
#[test]
fn a_far_wanderer_walks_a_direct_path_without_a_request() {
    let mut app = lod_yard(0.2);
    let center = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 0, center, true, true); // срез 0

    app.world_mut()
        .run_system_once(far_slice::<0>)
        .expect("срез");

    let movable = app.world().get::<Movable>(pawn).unwrap();
    assert!(
        matches!(movable.state, MovableState::Moving(_)),
        "пешка идёт, а не ждёт"
    );
    assert_eq!(movable.path.len(), 1, "путь-прямая — одна точка");
    assert!(
        app.world()
            .get::<crate::movement::PathfindingRequest>(pawn)
            .is_none(),
        "дальние заявок не подают"
    );
    assert!(
        app.world().get::<HumanFirstWanderTag>(pawn).is_none(),
        "первая цель выбрана — тег снят"
    );
}

/// Срезная система трогает только свой срез: пешка среза 0 не
/// обрабатывается системой среза 1 — «нулевой пропуск» обязателен, иначе
/// перебор всех дальних на каждом тике.
#[test]
fn a_slice_system_touches_only_its_own_slice() {
    let mut app = lod_yard(0.2);
    let center = crate::settings::MAP_SIZE / 2.0;
    let of_zero = spawn_pawn(&mut app, 0, center, true, true); // 0 % 8 = 0
    let of_one = spawn_pawn(&mut app, 1, center, true, true); // 1 % 8 = 1

    app.world_mut()
        .run_system_once(far_slice::<1>)
        .expect("срез");

    assert!(
        matches!(
            app.world().get::<Movable>(of_one).unwrap().state,
            MovableState::Moving(_)
        ),
        "пешка среза 1 обработана"
    );
    assert!(
        matches!(
            app.world().get::<Movable>(of_zero).unwrap().state,
            MovableState::Idle
        ),
        "пешка среза 0 не тронута чужой системой"
    );
}

/// Дальнее бегство: тот же вектор «от демона» и путь-прямая, но БЕЗ
/// `UrgentPath` — срочность имеет смысл только у заявки, которой нет.
#[test]
fn a_far_runner_flees_along_a_direct_path() {
    let mut app = lod_yard(0.2);
    let center = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 2, center, false, true);
    // демон к западу → бежать на восток
    spawn_demon(&mut app, center - Vec2::X * 20.0);
    let mut repath = FleeRepath::default();
    repath.0.almost_finish();
    app.world_mut()
        .entity_mut(pawn)
        .remove::<HumanWanderTag>()
        .insert((HumanFleeTag, repath));

    app.world_mut()
        .run_system_once(far_slice::<2>)
        .expect("срез");

    let movable = app.world().get::<Movable>(pawn).unwrap();
    assert!(
        matches!(movable.state, MovableState::Moving(_)),
        "беглец получил путь-прямую"
    );
    let heading = app.world().get::<WanderHeading>(pawn).unwrap().0;
    assert!(heading.x > 0.9, "курс — прочь от демона, а не {heading:?}");
    assert!(
        app.world().get::<UrgentPath>(pawn).is_none(),
        "дальнему не положено"
    );
}

/// Дальняя пешка паникует от демона в ячейках — теми же записями, что
/// ближняя паника, минус срочность и заявка.
#[test]
fn a_far_wanderer_panics_from_a_demon_in_the_cells() {
    let mut app = lod_yard(0.2);
    let center = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 3, center, false, true);
    // демон в сетке людей не нужен: сбор идёт по ней, пешку в неё ставим
    app.world_mut()
        .resource_mut::<SpatialGrid<Human>>()
        .insert(pawn, center);
    spawn_demon(&mut app, center - Vec2::X * 20.0);

    app.world_mut().run_system_once(far_onset).expect("сбор");

    assert!(
        app.world().get::<HumanFleeTag>(pawn).is_some(),
        "пешка бежит"
    );
    assert!(app.world().get::<HumanWanderTag>(pawn).is_none());
    assert!(app.world().get::<FleeRepath>(pawn).is_some());
    assert!(
        app.world().get::<UrgentPath>(pawn).is_none(),
        "заявки нет — срочности нет"
    );
    assert_eq!(
        app.world().get::<Movable>(pawn).unwrap().speed,
        Pace(0.0).speed(HUMAN_FLEE_SPEED, HumanStyle::default().spread)
    );
}

/// Дальнее бегство успокаивается той же лестницей: демонов в ячейках нет —
/// мирный шаг, пауза из потока, теги поменяны.
#[test]
fn a_far_runner_calms_down_without_demons() {
    let mut app = lod_yard(0.2);
    let center = crate::settings::MAP_SIZE / 2.0;
    let pawn = spawn_pawn(&mut app, 4, center, false, true);
    let mut repath = FleeRepath::default();
    repath.0.almost_finish();
    app.world_mut()
        .entity_mut(pawn)
        .remove::<HumanWanderTag>()
        .insert((HumanFleeTag, repath));

    app.world_mut()
        .run_system_once(far_slice::<4>)
        .expect("срез");

    assert!(
        app.world().get::<HumanWanderTag>(pawn).is_some(),
        "успокоилась"
    );
    assert!(app.world().get::<HumanFleeTag>(pawn).is_none());
    assert!(app.world().get::<FleeRepath>(pawn).is_none());
    assert_eq!(
        app.world().get::<Movable>(pawn).unwrap().speed,
        Pace(0.0).speed(HUMAN_WALK_SPEED, HumanStyle::default().spread)
    );
}

/// Спасение за краем — на срезе, в общую телеметрию: инвариант
/// `alive + killed + escaped` не различает ярусов.
#[test]
fn a_far_runner_escapes_at_the_edge() {
    let mut app = lod_yard(0.2);
    // беглец у кромки (но вне полосы спасения), идущий прочь: шаг среза
    // (dt = K тиков) выносит за край
    let middle = crate::settings::MAP_SIZE.y / 2.0;
    let edge = Vec2::new(4.0, middle);
    let pawn = spawn_pawn(&mut app, 5, edge, false, true);
    let mut movable = Movable::new(50.0);
    movable.state = MovableState::Moving(IVec2::new(0, 50));
    movable.path = [Vec2::new(-5.0, middle)].into();
    app.world_mut()
        .entity_mut(pawn)
        .remove::<HumanWanderTag>()
        .insert((HumanFleeTag, movable, MovableStateMovingTag));
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(1.0 / 64.0));

    app.world_mut()
        .run_system_once(far_slice::<5>)
        .expect("срез");

    assert!(
        app.world().get_entity(pawn).is_err(),
        "пешка спаслась и деспауннулась"
    );
    assert_eq!(app.world().resource::<Telemetry>().escaped, 1);
}

/// Пин Фазы 2: под тумблером детерминизма дальних не существует вовсе —
/// `FarTag` не появляется за целый прогон реплей-двора. Сломается, если
/// LOD-системы переведут из `SimPipeline::Live` в общую ветку.
#[test]
fn no_far_tag_appears_in_a_deterministic_run() {
    let yard = crate::map::osm::fixture::crowded_yard();
    let mut navmesh = crate::navigation::Navmesh::default();
    navmesh.fill_from_mapdata(&yard.map);
    let mut app = crate::determinism::replay::replay_app(yard.map, navmesh, yard.portal, 7, 64);
    crate::determinism::replay::run_to_tick(
        &mut app,
        96,
        &[1],
        crate::determinism::replay::Progress::Silent,
    );

    let far = app.world_mut().query::<&FarTag>().iter(app.world()).count();
    assert_eq!(
        far, 0,
        "детерминированный прогон завёл {far} дальних: LOD обязан жить только в Live"
    );
}
