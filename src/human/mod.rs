mod components;
mod systems;

use bevy::prelude::*;

pub use self::components::{Human, HumanFleeTag, HumanWanderTag, WanderPause};
use self::systems::{pick_wander_targets, spawn_humans};

pub struct HumanPlugin;

impl Plugin for HumanPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Human>()
            .register_type::<HumanWanderTag>()
            .register_type::<HumanFleeTag>()
            .register_type::<WanderPause>()
            .add_systems(Startup, spawn_humans)
            .add_systems(Update, pick_wander_targets);
    }
}
