//! Демо-сцена расталкивания пешек: толпа на пустой карте, крупным планом, с
//! честными числами перекрытия на экране.
//!
//! Зачем отдельная сцена, а не игра. Расталкивание
//! (`movement/separation/`) работает только во вьюпорте, только при зуме
//! ниже [`SEPARATION_MAX_ZOOM`] и не чаще раза в кадр — то есть проверяется
//! ровно в тех условиях, которые в игре надо *дождаться*: воронка у портала,
//! встречные потоки в коридоре, толпа на 30×. Здесь любой из этих случаев
//! ставится клавишей, и рядом висит счётчик перекрывшихся пар.
//!
//! **Числа считаются только по тому, что в кадре.** Расталкивание работает по
//! прямоугольнику вокруг камеры и за кадром намеренно не работает, так что
//! пешки за краем экрана в счёт не идут — иначе метрика мерила бы систему там,
//! где её нет (первая версия этой сцены так и сделала: «пары» набирались из
//! забредших за кадр, и вкл/выкл расталкивания не отличались).
//!
//! **Что здесь настоящее.** Всё, что считает: `MovementPlugin` со своим
//! `separate_pawns`, настоящие `SpatialGrid`, настоящие константы из
//! `settings.rs`. Своя в примере только раскладка толпы и маршруты — то есть
//! то, что в игре даёт поведение. Переписанная копия расталкивания ничего бы
//! не доказала.
//!
//! **Навигация — полигональная, как в игре.** `PolymeshDebug::enabled` включён
//! по умолчанию, и от него же зависит само расталкивание
//! (`movement::separation_runs`): на сеточной навигации его нет вовсе, и мерить
//! в этой сцене было бы нечего. Отсюда два следствия для раскладки:
//! - отрезки маршрутов прокладывает `find_path_polymesh` — waypoint'ы
//!   метрические, углами препятствий, а не центрами навтайлов. Пока меш
//!   строится (первые доли секунды), путь идёт прямой по центрам тайлов, как
//!   раньше: та же подмена, которой в игре сетка обслуживает запросы до
//!   готовности меша;
//! - стены «коридора» кладутся не только в сетку, но и в `MapData::walls` —
//!   иначе полигональный поиск о них не знает и водит пешек сквозь стену.
//!   Смена сценария поэтому пересобирает меш (`PolyNavmesh::clear`).
//!
//! Переключателя на сеточную навигацию здесь нет намеренно: на ней
//! расталкивания не бывает, а эта сцена ровно про него.
//!
//! **Почему толпа выглядит слипшейся даже когда всё правильно.** Радиус тела
//! [`HUMAN_BODY_RADIUS`] = 0.45 м, то есть разведённая пара стоит в 0.9 м, а
//! спрайт — [`HUMAN_SIZE`] = 1.0 м. Спрайты в покое перекрываются на 10%
//! стороны. Поэтому поверх спрайта рисуется круг настоящего радиуса тела:
//! зелёный — дистанция выдержана, красный — реальное перекрытие. Глазом эти
//! два случая на спрайтах неразличимы.
//!
//! ```text
//! cargo run --example crowd_demo
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | `1`–`5` | сценарий: куча / воронка / встречные колонны / коридор / блуждание |
//! | `R` | пересобрать сценарий |
//! | `S` | расталкивание вкл/выкл |
//! | `Space` | пауза |
//! | `-` `=` | скорость симуляции по лестнице 1…30× |
//! | колесо | зум (за 0.75 расталкивание отключается — это видно по счётчику прогонов) |
//!
//! Сцену можно не только смотреть, но и опрашивать: раз в две секунды та же
//! строка метрик уходит в stdout, а BRP поднят на своём порту (`BRP_PORT`,
//! по умолчанию 15704 — не 15702 игры), так что сценарий и скорость
//! переключаются снаружи:
//!
//! ```text
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res set Scenario . '"Funnel"'
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res set DemoSpeed .0 20
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res get Overlaps
//! ```
//!
//! **Пример не трогает конфиг игры.** Ни `PrefsPlugin`, ни `SettingsPlugin`,
//! ни `CameraPlugin`, ни `DevPlugin`, ни `MapPlugin` — то есть читать и писать
//! `settings.toml` здесь физически нечему (единственные записи на диск в
//! проекте: `prefs.rs`, `camera.rs`, `dev.rs::screenshot`, кеш Overpass). Свои
//! настройки демо держит в [`DemoConfig`] в этом же файле и никуда не
//! сохраняет.

mod args;
mod metrics;
mod panel;
mod scenario;

use bevy::diagnostic::{Diagnostic, RegisterDiagnostic};
use bevy::input::common_conditions::input_just_pressed;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use qwe::diagnostics::SIM_SEPARATION_MS;
use qwe::human::{HumanStyle, pick_wander_targets};
use qwe::loading::{AppState, PlayPhase};
use qwe::map::osm::MapData;
use qwe::movement::{
    MovableReachedDestinationEvent, SeparationLab, SeparationStyle, SlotLab, SlotSearch,
};
use qwe::navigation::PathfindingAlgorithm;
use qwe::rng::WorldSeed;
use qwe::settings::MAP_CENTER_PORTAL_POS;

use crate::args::{Args, apply_lab, parse_args};
use crate::metrics::{
    Overlaps, PathMisses, RunCounters, Trial, count_separation_runs, count_ticks, draw_bodies,
    finish_trial, measure_overlaps, report_to_stdout, sample_travel, sample_trial, take_shots,
};
use crate::panel::{
    spawn_mechanism_panel, spawn_overlay, spawn_sliders, sync_knob_rows, update_overlay,
};
use crate::scenario::{
    Scenario, drive_routes, interrupt_for_slot_claim, regroup_to_slot, respawn_scenario,
};

// ---------------------------------------------------------------- конфиг демо

/// Настройки сцены. Живут здесь и только здесь: демо ничего не читает с диска
/// и ничего туда не пишет (см. шапку модуля). Доменные величины — радиус тела,
/// скорость ходьбы, размер спрайта — наоборот, берутся из `settings.rs`: демо
/// обязано мерить числа игры, а не свою копию.
#[derive(Resource, Clone)]
pub(crate) struct DemoConfig {
    /// Центр арены. Середина карты — подальше от краёв, где навтайлы кончаются.
    pub(crate) centre: Vec2,
    pub(crate) start_zoom: f32,
    pub(crate) min_zoom: f32,
    pub(crate) max_zoom: f32,
    /// Лестница скоростей на `-`/`=`; 30× — потолок и в игре.
    pub(crate) speeds: [f32; 6],
    /// Сколько пешек в каждом сценарии и на каком масштабе они расставлены.
    pub(crate) pile: usize,
    pub(crate) pile_radius: f32,
    pub(crate) funnel: usize,
    pub(crate) funnel_radius: f32,
    pub(crate) column: usize,
    pub(crate) column_length: f32,
    /// Шаг между пешками в колонне. Больше дистанции покоя намеренно:
    /// стартовая раскладка обязана быть законной, иначе непонятно, кто создал
    /// перекрытие — поток или спавн.
    ///
    /// Держать его в согласии с [`HUMAN_BODY_RADIUS`] обязательно и вручную:
    /// шаг 1.2 м пережил здесь смену радиуса тела с 0.45 на 0.9 (дистанция
    /// покоя 0.9 → 1.8 м) и молча превратил «законную колонну» в стартовую
    /// давку — 95% пешко-времени в перекрытии с нулевого кадра, то есть весь
    /// замер мерил не поток, а разгребание спавна.
    pub(crate) column_spacing: f32,
    /// Поперечная ширина колонны, м: пешки раскладываются по полосам внутри
    /// неё. 0 — обе колонны в одну линию `y = centre.y`, самый злой лобовой
    /// случай и исторический дефолт этой сцены. Больше нуля — «улица»: у потока
    /// есть куда расслоиться ещё до первого касания, и видно, пользуется ли
    /// механизм этой свободой или всё равно сводит всех в колонну.
    pub(crate) column_width: f32,
    pub(crate) corridor: usize,
    pub(crate) corridor_gap: f32,
    pub(crate) corridor_length: f32,
    pub(crate) wander: usize,
    pub(crate) wander_box: f32,
    /// Сид раскладки: одна и та же куча от запуска к запуску.
    pub(crate) seed: u64,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            centre: MAP_CENTER_PORTAL_POS,
            start_zoom: 0.08,
            min_zoom: 0.02,
            max_zoom: 1.5,
            speeds: [1.0, 2.0, 5.0, 10.0, 20.0, 30.0],
            pile: 80,
            pile_radius: 2.0,
            funnel: 200,
            funnel_radius: 45.0,
            column: 40,
            column_length: 40.0,
            // 2 × HUMAN_BODY_RADIUS = 1.8 м дистанция покоя, плюс запас
            column_spacing: 2.0,
            column_width: 0.0,
            corridor: 120,
            corridor_gap: 4.0,
            corridor_length: 60.0,
            wander: 300,
            wander_box: 60.0,
            seed: 1,
        }
    }
}

/// Скорость симуляции — отдельным ресурсом, а не прямой записью в
/// `Time<Virtual>`: так её видно и можно поставить снаружи по BRP, а сравнение
/// «пар на 1× против пар на 30×» делается без рук на клавиатуре.
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
pub(crate) struct DemoSpeed(pub(crate) f32);

impl Default for DemoSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

fn main() {
    let args = parse_args();
    let mut app = App::new();
    // часть систем игры просит параметры, которых в этой сцене нет (`Gizmos`
    // до появления камеры, `MapData` у полимеша). В игре такое — ошибка, здесь
    // ожидаемое состояние, и ему место в предупреждении, а не в панике
    app.set_error_handler(bevy::ecs::error::warn);

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "qwe crowd demo — separation".to_string(),
                    resolution: (1400, 900).into(),
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                level: bevy::log::Level::WARN,
                filter: "warn,qwe=warn".to_string(),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::srgb(0.72, 0.71, 0.68)))
    // ровно те плагины игры, которые считают движение и расталкивание.
    // НЕ добавлять сюда `PrefsPlugin`/`CameraPlugin`/`DevPlugin`/`MapPlugin`:
    // они читают и пишут конфиг игры и её кеши (см. шапку модуля)
    .add_plugins((
        qwe::rng::RngPlugin,
        qwe::navigation::NavigationPlugin,
        qwe::spatial::SpatialPlugin,
        qwe::movement::MovementPlugin,
    ))
    .init_state::<AppState>()
    .add_sub_state::<PlayPhase>()
    // `Determinism` не вставляем сознательно: `separate_pawns` идёт под
    // `run_if(not(deterministic))` и в детерминированном режиме выключен
    .init_resource::<MapData>()
    .init_resource::<HumanStyle>()
    .init_resource::<DemoConfig>()
    .init_resource::<Scenario>()
    .init_resource::<DemoSpeed>()
    .init_resource::<Overlaps>()
    .init_resource::<RunCounters>()
    .init_resource::<PathMisses>()
    .init_resource::<Trial>()
    .register_type::<Scenario>()
    .register_type::<DemoSpeed>()
    .register_type::<Overlaps>()
    .register_type::<RunCounters>()
    .register_type::<PathMisses>()
    // свой порт: 15702 занимает игра пользователя, 15703 — её же
    // инспекция из `live-app`
    .add_plugins((
        bevy::remote::RemotePlugin::default(),
        bevy::remote::http::RemoteHttpPlugin::default().with_port(brp_port()),
    ))
    // плоский A*: не зависит от фоновой постройки иерархии HPA и готов с
    // первого кадра. Пути на пустой сетке короткие, цена неважна
    .insert_resource(PathfindingAlgorithm::Astar)
    .insert_resource(WorldSeed(1))
    // `separate_pawns` пишет этот замер, но регистрирует его
    // `GameDiagnosticsPlugin`, которого здесь нет: без регистрации замер
    // выбрасывается, и считать прогоны было бы нечем
    .register_diagnostic(
        Diagnostic::new(SIM_SEPARATION_MS)
            .with_suffix(" ms")
            .with_max_history_length(1),
    )
    .add_systems(
        Startup,
        (
            spawn_camera,
            spawn_overlay,
            spawn_sliders,
            spawn_mechanism_panel,
        ),
    )
    .add_systems(
        Update,
        (
            enter_live.run_if(in_state(PlayPhase::Warmup)),
            // блуждание — та же регистрация, что в `HumanPlugin` для
            // недетерминированного режима
            pick_wander_targets,
            respawn_scenario
                .run_if(resource_changed::<Scenario>.or_else(input_just_pressed(KeyCode::KeyR))),
            pick_scenario,
            toggle_separation.run_if(input_just_pressed(KeyCode::KeyS)),
            toggle_pause.run_if(input_just_pressed(KeyCode::Space)),
            zoom_camera,
            // бегунок ведёт та же система, что и в игре; `sync_knob_rows`
            // подтягивает панель механизмов после нажатия пресета
            (qwe::ui::slider::sync_slider_thumbs, sync_knob_rows),
            apply_speed.run_if(resource_changed::<DemoSpeed>),
            (
                count_separation_runs,
                measure_overlaps,
                draw_bodies,
                update_overlay,
                report_to_stdout,
                // замер — последним в кадре и строго после `measure_overlaps`:
                // он складывает ровно те числа, что тот посчитал
                sample_trial,
                take_shots,
                finish_trial,
            )
                .chain(),
        ),
    )
    // порядок явный: прерывание подошедших — до выдачи отрезков, иначе слот
    // они получат тиком позже (`interrupt_for_slot_claim` снимает тег
    // движения командой, а её применяет точка синхронизации цепочки)
    .add_systems(
        FixedUpdate,
        (
            count_ticks,
            interrupt_for_slot_claim,
            drive_routes,
            regroup_to_slot,
            sample_travel,
        )
            .chain(),
    )
    .add_observer(
        |_arrival: On<MovableReachedDestinationEvent>, mut trial: ResMut<Trial>| {
            trial.arrivals += 1;
        },
    );

    apply_args(&mut app, args);

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Playing);

    app.run();
}

/// Разложить командную строку по ресурсам — ДО первого кадра, чтобы толпа с
/// самого начала жила в том режиме, который меряют (см. [`Args`]).
pub(crate) fn apply_args(app: &mut App, args: Args) {
    let world = app.world_mut();
    if let Some(seconds) = args.seconds {
        // окно замера включает и печать `RESULT`, и выход: снаружи процесс
        // убивать не нужно, а значит и гадать, успел ли он дописать строку
        world.resource_mut::<Trial>().window = seconds;
    }
    {
        let mut trial = world.resource_mut::<Trial>();
        trial.shots = args.shots;
        trial.label = args.label.clone().unwrap_or_else(|| "baseline".to_string());
    }
    if let Some(scenario) = args.scenario {
        *world.resource_mut::<Scenario>() = scenario;
    }
    if let Some(speed) = args.speed {
        world.resource_mut::<DemoSpeed>().0 = speed;
    }
    {
        let mut config = world.resource_mut::<DemoConfig>();
        if let Some(pawns) = args.pawns {
            config.pile = pawns;
            config.funnel = pawns;
            config.column = pawns;
            config.corridor = pawns;
            config.wander = pawns;
        }
        if let Some(width) = args.width {
            config.column_width = width;
        }
        if let Some(spacing) = args.spacing {
            config.column_spacing = spacing;
        }
        if let Some(zoom) = args.zoom {
            config.start_zoom = zoom;
        }
        if let Some(seed) = args.seed {
            config.seed = seed;
        }
    }
    {
        let mut style = world.resource_mut::<SeparationStyle>();
        if let Some(enabled) = args.separation {
            style.enabled = enabled;
        }
        if let Some(hold) = args.hold {
            style.hold = hold;
        }
        if let Some(sidestep) = args.sidestep {
            style.sidestep = sidestep;
        }
        if let Some(backstep) = args.backstep {
            style.backstep = backstep;
        }
    }
    if let Some(radius) = args.radius {
        world.resource_mut::<HumanStyle>().body_radius = radius;
    }
    if let Some(search) = args.search {
        world.resource_mut::<SlotSearch>().0 = search;
    }
    if let Some(matching) = args.matching {
        world.resource_mut::<SlotLab>().matching = matching;
    }
    if let Some(slack) = args.slot_slack {
        world.resource_mut::<SlotLab>().slack = slack;
    }
    if let Some(claim_at) = args.claim_at {
        world.resource_mut::<SlotLab>().claim_at = claim_at;
    }
    if let Some(regroup) = args.regroup {
        world.resource_mut::<SlotLab>().regroup = regroup;
    }
    apply_lab(&mut world.resource_mut::<SeparationLab>(), &args.lab);
}

/// Порт BRP этой сцены. По умолчанию 15704: 15702 держит игра пользователя,
/// 15703 — её инспекция из скилла `live-app`, и ответ с чужого порта ничем не
/// отличается от своего.
pub(crate) fn brp_port() -> u16 {
    std::env::var("BRP_PORT")
        .map(|value| value.parse().expect("BRP_PORT is not a port number"))
        .unwrap_or(15704)
}

/// `PlayPhase` в игре переключает загрузчик, которого здесь нет: без `Live`
/// часть систем навигации молча не работает.
pub(crate) fn enter_live(mut next: ResMut<NextState<PlayPhase>>) {
    next.set(PlayPhase::Live);
}

pub(crate) fn spawn_camera(mut commands: Commands, config: Res<DemoConfig>) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(config.centre.extend(0.0))
            .with_scale(Vec3::splat(config.start_zoom)),
        Msaa::Off,
    ));
}

// -------------------------------------------------------------------- ввод

/// Выбор сценария цифрами. Один системный обработчик на пять клавиш, а не пять
/// систем с `run_if`: это переключатель, а не пять разных действий.
pub(crate) fn pick_scenario(keys: Res<ButtonInput<KeyCode>>, mut scenario: ResMut<Scenario>) {
    for (key, next) in [
        (KeyCode::Digit1, Scenario::Pile),
        (KeyCode::Digit2, Scenario::Funnel),
        (KeyCode::Digit3, Scenario::Columns),
        (KeyCode::Digit4, Scenario::Corridor),
        (KeyCode::Digit5, Scenario::Wander),
    ] {
        if keys.just_pressed(key) && *scenario != next {
            *scenario = next;
        }
    }
}

pub(crate) fn toggle_separation(mut style: ResMut<SeparationStyle>) {
    // менять `SeparationStyle` из примера безопасно ровно потому, что здесь нет
    // `SettingsPlugin`: это `SettingsGroup`, и с `PrefsPlugin` тумблер уехал бы
    // в конфиг игры
    style.enabled = !style.enabled;
}

pub(crate) fn toggle_pause(mut time: ResMut<Time<Virtual>>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
}

/// Скорость по лестнице — своей, а не через `SimTimePlugin`: губернатор кадров
/// из игры срезал бы запрошенные 30× и смазал бы сравнение «пар на 1× против
/// пар на 30×».
/// Возвращает новую ступень, а не пишет в ресурс: запись через `ResMut`
/// пометила бы `DemoSpeed` изменённым каждый кадр, и гейт `resource_changed` у
/// [`apply_speed`] перестал бы что-либо значить.
pub(crate) fn step_speed(
    keys: &ButtonInput<KeyCode>,
    config: &DemoConfig,
    current: f32,
) -> Option<f32> {
    let up = keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd);
    let down = keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract);
    if !up && !down {
        return None;
    }
    let index = config
        .speeds
        .iter()
        .position(|step| (step - current).abs() < 0.01)
        .unwrap_or(0);
    let next = if up {
        (index + 1).min(config.speeds.len() - 1)
    } else {
        index.saturating_sub(1)
    };
    Some(config.speeds[next])
}

pub(crate) fn apply_speed(speed: Res<DemoSpeed>, mut time: ResMut<Time<Virtual>>) {
    time.set_relative_speed(speed.0);
}

/// Зум колесом. Панорамирования нет намеренно: панелей в сцене нет, но правило
/// «мышь над UI не двигает мир» проще соблюсти, не заводя перетаскивание.
pub(crate) fn zoom_camera(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<DemoConfig>,
    scroll: Res<AccumulatedMouseScroll>,
    mut speed: ResMut<DemoSpeed>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    // `bypass_change_detection` — чтобы чтение текущей ступени не считалось
    // изменением: иначе `DemoSpeed` менялся бы каждый кадр
    if let Some(next) = step_speed(&keys, &config, speed.bypass_change_detection().0) {
        speed.0 = next;
    }

    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
    };
    if lines == 0.0 {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let zoom = (transform.scale.x * 1.2f32.powf(-lines)).clamp(config.min_zoom, config.max_zoom);
    transform.scale = Vec3::splat(zoom);
}
