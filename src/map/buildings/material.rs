//! Материал кровли: чем крыша покрыта, какого она от этого цвета и какую
//! фактуру кладёт поверх шейдер `assets/shaders/roof.wgsl`.
//!
//! До этого крыша была заливкой по назначению здания плюс ±3 % по индексу в
//! массиве, и с воздуха квартал читался как выкройка. На снимке же кровля —
//! это в первую очередь **материал**: рулонный битум панельного дома со швами
//! ковра и заплатами, гравийная засыпка, фальцевый металл общественного
//! здания, профлист гаража, черепица частного сектора, светлая ПВХ-мембрана
//! нового ТЦ. Материал выбирается детерминированно по назначению и посеву от
//! геометрии ([`roof_look`]), цвет берётся из палитры материала, а его
//! фактуру рисует шейдер по мировой координате, повёрнутой в длинную ось дома
//! ([`crate::map::meshing::ATTRIBUTE_ROOF`]).
//!
//! Стены едут в том же меше, что и крыши (2.5D — один слой с painter's
//! порядком), поэтому **код материала в атрибуте — один словарь на двоих**:
//! `0` это «фактуры нет вовсе» (оборудование кровли, кайма), `1…6` — кровля
//! ([`RoofKind`]), `7…11` — стена ([`WallKind`]), `12` — дверное полотно
//! ([`DOOR_CODE`]). Фронтон — верх той же стены и рамку берёт её же.
//!
//! Стена устроена по образцу кровли и выбирается тем же способом: таблица
//! материалов по назначению дома, слот в ней по посеву от первой вершины
//! контура, цвет — из палитры выбранного материала ([`wall_look`]). Отчего их
//! несколько, а не одна: панельные швы с балконами это ровно **один** тип
//! дома, а город состоит не из него. Кирпич, штукатурка частного сектора,
//! витраж торгового центра и профлист склада отличаются и рисунком проёмов, и
//! тем, что между ними, — а пока стена была одна, панельные швы носил весь
//! город, гараж и церковь включительно.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use super::garages::GarageRun;
use crate::map::meshing::{ATTRIBUTE_ROOF, Roof, min_area_rect};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};
use crate::map::seed::seed_from_point;
use crate::map::sun_light;
use crate::settings::ROOF_TEXTURE_DEFAULT;

const SHADER_PATH: &str = "shaders/roof.wgsl";

/// Чем крыша покрыта. Код материала (`code`) едет в вершинный атрибут и
/// разбирается шейдером; ноль занят «без фактуры» (оборудование кровли,
/// кайма), поэтому коды начинаются с единицы. Стена — не ноль и не кровля, а
/// продолжение того же словаря ([`WallKind`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofKind {
    /// Рулонный битумный ковёр: швы через метр, заплаты ремонта, лужи.
    /// Кровля почти всякого панельного и кирпичного дома.
    Bitumen,
    /// Гравийная засыпка по битуму — светлее и зернистее, без швов.
    Gravel,
    /// Фальцевый металл: частые рёбра с бликом, окрашенный или оцинковка.
    Seam,
    /// Профлист: волна много мельче фальца, гаражи и промка.
    Corrugated,
    /// Черепица, шифер, ондулин — ряды поперёк ската, частный сектор.
    Tile,
    /// ПВХ-мембрана: светлая, почти ровная, широкие полотнища. Новые ТЦ.
    Membrane,
    /// Лента гаражного кооператива: тот же профлист, но со швом на каждом
    /// боксе и своим тоном краски внутри шва. Ставится не по назначению, а
    /// по геометрии прогона — см. [`super::garages`].
    GarageRow,
    /// Кооператив целиком одним контуром: та же лента, но с **проездами**
    /// между парами рядов — на снимке ГСК это сетка, а не полоса.
    GarageBlock,
}

impl RoofKind {
    /// Исчерпывающий список **материалов** — по нему идёт витрина
    /// `roof_gallery`. Гаражных лент тут нет: их выбирает не назначение дома
    /// с посевом, а геометрия прогона ([`super::garages`]), и перебрать их
    /// витрина всё равно не может.
    pub const ALL: [Self; 6] = [
        Self::Bitumen,
        Self::Gravel,
        Self::Seam,
        Self::Corrugated,
        Self::Tile,
        Self::Membrane,
    ];

    /// Код для [`ATTRIBUTE_ROOF`]; `0` — вершина без фактуры.
    ///
    /// Код **позиционный** — номер варианта плюс единица, — а его зеркало это
    /// набор констант `roof.wgsl` (`BITUMEN = 1u` … `GARAGE_BLOCK = 8u`,
    /// дальше стены). Значит, вариант можно только дописать в конец: вставка в
    /// середину молча сдвинет коды всех, кто ниже, и шейдер начнёт класть
    /// чужую фактуру. Коды стен ([`WallKind::code`]) идут следом за последним
    /// кровельным, так что дописанная кровля сдвигает и их — зеркало в
    /// шейдере одно, и правится оно целиком.
    pub const fn code(self) -> u32 {
        self as u32 + 1
    }

    /// Сколько кодов занято кровлями. Считается по **последнему варианту**, а
    /// не по длине [`Self::ALL`], и это не мелочь: гаражные ленты в `ALL` не
    /// входят, а коды занимают, и стены, отсчитанные от длины списка, легли
    /// бы прямо на них.
    pub const CODES: u32 = Self::GarageBlock.code();

    pub fn label(self) -> &'static str {
        match self {
            Self::Bitumen => "Битум",
            Self::Gravel => "Гравий",
            Self::Seam => "Фальц",
            Self::Corrugated => "Профлист",
            Self::Tile => "Черепица",
            Self::Membrane => "Мембрана",
            // выбираются по геометрии прогона, а не по назначению, поэтому в
            // `ALL` их нет и витрина их не перебирает
            Self::GarageRow => "Гаражная лента",
            Self::GarageBlock => "Гаражный блок",
        }
    }

    /// Палитра материала: несколько правдоподобных цветов, дом выбирает свой
    /// посевом. Кремль и храм красятся не по материалу — их палитры знает
    /// [`palette`].
    pub fn palette(self) -> &'static [Color] {
        match self {
            Self::Bitumen => &BITUMEN_COLORS,
            Self::Gravel => &GRAVEL_COLORS,
            Self::Seam => &SEAM_COLORS,
            Self::Corrugated => &CORRUGATED_COLORS,
            Self::Tile => &TILE_COLORS,
            Self::Membrane => &MEMBRANE_COLORS,
            Self::GarageRow | Self::GarageBlock => &GARAGE_ROW_COLORS,
        }
    }
}

/// Чем облицована стена: не «из чего дом построен», а **что видно снаружи** и
/// как на этом расставлены проёмы. Материал решает и рисунок между окнами
/// (швы плит, ряды кирпича, рёбра профлиста), и сами окна, и бывают ли на
/// этой стене балконы.
///
/// Коды продолжают кровельные — один словарь в одном числе атрибута, — а
/// зеркало обоих половин лежит в `roof.wgsl` (`PANEL = 9u` … `GARAGE_DOORS =
/// 14u`, за ними `DOOR = 15u`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WallKind {
    /// Панель: межэтажные швы, вертикальные швы плит, окно на панель и
    /// столбцы балконов. Типовая многоэтажка — то, чем была единственная
    /// стена до появления остальных.
    Panel,
    /// Кирпич: ряды кладки вместо швов, окно уже панельного, с перемычкой и
    /// отливом; балконы реже и утоплены — лоджией, а не выступом.
    Brick,
    /// Штукатурка: гладкая стена в пятнах, окна мелкие и редкие, балконов
    /// нет. Частный сектор, малоэтажная застройка, храм.
    Plaster,
    /// Витраж: сплошное остекление лентами через этаж, между ними глухой
    /// пояс, вертикальные импосты. Торговый центр, офис, новая общественная
    /// коробка.
    Shopfront,
    /// Профлист: вертикальные рёбра во всю стену, ленточное окно под
    /// карнизом да ворота внизу. Склад, промка, гаражный ряд.
    Shed,
    /// Ворота: створка в **каждой** ячейке и ничего больше — ни окна, ни
    /// балкона. Стена гаражного прогона, и выбирается она не по назначению
    /// дома, а по геометрии — как и кровля прогона ([`super::garages`]):
    /// ячейка тут не панель в 3.2 м, а сам бокс, поэтому створка приходится
    /// ровно под шов на кровле, и ряд ворот с рядом боксов говорят одно и то
    /// же.
    GarageDoors,
}

impl WallKind {
    /// Исчерпывающий список — по нему идёт витрина `wall_gallery`.
    pub const ALL: [Self; 6] = [
        Self::Panel,
        Self::Brick,
        Self::Plaster,
        Self::Shopfront,
        Self::Shed,
        Self::GarageDoors,
    ];

    /// Код для [`ATTRIBUTE_ROOF`]: продолжение кровельного словаря, поэтому
    /// он **выводится** из числа кровельных кодов ([`RoofKind::CODES`]), а не
    /// пишется числом. Дописанная кровля сдвинет коды стен — и это правильно:
    /// зеркало в `roof.wgsl` одно на весь словарь и правится целиком.
    pub const fn code(self) -> u32 {
        RoofKind::CODES + 1 + self as u32
    }

    /// Сколько кодов занято облицовками — по **последнему** варианту, тем же
    /// правилом, что и [`RoofKind::CODES`]: от него отсчитывается дверь, и
    /// дописанная облицовка обязана её подвинуть, а не наехать на неё.
    pub const CODES: u32 = Self::GarageDoors.code();

    /// Стена ли это. Коды приходят из вершинного атрибута числом с плавающей
    /// точкой, и разбирать их порознь в тестах и в шейдере — верный способ
    /// разойтись; вопрос «стена или кровля» задаётся здесь.
    pub fn is_code(code: u32) -> bool {
        code >= Self::Panel.code() && code <= Self::CODES
    }
}

/// Код дверного полотна — последний в том же словаре, сразу за облицовками.
///
/// Дверь не облицовка и не кровля: это **отдельный четырёхугольник** поверх
/// стены, со своей рамой в клетку `[0, 1]²` ([`WallFrame::opening`]), и код ей
/// нужен ровно затем, чтобы шейдер узнал полотно и не искал в нём ни этажей,
/// ни панельных швов. Ставит её `buildings::layers::push_doors` по входам из
/// `osm::entrances` — там же, где их видит гизмо дверей и куда идут пешки.
///
/// [`WallFrame::opening`]: crate::map::meshing::WallFrame::opening
pub const DOOR_CODE: u32 = WallKind::CODES + 1;

impl WallKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Panel => "Панель",
            Self::Brick => "Кирпич",
            Self::Plaster => "Штукатурка",
            Self::Shopfront => "Витраж",
            Self::Shed => "Профлист",
            Self::GarageDoors => "Ворота",
        }
    }

    /// Палитра материала — как у кровель, дом выбирает свой цвет посевом.
    /// Кремль и храм красятся не по ней ([`wall_palette`]).
    pub fn palette(self) -> &'static [Color] {
        match self {
            Self::Panel => &PANEL_WALL_COLORS,
            Self::Brick => &BRICK_WALL_COLORS,
            Self::Plaster => &PLASTER_WALL_COLORS,
            Self::Shopfront => &SHOPFRONT_WALL_COLORS,
            Self::Shed => &SHED_WALL_COLORS,
            Self::GarageDoors => &GARAGE_WALL_COLORS,
        }
    }
}

/// Палитры стен. Светлота выверена по тому же правилу, что и кровельная, но с
/// обратным знаком: **стена светлее своей крыши**, и это не стиль, а то, что
/// держит коробку 2.5D — тёмный битум над светлой панелью. Отсюда и границы:
/// панель и штукатурка живут в 0.66–0.86, кирпич опускается до 0.5 только на
/// тёмно-красном, и один лишь витраж заведомо темнее кровли — но он достаётся
/// торговым центрам, которых в квартале единицы.
///
/// Разброс внутри палитры — тот же приём, что у кровель: **много тона, мало
/// светлоты** у типовой застройки (соседние панельки отличаются оттенком) и
/// наоборот у частного сектора, где через забор стоят охра, белёный кирпич и
/// голубая штукатурка.
const PANEL_WALL_COLORS: [Color; 6] = [
    Color::srgb(0.78, 0.76, 0.71),
    Color::srgb(0.74, 0.73, 0.72),
    Color::srgb(0.80, 0.77, 0.68),
    Color::srgb(0.72, 0.74, 0.74),
    Color::srgb(0.76, 0.71, 0.66),
    Color::srgb(0.70, 0.72, 0.70),
];
const BRICK_WALL_COLORS: [Color; 6] = [
    // силикатный белый — половина тульских кирпичных домов
    Color::srgb(0.80, 0.79, 0.74),
    Color::srgb(0.74, 0.72, 0.66),
    // красный керамический и его выцветшие оттенки
    Color::srgb(0.63, 0.42, 0.34),
    Color::srgb(0.70, 0.50, 0.40),
    Color::srgb(0.55, 0.38, 0.32),
    Color::srgb(0.72, 0.62, 0.50),
];
const PLASTER_WALL_COLORS: [Color; 7] = [
    Color::srgb(0.86, 0.84, 0.78),
    Color::srgb(0.82, 0.75, 0.60),
    Color::srgb(0.78, 0.72, 0.66),
    Color::srgb(0.72, 0.76, 0.70),
    Color::srgb(0.70, 0.75, 0.80),
    Color::srgb(0.84, 0.72, 0.62),
    Color::srgb(0.66, 0.64, 0.60),
];
const SHOPFRONT_WALL_COLORS: [Color; 4] = [
    Color::srgb(0.52, 0.56, 0.60),
    Color::srgb(0.60, 0.62, 0.64),
    Color::srgb(0.46, 0.52, 0.58),
    Color::srgb(0.64, 0.64, 0.62),
];
const SHED_WALL_COLORS: [Color; 5] = [
    Color::srgb(0.68, 0.69, 0.70),
    Color::srgb(0.56, 0.62, 0.70),
    Color::srgb(0.52, 0.60, 0.52),
    Color::srgb(0.72, 0.66, 0.56),
    Color::srgb(0.62, 0.60, 0.58),
];

/// Стена гаражного прогона: побелка, силикатный кирпич, крашеный простенок
/// между воротами. Светлее самих створок, которые шейдер кладёт поверх, —
/// иначе ряд ворот не читается вовсе.
const GARAGE_WALL_COLORS: [Color; 4] = [
    Color::srgb(0.74, 0.72, 0.67),
    Color::srgb(0.70, 0.68, 0.64),
    Color::srgb(0.72, 0.69, 0.62),
    Color::srgb(0.66, 0.66, 0.64),
];

/// Палитры материалов — по несколько правдоподобных цветов на каждый, дом
/// выбирает свой посевом. Светлота выверена по спутниковому снимку Тулы
/// **на общем плане**, где фактура уже погашена и от кровли остаётся один
/// базовый цвет: битум панельного дома там — средне-серый (~0.55 в sRGB),
/// а не почти чёрный; первая версия палитр (0.38–0.50) на светлой земле
/// карты (~0.9) читалась грязно-тёмными коробками.
///
/// Разброс внутри палитры разный по назначению. У **плоских кровель
/// корпусов** (битум, гравий, мембрана) он узкий: соседние панельные дома на
/// снимке отличаются оттенком, а не светлотой, и широкий разброс дал бы
/// конфетти. У **частного сектора** (черепица, а с ним фальц и профлист)
/// наоборот — цвета яркие и разные: красная и коричневая металлочерепица,
/// зелёная, синяя, серебристый и тёмный шифер стоят через забор друг от
/// друга, и палитра из серых оттенков делала посёлок одноцветным.
const BITUMEN_COLORS: [Color; 5] = [
    Color::srgb(0.55, 0.54, 0.52),
    Color::srgb(0.60, 0.58, 0.55),
    Color::srgb(0.50, 0.49, 0.48),
    Color::srgb(0.57, 0.55, 0.51),
    Color::srgb(0.63, 0.61, 0.58),
];
const GRAVEL_COLORS: [Color; 3] = [
    Color::srgb(0.70, 0.68, 0.64),
    Color::srgb(0.74, 0.72, 0.67),
    Color::srgb(0.66, 0.64, 0.60),
];
const SEAM_COLORS: [Color; 5] = [
    Color::srgb(0.62, 0.63, 0.64),
    Color::srgb(0.36, 0.52, 0.42),
    Color::srgb(0.60, 0.32, 0.27),
    Color::srgb(0.44, 0.50, 0.60),
    Color::srgb(0.70, 0.70, 0.69),
];
const CORRUGATED_COLORS: [Color; 5] = [
    Color::srgb(0.70, 0.71, 0.72),
    Color::srgb(0.42, 0.50, 0.62),
    Color::srgb(0.38, 0.52, 0.40),
    Color::srgb(0.62, 0.34, 0.28),
    Color::srgb(0.56, 0.40, 0.30),
];
const TILE_COLORS: [Color; 7] = [
    Color::srgb(0.72, 0.22, 0.18),
    Color::srgb(0.78, 0.45, 0.30),
    Color::srgb(0.58, 0.36, 0.26),
    Color::srgb(0.32, 0.50, 0.36),
    Color::srgb(0.30, 0.42, 0.64),
    Color::srgb(0.72, 0.71, 0.68),
    Color::srgb(0.45, 0.45, 0.44),
];
const MEMBRANE_COLORS: [Color; 3] = [
    Color::srgb(0.80, 0.80, 0.78),
    Color::srgb(0.75, 0.76, 0.76),
    Color::srgb(0.84, 0.84, 0.82),
];
/// Гаражная лента: оцинковка, шифер, крашеный суриком профлист — цвет один
/// на весь кооператив, разнобой боксов кладёт шейдер поверх.
///
/// Темнее прочих кровельных палитр на четверть, и это не вкус: на первом
/// закадровом снимке лента вышла почти белой полосой рядом с асфальтом в
/// 0.35. Гаражная кровля — это шифер и ржавая оцинковка, то есть тон между
/// битумом и асфальтом, а не свежий металл.
const GARAGE_ROW_COLORS: [Color; 4] = [
    Color::srgb(0.45, 0.45, 0.44),
    Color::srgb(0.41, 0.39, 0.36),
    Color::srgb(0.40, 0.30, 0.25),
    Color::srgb(0.36, 0.39, 0.38),
];
/// Храм остаётся зелёным, как его рисуют на картах, — но теперь это зелёный
/// **металл**, с фальцем и бликом. Единственная палитра не по материалу:
/// [`RoofKind::palette`] её не знает, её выбирает [`palette`] по назначению.
pub const CHURCH_ROOF_COLORS: [Color; 3] = [
    Color::srgb(0.36, 0.54, 0.46),
    Color::srgb(0.32, 0.50, 0.52),
    Color::srgb(0.40, 0.56, 0.40),
];

/// Доли материалов по назначению — десять слотов, то есть проценты по
/// десяткам; дом берёт слот посевом. Не выдумка: по спутнику Тулы частный
/// сектор — черепица и шифер с вкраплениями профлиста, панельные кварталы —
/// сплошной битум, промзона — профлист.
const HOUSE_ROOFS: [RoofKind; 10] = [
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Tile,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
];
const APARTMENTS_ROOFS: [RoofKind; 10] = [
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
    RoofKind::Membrane,
];
const COMMERCIAL_ROOFS: [RoofKind; 10] = [
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Membrane,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Corrugated,
];
const INDUSTRIAL_ROOFS: [RoofKind; 10] = [
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Gravel,
    RoofKind::Seam,
];
const GARAGE_ROOFS: [RoofKind; 10] = [
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Corrugated,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Seam,
    RoofKind::Tile,
];
const PUBLIC_ROOFS: [RoofKind; 10] = [
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Bitumen,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Seam,
    RoofKind::Gravel,
    RoofKind::Gravel,
    RoofKind::Membrane,
];

/// Пятно, ниже которого `building=yes` считается частным домом, — то же
/// число, по которому [`super::roofs`] ставит на него двускатную крышу: одна
/// граница, один смысл «это дом, а не корпус».
const SMALL_FOOTPRINT_MAX: f32 = 250.0;

/// Кровля дома глазами отрисовки: чем крыта, какого цвета и с какой рамкой
/// для шейдера.
pub struct RoofLook {
    /// Чем крыта — код материала уходит в шейдер фактуры, а сам материал
    /// решает, что стоит на кровле (`clutter.rs`).
    pub kind: RoofKind,
    /// Базовый цвет — из палитры материала; рампу по высоте и затенение
    /// ската кладут поверх вызывающие.
    pub base: Srgba,
    /// Что уходит в [`ATTRIBUTE_ROOF`] на каждой вершине кровли.
    pub frame: Roof,
    /// Гаражный прогон, если дом в него вошёл: у ленты кровля считается не по
    /// мировой точке и оси, а **в ячейках прогона** — по боксу вдоль и ряду
    /// поперёк, — и рамку каждой вершине даёт он (`layers::garage_frame`).
    pub(super) run: Option<GarageRun>,
}

impl RoofLook {
    /// Кровля, заданная напрямую: материал, цвет, длинная ось дома и посев
    /// вариаций фактуры. Игра все четыре числа выводит из самого дома
    /// ([`roof_look`]) — витрина `roof_gallery` перебирает ими все материалы
    /// и все их палитры.
    ///
    /// Посев несёт **два** независимых смысла, и оба выводит из него шейдер:
    /// фазу швов (чтобы ковры соседних домов не выстроились в одну линию) и
    /// **возраст** кровли (`roof.wgsl::roof_age` — хеш от посева: сколько на
    /// битуме заплат, сколько на ней луж, насколько она выгорела). Отдельного
    /// числа под возраст нет намеренно: на каждой вершине слоя зданий это ещё
    /// четыре байта, а корреляция двух узоров одного дома глазом не видна.
    pub fn new(kind: RoofKind, base: Srgba, axis: Vec2, seed: f32) -> Self {
        Self {
            kind,
            base,
            frame: Roof {
                axis,
                material: kind.code(),
                seed,
            },
            run: None,
        }
    }
}

/// Кровля этого дома: материал по назначению и посеву, цвет из палитры
/// материала, длинная ось контура — как ось фактуры.
///
/// Посев — от **первой вершины контура**, как у генератора дверей: он не
/// зависит ни от порядка домов в `MapData`, ни от режима отрисовки, поэтому
/// переключение режима высот не перекрашивает город.
pub(super) fn roof_look(building: &PolyArea) -> RoofLook {
    let seed = building_seed(building);
    let kind = kind_of(building, seed);
    let palette = palette(building, kind);
    let base = palette[(seed >> 8) as usize % palette.len()].to_srgba();
    // ±3 % яркости поверх выбранного цвета: два дома одной палитры и одного
    // слота всё-таки не близнецы
    let jitter = 1.0 + ((seed >> 16 & 0xff) as f32 / 255.0 - 0.5) * 0.06;
    let axis = min_area_rect(&building.outer)
        .and_then(|rect| (rect[1] - rect[0]).try_normalize())
        .unwrap_or(Vec2::X);
    RoofLook::new(
        kind,
        Srgba {
            red: base.red * jitter,
            green: base.green * jitter,
            blue: base.blue * jitter,
            alpha: 1.0,
        },
        axis,
        // старший байт посева — фаза фактуры и возраст кровли
        // ([`RoofLook::new`]); младшие уже разобраны на слот и цвет
        (seed >> 24) as f32 / 255.0,
    )
}

/// Материал кровли этого дома. Кремль крыт металлом (его цвет всё равно свой),
/// «дом» и мелкая коробка без назначения — частный сектор, остальное по
/// назначению.
fn kind_of(building: &PolyArea, seed: u32) -> RoofKind {
    if building.kind == AreaKind::Kremlin {
        return RoofKind::Seam;
    }
    let table = match building.building_use {
        BuildingUse::House => &HOUSE_ROOFS,
        BuildingUse::Apartments => &APARTMENTS_ROOFS,
        BuildingUse::Commercial => &COMMERCIAL_ROOFS,
        BuildingUse::Industrial => &INDUSTRIAL_ROOFS,
        BuildingUse::Garage | BuildingUse::GarageBlock => &GARAGE_ROOFS,
        BuildingUse::Church => return RoofKind::Seam,
        BuildingUse::Public => &PUBLIC_ROOFS,
        // `building=yes` — половина города: мелкая коробка это частный дом,
        // крупный контур — корпус, и кроют их по-разному
        BuildingUse::Other => {
            if footprint_area(building) <= SMALL_FOOTPRINT_MAX {
                &HOUSE_ROOFS
            } else {
                &APARTMENTS_ROOFS
            }
        }
    };
    table[seed as usize % table.len()]
}

/// Палитра цвета кровли: Кремль и храм — своё, остальные по материалу.
fn palette(building: &PolyArea, kind: RoofKind) -> &'static [Color] {
    if building.kind == AreaKind::Kremlin {
        return std::slice::from_ref(&super::KREMLIN_ROOF_COLOR);
    }
    if building.building_use == BuildingUse::Church {
        return &CHURCH_ROOF_COLORS;
    }
    kind.palette()
}

/// Кровля гаражного прогона: цвет и посев — **общие на весь прогон**. Именно
/// общий посев и делает из двадцати боксов одну ленту: иначе каждый красится
/// и ребрится сам по себе.
///
/// Рамка тут номинальная: у гаражной ленты координаты вершине приходят из её
/// собственных ячеек (`layers::garage_frame`), а не из мировой точки и оси.
pub(super) fn run_look(run: &GarageRun) -> RoofLook {
    let base = GARAGE_ROW_COLORS[(run.seed >> 8) as usize % GARAGE_ROW_COLORS.len()].to_srgba();
    let jitter = 1.0 + ((run.seed >> 16 & 0xff) as f32 / 255.0 - 0.5) * 0.06;
    // материал — по самому большому куску: у буквы Г одно крыло может быть
    // кооперативом, а другое лентой, и на кровле это видно по каждому куску
    // отдельно (`layers::garage_frame` берёт код у него), но `RoofKind` дома
    // один — им выбираются палитра и оборудование на кровле
    let kind = run_kind(run.main().block);
    RoofLook {
        kind,
        base: Srgba {
            red: base.red * jitter,
            green: base.green * jitter,
            blue: base.blue * jitter,
            alpha: 1.0,
        },
        frame: Roof {
            axis: run.main().axis,
            material: kind.code(),
            seed: garage_seed(run),
        },
        run: Some(run.clone()),
    }
}

/// Чем крыт гаражный кусок: кооператив рисуется рядами с проездами, лента —
/// одной гребёнкой боксов.
pub(super) fn run_kind(block: bool) -> RoofKind {
    match block {
        true => RoofKind::GarageBlock,
        false => RoofKind::GarageRow,
    }
}

/// Стена гаражного прогона: ворота, и цвет простенка тоже общий на прогон.
/// Выбирается она по геометрии, а не по назначению дома, поэтому мимо
/// [`wall_look`] и его таблиц — как и кровля прогона.
pub(super) fn run_wall_look(run: &GarageRun) -> WallLook {
    let palette = GARAGE_WALL_COLORS;
    let base = palette[(run.seed >> 12) as usize % palette.len()].to_srgba();
    let jitter = 1.0 + ((run.seed >> 20 & 0xff) as f32 / 255.0 - 0.5) * 0.06;
    WallLook::new(
        WallKind::GarageDoors,
        Srgba {
            red: base.red * jitter,
            green: base.green * jitter,
            blue: base.blue * jitter,
            alpha: 1.0,
        },
    )
}

/// Посев прогона в том виде, в каком его читает шейдер: доля единицы. Им
/// разнесены тона краски соседних лент — цвет уже выбран по тому же посеву,
/// но два прогона одного цвета не должны ещё и краситься одинаково.
pub(super) fn garage_seed(run: &GarageRun) -> f32 {
    (run.seed & 0xff) as f32 / 255.0
}

fn footprint_area(building: &PolyArea) -> f32 {
    crate::map::osm::model::signed_ring_area(&building.outer).abs()
}

/// Стена дома глазами отрисовки: чем облицована и какого от этого цвета.
/// Рамку ([`crate::map::meshing::WallFrame`]) собирает уже `layers.rs` — она
/// у каждой стены своя, а вид у всего дома один.
#[derive(Clone, Copy, Debug)]
pub struct WallLook {
    pub kind: WallKind,
    pub base: Srgba,
}

impl WallLook {
    /// Стена, заданная напрямую: материал и цвет. Игра оба выводит из дома
    /// ([`wall_look`]) — витрина `wall_gallery` перебирает ими все материалы
    /// и все их палитры.
    pub fn new(kind: WallKind, base: Srgba) -> Self {
        Self { kind, base }
    }
}

/// Стена этого дома: материал по назначению, этажности и посеву, цвет из
/// палитры материала.
///
/// Посев — тот же [`building_seed`], что у кровли, но разобранный **другими**
/// байтами: материал стены и материал кровли должны быть независимы, иначе
/// панельные дома окажутся ещё и все под одним битумом.
///
/// `storeys` приходит извне, а не считается здесь, потому что этажи — это
/// высота, поделённая на высоту этажа, и делит её `layers.rs`; дублировать то
/// же деление значит однажды разойтись с ним.
pub(super) fn wall_look(building: &PolyArea, storeys: f32) -> WallLook {
    let seed = building_seed(building);
    let kind = wall_kind_of(building, storeys, seed >> 4);
    let palette = wall_palette(building, kind);
    let base = palette[(seed >> 12) as usize % palette.len()].to_srgba();
    // ±3 % яркости поверх выбранного цвета — как у кровель: два дома одной
    // палитры и одного слота всё-таки не близнецы
    let jitter = 1.0 + ((seed >> 20 & 0xff) as f32 / 255.0 - 0.5) * 0.06;
    WallLook::new(
        kind,
        Srgba {
            red: base.red * jitter,
            green: base.green * jitter,
            blue: base.blue * jitter,
            alpha: 1.0,
        },
    )
}

/// Материал стены этого дома.
///
/// Порядок проверок здесь обратный кровельному, и намеренно: кровлю решает
/// назначение, а стену — сперва **рост**. Низкий дом не бывает ни панельным,
/// ни витражным, чем бы он ни был по тегу: у панели с балконами нет этажей,
/// у витража — высоты, ради которой его вешают. Поэтому всё ниже
/// [`LOW_RISE_STOREYS`] уходит в частный сектор, а таблица по назначению
/// разбирает то, что осталось.
fn wall_kind_of(building: &PolyArea, storeys: f32, seed: u32) -> WallKind {
    if building.kind == AreaKind::Kremlin {
        return WallKind::Brick;
    }
    let table: &[WallKind] = match building.building_use {
        BuildingUse::House => &HOUSE_WALLS,
        BuildingUse::Garage | BuildingUse::GarageBlock => &GARAGE_WALLS,
        BuildingUse::Church => return WallKind::Plaster,
        BuildingUse::Industrial => &INDUSTRIAL_WALLS,
        _ if storeys < LOW_RISE_STOREYS => &LOW_RISE_WALLS,
        BuildingUse::Apartments => &APARTMENTS_WALLS,
        BuildingUse::Commercial => &COMMERCIAL_WALLS,
        BuildingUse::Public => &PUBLIC_WALLS,
        // `building=yes` — половина города, и это ровно тот случай, где
        // этажность уже всё сказала: то, что доросло досюда, — корпус
        BuildingUse::Other => &APARTMENTS_WALLS,
    };
    table[seed as usize % table.len()]
}

/// Ниже скольких этажей дом считается малоэтажным, чем бы он ни был по тегу.
/// Тот же порог, по которому `layers.rs` не даёт стене балконов, — и это одна
/// константа, а не совпадающее число: одна граница, один смысл «панельного
/// дома тут нет».
const LOW_RISE_STOREYS: f32 = super::layers::BALCONY_STOREYS_MIN;

/// Таблицы материалов стен — по десять слотов, чтобы читались как проценты,
/// ровно как кровельные.
const HOUSE_WALLS: [WallKind; 10] = [
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Shed,
];
const LOW_RISE_WALLS: [WallKind; 10] = [
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Shed,
    WallKind::Shopfront,
];
const APARTMENTS_WALLS: [WallKind; 10] = [
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Panel,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
];
const COMMERCIAL_WALLS: [WallKind; 10] = [
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Panel,
];
const PUBLIC_WALLS: [WallKind; 10] = [
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Plaster,
    WallKind::Shopfront,
    WallKind::Shopfront,
    WallKind::Panel,
];
const INDUSTRIAL_WALLS: [WallKind; 10] = [
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Panel,
];
const GARAGE_WALLS: [WallKind; 10] = [
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Shed,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Brick,
    WallKind::Plaster,
];

/// Палитра цвета стены: Кремль и храм — своё, остальные по материалу. То же
/// правило и тот же порядок, что у [`palette`] для кровли.
fn wall_palette(building: &PolyArea, kind: WallKind) -> &'static [Color] {
    if building.kind == AreaKind::Kremlin {
        return std::slice::from_ref(&super::KREMLIN_FACADE_COLOR);
    }
    if building.building_use == BuildingUse::Church {
        return &CHURCH_WALL_COLORS;
    }
    kind.palette()
}

/// Храм белёный, и это не оттенок штукатурки, а её отсутствие: побелка по
/// кирпичу почти без тона.
const CHURCH_WALL_COLORS: [Color; 3] = [
    Color::srgb(0.93, 0.91, 0.86),
    Color::srgb(0.90, 0.88, 0.85),
    Color::srgb(0.88, 0.86, 0.78),
];

/// Посев дома — от его первой вершины ([`seed_from_point`]): материал кровли,
/// оборудование на ней и додуманная этажность держатся на одном числе, и оно
/// не зависит ни от порядка домов в выгрузке, ни от пересборки слоя.
pub(super) fn building_seed(building: &PolyArea) -> u32 {
    seed_from_point(building.outer.first().copied().unwrap_or(Vec2::ZERO))
}

/// Параметры фактуры кровель — юниформ шейдера. Зеркало `RoofParams` в
/// `roof.wgsl`: порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct RoofParams {
    /// Направление **на солнце** в плане (`map::sun_light`): по нему рёбра
    /// фальца и профлиста получают блик с одной стороны и тень с другой, а
    /// поперечное свету ребро видно сильнее продольного.
    pub light: Vec2,
    /// Общий множитель амплитуд — ползунок панели; ноль возвращает прежнюю
    /// плоскую заливку.
    pub intensity: f32,
}

/// Материал зданиевых слоёв: вершинный цвет × процедурная фактура кровли.
/// Один на всё приложение — [`RoofMaterialHandle`].
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct RoofMaterial {
    #[uniform(0)]
    pub params: RoofParams,
}

impl Material2d for RoofMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }

    /// Своя раскладка вершин: позиция, цвет и [`ATTRIBUTE_ROOF`] — меш без
    /// него этим материалом не нарисовать, и это намеренно: собирать слой для
    /// него надо через `MeshBuilder::with_roof_coords`.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_ROOF.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// Сила фактуры кровель; ползунок Texture секции Buildings и BRP,
/// сохраняется между запусками. Правка не пересобирает мешей — меняется
/// только юниформ материала ([`retune_roof_material`]).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roofs")]
pub struct RoofStyle {
    /// Множитель всех амплитуд, 0 — плоские крыши как прежде.
    pub texture: f32,
}

impl Default for RoofStyle {
    fn default() -> Self {
        Self {
            texture: ROOF_TEXTURE_DEFAULT,
        }
    }
}

impl RoofStyle {
    fn params(self) -> RoofParams {
        RoofParams {
            light: sun_light(),
            intensity: self.texture,
        }
    }
}

/// Хэндл материала кровель: слои пересобираются на каждый город и на каждую
/// правку режима высот, а материал живёт и переиспользуется.
#[derive(Resource)]
pub struct RoofMaterialHandle(Handle<RoofMaterial>);

impl RoofMaterialHandle {
    pub fn handle(&self) -> Handle<RoofMaterial> {
        self.0.clone()
    }
}

/// Материал кровель на старте приложения, с силой фактуры из сохранённых
/// настроек.
pub fn init_roof_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<RoofMaterial>>,
    style: Res<RoofStyle>,
) {
    let handle = materials.add(RoofMaterial {
        params: style.params(),
    });
    commands.insert_resource(RoofMaterialHandle(handle));
}

/// Правка ползунка Texture — новые параметры в материал; меши не трогаются.
pub fn retune_roof_material(
    style: Res<RoofStyle>,
    handle: Res<RoofMaterialHandle>,
    mut materials: ResMut<Assets<RoofMaterial>>,
) {
    if let Some(mut material) = materials.get_mut(&handle.0) {
        material.params = style.params();
    }
}
