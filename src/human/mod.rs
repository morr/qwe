mod behavior;
mod components;
mod systems;

use bevy::prelude::*;

use self::behavior::{escape, flee, panic};
pub use self::components::{
    CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag,
    Pace, PanicRecoil, WanderHeading, WanderPause,
};
pub use self::systems::spawn_population;
use self::systems::{pick_wander_targets, spawn_humans, sync_human_pace};
use crate::determinism::deterministic;
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
            .register_type::<PanicRecoil>()
            .register_type::<Pace>()
            .register_type::<HumanStyle>()
            .init_resource::<HumanStyle>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_humans.in_set(WorldInitSet::Spawn),
            )
            // Выбор целей зарегистрирован дважды, и это осознанно.
            //
            // В детерминированном режиме он обязан идти по тикам: в `Update`
            // и число прогонов за секунду, и дельта, которой заводится
            // `WanderPause`, зависят от fps.
            //
            // В обычном режиме он остаётся в `Update`: там на нём держится
            // прогрев (`loading.rs::poll_warmup` ждёт заявок, а `FixedUpdate`
            // на паузе прогрева не идёт вовсе) и покадровый ритм, к которому
            // подогнаны гейт видимости диспетчера и лимит заявок.
            .add_systems(
                FixedUpdate,
                (
                    pick_wander_targets.run_if(deterministic),
                    panic,
                    flee,
                    escape,
                )
                    .chain()
                    .in_set(SimSet::HumanBehavior),
            )
            .add_systems(
                Update,
                (
                    pick_wander_targets.run_if(not(deterministic)),
                    // ползунок разброса уже вышедшим людям — только на смену
                    // ресурса, проход по всей популяции каждый кадр не нужен
                    sync_human_pace.run_if(resource_changed::<HumanStyle>),
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}
