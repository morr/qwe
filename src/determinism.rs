//! Детерминированный режим: симуляция как чистая функция от
//! [`WorldSeed`](crate::rng::WorldSeed) и настроек.
//!
//! Тумблер гейтит **расписание**, а не жребий: засев ГПСЧ безусловен (см.
//! `src/rng.rs`) и работает в обоих режимах. Разница в другом — в том, что
//! умеет влиять на симуляцию помимо seed'а:
//!
//! | | тумблер выключен | тумблер включён |
//! |---|---|---|
//! | выбор целей блуждания | `Update`, то есть раз в кадр | `FixedUpdate`, раз в тик |
//! | ответ поиска пути | применяется в тот кадр, когда посчитался | ровно на тике `T + PATHFINDING_RETIRE_TICKS` |
//! | приоритет диспетчера | по удалённости от центра кадра, мирные вне экрана не считаются вовсе | FIFO по тику заявки, камера не участвует |
//! | бэкенд навигации | живой: достроился northstar — переключились на ходу | заморожен снимком на весь прогон ([`DeterministicRun`]) |
//! | расталкивание пешек | работает | выключено (косметика, завязанная на камеру и `FrameCount`) |
//!
//! Единица повтора — **тик**, а не кадр. Частота кадров меняет лишь то,
//! сколько тиков успевает пройти за кадр: шаг `Time<Fixed>` постоянен, ответ
//! поиска пути ждёт своего тика, а всё, что осталось в `Update`, только
//! рисует. Поэтому просадка fps замедляет проигрывание, но не меняет
//! содержимое тика N — сравнивать состояния надо по [`SimTick`], а не по
//! настенным часам.
//!
//! Прогон детерминирован или нет с тика 0, поэтому смена тумблера (как и
//! смена seed'а) запрашивает рестарт.

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::loading::{AppState, WorldStarted};
use crate::navigation::{Backend, Pathfinder};
use crate::restart::RestartPending;
use crate::rng::WorldSeed;

/// Тумблер `Deterministic` в панели World.
#[derive(Resource, Reflect, SettingsGroup, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "world", key = "deterministic")]
pub struct Determinism(pub bool);

/// Условие расписания «детерминированный режим включён»; отрицание —
/// `not(deterministic)`.
///
/// `Option`, а не `Res`: плагины движения и людей используются в тестах без
/// `DeterminismPlugin`, и отсутствие ресурса завалило бы валидацию параметров
/// — система молча не выполнилась бы (в отличие от «выполнилась в обычном
/// режиме», чего мы и хотим). Нет ресурса — режим выключен.
pub fn deterministic(mode: Option<Res<Determinism>>) -> bool {
    mode.is_some_and(|mode| mode.0)
}

/// Номер шага симуляции с начала прогона. Единица воспроизведения: состояние
/// мира — функция от `(seed, настройки, SimTick)`.
///
/// Не то же, что [`SimClock`](crate::sim_time::SimClock): часы считают
/// виртуальные секунды и теряют время, отброшенное `max_delta` на долгих
/// кадрах, а тики считают выполненные шаги.
#[derive(Resource, Reflect, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource, Default)]
pub struct SimTick(pub u64);

/// Замороженный на прогон бэкенд навигации.
///
/// Постройка northstar и polymesh идёт в фоне и заканчивается в момент
/// **реального** времени. Живой снимок (`Pathfinder::backend`) переключился
/// бы на готовый бэкенд посреди прогона, и повтор того же seed'а переключился
/// бы на другом тике — разные пути на одинаковых входах. Снимок берётся один
/// раз на входе в мир и держится до конца прогона; `Default` — заглушка
/// пустого мира до первой заморозки.
#[derive(Resource, Default)]
pub struct DeterministicRun(pub Backend);

pub struct DeterminismPlugin;

impl Plugin for DeterminismPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Determinism>()
            .register_type::<SimTick>()
            .init_resource::<Determinism>()
            .init_resource::<SimTick>()
            .init_resource::<DeterministicRun>()
            .add_observer(on_world_started)
            // снимок держит Arc'и навмеша, иерархии и меша — оставить его на
            // смену города значит продержать геометрию старого мира в памяти
            // всю загрузку нового; свежий снимок возьмёт `WorldStarted`
            .add_systems(OnExit(AppState::Playing), drop_frozen_backend)
            .add_systems(
                Update,
                request_restart_on_config_change
                    .run_if(in_state(AppState::Playing))
                    .run_if(
                        // `resource_added` отсекает восстановление настроек на
                        // сборке `App`: без него первый же кадр мира просил бы
                        // рестарт сам себе
                        (resource_changed::<WorldSeed>.and_then(not(resource_added::<WorldSeed>)))
                            .or_else(
                                resource_changed::<Determinism>
                                    .and_then(not(resource_added::<Determinism>)),
                            ),
                    ),
            );
    }
}

/// Новый прогон: тики с нуля, бэкенд — свежим снимком. На входе в `Live`
/// нужный бэкенд уже построен (в детерминированном режиме прогрев его
/// дожидается, см. `loading.rs::poll_warmup`), а на рестарте по R снимок
/// просто берётся заново с той же карты.
fn on_world_started(
    _event: On<WorldStarted>,
    mut tick: ResMut<SimTick>,
    mut run: ResMut<DeterministicRun>,
    pathfinder: Pathfinder,
) {
    tick.0 = 0;
    *run = DeterministicRun(pathfinder.backend());
}

fn drop_frozen_backend(mut run: ResMut<DeterministicRun>) {
    *run = DeterministicRun::default();
}

/// Смена seed'а или режима — новый прогон, а значит рестарт.
///
/// Флаг, а не прямой `commands.trigger(RestartEvent)`: рестарт сносит всю
/// сцену, и из `Update` он убил бы сущности, на которые уже собраны, но ещё
/// не применены команды соседних систем того же расписания. Снимает флаг
/// `restart::trigger_pending_restart` в `PreUpdate` — там несделанных буферов
/// нет вовсе (см. комментарий в `restart.rs`).
fn request_restart_on_config_change(mut pending: ResMut<RestartPending>) {
    pending.0 = true;
}

/// Голова цепочки `FixedUpdate`: тик считается до всякого поведения, чтобы
/// заявка, поданная на этом шаге, и ответ, снятый на нём же, говорили об
/// одном и том же номере.
pub fn advance_sim_tick(mut tick: ResMut<SimTick>) {
    tick.0 += 1;
}
