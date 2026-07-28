//! Стейт-машина демона: Wander / Chase / Devour.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonWanderTag, DevourUntil,
};
use crate::grid::world_to_tile;
use crate::human::{CorpseTag, FleeRepath, Human, HumanFleeTag, HumanWanderTag, WanderPause};
use crate::movement::{
    Movable, MovableState, MovableStateMovingTag, PathfindingTask, PreviousSimPosition, SimPosition,
};
use crate::navigation::{ArcNavmesh, find_passable_tile_near};
use crate::settings::{
    DEMON_AGGRO_RADIUS, DEMON_CHASE_SPEED, DEMON_DEVOUR_PAUSE, DEMON_WANDER_SPEED, KILL_DISTANCE,
    RADIUS_HYSTERESIS, Z_CORPSE,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

/// Wander → Chase: человек в радиусе агро.
pub fn acquire_targets(
    mut commands: Commands,
    humans: Res<SpatialGrid<Human>>,
    query: Query<(Entity, &SimPosition), (With<Demon>, With<DemonWanderTag>)>,
    mut movables: Query<&mut Movable>,
) {
    for (entity, sim_position) in &query {
        let Some((human, _)) = humans.nearest_in_range(sim_position.0, DEMON_AGGRO_RADIUS) else {
            continue;
        };

        if let Ok(mut movable) = movables.get_mut(entity) {
            movable.speed = DEMON_CHASE_SPEED;
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
/// догнали — `DemonCaughtHumanEvent`.
pub fn chase(
    mut commands: Commands,
    time: Res<Time>,
    arc_navmesh: Res<ArcNavmesh>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &ChaseTarget,
            &mut ChaseRepath,
            &mut Movable,
        ),
        (With<Demon>, With<DemonChaseTag>),
    >,
    targets: Query<&SimPosition, With<Human>>,
) {
    let navmesh = arc_navmesh.read();
    // один труп — одно убийство: дедупликация внутри тика, пока команды
    // (снятие `Human`) ещё не применились
    let mut killed_this_tick: bevy::platform::collections::HashSet<Entity> =
        bevy::platform::collections::HashSet::default();

    for (entity, sim_position, chase_target, mut repath, mut movable) in &mut query {
        // цель умерла (труп/despawn) — снова блуждание
        let Ok(target_position) = targets.get(chase_target.0) else {
            back_to_wander(&mut commands, entity, &mut movable);
            continue;
        };
        if killed_this_tick.contains(&chase_target.0) {
            back_to_wander(&mut commands, entity, &mut movable);
            continue;
        }

        let distance = sim_position.0.distance(target_position.0);

        // гистерезис выхода из погони
        if distance > DEMON_AGGRO_RADIUS * RADIUS_HYSTERESIS {
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

        // перепрокладка пути к цели — по таймеру, не каждый тик
        repath.0.tick(time.delta());
        let needs_first_path = matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        );
        if !repath.0.just_finished() && !needs_first_path {
            continue;
        }

        let target_tile = world_to_tile(target_position.0);
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
            &arc_navmesh,
            &mut commands,
        );
    }
}

fn back_to_wander(commands: &mut Commands, entity: Entity, movable: &mut Movable) {
    movable.speed = DEMON_WANDER_SPEED;
    commands
        .entity(entity)
        .remove::<(DemonChaseTag, ChaseTarget, ChaseRepath)>()
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
        .remove::<(DemonChaseTag, ChaseTarget, ChaseRepath, PathfindingTask)>()
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
    mut query: Query<(Entity, &mut DevourUntil, &mut Movable), (With<Demon>, With<DemonDevourTag>)>,
) {
    for (entity, mut devour_until, mut movable) in &mut query {
        devour_until.0.tick(time.delta());
        if !devour_until.0.is_finished() {
            continue;
        }
        movable.speed = DEMON_WANDER_SPEED;
        commands
            .entity(entity)
            .remove::<(DemonDevourTag, DevourUntil)>()
            .insert(DemonWanderTag);
        debug!("demon {entity} Devour => Wander");
    }
}
