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
//! порядком), поэтому материал обязан уметь и «без фактуры»: код `0` —
//! стена, фронтон, кайма.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use super::roofs::min_area_rect;
use crate::map::SHADOW_DIR;
use crate::map::meshing::{ATTRIBUTE_ROOF, Roof};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};
use crate::settings::ROOF_TEXTURE_DEFAULT;

const SHADER_PATH: &str = "shaders/roof.wgsl";

/// Чем крыша покрыта. Код материала (`code`) едет в вершинный атрибут и
/// разбирается шейдером; ноль занят «не кровлей» (стена, фронтон, кайма),
/// поэтому коды начинаются с единицы.
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
}

impl RoofKind {
    /// Исчерпывающий список материалов — по нему идёт витрина
    /// `roof_gallery`.
    pub const ALL: [Self; 6] = [
        Self::Bitumen,
        Self::Gravel,
        Self::Seam,
        Self::Corrugated,
        Self::Tile,
        Self::Membrane,
    ];

    /// Код для [`ATTRIBUTE_ROOF`]; `0` — вершина вне кровли.
    pub fn code(self) -> u32 {
        self as u32 + 1
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bitumen => "Битум",
            Self::Gravel => "Гравий",
            Self::Seam => "Фальц",
            Self::Corrugated => "Профлист",
            Self::Tile => "Черепица",
            Self::Membrane => "Мембрана",
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
        }
    }

    /// Парапет ставится только на плоскую кровлю мягкого типа — там он и есть
    /// на самом деле. У черепицы и профлиста вместо него свес, у фальца конёк.
    ///
    /// Свойство материала, а не слоя: по нему [`super::layers`] решает, класть
    /// ли кайму, и по нему же витрина подписывает материал.
    pub fn has_parapet(self) -> bool {
        matches!(self, Self::Bitumen | Self::Gravel | Self::Membrane)
    }
}

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
    /// Чем крыта — по нему решается, положен ли парапет.
    pub kind: RoofKind,
    /// Базовый цвет — из палитры материала; рампу по высоте и затенение
    /// ската кладут поверх вызывающие.
    pub base: Srgba,
    /// Что уходит в [`ATTRIBUTE_ROOF`] на каждой вершине кровли.
    pub frame: Roof,
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
        BuildingUse::Garage => &GARAGE_ROOFS,
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

fn footprint_area(building: &PolyArea) -> f32 {
    crate::map::osm::model::signed_ring_area(&building.outer).abs()
}

/// Посев дома из его первой вершины — три перемешивающих раунда, чтобы
/// соседние по координате дома не попадали в один слот таблицы. Сантиметры,
/// а не метры: два дома на одной улице отличаются десятками сантиметров.
pub(super) fn building_seed(building: &PolyArea) -> u32 {
    let point = building.outer.first().copied().unwrap_or(Vec2::ZERO);
    let x = (point.x * 100.0) as i32 as u32;
    let y = (point.y * 100.0) as i32 as u32;
    let mut hash = x ^ y.rotate_left(16);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^= hash >> 16;
    hash
}

/// Параметры фактуры кровель — юниформ шейдера. Зеркало `RoofParams` в
/// `roof.wgsl`: порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct RoofParams {
    /// Направление **на солнце** в плане (`-SHADOW_DIR`): по нему рёбра
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
            light: -SHADOW_DIR,
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
