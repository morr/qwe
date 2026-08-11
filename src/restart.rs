//! Рестарт сцены по R [Q17]: despawn всех сущностей сцены + сброс ресурсов →
//! мир отстраивается заново (залп демонов — спавнером, люди — здесь).

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::demon::Demon;
use crate::dev::TestWalker;
use crate::human::{CorpseTag, Human, HumanStyle, spawn_population};
use crate::loading::{AppState, WorldStarted};
use crate::navigation::ArcNavmesh;
use crate::rng::WorldSeed;

// `Default` в reflect-регистрации — для BRP: `brp event RestartEvent` без
// аргументов конструирует значение через `ReflectDefault`, и без него запрос
// валит приложение паникой в `process_remote_requests`
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event, Default)]
pub struct RestartEvent {
    /// Увезти камеру к порталу, как второе `R` подряд.
    ///
    /// Одиночное `R` оставляет камеру там, где велит настройка `position`:
    /// пользователь разглядывал участок карты и хочет разглядывать его
    /// дальше. Рестарт же по смене настройки мира (seed, тумблер
    /// детерминизма) — это **другой мир**, а не тот же сначала: смотреть на
    /// прежний участок незачем, и без переезда камеры перезапуск вообще
    /// незаметен — толпа на экране выглядит так же, и правка настройки
    /// читается как «ничего не произошло».
    pub to_portal: bool,
}

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
    // одиночное R камеру не двигает; портал — это второе R подряд, и его
    // ловит по времени `camera.rs::on_restart_place_camera`
    commands.trigger(RestartEvent { to_portal: false });
}

/// Отложенный рестарт, заказанный через [`RestartPending`], — в том же слоте
/// расписания, что и клавиша R, и ровно по той же причине.
///
/// Всегда «к порталу»: этот путь — только смена настройки мира, то есть
/// новый мир, а не текущий сначала (см. [`RestartEvent::to_portal`]).
fn trigger_pending_restart(mut commands: Commands, mut pending: ResMut<RestartPending>) {
    pending.0 = false;
    commands.trigger(RestartEvent { to_portal: true });
}

fn on_restart(
    _event: On<RestartEvent>,
    mut commands: Commands,
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

    // всё состояние прогона — спавнер, счётчики, часы, тики, замороженный
    // бэкенд — сбрасывают обсерверы `WorldStarted`, каждый в своём модуле;
    // триггер стоит до `spawn_population`, так что сбросы применяются раньше,
    // чем лягут команды спавна
    commands.trigger(WorldStarted);

    // состояние ГПСЧ сбрасывать нечего: все потоки выводятся из `WorldSeed` и
    // `PawnId`, а `spawned = 0` у сброшенного спавнера возвращает демонам те
    // же номера — значит, и те же потоки (см. `src/rng.rs`)
    spawn_population(&mut commands, &arc_navmesh.read(), style.spread, seed.0);
}
