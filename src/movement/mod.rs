mod components;
mod systems;

use bevy::app::RunFixedMainLoop;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

pub use self::components::{
    Movable, MovableReachedDestinationEvent, MovableState, MovableStateMovingTag,
    PathfindingRequest, PathfindingTask, PreviousSimPosition, SimPosition,
};
pub use self::systems::DrawMovePaths;
use self::systems::{
    dispatch_pathfinding_requests, draw_move_paths, interpolate_movable_transforms,
    listen_for_pathfinding_tasks, move_moving_entities, on_movable_added_init_sim_position,
    snapshot_previous_sim_positions, toggle_draw_move_paths,
};

pub struct MovementPlugin;

impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Movable>()
            .register_type::<MovableStateMovingTag>()
            .register_type::<SimPosition>()
            .register_type::<PreviousSimPosition>()
            .init_resource::<DrawMovePaths>();

        app.register_type::<PathfindingRequest>();
        app.add_observer(on_movable_added_init_sim_position)
            .add_systems(
                Update,
                (dispatch_pathfinding_requests, listen_for_pathfinding_tasks).chain(),
            )
            .add_systems(
                Update,
                (
                    toggle_draw_move_paths.run_if(input_just_pressed(KeyCode::KeyP)),
                    draw_move_paths,
                ),
            )
            // Симуляция движения — фиксированным шагом, визуальный `Transform` —
            // интерполяцией после цикла фиксированных шагов (идиома из
            // `examples/movement/physics_in_fixed_timestep.rs`).
            .add_systems(
                FixedUpdate,
                (snapshot_previous_sim_positions, move_moving_entities).chain(),
            )
            .add_systems(
                RunFixedMainLoop,
                interpolate_movable_transforms.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
            );
    }
}
