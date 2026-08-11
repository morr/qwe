mod behavior;
mod components;
mod decide;
mod systems;

use bevy::prelude::*;

use self::behavior::{escape, flee, panic};
pub use self::components::{
    CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag,
    Pace, PanicRecoil, PopulationSize, WanderHeading, WanderPause,
};
// `pick_wander_targets` наружу — им пользуется демо-сцена расталкивания
// (`examples/demos/crowd_demo.rs`), чтобы гонять толпу настоящим блужданием, а
// не своей выдумкой; `HumanPlugin` целиком ей не подходит (его `spawn_humans`
// расселил бы 20 000 пешек по всей карте)
pub use self::systems::{pick_wander_targets, spawn_population};
use self::systems::{spawn_humans, sync_human_pace};
use crate::determinism::{DeterminismPlugin, SimPipeline};
use crate::loading::{AppState, WorldInitSet};
use crate::prefs::TrackPrefExt;
use crate::spatial::SimSet;

pub struct HumanPlugin;

impl Plugin for HumanPlugin {
    fn build(&self, app: &mut App) {
        // ветки конвейера гейтит `DeterminismPlugin` — без него обе работали
        // бы разом; тот же довод, что и в `MovementPlugin`
        if !app.is_plugin_added::<DeterminismPlugin>() {
            app.add_plugins(DeterminismPlugin);
        }

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
            .track_pref::<HumanStyle>()
            .init_resource::<PopulationSize>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_humans.in_set(WorldInitSet::Spawn),
            )
            // Выбор целей и разброс скоростей зарегистрированы дважды, и это
            // осознанно: одна и та же работа в двух ветках конвейера
            // (`SimPipeline`), а какая из них поедет — решает не эта пара
            // строк, а гейт в `DeterminismPlugin`.
            //
            // В детерминированном режиме обе обязаны идти по тикам: в `Update`
            // и число прогонов за секунду, и дельта, которой заводится
            // `WanderPause`, зависят от fps, а правка ползунка разброса легла
            // бы между разными тиками при разном fps.
            //
            // В обычном режиме обе остаются в `Update`: там на выборе целей
            // держится прогрев (`loading.rs::poll_warmup` ждёт заявок, а
            // `FixedUpdate` на паузе прогрева не идёт вовсе) и покадровый ритм,
            // к которому подогнаны гейт видимости диспетчера и лимит заявок, а
            // ползунок обязан отзываться и на паузе симуляции.
            .add_systems(
                FixedUpdate,
                (
                    pick_wander_targets.in_set(SimPipeline::Deterministic),
                    // ползунок разброса — на тике, а не в кадре: скорости
                    // пешек входят в состояние прогона, и правка, применённая
                    // в кадре, легла бы между разными тиками при разном fps
                    sync_human_pace
                        .run_if(resource_changed::<HumanStyle>)
                        .in_set(SimPipeline::Deterministic),
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
                    pick_wander_targets,
                    // ползунок разброса уже вышедшим людям — только на смену
                    // ресурса, проход по всей популяции каждый кадр не нужен
                    sync_human_pace.run_if(resource_changed::<HumanStyle>),
                )
                    .run_if(in_state(AppState::Playing))
                    .in_set(SimPipeline::Live),
            );
    }
}
