use bevy::prelude::*;
use rand::Rng;

use crate::demon::components::{Demon, DemonSpawner, DemonWanderTag};
use crate::grid::world_to_tile;
use crate::loading::AppState;
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::{Pathfinder, find_passable_tile_near};
use crate::portal::PortalPos;
use crate::settings::{
    DEMON_CAP, DEMON_INITIAL_BURST, DEMON_SIZE, DEMON_SPEED, MAP_SIZE, PORTAL_DIAMETER, unit_z,
};

/// Блуждание: дистанция до следующей случайной точки, м.
const WANDER_DISTANCE: (f32, f32) = (40.0, 120.0);
/// Разброс направления «от портала», радианы.
const WANDER_SPREAD: f32 = 1.3;
/// Отступ целей блуждания от края карты, м.
const MAP_MARGIN: f32 = 4.0;

/// Стартовый залп; в `FixedUpdate`, а не в `Startup` — после рестарта сцены
/// сброшенный спавнер выпускает залп заново без отдельного кода.
pub fn spawn_initial_burst(
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    portal_pos: Res<PortalPos>,
) {
    if spawner.initial_burst_done {
        return;
    }
    spawner.initial_burst_done = true;

    for index in 0..DEMON_INITIAL_BURST {
        let angle = index as f32 / DEMON_INITIAL_BURST as f32 * std::f32::consts::TAU;
        spawn_demon(&mut commands, portal_pos.0, angle, spawner.spawned);
        spawner.spawned += 1;
    }
}

pub fn tick_spawner(
    time: Res<Time>,
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    portal_pos: Res<PortalPos>,
) {
    if spawner.spawned >= DEMON_CAP {
        return;
    }
    spawner.timer.tick(time.delta());
    if !spawner.timer.just_finished() {
        return;
    }

    let angle = rand::rng().random_range(0.0..std::f32::consts::TAU);
    spawn_demon(&mut commands, portal_pos.0, angle, spawner.spawned);
    spawner.spawned += 1;
}

/// Демон появляется у кромки портала под заданным углом.
fn spawn_demon(commands: &mut Commands, portal_pos: Vec2, angle: f32, index: usize) {
    let position = portal_pos + Vec2::from_angle(angle) * (PORTAL_DIAMETER / 2.0 + 1.0);

    // оттенки красного, чтобы демоны не сливались друг с другом
    let tint = 0.45 + (index % 5) as f32 * 0.08;
    commands.spawn((
        Sprite {
            color: Color::srgb(tint, 0.06, 0.10),
            custom_size: Some(Vec2::splat(DEMON_SIZE)),
            ..default()
        },
        Transform::from_translation(position.extend(unit_z(position.y))),
        Demon,
        DemonWanderTag,
        Movable::new(DEMON_SPEED),
        DespawnOnExit(AppState::Playing),
        Name::new("demon"),
    ));
}

/// Блуждающий демон без пути выбирает случайную проходимую точку «от портала»
/// и запрашивает путь. У края карты направление естественно заворачивает
/// внутрь из-за клампа цели в границы.
pub fn pick_wander_targets(
    mut commands: Commands,
    pathfinder: Pathfinder,
    portal_pos: Res<PortalPos>,
    mut query: Query<(Entity, &SimPosition, &mut Movable), (With<Demon>, With<DemonWanderTag>)>,
) {
    let mut rng = rand::rng();
    let navmesh = pathfinder.navmesh.read();

    for (entity, sim_position, mut movable) in &mut query {
        if !matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        ) {
            continue;
        }

        let away = (sim_position.0 - portal_pos.0).normalize_or(Vec2::from_angle(
            rng.random_range(0.0..std::f32::consts::TAU),
        ));
        let direction =
            Vec2::from_angle(rng.random_range(-WANDER_SPREAD..WANDER_SPREAD)).rotate(away);
        let distance = rng.random_range(WANDER_DISTANCE.0..WANDER_DISTANCE.1);
        let target = (sim_position.0 + direction * distance)
            .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN);

        let Some(target_tile) = find_passable_tile_near(&navmesh, world_to_tile(target)) else {
            continue;
        };

        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            target_tile,
            &mut commands,
        );
    }
}
