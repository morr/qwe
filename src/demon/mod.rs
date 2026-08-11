mod behavior;
mod components;
mod systems;

use bevy::prelude::*;

use self::behavior::{acquire_targets, chase, devour, on_demon_caught_human, pulse_devouring};
pub use self::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonLungeTag, DemonSpawner, DemonStyle, DemonWanderTag, DevourUntil,
};
use self::systems::{
    draw_lunge_paths, pick_wander_targets, spawn_initial_burst, sync_demon_speed, tick_spawner,
};
use crate::loading::{AppState, WorldStarted};
use crate::prefs::TrackPrefExt;
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
            .register_type::<DemonStyle>()
            .init_resource::<DemonSpawner>()
            .init_resource::<DemonStyle>()
            .track_pref::<DemonStyle>()
            .add_observer(on_demon_caught_human)
            .add_observer(on_world_started)
            .add_systems(
                FixedUpdate,
                (spawn_initial_burst, tick_spawner)
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            )
            // выбор цели блуждания — в `FixedUpdate`, а не в `Update`: это
            // решение симуляции, и в `Update` оно шло по разу на кадр, то есть
            // зависело от fps. Демонов сотни, лишних прогонов эта система не
            // боится
            .add_systems(
                FixedUpdate,
                (pick_wander_targets, acquire_targets, chase, devour)
                    .chain()
                    .in_set(SimSet::DemonBehavior),
            )
            .add_systems(
                Update,
                (
                    // косметика: пульсация масштаба спрайта и отрисовка
                    // траектории броска — на симуляцию не влияют
                    pulse_devouring,
                    draw_lunge_paths,
                    sync_demon_speed.run_if(resource_changed::<DemonStyle>),
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

/// Новый прогон мира — спавнер с нуля: `spawned = 0` возвращает демонам те же
/// номера, а значит и те же потоки ГПСЧ (см. `src/rng.rs`).
fn on_world_started(_event: On<WorldStarted>, mut spawner: ResMut<DemonSpawner>) {
    *spawner = DemonSpawner::default();
}
