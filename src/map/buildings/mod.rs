//! Слои зданий и режимы отображения их высоты из OSM. Пять режимов,
//! переключаемых на лету (панель Buildings, `ui/buildings.rs`): фасадная
//! полоса (статус-кво), длинные тени как у деревьев, тени с тонировкой крыш
//! по высоте, 2.5D-экструзия в стиле watabou и всё разом. Правка
//! `BuildingHeightMode` пересобирает только зданиевые слои
//! (`rebuild_buildings`).
//!
//! Геометрия разнесена по подмодулям: [`arches`] режет проходы
//! `building_passage` сквозь стены, [`roofs`] ставит скатные крыши
//! (двускатные во всех видах, изредка вальмовые) на малые дома, [`layers`] собирает сами меши
//! слоёв, [`material`] решает, чем
//! крыша крыта, [`heights`] — сколько у него этажей, когда OSM молчит, — и
//! фактуру кровли рисует шейдер её материала. Храмы и крепость — не дома:
//! [`temples`] ставит над храмом главы, шпили и минареты по его вере,
//! [`fortress`] кроет башни кремля шатрами и снимает с его стен окна.

mod arches;
mod clutter;
mod fortress;
mod garages;
mod heights;
mod layers;
pub mod material;
mod order;
mod roofs;
mod temples;

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use bevy::color::Mix;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use self::garages::garage_runs;
use self::heights::{height_mix, height_or_default};
pub(crate) use self::layers::SHADOW_LENGTH_RANGE;
use self::layers::{
    ShadowSweeps, extrusion_builder, facade_and_roof_builders, roof_shadow_builder, shadow_builder,
};
pub use self::layers::{push_house, wall_of};
use self::material::RoofMaterialHandle;
use self::order::draw_order;
pub use self::roofs::{RoofShape, ShapeFacts, shape_facts};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, PolyArea, RoadLine};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SunOnMap, sun_light};
use crate::settings::{ROOF_CLUTTER_MAX_ZOOM, Z_BUILDING};

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
/// Границы подъёма крыши, м. Нижняя была 2.5 — то есть семь настоящих метров:
/// одноэтажный дом, сарай и гараж рисовались одной высоты с двухэтажкой, и
/// разница этажности частного сектора пропадала. Метр — это трёхметровая стена
/// одноэтажного дома, над которой встаёт уже крыша.
const EXTRUDE_RANGE: RangeInclusive<f32> = 1.0..=30.0;

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

    /// Рисуются ли в этом режиме длинные тени. Спрашивают двое — слой зданий
    /// и цилиндры промзоны (`map/industry.rs`), которые рисуются как дома, —
    /// и список режимов обязан быть один на обоих.
    pub(crate) fn casts_shadows(self) -> bool {
        matches!(
            self,
            Self::Shadows | Self::ShadowsTint | Self::ExtrusionShadowsTint
        )
    }
}

/// Зданиевый слой карты — чтобы пересборка режима знала, что деспавнить.
#[derive(Component)]
pub struct BuildingLayerTag;

/// Теневой слой — своя метка, потому что пересобирается он реже прочих: тени
/// зависят от режима высот и от `MapData`, но **не** от ступени зума, а их
/// объединение стоит 60–80 % всей сборки, смотря по режиму и ступени: доля тем
/// ниже, чем дороже сами фасады. Переход через порог оборудования на кровле их
/// не трогает.
///
/// Доля, а не миллисекунды: абсолютное время зависит от энергетического
/// состояния машины (App Nap), поэтому перемерять его надо бенчем —
/// `examples/bench/map_meshing`, — а не строкой `building meshing:` в логе.
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

/// Во что обошёлся один слой: имя, вершины, время сборки.
pub struct LayerCost {
    pub name: &'static str,
    pub vertices: usize,
    pub elapsed: Duration,
}

/// Сборка зданиевых слоёв **без мира и без ассетов** — для офлайн-замера
/// (`examples/bench/map_meshing.rs`).
///
/// Существует потому, что мерить сборку в живом приложении на macOS нельзя:
/// невидимому окну система урезает приоритет (App Nap), и те же 116 мс
/// показывают себя пятью секундами. Здесь нет ни окна, ни GPU — только те же
/// билдеры, что зовёт `spawn_buildings`.
///
/// Аргументы — ровно те два решения, которые замер читает: режим высот и
/// оборудование на кровле. [`BuildingPlan`] здесь не берётся: его третье поле,
/// `shadows`, для замера не значит ничего — теневой слой мерится всегда, когда
/// он у режима есть, потому что бенчу нужна полная цена сборки, — и
/// `shadows: false` молча печатал бы те же числа, что `true`.
pub fn measure_layers(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    mode: BuildingHeightMode,
    clutter: bool,
) -> Vec<LayerCost> {
    let detail = RoofDetail {
        tinted: matches!(
            mode,
            BuildingHeightMode::ShadowsTint | BuildingHeightMode::ExtrusionShadowsTint
        ),
        clutter,
    };
    let mut costs = Vec::new();

    // общие входы слоёв — порядок отрисовки и теневые развёртки: их строят
    // один раз на сборку и делят между сборщиками, поэтому у каждого свой
    // ряд, ровно как `breaks` и `parking` у слоя машин. Вершин они не дают:
    // это не меш, а вход
    let started = Instant::now();
    let order = if matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        let order = draw_order(buildings, Lean::of());
        costs.push(LayerCost {
            name: "order",
            vertices: 0,
            elapsed: started.elapsed(),
        });
        order
    } else {
        Vec::new()
    };
    let started = Instant::now();
    let sweeps = mode.casts_shadows().then(|| {
        let sweeps = ShadowSweeps::of(buildings);
        costs.push(LayerCost {
            name: "sweeps",
            vertices: 0,
            elapsed: started.elapsed(),
        });
        sweeps
    });

    // замеряется число вершин, а не сам сборщик: у плоского режима билдеров
    // два, и склеивать их ради замера значило бы мерить ещё и склейку
    let mut measure = |name, build: &mut dyn FnMut() -> usize| {
        let started = Instant::now();
        let vertices = build();
        costs.push(LayerCost {
            name,
            vertices,
            elapsed: started.elapsed(),
        });
    };

    match mode {
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint => {
            measure("extruded", &mut || {
                extrusion_builder(buildings, passages, detail, &order)
                    .0
                    .vertex_count()
            });
        }
        BuildingHeightMode::Facade
        | BuildingHeightMode::Shadows
        | BuildingHeightMode::ShadowsTint => {
            measure("facades+roofs", &mut || {
                let (facades, roofs) = facade_and_roof_builders(buildings, passages, detail);
                facades.vertex_count() + roofs.vertex_count()
            });
        }
    }
    // развёртки есть ровно у теневых режимов — тем же приёмом, каким порядок
    // отрисовки служит кровельному слою признаком 2.5D
    if let Some(sweeps) = &sweeps {
        measure("shadows", &mut || {
            shadow_builder(
                buildings,
                passages,
                sweeps,
                mode == BuildingHeightMode::ExtrusionShadowsTint,
            )
            .vertex_count()
        });
        // тени на кровлях — свой ряд, а не слагаемое в чужом: слой лежит над
        // зданиевыми, строится другим сборщиком и стоит своих миллисекунд,
        // причём `spawn_buildings` печатает их отдельно тем же образом
        measure("roof shadows", &mut || {
            roof_shadow_builder(
                buildings,
                sweeps,
                (mode == BuildingHeightMode::ExtrusionShadowsTint).then_some(order.as_slice()),
            )
            .vertex_count()
        });
    }
    costs
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

    // порядок отрисовки строится один раз на сборку слоя и достаётся обоим,
    // кому он нужен: мешу экструзии, который кладёт по нему дома, и теням на
    // кровлях, которые им же вычитают тела соседей, нарисованных после цели.
    // Второй вызов стоил на Туле 9 мс из 31, во столько обходились кровельные
    // тени; без него ряд даёт 20 (`examples/bench/map_meshing`, где порядок
    // печатается своим рядом `order`). Он же признак 2.5D — в плоских режимах
    // его никто не строит и накрывать кровлю соседа нечем
    let order = matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    )
    .then(|| draw_order(buildings, Lean::of()));

    // разброс форм крыш считает только слой экструзии — по тому, что легло в меш
    let mut roof_mix = String::from("counted in 2.5D only");
    match &order {
        Some(order) => {
            let detail = RoofDetail {
                tinted: mode == BuildingHeightMode::ExtrusionShadowsTint,
                clutter: bucket.index == 0,
            };
            let (extruded, mix) = extrusion_builder(buildings, passages, detail, order);
            roof_mix = mix.to_string();
            spawn_layer(
                commands,
                meshes,
                extruded,
                Z_BUILDING,
                "building_extruded",
                LayerMaterial::Roof(roof.handle()),
            );
        }
        None => {
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

    let mut sweep_time = Duration::ZERO;
    let mut shadow_time = Duration::ZERO;
    let mut roof_shadow_time = Duration::ZERO;
    if with_shadows && mode.casts_shadows() {
        // оба теневых слоя красит один полупрозрачный материал, и спавнит их
        // общий `surface::spawn_layer`: он сам отсеивает пустой сборщик,
        // вешает `DespawnOnExit` и `Name`. Не локальное замыкание
        // `spawn_layer` — у теней своя метка `BuildingShadowTag`
        let translucent = materials.add(ColorMaterial {
            alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
            ..default()
        });
        // развёртки — общие для обоих теневых слоёв: свип на цепочку силуэта
        // каждого дома, и раньше их строил каждый сборщик у себя. На кровлях
        // это была большая часть цены слоя
        let sweep_started = Instant::now();
        let sweeps = ShadowSweeps::of(buildings);
        sweep_time = sweep_started.elapsed();

        let shadow_started = Instant::now();
        let shadows = shadow_builder(
            buildings,
            passages,
            &sweeps,
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
        let on_roofs = roof_shadow_builder(buildings, &sweeps, order.as_deref());
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
        "building meshing: {vertices} verts in {:?} (shadows {sweep_time:?} sweeps + {shadow_time:?} on ground + {roof_shadow_time:?} on roofs, {} buildings, {} in garage rows, {}, clutter {}, heights: {}, roofs: {})",
        started.elapsed(),
        buildings.len(),
        garage_runs(buildings).len(),
        mode.label(),
        bucket.index == 0,
        height_mix(buildings),
        roof_mix,
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
    drawn_lift(height_or_default(building), mode)
}

/// Подъём верха над контуром для объекта высотой `height`: тот же масштаб и та
/// же обрезка, что у крыш, и ноль в режимах без экструзии.
///
/// Отдельно от [`extrusion_lift`], потому что кренится не только дом: тем же
/// правилом встают цилиндры промзоны (`map/industry.rs`), у которых нет ни
/// контура, ни `BuildingUse`. Обрезка [`EXTRUDE_RANGE`] тут и есть главное:
/// без неё шестидесятиметровая заводская труба ложилась на карту
/// восьмидесятиметровой трубой.
pub(crate) fn drawn_lift(height: f32, mode: BuildingHeightMode) -> Vec2 {
    if !matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        return Vec2::ZERO;
    }
    let drawn = (height * EXTRUDE_SCALE).clamp(*EXTRUDE_RANGE.start(), *EXTRUDE_RANGE.end());
    Lean::of().lift(drawn)
}

/// Тон поверхности по повороту её наружной нормали (в плане) к свету
/// `map::sun_light`: к свету — светлее базового на `lit_mix`, от света — темнее
/// на `shaded_mix`, в обоих случаях пропорционально косинусу. Одно правило
/// для стен и скатов. Смешивание — в sRGB, в котором заданы вся палитра и
/// рампа `roof_color`: одинаковая константа даёт одинаковый видимый шаг, а
/// `Srgba` в сигнатуре делает пространство явным.
pub(crate) fn shade_by_light(base: Srgba, outward: Vec2, lit_mix: f32, shaded_mix: f32) -> Srgba {
    let lit = outward.dot(sun_light());
    if lit >= 0.0 {
        base.mix(&Srgba::WHITE, lit * lit_mix)
    } else {
        base.mix(&Srgba::BLACK, -lit * shaded_mix)
    }
}

#[cfg(test)]
mod tests;
