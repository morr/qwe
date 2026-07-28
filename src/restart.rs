//! Рестарт сцены по R [Q17]: despawn всех сущностей сцены + сброс ресурсов →
//! мир отстраивается заново (залп демонов — спавнером, люди — здесь).

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::demon::{Demon, DemonSpawner};
use crate::dev::TestWalker;
use crate::human::{CorpseTag, Human, spawn_population};
use crate::navigation::ArcNavmesh;
use crate::telemetry::Telemetry;

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct RestartEvent;

pub struct RestartPlugin;

impl Plugin for RestartPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<RestartEvent>()
            .add_observer(on_restart)
            .add_systems(
                Update,
                trigger_restart.run_if(input_just_pressed(KeyCode::KeyR)),
            );
    }
}

fn trigger_restart(mut commands: Commands) {
    commands.trigger(RestartEvent);
}

fn on_restart(
    _event: On<RestartEvent>,
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    mut telemetry: ResMut<Telemetry>,
    arc_navmesh: Res<ArcNavmesh>,
    scene_entities: Query<
        Entity,
        Or<(With<Human>, With<CorpseTag>, With<Demon>, With<TestWalker>)>,
    >,
) {
    let count = scene_entities.iter().count();
    info!("restart: despawning {count} scene entities");

    for entity in &scene_entities {
        commands.entity(entity).despawn();
    }

    *spawner = DemonSpawner::default();
    *telemetry = Telemetry::default();

    spawn_population(&mut commands, &arc_navmesh.read());
}
