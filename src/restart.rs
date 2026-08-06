//! Рестарт сцены по R [Q17]: despawn всех сущностей сцены + сброс ресурсов →
//! мир отстраивается заново (залп демонов — спавнером, люди — здесь).

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::demon::{Demon, DemonSpawner};
use crate::dev::TestWalker;
use crate::human::{CorpseTag, Human, HumanStyle, spawn_population};
use crate::loading::AppState;
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
            // `PreUpdate`, а не `Update`: рестарт сносит сцену прямо в
            // обсервере, и из середины `Update` он убивал сущности, на которые
            // уже собраны, но ещё не применены команды соседних систем того же
            // расписания (`pick_wander_targets`, диспетчер и приёмник поиска
            // пути) — их буфер применялся к мёртвой сущности и ронял
            // приложение. В `PreUpdate` после `InputSystems` несделанных
            // буферов нет вовсе, а спавн всё ещё успевает к распространению
            // трансформов в `PostUpdate` того же кадра.
            .add_systems(
                PreUpdate,
                trigger_restart.after(bevy::input::InputSystems).run_if(
                    input_just_pressed(KeyCode::KeyR).and_then(in_state(AppState::Playing)),
                ),
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
    style: Res<HumanStyle>,
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
    // часы симуляции сбрасывает свой обсервер в `sim_time`

    spawn_population(&mut commands, &arc_navmesh.read(), style.spread);
}
