//! Демо-сцена расталкивания пешек: толпа на пустой карте, крупным планом, с
//! честными числами перекрытия на экране.
//!
//! Зачем отдельная сцена, а не игра. Расталкивание
//! (`movement/separation.rs`) работает только во вьюпорте, только при зуме
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

use std::collections::VecDeque;

use bevy::diagnostic::{Diagnostic, DiagnosticsStore, RegisterDiagnostic};
use bevy::input::common_conditions::input_just_pressed;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::text::FontSize;

use bevy::ui_widgets::{SliderValue, ValueChange};
use qwe::diagnostics::SIM_SEPARATION_MS;
use qwe::grid::{tile_center, world_to_tile};
use qwe::human::{
    Human, HumanFirstWanderTag, HumanStyle, HumanWanderTag, Pace, WanderHeading, WanderPause,
    pick_wander_targets,
};
use qwe::loading::{AppState, PlayPhase};
use qwe::map::osm::MapData;
use qwe::movement::{
    DestinationClaim, DestinationClaims, Movable, MovableStateMovingTag, SeparationStyle,
    SimPosition, SlotSearch, slot_side,
};
use qwe::navigation::{ArcNavmesh, PathfindingAlgorithm};
use qwe::rng::{PawnId, RngDomain, WanderIndex, WorldSeed, decision_stream, stream};
use qwe::settings::{
    HUMAN_SIZE, HUMAN_SPEED_SPREAD, HUMAN_WALK_SPEED, MAP_CENTER_PORTAL_POS, SEPARATION_MAX_ZOOM,
    navtile_size, unit_z,
};
use qwe::ui::slider::{SliderRow, quantize, spawn_slider_row};
use rand::Rng;

// ---------------------------------------------------------------- конфиг демо

/// Настройки сцены. Живут здесь и только здесь: демо ничего не читает с диска
/// и ничего туда не пишет (см. шапку модуля). Доменные величины — радиус тела,
/// скорость ходьбы, размер спрайта — наоборот, берутся из `settings.rs`: демо
/// обязано мерить числа игры, а не свою копию.
#[derive(Resource, Clone)]
struct DemoConfig {
    /// Центр арены. Середина карты — подальше от краёв, где навтайлы кончаются.
    centre: Vec2,
    start_zoom: f32,
    min_zoom: f32,
    max_zoom: f32,
    /// Лестница скоростей на `-`/`=`; 30× — потолок и в игре.
    speeds: [f32; 6],
    /// Сколько пешек в каждом сценарии и на каком масштабе они расставлены.
    pile: usize,
    pile_radius: f32,
    funnel: usize,
    funnel_radius: f32,
    column: usize,
    column_length: f32,
    /// Шаг между пешками в колонне. Больше дистанции покоя 0.9 м намеренно:
    /// стартовая раскладка обязана быть законной, иначе непонятно, кто создал
    /// перекрытие — поток или спавн.
    column_spacing: f32,
    corridor: usize,
    corridor_gap: f32,
    corridor_length: f32,
    wander: usize,
    wander_box: f32,
    /// Сид раскладки: одна и та же куча от запуска к запуску.
    seed: u64,
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
            column_spacing: 1.2,
            corridor: 120,
            corridor_gap: 4.0,
            corridor_length: 60.0,
            wander: 300,
            wander_box: 60.0,
            seed: 1,
        }
    }
}

// ------------------------------------------------------------------ сценарии

#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
enum Scenario {
    /// Куча в одной точке, никто никуда не идёт: чистая сходимость.
    #[default]
    Pile,
    /// Все идут в одну точку и обратно — случай «толпа у портала».
    Funnel,
    /// Две колонны навстречу по ОДНОЙ линии waypoint'ов: проверка того, не
    /// стирает ли постановка на waypoint (`move_moving_entities`) боковой
    /// сдвиг, который дало расталкивание.
    Columns,
    /// То же, но в коридоре между двумя стенами: толчок в непроходимый тайл
    /// расталкивание отбрасывает целиком, без скольжения вдоль стены.
    Corridor,
    /// Настоящее блуждание игры: `pick_wander_targets` + настоящий A*.
    /// Контрольный случай — слипает ли толпу сама игровая связка.
    Wander,
}

impl Scenario {
    fn label(self) -> &'static str {
        match self {
            Self::Pile => "1 pile",
            Self::Funnel => "2 funnel",
            Self::Columns => "3 columns",
            Self::Corridor => "4 corridor",
            Self::Wander => "5 wander (real AI)",
        }
    }
}

/// Всё, что принадлежит сценарию и умирает при его смене.
#[derive(Component)]
struct DemoPawn;

/// Стена сценария: и спрайт на экране, и запись о заглушенных навтайлах.
#[derive(Component)]
struct DemoWall;

/// Замкнутый маршрут пешки: дойдя до последней точки, идём снова к первой.
/// Заменяет собой всё поведение — демо гоняет толпу по заранее известным
/// линиям, чтобы мерить расталкивание, а не блуждание.
#[derive(Component)]
struct Route {
    legs: Vec<Vec2>,
    next: usize,
}

// ------------------------------------------------------------------- метрики

/// Скорость симуляции — отдельным ресурсом, а не прямой записью в
/// `Time<Virtual>`: так её видно и можно поставить снаружи по BRP, а сравнение
/// «пар на 1× против пар на 30×» делается без рук на клавиатуре.
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
struct DemoSpeed(f32);

impl Default for DemoSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Перекрытие меньше этого считается сошедшимся: мягкий решатель оставляет
/// асимптотический хвост в единицы миллиметров, и без порога «пар» всегда
/// оказывались бы десятки — при перекрытии, которого нет ни на экране, ни по
/// смыслу.
const OVERLAP_EPSILON: f32 = 0.02;

/// Перекрытия текущего кадра — то, ради чего сцена и написана.
///
/// **Считается только то, что в кадре.** Расталкивание работает по
/// прямоугольнику вокруг камеры (`separation.rs`), и пешки за кадром не
/// разводятся намеренно — считать их значило бы мерить систему там, где она по
/// построению не работает. Прямоугольник здесь взят без запаса `VIEW_MARGIN`,
/// то есть строго внутри рабочего: всё, что попало в счёт, расталкивание
/// точно видело.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct Overlaps {
    /// Пешек в кадре и всего в сцене.
    pawns: usize,
    total: usize,
    /// Радиус, по которому посчитано это перекрытие — им же рисуются круги.
    radius: f32,
    pairs: usize,
    /// Максимальная нехватка расстояния до суммы радиусов, м.
    worst: f32,
    mean: f32,
    /// Сколько пешек участвует хотя бы в одной перекрывшейся паре.
    involved: usize,
    /// Позиции в кадре и признак перекрытия — чтобы гизмо рисовало ровно то,
    /// что посчитано, а не считало во второй раз.
    #[reflect(ignore)]
    bodies: Vec<(Vec2, bool)>,
    #[reflect(ignore)]
    links: Vec<(Vec2, Vec2)>,
}

/// Сколько тиков движения приходится на один прогон расталкивания. Прогоны
/// считаются не по своей копии гейта, а по настоящему замеру
/// `sim/separation_ms`: система пишет его ровно раз за прогон.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct RunCounters {
    runs: u64,
    #[reflect(ignore)]
    last_measurement: Option<std::time::Instant>,
    ticks_per_run: f32,
    window_ticks: u64,
    window_runs: u64,
}

#[derive(Component)]
struct OverlayText;

/// Пределы ползунка радиуса: от «меньше половины спрайта» (как было до правки)
/// до заведомо избыточного личного пространства.
const RADIUS_MIN: f32 = 0.3;
const RADIUS_MAX: f32 = 1.2;
const RADIUS_STEP: f32 = 0.01;

#[derive(Component)]
struct RadiusSlider;

#[derive(Component)]
struct RadiusValueLabel;

/// Пределы ползунка радиуса поиска слота. Снизу — меньше, чем нужно даже
/// десятку пешек, чтобы было видно, как хвост толпы остаётся без слотов и
/// сваливается в общую точку; сверху — вчетверо больше дефолта: 40 м на шаге
/// 2 м это 21 × 21 слот, с запасом на всю «воронку».
const SEARCH_MIN: f32 = 2.0;
const SEARCH_MAX: f32 = 40.0;
const SEARCH_STEP: f32 = 1.0;

#[derive(Component)]
struct SearchSlider;

#[derive(Component)]
struct SearchValueLabel;

fn main() {
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
    .register_type::<Scenario>()
    .register_type::<DemoSpeed>()
    .register_type::<Overlaps>()
    .register_type::<RunCounters>()
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
    .add_systems(Startup, (spawn_camera, spawn_overlay, spawn_sliders))
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
            // бегунок ведёт та же система, что и в игре
            qwe::ui::slider::sync_slider_thumbs,
            apply_speed.run_if(resource_changed::<DemoSpeed>),
            (
                count_separation_runs,
                measure_overlaps,
                draw_bodies,
                update_overlay,
                report_to_stdout,
            )
                .chain(),
        ),
    )
    .add_systems(FixedUpdate, (count_ticks, drive_routes));

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Playing);

    app.run();
}

/// Порт BRP этой сцены. По умолчанию 15704: 15702 держит игра пользователя,
/// 15703 — её инспекция из скилла `live-app`, и ответ с чужого порта ничем не
/// отличается от своего.
fn brp_port() -> u16 {
    std::env::var("BRP_PORT")
        .map(|value| value.parse().expect("BRP_PORT is not a port number"))
        .unwrap_or(15704)
}

/// `PlayPhase` в игре переключает загрузчик, которого здесь нет: без `Live`
/// часть систем навигации молча не работает.
fn enter_live(mut next: ResMut<NextState<PlayPhase>>) {
    next.set(PlayPhase::Live);
}

fn spawn_camera(mut commands: Commands, config: Res<DemoConfig>) {
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

/// Ползунки сцены — тот же кит строки-ползунка, что у панелей игры
/// (`qwe::ui::slider`), чтобы обе величины подбирались глазом на живой толпе, а
/// не пересборкой. Пишут прямо в ресурсы движения, откуда их берут и сама
/// механика, и гизмо этой сцены: `SeparationStyle::radius` — «личное
/// пространство», `SlotSearch` — докуда искать свободный слот назначения.
fn spawn_sliders(mut commands: Commands, style: Res<SeparationStyle>, search: Res<SlotSearch>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(150.),
                left: px(12.),
                width: px(240.),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0., 0., 0., 0.25)),
        ))
        .id();

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Body radius",
            value: style.radius,
            value_text: format!("{:.2} m", style.radius),
            range: (RADIUS_MIN, RADIUS_MAX, RADIUS_STEP),
        },
        RadiusValueLabel,
        RadiusSlider,
        |change: On<ValueChange<f32>>,
         mut commands: Commands,
         mut style: ResMut<SeparationStyle>,
         mut label: Query<&mut Text, With<RadiusValueLabel>>| {
            let stepped = quantize(change.value, RADIUS_MIN, RADIUS_MAX, RADIUS_STEP);
            // ползунок «управляемый»: он только сообщает о правке, а своё
            // `SliderValue` не трогает — без этой строки бегунок остаётся на
            // месте, хотя значение уже изменилось (и следующая протяжка
            // считается от старого)
            commands.entity(change.source).insert(SliderValue(stepped));
            style.radius = stepped;
            for mut text in &mut label {
                text.0 = format!("{stepped:.2} m");
            }
        },
    );

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Slot search",
            value: search.0,
            value_text: format!("{:.0} m", search.0),
            range: (SEARCH_MIN, SEARCH_MAX, SEARCH_STEP),
        },
        SearchValueLabel,
        SearchSlider,
        |change: On<ValueChange<f32>>,
         mut commands: Commands,
         mut search: ResMut<SlotSearch>,
         mut label: Query<&mut Text, With<SearchValueLabel>>| {
            let stepped = quantize(change.value, SEARCH_MIN, SEARCH_MAX, SEARCH_STEP);
            commands.entity(change.source).insert(SliderValue(stepped));
            search.0 = stepped;
            for mut text in &mut label {
                text.0 = format!("{stepped:.0} m");
            }
        },
    );
}

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        OverlayText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.),
            ..default()
        },
        TextColor(Color::srgb(0.1, 0.1, 0.12)),
        Node {
            position_type: PositionType::Absolute,
            top: px(10.),
            left: px(12.),
            ..default()
        },
    ));
}

// --------------------------------------------------------------------- спавн

/// Раскладка сценария: снести прошлый, вернуть навмешу проходимость, поставить
/// новый. Одна система на все пять — раскладки различаются только позициями и
/// маршрутами.
fn respawn_scenario(
    mut commands: Commands,
    config: Res<DemoConfig>,
    scenario: Res<Scenario>,
    navmesh: Res<ArcNavmesh>,
    old: Query<Entity, Or<(With<DemoPawn>, With<DemoWall>)>>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    clear_arena(&navmesh, config.centre);

    let mut rng = stream(config.seed, RngDomain::Population, 0);
    let centre = config.centre;

    match *scenario {
        Scenario::Pile => {
            for index in 0..config.pile {
                let angle = rng.random_range(0.0..std::f32::consts::TAU);
                let radius = config.pile_radius * rng.random_range(0.0f32..1.0).sqrt();
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    centre + Vec2::from_angle(angle) * radius,
                    None,
                    false,
                );
            }
        }
        Scenario::Funnel => {
            for index in 0..config.funnel {
                let angle = std::f32::consts::TAU * index as f32 / config.funnel as f32;
                let rim = centre + Vec2::from_angle(angle) * config.funnel_radius;
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    rim,
                    // в точку и обратно на обод: ходят по кругу без остановки
                    Some(Route {
                        legs: vec![centre, rim],
                        next: 0,
                    }),
                    false,
                );
            }
        }
        Scenario::Columns => {
            let half = config.column_length / 2.0;
            for index in 0..config.column {
                let offset = index as f32 * config.column_spacing;
                let left = centre + Vec2::new(-half - offset, 0.0);
                let right = centre + Vec2::new(half + offset, 0.0);
                // обе колонны идут по одной линии y = centre.y, то есть по
                // одним и тем же центрам навтайлов
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    left,
                    Some(Route {
                        legs: vec![right, left],
                        next: 0,
                    }),
                    false,
                );
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    (config.column + index) as u32,
                    right,
                    Some(Route {
                        legs: vec![left, right],
                        next: 0,
                    }),
                    false,
                );
            }
        }
        Scenario::Corridor => {
            let half = config.corridor_length / 2.0;
            let gap = config.corridor_gap / 2.0;
            spawn_wall(&mut commands, &navmesh, centre, half, gap, 1.0);
            spawn_wall(&mut commands, &navmesh, centre, half, gap, -1.0);

            for index in 0..config.corridor {
                let side = if index % 2 == 0 { -1.0 } else { 1.0 };
                let along = index as f32 / config.corridor as f32 * half;
                let lane = rng.random_range(-gap + 0.5..gap - 0.5);
                let start = centre + Vec2::new(side * (half + along), lane);
                let end = centre + Vec2::new(-side * (half + along), lane);
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    start,
                    Some(Route {
                        legs: vec![end, start],
                        next: 0,
                    }),
                    false,
                );
            }
        }
        Scenario::Wander => {
            let half = config.wander_box / 2.0;
            for index in 0..config.wander {
                let position = centre
                    + Vec2::new(rng.random_range(-half..half), rng.random_range(-half..half));
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    position,
                    None,
                    true,
                );
            }
        }
    }
}

/// Пешка сценария. Бандл — тот же, что у `human::spawn_population`, иначе
/// расталкивание её не увидит: оно берёт кандидатов из `SpatialGrid<Human>`
/// (наполняется обсервером по `Transform`) и требует `PawnId` не-опционально.
fn spawn_pawn(
    commands: &mut Commands,
    seed: u64,
    pawn_id: u32,
    position: Vec2,
    route: Option<Route>,
    wandering: bool,
) {
    let mut rng = decision_stream(seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);
    let color = Color::hsl(
        rng.random_range(0.0..360.0),
        rng.random_range(0.35..0.75),
        rng.random_range(0.35..0.65),
    );
    let pace = Pace(rng.random_range(-1.0..=1.0));
    let heading = WanderHeading(Vec2::from_angle(
        rng.random_range(0.0..std::f32::consts::TAU),
    ));

    let mut entity = commands.spawn((
        DemoPawn,
        Sprite {
            color,
            custom_size: Some(Vec2::splat(HUMAN_SIZE)),
            ..default()
        },
        Transform::from_translation(position.extend(unit_z(position.y))),
        Human,
        Movable::new(pace.speed(HUMAN_WALK_SPEED, HUMAN_SPEED_SPREAD)),
        pace,
        PawnId(pawn_id),
        WanderIndex::ready(),
        Name::new("demo pawn"),
    ));
    if let Some(route) = route {
        entity.insert(route);
    }
    if wandering {
        // то, что спрашивает `pick_wander_targets`
        entity.insert((
            HumanWanderTag,
            HumanFirstWanderTag,
            WanderPause(Timer::from_seconds(0.0, TimerMode::Once)),
            heading,
        ));
    }
}

/// Полоса стены вдоль коридора: спрайт для глаза и заглушенные навтайлы для
/// расталкивания и поиска пути.
fn spawn_wall(
    commands: &mut Commands,
    navmesh: &ArcNavmesh,
    centre: Vec2,
    half_length: f32,
    gap: f32,
    side: f32,
) {
    let thickness = 6.0;
    let band = centre + Vec2::new(0.0, side * (gap + thickness / 2.0));
    let size = Vec2::new(half_length * 2.0 + thickness, thickness);

    {
        let mut navmesh = navmesh.write();
        let lo = world_to_tile(band - size / 2.0);
        let hi = world_to_tile(band + size / 2.0);
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                navmesh.set_passable(x, y, false);
            }
        }
    }

    commands.spawn((
        DemoWall,
        Sprite {
            color: Color::srgb(0.42, 0.40, 0.38),
            custom_size: Some(size),
            ..default()
        },
        Transform::from_translation(band.extend(unit_z(band.y) - 1.0)),
    ));
}

/// Вернуть арене проходимость: стены прошлого сценария иначе остались бы в
/// навмеше навсегда — он живёт в ресурсе, а не в сущностях сцены.
fn clear_arena(navmesh: &ArcNavmesh, centre: Vec2) {
    const ARENA_HALF: f32 = 120.0;
    let mut navmesh = navmesh.write();
    let lo = world_to_tile(centre - Vec2::splat(ARENA_HALF));
    let hi = world_to_tile(centre + Vec2::splat(ARENA_HALF));
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            navmesh.set_passable(x, y, true);
        }
    }
}

// ------------------------------------------------------------------ движение

/// Выдать вставшей пешке следующий отрезок маршрута. Заменяет собой весь
/// асинхронный поиск пути: путь строится прямой, но waypoint'ами по центрам
/// навтайлов — ровно в таком виде его отдаёт сеточный A*, и без этого не
/// проверить, стирает ли постановка на waypoint боковой сдвиг.
/// Отрезок идёт через тот же слот назначения, что и цели в игре
/// (`movement::destination`): без этого «воронка» гоняла бы 200 пешек в одну
/// точку — то есть проверяла бы не расталкивание, а очередь к одному тайлу.
fn drive_routes(
    mut commands: Commands,
    navmesh: Res<ArcNavmesh>,
    style: Res<SeparationStyle>,
    search: Res<SlotSearch>,
    mut claims: ResMut<DestinationClaims>,
    mut pawns: Query<
        (
            Entity,
            &mut Movable,
            &mut Route,
            &SimPosition,
            Option<&DestinationClaim>,
        ),
        Without<MovableStateMovingTag>,
    >,
) {
    claims.sync(slot_side(style.human_radius() * 2.0), search.0);
    let navmesh = navmesh.read();
    for (entity, mut movable, mut route, position, claim) in &mut pawns {
        let leg = route.legs[route.next];
        route.next = (route.next + 1) % route.legs.len();
        let desired = world_to_tile(leg);
        let slot = claims.claim_slot(entity, claim.map(|claim| claim.0), desired, |tile| {
            navmesh.is_passable(tile.x, tile.y)
        });
        let (target_tile, target) = match slot {
            Some((slot, tile)) => {
                commands.entity(entity).insert(DestinationClaim(slot));
                (tile, tile_center(tile))
            }
            None => (desired, leg),
        };
        let path = straight_path(position.0, target);
        movable.to_moving(target_tile, path, entity, &mut commands);
    }
}

/// Центры навтайлов вдоль прямой — форма пути сеточного поиска.
fn straight_path(from: Vec2, to: Vec2) -> VecDeque<Vec2> {
    let steps = ((to - from).length() / navtile_size()).ceil().max(1.0) as i32;
    let mut path = VecDeque::new();
    let mut previous = None;
    for step in 1..=steps {
        let point = from.lerp(to, step as f32 / steps as f32);
        let tile = world_to_tile(point);
        if previous != Some(tile) {
            path.push_back(tile_center(tile));
            previous = Some(tile);
        }
    }
    path
}

// -------------------------------------------------------------------- ввод

/// Выбор сценария цифрами. Один системный обработчик на пять клавиш, а не пять
/// систем с `run_if`: это переключатель, а не пять разных действий.
fn pick_scenario(keys: Res<ButtonInput<KeyCode>>, mut scenario: ResMut<Scenario>) {
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

fn toggle_separation(mut style: ResMut<SeparationStyle>) {
    // менять `SeparationStyle` из примера безопасно ровно потому, что здесь нет
    // `SettingsPlugin`: это `SettingsGroup`, и с `PrefsPlugin` тумблер уехал бы
    // в конфиг игры
    style.enabled = !style.enabled;
}

fn toggle_pause(mut time: ResMut<Time<Virtual>>) {
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
fn step_speed(keys: &ButtonInput<KeyCode>, config: &DemoConfig, current: f32) -> Option<f32> {
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

fn apply_speed(speed: Res<DemoSpeed>, mut time: ResMut<Time<Virtual>>) {
    time.set_relative_speed(speed.0);
}

/// Зум колесом. Панорамирования нет намеренно: панелей в сцене нет, но правило
/// «мышь над UI не двигает мир» проще соблюсти, не заводя перетаскивание.
fn zoom_camera(
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

// ------------------------------------------------------------------ метрики

fn count_ticks(mut counters: ResMut<RunCounters>) {
    counters.window_ticks += 1;
}

/// Прогон расталкивания виден по свежему замеру `sim/separation_ms`: система
/// пишет его последним действием, ровно раз за прогон.
fn count_separation_runs(diagnostics: Res<DiagnosticsStore>, mut counters: ResMut<RunCounters>) {
    let time = diagnostics
        .get(&SIM_SEPARATION_MS)
        .and_then(|diagnostic| diagnostic.measurement())
        .map(|measurement| measurement.time);
    if time.is_some() && time != counters.last_measurement {
        counters.last_measurement = time;
        counters.runs += 1;
        counters.window_runs += 1;
    }
    // окно в пару секунд: показывать среднее за весь запуск бессмысленно —
    // скорость и зум по ходу меняют картину
    if counters.window_ticks >= 128 {
        counters.ticks_per_run = if counters.window_runs > 0 {
            counters.window_ticks as f32 / counters.window_runs as f32
        } else {
            f32::INFINITY
        };
        counters.window_ticks = 0;
        counters.window_runs = 0;
    }
}

/// Пересчитать перекрытия. Своя мелкая сетка — та же, что у самого
/// расталкивания ([`SEPARATION_CELL`]), но считаем в лоб: пешек здесь сотни, и
/// понятность важнее скорости.
fn measure_overlaps(
    pawns: Query<&SimPosition, With<DemoPawn>>,
    camera: Query<&Transform, With<Camera2d>>,
    window: Query<&Window>,
    style: Res<SeparationStyle>,
    mut overlaps: ResMut<Overlaps>,
) {
    let (Ok(camera), Ok(window)) = (camera.single(), window.single()) else {
        return;
    };
    let cell = style.cell();
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x;
    let min = camera.translation.truncate() - half_view;
    let max = camera.translation.truncate() + half_view;

    let total = pawns.iter().len();
    let positions: Vec<Vec2> = pawns
        .iter()
        .map(|position| position.0)
        .filter(|position| position.cmpge(min).all() && position.cmple(max).all())
        .collect();

    let mut cells: HashMap<IVec2, Vec<usize>> = HashMap::new();
    for (index, position) in positions.iter().enumerate() {
        cells
            .entry((*position / cell).floor().as_ivec2())
            .or_default()
            .push(index);
    }

    let min_distance = style.radius * 2.0;
    let mut pairs = 0usize;
    let mut worst = 0.0f32;
    let mut sum = 0.0f32;
    let mut involved = vec![false; positions.len()];
    overlaps.links.clear();

    for (index, position) in positions.iter().enumerate() {
        let cell = (*position / cell).floor().as_ivec2();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(neighbours) = cells.get(&(cell + IVec2::new(dx, dy))) else {
                    continue;
                };
                for &other in neighbours {
                    if other <= index {
                        continue;
                    }
                    let overlap = min_distance - position.distance(positions[other]);
                    if overlap > OVERLAP_EPSILON {
                        pairs += 1;
                        sum += overlap;
                        worst = worst.max(overlap);
                        involved[index] = true;
                        involved[other] = true;
                        overlaps.links.push((*position, positions[other]));
                    }
                }
            }
        }
    }

    overlaps.pawns = positions.len();
    overlaps.total = total;
    overlaps.radius = style.radius;
    overlaps.pairs = pairs;
    overlaps.worst = worst;
    overlaps.mean = if pairs > 0 { sum / pairs as f32 } else { 0.0 };
    overlaps.involved = involved.iter().filter(|flag| **flag).count();
    overlaps.bodies = positions
        .iter()
        .zip(involved)
        .map(|(position, flag)| (*position, flag))
        .collect();
}

/// Круг настоящего радиуса тела поверх спрайта: красный — перекрытие, зелёный
/// — дистанция выдержана. Без него «вплотную» и «друг на друге» на спрайтах
/// 1.0 м при дистанции покоя 0.9 м неразличимы.
fn draw_bodies(mut gizmos: Gizmos, overlaps: Res<Overlaps>) {
    const RED: Color = Color::srgb(0.9, 0.1, 0.1);
    for (position, overlapping) in &overlaps.bodies {
        let color = if *overlapping {
            RED
        } else {
            Color::srgb(0.15, 0.55, 0.2)
        };
        gizmos.circle_2d(*position, overlaps.radius, color);
    }
    for (from, to) in &overlaps.links {
        gizmos.line_2d(*from, *to, RED);
    }
}

/// Та же сводка в stdout раз в две реальные секунды: сцену смотрят глазами, но
/// числа надо ещё и приложить к отчёту, а из окна их не скопировать.
fn report_to_stdout(
    real: Res<Time<Real>>,
    scenario: Res<Scenario>,
    style: Res<SeparationStyle>,
    speed: Res<DemoSpeed>,
    overlaps: Res<Overlaps>,
    counters: Res<RunCounters>,
    mut next_report: Local<f32>,
) {
    let now = real.elapsed_secs();
    if now < *next_report {
        return;
    }
    *next_report = now + 2.0;
    println!(
        "{:<20} sep {:<3} {:>4.0}x  in view {:>4}  pairs {:>4}  involved {:>4}  worst {:>6.3}  mean {:>6.3}  ticks/run {:>5.1}",
        scenario.label(),
        if style.enabled { "on" } else { "off" },
        speed.0,
        overlaps.pawns,
        overlaps.pairs,
        overlaps.involved,
        overlaps.worst,
        overlaps.mean,
        counters.ticks_per_run,
    );
}

#[allow(clippy::too_many_arguments)]
fn update_overlay(
    scenario: Res<Scenario>,
    style: Res<SeparationStyle>,
    speed: Res<DemoSpeed>,
    overlaps: Res<Overlaps>,
    counters: Res<RunCounters>,
    time: Res<Time<Virtual>>,
    camera: Query<&Transform, With<Camera2d>>,
    mut overlay: Query<&mut Text, With<OverlayText>>,
) {
    let zoom = camera.single().map(|camera| camera.scale.x).unwrap_or(0.0);
    let gated = zoom >= SEPARATION_MAX_ZOOM;
    let share = if overlaps.pawns > 0 {
        overlaps.involved as f32 / overlaps.pawns as f32 * 100.0
    } else {
        0.0
    };

    let text = format!(
        "{scenario}\n\
         pawns in view {pawns} of {total}   overlapping pairs {pairs}   involved {involved} ({share:.0}%)\n\
         worst {worst:.3} m   mean {mean:.3} m   (rest distance {rest:.2} m, sprite {sprite:.2} m)\n\
         separation {separation}{gate}   speed {speed:.0}x{paused}   zoom {zoom:.3}\n\
         move ticks per separation run {per_run}   runs {runs}\n\
         \n\
         1-5 scenario   R respawn   S separation   Space pause   -/= speed   wheel zoom",
        scenario = scenario.label(),
        pawns = overlaps.pawns,
        total = overlaps.total,
        pairs = overlaps.pairs,
        involved = overlaps.involved,
        share = share,
        worst = overlaps.worst,
        mean = overlaps.mean,
        rest = overlaps.radius * 2.0,
        sprite = HUMAN_SIZE,
        separation = if style.enabled { "ON" } else { "OFF" },
        gate = if gated { " (zoomed out: gated)" } else { "" },
        speed = speed.0,
        paused = if time.is_paused() { " PAUSED" } else { "" },
        zoom = zoom,
        per_run = if counters.ticks_per_run.is_finite() {
            format!("{:.1}", counters.ticks_per_run)
        } else {
            "-".to_string()
        },
        runs = counters.runs,
    );

    for mut overlay in &mut overlay {
        overlay.0 = text.clone();
    }
}
