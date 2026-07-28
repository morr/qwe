use bevy::prelude::*;
use bevy::tasks::futures::check_ready;

use crate::grid::{tile_center, world_to_tile};
use crate::movement::components::{
    Movable, MovableState, MovableStateMovingTag, PathfindingTask, PreviousSimPosition, SimPosition,
};
use crate::settings::unit_z;

/// Снимок позиции на начало фиксированного шага — второй конец интерполяции.
/// Для всех сущностей, не только движущихся: иначе у остановившейся сущности
/// `PreviousSimPosition` протухает и `Transform` дрожит вокруг цели.
pub fn snapshot_previous_sim_positions(mut query: Query<(&mut PreviousSimPosition, &SimPosition)>) {
    for (mut previous, current) in &mut query {
        previous.0 = current.0;
    }
}

/// Движение в `FixedUpdate`, двигает `SimPosition` по waypoint'ам пути.
pub fn move_moving_entities(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Movable, &mut SimPosition), With<MovableStateMovingTag>>,
    time: Res<Time>,
) {
    for (entity, mut movable, mut sim_position) in &mut query {
        if !matches!(movable.state, MovableState::Moving(_)) {
            continue;
        }

        let mut remaining_time = time.delta_secs();
        loop {
            if movable.path.is_empty() {
                let destination_reached = matches!(
                    movable.state,
                    MovableState::Moving(target) if world_to_tile(sim_position.0) == target
                );
                movable.to_idle(entity, &mut commands, destination_reached);
                break;
            }

            let target = tile_center(*movable.path.front().expect("path is non-empty"));
            let to_target = target - sim_position.0;
            let distance = to_target.length();
            let distance_to_move = movable.speed * remaining_time;

            if distance_to_move < distance {
                sim_position.0 += to_target.normalize_or_zero() * distance_to_move;
                break;
            }

            // дошли до waypoint'а — встаём на него и тратим остаток времени
            sim_position.0 = target;
            movable.path.pop_front();
            remaining_time -= distance / movable.speed;
            if remaining_time <= 0.0 {
                break;
            }
        }
    }
}

/// Визуальная позиция: лерп между прошлым и текущим фиксированным шагом,
/// плюс y-сортировка (z от y). Живёт в `AfterFixedMainLoop`.
pub fn interpolate_movable_transforms(
    fixed_time: Res<Time<Fixed>>,
    mut query: Query<(&mut Transform, &SimPosition, &PreviousSimPosition)>,
) {
    let alpha = fixed_time.overstep_fraction();

    for (mut transform, sim_position, previous) in &mut query {
        let rendered = previous.0.lerp(sim_position.0, alpha);
        if transform.translation.truncate() == rendered {
            continue;
        }
        transform.translation = rendered.extend(unit_z(rendered.y));
    }
}

/// `SimPosition` инициализируется из `Transform`: позицию при спавне задают
/// трансформом.
pub fn on_movable_added_init_sim_position(
    event: On<Add, Movable>,
    mut query: Query<(&Transform, &mut SimPosition, &mut PreviousSimPosition)>,
) {
    let Ok((transform, mut sim_position, mut previous)) = query.get_mut(event.entity) else {
        return;
    };

    sim_position.0 = transform.translation.truncate();
    previous.0 = sim_position.0;
}

/// Снимает готовые асинхронные ответы поиска пути.
pub fn listen_for_pathfinding_tasks(
    mut commands: Commands,
    mut tasks: Query<(Entity, &mut Movable, &mut PathfindingTask)>,
) {
    for (entity, mut movable, mut task) in &mut tasks {
        let Some(result) = check_ready(&mut task.0) else {
            continue;
        };
        commands.entity(entity).remove::<PathfindingTask>();

        let MovableState::Pathfinding(end_tile) = movable.state else {
            continue;
        };
        // устаревший ответ — уже запрошена другая цель
        if end_tile != result.end_tile {
            continue;
        }

        let Some(path) = result.path else {
            movable.to_pathfinding_error(entity, end_tile, &mut commands);
            continue;
        };

        // путь всегда включает стартовый тайл; один элемент — мы уже на месте
        if path.len() == 1 {
            movable.to_idle(entity, &mut commands, true);
        } else {
            movable.to_moving(
                end_tile,
                path.into_iter().skip(1).collect(),
                entity,
                &mut commands,
            );
        }
    }
}

/// Отрисовка путей движущихся сущностей; в финальной сцене выключена,
/// переключается клавишей P.
#[derive(Resource, Default)]
pub struct DrawMovePaths(pub bool);

pub fn toggle_draw_move_paths(mut draw: ResMut<DrawMovePaths>) {
    draw.0 = !draw.0;
    info!("draw move paths: {}", draw.0);
}

pub fn draw_move_paths(
    draw: Res<DrawMovePaths>,
    mut gizmos: Gizmos,
    query: Query<(&SimPosition, &Movable), With<MovableStateMovingTag>>,
) {
    if !draw.0 {
        return;
    }

    for (sim_position, movable) in &query {
        let mut points = vec![sim_position.0];
        points.extend(movable.path.iter().map(|&tile| tile_center(tile)));
        gizmos.linestrip_2d(points, Color::srgba(0.9, 0.2, 0.9, 0.7));
    }
}
