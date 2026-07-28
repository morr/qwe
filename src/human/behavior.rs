//! Стейт-машина человека: Wander / Flee, спасение за краем карты.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::{ChaseTarget, Demon};
use crate::grid::world_to_tile;
use crate::human::components::{FleeRepath, Human, HumanFleeTag, HumanWanderTag, WanderPause};
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::{ArcNavmesh, PathfindingAlgorithm, find_passable_tile_near};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_PANIC_RADIUS, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, MAP_SIZE,
    RADIUS_HYSTERESIS,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

/// Шаг бегства: насколько далеко от себя прокладывается точка «от демона», м.
const FLEE_STEP: (f32, f32) = (40.0, 60.0);
/// Зона у границы карты, попадание в которую при бегстве — «спасся», м.
const ESCAPE_MARGIN: f32 = 2.0;
/// Веер разбегания: максимальное отклонение от вектора «прочь от демона»,
/// радианы (±≈34°). Толпа без него выстраивается в колонну.
const FLEE_SPREAD: f32 = 0.6;

/// Персональный угол веера: детерминирован по сущности, чтобы человек между
/// перепрокладками держал свою сторону, а не зигзагами метался.
fn personal_spread(entity: Entity) -> f32 {
    let hash = entity.index().index().wrapping_mul(2654435761);
    ((hash >> 8) as f32 / (u32::MAX >> 8) as f32 * 2.0 - 1.0) * FLEE_SPREAD
}

/// Wander → Flee: демон в радиусе паники.
pub fn panic(
    mut commands: Commands,
    demons: Res<SpatialGrid<Demon>>,
    query: Query<(Entity, &SimPosition), (With<Human>, With<HumanWanderTag>)>,
    mut movables: Query<&mut Movable>,
) {
    for (entity, sim_position) in &query {
        if demons
            .nearest_in_range(sim_position.0, HUMAN_PANIC_RADIUS)
            .is_none()
        {
            continue;
        }

        if let Ok(mut movable) = movables.get_mut(entity) {
            movable.speed = HUMAN_FLEE_SPEED;
        }
        let mut repath = FleeRepath::default();
        // первый путь — сразу, дальше по таймеру со случайным периодом
        let period = rand::rng().random_range(0.7..1.2);
        repath
            .0
            .set_duration(std::time::Duration::from_secs_f32(period));
        commands
            .entity(entity)
            .remove::<HumanWanderTag>()
            .insert((HumanFleeTag, repath));
    }
}

/// Flee: бег от ближайшего демона с троттлингом перепрокладки;
/// демоны отстали (×1.5 радиуса) — успокаивается.
pub fn flee(
    mut commands: Commands,
    time: Res<Time>,
    arc_navmesh: Res<ArcNavmesh>,
    algorithm: Res<PathfindingAlgorithm>,
    demons: Res<SpatialGrid<Demon>>,
    chasing: Query<&ChaseTarget, With<Demon>>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut FleeRepath,
            &mut WanderPause,
            &mut Movable,
        ),
        (With<Human>, With<HumanFleeTag>),
    >,
) {
    let navmesh = arc_navmesh.read();
    let mut rng = rand::rng();
    // за кем прямо сейчас гонятся — те бегут по чистому вектору от демона
    let chased: bevy::platform::collections::HashSet<Entity> =
        chasing.iter().map(|chase_target| chase_target.0).collect();

    for (entity, sim_position, mut repath, mut pause, mut movable) in &mut query {
        let Some((_, demon_position)) =
            demons.nearest_in_range(sim_position.0, HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS)
        else {
            // демоны далеко — мирный режим, отдышаться перед новой прогулкой
            movable.speed = HUMAN_WALK_SPEED;
            movable.to_idle(entity, &mut commands, false);
            pause.0.set_duration(std::time::Duration::from_secs_f32(
                rng.random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1),
            ));
            pause.0.reset();
            commands
                .entity(entity)
                .remove::<(HumanFleeTag, FleeRepath)>()
                .insert(HumanWanderTag);
            continue;
        };

        repath.0.tick(time.delta());
        let needs_path = matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        );
        if !repath.0.just_finished() && !needs_path {
            continue;
        }

        let mut away = (sim_position.0 - demon_position).normalize_or(Vec2::X);
        // не преследуемые разбегаются веером — каждый под своим углом
        if !chased.contains(&entity) {
            away = Vec2::from_angle(personal_spread(entity)).rotate(away);
        }
        let step = rng.random_range(FLEE_STEP.0..FLEE_STEP.1);
        // не клампим к «безопасной» зоне: цель у самой границы — путь к спасению
        let target = (sim_position.0 + away * step).clamp(Vec2::splat(1.0), MAP_SIZE - 1.0);

        let Some(target_tile) = find_passable_tile_near(&navmesh, world_to_tile(target)) else {
            continue;
        };
        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            target_tile,
            &arc_navmesh,
            *algorithm,
            &mut commands,
        );
    }
}

/// Паникующий пересёк границу карты — «спасся», despawn [Q12].
pub fn escape(
    mut commands: Commands,
    mut telemetry: ResMut<Telemetry>,
    query: Query<(Entity, &SimPosition), (With<Human>, With<HumanFleeTag>)>,
) {
    for (entity, sim_position) in &query {
        let pos = sim_position.0;
        if pos.x <= ESCAPE_MARGIN
            || pos.y <= ESCAPE_MARGIN
            || pos.x >= MAP_SIZE.x - ESCAPE_MARGIN
            || pos.y >= MAP_SIZE.y - ESCAPE_MARGIN
        {
            commands.entity(entity).despawn();
            telemetry.escaped += 1;
            debug!("human {entity} escaped (total {})", telemetry.escaped);
        }
    }
}
