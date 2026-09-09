//! Витрина кровель: все материалы и все формы, которыми игра кроет дома,
//! разом на одном экране — и панель, которой их вид можно крутить вживую.
//!
//! Кровля — это две независимые вещи, и витрина разложена по ним на две
//! сетки. **Чем крыта** — материал (`RoofKind`), он же цвет и фактура: нижняя
//! сетка, семь блоков. **Какой формы** — плоская, двускатная или вальмовая
//! (`RoofShape`), и решает её не материал, а `roofs.rs`, по контуру и посеву:
//! верхняя сетка, [`shapes`], пять контуров под всеми тремя формами и под
//! выбором самой игры.
//!
//! **Дом здесь — дом, а не одна крыша.** Всё, что стоит на витрине, кладёт
//! [`push_house`] — тот самый вызов, которым `layers.rs` строит 2.5D-город:
//! видимые стены, подъём крыши над ними, скаты, фронтоны и труба на коньке.
//! Иначе главного и не увидеть: подъём конька, отсутствие фронтонов у вальмы
//! и разная высота силуэта у двускатного и вальмового дома одного размера на
//! голой крыше не читаются вовсе.
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
//! **Геометрия здесь та же, что в игре, а не её копия.** Числа под контурами
//! верхней сетки — заполнение описанного прямоугольника, вылет ската вальмы,
//! подъём конька — приходят из `roofs::shape_facts`, то есть ровно те, по
//! которым игра и принимает решение; форму, стены и скаты кладёт
//! [`push_house`].
//!
//! Отличий от игры три, все намеренные:
//!
//! - **материал и цвет выбирает витрина, а не посев.** В городе их решает
//!   `roof_look`: назначение здания даёт таблицу материалов, посев от первой
//!   вершины контура — слот в ней и цвет в палитре. Перебрать так все
//!   сочетания нельзя (мембраны не бывает на частном доме), поэтому витрина
//!   собирает `RoofLook` напрямую — тем же публичным конструктором, которым
//!   пользуется и `roof_look`;
//! - **цвет берётся из палитры дословно**, без игрового разброса ±3 % по
//!   посеву: под каждым домом написан его hex, и он обязан совпадать с
//!   константой в `material.rs`;
//! - **форму витрина заказывает**, а не выводит из дома: `RoofShape` — вход,
//!   которого у города нет, он-то как раз и нужен, чтобы поставить рядом один
//!   контур под тремя крышами. Отказ при этом не подменяется: если игра на
//!   этом контуре двускатную не строит, дом остаётся плоским и в подписи так
//!   и написано.
//!
//! **Панель слева — то, что в игре приходит от самого дома**: высота стен,
//! поворот длинной оси (по ней повёрнута рамка кровли — швы ковра идут вдоль
//! конька, рёбра фальца по скату, и на повороте это видно), посев фазы, двор;
//! плюс два игровых ползунка — `RoofStyle::texture` и **час съёмки**
//! (`SunStyle`: азимут и высота солнца). Дефолт каждой равен игровому, поэтому
//! отклонение читается как «на столько мы от игры отошли».
//!
//! **Солнце здесь то же самое, что в городе** — та же процессная глобаль
//! (`map/sun.rs`), которую читают и шейдер кровли (`RoofParams::light`:
//! азимут решает, какое ребро фальца блестит, а какое в тени), и тени
//! оборудования. Высота солнца — единственная ручка витрины, меняющая
//! **длину**: на 15° тень трубы втрое длиннее её самой, и на этом видно,
//! обрезана ли она краем кровли (`clutter::shadow_reach`). Игрового оседания
//! ползунка (`settle_sun`) здесь нет: оно про стомиллисекундную пересборку
//! города, а витрина пересобирается мгновенно.
//!
//! **Под ручками — константы**, прочитанные из самих исходников
//! (`constants.rs`): фактуры — из `roof.wgsl` (шаг сетки заплат, размер латки,
//! доли клеток у новой и у старой кровли), формы — из `roofs.rs` (порог
//! заполнения прямоугольника, доля вальмовых, вылет ската и его зажим, уклон).
//! Ползунка на них нет и не будет — они в коде, — но подобрать число, глядя на
//! картинку, нельзя, не видя его текущего значения рядом с ней.
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
//!
//! `ROOF_GALLERY_SHOT=путь.png` — поднять окно, снять витрину и выйти. Нужно
//! затем, что у примера нет BRP: без этого проверить внешний вид из сессии
//! нечем. Окно поднимается само — перекрытое чужим окном macOS снимает
//! чёрным.

mod constants;
mod panel;
mod params;
mod shapes;
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
    CHURCH_ROOF_COLORS, RoofKind, RoofLook, RoofMaterial, RoofMaterialHandle, init_roof_material,
    retune_roof_material,
};
use qwe::map::buildings::{RoofShape, push_house};
use qwe::map::osm::{AreaKind, BuildingUse, PolyArea};
use qwe::map::{GROUND_COLOR, MeshBuilder, RoofStyle, SunOnMap, SunStyle, apply_sun};
use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::panel::{
    spawn_panel, spawn_readout, sync_param_rows, sync_reset_button, update_readout,
};
use crate::params::Tuning;
use crate::shot::{ShotRequest, auto_shot, request_shot};

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

/// Зазор между подписями нижнего ряда форм и заголовками блоков материалов,
/// м. Небольшой: место под сами подписи уже отмерено `shapes::BOTTOM_REACH`,
/// и складывать два запаса значит развести сетки на полэкрана.
const SHAPES_GAP: f32 = 12.0;
/// Чем крыты дома сетки форм. Черепица — кровля частного сектора, а сетка
/// форм именно про него: скатную крышу игра ставит только частному дому.
/// Цвет один на всю сетку и берётся из палитры материала, чтобы рядом
/// сравнивалась форма, а не оттенок.
const SHAPE_ROOF: RoofKind = RoofKind::Tile;

/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в метрах. При 0.1 подпись выходит ~2 м высотой, то есть примерно в четверть
/// самого мелкого дома.
const TEXT_SCALE: f32 = 0.1;
const CAPTION_FONT: f32 = 24.0;
const HEADER_FONT: f32 = 32.0;
/// Подписи сетки форм: в них по две-три строки, и общий `CAPTION_FONT` в шаг
/// клетки по высоте не помещается.
const SHAPE_CAPTION_FONT: f32 = 22.0;
/// Поля вокруг сетки при стартовом зуме, доля её размера.
const VIEW_MARGIN: f32 = 1.12;

/// Подложка витрины. Земля карты — то, на чём крыши стоят в игре; серый —
/// нейтральный фон, на котором честно сравниваются цвета палитр; тёмный
/// показывает, насколько светлы мембрана и гравий.
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

/// Что видно на витрине помимо домов: подложка и подписи. Одним ресурсом, а
/// не двумя, потому что они связаны — чернила подписи выбирает подложка, обе
/// правки садятся на одни и те же сущности, и системе пересборки нужны сразу
/// обе.
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

/// Слой кровель — один слитый меш на всю витрину, как `building_roofs` в игре.
#[derive(Component)]
struct RoofLayer;

/// Подписи блоков и домов. Живут отдельно от кровель: от ручек они не зависят.
#[derive(Component)]
struct Caption;

/// Подписи сетки форм. В отличие от остальных, зависят от того, что легло в
/// меш: заказанную форму игра ставит не всегда, и написать «отказ» можно
/// только после сборки. Поэтому они деспавнятся и спавнятся вместе с кровлями.
#[derive(Component)]
struct ShapeCaption;

/// Блок витрины: один материал и его палитра.
struct Block {
    kind: RoofKind,
    /// Заголовок блока — у материала его имя, у храма имя назначения: палитра
    /// там выбрана не материалом.
    title: &'static str,
    /// Чем этот материал отличается на глаз — в том же заголовке.
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
        .init_resource::<View>()
        .init_resource::<Tuning>()
        .init_resource::<RoofStyle>()
        .init_resource::<SunOnMap>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_labels,
                spawn_panel,
                spawn_readout,
                // тот же старт, что у `MapPlugin`: солнце в глобаль, и только
                // потом хэндл материала — его юниформ `light` берётся из неё
                // один раз на всё приложение
                (apply_sun_from_tuning, apply_sun, init_roof_material).chain(),
                request_shot("ROOF_GALLERY_SHOT"),
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
                // солнце — в ту же процессную глобаль, из которой его читает
                // игра, и **до** сборки в этом же кадре. В игре оно едет через
                // `PreUpdate` и оседание ползунка, здесь источник истины один
                // (`Tuning`) и ждать нечего: пересборка витрины дешевле города
                apply_sun_from_tuning,
                apply_sun,
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает
                // ту же сборку, что потом идёт на каждую правку ручки
                rebuild_roofs.run_if(resource_changed::<Tuning>),
                (apply_texture, sync_param_rows, sync_reset_button)
                    .run_if(resource_changed::<Tuning>),
                // юниформ материала, а не пересборка мешей — как в игре. Солнце
                // сюда тоже приходит: `light` в `RoofParams` это азимут
                retune_roof_material
                    .run_if(resource_changed::<RoofStyle>.or_else(resource_changed::<SunOnMap>)),
                apply_ground.run_if(resource_changed::<View>),
                auto_shot.run_if(resource_exists::<ShotRequest>),
            )
                .chain(),
        )
        .run();
}

/// Прямоугольник сетки материалов вместе с полями под заголовки и подписи.
fn material_rect() -> Rect {
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

/// Центр левой верхней клетки сетки форм: над блоками материалов, по их
/// середине, с местом слева под подписи рядов. Нижний ряд поднят ещё на
/// `BOTTOM_REACH` — на столько уходят вниз его подписи, иначе они легли бы на
/// заголовки блоков материалов.
fn shapes_origin() -> Vec2 {
    let materials = material_rect();
    let width = shapes::ROW_LABEL_WIDTH + shapes::COLUMNS.len() as f32 * shapes::CELL_PITCH.x;
    let rows = shapes::row_count() as f32;
    Vec2::new(
        materials.center().x - width / 2.0 + shapes::ROW_LABEL_WIDTH + shapes::CELL_PITCH.x / 2.0,
        materials.max.y + SHAPES_GAP + shapes::BOTTOM_REACH + (rows - 1.0) * shapes::CELL_PITCH.y,
    )
}

/// Прямоугольник сетки форм вместе с колонкой подписей слева и подписями под
/// нижним рядом.
fn shapes_rect() -> Rect {
    let origin = shapes_origin();
    let columns = shapes::COLUMNS.len() as f32;
    let rows = shapes::row_count() as f32;
    Rect::from_corners(
        origin
            - Vec2::new(
                shapes::CELL_PITCH.x / 2.0 + shapes::ROW_LABEL_WIDTH,
                (rows - 1.0) * shapes::CELL_PITCH.y + shapes::BOTTOM_REACH,
            ),
        origin + Vec2::new((columns - 0.5) * shapes::CELL_PITCH.x, shapes::TOP_REACH),
    )
}

/// Прямоугольник всей витрины в мировых единицах — по нему ставится камера.
fn grid_rect() -> Rect {
    material_rect().union(shapes_rect())
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
fn spawn_labels(mut commands: Commands, assets: Res<AssetServer>, view: Res<View>) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    for (index, block) in blocks().iter().enumerate() {
        let cells = cells(index, block);
        let Some(first) = cells.first() else {
            continue;
        };
        let base = first.centre.y - first.half.y;
        commands.spawn((
            Caption,
            Text2d::new(format!("{} — {}", block.title, block.note)),
            label_font(&font, HEADER_FONT),
            TextColor(view.ground.ink()),
            Anchor::CENTER_LEFT,
            Transform::from_xyz(first.centre.x - first.half.x, base + HEADER_RISE, 2.0)
                .with_scale(Vec3::splat(TEXT_SCALE)),
        ));
        for cell in &cells {
            commands.spawn((
                Caption,
                Text2d::new(cell.color.to_hex()),
                label_font(&font, CAPTION_FONT),
                TextColor(view.ground.ink()),
                Transform::from_xyz(cell.centre.x, base - CAPTION_DROP, 2.0)
                    .with_scale(Vec3::splat(TEXT_SCALE)),
            ));
        }
    }
}

/// Пересборка витрины под текущие ручки: деспавн прежнего слоя и сборка
/// нового тем же вызовом, которым дома строит игра.
///
/// Порядок укладки — painter's, как в `extrusion_builder`: дальние дома
/// раньше ближних, иначе поднятая крыша соседа сверху окажется под ним. В
/// сетках он совпадает с порядком обхода — ряды идут сверху вниз, а внутри
/// ряда дома разнесены зазором и не перекрываются вовсе, — поэтому сортировки
/// тут нет: сетка витрины не город, её раскладку выбирает она сама.
fn rebuild_roofs(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<AssetServer>,
    roof: Res<RoofMaterialHandle>,
    tuning: Res<Tuning>,
    view: Res<View>,
    existing: Query<Entity, Or<(With<RoofLayer>, With<ShapeCaption>)>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let rotation = Rot2::degrees(tuning.axis_deg);
    let axis = rotation * Vec2::X;
    let mut builder = MeshBuilder::with_roof_coords();
    // посев у каждого дома свой, ручка крутит их все разом: в игре он берётся
    // из первой вершины контура затем, чтобы швы соседних домов не выстроились
    // в одну линию через квартал, и затем, чтобы у домов был разный возраст
    // кровли (`roof.wgsl::roof_age`) — отсюда разное число заплат на битумных
    // домах одного блока
    let mut house = 0usize;
    let mut next_seed = || {
        let seed = (tuning.seed + house as f32 * PHASE_STEP).fract();
        house += 1;
        seed
    };

    // сетка форм стоит выше материалов, значит и укладывается раньше
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    let shape_color = SHAPE_ROOF.palette()[0].to_srgba();
    for cell in shapes::cells(shapes_origin(), tuning.height) {
        let look = RoofLook::new(SHAPE_ROOF, shape_color, axis, next_seed());
        // оборудование включено: труба на коньке — часть того, как читается
        // скатная крыша, и ставит её то же правило, что в городе
        let drawn = push_house(
            &mut builder,
            &cell.area,
            &look,
            shape_color,
            cell.shape,
            true,
        );
        spawn_shape_captions(&mut commands, &font, &view, &cell, drawn);
    }

    for (index, block) in blocks().iter().enumerate() {
        for cell in cells(index, block) {
            let look = RoofLook::new(block.kind, cell.color, axis, next_seed());
            // форма — плоская, и заказана нарочно: блок материалов про цвет и
            // фактуру, а скат увёл бы половину кровли из-под взгляда.
            // Оборудования по той же причине нет: шахты и будки закрывают
            // ровно то, ради чего сюда смотрят
            push_house(
                &mut builder,
                &material_house(&cell, rotation, &tuning),
                &look,
                cell.color,
                RoofShape::Flat,
                false,
            );
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

/// Подписи одной клетки сетки форм: что легло на дом — под ним, а имя контура
/// с числами выбора — слева от ряда, один раз на ряд.
fn spawn_shape_captions(
    commands: &mut Commands,
    font: &Handle<Font>,
    view: &View,
    cell: &shapes::ShapeCell,
    drawn: RoofShape,
) {
    let label = |text: String, at: Vec2, anchor: Anchor| {
        (
            Caption,
            ShapeCaption,
            Text2d::new(text),
            label_font(font, SHAPE_CAPTION_FONT),
            TextColor(view.ground.ink()),
            anchor,
            view.visibility(),
            Transform::from_translation(at.extend(2.0)).with_scale(Vec3::splat(TEXT_SCALE)),
        )
    };
    commands.spawn(label(
        shapes::cell_caption(cell, drawn),
        cell.caption_at(),
        Anchor::TOP_CENTER,
    ));
    if cell.first_in_row() {
        commands.spawn(label(
            shapes::row_caption(cell.row),
            cell.centre - Vec2::new(shapes::CELL_PITCH.x / 2.0 + 1.0, 0.0),
            Anchor::CENTER_RIGHT,
        ));
    }
}

/// Дом блока материалов: прямоугольная коробка с необязательным двором,
/// повёрнутая на угол длинной оси.
///
/// Назначение у всех одно (`Other`) и намеренно: по нему игра красит стены
/// (`facade_color`), и разные тона стен под разными блоками сбивали бы
/// сравнение самих кровель.
fn material_house(cell: &Cell, rotation: Rot2, tuning: &Tuning) -> PolyArea {
    let (outer, holes) = footprint(cell, rotation, tuning.courtyard);
    PolyArea {
        outer,
        holes,
        kind: AreaKind::Building,
        building_use: BuildingUse::Other,
        height: Some(tuning.height),
        entrances: Vec::new(),
    }
}

/// Шаг фазы между соседними домами витрины — иррациональная доля, чтобы фазы
/// не повторялись по кругу палитры.
const PHASE_STEP: f32 = 0.147;

/// Наименьшая сторона двора и наименьшее поле от двора до края крыши, м:
/// двор уже метра не читается, а поле уже трёх метров не оставляет места
/// стене двора — она рисуется по дальней стороне дырки и требует ширины.
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

fn cycle_ground(mut view: ResMut<View>) {
    view.ground = view.ground.next();
}

fn toggle_captions(mut view: ResMut<View>) {
    view.captions = !view.captions;
}

/// Ручки солнца — в игровой `SunOnMap`, из которого `apply_sun` кладёт его в
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
