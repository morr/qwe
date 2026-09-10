//! Витрина стен: все облицовки, которыми игра одевает дома, разом на одном
//! экране — и панель, которой их вид можно крутить вживую.
//!
//! Стена — это две независимые вещи, и витрина разложена по ним на две сетки.
//!
//! **Чем облицована** — материал (`WallKind`), он же цвет, рисунок между
//! окнами и сами окна: сетка [`kind_cells`], пять блоков. Внутри блока —
//! **лестница этажей** (2, 4, 5, 9, 16), потому что рисунок стены зависит от
//! роста дома не меньше, чем от материала: балкона нет ниже
//! `BALCONY_STOREYS_MIN`, первый этаж у всех свой (вход, витрина, ворота), а
//! шестнадцатиэтажка от пятиэтажки отличается не высотой картинки, а тем, что
//! на ней помещается.
//!
//! **Кому что достаётся** — сетка [`use_cells`]: по строке на `BuildingUse`, в
//! строке два дома, малоэтажный и высокий, и облицовку им выбирает сама игра
//! ([`wall_of`]). Это единственное место, где видно правило целиком: назначение
//! даёт таблицу, рост — развилку внутри неё, и один и тот же тег `commercial`
//! на двух этажах даёт кирпич, а на девяти витраж.
//!
//! **Дом здесь — дом, а не одна стена.** Всё, что стоит на витрине, кладёт
//! [`push_house`] — тот самый вызов, которым `layers.rs` строит 2.5D-город.
//! Иначе главного и не увидеть: стена читается вместе с той, что рядом (одна
//! освещена, другая в тени), и вместе с крышей, которая её накрывает.
//!
//! Отличий от игры два, оба намеренные:
//!
//! - **материал и цвет выбирает витрина**, а не посев. В городе их решает
//!   `wall_look`: назначение и этажность дают таблицу, посев от первой вершины
//!   контура — слот в ней и цвет в палитре. Перебрать так все сочетания нельзя
//!   (витраж не выпадает частному дому), поэтому нижняя сетка собирает
//!   `WallLook` напрямую — тем же публичным конструктором, которым пользуется и
//!   `wall_look`. Верхняя сетка, наоборот, зовёт игровой выбор и ничего не
//!   подменяет;
//! - **этажность задана**, а не выведена из тега: в игре её даёт `height` или
//!   `heights.rs`, здесь она вход, иначе лестницу не построить.
//!
//! **Панель слева — то, что в игре приходит от самого дома**: длина (от неё
//! число панелей в стене, а значит и ширина окна в метрах), поворот длинной
//! оси, посев, двор; плюс два игровых ползунка — `RoofStyle::texture` и час
//! съёмки (`SunStyle`). Поворот тут стоит вторым по важности после материала:
//! ракурс сжимает стену, и окна на западной стене гаснут раньше, чем на южной.
//!
//! **Плашка внизу справа — масштаб**, и она печатает не только метры на
//! пиксель, но и **экранный размер этажа**: стена считается в ячейках, гашение
//! идёт по ним, и «окна пропали» от «фактура выключена» иначе не отличить.
//!
//! Пример не трогает конфиг игры: ни `PrefsPlugin`, ни `MapPlugin`, ни
//! `CameraPlugin`. Материал при этом игровой: `Material2dPlugin::<RoofMaterial>`
//! плюс те же `init_roof_material` / `retune_roof_material`, что поднимает
//! `MapPlugin`.
//!
//! ```text
//! cargo run --example wall_gallery
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `G` | подложка: земля карты → нейтральный серый → тёмный |
//! | `L` | подписи вкл/выкл |
//!
//! `WALL_GALLERY_SHOT=путь.png` — поднять окно, снять витрину и выйти. Нужно
//! затем, что у примера нет BRP: без этого проверить внешний вид из сессии
//! нечем.

mod constants;
mod panel;
mod params;
#[path = "../gallery_shot.rs"]
mod shot;

use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::sprite_render::Material2dPlugin;
use bevy::window::PrimaryWindow;
use qwe::camera::{hovering_ui, zoom_to_cursor};
use qwe::map::buildings::material::{
    RoofKind, RoofLook, RoofMaterial, RoofMaterialHandle, WallKind, WallLook, init_roof_material,
    retune_roof_material,
};
use qwe::map::buildings::{BuildingHeightMode, RoofShape, extrusion_lift, push_house, wall_of};
use qwe::map::osm::entrances::generate_entrances;
use qwe::map::osm::{AreaKind, BuildingUse, MapData, PolyArea};
use qwe::map::{GROUND_COLOR, MeshBuilder, RoofStyle, SunOnMap, SunStyle, apply_sun};
use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::panel::{
    spawn_panel, spawn_readout, sync_param_rows, sync_reset_button, update_readout,
};
use crate::params::Tuning;
use crate::shot::{ShotRequest, auto_shot, request_shot};

const WINDOW_WIDTH: f32 = 1600.0;
const WINDOW_HEIGHT: f32 = 900.0;

/// Лестница этажности внутри блока материала. Числа выбраны не поровну, а по
/// развилкам: 2 — малоэтажка, у неё балконов нет ни при каком материале;
/// 4 — ровно `BALCONY_STOREYS_MIN`, первый этаж, на котором они появляются;
/// 5 и 9 — типовая застройка; 16 — башня, на которой видно, что рисунок
/// повторяется этажом, а не растягивается.
const STOREY_LADDER: [u32; 5] = [2, 4, 5, 9, 16];

/// Этажи, на которых показан игровой выбор облицовки: малоэтажный дом и
/// высокий. Развилка `LOW_RISE_STOREYS` лежит между ними, и в этом весь смысл
/// пары — один тег даёт разные стены.
const USE_LADDER: [u32; 2] = [2, 9];

/// Высота этажа, м, — та же, по которой `layers.rs` считает их число. Здесь
/// она нужна в обратную сторону: из этажей получить высоту дома.
const STOREY_HEIGHT: f32 = 3.0;

/// Ширина дома в долях длины: длинная ось должна быть заметно длинной, иначе
/// не разглядеть, что окна идут вдоль неё рядом, а не решёткой.
const DEPTH_RATIO: f32 = 0.5;
/// Зазор между домами, м.
const BUILDING_GAP: f32 = 7.0;

/// Длина дома в сетке назначений, м. Короче, чем в сетке материалов: там
/// смотрят на рисунок, здесь — на то, какой он вообще выпал.
const USE_LENGTH: f32 = 16.0;

/// Шаг рядов, м. У сетки материалов ряд обязан вместить шестнадцатиэтажку с её
/// подъёмом (16 × 3 × 0.35 ≈ 17 м) плюс глубину дома и подписи; у сетки
/// назначений домов меньше и ряд плотнее.
const KIND_PITCH_Y: f32 = 52.0;
const USE_PITCH_Y: f32 = 30.0;
/// Зазор между сеткой материалов и сеткой назначений, м.
const GRID_GAP_X: f32 = 26.0;

/// Заголовок ряда над домами и подпись под домом, м от базовой линии.
const HEADER_RISE: f32 = 24.0;
const CAPTION_DROP: f32 = 5.0;

/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в метрах.
const TEXT_SCALE: f32 = 0.1;
const CAPTION_FONT: f32 = 22.0;
const HEADER_FONT: f32 = 30.0;
/// Поля вокруг сеток при стартовом зуме, доля их размера.
const VIEW_MARGIN: f32 = 1.10;

/// Шаг фазы между соседними домами витрины — иррациональная доля, чтобы фазы
/// не повторялись по кругу.
const PHASE_STEP: f32 = 0.147;

/// Крыша, которой накрыты все дома витрины. Битум и плоская форма — намеренно:
/// скат увёл бы половину стены из-под взгляда, а оборудование на кровле
/// перетягивает внимание на себя. Цвет один на всю витрину, чтобы стены
/// сравнивались между собой, а не с крышей над каждой.
const GALLERY_ROOF: RoofKind = RoofKind::Bitumen;

/// Нарисованная высота этажа, м: настоящие три метра, сжатые подъёмом.
///
/// Не копия `EXTRUDE_SCALE` числом — тот приватен, и копия разошлась бы с ним
/// в первую же правку. Берётся из самого `extrusion_lift`, по дому заведомо в
/// середине его зажима (`EXTRUDE_RANGE`, 2.5…30 м): десять этажей дают подъём
/// 10.5 м, ни в один край не упирающийся.
pub(crate) fn drawn_storey() -> f32 {
    const PROBE_STOREYS: f32 = 10.0;
    let probe = house(Vec2::ZERO, Vec2::splat(5.0), Rot2::IDENTITY, 0.0)
        .with_height(PROBE_STOREYS * STOREY_HEIGHT);
    extrusion_lift(&probe, BuildingHeightMode::Extrusion).y / PROBE_STOREYS
}

/// Подложка витрины. Земля карты — то, на чём дома стоят в игре; серый —
/// нейтральный фон, на котором честно сравниваются цвета палитр; тёмный
/// показывает, насколько светлы штукатурка и силикатный кирпич.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Ground {
    #[default]
    Map,
    Grey,
    Dark,
}

impl Ground {
    fn color(self) -> Color {
        match self {
            Self::Map => GROUND_COLOR,
            Self::Grey => Color::srgb(0.5, 0.5, 0.5),
            Self::Dark => Color::srgb(0.16, 0.16, 0.17),
        }
    }

    /// Чернила подписей: на светлой подложке тёмные, на тёмной светлые.
    fn ink(self) -> Color {
        match self {
            Self::Map | Self::Grey => Color::srgb(0.14, 0.16, 0.20),
            Self::Dark => Color::srgb(0.86, 0.87, 0.90),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Map => Self::Grey,
            Self::Grey => Self::Dark,
            Self::Dark => Self::Map,
        }
    }
}

/// Что видно на витрине помимо домов: подложка и подписи. Одним ресурсом, а не
/// двумя, потому что они связаны — чернила подписи выбирает подложка.
#[derive(Resource)]
struct View {
    ground: Ground,
    captions: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            ground: Ground::default(),
            captions: true,
        }
    }
}

impl View {
    fn visibility(&self) -> Visibility {
        if self.captions {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    }
}

/// Слой домов — один слитый меш на всю витрину, как `building_extruded` в игре.
#[derive(Component)]
struct WallLayer;

/// Подписи. Живут вместе с домами: под каждым написано, что на нём стоит, а
/// в сетке назначений это вообще результат сборки — какой материал выбрала
/// игра, до вызова `wall_of` неизвестно.
#[derive(Component)]
struct Caption;

/// Чем этот материал отличается на глаз — в заголовке ряда, рядом с именем.
fn note(kind: WallKind) -> &'static str {
    match kind {
        WallKind::Panel => "швы плит, окно на панель, столбцы балконов",
        WallKind::Brick => "ряды кладки, окно уже, лоджии вместо выступов",
        WallKind::Plaster => "гладкая, мелкие окна, балконов нет",
        WallKind::Shopfront => "лента остекления, витрина на первом этаже",
        WallKind::Shed => "рёбра профлиста, окно под карнизом, ворота внизу",
    }
}

fn use_label(building_use: BuildingUse) -> &'static str {
    match building_use {
        BuildingUse::House => "Частный дом",
        BuildingUse::Apartments => "Многоквартирный",
        BuildingUse::Commercial => "Торговля, офис",
        BuildingUse::Industrial => "Промзона, склад",
        BuildingUse::Garage => "Гараж, сарай",
        BuildingUse::Church => "Храм",
        BuildingUse::Public => "Общественное",
        BuildingUse::Other => "building=yes",
    }
}

/// Все назначения по порядку — исчерпывающий список, тот же, что у парсера.
const USES: [BuildingUse; 8] = [
    BuildingUse::House,
    BuildingUse::Apartments,
    BuildingUse::Other,
    BuildingUse::Commercial,
    BuildingUse::Public,
    BuildingUse::Industrial,
    BuildingUse::Garage,
    BuildingUse::Church,
];

/// Дом витрины: где стоит, какой величины, сколько этажей и чем облицован.
struct Cell {
    centre: Vec2,
    half: Vec2,
    storeys: u32,
    /// `None` — облицовку выбирает сама игра (сетка назначений).
    wall: Option<WallLook>,
    building_use: BuildingUse,
}

impl Cell {
    fn height(&self) -> f32 {
        self.storeys as f32 * STOREY_HEIGHT
    }
}

/// Шаг домов по x в сетке материалов — длина дома плюс зазор.
fn kind_pitch_x(tuning: &Tuning) -> f32 {
    tuning.length + BUILDING_GAP
}

/// Сетка материалов: ряд на облицовку, в ряду лестница этажей. Цвет берётся из
/// палитры материала по кругу — палитры разной длины, и так на каждой видно
/// хотя бы несколько её цветов.
fn kind_cells(tuning: &Tuning) -> Vec<Cell> {
    let half = Vec2::new(tuning.length, tuning.length * DEPTH_RATIO) / 2.0;
    let mut cells = Vec::new();
    for (row, kind) in WallKind::ALL.into_iter().enumerate() {
        let palette = kind.palette();
        for (slot, storeys) in STOREY_LADDER.into_iter().enumerate() {
            let centre = Vec2::new(
                slot as f32 * kind_pitch_x(tuning) + half.x,
                -(row as f32) * KIND_PITCH_Y + half.y,
            );
            cells.push(Cell {
                centre,
                half,
                storeys,
                wall: Some(WallLook::new(
                    kind,
                    palette[slot % palette.len()].to_srgba(),
                )),
                building_use: BuildingUse::Other,
            });
        }
    }
    cells
}

/// Сетка назначений: строка на `BuildingUse`, в строке малоэтажный дом и
/// высокий. Облицовку выбирает игра — витрина её не заказывает.
fn use_cells(tuning: &Tuning) -> Vec<Cell> {
    let half = Vec2::new(USE_LENGTH, USE_LENGTH * DEPTH_RATIO) / 2.0;
    let origin_x = STOREY_LADDER.len() as f32 * kind_pitch_x(tuning) + GRID_GAP_X;
    let mut cells = Vec::new();
    for (row, building_use) in USES.into_iter().enumerate() {
        for (slot, storeys) in USE_LADDER.into_iter().enumerate() {
            cells.push(Cell {
                centre: Vec2::new(
                    origin_x + slot as f32 * (USE_LENGTH + BUILDING_GAP) + half.x,
                    -(row as f32) * USE_PITCH_Y + half.y,
                ),
                half,
                storeys,
                wall: None,
                building_use,
            });
        }
    }
    cells
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe wall gallery — все облицовки стен".to_string(),
                        resolution: (WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32).into(),
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
        .add_plugins(PanCameraPlugin)
        // материал зданий — игровой, вместе с его шейдером и юниформом
        .add_plugins(Material2dPlugin::<RoofMaterial>::default())
        // киты панели игры — кнопки, ползунки, тема, а заодно и шрифт с
        // кириллицей: во встроенном `default_font` её нет
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<View>()
        .init_resource::<Tuning>()
        .init_resource::<RoofStyle>()
        .init_resource::<SunOnMap>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_panel,
                spawn_readout,
                // тот же старт, что у `MapPlugin`: солнце в глобаль, и только
                // потом хэндл материала — его юниформ `light` берётся из неё
                (apply_sun_from_tuning, apply_sun, init_roof_material).chain(),
                request_shot("WALL_GALLERY_SHOT"),
            ),
        )
        .add_systems(
            Update,
            (
                cycle_ground.run_if(input_just_pressed(KeyCode::KeyG)),
                toggle_captions.run_if(input_just_pressed(KeyCode::KeyL)),
                // колесо над панелью витрине панели и принадлежит: тот же
                // гейт, что у игры (`camera.rs`)
                zoom_to_cursor.run_if(not(hovering_ui)),
                update_readout,
                apply_sun_from_tuning,
                apply_sun,
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает ту
                // же сборку, что потом идёт на каждую правку ручки
                rebuild_walls.run_if(resource_changed::<Tuning>.or_else(resource_changed::<View>)),
                (apply_texture, sync_param_rows, sync_reset_button)
                    .run_if(resource_changed::<Tuning>),
                retune_roof_material
                    .run_if(resource_changed::<RoofStyle>.or_else(resource_changed::<SunOnMap>)),
                apply_ground.run_if(resource_changed::<View>),
                auto_shot.run_if(resource_exists::<ShotRequest>),
            )
                .chain(),
        )
        .run();
}

/// Прямоугольник всей витрины в мировых единицах — по нему ставится камера.
/// Считается по домам вместе с полями под заголовки и подписи; подъём крыши в
/// него не входит и не должен: он уводит верхний ряд вверх ровно настолько,
/// насколько заголовку и так отведено места.
fn grid_rect(tuning: &Tuning) -> Rect {
    let mut rect = Rect::from_corners(Vec2::ZERO, Vec2::ZERO);
    for cell in kind_cells(tuning).iter().chain(use_cells(tuning).iter()) {
        rect = rect.union(Rect::from_center_half_size(cell.centre, cell.half));
        let base = cell.centre.y - cell.half.y;
        rect = rect.union_point(Vec2::new(cell.centre.x, base - CAPTION_DROP - 2.0));
        rect = rect.union_point(Vec2::new(cell.centre.x, base + HEADER_RISE + 3.0));
    }
    rect
}

/// Полоса окна, занятая панелью: отступ от края экрана, сама панель и такой же
/// зазор справа от неё.
fn panel_span() -> f32 {
    PANEL_WIDTH_PX + 2.0 * UI_SCREEN_EDGE_PX_OFFSET
}

/// Экранная ширина, оставшаяся витрине от панели, — по живому окну, а не по
/// `WINDOW_WIDTH`: константа только **просит** размер у ОС, а выдаёт его ОС.
fn viewport_width(window: &Window) -> f32 {
    window.width() - panel_span()
}

fn spawn_camera(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    tuning: Res<Tuning>,
) {
    let rect = grid_rect(&tuning);
    let zoom = (rect.width() * VIEW_MARGIN / viewport_width(&window))
        .max(rect.height() * VIEW_MARGIN / window.height());
    // Свободная часть окна лежит правее центра экрана ровно на полширины
    // занятой панелью полосы — значит камера едет ВЛЕВО на столько же, и мир
    // на экране уходит вправо, из-под панели.
    let centre = rect.center() - Vec2::new(panel_span() / 2.0 * zoom, 0.0);
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(centre.extend(0.0)).with_scale(Vec3::splat(zoom)),
        Msaa::Off,
        PanCamera {
            zoom_factor: zoom,
            min_zoom: zoom / 60.0,
            max_zoom: zoom * 4.0,
            // колесо ведёт `camera::zoom_to_cursor`: линейный зум самого
            // PanCamera на крупном плане неуправляем
            zoom_speed: 0.0,
            key_zoom_in: None,
            key_zoom_out: None,
            // витрина плоская, поворот камеры тут только мешает
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            pan_speed: 40.0,
            ..default()
        },
    ));
}

/// Пересборка витрины под текущие ручки: деспавн прежнего слоя и сборка нового
/// тем же вызовом, которым дома строит игра.
///
/// Порядок укладки — painter's, как в `extrusion_builder`: дальние дома раньше
/// ближних. В сетках он совпадает с порядком обхода — ряды идут сверху вниз, а
/// внутри ряда дома разнесены зазором и не перекрываются вовсе, — поэтому
/// сортировки тут нет: сетка витрины не город, её раскладку она выбирает сама.
///
/// Подписи спавнятся здесь же, а не один раз: в сетке назначений под домом
/// написан **выбранный игрой** материал, а он известен только после `wall_of`.
fn rebuild_walls(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<AssetServer>,
    roof: Res<RoofMaterialHandle>,
    tuning: Res<Tuning>,
    view: Res<View>,
    existing: Query<Entity, Or<(With<WallLayer>, With<Caption>)>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let font: Handle<Font> = assets.load(fonts::REGULAR);
    let rotation = Rot2::degrees(tuning.axis_deg);
    let axis = rotation * Vec2::X;
    let mut builder = MeshBuilder::with_roof_coords();
    let mut index = 0usize;

    let kinds = kind_cells(&tuning);
    for cell in &kinds {
        let seed = (tuning.seed + index as f32 * PHASE_STEP).fract();
        index += 1;
        let wall = cell.wall.expect("в сетке материалов облицовка заказана");
        let drawn = push_cell(&mut builder, cell, &wall, rotation, axis, seed, &tuning);
        let base = cell.centre.y - cell.half.y;
        spawn_caption(
            &mut commands,
            &font,
            &view,
            format!("{} эт · {}", cell.storeys, wall.base.to_hex()),
            Vec2::new(cell.centre.x, base - CAPTION_DROP),
            Anchor::TOP_CENTER,
            CAPTION_FONT,
        );
        debug_assert_eq!(drawn, RoofShape::Flat, "витрина стен просит плоскую крышу");
    }
    // заголовок ряда — один раз на материал, у левого края первой клетки
    for (row, kind) in WallKind::ALL.into_iter().enumerate() {
        let Some(first) = kinds.get(row * STOREY_LADDER.len()) else {
            continue;
        };
        spawn_caption(
            &mut commands,
            &font,
            &view,
            format!("{} — {}", kind.label(), note(kind)),
            Vec2::new(
                first.centre.x - first.half.x,
                first.centre.y - first.half.y + HEADER_RISE,
            ),
            Anchor::CENTER_LEFT,
            HEADER_FONT,
        );
    }

    let uses = use_cells(&tuning);
    for cell in &uses {
        let seed = (tuning.seed + index as f32 * PHASE_STEP).fract();
        index += 1;
        // здесь витрина ничего не заказывает: что выбрала бы игра, то и стоит
        let house = house(cell.centre, cell.half, rotation, tuning.courtyard)
            .with_height(cell.height())
            .with_use(cell.building_use);
        let wall = wall_of(&house);
        push_cell(&mut builder, cell, &wall, rotation, axis, seed, &tuning);
        let base = cell.centre.y - cell.half.y;
        spawn_caption(
            &mut commands,
            &font,
            &view,
            format!("{} эт · {}", cell.storeys, wall.kind.label()),
            Vec2::new(cell.centre.x, base - CAPTION_DROP),
            Anchor::TOP_CENTER,
            CAPTION_FONT,
        );
    }
    for (row, building_use) in USES.into_iter().enumerate() {
        let Some(first) = uses.get(row * USE_LADDER.len()) else {
            continue;
        };
        spawn_caption(
            &mut commands,
            &font,
            &view,
            use_label(building_use).to_string(),
            Vec2::new(
                first.centre.x - first.half.x,
                first.centre.y - first.half.y + HEADER_RISE * 0.55,
            ),
            Anchor::CENTER_LEFT,
            CAPTION_FONT,
        );
    }

    commands.spawn((
        WallLayer,
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(roof.handle()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Name::new("wall_gallery"),
    ));
}

/// Один дом витрины в меш. Крыша заказана плоской, оборудования нет — витрина
/// про стены, и всё, что стоит на кровле, здесь только мешает.
fn push_cell(
    builder: &mut MeshBuilder,
    cell: &Cell,
    wall: &WallLook,
    rotation: Rot2,
    axis: Vec2,
    seed: f32,
    tuning: &Tuning,
) -> RoofShape {
    let area = house(cell.centre, cell.half, rotation, tuning.courtyard)
        .with_height(cell.height())
        .with_use(cell.building_use)
        .with_doors();
    let roof_color = GALLERY_ROOF.palette()[0].to_srgba();
    let look = RoofLook::new(GALLERY_ROOF, roof_color, axis, seed);
    push_house(
        builder,
        &area,
        &look,
        wall,
        roof_color,
        RoofShape::Flat,
        false,
    )
}

fn spawn_caption(
    commands: &mut Commands,
    font: &Handle<Font>,
    view: &View,
    text: String,
    at: Vec2,
    anchor: Anchor,
    size: f32,
) {
    commands.spawn((
        Caption,
        Text2d::new(text),
        label_font(font, size),
        TextColor(view.ground.ink()),
        anchor,
        view.visibility(),
        Transform::from_translation(at.extend(2.0)).with_scale(Vec3::splat(TEXT_SCALE)),
    ));
}

/// Наименьшая сторона двора и наименьшее поле от двора до края крыши, м: двор
/// уже метра не читается, а поле уже трёх метров не оставляет места стене
/// двора — она рисуется по дальней стороне дырки и требует ширины.
const MIN_COURTYARD_SIDE: f32 = 1.0;
const MIN_COURTYARD_MARGIN: f32 = 3.0;

/// Прямоугольный дом с необязательным двором, повёрнутый на угол длинной оси.
/// Внешнее кольцо — CCW, дырка — наоборот, как их отдаёт OSM после сборки
/// колец.
fn house(centre: Vec2, half: Vec2, rotation: Rot2, courtyard: f32) -> PolyArea {
    let ring = |half: Vec2| -> Vec<Vec2> {
        [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(half.x, half.y),
            Vec2::new(-half.x, half.y),
        ]
        .map(|corner| centre + rotation * corner)
        .to_vec()
    };
    let hole_half = half * courtyard;
    let holes = if courtyard > 0.0
        && hole_half.min_element() * 2.0 >= MIN_COURTYARD_SIDE
        && (half - hole_half).min_element() >= MIN_COURTYARD_MARGIN
    {
        let mut hole = ring(hole_half);
        hole.reverse();
        vec![hole]
    } else {
        Vec::new()
    };
    PolyArea {
        outer: ring(half),
        holes,
        kind: AreaKind::Building,
        building_use: BuildingUse::Other,
        height: None,
        entrances: Vec::new(),
    }
}

/// Мелкие строители дома — чтобы вызов читался в одну строку, а не собирал
/// структуру по полю на строчку в каждом из трёх мест.
trait HouseExt {
    fn with_height(self, height: f32) -> Self;
    fn with_use(self, building_use: BuildingUse) -> Self;
    fn with_doors(self) -> Self;
}

impl HouseExt for PolyArea {
    fn with_height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    fn with_use(mut self, building_use: BuildingUse) -> Self {
        self.building_use = building_use;
        self
    }

    /// Входы — **игровым генератором** (`osm::entrances`), тем же, что ставит
    /// двери городу: дверь на стене витрины обязана стоять по тем же правилам,
    /// иначе витрина врёт ровно про то, ради чего дверь и рисуют. Дорог здесь
    /// нет, поэтому все грани равны по оценке и вход достаётся первой из них —
    /// а на длинном корпусе к нему добавляется сквозная пара со двора.
    fn with_doors(self) -> Self {
        let mut map = MapData {
            buildings: vec![self],
            ..default()
        };
        generate_entrances(&mut map);
        map.buildings.pop().expect("дом вернулся из генератора")
    }
}

/// Ползунок Texture — в игровую `RoofStyle`, дальше его подхватывает
/// `retune_roof_material`.
fn apply_texture(tuning: Res<Tuning>, mut style: ResMut<RoofStyle>) {
    style.set_if_neq(RoofStyle {
        texture: tuning.texture,
    });
}

/// Подпись шрифтом панелей игры: во встроенном шрифте bevy кириллицы нет.
fn label_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn cycle_ground(mut view: ResMut<View>) {
    view.ground = view.ground.next();
}

fn toggle_captions(mut view: ResMut<View>) {
    view.captions = !view.captions;
}

/// Ручки солнца — в игровую `SunOnMap`, из которой `apply_sun` кладёт его в
/// глобаль. Витрина пишет сразу «солнце карты», минуя `SunStyle` и его
/// оседание: оседание существует ради пересборки города, а не витрины.
fn apply_sun_from_tuning(tuning: Res<Tuning>, mut sun: ResMut<SunOnMap>) {
    sun.set_if_neq(SunOnMap(SunStyle {
        azimuth: tuning.sun_azimuth,
        elevation: tuning.sun_elevation,
    }));
}

/// Подложка и подписи одной системой: чернила зависят от подложки, а видимость
/// — от тумблера, и обе живут на одних и тех же сущностях.
fn apply_ground(
    view: Res<View>,
    mut clear: ResMut<ClearColor>,
    mut captions: Query<(&mut Visibility, &mut TextColor), With<Caption>>,
) {
    clear.0 = view.ground.color();
    let visibility = view.visibility();
    for (mut value, mut color) in &mut captions {
        *value = visibility;
        color.0 = view.ground.ink();
    }
}
