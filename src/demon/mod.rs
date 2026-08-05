mod behavior;
mod components;
mod systems;

use bevy::prelude::*;

use self::behavior::{acquire_targets, chase, devour, on_demon_caught_human, pulse_devouring};
pub use self::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonLungeTag, DemonSpawnPause, DemonSpawner, DemonStyle, DemonWanderTag, DevourUntil,
};
use self::systems::{
    draw_lunge_paths, pick_wander_targets, spawn_initial_burst, sync_demon_speed, tick_spawn_pause,
    tick_spawner,
};
use crate::loading::AppState;
use crate::spatial::SimSet;

pub struct DemonPlugin;

impl Plugin for DemonPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Demon>()
            .register_type::<DemonWanderTag>()
            .register_type::<DemonChaseTag>()
            .register_type::<DemonDevourTag>()
            .register_type::<DemonLungeTag>()
            .register_type::<ChaseTarget>()
            .register_type::<ChaseRepath>()
            .register_type::<DevourUntil>()
            .register_type::<DemonSpawnPause>()
            .register_type::<DemonStyle>()
            .init_resource::<DemonSpawner>()
            .init_resource::<DemonStyle>()
            .add_observer(on_demon_caught_human)
            .add_systems(
                FixedUpdate,
                (spawn_initial_burst, tick_spawner)
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (acquire_targets, chase, devour)
                    .chain()
                    .in_set(SimSet::DemonBehavior),
            )
            .add_systems(
                Update,
                (
                    tick_spawn_pause,
                    pick_wander_targets,
                    pulse_devouring,
                    draw_lunge_paths,
                    sync_demon_speed.run_if(resource_changed::<DemonStyle>),
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}
