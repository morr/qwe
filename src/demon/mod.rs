mod components;
mod systems;

use bevy::prelude::*;

pub use self::components::{Demon, DemonChaseTag, DemonDevourTag, DemonSpawner, DemonWanderTag};
use self::systems::{pick_wander_targets, spawn_initial_burst, tick_spawner};

pub struct DemonPlugin;

impl Plugin for DemonPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Demon>()
            .register_type::<DemonWanderTag>()
            .register_type::<DemonChaseTag>()
            .register_type::<DemonDevourTag>()
            .init_resource::<DemonSpawner>()
            .add_systems(Startup, spawn_initial_burst)
            .add_systems(FixedUpdate, tick_spawner)
            .add_systems(Update, pick_wander_targets);
    }
}
