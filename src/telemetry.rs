//! Счётчики исходов [Q12]: жив / убит / спасся. Живые считаются запросом
//! `count Human` по BRP; здесь — накопительные счётчики событий.
//! Инвариант: `alive + killed + escaped == HUMAN_COUNT`.

use bevy::prelude::*;

use crate::loading::WorldStarted;

#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct Telemetry {
    pub killed: usize,
    pub escaped: usize,
}

pub struct TelemetryPlugin;

impl Plugin for TelemetryPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Telemetry>()
            .init_resource::<Telemetry>()
            .add_observer(on_world_started);
    }
}

/// Новый прогон мира — счётчики исходов с нуля.
fn on_world_started(_event: On<WorldStarted>, mut telemetry: ResMut<Telemetry>) {
    *telemetry = Telemetry::default();
}
