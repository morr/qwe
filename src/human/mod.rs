mod behavior;
mod components;
mod systems;

use bevy::prelude::*;

use self::behavior::{escape, flee, panic};
pub use self::components::{
    CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanWanderTag, WanderHeading,
    WanderPause,
};
pub use self::systems::spawn_population;
use self::systems::{pick_wander_targets, spawn_humans};
use crate::loading::{AppState, WorldInitSet};
use crate::spatial::SimSet;

pub struct HumanPlugin;

impl Plugin for HumanPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Human>()
            .register_type::<HumanWanderTag>()
            .register_type::<HumanFirstWanderTag>()
            .register_type::<HumanFleeTag>()
            .register_type::<CorpseTag>()
            .register_type::<FleeRepath>()
            .register_type::<WanderPause>()
            .register_type::<WanderHeading>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_humans.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                FixedUpdate,
                (panic, flee, escape).chain().in_set(SimSet::HumanBehavior),
            )
            .add_systems(
                Update,
                pick_wander_targets.run_if(in_state(AppState::Playing)),
            );
    }
}
