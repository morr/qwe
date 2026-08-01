mod components;
mod systems;

use bevy::app::RunFixedMainLoop;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

pub use self::components::{
    Movable, MovableReachedDestinationEvent, MovableState, MovableStateMovingTag,
    PathfindingRequest, PathfindingTask, PreviousSimPosition, SimPosition,
};
pub use self::systems::{
    DrawMovePaths, MOVEPATH_ARROW_TIP, MOVEPATH_COLOR, wanderers_dispatched_at_zoom,
};
use crate::spatial::SimSet;

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
            .init_resource::<DrawMovePaths>()
            .register_type::<DrawMovePaths>()
            // системы плагина пишут диагностику; без стора их параметры
            // не валидируются и шаг движения молча не выполняется
            .init_resource::<bevy::diagnostic::DiagnosticsStore>();

        app.register_type::<PathfindingRequest>();
        app.add_observer(on_movable_added_init_sim_position)
            .add_systems(
                Update,
                // приёмка ДО диспетчера: снятые готовые таски освобождают
                // бюджет in-flight в этом же кадре. В обратном порядке бюджет
                // каждый кадр видел ~250 уже готовых, но не снятых тасков и
                // выдавал вдвое меньше новых — на 30x диспетчер хронически
                // голодал (156 из 258 стоящих бегущих ждали в очереди)
                (listen_for_pathfinding_tasks, dispatch_pathfinding_requests).chain(),
            )
            .add_systems(
                Update,
                (
                    toggle_draw_move_paths.run_if(input_just_pressed(KeyCode::KeyM)),
                    draw_move_paths,
                ),
            )
            // Симуляция движения — фиксированным шагом, визуальный `Transform` —
            // интерполяцией после цикла фиксированных шагов (идиома из
            // `examples/movement/physics_in_fixed_timestep.rs`).
            // Порядок внутри шага явный: снимок «прошлой» позиции — до того,
            // как её тронет поведение (демон в броске двигает `SimPosition`
            // сам), а шаг по пути — после всего поведения.
            .add_systems(
                FixedUpdate,
                // `.chain()` обязателен и сам по себе: привязки к `SimSet`
                // ничего не упорядочивают, когда эти множества пусты (плагин
                // движения используется отдельно в тестах)
                (
                    snapshot_previous_sim_positions.before(SimSet::SpatialRebuild),
                    move_moving_entities.after(SimSet::HumanBehavior),
                )
                    .chain(),
            )
            .add_systems(
                RunFixedMainLoop,
                interpolate_movable_transforms.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
            );
    }
}
