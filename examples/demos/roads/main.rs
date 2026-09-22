//! Витрина пересечений дорог: типовые узлы города — крестовины, Т и Y,
//! кольца, переход широкой в узкую, въезд во двор — колонкой, один под другим.
//!
//! **Пример здесь — данные OSM, а не геометрия.** Каждый узел — срез
//! настоящей выгрузки Overpass, нарезанный при запуске из кеша города
//! (`map::osm::crop`), и проходит тот же путь, что карта в игре: игровой `parse`
//! со всеми его проходами (дома с тротуаров, кварталы к дорогам, стоянки до
//! проездов) → `MapData` → игровые `mesh_*` → игровые `spawn_*`. Своей геометрии
//! у витрины нет вовсе, так что на одном перекрёстке виден весь процесс «данные
//! OSM → рендер», и любую его стадию можно потрогать: выгрузить срез файлом
//! (`ROADS_DUMP`), положить замороженным, поправить и нажать `F5`. Что такое
//! пример и откуда берётся его адрес — в [`samples`].
//!
//! **Координаты под примером — игровые.** Вырезка проецируется проекцией своего
//! города, поэтому разобранный узел стоит в тех же метрах карты, что и в игре
//! (`brp cam x y` приведёт к нему же), а в колонку его ставит сдвиг уже
//! собранных слоёв. Сдвиг делается **после** спавна и по факту появления меша
//! ([`place_new`]), а не параметром сборки: так слои кладут игровые двери
//! (`spawn_road_meshes`, `spawn_building_meshes`, `spawn_tree_meshes`) как есть,
//! без варианта «со смещением» у каждой. Отсюда же сборка по примеру на кадр —
//! между спавном и сдвигом должно быть ясно, чей это меш.
//!
//! Вырезка шире видимого окна (`--margin`), а собранные слои режутся по окну
//! (`MeshBuilder::clip_to_rect`): обрезанные концы дорог, тупиковые разрывы
//! разметки на них и половины теней остаются за кадром, а в окне узел выглядит
//! так же, как посреди города. Резать обязательно, маской поверх не обойтись:
//! окна стоят встык, и поле одного примера иначе ложится в окно соседа.
//!
//! Слои — те, что видны на перекрёстке: поверхности (со стоянками и их
//! разметкой), дороги, дома, ограды, рельсы, деревья. Трамвай и промзона в игре
//! по умолчанию выключены, вагоны стоят только на станционных путях — их здесь
//! нет. **Машин нет нарочно**: витрина про полотно и узел, а ряд у бордюра
//! закрывает ровно их — кромку, радиус примыкания, разметку у перекрёстка.
//! Ступени зума взяты ближние: три десятка окон по сотне метров — не город,
//! экономить тут нечего.
//!
//! Пример не трогает конфиг игры: ни `PrefsPlugin`, ни `MapPlugin` — `City`,
//! `RoadStyle` и `RoadShape` здесь обычные ресурсы с игровыми дефолтами.
//! Ползунки формы доезжают до примеров после паузы (`settle_road_shape`), как
//! в игре; ширина полосы уходит в глобаль разбора перед нарезкой, так что
//! пример с другой шириной разобран заново, а не растянут.
//!
//! ```text
//! cargo run --example roads
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `↑` `↓` | к соседнему примеру |
//! | `F5` | перечитать кеш города и нарезать срезы заново |
//!
//! `ROADS_DUMP=папка` — выгрузить туда каждый срез файлом (замороженный срез
//! для `data/<город>/`);
//! `ROADS_SHOT=путь.png` — поднять окно, снять витрину и выйти;
//! `ROADS_SAMPLE=N` ставит камеру на пример N (с единицы) крупным планом;
//! `ROADS_CITY=<slug>` (`berlin`, `paris`…) открывает витрину на этом городе —
//! для автоснимка не Тулы.

mod overlay;
mod panel;
mod samples;
#[path = "../gallery_shot.rs"]
mod shot;

use bevy::asset::RenderAssetUsages;
use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::sprite_render::Material2dPlugin;
use bevy::text::TextBounds;
use bevy::window::PrimaryWindow;
use qwe::camera::{drag_pan, hovering_ui, key_pan, pan_controller, zoom_to_cursor};
use qwe::city::City;
use qwe::map::buildings::material::{RoofMaterial, init_roof_material};
use qwe::map::buildings::{
    BuildingPlan, BuildingZoomBucket, mesh_buildings, spawn_building_meshes,
};
use qwe::map::osm::parse::parse_response;
use qwe::map::surface::{
    LayerMesh, SurfaceMaterial, init_flat_materials, init_surface_materials,
    retune_surface_materials, retunes_on, spawn_layers,
};
use qwe::map::trees::{
    ConiferField, ConiferNoiseStyle, CrownMaterial, CrownParams, TreeMaterials, TreeRowStyle,
    TreeStyle, mesh_trees, spawn_tree_meshes,
};
use qwe::map::{
    BuildingHeightMode, FenceZoomBucket, GROUND_COLOR, MeshBuilder, PaintMaterial, ParkingLayout,
    RailZoomBucket, RoadPaintStyle, RoadShape, RoadShapeOnMap, RoadStyle, RoofStyle, SunOnMap,
    SurfaceStyle, apply_sun, mesh_fences, mesh_rails, mesh_roads, mesh_surfaces,
    mesh_tree_row_band, set_lane_width, settle_road_shape, spawn_road_meshes,
};
use qwe::ui::knob::AddKnobsExt;
use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::overlay::NetworkOverlay;
use crate::panel::{StatusLine, spawn_panel, sync_city_buttons};
use crate::samples::Sample;
use crate::shot::{ShotRequest, auto_shot, request_shot};

const WINDOW_WIDTH: f32 = 1500.0;
const WINDOW_HEIGHT: f32 = 950.0;

/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, сцена — в
/// метрах.
const TEXT_SCALE: f32 = 0.25;
const TITLE_FONT: f32 = 34.0;
const BODY_FONT: f32 = 22.0;
/// Зазор между окнами соседних примеров и между окном и его подписью, м.
const GAP: f32 = 30.0;
/// Высота, которую занимает подпись примера (заголовок, адрес, заметка и три
/// строки стадий), м — с запасом на перенос длинной заметки.
const CAPTION_HEIGHT: f32 = 110.0;
/// Ширина колонки подписей слева от окон, м.
const CAPTION_WIDTH: f32 = 220.0;
/// Поля вокруг кадра при стартовом зуме, доля его размера.
const VIEW_MARGIN: f32 = 1.05;

/// Рамки окон и подписи лежат над всем, что рисует карта (кроны — `Z_TREE` 20).
const Z_FRAME: f32 = 30.0;
const Z_CAPTION: f32 = 31.0;
const FRAME_WIDTH: f32 = 0.6;

const INK: Color = Color::srgb(0.14, 0.16, 0.20);
const INK_DIM: Color = Color::srgb(0.34, 0.36, 0.40);

/// Всё, что витрина положила в мир и уберёт при пересборке: сдвинутые слои
/// карты, рамки окон, подписи. У слоёв карты метка заодно значит «уже сдвинут».
#[derive(Component)]
struct Placed;

/// Метка слоёв, которые витрина кладёт общим `spawn_layers` сама (поверхности,
/// ограды, рельсы, полоса аллей): своей игровой метки пересборки им
/// здесь не нужно — пересобирается витрина целиком.
#[derive(Component, Clone, Copy)]
struct SampleLayer;

/// Очередь сборки: примеры города, их места в колонке и сколько уже собрано.
#[derive(Resource, Default)]
struct Gallery {
    samples: Vec<Sample>,
    /// Центр окна каждого примера в мире витрины.
    slots: Vec<Vec2>,
    built: usize,
    /// Сдвиг для мешей, заспавненных в этом кадре: из игровых координат
    /// примера — в его место в колонке.
    pending_shift: Option<Vec2>,
}

impl Gallery {
    fn max_half(&self) -> f32 {
        self.samples
            .iter()
            .map(|sample| sample.half)
            .fold(0.0, f32::max)
    }
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe roads — типовые пересечения: данные OSM → рендер".to_string(),
                        resolution: (WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // `roads=info` — свои строки витрины: цена чтения кеша и
                    // `geo` примера, добавленного по `at`
                    level: bevy::log::Level::INFO,
                    filter: "warn,qwe=warn,roads=info".to_string(),
                    ..default()
                }),
        )
        .add_plugins(PanCameraPlugin)
        .add_plugins(Material2dPlugin::<SurfaceMaterial>::default())
        .add_plugins(Material2dPlugin::<PaintMaterial>::default())
        .add_plugins(Material2dPlugin::<RoofMaterial>::default())
        .add_plugins(Material2dPlugin::<CrownMaterial>::default())
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .add_plugins(qwe::ui::QuitOnEscPlugin)
        .add_plugins(qwe::ui::AgentBadgePlugin)
        .insert_resource(start_city())
        .init_resource::<RoadStyle>()
        .init_resource::<RoadShape>()
        .init_resource::<RoadShapeOnMap>()
        .init_resource::<RoadPaintStyle>()
        .init_resource::<RoofStyle>()
        .init_resource::<SurfaceStyle>()
        .init_resource::<SunOnMap>()
        .init_resource::<Gallery>()
        .init_resource::<NetworkOverlay>()
        // подписи строк стиля ведёт кит — по разу на ресурс, как в игре
        .add_knobs::<RoadStyle>()
        .add_knobs::<RoadShape>()
        .add_knobs::<RoadPaintStyle>()
        .add_knobs::<NetworkOverlay>()
        .insert_resource(ClearColor(GROUND_COLOR))
        .add_systems(
            Startup,
            (
                spawn_camera,
                // солнце — до кровельного материала: его юниформ `light`
                // пишется один раз на всё приложение
                (apply_sun, init_roof_material).chain(),
                init_surface_materials,
                init_flat_materials,
                spawn_panel,
                request_shot("ROADS_SHOT"),
            ),
        )
        .add_systems(
            Update,
            (
                // колесо над панелью принадлежит панели: тот же гейт, что у
                // игры, — ввод, адресованный UI, в мир не идёт
                zoom_to_cursor.run_if(not(hovering_ui)),
                // протяжка решает «камера или панель» сама, в кадре нажатия
                (drag_pan, key_pan),
                step_to_neighbour,
                sync_city_buttons.run_if(resource_changed::<City>),
                // форма — после паузы, как в игре; ширина полосы — в глобаль
                // разбора, краски и колеи раньше, чем её прочтут материалы
                settle_road_shape,
                apply_lane_width.run_if(resource_changed::<RoadShapeOnMap>),
                // краска и колея — юниформы, как в игре: слои не пересобираются
                retune_surface_materials
                    .run_if(retunes_on().or_else(resource_changed::<RoadShapeOnMap>)),
                // на первом кадре оба ресурса числятся изменёнными — первая
                // сборка идёт той же дорогой, что и всякая следующая
                reload.run_if(
                    resource_changed::<City>
                        .or_else(resource_changed::<RoadStyle>)
                        .or_else(resource_changed::<RoadShapeOnMap>)
                        .or_else(resource_changed::<NetworkOverlay>)
                        .or_else(input_just_pressed(KeyCode::F5)),
                ),
                build_next,
                place_new,
                auto_shot.run_if(resource_exists::<ShotRequest>),
            )
                .chain(),
        )
        .run();
}

/// Полоса окна, занятая панелью: отступ от края, сама панель и такой же зазор.
/// Город, с которого открывается витрина: `ROADS_CITY`, иначе игровой
/// дефолт.
fn start_city() -> City {
    let requested = std::env::var("ROADS_CITY").ok();
    City::ALL
        .into_iter()
        .find(|city| requested.as_deref() == Some(city.slug()))
        .unwrap_or_default()
}

fn panel_span() -> f32 {
    PANEL_WIDTH_PX + 2.0 * UI_SCREEN_EDGE_PX_OFFSET
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Msaa::Off,
        // контроллер и всё движение — игровые: пределы зума, колесо к курсору,
        // WASD и протяжка в экранной скорости (`camera::key_pan`/`drag_pan`)
        pan_controller(1.0),
    ));
}

/// Камера на пример: окно с подписью по ширине свободной части экрана — или,
/// крупным планом, одно окно по высоте.
fn frame_sample(
    gallery: &Gallery,
    index: usize,
    close_up: bool,
    window: &Window,
    camera: &mut (Mut<Transform>, Mut<PanCamera>),
) {
    let Some(slot) = gallery.slots.get(index) else {
        return;
    };
    let viewport_width = window.width() - panel_span();
    let (centre, zoom) = if close_up {
        let half = gallery.samples[index].half;
        (
            *slot,
            2.0 * half * VIEW_MARGIN / window.height().min(viewport_width),
        )
    } else {
        // подписи стоят слева от окон
        let left = -GAP - CAPTION_WIDTH;
        // справа от окна — эталонный снимок того же размера
        let right = 4.0 * gallery.max_half() + GAP;
        let zoom = (right - left) * VIEW_MARGIN / viewport_width;
        // верх окна — у верха экрана: колонка читается сверху вниз
        let top = slot.y + gallery.samples[index].half + GAP;
        (
            Vec2::new((left + right) / 2.0, top - window.height() * zoom / 2.0),
            zoom,
        )
    };
    // свободная часть экрана лежит правее его центра на полполосы панели
    let centre = centre - Vec2::new(panel_span() / 2.0 * zoom, 0.0);
    camera.0.translation = centre.extend(0.0);
    camera.0.scale = Vec3::splat(zoom);
    camera.1.zoom_factor = zoom;
}

/// `↑`/`↓` — к соседнему примеру: колонка длинная, а панорамой её листать долго.
fn step_to_neighbour(
    keys: Res<ButtonInput<KeyCode>>,
    gallery: Res<Gallery>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&mut Transform, &mut PanCamera)>,
) {
    let step = match (
        keys.just_pressed(KeyCode::ArrowDown),
        keys.just_pressed(KeyCode::ArrowUp),
    ) {
        (true, false) => 1,
        (false, true) => -1,
        _ => return,
    };
    // текущий — тот, чьё окно ближе всего к верхней трети экрана
    let zoom = camera.0.scale.x;
    let eye = camera.0.translation.y + window.height() * zoom / 6.0;
    let Some(current) = (0..gallery.slots.len()).min_by(|a, b| {
        (gallery.slots[*a].y - eye)
            .abs()
            .total_cmp(&(gallery.slots[*b].y - eye).abs())
    }) else {
        return;
    };
    let next = (current as i32 + step).clamp(0, gallery.slots.len() as i32 - 1) as usize;
    frame_sample(&gallery, next, false, &window, &mut camera);
}

/// Пересборка витрины: убрать всё положенное, перечитать примеры города с
/// диска, разложить колонку, положить рамки окон. Сами примеры собирает
/// [`build_next`] — по одному на кадр.
// Параметров десять, и все настоящие: разовая пересборка трогает и мир, и
// камеру, и строку панели — то же исключение, что у `map::spawn::spawn_map`
#[allow(clippy::too_many_arguments)]
fn reload(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    city: Res<City>,
    mut gallery: ResMut<Gallery>,
    mut status: Single<&mut Text, With<StatusLine>>,
    placed: Query<Entity, With<Placed>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&mut Transform, &mut PanCamera)>,
) {
    for entity in &placed {
        commands.entity(entity).despawn();
    }
    *gallery = Gallery::default();

    match samples::load(*city) {
        Ok(samples) => gallery.samples = samples,
        Err(reason) => {
            status.0 = reason.clone();
            commands.spawn((
                Placed,
                Text2d::new(reason),
                caption_font(&assets, TITLE_FONT),
                TextColor(INK),
                Transform::from_xyz(0.0, 0.0, Z_CAPTION).with_scale(Vec3::splat(TEXT_SCALE)),
            ));
            let camera = &mut *camera;
            camera.0.translation = Vec3::ZERO;
            camera.0.scale = Vec3::splat(0.4);
            camera.1.zoom_factor = 0.4;
            return;
        }
    }

    // колонка: окна одно под другим, по центру x = 0
    let mut top = 0.0;
    gallery.slots = gallery
        .samples
        .iter()
        .map(|sample| {
            // левый край у всех окон общий, x = 0: подпись стоит вплотную к окну
            let slot = Vec2::new(sample.half, top - sample.half);
            // строка не ниже своей подписи: у малого окна текст длиннее окна
            top -= (2.0 * sample.half).max(CAPTION_HEIGHT) + GAP;
            slot
        })
        .collect();

    commands.spawn((
        Placed,
        Mesh2d(meshes.add(frames(&gallery).build())),
        MeshMaterial2d(materials.add(Color::WHITE)),
        Transform::from_xyz(0.0, 0.0, Z_FRAME),
        Name::new("roads_frames"),
    ));

    // эталон — снимок того же окна с Яндекс Карт, справа от рендера
    for (sample, slot) in gallery.samples.iter().zip(&gallery.slots) {
        let Some(image) = sample.reference.as_deref().and_then(load_reference) else {
            continue;
        };
        commands.spawn((
            Placed,
            Sprite {
                image: images.add(image),
                custom_size: Some(Vec2::splat(2.0 * sample.half)),
                ..default()
            },
            Transform::from_translation(reference_centre(sample, *slot).extend(Z_FRAME - 1.0)),
            Name::new("roads_reference"),
        ));
    }

    // стиль дорог камеру не трогает: сравнивать стыки надо на том же месте
    if city.is_changed() {
        let requested = std::env::var("ROADS_SAMPLE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|number| (1..=gallery.samples.len()).contains(number));
        frame_sample(
            &gallery,
            requested.map_or(0, |number| number - 1),
            requested.is_some(),
            &window,
            &mut camera,
        );
    }
}

/// Рамки окон. Сами слои за окно не выходят — их режет [`build_next`], — а без
/// рамки фактурная земля окна и плоский фон сходятся едва заметной ступенью,
/// которая читается как дефект.
fn frames(gallery: &Gallery) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let ink = INK_DIM.to_linear();
    let mut frame = |centre: Vec2, half: f32| {
        let (min, max) = (centre - half, centre + half);
        let w = FRAME_WIDTH;
        builder.push_rect(min - w, Vec2::new(max.x + w, min.y), ink);
        builder.push_rect(Vec2::new(min.x - w, max.y), max + w, ink);
        builder.push_rect(min - w, Vec2::new(min.x, max.y + w), ink);
        builder.push_rect(Vec2::new(max.x, min.y - w), max + w, ink);
    };
    for (sample, slot) in gallery.samples.iter().zip(&gallery.slots) {
        frame(*slot, sample.half);
        if sample.reference.is_some() {
            frame(reference_centre(sample, *slot), sample.half);
        }
    }
    builder
}

/// Центр эталонного снимка: справа от окна примера, того же размера — охват у
/// них один, так что узел стоит в обоих квадратах на одном месте.
fn reference_centre(sample: &Sample, slot: Vec2) -> Vec2 {
    slot + Vec2::new(2.0 * sample.half + GAP, 0.0)
}

/// Снимок Яндекс Карт с диска — спрайтом в метрах окна. Мимо `AssetServer`:
/// файл лежит рядом с вырезкой, вне `assets/`, куда сервер не ходит.
fn load_reference(path: &std::path::Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::RENDER_WORLD,
    )
    .inspect_err(|error| warn!("{}: {error}", path.display()))
    .ok()
}

/// Следующий пример очереди: вырезка OSM → игровой `parse` → игровые `mesh_*`
/// → игровые `spawn_*`. Порядок слоёв и их входы — те же, что у
/// `map::spawn::spawn_map` и цепочки `rebuild_*` за ним.
#[allow(clippy::too_many_arguments)]
/// Ширина полосы — в глобаль, которую читают разбор, краска и колея
/// асфальта: игра пишет её перед потоком загрузки, витрина — перед разбором
/// примеров, которые `reload` по этой же правке соберёт заново.
fn apply_lane_width(shape: Res<RoadShapeOnMap>) {
    set_lane_width(shape.0.lane_width());
}

#[allow(clippy::too_many_arguments)]
fn build_next(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: TreeMaterials,
    assets: Res<AssetServer>,
    city: Res<City>,
    road_style: Res<RoadStyle>,
    road_shape: Res<RoadShapeOnMap>,
    overlay: Res<NetworkOverlay>,
    mut gallery: ResMut<Gallery>,
    mut status: Single<&mut Text, With<StatusLine>>,
) {
    let index = gallery.built;
    let Some(sample) = gallery.samples.get(index) else {
        return;
    };
    let slot = gallery.slots[index];
    let started = std::time::Instant::now();

    let map = parse_response(&sample.osm, *city);
    let parsed = started.elapsed();

    // Всякий слой режется окном примера (в игровых координатах — до сдвига):
    // окна стоят в колонке встык, а слой рисует и за окном — хвост дороги, дом
    // на краю вырезки целиком, квад земли во всю карту. Необрезанное ложилось
    // в окно соседа: дом чужого перекрёстка посреди проспекта.
    let (window_min, window_max) = (sample.at - sample.half, sample.at + sample.half);
    let clip = |mut layers: Vec<LayerMesh>| {
        for layer in &mut layers {
            layer.builder.clip_to_rect(window_min, window_max);
        }
        layers
    };

    // раскладка стоянок — вход поверхностям: по ней рисуется разметка мест
    let layout = ParkingLayout::new(&map.parking, &map.roads);
    let (surfaces, _) = mesh_surfaces(&map, &layout);
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials.layers,
        clip(surfaces),
        SampleLayer,
    );

    let (road_layers, road_report) = mesh_roads(&map, *road_style, road_shape.0);
    let road_layers = clip(road_layers);
    let road_line = road_report.to_string();
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials.layers,
        (road_layers, road_report),
    );

    if overlay.visible {
        spawn_layers(
            &mut commands,
            &mut meshes,
            &materials.layers,
            clip(vec![qwe::map::mesh_network_overlay(&map)]),
            SampleLayer,
        );
    }

    let plan = BuildingPlan {
        mode: BuildingHeightMode::default(),
        bucket: BuildingZoomBucket::at(0),
        shadows: true,
    };
    let (mut buildings, building_report) = mesh_buildings(plan, &map.buildings, &map.roads);
    buildings.layers = clip(buildings.layers);
    buildings.shadows = clip(buildings.shadows);
    spawn_building_meshes(
        &mut commands,
        &mut meshes,
        &materials.layers,
        (buildings, building_report),
    );

    let (fences, _) = mesh_fences(FenceZoomBucket::at(0), &map.fences, &map.roads);
    let (rails, _) = mesh_rails(RailZoomBucket::at(0), &map.rails);
    let tree_rows = mesh_tree_row_band(&map.tree_rows, &TreeRowStyle::default());
    for layers in [fences, rails, tree_rows] {
        spawn_layers(
            &mut commands,
            &mut meshes,
            &materials.layers,
            clip(layers),
            SampleLayer,
        );
    }

    // порода кроны читается с поля хвои — оно считается по посаженным
    // деревьям, как в `trees::build_conifer_field`
    let tree_style = TreeStyle::default();
    let mut field = ConiferField::default();
    field.resample(
        map.trees.positions(),
        &ConiferNoiseStyle::default(),
        tree_style.noise_mix,
    );
    field.set_share(tree_style.conifer_share);
    let (mut trees, tree_report) =
        mesh_trees(&tree_style, &CrownParams::default(), &map.trees, &field);
    // крона — сущность, а не часть слоя: её не режут, а оставляют по центру.
    // Свес за окно — метры, до соседнего окна `GAP`
    trees
        .crowns
        .retain(|crown| crown.at.cmpge(window_min).all() && crown.at.cmple(window_max).all());
    trees.shadows = clip(trees.shadows);
    spawn_tree_meshes(
        &mut commands,
        &mut meshes,
        &mut materials,
        (trees, tree_report),
    );

    spawn_caption(
        &mut commands,
        &assets,
        index,
        sample,
        slot,
        &format!(
            "OSM: {} way, {} node, {} relation  →  MapData: {} дорог, {} дорожных узлов, {} площадей дорог, {} домов, {} стоянок, {} деревьев\n\
             разбор {:.0?}, сборка слоёв {:.0?}\n{road_line}",
            sample.elements[1],
            sample.elements[0],
            sample.elements[2],
            map.roads.len(),
            map.road_nodes.len(),
            map.road_areas.len(),
            map.buildings.len(),
            map.parking.len(),
            map.trees.len(),
            parsed,
            started.elapsed() - parsed,
        ),
    );

    let shift = slot - sample.at;
    gallery.pending_shift = Some(shift);
    gallery.built += 1;
    status.0 = format!("собрано {} из {}", gallery.built, gallery.samples.len());
}

/// Сдвиг только что заспавненных слоёв примера на его место в колонке. Ловит
/// всякий новый меш без метки [`Placed`] — поэтому своё (рамки) витрина кладёт
/// уже с меткой, а примеры собирает по одному на кадр.
fn place_new(
    mut commands: Commands,
    mut gallery: ResMut<Gallery>,
    mut fresh: Query<(Entity, &mut Transform), (Added<Mesh2d>, Without<Placed>)>,
) {
    let Some(shift) = gallery.pending_shift.take() else {
        return;
    };
    for (entity, mut transform) in &mut fresh {
        transform.translation += shift.extend(0.0);
        commands.entity(entity).insert(Placed);
    }
}

/// Подпись слева от окна: вид пересечения, полный адрес, игровые координаты,
/// на что смотреть — и три строки о том, что вышло из каждой стадии.
fn spawn_caption(
    commands: &mut Commands,
    assets: &AssetServer,
    index: usize,
    sample: &Sample,
    slot: Vec2,
    stages: &str,
) {
    let left = -GAP - CAPTION_WIDTH;
    let top = slot.y + sample.half;
    let bounds = TextBounds::new_horizontal(CAPTION_WIDTH / TEXT_SCALE);
    let mut line = |text: String, size: f32, color: Color, drop: f32| {
        commands.spawn((
            Placed,
            Text2d::new(text),
            caption_font(assets, size),
            TextColor(color),
            bounds,
            Anchor::TOP_LEFT,
            Transform::from_translation(Vec2::new(left, top - drop).extend(Z_CAPTION))
                .with_scale(Vec3::splat(TEXT_SCALE)),
        ));
    };
    line(
        format!("{}. {}", index + 1, sample.title),
        TITLE_FONT,
        INK,
        0.0,
    );
    line(
        format!(
            "{}\nигровые координаты: x {:.0}, y {:.0}  ·  окно {:.0} × {:.0} м\n\n{}",
            sample.address,
            sample.at.x,
            sample.at.y,
            2.0 * sample.half,
            2.0 * sample.half,
            sample.note,
        ),
        BODY_FONT,
        INK,
        14.0,
    );
    line(
        format!("{}\n{stages}", sample.source),
        BODY_FONT * 0.8,
        INK_DIM,
        78.0,
    );
}

/// Подпись шрифтом панелей игры: во встроенном шрифте bevy кириллицы нет.
fn caption_font(assets: &AssetServer, size: f32) -> TextFont {
    TextFont {
        font: assets.load::<Font>(fonts::REGULAR).into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}
