//! Сценарии сцены: из чего каждый состоит, чем застраивается арена и как
//! пешки по ней ходят.

use std::collections::VecDeque;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use qwe::grid::{tile_center, world_to_tile};
use qwe::human::{
    Human, HumanFirstWanderTag, HumanStyle, HumanWanderTag, Pace, WanderHeading, WanderPause,
};
use qwe::map::osm::{MapData, WallLine};
use qwe::movement::{
    DestinationClaim, DestinationClaims, Movable, MovableStateMovingTag, SimPosition, SlotLab,
    SlotSearch, slot_side_with_slack, slot_target,
};
use qwe::navigation::{ArcNavmesh, Pathfinder, PolyNavmesh, PolymeshDebug, find_path_polymesh};
use qwe::rng::{PawnId, RngDomain, WanderIndex, decision_stream, stream};
use qwe::settings::{HUMAN_SIZE, HUMAN_SPEED_SPREAD, HUMAN_WALK_SPEED, navtile_size, unit_z};
use rand::Rng;

use crate::DemoConfig;
use crate::metrics::{LastSample, PathMisses, PawnWindow, ProgressSample, WindowOrigin};

#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub(crate) enum Scenario {
    /// Куча в одной точке, никто никуда не идёт: чистая сходимость.
    #[default]
    Pile,
    /// Все идут с обода в одну точку и там остаются — случай «толпа у портала».
    Funnel,
    /// Две колонны навстречу по ОДНОЙ линии waypoint'ов: проверка того, не
    /// стирает ли постановка на waypoint (`move_moving_entities`) боковой
    /// сдвиг, который дало расталкивание.
    Columns,
    /// То же, но в коридоре между двумя стенами: толчок в непроходимый тайл
    /// расталкивание отбрасывает целиком, без скольжения вдоль стены.
    Corridor,
    /// Настоящее блуждание игры: `pick_wander_targets` + настоящий A*.
    /// Контрольный случай — слипает ли толпу сама игровая связка.
    Wander,
}

impl Scenario {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pile => "1 pile",
            Self::Funnel => "2 funnel",
            Self::Columns => "3 columns",
            Self::Corridor => "4 corridor",
            Self::Wander => "5 wander (real AI)",
        }
    }
}

/// Всё, что принадлежит сценарию и умирает при его смене.
#[derive(Component)]
pub(crate) struct DemoPawn;

/// Стена сценария: и спрайт на экране, и запись о заглушенных навтайлах.
#[derive(Component)]
pub(crate) struct DemoWall;

/// Маршрут пешки. Заменяет собой всё поведение — демо гоняет толпу по заранее
/// известным линиям, чтобы мерить расталкивание, а не блуждание.
///
/// `cycle` решает, что происходит после последней точки: замкнутый маршрут
/// начинает круг заново, незамкнутый кончается — [`drive_routes`] снимает
/// компонент, и пешка остаётся стоять там, куда пришла. Без этого дошедшая до
/// цели пешка немедленно получала бы отрезок назад и ходила бы туда-сюда,
/// неотличимо от блуждания.
#[derive(Component)]
pub(crate) struct Route {
    pub(crate) legs: Vec<Vec2>,
    pub(crate) next: usize,
    pub(crate) cycle: bool,
}

// --------------------------------------------------------------------- спавн

/// Раскладка сценария: снести прошлый, вернуть навмешу проходимость, поставить
/// новый. Одна система на все пять — раскладки различаются только позициями и
/// маршрутами.
///
/// Стены живут в двух местах сразу: тайлы сетки и `MapData::walls` для
/// полигонального меша. Второе — не дубль ради дубля: меш строится из
/// векторной геометрии карты и о правках сетки не знает вовсе, так что без
/// записи в `MapData` пешки ходили бы сквозь стены коридора.
#[allow(clippy::too_many_arguments)]
pub(crate) fn respawn_scenario(
    mut commands: Commands,
    config: Res<DemoConfig>,
    scenario: Res<Scenario>,
    navmesh: Res<ArcNavmesh>,
    mut map: ResMut<MapData>,
    mut poly: ResMut<PolyNavmesh>,
    mut polymesh: ResMut<PolymeshDebug>,
    mut misses: ResMut<PathMisses>,
    old: Query<Entity, Or<(With<DemoPawn>, With<DemoWall>)>>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    clear_arena(&navmesh, config.centre);
    // геометрия прошлого сценария — из карты тоже; пересобрать меш придётся,
    // если стены были или появятся
    let rebuild_polymesh = !map.walls.is_empty() || matches!(*scenario, Scenario::Corridor);
    map.walls.clear();
    misses.0 = 0;

    let mut rng = stream(config.seed, RngDomain::Population, 0);
    let centre = config.centre;

    match *scenario {
        Scenario::Pile => {
            for index in 0..config.pile {
                let angle = rng.random_range(0.0..std::f32::consts::TAU);
                let radius = config.pile_radius * rng.random_range(0.0f32..1.0).sqrt();
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    centre + Vec2::from_angle(angle) * radius,
                    None,
                    false,
                );
            }
        }
        Scenario::Funnel => {
            for index in 0..config.funnel {
                let angle = std::f32::consts::TAU * index as f32 / config.funnel as f32;
                let rim = centre + Vec2::from_angle(angle) * config.funnel_radius;
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    rim,
                    // с обода в точку и всё: дошедшая пешка остаётся в центре.
                    // Обратного отрезка нет намеренно — с ним воронка после
                    // первого прохода превращалась в хождение туда-сюда, и
                    // сорвавшуюся с пути пешку было не отличить от блуждающей
                    Some(Route {
                        legs: vec![centre],
                        next: 0,
                        cycle: false,
                    }),
                    false,
                );
            }
        }
        Scenario::Columns => {
            let half = config.column_length / 2.0;
            // полос ровно столько, сколько влезает по шагу колонны: при ширине
            // 0 полоса одна и раскладка та же, что была
            let lanes = (config.column_width / config.column_spacing)
                .floor()
                .max(0.0) as usize
                + 1;
            for index in 0..config.column {
                let lane = index % lanes;
                let rank = index / lanes;
                let across = if lanes > 1 {
                    -config.column_width / 2.0
                        + config.column_width * lane as f32 / (lanes - 1) as f32
                } else {
                    0.0
                };
                let offset = rank as f32 * config.column_spacing;
                let left = centre + Vec2::new(-half - offset, across);
                let right = centre + Vec2::new(half + offset, across);
                // при нулевой ширине обе колонны идут по одной линии
                // y = centre.y, то есть по одним и тем же центрам навтайлов
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    left,
                    Some(Route {
                        legs: vec![right, left],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    (config.column + index) as u32,
                    right,
                    Some(Route {
                        legs: vec![left, right],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
            }
        }
        Scenario::Corridor => {
            let half = config.corridor_length / 2.0;
            let gap = config.corridor_gap / 2.0;
            spawn_wall(&mut commands, &navmesh, &mut map, centre, half, gap, 1.0);
            spawn_wall(&mut commands, &navmesh, &mut map, centre, half, gap, -1.0);

            for index in 0..config.corridor {
                let side = if index % 2 == 0 { -1.0 } else { 1.0 };
                let along = index as f32 / config.corridor as f32 * half;
                let lane = rng.random_range(-gap + 0.5..gap - 0.5);
                let start = centre + Vec2::new(side * (half + along), lane);
                let end = centre + Vec2::new(-side * (half + along), lane);
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    start,
                    Some(Route {
                        legs: vec![end, start],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
            }
        }
        Scenario::Wander => {
            let half = config.wander_box / 2.0;
            for index in 0..config.wander {
                let position = centre
                    + Vec2::new(rng.random_range(-half..half), rng.random_range(-half..half));
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    position,
                    None,
                    true,
                );
            }
        }
    }

    // меш описывает геометрию прошлого сценария — под новую его надо
    // построить заново. В игре ту же работу делает вход в `Playing` при смене
    // города; здесь состояние не меняется, поэтому постройку будит правка
    // тумблера, на которую подписан `sync_polymesh_build`
    if rebuild_polymesh {
        poly.clear();
        polymesh.set_changed();
    }
}

/// Пешка сценария. Бандл — тот же, что у `human::spawn_population`, иначе
/// расталкивание её не увидит: оно берёт кандидатов из `SpatialGrid<Human>`
/// (наполняется обсервером по `Transform`) и требует `PawnId` не-опционально.
pub(crate) fn spawn_pawn(
    commands: &mut Commands,
    seed: u64,
    pawn_id: u32,
    position: Vec2,
    route: Option<Route>,
    wandering: bool,
) {
    let mut rng = decision_stream(seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);
    let color = Color::hsl(
        rng.random_range(0.0..360.0),
        rng.random_range(0.35..0.75),
        rng.random_range(0.35..0.65),
    );
    let pace = Pace(rng.random_range(-1.0..=1.0));
    let heading = WanderHeading(Vec2::from_angle(
        rng.random_range(0.0..std::f32::consts::TAU),
    ));

    let mut entity = commands.spawn((
        DemoPawn,
        Sprite {
            color,
            custom_size: Some(Vec2::splat(HUMAN_SIZE)),
            ..default()
        },
        Transform::from_translation(position.extend(unit_z(position.y))),
        Human,
        Movable::new(pace.speed(HUMAN_WALK_SPEED, HUMAN_SPEED_SPREAD)),
        pace,
        PawnId(pawn_id),
        WanderIndex::ready(),
        LastSample(position),
        WindowOrigin(position),
        ProgressSample::default(),
        PawnWindow::default(),
        Name::new("demo pawn"),
    ));
    if let Some(route) = route {
        entity.insert(route);
    }
    if wandering {
        // то, что спрашивает `pick_wander_targets`
        entity.insert((
            HumanWanderTag,
            HumanFirstWanderTag,
            WanderPause(Timer::from_seconds(0.0, TimerMode::Once)),
            heading,
        ));
    }
}

/// Полоса стены вдоль коридора: спрайт для глаза, заглушенные навтайлы для
/// расталкивания и линия в `MapData::walls` для полигонального меша.
///
/// Записей две, потому что бэкендов два и читают они разное: расталкивание
/// проверяет проходимость тайла (`separation/`), а меш строится из
/// векторных контуров карты. Стена, забытая во втором, — дыра ровно в том
/// сценарии, ради которого она поставлена.
pub(crate) fn spawn_wall(
    commands: &mut Commands,
    navmesh: &ArcNavmesh,
    map: &mut MapData,
    centre: Vec2,
    half_length: f32,
    gap: f32,
    side: f32,
) {
    let thickness = 6.0;
    let band = centre + Vec2::new(0.0, side * (gap + thickness / 2.0));
    let size = Vec2::new(half_length * 2.0 + thickness, thickness);

    // осевая ленты — то же, чем стена задана в OSM: `ribbon_outline` раздует
    // её обратно до `size` при постройке меша
    map.walls.push(WallLine {
        points: vec![
            band - Vec2::new(size.x / 2.0, 0.0),
            band + Vec2::new(size.x / 2.0, 0.0),
        ],
        width: thickness,
    });

    {
        let mut navmesh = navmesh.write();
        let lo = world_to_tile(band - size / 2.0);
        let hi = world_to_tile(band + size / 2.0);
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                navmesh.set_passable(x, y, false);
            }
        }
    }

    commands.spawn((
        DemoWall,
        Sprite {
            color: Color::srgb(0.42, 0.40, 0.38),
            custom_size: Some(size),
            ..default()
        },
        Transform::from_translation(band.extend(unit_z(band.y) - 1.0)),
    ));
}

/// Вернуть арене проходимость: стены прошлого сценария иначе остались бы в
/// навмеше навсегда — он живёт в ресурсе, а не в сущностях сцены.
pub(crate) fn clear_arena(navmesh: &ArcNavmesh, centre: Vec2) {
    const ARENA_HALF: f32 = 120.0;
    let mut navmesh = navmesh.write();
    let lo = world_to_tile(centre - Vec2::splat(ARENA_HALF));
    let hi = world_to_tile(centre + Vec2::splat(ARENA_HALF));
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            navmesh.set_passable(x, y, true);
        }
    }
}

// ------------------------------------------------------------------ движение

/// Выдать вставшей пешке следующий отрезок маршрута. Заменяет собой очередь и
/// асинхронность настоящего диспетчера, но не сам поиск: путь по готовому мешу
/// считает `find_path_polymesh` — тот же вызов, что делает таск в игре, только
/// синхронно (пешек здесь сотни, и то лишь в момент, когда отрезок кончился).
///
/// Пока меш строится, путь идёт прямой по центрам навтайлов — в такой форме
/// его отдаёт сеточный A*, и это ровно то, чем в игре сетка обслуживает
/// запросы до готовности меша.
///
/// Незамкнутый маршрут после последнего отрезка снимается: пешка выпадает из
/// запроса этой системы и остаётся стоять. Заявку на слот (`DestinationClaim`)
/// при этом не трогаем — пешка на нём стоит, и отпустить его значило бы отдать
/// занятое место следующему.
/// Отрезок идёт через тот же слот назначения, что и цели в игре
/// (`movement::destination`): без этого «воронка» гоняла бы 200 пешек в одну
/// точку — то есть проверяла бы не расталкивание, а очередь к одному тайлу.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_routes(
    mut commands: Commands,
    pathfinder: Pathfinder,
    style: Res<HumanStyle>,
    search: Res<SlotSearch>,
    lab: Res<SlotLab>,
    mut claims: ResMut<DestinationClaims>,
    mut misses: ResMut<PathMisses>,
    mut pawns: Query<
        (
            Entity,
            &mut Movable,
            &mut Route,
            &SimPosition,
            Option<&DestinationClaim>,
        ),
        Without<MovableStateMovingTag>,
    >,
    mut batch: Local<Vec<(Entity, Option<IVec2>, IVec2, Vec2)>>,
    mut slots: Local<Vec<Option<(IVec2, IVec2)>>>,
) {
    claims.sync(
        slot_side_with_slack(style.body_radius * 2.0, lab.slack),
        search.0,
    );
    let polymesh = pathfinder.mode().mesh();
    let navmesh = pathfinder.navmesh.read();

    // заявки — всем пакетом и ДО прокладки путей, как в игре
    // (`assign_destination_slots` идёт раньше диспетчера): пакетное назначение
    // (`SlotMatching::Batch`) без пакета вырождается в жадное
    batch.clear();
    batch.extend(
        pawns
            .iter()
            // отложенная выдача ([`SlotLab::claim_at`]): пока цель далеко,
            // пешка в пакет не входит вовсе — слот занимать рано, и занятым в
            // индексе он числиться не должен
            .filter(|(_, _, route, position, _)| {
                lab.claim_at <= 0.0 || position.0.distance(route.legs[route.next]) <= lab.claim_at
            })
            .map(|(entity, _, route, position, claim)| {
                (
                    entity,
                    claim.map(|claim| claim.0),
                    world_to_tile(route.legs[route.next]),
                    position.0,
                )
            }),
    );
    qwe::movement::claim_batch(
        &mut claims,
        lab.matching,
        &batch,
        |tile| navmesh.is_passable(tile.x, tile.y),
        &mut slots,
    );
    let assigned: HashMap<Entity, Option<(IVec2, IVec2)>> = batch
        .iter()
        .zip(slots.iter())
        .map(|((entity, ..), slot)| (*entity, *slot))
        .collect();

    for (entity, mut movable, mut route, position, _) in &mut pawns {
        let leg = route.legs[route.next];
        let desired = world_to_tile(leg);
        // не в пакете — значит цель ещё далеко и слот не выдан: пешка идёт в
        // саму точку, а отрезок НЕ засчитывается пройденным. Подойдя, она
        // остановится ([`interrupt_for_slot_claim`]) и получит слот здесь же
        let approaching = !assigned.contains_key(&entity);
        let (target_tile, target) = match assigned.get(&entity).copied().flatten() {
            Some((slot, tile)) => {
                commands.entity(entity).insert(DestinationClaim(slot));
                (tile, tile_center(tile))
            }
            None => (desired, leg),
        };

        let path = match polymesh.as_deref() {
            // путь включает стартовую точку — её и отбрасываем, как это
            // делает приёмник ответа в игре
            Some(build) => match find_path_polymesh(build, position.0, target) {
                Some(points) => points.into_iter().skip(1).collect(),
                None => {
                    // цель не села на меш — пешка стоит и пробует снова на
                    // следующем тике, отрезок не считается пройденным
                    misses.0 += 1;
                    continue;
                }
            },
            None => straight_path(position.0, target),
        };

        if !approaching {
            route.next += 1;
            if route.next == route.legs.len() {
                if route.cycle {
                    route.next = 0;
                } else {
                    commands.entity(entity).remove::<Route>();
                }
            }
        }
        // путь из одной точки означает «уже на месте» (тот же контракт, что у
        // `apply_result`): отрезок засчитан, а вести пешку по пустому пути
        // нельзя — `move_moving_entities` докатывал бы её по инерции
        if path.is_empty() {
            continue;
        }
        movable.to_moving(target_tile, path, entity, &mut commands);
    }
}

/// Вернуть на свой слот пешку, которую с него столкнули ([`SlotLab::regroup`]).
///
/// Запрос — ровно тот же, что у переписи `finish_trial`: осевшая (не идёт и
/// маршрута нет) со своей заявкой. Дальше — прямой отрезок к цели слота; путь
/// короткий, поэтому прокладывается той же `find_path_polymesh`, что и всё
/// остальное в этой сцене.
pub(crate) fn regroup_to_slot(
    mut commands: Commands,
    pathfinder: Pathfinder,
    style: Res<HumanStyle>,
    lab: Res<SlotLab>,
    mut pawns: Query<
        (Entity, &mut Movable, &SimPosition, &DestinationClaim),
        (Without<MovableStateMovingTag>, Without<Route>),
    >,
) {
    if lab.regroup <= 0.0 {
        return;
    }
    let side = slot_side_with_slack(style.body_radius * 2.0, lab.slack);
    let polymesh = pathfinder.mode().mesh();
    for (entity, mut movable, position, claim) in &mut pawns {
        let home = slot_target(claim.0, side);
        let target = tile_center(home);
        if position.0.distance(target) <= lab.regroup {
            continue;
        }
        let path: VecDeque<Vec2> = match polymesh.as_deref() {
            Some(build) => match find_path_polymesh(build, position.0, target) {
                Some(points) => points.into_iter().skip(1).collect(),
                None => continue,
            },
            None => straight_path(position.0, target),
        };
        if path.is_empty() {
            continue;
        }
        movable.to_moving(home, path, entity, &mut commands);
    }
}

/// Остановить подошедшую к цели пешку, у которой слота ещё нет, — чтобы
/// [`drive_routes`] выдал ей слот здесь и сейчас ([`SlotLab::claim_at`]).
///
/// Без остановки отложенная выдача не работает вовсе: пешка, идущая в саму
/// точку, доберётся до неё только сквозь всех, кто там уже осел, — то есть
/// ровно тем способом, ради избавления от которого выдача и откладывается.
/// Прерывание — это и есть момент «дошёл до толпы»: дальше пешка выбирает
/// ближайший к цели свободный слот, и все занятые лежат глубже неё.
pub(crate) fn interrupt_for_slot_claim(
    mut commands: Commands,
    lab: Res<SlotLab>,
    mut pawns: Query<
        (Entity, &mut Movable, &Route, &SimPosition),
        (With<MovableStateMovingTag>, Without<DestinationClaim>),
    >,
) {
    if lab.claim_at <= 0.0 {
        return;
    }
    for (entity, mut movable, route, position) in &mut pawns {
        if position.0.distance(route.legs[route.next]) > lab.claim_at {
            continue;
        }
        // не «дошла»: событие прибытия здесь было бы ложью, идти ей ещё в слот
        movable.to_idle(entity, &mut commands, false);
    }
}

/// Центры навтайлов вдоль прямой — форма пути сеточного поиска.
pub(crate) fn straight_path(from: Vec2, to: Vec2) -> VecDeque<Vec2> {
    let steps = ((to - from).length() / navtile_size()).ceil().max(1.0) as i32;
    let mut path = VecDeque::new();
    let mut previous = None;
    for step in 1..=steps {
        let point = from.lerp(to, step as f32 / steps as f32);
        let tile = world_to_tile(point);
        if previous != Some(tile) {
            path.push_back(tile_center(tile));
            previous = Some(tile);
        }
    }
    path
}
