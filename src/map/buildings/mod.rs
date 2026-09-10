//! Слои зданий и режимы отображения их высоты из OSM. Пять режимов,
//! переключаемых на лету (панель Buildings, `ui/buildings.rs`): фасадная
//! полоса (статус-кво), длинные тени как у деревьев, тени с тонировкой крыш
//! по высоте, 2.5D-экструзия в стиле watabou и всё разом. Правка
//! `BuildingHeightMode` пересобирает только зданиевые слои
//! (`rebuild_buildings`).
//!
//! Геометрия разнесена по подмодулям: [`arches`] режет проходы
//! `building_passage` сквозь стены, [`roofs`] ставит скатные крыши
//! (двускатные и вальмовые) на малые дома, [`layers`] собирает сами меши
//! слоёв, [`material`] решает, чем
//! крыша крыта, [`heights`] — сколько у него этажей, когда OSM молчит, — и
//! фактуру кровли рисует шейдер её материала.

mod arches;
mod clutter;
mod heights;
mod layers;
pub mod material;
mod roofs;

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use bevy::color::Mix;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use self::heights::{height_mix, height_or_default};
pub use self::layers::push_house;
use self::layers::{
    extrusion_builder, facade_and_roof_builders, roof_shadow_builder, shadow_builder,
};
use self::material::RoofMaterialHandle;
pub use self::roofs::{RoofShape, ShapeFacts, shape_facts};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{AreaKind, BuildingUse, MapData, PolyArea, RoadLine};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SunOnMap, sun_light};
use crate::settings::{ROOF_CLUTTER_MAX_ZOOM, Z_BUILDING};

/// Палитра **стен** по назначению: тёплые тона у жилья, серые у промзоны и
/// гаражей, охра у казённых зданий, белёный кирпич у храма.
///
/// Цвета крыш отсюда ушли в [`material`]: крыша теперь красится своим
/// материалом (битум, металл, черепица), а не назначением, и прежнее правило
/// «крыша светлее стены» вместе с ними. На снимке сверху ровно наоборот —
/// тёмный битумный ковёр на светлой панельной стене, и объём коробки держат
/// разные тона двух видимых стен, а не контраст с крышей.
const FACADE_COLOR: Color = Color::srgb(0.663, 0.616, 0.529);
const HOUSE_FACADE_COLOR: Color = Color::srgb(0.70, 0.60, 0.50);
const APARTMENTS_FACADE_COLOR: Color = Color::srgb(0.615, 0.575, 0.515);
const COMMERCIAL_FACADE_COLOR: Color = Color::srgb(0.575, 0.565, 0.545);
const INDUSTRIAL_FACADE_COLOR: Color = Color::srgb(0.505, 0.505, 0.49);
const GARAGE_FACADE_COLOR: Color = Color::srgb(0.48, 0.46, 0.43);
const CHURCH_FACADE_COLOR: Color = Color::srgb(0.93, 0.91, 0.86);
const PUBLIC_FACADE_COLOR: Color = Color::srgb(0.70, 0.62, 0.45);
const KREMLIN_ROOF_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);
const KREMLIN_FACADE_COLOR: Color = Color::srgb(0.42, 0.18, 0.15);

/// Фасады чуть ниже крыш: крыша соседа сверху прикрывает полосу — иначе
/// широкая полоса высотки залезала бы на низкого соседа.
const Z_FACADE: f32 = Z_BUILDING - 0.1;

/// Тени зданий на земле — под всеми зданиевыми слоями (фасады 4.9, крыши и
/// экструзия 5.0): крыша или стена соседа сама маскирует тень. Выше портала
/// (4) и трупов (3): они на улице и в тени по смыслу.
const Z_BUILDING_SHADOW: f32 = Z_BUILDING - 0.5;
/// Тени, падающие **на кровли** ([`layers::roof_shadow_builder`]), — наоборот,
/// над всеми зданиевыми слоями: это единственная часть тени, которая обязана
/// лежать поверх крыши, стен и оборудования на ней. Волосок над кровлей и всё
/// ещё ниже юнитов (10).
const Z_ROOF_SHADOW: f32 = Z_BUILDING + 0.05;

/// Метров подъёма крыши на метр высоты в 2.5D: драматичнее фасадной полосы,
/// но карта остаётся видом сверху, а не изометрией.
const EXTRUDE_SCALE: f32 = 0.35;
/// Косина подъёма: метров вправо на метр вверх. Строго вертикальный подъём
/// показывал одну южную стену, и дом читался как крыша с тёмной полосой
/// под ней. С косым сдвигом видны две стены — южная в тени и западная на
/// свету (свет из верхнего левого угла, как у теней) — и крыша: три тона,
/// из которых и складывается объём у watabou и в 3D-режиме 2GIS. Камера
/// при этом как бы смотрит из нижнего левого угла: дальние дома те, что
/// выше и правее.
const EXTRUDE_SKEW: f32 = 0.4;
/// Границы подъёма крыши, м.
const EXTRUDE_RANGE: RangeInclusive<f32> = 2.5..=30.0;

/// Режим отображения высоты зданий; переключается панелью Buildings и BRP,
/// сохраняется в настройках между запусками.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "buildings", key = "height_mode")]
pub enum BuildingHeightMode {
    /// Тёмная полоса фасада, сдвинутая вниз на долю высоты.
    Facade,
    /// Полоса фасада + длинная тень, длина пропорциональна высоте.
    Shadows,
    /// Тени + тонировка крыш по высоте: выше — темнее и глуше.
    ShadowsTint,
    /// 2.5D: крыша поднята на долю высоты, между контуром и крышей — стены.
    Extrusion,
    /// Всё разом: 2.5D-экструзия + тонировка крыш + длинные тени: город с
    /// объёмом читается как город, а плоский фасад — как чертёж.
    #[default]
    ExtrusionShadowsTint,
}

impl BuildingHeightMode {
    pub const ALL: [Self; 5] = [
        Self::Facade,
        Self::Shadows,
        Self::ShadowsTint,
        Self::Extrusion,
        Self::ExtrusionShadowsTint,
    ];

    /// Следующий по циклу — для кнопки-переключателя.
    pub fn next(self) -> Self {
        match self {
            Self::Facade => Self::Shadows,
            Self::Shadows => Self::ShadowsTint,
            Self::ShadowsTint => Self::Extrusion,
            Self::Extrusion => Self::ExtrusionShadowsTint,
            Self::ExtrusionShadowsTint => Self::Facade,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Facade => "Facade",
            Self::Shadows => "Shadows",
            Self::ShadowsTint => "Shadows+tint",
            Self::Extrusion => "2.5D",
            Self::ExtrusionShadowsTint => "2.5D+shadows+tint",
        }
    }
}

/// Зданиевый слой карты — чтобы пересборка режима знала, что деспавнить.
#[derive(Component)]
pub struct BuildingLayerTag;

/// Теневой слой — своя метка, потому что пересобирается он реже прочих: тени
/// зависят от режима высот и от `MapData`, но **не** от ступени зума, а их
/// объединение стоит 90 мс из 115 мс всей сборки. Переход через порог
/// оборудования на кровле их не трогает.
#[derive(Component)]
pub struct BuildingShadowTag;

/// Ступени детализации кровли — единственное, чем зум правит слой зданий:
/// вблизи на крышах стоит оборудование ([`clutter`]), дальше его нет. Коробка
/// в метр становится субпиксельной и мерцает при панораме, а нарисована она в
/// том же меше, что и дома, — снять её можно только пересборкой слоя, как
/// пересобирают себя путь и трамвай.
pub enum BuildingLods {}

impl ZoomLods for BuildingLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [ROOF_CLUTTER_MAX_ZOOM, f32::INFINITY].into_iter()
    }
}

/// Текущая ступень: `0` — с оборудованием, `1` — без.
pub type BuildingZoomBucket = ZoomBucket<BuildingLods>;

/// Что строить: режим высот, ступень зума и надо ли трогать теневой слой.
/// Одним значением, а не тремя параметрами, — так `spawn_buildings`
/// укладывается в семь аргументов, а вызывающий видит все три решения рядом.
#[derive(Clone, Copy)]
pub struct BuildingPlan {
    pub mode: BuildingHeightMode,
    pub bucket: BuildingZoomBucket,
    /// `false` — теневой слой оставить как есть (см. [`BuildingShadowTag`]).
    pub shadows: bool,
}

/// Что билдеры слоёв рисуют сверх геометрии. Оба флага — не про режим высот,
/// а про подробность, поэтому едут одним значением, а не парой булей в
/// каждой сигнатуре.
#[derive(Clone, Copy)]
pub(super) struct RoofDetail {
    /// Рампа тона крыш по высоте (режимы `*ShadowsTint`).
    pub(super) tinted: bool,
    /// Оборудование на кровле — по ступени зума.
    pub(super) clutter: bool,
}

/// Спавн зданиевых слоёв в выбранном режиме. Вызывается из `spawn_map` при
/// входе в мир и из `rebuild_buildings` при переключении режима.
pub fn spawn_buildings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    roof: &RoofMaterialHandle,
    plan: BuildingPlan,
    buildings: &[PolyArea],
    passages: &[RoadLine],
) {
    let BuildingPlan {
        mode,
        bucket,
        shadows: with_shadows,
    } = plan;
    // фасады и тени — плоский белый `ColorMaterial` под вершинные цвета;
    // всё, где есть крыша, идёт через `RoofMaterial` (у стен в том же меше
    // код материала нулевой, и фактуры они не получают)
    let started = Instant::now();
    let opaque = materials.add(Color::WHITE);
    let mut skipped = 0;
    let mut vertices = 0;
    let mut spawn_layer = |commands: &mut Commands,
                           meshes: &mut Assets<Mesh>,
                           builder: MeshBuilder,
                           z,
                           name,
                           material| {
        skipped += builder.skipped_polygons();
        vertices += builder.vertex_count();
        surface::spawn_layer(
            commands,
            meshes,
            builder,
            z,
            name,
            material,
            BuildingLayerTag,
        );
    };

    match mode {
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint => {
            let detail = RoofDetail {
                tinted: mode == BuildingHeightMode::ExtrusionShadowsTint,
                clutter: bucket.index == 0,
            };
            spawn_layer(
                commands,
                meshes,
                extrusion_builder(buildings, passages, detail),
                Z_BUILDING,
                "building_extruded",
                LayerMaterial::Roof(roof.handle()),
            );
        }
        BuildingHeightMode::Facade
        | BuildingHeightMode::Shadows
        | BuildingHeightMode::ShadowsTint => {
            let detail = RoofDetail {
                tinted: mode == BuildingHeightMode::ShadowsTint,
                clutter: bucket.index == 0,
            };
            let (facades, roofs) = facade_and_roof_builders(buildings, passages, detail);
            spawn_layer(
                commands,
                meshes,
                facades,
                Z_FACADE,
                "building_facades",
                LayerMaterial::Flat(opaque.clone()),
            );
            spawn_layer(
                commands,
                meshes,
                roofs,
                Z_BUILDING,
                "building_roofs",
                LayerMaterial::Roof(roof.handle()),
            );
        }
    }

    let mut shadow_time = Duration::ZERO;
    let mut roof_shadow_time = Duration::ZERO;
    if with_shadows
        && matches!(
            mode,
            BuildingHeightMode::Shadows
                | BuildingHeightMode::ShadowsTint
                | BuildingHeightMode::ExtrusionShadowsTint
        )
    {
        // оба теневых слоя красит один полупрозрачный материал, и спавнит их
        // общий `surface::spawn_layer`: он сам отсеивает пустой сборщик,
        // вешает `DespawnOnExit` и `Name`. Не локальное замыкание
        // `spawn_layer` — у теней своя метка `BuildingShadowTag`
        let translucent = materials.add(ColorMaterial {
            alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
            ..default()
        });
        let shadow_started = Instant::now();
        let shadows = shadow_builder(
            buildings,
            passages,
            mode == BuildingHeightMode::ExtrusionShadowsTint,
        );
        shadow_time = shadow_started.elapsed();
        vertices += shadows.vertex_count();
        surface::spawn_layer(
            commands,
            meshes,
            shadows,
            Z_BUILDING_SHADOW,
            "building_shadows",
            LayerMaterial::Flat(translucent.clone()),
            BuildingShadowTag,
        );

        // Тени на кровлях — **над** зданиевыми слоями, а не под ними: это
        // единственный кусок тени, который обязан лежать поверх крыши.
        // Своя метка не нужна — деспавнится он вместе с наземным слоем.
        let roof_started = Instant::now();
        let on_roofs =
            roof_shadow_builder(buildings, mode == BuildingHeightMode::ExtrusionShadowsTint);
        roof_shadow_time = roof_started.elapsed();
        vertices += on_roofs.vertex_count();
        surface::spawn_layer(
            commands,
            meshes,
            on_roofs,
            Z_ROOF_SHADOW,
            "roof_shadows",
            LayerMaterial::Flat(translucent),
            BuildingShadowTag,
        );
    }

    // тот же отчёт, что у дорог и путей: по нему видно, во что обошёлся
    // режим и сколько геометрии добавило оборудование кровель
    info!(
        "building meshing: {vertices} verts in {:?} (shadows {shadow_time:?} + {roof_shadow_time:?} on roofs, {} buildings, {}, clutter {}, heights: {})",
        started.elapsed(),
        buildings.len(),
        mode.label(),
        bucket.index == 0,
        height_mix(buildings),
    );
    if skipped > 0 {
        warn!("building meshing: {skipped} degenerate polygons skipped");
    }
}

/// Пересборка зданиевых слоёв после переключения режима из UI или BRP:
/// деспавн старых слоёв и повторный спавн из той же `MapData`.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_buildings(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    roof: Res<RoofMaterialHandle>,
    mode: Res<BuildingHeightMode>,
    sun: Res<SunOnMap>,
    bucket: Res<BuildingZoomBucket>,
    map: Res<MapData>,
    layers: Query<Entity, With<BuildingLayerTag>>,
    shadows: Query<Entity, With<BuildingShadowTag>>,
) {
    // ступень зума решает только судьбу оборудования на кровле; тени от неё
    // не зависят, а стоят дороже всего остального вместе взятого
    let with_shadows = mode.is_changed() || sun.is_changed();
    for entity in &layers {
        commands.entity(entity).despawn();
    }
    if with_shadows {
        for entity in &shadows {
            commands.entity(entity).despawn();
        }
    }
    spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        &roof,
        BuildingPlan {
            mode: *mode,
            bucket: *bucket,
            shadows: with_shadows,
        },
        &map.buildings,
        &map.roads,
    );
}

/// Отклонение верха **этого** дома от отвеса: единичное направление и метров
/// смещения на метр нарисованной высоты. Одно значение на дом — по нему
/// выбираются видимые стены, поднимается крыша, встаёт труба на коньке и
/// сортируется painter's порядок.
///
/// Сдвиг сейчас один и тот же у всех домов: так выглядит кадр со **спутника**
/// (сцена в 5 км с орбиты в 500 км занимает доли градуса, и отклонение по
/// кадру практически постоянно) и ортофотоплан. Лучевое отклонение от надира —
/// признак съёмки с самолёта — пробовали и убрали: надир прибит к центру
/// карты, а не кадра, поэтому при подвижной камере веер виден только вокруг
/// центра, а следовать за камерой он не может — пересборка слоя стоит десятки
/// миллисекунд. Вернуть его имеет смысл вместе с переносом сдвига в вершинный
/// шейдер.
#[derive(Clone, Copy)]
pub struct Lean {
    /// Смещение верха на метр нарисованной высоты. Вектором, а не парой
    /// «направление × длина»: у постоянного сдвига это ровно `(0.4, 1)`, и
    /// круг через `normalize`/`length` сдвинул бы его на единицу последнего
    /// разряда — что немедленно видно на обрезке `EXTRUDE_RANGE`.
    per_meter: Vec2,
}

impl Lean {
    /// Отклонение дома.
    pub(super) fn of() -> Self {
        Self {
            per_meter: Vec2::new(EXTRUDE_SKEW, 1.0),
        }
    }

    /// Единичное направление отклонения: по нему выбираются видимые стены.
    pub(super) fn dir(self) -> Vec2 {
        self.per_meter.try_normalize().unwrap_or(Vec2::Y)
    }

    /// Смещение верха для `drawn` нарисованных метров высоты.
    pub(super) fn lift(self, drawn: f32) -> Vec2 {
        self.per_meter * drawn
    }

    /// Сдвиг конька над карнизом для `rise` настоящих метров: тот же масштаб,
    /// что у стен, но без `EXTRUDE_RANGE` — обрезка держит стены в разумных
    /// пределах, а конёк и так ограничен `ROOF_RISE_MAX`.
    pub(super) fn ridge(self, rise: f32) -> Vec2 {
        self.lift(rise * EXTRUDE_SCALE)
    }

    /// Ключ painter's сортировки: больше — дальше, пишется раньше. «Дальше»
    /// при постоянном сдвиге — дальний конец вектора отклонения. Ключ берётся
    /// у **значения**, как и всё остальное здесь: в тот день, когда `of()`
    /// снова начнёт зависеть от дома, сортировка обязана поехать вместе с
    /// наклоном, а не остаться на дефолтном.
    pub(super) fn depth(self, at: Vec2) -> f32 {
        at.dot(self.dir())
    }
}

/// Центр контура — по нему считается отклонение и порядок отрисовки. Bounds,
/// а не центроид: сортировка и так была по ним, и лишний обход контура тут ни
/// к чему.
pub(super) fn building_center(building: &PolyArea) -> Vec2 {
    let (min, max) = crate::map::osm::model::ring_bounds(&building.outer);
    (min + max) * 0.5
}

/// На сколько в этом режиме поднята крыша относительно настоящего контура.
/// В режимах без экструзии — ноль. Одна точка входа на всех, кому нужен этот
/// сдвиг: слой экструзии, заплатка арки в тенях и всякий, кто захочет
/// поставить метку на нарисованный дом, а не на его настоящий контур.
pub fn extrusion_lift(building: &PolyArea, mode: BuildingHeightMode) -> Vec2 {
    if !matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        return Vec2::ZERO;
    }
    let height = (height_or_default(building) * EXTRUDE_SCALE)
        .clamp(*EXTRUDE_RANGE.start(), *EXTRUDE_RANGE.end());
    Lean::of().lift(height)
}

/// Базовый цвет стены по типу здания: Кремль — свой, остальные по назначению
/// (`BuildingUse`). Крыша красится не отсюда, а материалом ([`material`]).
fn facade_color(building: &PolyArea) -> Color {
    if building.kind == AreaKind::Kremlin {
        return KREMLIN_FACADE_COLOR;
    }
    match building.building_use {
        BuildingUse::House => HOUSE_FACADE_COLOR,
        BuildingUse::Apartments => APARTMENTS_FACADE_COLOR,
        BuildingUse::Commercial => COMMERCIAL_FACADE_COLOR,
        BuildingUse::Industrial => INDUSTRIAL_FACADE_COLOR,
        BuildingUse::Garage => GARAGE_FACADE_COLOR,
        BuildingUse::Church => CHURCH_FACADE_COLOR,
        BuildingUse::Public => PUBLIC_FACADE_COLOR,
        BuildingUse::Other => FACADE_COLOR,
    }
}

/// Тон поверхности по повороту её наружной нормали (в плане) к свету
/// `map::sun_light`: к свету — светлее базового на `lit_mix`, от света — темнее
/// на `shaded_mix`, в обоих случаях пропорционально косинусу. Одно правило
/// для стен и скатов. Смешивание — в sRGB, в котором заданы вся палитра и
/// рампа `roof_color`: одинаковая константа даёт одинаковый видимый шаг, а
/// `Srgba` в сигнатуре делает пространство явным.
pub(super) fn shade_by_light(base: Srgba, outward: Vec2, lit_mix: f32, shaded_mix: f32) -> Srgba {
    let lit = outward.dot(sun_light());
    if lit >= 0.0 {
        base.mix(&Srgba::WHITE, lit * lit_mix)
    } else {
        base.mix(&Srgba::BLACK, -lit * shaded_mix)
    }
}

#[cfg(test)]
mod tests;
