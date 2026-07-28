mod behavior;
mod components;
mod systems;

use bevy::prelude::*;

use self::behavior::{acquire_targets, chase, devour, on_demon_caught_human};
pub use self::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonSpawner, DemonWanderTag, DevourUntil,
};
use self::systems::{pick_wander_targets, spawn_initial_burst, tick_spawner};
use crate::spatial::SimSet;

pub struct DemonPlugin;

impl Plugin for DemonPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Demon>()
            .register_type::<DemonWanderTag>()
            .register_type::<DemonChaseTag>()
            .register_type::<DemonDevourTag>()
            .register_type::<ChaseTarget>()
            .register_type::<ChaseRepath>()
            .register_type::<DevourUntil>()
            .init_resource::<DemonSpawner>()
            .add_observer(on_demon_caught_human)
            .add_systems(FixedUpdate, (spawn_initial_burst, tick_spawner).chain())
            .add_systems(
                FixedUpdate,
                (acquire_targets, chase, devour)
                    .chain()
                    .in_set(SimSet::DemonBehavior),
            )
            .add_systems(Update, pick_wander_targets);
    }
}
