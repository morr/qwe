mod behavior;
mod claims;
mod components;
mod decide;
mod horde;
mod look;
mod systems;

use bevy::prelude::*;

use self::behavior::{
    acquire_slice, acquire_targets, chase, devour, on_demon_caught_human, pulse_devouring,
};
pub use self::components::{
    ACQUIRE_SLICE_TICKS, ChaseRepath, ChaseTarget, DEMON_CAP, DEMON_CAP_MAX, DEMON_CAP_MIN,
    DEMON_CAP_STEP, DEMON_LUNGE_BOOST, DEMON_LUNGE_BOOST_MAX, DEMON_LUNGE_BOOST_MIN,
    DEMON_LUNGE_BOOST_STEP, DEMON_SPAWN_INTERVAL, DEMON_SPAWN_INTERVAL_MAX,
    DEMON_SPAWN_INTERVAL_MIN, DEMON_SPAWN_INTERVAL_STEP, DEMON_SPEED_FACTOR,
    DEMON_SPEED_FACTOR_MAX, DEMON_SPEED_FACTOR_MIN, DEMON_SPEED_FACTOR_STEP, Demon,
    DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag, DemonLungeTag, DemonSpawner, DemonStyle,
    DemonWanderTag, DevourUntil, HordeStyle, WAVE_EVERY_DEFAULT, WAVE_EVERY_MAX, WAVE_EVERY_MIN,
    WAVE_EVERY_STEP, WAVE_LEN_DEFAULT, WAVE_LEN_MAX, WAVE_LEN_MIN, WAVE_LEN_STEP,
    WAVE_SIZE_DEFAULT, WAVE_SIZE_MAX, WAVE_SIZE_MIN, WAVE_SIZE_STEP, WarpOrigin,
};
use self::horde::play_waves;
pub use self::horde::{HORDE_WAVE_KEY, HordeSchedule};
pub use self::look::DemonHalo;
use self::systems::{
    demon_speed_changed, draw_lunge_paths, pick_wander_targets, spawn_initial_burst,
    sync_demon_speed, tick_spawner,
};
use crate::determinism::{DeterminismPlugin, SimPipeline};
use crate::loading::{PlayPhase, WorldStarted};
use crate::prefs::PrefsExt;
use crate::tick_stage::TickStage;

pub struct DemonPlugin;

impl Plugin for DemonPlugin {
    fn build(&self, app: &mut App) {
        // Множества конвейера гейтит `DeterminismPlugin`, и без него
        // незасетапленное множество не гейтит **ничего** — включая гейт на мир.
        // Та же зависимость, что у плагинов движения, людей и навигации, и
        // объявляется так же здесь: демонов поднимают и отдельно, в тестах
        if !app.is_plugin_added::<DeterminismPlugin>() {
            app.add_plugins(DeterminismPlugin);
        }

        app.register_type::<Demon>()
            .register_type::<DemonWanderTag>()
            .register_type::<DemonChaseTag>()
            .register_type::<DemonDevourTag>()
            .register_type::<DemonLungeTag>()
            .register_type::<ChaseTarget>()
            .register_type::<ChaseRepath>()
            .register_type::<DevourUntil>()
            .register_type::<DemonSpawner>()
            .register_type::<WarpOrigin>()
            .register_type::<HordeSchedule>()
            .register_type::<DemonHalo>()
            .register_pref::<DemonStyle>()
            .register_pref::<HordeStyle>()
            .init_resource::<DemonSpawner>()
            .init_resource::<HordeSchedule>()
            // атлас силуэтов собирает `SilhouettePlugin`; пустой ресурс —
            // квадраты, как и у людей (см. `HumanPlugin`)
            .init_resource::<crate::silhouette::Silhouettes>()
            .add_observer(on_demon_caught_human)
            .add_observer(on_world_started)
            .add_observer(crate::demon::horde::on_world_started)
            // `Live`, а не `Playing`: номера демонам раздаёт спавнер, а обнуляет
            // его `WorldStarted` — объявляемый на входе в `Live`. Тик, успевший
            // пройти раньше объявления, выпускает залп со старым счётчиком, и
            // сброс раздаёт те же `PawnId` второму залпу: в мире оказываются два
            // демона №1, а на них стоит весь детерминизм (поток ГПСЧ пешки, ключ
            // очереди диспетчера). До сих пор этого не случалось только потому,
            // что прогрев держит мир на паузе, — но пауза принадлежит
            // `sim_time`, снимается пробелом и в детерминированном режиме длится
            // все 11–14 с постройки навигации.
            .add_systems(
                FixedUpdate,
                (spawn_initial_burst, tick_spawner, play_waves)
                    .chain()
                    // рождение — стадия `Spawner`, до пересборки сетки демонов:
                    // ребро к следующей стадии ставит точку синхронизации,
                    // команды спавна применяются на ней, и демон входит в сетку
                    // и действует на том же тике, на котором родился. Без ребра
                    // порядок решал топологический разбор расписания и гонка с
                    // точкой синхронизации соседей. Залп, фон и волна — одна
                    // цепочка, один класс порядка
                    .in_set(TickStage::Spawner)
                    // спавнер идёт в обоих режимах и живёт внутри мира; с
                    // `62ea098` «без множества» читается как «вне мира»
                    .in_set(SimPipeline::BothModes)
                    .run_if(in_state(PlayPhase::Live)),
            )
            // выбор цели блуждания — в `FixedUpdate`, а не в `Update`: это
            // решение симуляции, и в `Update` оно шло по разу на кадр, то есть
            // зависело от fps. Демонов сотни, лишних прогонов эта система не
            // боится
            .add_systems(
                FixedUpdate,
                (
                    // ползунок скорости — на тике по той же причине, что и
                    // разброс людей: `Movable::speed` входит в состояние
                    // прогона, и правка, применённая в кадре, легла бы между
                    // разными тиками при разном fps
                    // порядок против веток поведения держим прямым ребром:
                    // внешней цепочки у кортежа больше нет (см. ниже), а
                    // «кто первый» у исполнителя — скрытое состояние
                    sync_demon_speed
                        .run_if(demon_speed_changed)
                        .in_set(SimPipeline::Deterministic)
                        .before(pick_wander_targets),
                    // ветка объявлена явно: `pick_wander_targets` и `chase`
                    // берут `Res<Backend>`, а гейт на мир приезжает только с
                    // множеством конвейера. `TickStage::DemonBehavior` держит
                    // порядок (демоны раньше людей), но стадии — только
                    // порядок: без ветки конвейера множество не гейтит ничего
                    (pick_wander_targets, chase, devour)
                        .chain()
                        .in_set(SimPipeline::BothModes),
                    // acquisition целиком — только детерминизм (реплей не
                    // знает про живые срезы; паттерн фазы 2 плана ×10), в
                    // живом режиме его место занимают четверти ниже. Явные
                    // рёбра к концам цепочки: цель блуждания выбирается до
                    // перевода в погоню, свежая заявка `ChaseTarget`
                    // применяется точкой синхронизации до `chase`. Ветки —
                    // СОСЕДИ кортежа, не его элементы, и внешней цепочки у
                    // кортежа нет: ребро цепочки легло бы после всей тройки
                    // (в том числе после `chase`), и явное ребро назад в
                    // `chase` замкнуло бы отношение к множеству, членом
                    // которого `chase` остаётся по своему же `SystemTypeSet`
                    // — CrossDependency при инициализации расписания
                    acquire_targets
                        .in_set(SimPipeline::Deterministic)
                        .after(pick_wander_targets)
                        .before(chase),
                    (
                        acquire_slice::<0>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0.is_multiple_of(ACQUIRE_SLICE_TICKS)
                        }),
                        acquire_slice::<1>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % ACQUIRE_SLICE_TICKS == 1
                        }),
                        acquire_slice::<2>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % ACQUIRE_SLICE_TICKS == 2
                        }),
                        acquire_slice::<3>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % ACQUIRE_SLICE_TICKS == 3
                        }),
                    )
                        // срезы не читают писем друг друга (между собой на тике
                        // работает один), точка синхронизации одна — перед
                        // `chase`, читающим `ChaseTarget`
                        .chain_ignore_deferred()
                        .in_set(SimPipeline::Live)
                        .after(pick_wander_targets)
                        .before(chase),
                )
                    .in_set(TickStage::DemonBehavior),
            )
            .add_systems(
                Update,
                (
                    // косметика: пульсация масштаба спрайта и отрисовка
                    // траектории броска — на симуляцию не влияют
                    pulse_devouring,
                    draw_lunge_paths,
                )
                    // косметика идёт в обоих режимах, но только внутри мира —
                    // и то и другое теперь сказано одним множеством
                    .in_set(SimPipeline::BothModes),
            )
            // в обычном режиме ползунок обязан отзываться и на паузе
            // симуляции — как разброс людей
            .add_systems(
                Update,
                sync_demon_speed
                    .run_if(demon_speed_changed)
                    .in_set(SimPipeline::Live),
            );
    }
}

/// Новый прогон мира — спавнер с нуля: `spawned = 0` возвращает демонам те же
/// номера, а значит и те же потоки ГПСЧ (см. `src/rng.rs`).
///
/// Ровно поэтому у сброса есть предусловие: **живых демонов быть не должно**.
/// Возврат номеров безопасен лишь тогда, когда прежние их владельцы уже сняты
/// со сцены; иначе в мире оказываются два демона с одним `PawnId`, а на его
/// уникальности стоят и поток ГПСЧ пешки, и ключ очереди детерминированного
/// диспетчера (`movement::order::pawn_key`). Все три штатных пути сброса —
/// вход в `Live`, R и смена города — снимают демонов раньше события; жалоба
/// здесь означает четвёртый путь, а не мелочь в счёте.
fn on_world_started(
    _event: On<WorldStarted>,
    mut spawner: ResMut<DemonSpawner>,
    alive: Query<(), With<Demon>>,
) {
    let alive = alive.iter().count();
    if alive > 0 {
        warn!("world start with {alive} demons still alive: their PawnIds are about to be reused");
    }
    debug_assert_eq!(alive, 0, "сброс спавнера под живыми демонами");
    *spawner = DemonSpawner::default();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use bevy::ecs::schedule::ScheduleLabel;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::loading::AppState;
    use crate::navigation::{ArcNavmesh, Backend, Navmesh};
    use crate::portal::PortalPos;
    use crate::rng::PawnId;
    use crate::settings::{DEMON_INITIAL_BURST, MAP_SIZE};
    use crate::spatial::SpatialGrid;

    /// Расписание берётся у самого [`DemonPlugin`]: гейт фазы и есть то, что
    /// проверяется, и повторять его регистрацию в тесте значило бы проверять
    /// копию вместо оригинала.
    fn app() -> App {
        let mut app = App::new();
        // косметика (`draw_lunge_paths`) просит `Gizmos` из рендера, которого
        // здесь нет; симуляции она не касается
        app.set_error_handler(bevy::ecs::error::warn);
        let navmesh = Arc::new(RwLock::new(Navmesh::default()));
        // `SpatialPlugin` — не декорация: без него цепочка стадий тика
        // (`TickStage`) не настроена, и проверять порядок было бы не на чем.
        // Он же заводит обе сетки, поэтому
        // ручной `init_resource` для людской ушёл
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            DemonPlugin,
            crate::spatial::SpatialPlugin,
        ))
        // без ручного шага кадр длится микросекунды, `Time<Fixed>` не
        // набирает ни одного шага — и обе половины теста были бы пусты не
        // потому, что залпа нет, а потому, что тиков нет
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .insert_resource(Backend::from_grid(navmesh.clone()))
        .insert_resource(ArcNavmesh(navmesh))
        .insert_resource(PortalPos(Vec2::splat(100.0)))
        .insert_resource(crate::rng::WorldSeed(1))
        .init_resource::<bevy::diagnostic::DiagnosticsStore>()
        .init_resource::<crate::telemetry::Telemetry>()
        .add_plugins(crate::loading::SimBootPlugin);
        app
    }

    fn demon_pawn_ids(app: &mut App) -> Vec<u32> {
        let mut query = app.world_mut().query_filtered::<&PawnId, With<Demon>>();
        let mut ids: Vec<u32> = query.iter(app.world()).map(|pawn_id| pawn_id.0).collect();
        ids.sort_unstable();
        ids
    }

    /// Сколько демонов лежит в сетке — по всей карте, а не вокруг точки:
    /// `SimPosition` новорождённого заполняет обсервер `MovementPlugin`, а его
    /// в этом дворе нет, и место в сетке здесь ничего не значит. Проверяется
    /// членство, а не координата.
    fn demons_in_grid(app: &App) -> usize {
        let mut seen = 0;
        app.world()
            .resource::<SpatialGrid<Demon>>()
            .for_each_in_rect(Vec2::ZERO, MAP_SIZE, |_| seen += 1);
        seen
    }

    /// Пары систем, чьи доступы пересекаются, а взаимный порядок расписание не
    /// задаёт. Считает их сама постройка графа, поэтому расписание достаточно
    /// **проинициализировать** — гонять системы не нужно (а и нечем: они
    /// просят ресурсы мира). Расписание при этом уезжает из `Schedules`
    /// насовсем: на каждую метку вызов один, приложение после проверки не
    /// запускается.
    fn unordered_conflicts(app: &mut App, label: impl ScheduleLabel) -> Vec<String> {
        let world = app.world_mut();
        let mut schedule = world
            .resource_mut::<Schedules>()
            .remove(label)
            .expect("расписание заведено плагином");
        schedule.initialize(world).expect("расписание строится");
        schedule
            .graph()
            .conflicting_systems()
            .to_string(schedule.graph(), world.components())
            .map(|(a, b, on)| format!("{a} ↔ {b} (по {on:?})"))
            .collect()
    }

    /// Ветки демонского конвейера упорядочены изнутри: пара с пересекающимися
    /// доступами обязана иметь явное ребро — «кто первый» у исполнителя
    /// скрытое состояние (доктрины рёбер stage-кортежа выше). Стадия порядок
    /// между своими членами не задаёт, и глобальный свив ambiguities
    /// (обзор №27) был временной врезкой, а не пином. Зеркало пина
    /// человеческой ветки — `human::tests::neither_branch_leaves_the_wander_pair_unordered`.
    #[test]
    fn neither_demon_branch_leaves_a_pair_unordered() {
        let mut app = App::new();
        // `SpatialPlugin` — как в дворе выше: без него цепочка стадий тика не
        // настроена, и пин проверял бы не тот граф, что крутит игра
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            DemonPlugin,
            crate::spatial::SpatialPlugin,
        ));

        assert!(
            unordered_conflicts(&mut app, Update).is_empty(),
            "Update branch has unordered conflicting systems"
        );
        assert!(
            unordered_conflicts(&mut app, FixedUpdate).is_empty(),
            "FixedUpdate branch has unordered conflicting systems"
        );
    }

    /// Прогрев, в котором мир поехал: пауза принадлежит `sim_time` и снимается
    /// пробелом, а в детерминированном режиме прогрев длится все 11–14 с
    /// постройки навигации — так что тик до объявления старта не гипотеза.
    fn tick_through_a_moving_warmup(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        for _ in 0..4 {
            app.update();
        }
    }

    /// Регрессия: два демона №1 в одном мире.
    ///
    /// Номера раздаёт спавнер, а обнуляет его `WorldStarted` на входе в `Live`.
    /// Залп, успевший пройти раньше объявления, доставался второму залпу теми
    /// же `PawnId` — и `dispatch_pathfinding_requests_deterministic` падал на
    /// неуникальном ключе очереди тиков через тридцать после старта.
    #[test]
    fn a_warmup_that_ticks_does_not_hand_out_a_pawn_id_twice() {
        let app = &mut app();
        tick_through_a_moving_warmup(app);
        // без этого тест был бы пуст: демонов нет не потому, что их не пустил
        // гейт, а потому, что `FixedUpdate` вообще не прогонялся
        assert!(
            app.world().resource::<Time<Fixed>>().elapsed_secs() > 0.0,
            "прогрев обязан тикать, иначе проверять нечего"
        );
        assert!(
            demon_pawn_ids(app).is_empty(),
            "до объявления старта мира демонов быть не должно"
        );

        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        for _ in 0..4 {
            app.update();
        }

        let ids = demon_pawn_ids(app);
        assert!(!ids.is_empty(), "в `Live` залп обязан выйти");
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique, "номера демонов повторились: {ids:?}");
    }

    /// Спавн демона — единственное место, где вид объявляет своё тело;
    /// забытый компонент означал бы демона с человеческим допуском прихода
    /// (`movement::BodyScale`, умолчание `#[require]` — человеческое).
    #[test]
    fn every_demon_from_the_portal_carries_a_demon_body() {
        let app = &mut app();
        tick_through_a_moving_warmup(app);
        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        for _ in 0..4 {
            app.update();
        }

        let bodies: Vec<crate::movement::BodyScale> = app
            .world_mut()
            .query_filtered::<&crate::movement::BodyScale, With<Demon>>()
            .iter(app.world())
            .copied()
            .collect();

        assert!(!bodies.is_empty(), "в `Live` залп обязан выйти");
        assert!(
            bodies
                .iter()
                .all(|body| *body == crate::movement::BodyScale::DEMON),
            "демон вышел с чужим телом: {bodies:?}"
        );
    }

    /// Регрессия: гейт на мир демонскому поведению несёт ветка конвейера, не
    /// стадия.
    ///
    /// `pick_wander_targets` и `chase` берут `Res<Backend>`, которого вне
    /// `Playing` не существует, а `TickStage::DemonBehavior` — только порядок:
    /// настроена цепочка стадий или нет (её настраивает `TickStagePlugin`,
    /// поднимаемый `SpatialPlugin`), стадия не гейтит **ничего**. Здесь мир
    /// не поднимался, `AppState` остался `Loading`, гейт `in_world` из ветки
    /// `SimPipeline::BothModes` ложь — и поведение обязано молчать.
    ///
    /// Ресурсы мира (`Backend`, `PortalPos`) хелпер `app()` всё же вставляет —
    /// иначе система не прошла бы валидацию параметров и тест был бы зелён по
    /// неверной причине: не «гейт сработал», а «ресурса не нашлось».
    /// Наблюдаемое — счётчик решений: `pick_wander_targets` сдвигает его
    /// безусловно, и сдвиг виден без проходимого навмеша.
    #[test]
    fn demon_behavior_does_not_run_outside_the_world() {
        let app = &mut app();
        let demon = app
            .world_mut()
            .spawn((
                Demon,
                DemonWanderTag,
                crate::movement::Movable::new(10.0),
                PawnId(0),
                crate::rng::WanderIndex::ready(),
            ))
            .id();
        for _ in 0..4 {
            app.update();
        }

        // без тиков проверять нечего: см. довод про `TimeUpdateStrategy` в `app()`
        assert!(
            app.world().resource::<Time<Fixed>>().elapsed_secs() > 0.0,
            "тест обязан тикать, иначе он зелен впустую"
        );
        assert_eq!(
            app.world().get::<crate::rng::WanderIndex>(demon).copied(),
            Some(crate::rng::WanderIndex::ready()),
            "поведение демонов отработало вне мира"
        );
    }

    /// Счётчик спавнера — источник `PawnId` и seed потока ГПСЧ демона, поэтому
    /// инкремент обязан быть ровно один на демона: пропуск раздаёт номер
    /// дважды, лишний — оставляет дыру и сдвигает потоки следующих демонов.
    /// Источников с фазы орды два — фон и волна (`demon::horde`), номера
    /// берёт общий: волна включена явно, чтобы смешанный источник был
    /// обязан присутствовать, а не случился по жребию очага.
    #[test]
    fn every_spawned_demon_advances_the_counter_exactly_once() {
        let app = &mut app();
        tick_through_a_moving_warmup(app);
        app.world_mut().insert_resource(HordeStyle {
            wave_size: 20,
            wave_every: 600.0,
            wave_len: 5,
        });
        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        for _ in 0..4 {
            app.update();
        }

        assert!(
            app.world().resource::<HordeSchedule>().wave_index > 0,
            "волна не вышла: проверка смешанного источника пуста"
        );
        let ids = demon_pawn_ids(app);
        assert!(!ids.is_empty(), "в `Live` залп обязан выйти");
        assert_eq!(
            ids,
            (0..ids.len() as u32).collect::<Vec<_>>(),
            "номера демонов обязаны идти подряд с нуля: {ids:?}"
        );
        assert_eq!(
            app.world().resource::<DemonSpawner>().spawned,
            ids.len(),
            "счётчик спавнера разошёлся с числом выпущенных демонов"
        );
    }

    /// Состояние прогона обязано быть видно живому осмотру: `DemonSpawner` и
    /// расписание орды сбрасывает тот же `WorldStarted`, что `Telemetry`,
    /// `SimTick` и `TickDebt`, и все они читаются по BRP (`CONTEXT.md`,
    /// «WorldStarted»). Проверяется именно то, что требует `bevy/get_resource`:
    /// тип в реестре **и** данные `ReflectResource` на нём — одного
    /// `register_type` без `#[reflect(Resource)]` не хватило бы.
    #[test]
    fn the_spawner_state_is_visible_over_brp() {
        let app = app();
        let registry = app.world().resource::<AppTypeRegistry>().read();
        for (name, ty) in [
            ("DemonSpawner", std::any::TypeId::of::<DemonSpawner>()),
            ("HordeSchedule", std::any::TypeId::of::<HordeSchedule>()),
        ] {
            let registration = registry.get(ty).unwrap_or_else(|| {
                panic!("{name} не в реестре типов: `brp res get` его не увидит")
            });
            assert!(
                registration.data::<ReflectResource>().is_some(),
                "у {name} нет ReflectResource: BRP не сможет его прочитать"
            );
        }
    }

    /// Пин порядка тика: спавнер стоит в стадии `Spawner`, раньше стадии
    /// `DemonGrid`, и демон, родившийся на тике, входит в сетку демонов на
    /// нём же — то есть действует с первого тика своего существования
    /// (`1ab9ab6`).
    ///
    /// До фикса обе системы спавнера не состояли ни в одном множестве, и точка
    /// синхронизации, на которой применяются команды спавна, могла прийтись и
    /// до пересборки сетки, и после: решал топологический разбор расписания и
    /// то, кто из систем успел завершиться, а не эта строка.
    #[test]
    fn a_demon_born_on_a_tick_enters_the_demon_grid_on_that_same_tick() {
        let app = &mut app();
        // ровно один тик на кадр: проверяется именно тик рождения, а на
        // дефолтных 64 Гц один `update` на 100 мс дал бы шесть. Волны
        // выключены, фон заморожен (интервал в потолке шкалы — таймер не
        // огоньёт ни разу): пин — о шве залпа, порции волны и фоновые демоны
        // легитимно выходят в соседних тиках
        app.insert_resource(Time::<Fixed>::from_seconds(0.1));
        app.insert_resource(HordeStyle {
            wave_size: 0,
            ..Default::default()
        });
        app.insert_resource(DemonStyle {
            interval: DEMON_SPAWN_INTERVAL_MAX,
            ..Default::default()
        });

        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();
        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        // кадр перехода: `OnEnter(Live)` объявляет старт и снимает паузу, но
        // виртуальные часы этого кадра стояли — тика в нём нет
        app.update();
        assert!(
            demon_pawn_ids(app).is_empty(),
            "залп выходит на первом тике `Live`, а не в кадре перехода"
        );

        app.update();

        let born = demon_pawn_ids(app).len();
        assert_eq!(born, DEMON_INITIAL_BURST, "залп обязан выйти целиком");
        assert_eq!(
            demons_in_grid(app),
            born,
            "демон родился на тике, а в сетку демонов вошёл не на нём"
        );
    }
}
