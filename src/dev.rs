//! Инструменты для отладочных сессий (skill `live-app`): скриншот в файл по
//! клавише F12 или BRP-событию `TakeScreenshotEvent`, и **закадровый** снимок
//! ([`OffscreenShotEvent`]) — тот же кадр, но мимо окна.
//!
//! Закадровый нужен потому, что обычный снимает **поверхность окна**: на
//! заблокированном экране, под другим окном или на спящем дисплее macOS отдаёт
//! чёрный кадр, и проверить картинку нечем. Здесь же кадр рисуется во
//! внеэкранную текстуру своей камерой, и оконный сервер к этому не причастен.
//! Заодно снимок получает **свою рамку**: точку, зум и размер, не трогая
//! камеру пользователя.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::camera_controller::pan_camera::PanCamera;
use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::grid::world_to_tile;
use crate::loading::AppState;
use crate::map::buildings::{BuildingHeightMode, BuildingPlan, mesh_buildings};
use crate::map::inspect::{InspectIndex, InspectOrigin};
use crate::map::osm::MapData;
use crate::map::surface::MapLayer;
use crate::map::{
    FenceZoomBucket, ParkingLayout, RailZoomBucket, RoadShapeOnMap, RoadStyle, TreeStyle,
    VergeZoomBucket, build_roads, mesh_fences, mesh_rails, mesh_surfaces,
};
use crate::movement::Movable;
use crate::settings::unit_z;

const SCREENSHOT_PATH: &str = "screenshot.png";
const OFFSCREEN_PATH: &str = "offscreen.png";
/// Размер закадрового кадра по умолчанию, px. Длинная сторона ровно 1568 —
/// граница, за которой картинку всё равно ужмут при чтении, так что кадр
/// доезжает до глаз без пережатия.
const OFFSCREEN_SIZE: UVec2 = UVec2::new(1568, 980);
/// Сколько кадров дать камере отрисоваться, прежде чем снимать. Одного мало:
/// первый уходит на то, чтобы цель вообще появилась в графе рендера, второй —
/// на сам кадр; а когда снимок ещё и двигает зум пользовательской камеры (см.
/// [`on_offscreen_shot`]), несколько кадров нужны слоям с зум-LOD на пересборку.
const WARMUP_FRAMES: u32 = 6;
/// Нижняя граница стороны кадра, px: `size` приходит из BRP, а текстура с
/// нулевой стороной — ошибка валидации wgpu, то есть падение по чужому вводу.
/// 16 — заведомо безопасный минимум, замером он не выбирался.
const OFFSCREEN_MIN_SIDE: u32 = 16;

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct TakeScreenshotEvent;

/// Закадровый снимок: кадр рисуется во внеэкранную текстуру и кладётся в файл,
/// минуя окно. Все поля необязательны — пустое событие снимает `offscreen.png`
/// [`OFFSCREEN_SIZE`] (1568 × 980) с текущей позиции камеры и текущим зумом:
///
/// ```text
/// brp event OffscreenShotEvent '{"at":[2300,1900],"zoom":0.4,"path":"gsk.png"}'
/// ```
// `reflect(Default)` обязателен: BRP собирает событие из **частичного**
// JSON (`{"at": [...]}`), и недостающие поля берутся из `Default`
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event, Default)]
pub struct OffscreenShotEvent {
    /// Куда смотреть, м. `None` — туда же, куда смотрит камера пользователя.
    pub at: Option<Vec2>,
    /// Метров на условный пиксель кадра. `None` — как у камеры.
    pub zoom: Option<f32>,
    /// Размер кадра, px.
    pub size: Option<UVec2>,
    /// Куда положить файл; путь относительно рабочего каталога.
    pub path: Option<String>,
}

/// Камера закадрового снимка: живёт ровно столько кадров, сколько нужно, чтобы
/// её цель успела отрисоваться.
#[derive(Component)]
struct OffscreenCamera {
    target: Handle<Image>,
    path: String,
    frames: u32,
    /// Зум пользовательской камеры до снимка — вернуть, когда снято.
    /// `None` — зум не трогали.
    restore_zoom: Option<f32>,
}

/// Тестовый агент навигации: спавнится в `from` и идёт в `to` (метры).
/// Триггерится по BRP: `brp event SpawnTestWalkerEvent '{"from":[..],"to":[..]}'`.
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct SpawnTestWalkerEvent {
    pub from: Vec2,
    pub to: Vec2,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct TestWalker;

/// Проба слоёв: что карты нарисовано в точке. Ответ — [`LayerProbe`], один
/// ресурс на последнюю пробу, читается по BRP (`brp probe`).
///
/// Это «какой слой рисует глитч» одним запросом вместо часа eliminations:
/// скрытий-пересниманий и временных проб, вписываемых в исходник на время
/// дефекта. Обсервер выполняется синхронно внутри BRP-вызова
/// (`world.trigger_event` зовёт `World::trigger`), так что ответ готов к
/// моменту, когда `brp probe` читает ресурс.
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event, Default)]
pub struct ProbeLayersEvent {
    /// Точка в метрах карты.
    pub at: Vec2,
    /// Радиус поиска ближайших вершин, м; `None` — [`PROBE_RADIUS`].
    pub radius: Option<f32>,
}

/// Ответ последней [`ProbeLayersEvent`]: слои под точкой, верхние первыми, с
/// числом накрывающих точку треугольников и ближайшими вершинами.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource, Default)]
pub struct LayerProbe {
    pub report: String,
}

/// Радиус поиска ближайших вершин по умолчанию, м.
const PROBE_RADIUS: f32 = 2.0;
/// Сколько ближайших вершин показывать на слой.
const PROBE_NEAREST: usize = 3;
/// Порог «на ребре — считаем внутри», м²: знаковый тест с нулём на ребре
/// выкидывает точку, стоящую ровно на кромке, — как раз то место, из-за
/// которого пробу и делают.
const PROBE_EDGE: f32 = 1e-6;

/// Точка в треугольнике; лежащая на ребре — внутри.
fn in_triangle(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> bool {
    if (b - a).perp_dot(c - a).abs() <= f32::EPSILON {
        return false; // вырожденный
    }
    let d = [
        (b - a).perp_dot(p - a),
        (c - b).perp_dot(p - b),
        (a - c).perp_dot(p - c),
    ];
    d.iter().all(|v| *v >= -PROBE_EDGE) || d.iter().all(|v| *v <= PROBE_EDGE)
}

/// Одна строка отчёта пробы: слой с точной геометрией ([`on_probe_layers`]).
struct ProbeRow {
    name: &'static str,
    z: f32,
    covering: usize,
    triangles: usize,
    vertices: usize,
    nearest: Vec<(f32, Vec2)>,
    /// Сущность живого мира с этим именем — чтобы `brp hide` не искал её
    /// вручную; `None` — слоя сейчас нет в мире (пересборка на другой ступени).
    entity: Option<Entity>,
    hidden: bool,
}

/// Слой живого мира, как он нужен отчёту пробы: сущность, имя, z, скрытость
/// и то, накрывает ли габарит (`Aabb`) точку пробы.
struct LiveLayer<'a> {
    entity: Entity,
    name: &'a str,
    z: f32,
    hidden: bool,
    covers: bool,
}

/// Входы пересборки слоёв для пробы: те же ресурсы, что читают адаптеры
/// (`roads::rebuild_roads`, `rail::build_rails`…), одним параметром — идиома
/// `map::surface::LayerMaterials`.
#[derive(SystemParam)]
struct ProbeSources<'w> {
    map: Res<'w, MapData>,
    road_style: Res<'w, RoadStyle>,
    road_shape: Res<'w, RoadShapeOnMap>,
    verge_bucket: Res<'w, VergeZoomBucket>,
    rail_bucket: Res<'w, RailZoomBucket>,
    fence_bucket: Res<'w, FenceZoomBucket>,
    height_mode: Res<'w, BuildingHeightMode>,
}

fn on_probe_layers(
    event: On<ProbeLayersEvent>,
    mut probe: ResMut<LayerProbe>,
    sources: ProbeSources,
    index: Res<InspectIndex>,
    origin: Res<InspectOrigin>,
    trees: Res<TreeStyle>,
    layers: Query<(Entity, &Name, &Aabb, &Transform, &Visibility), With<MapLayer>>,
) {
    let ProbeSources {
        map,
        road_style,
        road_shape,
        verge_bucket,
        rail_bucket,
        fence_bucket,
        height_mode,
    } = sources;
    let at = event.at;
    let radius = event.radius.unwrap_or(PROBE_RADIUS).max(0.0);
    let started = std::time::Instant::now();

    // Живой мир: z, скрытость и габариты (Aabb) посаженных слоёв. Вершин у
    // живых мешей в главном мире нет — слои собираются с RenderAssetUsages::
    // RENDER_WORLD и после извлечения в рендер пусты, поэтому точная геометрия
    // ниже пересобирается теми же чистыми `mesh_*`, что зовут адаптеры; Aabb
    // остаётся, потому что считается при спавне, до извлечения. Габариты стоят
    // в мировых метрах, точка пришла в метрах карты: витрина `roads` ставит
    // [`InspectOrigin`], у игры он ноль.
    let world_at = at + origin.0;
    let mut live: Vec<LiveLayer> = Vec::new();
    for (entity, name, aabb, transform, visibility) in &layers {
        let (min, max) = (aabb.min(), aabb.max());
        let covers = world_at.x >= min.x
            && world_at.x <= max.x
            && world_at.y >= min.y
            && world_at.y <= max.y;
        live.push(LiveLayer {
            entity,
            name: name.as_str(),
            z: transform.translation.z,
            hidden: *visibility == Visibility::Hidden,
            covers,
        });
    }

    // Пересборка: те же входы, что у адаптеров пересборки. Кроны не входят —
    // они рассыпаются в сущности (куски — уже слои живого мира выше).
    let layout = ParkingLayout::new(&map.parking, &map.roads, map.traffic_side);
    let (surfaces, _) = mesh_surfaces(&map, &layout);
    let roads = build_roads(&map, *road_style, road_shape.0, *verge_bucket).layers;
    let (rails, _) = mesh_rails(*rail_bucket, &map.rails);
    let (fences, _) = mesh_fences(*fence_bucket, &map.fences, &map.roads);
    let (buildings, _) =
        mesh_buildings(BuildingPlan::game(*height_mode), &map.buildings, &map.roads);
    // имена всего пересобранного: слой из этого набора в хвост отчёта не
    // попадает — его геометрию уже спросили точно, далеко она или рядом
    let rebuilt: std::collections::BTreeSet<&str> = surfaces
        .iter()
        .chain(&roads)
        .chain(&rails)
        .chain(&fences)
        .chain(&buildings.layers)
        .chain(&buildings.shadows)
        .map(|layer| layer.name)
        .collect();

    let mut rows: Vec<ProbeRow> = surfaces
        .iter()
        .chain(&roads)
        .chain(&rails)
        .chain(&fences)
        .chain(&buildings.layers)
        .chain(&buildings.shadows)
        .map(|layer| {
            let covering = layer
                .builder
                .triangles()
                .filter(|[a, b, c]| in_triangle(*a, *b, *c, at))
                .count();
            let nearest = layer
                .builder
                .nearest_vertices(at, radius, PROBE_NEAREST)
                .into_iter()
                .map(|(d2, p)| (d2.sqrt(), p))
                .collect();
            // одноимённый слой живого мира — оттуда сущность для `brp hide`
            // и его скрытость
            let (entity, hidden) = live
                .iter()
                .find(|live_layer| live_layer.name == layer.name)
                .map_or((None, false), |live_layer| {
                    (Some(live_layer.entity), live_layer.hidden)
                });
            ProbeRow {
                name: layer.name,
                z: layer.z,
                triangles: layer.builder.triangles().count(),
                vertices: layer.builder.vertex_count(),
                covering,
                nearest,
                entity,
                hidden,
            }
        })
        .filter(|row| row.covering > 0 || !row.nearest.is_empty())
        .collect();
    rows.sort_by(|a, b| b.z.total_cmp(&a.z));

    let mut report = format!(
        "probe at {:.1}, {:.1} radius {:.1} — {} слоёв с геометрией рядом, {:.1?} на пересборку\n",
        at.x,
        at.y,
        radius,
        rows.len(),
        started.elapsed()
    );
    for row in &rows {
        let nearest = row
            .nearest
            .iter()
            .map(|(d, p)| format!("({:.1},{:.1}) {:.2} м", p.x, p.y, d))
            .collect::<Vec<_>>()
            .join("; ");
        let entity = row
            .entity
            .map_or_else(|| "-".to_string(), |e| e.index().to_string());
        report.push_str(&format!(
            "  z {:7.3}  {:>8}  {:<22} tris {}/{}  verts {}  near {}{}\n",
            row.z,
            entity,
            row.name,
            row.covering,
            row.triangles,
            row.vertices,
            nearest,
            if row.hidden { "  [hidden]" } else { "" }
        ));
    }

    // Семантический индекс (`map/inspect.rs`): признаки объектов в точке —
    // то, что панель инспектора (`ui/inspector.rs`) показывает на каждом
    // движении курсора. Дёшево: одна ячейка сетки, без пересборки. Слой
    // отвечает «какой меш рисует пиксель», индекс — «что это по данным».
    let hits = index.hits(&map, trees.density, at);
    report.push_str(&format!(
        "features at the point (semantic index): {}\n",
        hits.len()
    ));
    for hit in &hits {
        report.push_str(&format!("  z {:6.3}  {}\n", hit.z, hit.label));
    }

    // Слои вне пересборки, чей габарит накрывает точку (кроны, их тени — всё,
    // что рассыпается в сущности): свернуть по имени, чтобы 200 кусков не
    // занимали отчёт
    let mut by_name: std::collections::BTreeMap<&str, (usize, f32, f32, bool)> =
        std::collections::BTreeMap::new();
    for live_layer in &live {
        if !live_layer.covers || rebuilt.contains(live_layer.name) {
            continue;
        }
        let entry = by_name.entry(live_layer.name).or_insert((
            0,
            live_layer.z,
            live_layer.z,
            live_layer.hidden,
        ));
        entry.0 += 1;
        entry.1 = entry.1.max(live_layer.z);
        entry.2 = entry.2.min(live_layer.z);
        entry.3 |= live_layer.hidden;
    }
    for (name, (count, top, bottom, hidden)) in by_name {
        let span = if (top - bottom).abs() > f32::EPSILON {
            format!("z {top:.3}–{bottom:.3}")
        } else {
            format!("z {top:.3}")
        };
        report.push_str(&format!(
            "  {span:>15}  {name} ×{count} — по габариту; точной геометрии нет (вне пересборки){}\n",
            if hidden { "  [hidden]" } else { "" }
        ));
    }
    info!("{}", report.trim_end());
    probe.report = report;
}

/// Порт BRP: `BRP_PORT` из окружения, иначе дефолтный 15702. `None` — порт
/// занят чужим экземпляром, BRP у этого запуска не будет.
///
/// Порт из окружения нужен, чтобы экземпляр агента (`BRP_PORT=15703 cargo run`)
/// не дрался за порт с тем, который пользователь держит открытым. Занятость
/// проверяется до старта, потому что `bevy_remote` спавнит сервер через
/// `.detach()` и ошибку bind не видит никто: второй экземпляр молча остаётся
/// без BRP, а клиент получает ответы от первого — и это выглядит как
/// «изменение не сработало», а не как «я разговариваю с чужим приложением».
///
/// Явно заданный порт при этом обязателен: если он занят, запуск падает, а не
/// продолжается без BRP. Дефолтный порт — только предупреждение, чтобы второе
/// окно, открытое руками, всё-таки запускалось. Игре и витрине `roads` он
/// нужен поровну.
pub fn brp_port() -> Option<u16> {
    let requested = std::env::var("BRP_PORT").ok();
    let port = match &requested {
        Some(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("BRP_PORT is not a port number: {value:?}")),
        None => bevy::remote::http::DEFAULT_PORT,
    };
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        // Сокет закрывается сразу, bind'ит его уже сам RemoteHttpPlugin
        Ok(_) => Some(port),
        Err(err) if requested.is_some() => {
            panic!(
                "BRP port {port} is busy ({err}) — another instance already holds it; pick a free BRP_PORT"
            )
        }
        Err(err) => {
            eprintln!(
                "qwe: BRP port {port} is busy ({err}) — remote protocol off for this instance"
            );
            None
        }
    }
}

/// `QWE_PERF_NOVSYNC=1` — окно без vsync: кадр не упирается в 60 Гц дисплея,
/// и `frame_time` в логе диагностики становится измеримым. Без переменной —
/// обычный `AutoVsync`.
pub fn perf_present_mode() -> bevy::window::PresentMode {
    if std::env::var_os("QWE_PERF_NOVSYNC").is_some() {
        bevy::window::PresentMode::AutoNoVsync
    } else {
        bevy::window::PresentMode::AutoVsync
    }
}

/// `QWE_PERF_WINDOW=WxH` — логический размер окна при запуске (замер цены
/// разрешения). Без переменной — 1920 × 1080.
pub fn perf_window_resolution() -> bevy::window::WindowResolution {
    let Ok(spec) = std::env::var("QWE_PERF_WINDOW") else {
        return (1920, 1080).into();
    };
    let size = spec
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
    let Some(size) = size else {
        panic!("QWE_PERF_WINDOW is not WxH: {spec:?}");
    };
    size.into()
}

/// Переключатели замеров из окружения, которые ставятся внутри приложения:
///
/// - `QWE_PERF_POP=<n>` — `n` людей вместо штатной популяции (цена спрайтов);
/// - `QWE_PERF_RENDER_DIAG=1` — `RenderDiagnosticsPlugin`: CPU-время каждого
///   прохода рендера (`render/<pass>/elapsed_cpu`) в логе диагностики;
/// - `QWE_PERF_FRAME_SPIKES=<ms>` — кадр длиннее порога печатает строку
///   `frame spike: 43.2 ms (zoom 1.83)`: детектор рывков зума, которых
///   секундные средние `frame_time` не показывают. Реальное время кадра
///   ([`Time<Real>`]), не виртуальное: на паузе виртуальное стоит на нуле.
///
/// Без переменных ничего не меняется.
fn add_perf_toggles(app: &mut App) {
    if let Ok(n) = std::env::var("QWE_PERF_POP") {
        let n: usize = n
            .parse()
            .unwrap_or_else(|_| panic!("QWE_PERF_POP is not a number: {n:?}"));
        app.insert_resource(crate::human::PopulationSize(n));
    }
    if std::env::var_os("QWE_PERF_RENDER_DIAG").is_some() {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    if let Ok(ms) = std::env::var("QWE_PERF_FRAME_SPIKES") {
        let ms: f32 = ms
            .parse()
            .unwrap_or_else(|_| panic!("QWE_PERF_FRAME_SPIKES is not a number: {ms:?}"));
        app.insert_resource(FrameSpikeThreshold(ms));
        app.add_systems(Last, log_frame_spikes.run_if(in_state(AppState::Playing)));
    }
}

/// Порог `QWE_PERF_FRAME_SPIKES`, в мс длины кадра.
#[derive(Resource)]
struct FrameSpikeThreshold(f32);

/// Каждый кадр длиннее порога — строка в лог с его длиной и зумом камеры:
/// зум в той же строке привязывает пик к порогу зум-LOD, через который
/// прошло в этом кадре колесо.
fn log_frame_spikes(
    time: Res<Time<Real>>,
    camera: Single<&PanCamera, With<Camera2d>>,
    threshold: Res<FrameSpikeThreshold>,
    mut frames: Local<u64>,
) {
    *frames += 1;
    let ms = time.delta_secs() * 1000.0;
    if ms >= threshold.0 {
        info!(
            "frame spike: {ms:.1} ms (zoom {:.2}, frame {})",
            camera.zoom_factor, *frames
        );
    }
}

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        add_perf_toggles(app);
        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin::default(),
        ))
        .register_type::<TakeScreenshotEvent>()
        .register_type::<OffscreenShotEvent>()
        .register_type::<SpawnTestWalkerEvent>()
        .register_type::<TestWalker>()
        .register_type::<ProbeLayersEvent>()
        .register_type::<LayerProbe>()
        .init_resource::<LayerProbe>()
        .add_observer(on_take_screenshot)
        .add_observer(on_offscreen_shot)
        .add_observer(on_spawn_test_walker)
        .add_observer(on_probe_layers)
        .add_systems(Update, capture_offscreen)
        .add_systems(
            Update,
            trigger_screenshot.run_if(bevy::input::common_conditions::input_just_pressed(
                KeyCode::F12,
            )),
        );
    }
}

fn on_spawn_test_walker(event: On<SpawnTestWalkerEvent>, mut commands: Commands) {
    let mut movable = Movable::new(4.0);
    let entity = commands
        .spawn((
            Sprite {
                color: Color::srgb(0.1, 0.1, 0.9),
                custom_size: Some(Vec2::splat(2.0)),
                ..default()
            },
            Transform::from_translation(event.from.extend(unit_z(event.from.y))),
            TestWalker,
            // ходок срочен: он и раньше им был — правило `!is_human` считало
            // срочным всё, что не человек, а вида у ходока нет вовсе. Без
            // маркера его заявка попала бы в очередь гуляющих и ждала камеру
            crate::movement::UrgentPath,
            DespawnOnExit(AppState::Playing),
            Name::new("test_walker"),
        ))
        .id();
    movable.to_pathfinding(
        entity,
        world_to_tile(event.from),
        world_to_tile(event.to),
        &mut commands,
    );
    commands.entity(entity).insert(movable);
}

fn trigger_screenshot(mut commands: Commands) {
    commands.trigger(TakeScreenshotEvent);
}

fn on_take_screenshot(_event: On<TakeScreenshotEvent>, mut commands: Commands) {
    info!("saving screenshot to {SCREENSHOT_PATH}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(SCREENSHOT_PATH));
}

/// Своя камера во внеэкранную текстуру. Пост-обработка у неё та же, что у
/// пользовательской (`post::camera_post_process`), иначе снимок показывал бы
/// не то, что видно на экране: без bloom портал, ореолы демонов и искры душ
/// теряют свечение. Виньетка (`post.rs`) в кадр не попадает — она UI-нода и
/// живёт на камере панелей.
fn on_offscreen_shot(
    event: On<OffscreenShotEvent>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    // не просто `With<Camera2d>`: закадровая камера, которую этот же обсервер
    // и спавнит, тоже `Camera2d`, и пока она жива (несколько кадров) `Single`
    // матчил бы две сущности. Обсервер при этом **молча** пропускается — второе
    // событие подряд не оставляло ни файла, ни строки в логе. Здесь фильтр
    // держит уже сам `&mut PanCamera`: он есть только у камеры пользователя
    camera: Single<(&mut Transform, &Projection, &mut PanCamera), With<Camera2d>>,
    antialias: Res<crate::post::Antialias>,
) {
    let (mut transform, projection, mut controller) = camera.into_inner();
    let size = event
        .size
        .unwrap_or(OFFSCREEN_SIZE)
        .max(UVec2::splat(OFFSCREEN_MIN_SIDE));
    let at = event.at.unwrap_or(transform.translation.truncate());
    let zoom = event.zoom.unwrap_or(transform.scale.x).max(f32::EPSILON);
    let path = event.path.clone().unwrap_or(OFFSCREEN_PATH.to_string());

    // Ступени зум-LOD (машины, оборудование на кровле, шпалы) считаются по
    // **пользовательской** камере: у слоёв один меш на всех, и своей ступени
    // у второго вида быть не может. Поэтому снимок с другим зумом на время
    // переставляет зум той камеры — иначе кадр показывал бы шпалы там, где
    // их на таком плане не рисуют, и не показывал бы там, где рисуют.
    let restore_zoom = (zoom != transform.scale.x).then_some(transform.scale.x);
    if restore_zoom.is_some() {
        transform.scale = Vec3::splat(zoom);
        controller.zoom_factor = zoom;
    }

    let mut image = Image::new_fill(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // цель рендера и источник копирования: первое — чтобы в неё рисовать,
    // второе — чтобы `Screenshot::image` смог её прочитать
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    let target = images.add(image);

    info!("offscreen shot: {size} at {at} zoom {zoom} -> {path}");
    commands.spawn((
        Camera2d,
        Camera {
            // раньше пользовательской в общем порядке отрисовки. Это не
            // защита от предупреждения о неоднозначности: `sort_cameras`
            // (bevy_render) ключует его парой (order, target), а цели здесь
            // разные — окно и текстура, так что одинаковый порядок движок бы
            // и не заметил
            order: -1,
            ..default()
        },
        // в 0.19 цель — самостоятельный компонент, а не поле `Camera`
        RenderTarget::Image(target.clone().into()),
        projection.clone(),
        Transform::from_translation(at.extend(0.0)).with_scale(Vec3::splat(zoom)),
        // снимок сглажен так же, как окно
        antialias.msaa(),
        crate::post::camera_post_process(),
        OffscreenCamera {
            target,
            path,
            frames: 0,
            restore_zoom,
        },
        Name::new("offscreen_camera"),
    ));
}

/// Дать камере отрисоваться [`WARMUP_FRAMES`] кадров, снять её цель и **на
/// следующем кадре** убрать камеру.
///
/// Снять и убрать в один кадр нельзя, и это не осторожность, а устройство
/// движка: `prepare_screenshots` подменяет выходное вложение цели своей
/// текстурой на тот кадр, в котором снимок запрошен, — то есть рисует в неё
/// **сама камера**, и если её в этом кадре уже нет, в файл уходит чистый ноль.
fn capture_offscreen(
    mut commands: Commands,
    mut cameras: Query<(Entity, &mut OffscreenCamera)>,
    // камера пользователя: у закадровой `PanCamera` нет, так что сама выборка
    // и есть фильтр
    main: Single<(&mut Transform, &mut PanCamera), With<Camera2d>>,
) {
    let (mut transform, mut controller) = main.into_inner();
    for (entity, mut shot) in &mut cameras {
        shot.frames += 1;
        match shot.frames.cmp(&WARMUP_FRAMES) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                commands
                    .spawn(Screenshot::image(shot.target.clone()))
                    .observe(save_to_disk(shot.path.clone()));
            }
            std::cmp::Ordering::Greater => {
                // зум вернуть только теперь: слои с зум-LOD пересобираются по
                // нему, и ранний возврат успел бы попасть в снимаемый кадр
                if let Some(zoom) = shot.restore_zoom {
                    transform.scale = Vec3::splat(zoom);
                    controller.zoom_factor = zoom;
                }
                commands.entity(entity).despawn();
            }
        }
    }
}
