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
//! | бэкенд навигации | ресурс `Backend` переснимается каждый кадр: достроился northstar — переключились на ходу | тот же ресурс, но записан один раз на `WorldStarted` и заморожен на весь прогон |
//! | постройка иерархии | стартует после прогрева, чтобы не отбирать ядра | стартует до прогрева: прогон обязан целиком пройти на одном бэкенде |
//! | разброс скоростей (ползунок) | применяется в кадре правки | на ближайшем тике |
//! | расталкивание пешек | работает | выключено (косметика, завязанная на камеру и `FrameCount`) |
//!
//! В расписании эта таблица записана двумя множествами —
//! [`SimPipeline::Live`] и [`SimPipeline::Deterministic`]; система объявляет
//! свою ветку, а гейтятся ветки один раз здесь. Единственное исключение —
//! `separation_runs`, которому режим нужен и в отрицании (док у условия).
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

pub mod replay;

use bevy::ecs::schedule::ScheduleLabel;

use crate::loading::{AppState, PlayPhase, WorldStarted, in_world};
use crate::navigation::Pathfinder;
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
    /// Повторяемый: всё по тикам, бэкенд заморожен, очередь FIFO.
    Deterministic,
    /// Работает в обоих режимах. Существует ради **гейта на мир**: множества
    /// гейтятся здесь разом, и система, состоящая в любом из трёх, сама
    /// `in_state` больше не пишет.
    ///
    /// Размен, который стоит знать: раньше «система без множества» читалась как
    /// «идёт в обоих режимах», и это был полезный сигнал. Теперь без множества
    /// остаются только системы, живущие **вне** мира, — а «в обоих режимах»
    /// приходится объявлять явно. Взамен исчезает целый класс отказов: гейт на
    /// мир нельзя забыть, потому что его никто и не пишет.
    BothModes,
}

/// Все три ветки разом — для одного расписания.
///
/// Два условия, и оба считаются по разу на множество за прогон расписания, а не
/// по разу на систему:
///
/// * **режим** — ветка таблицы, своя у `Live` и `Deterministic`;
/// * **мир** — [`in_world`], общий на все три. Вне мира нет ресурсов мира, и
///   система симуляции валится на валидации параметров; это уже роняло запуск,
///   когда живая цепочка поиска пути отработала в `Loading`.
fn gate_pipelines<S: ScheduleLabel>(app: &mut App, schedule: S) {
    app.configure_sets(
        schedule,
        (
            SimPipeline::Live.run_if(not(deterministic)),
            SimPipeline::Deterministic.run_if(deterministic),
            SimPipeline::BothModes,
        )
            .run_if(in_world),
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

pub struct DeterminismPlugin;

impl Plugin for DeterminismPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Determinism>()
            .register_type::<SimTick>()
            .init_resource::<Determinism>()
            .track_pref::<Determinism>()
            .init_resource::<SimTick>()
            // просит `request_restart_on_config_change`, а живёт в
            // `RestartPlugin`, которого в тестах и демо-сценах может не быть:
            // без ресурса система не прошла бы валидацию параметров.
            // `init_resource` идемпотентен — настоящий ресурс это не подменяет
            .init_resource::<RestartPending>()
            .add_observer(on_world_started);

        // Каждое расписание, где ветки встречаются, гейтится отдельно —
        // конфигурация множества принадлежит расписанию, а не приложению.
        // `OnEnter` тоже здесь: постройка иерархии стартует в разных фазах
        // (`Live` — после прогрева, `Deterministic` — до него), и это ровно
        // такой же выбор ветки, как и всё остальное в таблице
        gate_pipelines(app, PreUpdate);
        gate_pipelines(app, Update);
        gate_pipelines(app, FixedUpdate);
        gate_pipelines(app, OnEnter(PlayPhase::Live));
        gate_pipelines(app, OnEnter(PlayPhase::Warmup));

        app.add_systems(
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

/// Новый прогон: тики с нуля, бэкенд — свежим снимком.
///
/// Это и есть **детерминированная политика** ресурса
/// [`Backend`](crate::navigation::Backend): постройка
/// northstar и polymesh идёт в фоне и заканчивается в момент **реального**
/// времени, поэтому переснимать снимок по ходу прогона нельзя — повтор того
/// же seed'а переключился бы на другом тике, то есть на разных путях при
/// одинаковых входах. Снимок берётся один раз на входе в мир и держится до
/// конца прогона: в этом режиме `refresh_backend` не запускается.
///
/// Пишется в обоих режимах, и в живом это не лишнее, а страховка: там снимок
/// поверх кладёт `refresh_backend` в том же кадре. На входе в `Live` нужный
/// бэкенд уже построен (в детерминированном режиме прогрев его дожидается,
/// см. `loading.rs::poll_warmup`), а на рестарте по R снимок просто берётся
/// заново с той же карты.
/// Ресурс вставляется командой, а не пишется через `ResMut`: объявить старт
/// мира можно и до того, как он появился (`replay.rs` объявляет его раньше
/// входа в `Playing`), а `ResMut` на отсутствующий ресурс завалил бы
/// валидацию параметров — наблюдатель молча не отработал бы весь, вместе со
/// сбросом тиков.
fn on_world_started(
    _event: On<WorldStarted>,
    mut commands: Commands,
    mut tick: ResMut<SimTick>,
    pathfinder: Pathfinder,
) {
    tick.0 = 0;
    commands.insert_resource(pathfinder.backend());
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::grid::tile_center;
    use crate::navigation::{ArcNavmesh, Backend, Navmesh, use_flat_grid};

    /// Тайл, по которому видно, ЧЕЙ снимок лежит ресурсом.
    const PROBE: IVec2 = IVec2::ZERO;

    fn navmesh_with_probe(passable: bool) -> Arc<RwLock<Navmesh>> {
        let mut navmesh = Navmesh::default();
        navmesh.set_passable(PROBE.x, PROBE.y, passable);
        Arc::new(RwLock::new(navmesh))
    }

    /// Замороженный снимок берётся **объявлением старта**, а не входом в мир.
    ///
    /// Разница не теоретическая: `navigation::insert_backend` сеет снимок на
    /// `OnEnter(Playing)`, когда ни иерархии northstar, ни полигонального меша
    /// ещё нет — их постройка стартует в том же `OnEnter`. К объявлению они
    /// готовы (прогрев их дожидается), а на рестарте по R — тем более. Прогон,
    /// донёсший до конца посевной снимок, шёл бы не тем бэкендом, что
    /// показывает панель; ровно этот класс дефекта уже случался (см. заголовок
    /// `tests/determinism.rs`).
    ///
    /// Мир под снимком здесь подменяется целиком, а не алгоритм: снимок
    /// собирается одним вызовом `Pathfinder::backend`, поэтому «пересняли» —
    /// свойство наблюдаемое по любому его полю, а проходимость видна снаружи
    /// без новых геттеров.
    #[test]
    fn the_frozen_backend_is_snapped_at_the_announcement_not_at_world_entry() {
        let mut world = World::new();
        world.init_resource::<SimTick>();
        // бэкенд здесь неважен, важно ЧЕЙ мир под ним; ресурсы навигации нужны
        // лишь чтобы снимок вообще собрался
        use_flat_grid(&mut world);
        world.add_observer(on_world_started);

        // посев на входе в мир: проба проходима
        world.insert_resource(Backend::from_grid(navmesh_with_probe(true)));
        // к объявлению мир уже другой
        world.insert_resource(ArcNavmesh(navmesh_with_probe(false)));

        world.trigger(WorldStarted);
        world.flush();

        assert!(
            !world
                .resource::<Backend>()
                .walkable()
                .allows(tile_center(PROBE)),
            "прогон унёс посевной снимок вместо снятого на объявлении"
        );
    }

    /// Вторая половина того же обсервера — и она уже закреплена снаружи
    /// (`tests/determinism.rs::a_restart_replays_the_run`); здесь — вблизи, на
    /// том же стенде, чтобы обе половины падали адресно.
    #[test]
    fn a_new_run_counts_ticks_from_zero() {
        let mut world = World::new();
        world.init_resource::<SimTick>();
        use_flat_grid(&mut world);
        world.insert_resource(ArcNavmesh(navmesh_with_probe(true)));
        world.add_observer(on_world_started);

        world.resource_mut::<SimTick>().0 = 4096;
        world.trigger(WorldStarted);

        assert_eq!(world.resource::<SimTick>().0, 0);
    }
}
