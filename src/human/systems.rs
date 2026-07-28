use bevy::prelude::*;
use rand::Rng;

use crate::grid::{tile_center, world_to_tile};
use crate::human::components::{Human, HumanWanderTag, WanderPause};
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::{ArcNavmesh, PathfindingAlgorithm, find_passable_tile_near};
use crate::settings::{
    GRID_SIZE, HUMAN_COUNT, HUMAN_SIZE, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, HUMAN_WANDER_RANGE,
    MAP_SIZE, unit_z,
};

/// Отступ целей блуждания от края карты, м.
const MAP_MARGIN: f32 = 4.0;

pub fn spawn_humans(mut commands: Commands, arc_navmesh: Res<ArcNavmesh>) {
    spawn_population(&mut commands, &arc_navmesh.read());
}

/// Спавн населения; вызывается на старте и при рестарте сцены.
pub fn spawn_population(commands: &mut Commands, navmesh: &crate::navigation::Navmesh) {
    let mut rng = rand::rng();

    for _ in 0..HUMAN_COUNT {
        let tile = loop {
            let candidate = IVec2::new(
                rng.random_range(0..GRID_SIZE.x),
                rng.random_range(0..GRID_SIZE.y),
            );
            if navmesh.is_passable(candidate.x, candidate.y) {
                break candidate;
            }
        };
        let position = tile_center(tile);

        // пастельная «одежда» со случайным тоном
        let color = Color::hsl(
            rng.random_range(0.0..360.0),
            rng.random_range(0.35..0.75),
            rng.random_range(0.35..0.65),
        );
        // рассинхронизация первых прогулок, чтобы не было залпа запросов пути
        let pause =
            Timer::from_seconds(rng.random_range(0.0..HUMAN_WANDER_PAUSE.1), TimerMode::Once);

        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(HUMAN_SIZE)),
                ..default()
            },
            Transform::from_translation(position.extend(unit_z(position.y))),
            Human,
            HumanWanderTag,
            Movable::new(HUMAN_WALK_SPEED),
            WanderPause(pause),
            Name::new("human"),
        ));
    }
}

/// Мирное блуждание: пауза 2–10 с → случайная проходимая точка в 20–40 м →
/// путь → по прибытии снова пауза.
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    arc_navmesh: Res<ArcNavmesh>,
    algorithm: Res<PathfindingAlgorithm>,
    mut query: Query<
        (Entity, &SimPosition, &mut Movable, &mut WanderPause),
        (With<Human>, With<HumanWanderTag>),
    >,
) {
    let mut rng = rand::rng();
    let navmesh = arc_navmesh.read();

    for (entity, sim_position, mut movable, mut pause) in &mut query {
        if !matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        ) {
            continue;
        }

        pause.0.tick(time.delta());
        if !pause.0.is_finished() {
            continue;
        }

        let direction = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU));
        let distance = rng.random_range(HUMAN_WANDER_RANGE.0..HUMAN_WANDER_RANGE.1);
        let target = (sim_position.0 + direction * distance)
            .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN);

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

        // следующая пауза — уже после прибытия
        let next_pause = rng.random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1);
        pause
            .0
            .set_duration(std::time::Duration::from_secs_f32(next_pause));
        pause.0.reset();
    }
}
