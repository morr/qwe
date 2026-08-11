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
//! | постройка иерархии | стартует после прогрева, чтобы не отбирать ядра | стартует до прогрева: прогон обязан целиком пройти на одном бэкенде |
//! | разброс скоростей (ползунок) | применяется в кадре правки | на ближайшем тике |
//! | расталкивание пешек | работает | выключено (косметика, завязанная на камеру и `FrameCount`) |
//!
//! В расписании эта таблица записана двумя множествами — [`SimPipeline::Live`]
//! и [`SimPipeline::Locked`]; система объявляет свою ветку, а гейтятся ветки
//! один раз здесь. Единственное исключение — `separation_runs`, которому режим
//! нужен и в отрицании (док у самого условия).
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

use bevy::ecs::schedule::ScheduleLabel;

use crate::loading::{AppState, PlayPhase, WorldStarted};
use crate::navigation::{Backend, Pathfinder};
use crate::prefs::{TrackPrefExt, retuned};
use crate::restart::RestartPending;
use crate::rng::WorldSeed;

/// Тумблер `Deterministic` в панели World.
#[derive(Resource, Reflect, SettingsGroup, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "world", key = "deterministic")]
pub struct Determinism(pub bool);

/// Условие расписания «детерминированный режим включён».
///
/// Прямо в `run_if` системы ему делать нечего — для этого есть
/// [`SimPipeline`]. Оно нужно там, где режим — лишь часть более сложного
/// правила и множеством не выражается: [`separation_runs`] требует
/// **отрицания** («расталкивания нет — почисти его следы»), а отрицать
/// множество нельзя.
///
/// [`separation_runs`]: crate::movement::separation::separation_runs
pub fn deterministic(mode: Res<Determinism>) -> bool {
    mode.0
}

/// Две ветки конвейера прогона. Тумблер не выбирает «делать или не делать» —
/// он выбирает, **каким способом** делается одно и то же: где стартует
/// постройка иерархии, где выбираются цели блуждания, каким конвейером ходит
/// поиск пути (таблица режимов — в шапке модуля).
///
/// Система объявляет свою ветку и про режим не знает вовсе; гейт стоит один
/// раз здесь, в [`DeterminismPlugin`], на каждое расписание, где ветки
/// встречаются. Забыть `run_if` больше нельзя — забыть можно только сет, а
/// система без сета видна тем, что работает в обоих режимах сразу.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimPipeline {
    /// Обычный прогон: покадровый ритм, живой бэкенд, приоритет от камеры.
    Live,
    /// Детерминированный: всё по тикам, бэкенд заморожен, очередь FIFO.
    Locked,
}

/// Обе ветки разом — для одного расписания. Условие считается по разу на
/// множество за прогон расписания, а не по разу на систему, как считался
/// прежний `run_if` на каждой точке.
fn gate_pipelines<S: ScheduleLabel>(app: &mut App, schedule: S) {
    app.configure_sets(
        schedule,
        (
            SimPipeline::Live.run_if(not(deterministic)),
            SimPipeline::Locked.run_if(deterministic),
        ),
    );
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
            .track_pref::<Determinism>()
            .init_resource::<SimTick>()
            .init_resource::<DeterministicRun>()
            // просит `request_restart_on_config_change`, а живёт в
            // `RestartPlugin`, которого в тестах и демо-сценах может не быть:
            // без ресурса система не прошла бы валидацию параметров.
            // `init_resource` идемпотентен — настоящий ресурс это не подменяет
            .init_resource::<RestartPending>()
            .add_observer(on_world_started);

        // Каждое расписание, где ветки встречаются, гейтится отдельно —
        // конфигурация множества принадлежит расписанию, а не приложению.
        // `OnEnter` тоже здесь: постройка иерархии стартует в разных фазах
        // (`Live` — после прогрева, `Locked` — до него), и это ровно такой же
        // выбор ветки, как и всё остальное в таблице
        gate_pipelines(app, Update);
        gate_pipelines(app, FixedUpdate);
        gate_pipelines(app, OnEnter(PlayPhase::Live));
        gate_pipelines(app, OnEnter(PlayPhase::Warmup));

        app
            // снимок держит Arc'и навмеша, иерархии и меша — оставить его на
            // смену города значит продержать геометрию старого мира в памяти
            // всю загрузку нового; свежий снимок возьмёт `WorldStarted`
            .add_systems(OnExit(AppState::Playing), drop_frozen_backend)
            .add_systems(
                Update,
                request_restart_on_config_change
                    .run_if(in_state(AppState::Playing))
                    // `retuned` отсекает восстановление настроек на сборке
                    // `App`: по голому `resource_changed` первый же кадр мира
                    // просил бы рестарт сам себе
                    .run_if(retuned::<WorldSeed>.or_else(retuned::<Determinism>)),
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
