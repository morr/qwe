//! Витрина кровель: все материалы, которыми игра кроет дома, разом на одном
//! экране — и панель, которой их вид можно крутить вживую.
//!
//! Чем крыша покрыта, задано перечислением `RoofKind` — шесть материалов
//! (`Bitumen`, `Gravel`, `Seam`, `Corrugated`, `Tile`, `Membrane`); седьмым
//! блоком стоит **храм**: тот же фальцевый металл, но по своей зелёной палитре
//! (`CHURCH_ROOF_COLORS`), единственной не по материалу. Это исчерпывающий
//! список: кровли, которой здесь нет, в игре не бывает.
//!
//! В каждом блоке — **по дому на каждый цвет палитры материала**, от крупного
//! корпуса к частной коробке (30 → 8 м). Два разреза сразу: цвета материала
//! видны рядом (у плоских кровель корпусов палитры узкие по светлоте и
//! широкие по тону — на снимке соседние панельные дома отличаются оттенком, а
//! не устраивают конфетти; у черепицы частного сектора наоборот, яркие и
//! разные — красная, зелёная, синяя через забор друг от друга), а
//! размер показывает главное про фактуру — она задана **в метрах** и не
//! масштабируется вместе с домом: на 30-метровом корпусе видны и швы, и
//! заплаты, на восьмиметровой коробке от них остаётся пара полос.
//!
//! **Геометрия здесь та же, что в игре, а не её копия.** Дом кладёт
//! [`push_flat_roof`] — ровно тот вызов, которым `layers.rs` рисует всякую
//! плоскую крышу города, и в плоских режимах, и в 2.5D. Отсюда и парапет: его
//! ставит не витрина, а правило материала (`RoofKind::has_parapet`) — мягкая
//! кровля (битум, гравий, мембрана) получает кайму 0.7 м, черепица и профлист
//! обходятся свесом. В подписи блока «· парапет» печатается по тому же
//! правилу, а не руками.
//!
//! Отличий от игры два, оба намеренные:
//!
//! - **материал и цвет выбирает витрина, а не посев.** В городе их решает
//!   `roof_look`: назначение здания даёт таблицу материалов, посев от первой
//!   вершины контура — слот в ней и цвет в палитре. Перебрать так все
//!   сочетания нельзя (мембраны не бывает на частном доме), поэтому витрина
//!   собирает `RoofLook` напрямую — тем же публичным конструктором, которым
//!   пользуется и `roof_look`;
//! - **цвет берётся из палитры дословно**, без игрового разброса ±3 % по
//!   посеву: под каждым домом написан его hex, и он обязан совпадать с
//!   константой в `material.rs`.
//!
//! Форму крыши витрина не показывает: скаты, конёк и фронтон решает не
//! материал, а `roofs.rs` — по назначению и по тому, насколько контур
//! заполняет свой прямоугольник.
//!
//! **Панель слева — то, что в игре приходит от самого дома**: поворот длинной
//! оси (по ней повёрнута рамка кровли — швы ковра идут вдоль конька, рёбра
//! фальца по скату, и на повороте это видно), посев фазы, двор; плюс
//! единственный игровой ползунок кровель, `RoofStyle::texture`. Дефолт каждой
//! равен игровому, поэтому отклонение читается как «на столько мы от игры
//! отошли».
//!
//! **Под ручками — константы фактуры**, прочитанные из самого `roof.wgsl`
//! (`constants.rs`): шаг сетки заплат, размер латки, доли клеток у новой и у
//! старой кровли. Ползунка на них нет и не будет — они в шейдере, — но
//! подобрать число, глядя на картинку, нельзя, не видя его текущего значения
//! рядом с ней.
//!
//! **Плашка внизу справа — масштаб.** Все октавы фактуры гаснут по
//! `visible(длина волны, px)` из `roof.wgsl`: короче полутора пикселей —
//! ничего, длиннее четырёх — целиком. На общем плане от кровли остаётся чистый
//! цвет, и без числа на экране это не отличить от выключенной фактуры.
//!
//! Пример не трогает конфиг игры: ни `PrefsPlugin`, ни `MapPlugin`, ни
//! `CameraPlugin` — читать и писать `settings.toml` тут нечему. Материал
//! кровель при этом игровой: `Material2dPlugin::<RoofMaterial>` плюс те же
//! `init_roof_material` / `retune_roof_material`, что поднимает `MapPlugin`.
//! Колесо крутит игровая `camera::zoom_to_cursor` под своим гейтом
//! `not(hovering_ui)`: из модуля взяты две функции, плагин с его настройками —
//! нет.
//!
//! ```text
//! cargo run --example roof_gallery
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `G` | подложка: земля карты → нейтральный серый → тёмный |
//! | `L` | подписи вкл/выкл |

mod constants;
mod panel;
mod params;

use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::sprite_render::Material2dPlugin;
use bevy::window::PrimaryWindow;
use qwe::camera::{hovering_ui, zoom_to_cursor};
use qwe::map::buildings::material::{
    CHURCH_ROOF_COLORS, RoofKind, RoofLook, RoofMaterial, RoofMaterialHandle, init_roof_material,
    retune_roof_material,
};
use qwe::map::buildings::push_flat_roof;
use qwe::map::{GROUND_COLOR, MeshBuilder, RoofStyle};
use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::panel::{
    spawn_panel, spawn_readout, sync_param_rows, sync_reset_button, update_readout,
};
use crate::params::Tuning;

const WINDOW_WIDTH: f32 = 1500.0;
const WINDOW_HEIGHT: f32 = 860.0;

/// Крупный корпус и частная коробка, м: между ними размеры блока распределены
/// поровну, по дому на цвет палитры. Верхняя граница — типовой панельный дом,
/// нижняя — гараж или сарай; на них фактура читается совсем по-разному, и
/// в этом весь смысл ряда.
const BIG_LENGTH: f32 = 30.0;
const SMALL_LENGTH: f32 = 8.0;
/// Ширина дома в долях длины: длинная ось должна быть заметно длинной, иначе
/// поворот рамки кровли не на чем разглядеть.
const DEPTH_RATIO: f32 = 0.62;
/// Зазор между домами блока, м.
const BUILDING_GAP: f32 = 5.0;

/// Шаг сетки блоков, м. По x он не константа, а самый широкий блок плюс это
/// поле ([`block_pitch_x`]): палитры разной длины (семь цветов у черепицы,
/// три у мембраны), и зашитое число ломалось бы на каждом новом цвете. По y —
/// дом, заголовок над ним и подписи под ним.
const BLOCK_MARGIN_X: f32 = 15.0;
const BLOCK_PITCH_Y: f32 = 46.0;
/// Блоков в ряду: семь материалов ложатся в 2 × 4, и такая сетка ближе всего
/// по пропорциям к окну.
const BLOCK_COLUMNS: usize = 2;

/// Заголовок блока над домами и подпись под домом, м от базовой линии.
const HEADER_RISE: f32 = 24.0;
const CAPTION_DROP: f32 = 5.0;

/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в метрах. При 0.1 подпись выходит ~2 м высотой, то есть примерно в четверть
/// самого мелкого дома.
const TEXT_SCALE: f32 = 0.1;
const CAPTION_FONT: f32 = 24.0;
const HEADER_FONT: f32 = 32.0;
/// Поля вокруг сетки при стартовом зуме, доля её размера.
const VIEW_MARGIN: f32 = 1.12;

/// Подложка витрины. Земля карты — то, на чём крыши стоят в игре; серый —
/// нейтральный фон, на котором честно сравниваются цвета палитр; тёмный
/// показывает, насколько светлы мембрана и гравий.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
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

/// Показывать ли подписи. Отдельным ресурсом, а не состоянием сущностей:
/// подложка перекрашивает их же, и держать оба признака в одном месте проще.
#[derive(Resource)]
struct Show {
    captions: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self { captions: true }
    }
}

/// Слой кровель — один слитый меш на всю витрину, как `building_roofs` в игре.
#[derive(Component)]
struct RoofLayer;

/// Подписи блоков и домов. Живут отдельно от кровель: от ручек они не зависят.
#[derive(Component)]
struct Caption;

/// Блок витрины: один материал и его палитра.
struct Block {
    kind: RoofKind,
    /// Заголовок блока — у материала его имя, у храма имя назначения: палитра
    /// там выбрана не материалом.
    title: &'static str,
    /// Чем этот материал отличается на глаз — в том же заголовке. Пометка про
    /// парапет к подписи не относится: её печатает правило материала.
    note: &'static str,
    palette: &'static [Color],
}

/// Все блоки по порядку: шесть материалов и храм.
fn blocks() -> Vec<Block> {
    let mut blocks: Vec<Block> = RoofKind::ALL
        .into_iter()
        .map(|kind| Block {
            kind,
            title: kind.label(),
            note: match kind {
                RoofKind::Bitumen => "рулонный ковёр, заплаты по возрасту, лужи",
                RoofKind::Gravel => "засыпка по битуму",
                RoofKind::Seam => "металл, рёбра по скату",
                RoofKind::Corrugated => "волна 30 см по скату",
                RoofKind::Tile => "ряды вдоль конька",
                RoofKind::Membrane => "ПВХ, полотнища 2 м",
            },
            palette: kind.palette(),
        })
        .collect();
    blocks.push(Block {
        kind: RoofKind::Seam,
        title: "Храм",
        note: "тот же фальц, зелёная палитра",
        palette: &CHURCH_ROOF_COLORS,
    });
    blocks
}

/// Дом витрины: где стоит, какой величины и какого цвета.
struct Cell {
    centre: Vec2,
    half: Vec2,
    color: Srgba,
}

/// Длина дома витрины по его номеру в блоке из `count`: ровным шагом от
/// крупного корпуса к мелкой коробке.
fn house_length(slot: usize, count: usize) -> f32 {
    let t = if count > 1 {
        slot as f32 / (count - 1) as f32
    } else {
        0.0
    };
    BIG_LENGTH + (SMALL_LENGTH - BIG_LENGTH) * t
}

/// Ширина блока из `count` домов вместе с зазорами между ними.
fn block_width(count: usize) -> f32 {
    (0..count)
        .map(|slot| house_length(slot, count))
        .sum::<f32>()
        + BUILDING_GAP * count.saturating_sub(1) as f32
}

/// Шаг сетки по x: самый широкий блок витрины плюс поле.
fn block_pitch_x() -> f32 {
    blocks()
        .iter()
        .map(|block| block_width(block.palette.len()))
        .fold(0.0, f32::max)
        + BLOCK_MARGIN_X
}

/// Дома одного блока: по дому на цвет палитры, длина ровным шагом от крупного
/// корпуса к мелкой коробке. Стоят в ряд по общей базовой линии — так размеры
/// сравниваются глазом, а не по памяти.
fn cells(index: usize, block: &Block) -> Vec<Cell> {
    let origin = Vec2::new(
        (index % BLOCK_COLUMNS) as f32 * block_pitch_x(),
        -((index / BLOCK_COLUMNS) as f32) * BLOCK_PITCH_Y,
    );
    let count = block.palette.len();
    let mut cursor = 0.0;
    let mut cells = Vec::with_capacity(count);
    for (slot, color) in block.palette.iter().enumerate() {
        let length = house_length(slot, count);
        let half = Vec2::new(length, length * DEPTH_RATIO) / 2.0;
        cells.push(Cell {
            centre: origin + Vec2::new(cursor + half.x, half.y),
            half,
            color: color.to_srgba(),
        });
        cursor += length + BUILDING_GAP;
    }
    cells
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe roof gallery — все типы кровель".to_string(),
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
        // материал кровель — игровой, вместе с его шейдером и юниформом
        .add_plugins(Material2dPlugin::<RoofMaterial>::default())
        // киты панели игры — кнопки, ползунки, тема. Заодно и шрифт: во
        // встроенном `default_font` кириллицы нет, и подписи выходят
        // квадратиками, а feathers несёт в себе Fira Sans, на котором написаны
        // панели игры
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<Ground>()
        .init_resource::<Show>()
        .init_resource::<Tuning>()
        .init_resource::<RoofStyle>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_labels,
                spawn_panel,
                spawn_readout,
                // тот же старт, что у `MapPlugin`: хэндл материала с силой
                // фактуры из ресурса
                init_roof_material,
            ),
        )
        .add_systems(
            Update,
            (
                cycle_ground.run_if(input_just_pressed(KeyCode::KeyG)),
                toggle_captions.run_if(input_just_pressed(KeyCode::KeyL)),
                // колесо над панелью витрине панели и принадлежит: тот же
                // гейт, что у игры (`camera.rs`), — ввод, адресованный UI, в
                // мир не идёт
                zoom_to_cursor.run_if(not(hovering_ui)),
                update_readout,
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает
                // ту же сборку, что потом идёт на каждую правку ручки
                rebuild_roofs.run_if(resource_changed::<Tuning>),
                (apply_texture, sync_param_rows, sync_reset_button)
                    .run_if(resource_changed::<Tuning>),
                // юниформ материала, а не пересборка мешей — как в игре
                retune_roof_material.run_if(resource_changed::<RoofStyle>),
                apply_ground.run_if(resource_changed::<Ground>.or_else(resource_changed::<Show>)),
            )
                .chain(),
        )
        .run();
}

/// Прямоугольник сетки в мировых единицах, вместе с полями под заголовки и
/// подписи, — по нему ставится камера.
fn grid_rect() -> Rect {
    let mut rect = Rect::from_corners(Vec2::ZERO, Vec2::ZERO);
    for (index, block) in blocks().iter().enumerate() {
        for cell in cells(index, block) {
            rect = rect.union(Rect::from_center_half_size(cell.centre, cell.half));
            let base = cell.centre.y - cell.half.y;
            rect = rect.union_point(Vec2::new(cell.centre.x, base - CAPTION_DROP - 2.0));
            rect = rect.union_point(Vec2::new(cell.centre.x, base + HEADER_RISE + 3.0));
        }
    }
    rect
}

/// Полоса окна, занятая панелью: отступ от края экрана, сама панель и такой
/// же зазор справа от неё.
fn panel_span() -> f32 {
    PANEL_WIDTH_PX + 2.0 * UI_SCREEN_EDGE_PX_OFFSET
}

/// Экранная ширина, оставшаяся витрине от панели, — по живому окну, а не по
/// `WINDOW_WIDTH`: константа только **просит** размер у ОС, а выдаёт его ОС.
fn viewport_width(window: &Window) -> f32 {
    window.width() - panel_span()
}

fn spawn_camera(mut commands: Commands, window: Single<&Window, With<PrimaryWindow>>) {
    let rect = grid_rect();
    // масштаб = мировых метров на логический пиксель (`ScalingMode::WindowSize`
    // меряет проекцию по логическому вьюпорту, в тех же единицах, что и
    // `PANEL_WIDTH_PX`), по свободной от панели части окна: иначе левый
    // столбец витрины стоит под панелью. Сетка тут почти квадратная, поэтому
    // кадрируется по обеим сторонам, а не только по ширине.
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
            min_zoom: zoom / 40.0,
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

/// Заголовки блоков и hex под каждым домом. От ручек не зависят, поэтому
/// спавнятся один раз и переживают пересборку кровель.
fn spawn_labels(mut commands: Commands, assets: Res<AssetServer>, ground: Res<Ground>) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    for (index, block) in blocks().iter().enumerate() {
        let cells = cells(index, block);
        let Some(first) = cells.first() else {
            continue;
        };
        let base = first.centre.y - first.half.y;
        // «· парапет» печатает правило материала, а не витрина: кайму по
        // контуру кладёт оно же
        let parapet = if block.kind.has_parapet() {
            " · парапет"
        } else {
            ""
        };
        commands.spawn((
            Caption,
            Text2d::new(format!("{} — {}{parapet}", block.title, block.note)),
            label_font(&font, HEADER_FONT),
            TextColor(ground.ink()),
            Anchor::CENTER_LEFT,
            Transform::from_xyz(first.centre.x - first.half.x, base + HEADER_RISE, 2.0)
                .with_scale(Vec3::splat(TEXT_SCALE)),
        ));
        for cell in &cells {
            commands.spawn((
                Caption,
                Text2d::new(cell.color.to_hex()),
                label_font(&font, CAPTION_FONT),
                TextColor(ground.ink()),
                Transform::from_xyz(cell.centre.x, base - CAPTION_DROP, 2.0)
                    .with_scale(Vec3::splat(TEXT_SCALE)),
            ));
        }
    }
}

/// Пересборка витрины под текущие ручки: деспавн прежнего слоя и сборка
/// нового тем же вызовом, которым кровли строит игра.
fn rebuild_roofs(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    roof: Res<RoofMaterialHandle>,
    tuning: Res<Tuning>,
    existing: Query<Entity, With<RoofLayer>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let rotation = Rot2::degrees(tuning.axis_deg);
    let axis = rotation * Vec2::X;
    let mut builder = MeshBuilder::with_roof_coords();
    let mut house = 0;
    for (index, block) in blocks().iter().enumerate() {
        for cell in cells(index, block) {
            // посев у каждого дома свой, ручка крутит их все разом: в игре он
            // берётся из первой вершины контура затем, чтобы швы соседних домов
            // не выстроились в одну линию через квартал, и затем, чтобы у домов
            // был разный возраст кровли (`roof.wgsl::roof_age`) — отсюда
            // разное число заплат на битумных домах одного блока
            let seed = (tuning.seed + house as f32 * PHASE_STEP).fract();
            let look = RoofLook::new(block.kind, cell.color, axis, seed);
            let (outer, holes) = footprint(&cell, rotation, tuning.courtyard);
            push_flat_roof(&mut builder, &look, &outer, &holes, cell.color);
            house += 1;
        }
    }

    commands.spawn((
        RoofLayer,
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(roof.handle()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Name::new("roof_gallery"),
    ));
}

/// Шаг фазы между соседними домами витрины — иррациональная доля, чтобы фазы
/// не повторялись по кругу палитры.
const PHASE_STEP: f32 = 0.147;

/// Наименьшая сторона двора и наименьшее поле от двора до края крыши, м:
/// двор уже метра не читается, а поле уже трёх метров съедает парапет
/// (0.7 м с каждой стороны) вместе с заливкой между ним и внешней каймой.
const MIN_COURTYARD_SIDE: f32 = 1.0;
const MIN_COURTYARD_MARGIN: f32 = 3.0;

/// Контур дома и его двор, повёрнутые на угол длинной оси. Внешнее кольцо —
/// CCW, дырка — наоборот, как их отдаёт OSM после сборки колец.
fn footprint(cell: &Cell, rotation: Rot2, courtyard: f32) -> (Vec<Vec2>, Vec<Vec<Vec2>>) {
    let ring = |half: Vec2| -> Vec<Vec2> {
        [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(half.x, half.y),
            Vec2::new(-half.x, half.y),
        ]
        .map(|corner| cell.centre + rotation * corner)
        .to_vec()
    };
    let outer = ring(cell.half);
    let hole_half = cell.half * courtyard;
    let holes = if courtyard > 0.0
        && hole_half.min_element() * 2.0 >= MIN_COURTYARD_SIDE
        && (cell.half - hole_half).min_element() >= MIN_COURTYARD_MARGIN
    {
        let mut hole = ring(hole_half);
        hole.reverse();
        vec![hole]
    } else {
        Vec::new()
    };
    (outer, holes)
}

/// Ползунок Texture — в игровой `RoofStyle`, дальше его подхватывает
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

fn cycle_ground(mut ground: ResMut<Ground>) {
    *ground = ground.next();
}

fn toggle_captions(mut show: ResMut<Show>) {
    show.captions = !show.captions;
}

/// Подложка и подписи одной системой: чернила зависят от подложки, а видимость
/// — от тумблера, и обе живут на одних и тех же сущностях.
fn apply_ground(
    ground: Res<Ground>,
    show: Res<Show>,
    mut clear: ResMut<ClearColor>,
    mut captions: Query<(&mut Visibility, &mut TextColor), With<Caption>>,
) {
    clear.0 = ground.color();
    let visibility = if show.captions {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut value, mut color) in &mut captions {
        *value = visibility;
        color.0 = ground.ink();
    }
}
