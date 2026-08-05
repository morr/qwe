//! Стейт-машина демона: Wander / Chase / Devour.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonLungeTag, DemonSpawnPause, DemonWanderTag, DevourUntil,
};
use crate::grid::world_to_tile;
use crate::human::{CorpseTag, FleeRepath, Human, HumanFleeTag, HumanWanderTag, WanderPause};
use crate::movement::{
    Movable, MovableState, MovableStateMovingTag, PathfindingRequest, PathfindingTask,
    PreviousSimPosition, SimPosition,
};
use crate::navigation::{Pathfinder, find_passable_tile_near, line_of_sight};
use crate::settings::{
    DEMON_AGGRO_RADIUS, DEMON_DEVOUR_PAUSE, DEMON_LUNGE_RANGE, DEMON_SPEED, KILL_DISTANCE,
    RADIUS_HYSTERESIS, Z_CORPSE,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

type ChaserCounts = bevy::platform::collections::HashMap<Entity, usize>;

/// Лимит демонов на одну цель — «клещи» из двух допустимы, толпа — нет.
const MAX_CHASERS_PER_TARGET: usize = 2;
/// Переключение на свободного человека, если он не дальше ×1.5 текущей цели.
const SWITCH_DISTANCE_FACTOR: f32 = 1.5;
/// Переключение на заметно более близкого человека: новая цель должна быть
/// ближе текущей минимум на треть — иначе две почти равноудалённые жертвы
/// перекидывают демона каждый такт перепрокладки, а каждое переключение
/// стоит нового запроса пути.
const CLOSER_SWITCH_FACTOR: f32 = 0.7;

fn chasers_of(target: Entity, chasers: &ChaserCounts) -> usize {
    chasers.get(&target).copied().unwrap_or(0)
}

/// Wander → Chase: ближайший человек в радиусе агро, у которого ещё нет
/// `MAX_CHASERS_PER_TARGET` преследователей. Демон в паузе после спавна
/// (`DemonSpawnPause`) агро не берёт: у портала люди ходят постоянно, и первая
/// же жертва срывала бы паузу, ради которой она и заведена.
pub fn acquire_targets(
    mut commands: Commands,
    humans: Res<SpatialGrid<Human>>,
    positions: Query<&SimPosition, With<Human>>,
    chasing: Query<&ChaseTarget, With<Demon>>,
    query: Query<
        (Entity, &SimPosition),
        (With<Demon>, With<DemonWanderTag>, Without<DemonSpawnPause>),
    >,
    mut movables: Query<&mut Movable>,
) {
    let mut chasers: ChaserCounts = ChaserCounts::default();
    for chase_target in &chasing {
        *chasers.entry(chase_target.0).or_insert(0) += 1;
    }

    for (entity, sim_position) in &query {
        let Some((human, _)) = humans.nearest_in_range_where(
            sim_position.0,
            DEMON_AGGRO_RADIUS,
            |candidate| positions.get(candidate).ok().map(|p| p.0),
            |candidate| chasers_of(candidate, &chasers) < MAX_CHASERS_PER_TARGET,
        ) else {
            continue;
        };
        *chasers.entry(human).or_insert(0) += 1;

        if let Ok(mut movable) = movables.get_mut(entity) {
            movable.speed = DEMON_SPEED;
        }
        commands.entity(entity).remove::<DemonWanderTag>().insert((
            DemonChaseTag,
            ChaseTarget(human),
            ChaseRepath::default(),
        ));
        debug!("demon {entity} Wander => Chase {human}");
    }
}

/// Chase: догоняем цель; цель умерла/сбежала/далеко — обратно в Wander;
/// догнали — `DemonCaughtHumanEvent`. В такт перепрокладки пробуем сменить
/// цель: если делим её с другим демоном — на никем не занятого человека не
/// дальше ×1.5 текущей дистанции, иначе — на любого, кто ближе ×0.7 её.
/// В обоих случаях только при прямой видимости. Вблизи — бросок напрямую
/// (см. `DEMON_LUNGE_RANGE`).
///
/// `Without<Human>` в фильтре обязателен: обе выборки трогают `SimPosition`,
/// и без него планировщик видит конфликт доступа.
pub fn chase(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    time: Res<Time>,
    pathfinder: Pathfinder,
    humans: Res<SpatialGrid<Human>>,
    mut query: Query<
        (
            Entity,
            &mut SimPosition,
            &mut ChaseTarget,
            &mut ChaseRepath,
            &mut Movable,
            Has<DemonLungeTag>,
        ),
        (With<Demon>, With<DemonChaseTag>, Without<Human>),
    >,
    targets: Query<&SimPosition, With<Human>>,
) {
    let started = std::time::Instant::now();
    let navmesh = pathfinder.navmesh.read();
    // один труп — одно убийство: дедупликация внутри тика, пока команды
    // (снятие `Human`) ещё не применились
    let mut killed_this_tick: bevy::platform::collections::HashSet<Entity> =
        bevy::platform::collections::HashSet::default();

    let mut chasers: ChaserCounts = ChaserCounts::default();
    for (_, _, chase_target, _, _, _) in &query {
        *chasers.entry(chase_target.0).or_insert(0) += 1;
    }

    for (entity, mut sim_position, mut chase_target, mut repath, mut movable, lunging) in &mut query
    {
        // цель умерла (труп/despawn) — снова блуждание
        let Ok(target_position) = targets.get(chase_target.0) else {
            back_to_wander(&mut commands, entity, &mut movable);
            continue;
        };
        if killed_this_tick.contains(&chase_target.0) {
            back_to_wander(&mut commands, entity, &mut movable);
            continue;
        }

        let mut target_pos = target_position.0;
        let distance = sim_position.0.distance(target_pos);

        // гистерезис выхода из погони
        if distance > DEMON_AGGRO_RADIUS * RADIUS_HYSTERESIS {
            *chasers.entry(chase_target.0).or_insert(1) -= 1;
            back_to_wander(&mut commands, entity, &mut movable);
            continue;
        }

        if distance < KILL_DISTANCE {
            killed_this_tick.insert(chase_target.0);
            commands.trigger(DemonCaughtHumanEvent {
                demon: entity,
                human: chase_target.0,
            });
            continue;
        }

        // Финальный бросок. Тайловый путь ведёт к ЦЕНТРУ тайла жертвы, а та
        // внутри тайла продолжает двигаться: остаток до полутора метров
        // тайловой навигацией не покрывается, и демон бесконечно «почти
        // догоняет». Вблизи идём прямо на текущую позицию цели — но только
        // при прямой видимости: жертва, скрывшаяся за углом здания, снова
        // догоняется обычным путём, сквозь стены бросок не проходит.
        if distance <= DEMON_LUNGE_RANGE && line_of_sight(&navmesh, sim_position.0, target_pos) {
            // путь больше не нужен: дальше демона ведёт бросок, а не
            // `move_moving_entities`
            if !matches!(movable.state, MovableState::Idle) {
                movable.to_idle(entity, &mut commands, false);
            }
            if !lunging {
                commands.entity(entity).insert(DemonLungeTag);
            }
            let step = (movable.speed * time.delta_secs()).min(distance);
            let lunge = (target_pos - sim_position.0).normalize_or_zero() * step;
            sim_position.0 += lunge;
            continue;
        }

        // цель разорвала дистанцию или ушла за угол — бросок отменён
        if lunging {
            commands.entity(entity).remove::<DemonLungeTag>();
        }

        // перепрокладка пути к цели — по таймеру, не каждый тик
        repath.0.tick(time.delta());
        let needs_first_path = matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        );
        if !repath.0.just_finished() && !needs_first_path {
            continue;
        }

        // Смена цели, два случая. Цель делим с другим демоном — берём любого
        // никем не занятого человека не дальше ×1.5 текущей дистанции («клещи»
        // распадаются). Цель своя — берём человека, оказавшегося заметно ближе
        // неё, иначе демон пробегает сквозь толпу мимо доступной добычи.
        // Радиус пропорционален текущей дистанции, и это ровно то, что нужно:
        // вплотную к жертве (2 м) он 1.4 м — демон уже никуда не сворачивает;
        // в хвосте гистерезиса (67.5 м) — 47 м, обход сетки остаётся 3×3.
        let shared = chasers_of(chase_target.0, &chasers) >= MAX_CHASERS_PER_TARGET;
        let (switch_radius, chaser_limit) = if shared {
            (distance * SWITCH_DISTANCE_FACTOR, 1)
        } else {
            (distance * CLOSER_SWITCH_FACTOR, MAX_CHASERS_PER_TARGET)
        };
        let switch = humans
            .nearest_in_range_where(
                sim_position.0,
                switch_radius,
                |candidate| targets.get(candidate).ok().map(|p| p.0),
                |candidate| {
                    candidate != chase_target.0
                        && !killed_this_tick.contains(&candidate)
                        && chasers_of(candidate, &chasers) < chaser_limit
                },
            )
            // Прямая видимость — только у победителя поиска: близкий по евклиду,
            // но отрезанный домом или рекой человек недостижим, и демон топтался
            // бы, перекидывая цель туда-обратно. В фильтре выше `line_of_sight`
            // прогнался бы по каждому кандидату в 3×3 клетках (при 20 000
            // человек это десятки), поэтому он здесь. Не прошёл — цель остаётся,
            // следующая попытка через такт перепрокладки.
            .filter(|&(_, pos)| line_of_sight(&navmesh, sim_position.0, pos));
        if let Some((new_target, new_pos)) = switch {
            *chasers.entry(chase_target.0).or_insert(1) -= 1;
            *chasers.entry(new_target).or_insert(0) += 1;
            debug!(
                "demon {entity} switches chase {} => {new_target}",
                chase_target.0
            );
            chase_target.0 = new_target;
            target_pos = new_pos;
        }

        let target_tile = world_to_tile(target_pos);
        let current_goal = match movable.state {
            MovableState::Moving(goal) | MovableState::Pathfinding(goal) => Some(goal),
            _ => None,
        };
        if current_goal == Some(target_tile) {
            continue;
        }

        let Some(goal_tile) = find_passable_tile_near(&navmesh, target_tile) else {
            continue;
        };
        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            goal_tile,
            &mut commands,
        );
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_CHASE_MS, started);
}

fn back_to_wander(commands: &mut Commands, entity: Entity, movable: &mut Movable) {
    movable.speed = DEMON_SPEED;
    commands
        .entity(entity)
        .remove::<(DemonChaseTag, ChaseTarget, ChaseRepath, DemonLungeTag)>()
        .insert(DemonWanderTag);
    debug!("demon {entity} Chase => Wander");
}

/// Наблюдатель убийства: человек становится трупом, демон — в Devour.
pub fn on_demon_caught_human(
    event: On<DemonCaughtHumanEvent>,
    mut commands: Commands,
    mut telemetry: ResMut<Telemetry>,
    humans: Query<(), With<Human>>,
    mut sprites: Query<(&mut Sprite, &mut Transform)>,
    mut movables: Query<&mut Movable>,
) {
    let DemonCaughtHumanEvent { demon, human } = *event;

    // два демона могли догнать одновременно — труп не убивают дважды
    if humans.get(human).is_err() {
        return;
    }

    // человек → труп: поведение и движение снимаются, спрайт «лежит»
    commands
        .entity(human)
        .remove::<(
            Human,
            HumanWanderTag,
            HumanFleeTag,
            WanderPause,
            FleeRepath,
            Movable,
            MovableStateMovingTag,
            PathfindingTask,
            PathfindingRequest,
            SimPosition,
            PreviousSimPosition,
        )>()
        .insert(CorpseTag);
    if let Ok((mut sprite, mut transform)) = sprites.get_mut(human) {
        sprite.color = Color::srgb(0.35, 0.16, 0.14);
        sprite.custom_size = Some(Vec2::new(1.6, 0.8));
        transform.translation.z = Z_CORPSE;
    }
    telemetry.killed += 1;

    // демон → Devour
    if let Ok(mut movable) = movables.get_mut(demon) {
        movable.to_idle(demon, &mut commands, false);
    }
    let pause = rand::rng().random_range(DEMON_DEVOUR_PAUSE.0..DEMON_DEVOUR_PAUSE.1);
    commands
        .entity(demon)
        .remove::<(
            DemonChaseTag,
            ChaseTarget,
            ChaseRepath,
            DemonLungeTag,
            PathfindingTask,
            PathfindingRequest,
        )>()
        .insert((
            DemonDevourTag,
            DevourUntil(Timer::from_seconds(pause, TimerMode::Once)),
        ));
    debug!(
        "demon {demon} Chase => Devour (killed {human}, total {})",
        telemetry.killed
    );
}

/// Devour → Wander по истечении паузы.
pub fn devour(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<
        (Entity, &mut DevourUntil, &mut Movable, &mut Transform),
        (With<Demon>, With<DemonDevourTag>),
    >,
) {
    for (entity, mut devour_until, mut movable, mut transform) in &mut query {
        devour_until.0.tick(time.delta());
        if !devour_until.0.is_finished() {
            continue;
        }
        transform.scale = Vec3::ONE;
        movable.speed = DEMON_SPEED;
        commands
            .entity(entity)
            .remove::<(DemonDevourTag, DevourUntil)>()
            .insert(DemonWanderTag);
        debug!("demon {entity} Devour => Wander");
    }
}

/// Период пульсации пожирающего демона, сек.
const DEVOUR_PULSE_PERIOD: f32 = 0.5;
/// Амплитуда пульсации: от ×1 до ×1.5 размера.
const DEVOUR_PULSE_MAX_SCALE: f32 = 1.5;

/// Пожирающий демон пульсирует: размер ходит по синусоиде ×1 → ×1.5 → ×1.
pub fn pulse_devouring(
    mut query: Query<(&DevourUntil, &mut Transform), (With<Demon>, With<DemonDevourTag>)>,
) {
    use std::f32::consts::TAU;

    for (devour_until, mut transform) in &mut query {
        let phase = devour_until.0.elapsed_secs() / DEVOUR_PULSE_PERIOD * TAU;
        let scale = 1.0 + (DEVOUR_PULSE_MAX_SCALE - 1.0) * 0.5 * (1.0 - phase.cos());
        transform.scale = Vec3::splat(scale);
    }
}
