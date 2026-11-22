//! Материал кровли: чем крыша покрыта, какого она от этого цвета и какую
//! фактуру кладёт поверх шейдер `assets/shaders/roof.wesl`.
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
//! `0` это «фактуры нет вовсе» (оборудование кровли, кайма, главы храмов),
//! `1…8` — кровля и гаражные ленты ([`RoofKind`]), `9…15` — стена
//! ([`WallKind`]), `16` — дверное полотно ([`DOOR_CODE`]). Фронтон — верх той
//! же стены и рамку берёт её же.
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
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};

use super::classify::classify;
use super::garages::GarageRun;
use super::{HouseFootprint, temples};
use crate::map::meshing::{ATTRIBUTE_ROOF, Roof};
use crate::map::osm::{BuildingUse, PolyArea};
use crate::map::seed::seed_from_point;
use crate::map::{SunOnMap, sun_light};
use crate::prefs::retuned;

mod kinds;

pub use self::kinds::{DOOR_CODE, RoofKind, WallKind};
use self::kinds::{
    GARAGE_ROW_COLORS, GARAGE_WALL_COLORS, kind_of, palette, wall_kind_of, wall_palette,
};

const SHADER_PATH: &str = "shaders/roof.wesl";

/// Дефолт и границы ползунка Texture секции Buildings
/// ([`RoofStyle::texture`]) — множитель амплитуд фактуры крыш: 0 — плоская
/// заливка материала без швов и зерна, 1 — как задумано, полтора — фактура
/// начинает спорить с цветом. Отдельно от поверхностей: кровля смотрит прямо
/// в камеру и её фактура заметнее земли, так что и крутить их порознь.
pub const ROOF_TEXTURE_DEFAULT: f32 = 1.0;
pub const ROOF_TEXTURE_MIN: f32 = 0.0;
pub const ROOF_TEXTURE_MAX: f32 = 1.5;
pub const ROOF_TEXTURE_STEP: f32 = 0.1;

// Умолчание ползунка — внутри его же диапазона.
const _: () = {
    assert!(ROOF_TEXTURE_DEFAULT >= ROOF_TEXTURE_MIN && ROOF_TEXTURE_DEFAULT <= ROOF_TEXTURE_MAX);
};

/// Кровля дома глазами отрисовки: чем крыта, какого цвета и с какой рамкой
/// для шейдера.
#[derive(Clone)]
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
    /// поперёк, — и рамку каждой вершине даёт он (`layers::garages::garage_frame`).
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
    /// **возраст** кровли (`roof.wesl::roof_age` — хеш от посева: сколько на
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
/// переключение режима высот не перекрашивает город. Ось — из готового пятна
/// дома ([`HouseFootprint`]); `None` — пятна нет, она считается на месте, как
/// раньше своим `min_area_rect`.
pub(super) fn roof_look(building: &PolyArea, fp: Option<HouseFootprint>) -> RoofLook {
    let seed = look_seed(building);
    let class = classify(building, fp);
    let kind = kind_of(building, &class, seed);
    let palette = palette(building, &class, kind);
    let base = palette[(seed >> 8) as usize % palette.len()].to_srgba();
    // ±3 % яркости поверх выбранного цвета: два дома одной палитры и одного
    // слота всё-таки не близнецы. Цвет из разметки — как есть: его подбирали
    // по фотографии, и дрожать ему незачем
    let (base, jitter) = match temples::tagged_roof(building) {
        Some(tagged) => (tagged, 1.0),
        None => (base, value_jitter(seed >> 16)),
    };
    let axis = fp
        .or_else(|| HouseFootprint::of(building))
        .and_then(HouseFootprint::axis)
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

/// Кровля гаражного прогона: цвет и посев — **общие на весь прогон**. Именно
/// общий посев и делает из двадцати боксов одну ленту: иначе каждый красится
/// и ребрится сам по себе.
///
/// Рамка тут номинальная: у гаражной ленты координаты вершине приходят из её
/// собственных ячеек (`layers::garages::garage_frame`), а не из мировой точки и оси.
pub(super) fn run_look(run: &GarageRun) -> RoofLook {
    let base = GARAGE_ROW_COLORS[(run.seed >> 8) as usize % GARAGE_ROW_COLORS.len()].to_srgba();
    let jitter = value_jitter(run.seed >> 16);
    // материал — по самому большому куску: у буквы Г одно крыло может быть
    // кооперативом, а другое лентой, и на кровле это видно по каждому куску
    // отдельно (`layers::garages::garage_frame` берёт код у него), но `RoofKind` дома
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
    let jitter = value_jitter(run.seed >> 20);
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

/// Стена этого дома: материал по классу здания и посеву, цвет из палитры
/// материала.
///
/// Посев — тот же [`building_seed`], что у кровли, но разобранный **другими**
/// байтами: материал стены и материал кровли должны быть независимы, иначе
/// панельные дома окажутся ещё и все под одним битумом.
///
/// Рост, назначение и пятно уже разобраны классификатором
/// ([`super::classify`]) — развилка по этажам, которая раньше жила здесь
/// (`LOW_RISE_STOREYS`), теперь часть его гварда частного сектора, и стены
/// читают класс, а не дом второй раз. Пятно приходит готовым, как у
/// [`roof_look`]: без него классификатор для жилья считает `min_area_rect`
/// сам — та цена, которую пятновая сборка уже заплатила.
pub(super) fn wall_look(building: &PolyArea, fp: Option<HouseFootprint>) -> WallLook {
    let seed = look_seed(building);
    let class = classify(building, fp);
    let kind = wall_kind_of(&class, seed >> 4);
    let palette = wall_palette(building, &class, kind);
    let base = palette[(seed >> 12) as usize % palette.len()].to_srgba();
    // ±3 % яркости поверх выбранного цвета — как у кровель: два дома одной
    // палитры и одного слота всё-таки не близнецы; цвет из разметки — как есть
    let (base, jitter) = match temples::tagged_wall(building) {
        Some(tagged) => (tagged, 1.0),
        None => (base, value_jitter(seed >> 20)),
    };
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

/// Кровля храма в витрине кровель — православная палитра: фальц по вере
/// теперь выбирает [`temples::roof_palette`], и православная из них — та, что
/// стояла здесь раньше, зелёный металл.
pub const CHURCH_ROOF_COLORS: [Color; 5] = temples::ORTHODOX_ROOFS;

/// Посев дома — от его первой вершины ([`seed_from_point`]): материал кровли,
/// оборудование на ней и додуманная этажность держатся на одном числе, и оно
/// не зависит ни от порядка домов в выгрузке, ни от пересборки слоя.
pub(super) fn building_seed(building: &PolyArea) -> u32 {
    seed_from_point(building.outer.first().copied().unwrap_or(Vec2::ZERO))
}

/// Посев, которым дому выбираются материал и цвета. У части храма — посев
/// **храма** ([`crate::map::osm::Sacred::complex`]): барабаны, колокольня и
/// сам собор красятся одной палитрой, а не каждый своей. Форма, этажность и
/// оборудование остаются на собственном посеве дома ([`building_seed`]).
pub(super) fn look_seed(building: &PolyArea) -> u32 {
    match building.building_use {
        BuildingUse::Church(sacred) if sacred.complex != 0 => sacred.complex,
        _ => building_seed(building),
    }
}

/// Множитель ±3 % яркости поверх выбранного цвета: два дома одной палитры и
/// одного слота всё-таки не близнецы. Байт посева даёт вызывающий — у кровли,
/// стен и прогонов он свой, арифметика одна.
pub(super) fn value_jitter(byte: u32) -> f32 {
    1.0 + ((byte & 0xff) as f32 / 255.0 - 0.5) * 0.06
}

/// Параметры фактуры кровель — юниформ шейдера. Зеркало `RoofParams` в
/// `roof.wesl`: порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct RoofParams {
    /// Направление **на солнце** в плане (`map::sun_light`): по нему рёбра
    /// фальца и профлиста получают блик с одной стороны и тень с другой, а
    /// поперечное свету ребро видно сильнее продольного.
    pub light: Vec2,
    /// Общий множитель амплитуд — ползунок панели; ноль возвращает прежнюю
    /// плоскую заливку.
    pub intensity: f32,
    /// Видно ли оборудование кровли: `1` — да, `0` — вершинный шейдер
    /// схлопывает его треугольники в точку (метка — знак слота материала,
    /// `meshing::MeshBuilder::set_clutter`). Пишет [`show_roof_clutter`] по
    /// ступени зума; слой зданий от этого не пересобирается.
    pub clutter: f32,
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
        _pipeline: &Material2dPipeline,
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
    /// Параметры материала; оборудование кровли — видно (его прячет
    /// [`show_roof_clutter`], а ретюн ползунком сохраняет что есть).
    fn params(self) -> RoofParams {
        RoofParams {
            light: sun_light(),
            intensity: self.texture,
            clutter: 1.0,
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

/// Когда перетюнивать кровельный материал — **условие живёт рядом с системой**,
/// а не у того, кто ставит её в расписание: причина тут кровельная, и узнать её
/// надо, правя `material.rs`, а не `map/mod.rs`. Не `rebuilds_on`, как у слоёв:
/// пересобирать нечего, переписывается юниформ уже созданного материала.
///
/// `RoofStyle` — это ползунок Texture. `SunOnMap` в списке потому, что
/// [`RoofStyle::params`] кладёт в юниформ [`sun_light`] — глобаль осевшего
/// солнца, — и без этого условия блики фальца остались бы освещёнными с угла, с
/// которым грузился город, пока все остальные тени карты едут за осевшим
/// солнцем. Осевшим (`SunOnMap`), а не ползунком (`SunStyle`), — как везде на
/// карте: на шкале семьдесят делений.
///
/// **Условие одно, регистрация одна** (см. `crate::map::roads::rebuilds_on`).
pub fn retunes_on() -> impl SystemCondition<()> {
    retuned::<RoofStyle>.or_else(retuned::<SunOnMap>)
}

/// Правка ползунка Texture или осевшего солнца — новые параметры в материал;
/// меши не трогаются.
pub fn retune_roof_material(
    style: Res<RoofStyle>,
    handle: Res<RoofMaterialHandle>,
    mut materials: ResMut<Assets<RoofMaterial>>,
) {
    if let Some(mut material) = materials.get_mut(&handle.0) {
        // видимость оборудования — дело ступени зума, а не ползунка
        material.params = RoofParams {
            clutter: material.params.clutter,
            ..style.params()
        };
    }
}

/// Оборудование кровли видно на ступени `0` зданиевых ступеней зума
/// ([`super::BuildingLods`]) и спрятано дальше: шейдер схлопывает его
/// треугольники, а меш остаётся тем же.
///
/// Каждый кадр, а не по смене ступени: одно чтение ассета и сравнение, а
/// пишет система только на пересечении порога. Так же ей не нужен отдельный
/// вызов на входе в мир — ступень там сеется без флага изменения
/// (`zoom::seed_zoom_bucket`), и условие по смене её бы пропустило.
///
/// Раньше порог 0.5 м/px пересобирал весь слой экструзии — 535 тыс. ⇄ 1.18 млн
/// вершин на Туле (83 мс сборки, 20–47 МБ заливки), Калуга 106 мс — ради
/// коробок в метр, субпиксельных на этом зуме.
pub fn show_roof_clutter(
    bucket: Res<super::BuildingZoomBucket>,
    handle: Res<RoofMaterialHandle>,
    mut materials: ResMut<Assets<RoofMaterial>>,
) {
    let shown = if bucket.index == 0 { 1.0 } else { 0.0 };
    // `get` сначала: `get_mut` помечает ассет изменённым и перезаливает юниформ
    if materials
        .get(&handle.0)
        .is_some_and(|material| material.params.clutter != shown)
        && let Some(mut material) = materials.get_mut(&handle.0)
    {
        material.params.clutter = shown;
    }
}
