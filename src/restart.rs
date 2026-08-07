//! Рестарт сцены по R [Q17]: despawn всех сущностей сцены + сброс ресурсов →
//! мир отстраивается заново (залп демонов — спавнером, люди — здесь).

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::demon::{Demon, DemonSpawner};
use crate::dev::TestWalker;
use crate::human::{CorpseTag, Human, HumanStyle, spawn_population};
use crate::loading::AppState;
use crate::navigation::ArcNavmesh;
use crate::rng::WorldSeed;
use crate::telemetry::Telemetry;

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct RestartEvent;

/// «Рестарт заказан, выполнить в ближайшем `PreUpdate`».
///
/// Единственный способ попросить рестарт откуда угодно, кроме клавиши R:
/// смена seed'а, переключение детерминированного режима, запись по BRP. Прямо
/// триггерить [`RestartEvent`] из `Update` нельзя — `on_restart` сносит сцену
/// в обсервере, и команды соседних систем того же расписания применились бы к
/// уже мёртвым сущностям (см. комментарий к расписанию ниже).
#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource, Default)]
pub struct RestartPending(pub bool);

pub struct RestartPlugin;

impl Plugin for RestartPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<RestartEvent>()
            .register_type::<RestartPending>()
            .init_resource::<RestartPending>()
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
                (
                    trigger_restart
                        .run_if(
                            input_just_pressed(KeyCode::KeyR).and_then(in_state(AppState::Playing)),
                        )
                        // «r» в поле ввода — это буква, а не рестарт мира
                        .run_if(not(crate::ui::typing_in_text_input)),
                    trigger_pending_restart
                        .run_if(in_state(AppState::Playing))
                        .run_if(|pending: Res<RestartPending>| pending.0),
                )
                    .after(bevy::input::InputSystems),
            );
    }
}

fn trigger_restart(mut commands: Commands) {
    commands.trigger(RestartEvent);
}

/// Отложенный рестарт, заказанный через [`RestartPending`], — в том же слоте
/// расписания, что и клавиша R, и ровно по той же причине.
fn trigger_pending_restart(mut commands: Commands, mut pending: ResMut<RestartPending>) {
    pending.0 = false;
    commands.trigger(RestartEvent);
}

#[allow(clippy::too_many_arguments)]
fn on_restart(
    _event: On<RestartEvent>,
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    mut telemetry: ResMut<Telemetry>,
    arc_navmesh: Res<ArcNavmesh>,
    style: Res<HumanStyle>,
    seed: Res<WorldSeed>,
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

    // состояние ГПСЧ сбрасывать нечего: все потоки выводятся из `WorldSeed` и
    // `PawnId`, а `spawned = 0` у сброшенного спавнера возвращает демонам те
    // же номера — значит, и те же потоки (см. `src/rng.rs`)
    spawn_population(&mut commands, &arc_navmesh.read(), style.spread, seed.0);
}
