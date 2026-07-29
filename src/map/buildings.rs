//! Слои зданий и режимы отображения их высоты из OSM. Пять режимов,
//! переключаемых на лету (панель Buildings, `ui/buildings.rs`): фасадная
//! полоса (статус-кво), длинные тени как у деревьев, тени с тонировкой крыш
//! по высоте, 2.5D-экструзия в стиле watabou и всё разом. Правка
//! `BuildingHeightMode` пересобирает только зданиевые слои
//! (`rebuild_buildings`).

use std::collections::HashMap;
use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::loading::AppState;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{point_in_area, ring_bounds};
use crate::map::osm::{AreaKind, MapData, PolyArea, RoadLine};
use crate::settings::{ARCH_HEIGHT, Z_BUILDING};

const ROOF_COLOR: Color = Color::srgb(0.949, 0.929, 0.878);
const FACADE_COLOR: Color = Color::srgb(0.663, 0.616, 0.529);
const KREMLIN_ROOF_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);
const KREMLIN_FACADE_COLOR: Color = Color::srgb(0.42, 0.18, 0.15);

/// Высота здания без OSM-данных — пятиэтажка. Через `FACADE_SCALE` даёт
/// прежние 3 м фасадной полосы, так что режим Facade без высот не меняется.
const DEFAULT_BUILDING_HEIGHT: f32 = 15.0;
/// Доля реальной высоты, уходящая в полосу фасада. Рисовать все 60 м башни —
/// значит закрасить полквартала: карта сверху, а не изометрия. При 0.2
/// пятиэтажка (15 м) даёт прежние 3 м, и разница этажности всё равно читается.
const FACADE_SCALE: f32 = 0.2;
/// Границы полосы, м: сарай не должен потерять кромку, небоскрёб — накрыть
/// соседний квартал.
const FACADE_HEIGHT_RANGE: RangeInclusive<f32> = 1.5..=12.0;
/// Фасады чуть ниже крыш: крыша соседа сверху прикрывает полосу — иначе
/// широкая полоса высотки залезала бы на низкого соседа.
const Z_FACADE: f32 = Z_BUILDING - 0.1;

/// Направление тени — то же, что у крон деревьев (`trees::SHADOW_DIR`):
/// 30° вниз-вправо, нормировано. Один источник света на всю карту.
const SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);
/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`, как у крон.
const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);
/// Метров тени на метр высоты. Пятиэтажка (15 м) отбрасывает 9 м — тень
/// перечёркивает типичную улицу (8–16 м), но не глотает соседний квартал.
const SHADOW_LENGTH_SCALE: f32 = 0.6;
/// Границы длины тени, м: у сарая тень обязана остаться заметной, у башни —
/// не накрыть полкарты.
const SHADOW_LENGTH_RANGE: RangeInclusive<f32> = 3.0..=45.0;
/// Тени зданий — под всеми зданиевыми слоями (фасады 4.9, крыши и экструзия
/// 5.0): крыша или стена соседа сама маскирует тень, и тень никогда не
/// ложится на крышу дома той же высоты — дешёвая замена честному учёту
/// высот. Выше портала (4) и трупов (3): они на улице и в тени по смыслу.
const Z_BUILDING_SHADOW: f32 = Z_BUILDING - 0.5;

/// Высота, на которой рампа тона крыш выходит в максимум: Тула почти вся
/// ниже 75 м, и sqrt в формуле отдаёт разрешение диапазону 5–30 м.
const ROOF_TINT_MAX_HEIGHT: f32 = 60.0;
/// Цвет крыши «в пределе»: темнее и глуше базового, но всё ещё тёплый —
/// высотки читаются с общего плана, не превращая карту в теплокарту.
const ROOF_TALL_COLOR: Color = Color::srgb(0.71, 0.63, 0.55);
/// Насколько рампа может увести крышу к `ROOF_TALL_COLOR` в пределе.
const ROOF_TINT_MAX_MIX: f32 = 0.7;

/// Метров подъёма крыши на метр высоты в 2.5D: драматичнее фасадной полосы,
/// но карта остаётся видом сверху, а не изометрией.
const EXTRUDE_SCALE: f32 = 0.35;
/// Границы подъёма крыши, м.
const EXTRUDE_RANGE: RangeInclusive<f32> = 2.5..=30.0;
/// Осветление верхних вершин стены — дешёвый вертикальный градиент.
const WALL_TOP_LIGHTEN: f32 = 0.15;

/// Цвет проёма арки — та же подложка, что у земли в `spawn.rs`: сквозь арку
/// видно двор, а не стену.
const ARCH_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
/// Дальше скольких метров конец прохода не считается выходом на стену:
/// конец в OSM — общая вершина контура, так что реально там ноль; запас
/// покрывает шум проекции и слегка неровную разметку.
const ARCH_WALL_REACH: f32 = 6.0;
/// Насколько дальше ближайшей грани всё ещё «та же» стена, м: у общей вершины
/// двух граней обе на нулевом расстоянии, и проём обязан кроиться по обеим.
const ARCH_WALL_TIE: f32 = 0.5;

/// Режим отображения высоты зданий; переключается панелью Buildings и BRP,
/// сохраняется в настройках между запусками.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "buildings", key = "height_mode")]
pub enum BuildingHeightMode {
    /// Статус-кво: тёмная полоса фасада, сдвинутая вниз на долю высоты.
    #[default]
    Facade,
    /// Полоса фасада + длинная тень, длина пропорциональна высоте.
    Shadows,
    /// Тени + тонировка крыш по высоте: выше — темнее и глуше.
    ShadowsTint,
    /// 2.5D: крыша поднята на долю высоты, между контуром и крышей — стены.
    Extrusion,
    /// Всё разом: 2.5D-экструзия + тонировка крыш + длинные тени.
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

/// Спавн зданиевых слоёв в выбранном режиме. Вызывается из `spawn_map` при
/// входе в мир и из `rebuild_buildings` при переключении режима.
pub fn spawn_buildings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    mode: BuildingHeightMode,
    buildings: &[PolyArea],
    passages: &[RoadLine],
) {
    // вершинные цвета — материал один, белый; тени — свой blend-материал
    let opaque = materials.add(Color::WHITE);
    let mut skipped = 0;
    let mut spawn_layer =
        |commands: &mut Commands, meshes: &mut Assets<Mesh>, builder: MeshBuilder, z, name| {
            skipped += builder.skipped_polygons();
            if builder.is_empty() {
                return;
            }
            commands.spawn((
                BuildingLayerTag,
                Mesh2d(meshes.add(builder.build())),
                MeshMaterial2d(opaque.clone()),
                Transform::from_xyz(0.0, 0.0, z),
                DespawnOnExit(AppState::Playing),
                Name::new(name),
            ));
        };

    match mode {
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint => {
            let tinted = mode == BuildingHeightMode::ExtrusionShadowsTint;
            spawn_layer(
                commands,
                meshes,
                extrusion_builder(buildings, passages, tinted),
                Z_BUILDING,
                "building_extruded",
            );
        }
        BuildingHeightMode::Facade
        | BuildingHeightMode::Shadows
        | BuildingHeightMode::ShadowsTint => {
            let tinted = mode == BuildingHeightMode::ShadowsTint;
            let (facades, roofs) = facade_and_roof_builders(buildings, passages, tinted);
            spawn_layer(commands, meshes, facades, Z_FACADE, "building_facades");
            spawn_layer(commands, meshes, roofs, Z_BUILDING, "building_roofs");
        }
    }

    if matches!(
        mode,
        BuildingHeightMode::Shadows
            | BuildingHeightMode::ShadowsTint
            | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        let shadows = shadow_builder(
            buildings,
            passages,
            mode == BuildingHeightMode::ExtrusionShadowsTint,
        );
        if !shadows.is_empty() {
            commands.spawn((
                BuildingLayerTag,
                Mesh2d(meshes.add(shadows.build())),
                MeshMaterial2d(materials.add(ColorMaterial {
                    alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.0, Z_BUILDING_SHADOW),
                DespawnOnExit(AppState::Playing),
                Name::new("building_shadows"),
            ));
        }
    }

    if skipped > 0 {
        warn!("building meshing: {skipped} degenerate polygons skipped");
    }
}

/// Арки: `tunnel=building_passage` — это дорога, проложенная сквозь дом, и в
/// navmesh она уже прорезана коридором. Здание же рисуется сплошным, и пешки
/// идут сквозь нарисованную стену — карта врёт.
///
/// Проём лежит **в плоскости стены** и выровнен по грани контура. Ширина —
/// ширина самой дороги, спроецированная углом входа (|sin| между дорогой и
/// гранью: перпендикулярный вход — полная ширина, скользящий — почти ничего)
/// и подрезанная концами грани, чтобы у арки возле угла дома квад не повисал
/// в воздухе. Высота — [`ARCH_HEIGHT`] настоящих метров долей высоты
/// **этого** дома: `band × 6 / height`. Не через `EXTRUDE_SCALE` — подъём
/// обрезан `EXTRUDE_RANGE`, и у сарая или башни нарисованный метр стоит не
/// тех же 0.35 настоящих.
///
/// Стена ищется не пересечением дороги с контуром, а от **концов** прохода: в
/// OSM арку сплошь и рядом размечают отрезком от вершины контура до вершины
/// контура (арка 485488257 в Туле — ровно такая), то есть дорога лежит внутри
/// дома и стен касается только концами. Пересечения там нет вовсе, зато
/// каждый конец — и есть выход арки наружу.
struct ArchOpening {
    /// Грань, в которой прорезан проём.
    a: Vec2,
    b: Vec2,
    /// Интервал проёма вдоль грани, м от `a`.
    low: f32,
    high: f32,
    /// Вертикальный габарит проёма — доля полосы/подъёма режима.
    sill: Vec2,
}

fn arch_openings(building: &PolyArea, passages: &[&RoadLine], band: Vec2) -> Vec<ArchOpening> {
    if passages.is_empty() || band == Vec2::ZERO {
        return Vec::new();
    }
    // доля стены, которую занимает проём; у совсем низкого дома арка не
    // может быть выше него самого
    let sill = band * (ARCH_HEIGHT / height_or_default(building)).min(1.0);

    // видимые стены — те же грани, что рисует `extrusion_builder`
    let walls: Vec<(Vec2, Vec2)> = silhouette_edges(&building.outer, Vec2::NEG_Y)
        .into_iter()
        .chain(
            building
                .holes
                .iter()
                .flat_map(|hole| silhouette_edges(hole, Vec2::Y)),
        )
        .collect();

    let mut openings = Vec::new();
    for passage in passages {
        // конец прохода и направление, которым дорога входит в дом
        let ends = [
            passage.points.first().zip(passage.points.get(1)),
            passage
                .points
                .last()
                .zip(passage.points.iter().rev().nth(1)),
        ];
        for &(&point, &neighbour) in ends.iter().flatten() {
            let Some(direction) = (neighbour - point).try_normalize() else {
                continue;
            };
            // конец прохода в OSM — общая вершина контура, то есть точка
            // стыка ДВУХ граней: проём, зажатый в одну из них, обрезался бы
            // до половины ширины дороги. Кроим по всем граням в пределах
            // допуска от ближайшей — на стыке куски продолжают друг друга.
            let nearest = walls
                .iter()
                .map(|&(a, b)| point.distance(closest_on_segment(point, a, b)))
                .fold(f32::MAX, f32::min);
            if nearest > ARCH_WALL_REACH {
                continue;
            }
            for &(a, b) in &walls {
                let at = closest_on_segment(point, a, b);
                if point.distance(at) > nearest + ARCH_WALL_TIE {
                    continue;
                }
                let Some(along) = (b - a).try_normalize() else {
                    continue;
                };
                // дорога под углом к стене дырявит её уже собственной ширины
                let half = passage.width / 2.0 * direction.perp_dot(along).abs();

                let length = (b - a).length();
                let base = (at - a).dot(along);
                let (low, high) = ((base - half).max(0.0), (base + half).min(length));
                if high - low < 0.05 {
                    continue;
                }
                openings.push(ArchOpening {
                    a,
                    b,
                    low,
                    high,
                    sill,
                });
            }
        }
    }
    openings
}

/// Стена с проёмами: боковые куски во всю высоту и перемычка над каждой
/// аркой. Это **настоящий вырез** — в дыру просвечивают нижние слои (дорога,
/// проложенная сквозь дом, тень), а не закраска цветом земли.
fn push_wall_with_openings(
    builder: &mut MeshBuilder,
    a: Vec2,
    b: Vec2,
    lift: Vec2,
    openings: &[ArchOpening],
    bottom: LinearRgba,
    top: LinearRgba,
) {
    let mut cuts: Vec<&ArchOpening> = openings
        .iter()
        .filter(|opening| opening.a == a && opening.b == b)
        .collect();
    if cuts.is_empty() {
        builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
        return;
    }
    cuts.sort_by(|first, second| first.low.total_cmp(&second.low));

    let Some(along) = (b - a).try_normalize() else {
        return;
    };
    let length = (b - a).length();
    let piece = |builder: &mut MeshBuilder, from: f32, to: f32| {
        if to - from < 0.01 {
            return;
        }
        let (p0, p1) = (a + along * from, a + along * to);
        builder.push_quad_gradient([p0, p1, p1 + lift, p0 + lift], [bottom, bottom, top, top]);
    };

    let mut cursor = 0.0;
    for cut in cuts {
        piece(builder, cursor, cut.low);
        // перемычка над проёмом; цвет её низа — градиент стены на этой высоте
        let fraction = if lift.length_squared() > 0.0 {
            (cut.sill.length() / lift.length()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sill_color = bottom.mix(&top, fraction);
        let (p0, p1) = (a + along * cut.low, a + along * cut.high);
        builder.push_quad_gradient(
            [p0 + cut.sill, p1 + cut.sill, p1 + lift, p0 + lift],
            [sill_color, sill_color, top, top],
        );
        cursor = cut.high.max(cursor);
    }
    piece(builder, cursor, length);
}

/// Фасадные режимы: полоса фасада — один earcut-полигон на всё здание, и
/// честный паз в нём потребовал бы булевой операции над полигонами. Здесь
/// проём **закрашивается** цветом подложки — компромисс; настоящий вырез
/// живёт в 2.5D (`push_wall_with_openings`).
fn push_arches(builder: &mut MeshBuilder, building: &PolyArea, passages: &[&RoadLine], band: Vec2) {
    // проём затенён перемычкой над ним — подложка мешается с тоном тени
    let color = ARCH_COLOR
        .mix(&Color::srgb(0.22, 0.24, 0.33), SHADOW_COLOR.alpha())
        .to_linear();
    for opening in arch_openings(building, passages, band) {
        let Some(along) = (opening.b - opening.a).try_normalize() else {
            continue;
        };
        push_swept_quad(
            builder,
            [
                opening.a + along * opening.low,
                opening.a + along * opening.high,
            ],
            opening.sill,
            color,
        );
    }
}

/// Ближайшая точка отрезка — как `distance_to_segment`, но нужна сама точка.
fn closest_on_segment(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return from;
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    from + segment * t
}

/// Проходы, разложенные по домам, которые они прорезают: `building_passage`
/// размечают ровно тем куском дороги, что лежит под домом, поэтому дом
/// ищется по середине прохода.
fn arches_by_building<'a>(
    buildings: &[PolyArea],
    passages: &'a [RoadLine],
) -> HashMap<usize, Vec<&'a RoadLine>> {
    let mut by_building: HashMap<usize, Vec<&RoadLine>> = HashMap::new();
    for passage in passages.iter().filter(|road| road.passage) {
        let Some(middle) = passage_middle(passage) else {
            continue;
        };
        let pierced = buildings.iter().position(|building| {
            let (min, max) = ring_bounds(&building.outer);
            middle.x >= min.x
                && middle.x <= max.x
                && middle.y >= min.y
                && middle.y <= max.y
                && point_in_area(middle, building)
        });
        if let Some(index) = pierced {
            by_building.entry(index).or_default().push(passage);
        }
    }
    by_building
}

/// Середина ломаной по длине — устойчивее к неравномерным сегментам, чем
/// средняя точка списка.
fn passage_middle(passage: &RoadLine) -> Option<Vec2> {
    let total: f32 = passage
        .points
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum();
    if total <= 0.0 {
        return passage.points.first().copied();
    }
    let mut walked = 0.0;
    for segment in passage.points.windows(2) {
        let length = segment[0].distance(segment[1]);
        if walked + length >= total / 2.0 {
            let t = (total / 2.0 - walked) / length;
            return Some(segment[0].lerp(segment[1], t));
        }
        walked += length;
    }
    passage.points.last().copied()
}

/// Отрезок, протянутый вектором `sweep`, — прямоугольник проёма в стене.
fn push_swept_quad(builder: &mut MeshBuilder, edge: [Vec2; 2], sweep: Vec2, color: LinearRgba) {
    builder.push_polygon(
        &[edge[0], edge[1], edge[1] + sweep, edge[0] + sweep],
        &[],
        color,
    );
}

/// Пересборка зданиевых слоёв после переключения режима из UI или BRP:
/// деспавн старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_buildings(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mode: Res<BuildingHeightMode>,
    map: Res<MapData>,
    existing: Query<Entity, With<BuildingLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        *mode,
        &map.buildings,
        &map.roads,
    );
}

/// Высота здания с дефолтом — `None` в OSM это норма, а не ошибка.
fn height_or_default(building: &PolyArea) -> f32 {
    building.height.unwrap_or(DEFAULT_BUILDING_HEIGHT)
}

/// На сколько в этом режиме поднята крыша относительно настоящего контура.
/// В режимах без экструзии — ноль.
///
/// Публично, потому что оверлей дверей обязан повторять тот же сдвиг: дверь
/// живёт на настоящем контуре, а он в 2.5D уходит под нарисованный дом, и
/// метка на северной грани иначе читается как «дверь в середине здания».
pub fn extrusion_lift(building: &PolyArea, mode: BuildingHeightMode) -> Vec2 {
    if !matches!(
        mode,
        BuildingHeightMode::Extrusion | BuildingHeightMode::ExtrusionShadowsTint
    ) {
        return Vec2::ZERO;
    }
    let height = (height_or_default(building) * EXTRUDE_SCALE)
        .clamp(*EXTRUDE_RANGE.start(), *EXTRUDE_RANGE.end());
    Vec2::new(0.0, height)
}

/// Базовые цвета крыши и фасада по типу здания.
fn base_colors(building: &PolyArea) -> (Color, Color) {
    match building.kind {
        AreaKind::Kremlin => (KREMLIN_ROOF_COLOR, KREMLIN_FACADE_COLOR),
        _ => (ROOF_COLOR, FACADE_COLOR),
    }
}

/// Цвет крыши: базовый по типу, при `tinted` — рампа по высоте (Кремль и
/// здания без высоты рампу пропускают), поверх — лёгкая вариация тона по
/// индексу, чтобы кварталы не сливались.
fn roof_color(building: &PolyArea, index: usize, tinted: bool) -> LinearRgba {
    let (roof_base, _) = base_colors(building);
    let ramped = match building.height {
        Some(height) if tinted && building.kind != AreaKind::Kremlin => {
            let t = (height / ROOF_TINT_MAX_HEIGHT).clamp(0.0, 1.0).sqrt();
            roof_base.mix(&ROOF_TALL_COLOR, t * ROOF_TINT_MAX_MIX)
        }
        _ => roof_base,
    };
    let tint = 1.0 - (index % 3) as f32 * 0.025;
    LinearRgba::from(ramped.to_srgba() * tint)
}

/// Фасадная полоса + крыши (режимы Facade / Shadows / ShadowsTint).
fn facade_and_roof_builders(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    tinted: bool,
) -> (MeshBuilder, MeshBuilder) {
    let arches = arches_by_building(buildings, passages);
    let mut facades = MeshBuilder::default();
    let mut roofs = MeshBuilder::default();
    for (index, building) in buildings.iter().enumerate() {
        let (_, facade_color) = base_colors(building);

        // фасад — тот же контур, сдвинутый вниз: тёмная кромка видна
        // только вдоль южных граней любого полигона. Сдвиг — по высоте из
        // OSM, так что этажность города видна прямо на карте
        let facade_height = (height_or_default(building) * FACADE_SCALE)
            .clamp(*FACADE_HEIGHT_RANGE.start(), *FACADE_HEIGHT_RANGE.end());
        let offset = Vec2::new(0.0, -facade_height);
        let facade_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + offset).collect();
        let facade_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + offset).collect())
            .collect();
        facades.push_polygon(&facade_outer, &facade_holes, facade_color.to_linear());
        // крыши — отдельный слой поверх фасадов, так что вырезать проём из
        // полосы достаточно: над аркой крыша останется целой сама собой
        if let Some(passages) = arches.get(&index) {
            push_arches(&mut facades, building, passages, offset);
        }
        roofs.push_polygon(
            &building.outer,
            &building.holes,
            roof_color(building, index, tinted),
        );
    }
    (facades, roofs)
}

/// Тени зданий: на каждую непрерывную цепочку рёбер-силуэта внешнего кольца —
/// **один** свип-полигон `[цепочка, цепочка + сдвиг в обратном порядке]`.
/// Не квады на ребро: у ступенчатого фасада квады соседних ступеней
/// перекрываются вдоль тени, и полупрозрачность складывалась в полосы двойной
/// темноты. Свип цепочки самопересечься не может: перп-шаг ребра силуэта
/// равен `outward·SHADOW_DIR > 0`, то есть цепочка монотонна вдоль
/// перпендикуляра тени.
///
/// Затем **все** свипы карты объединяются булевым union (`i_overlay`) в набор
/// непересекающихся фигур с дырками: тени смежных корпусов и соседних зданий
/// перекрываются на земле, а любое наложение внутри одного полупрозрачного
/// слоя читается как пятно двойной темноты. После union альфа везде ровно
/// одна. Часть тени под зданиями закрывают их непрозрачные слои. Дыры (дворы)
/// пропускаются: их тень падает внутрь футпринта.
/// `extruded` — арки в 2.5D прорезаны по-настоящему, и сквозь дыру видна
/// голая дорога: без заплатки тени проём светится, хотя физически он затенён
/// перемычкой. Заплатка кладётся сюда, в теневой слой: он ниже зданий и
/// просвечивает ровно сквозь вырез.
fn shadow_builder(buildings: &[PolyArea], passages: &[RoadLine], extruded: bool) -> MeshBuilder {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let mut sweeps: Vec<Vec<[f32; 2]>> = Vec::new();
    for building in buildings {
        let length = (height_or_default(building) * SHADOW_LENGTH_SCALE)
            .clamp(*SHADOW_LENGTH_RANGE.start(), *SHADOW_LENGTH_RANGE.end());
        let offset = SHADOW_DIR * length;
        for chain in silhouette_chains(&building.outer, SHADOW_DIR) {
            let mut sweep: Vec<Vec2> = chain.clone();
            sweep.extend(chain.iter().rev().map(|&point| point + offset));
            // NonZero гасит контуры противоположного обхода — свипы обязаны
            // быть одинаково закручены, а обход source-колец OSM произволен
            if signed_area(&sweep) < 0.0 {
                sweep.reverse();
            }
            sweeps.push(sweep.into_iter().map(|point| [point.x, point.y]).collect());
        }
    }

    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    for shape in sweeps.simplify_shape(FillRule::NonZero) {
        let mut rings = shape.into_iter().map(|contour| {
            contour
                .into_iter()
                .map(Vec2::from_array)
                .collect::<Vec<Vec2>>()
        });
        let Some(outer) = rings.next() else {
            continue;
        };
        let holes: Vec<Vec<Vec2>> = rings.collect();
        builder.push_polygon(&outer, &holes, color);
    }

    if extruded {
        for (index, passages) in arches_by_building(buildings, passages) {
            let building = &buildings[index];
            let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
            for opening in arch_openings(building, &passages, lift) {
                let Some(along) = (opening.b - opening.a).try_normalize() else {
                    continue;
                };
                let (p0, p1) = (
                    opening.a + along * opening.low,
                    opening.a + along * opening.high,
                );
                builder.push_quad([p0, p1, p1 + opening.sill, p0 + opening.sill], color);
            }
        }
    }
    builder
}

/// Непрерывные (циклически) цепочки рёбер-силуэта кольца — рёбер, чья
/// наружная нормаль смотрит по `direction`. Обход начинается после
/// освещённого ребра, чтобы цепочка не рвалась на шве кольца.
fn silhouette_chains(ring: &[Vec2], direction: Vec2) -> Vec<Vec<Vec2>> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_area(ring).signum();
    let count = ring.len();
    let is_silhouette = |index: usize| {
        let edge = ring[(index + 1) % count] - ring[index];
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        outward.dot(direction) > 0.0
    };
    let Some(lit) = (0..count).find(|&index| !is_silhouette(index)) else {
        // у простого кольца все рёбра силуэтными быть не могут — кривой
        // контур OSM остаётся без тени, а не роняет карту
        return Vec::new();
    };

    let mut chains: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for step in 1..=count {
        let index = (lit + step) % count;
        if is_silhouette(index) {
            if current.is_empty() {
                current.push(ring[index]);
            }
            current.push(ring[(index + 1) % count]);
        } else if !current.is_empty() {
            chains.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chains.push(current);
    }
    chains
}

/// 2.5D-экструзия: painter's algorithm внутри одного меша — треугольники
/// растеризуются в порядке index-буфера, поэтому здания пишутся с севера на
/// юг (южное поверх), на здание сначала стены, потом крыша. Фасадной полосы
/// в этом режиме нет — её заменяют настоящие стены; `tinted` включает рампу
/// тона крыш, как в `ShadowsTint`.
fn extrusion_builder(buildings: &[PolyArea], passages: &[RoadLine], tinted: bool) -> MeshBuilder {
    let arches = arches_by_building(buildings, passages);
    let mut order: Vec<usize> = (0..buildings.len()).collect();
    order.sort_by(|&a, &b| {
        let center_y = |building: &PolyArea| {
            let (min, max) = ring_bounds(&building.outer);
            min.y + max.y
        };
        center_y(&buildings[b]).total_cmp(&center_y(&buildings[a]))
    });

    let mut builder = MeshBuilder::default();
    for index in order {
        let building = &buildings[index];
        let (_, facade_color) = base_colors(building);
        // через тот же хелпер, что и оверлей дверей, — иначе они разъедутся
        let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);

        let wall_bottom = facade_color.to_linear();
        let wall_top = facade_color
            .mix(&Color::WHITE, WALL_TOP_LIGHTEN)
            .to_linear();
        // арки вырезаются из стен по-настоящему: сквозь проём видны нижние
        // слои — дорога, идущая сквозь дом, и всё, что движок рисует под ней
        let openings = arches
            .get(&index)
            .map(|passages| arch_openings(building, passages, lift))
            .unwrap_or_default();
        // при сдвиге крыши строго вверх видимы только стены южных рёбер
        for (a, b) in silhouette_edges(&building.outer, Vec2::NEG_Y) {
            push_wall_with_openings(&mut builder, a, b, lift, &openings, wall_bottom, wall_top);
        }
        // двор: видима внутренняя стена его северной стороны — та, чья
        // наружная (для кольца дыры) нормаль смотрит вверх
        for hole in &building.holes {
            for (a, b) in silhouette_edges(hole, Vec2::Y) {
                push_wall_with_openings(&mut builder, a, b, lift, &openings, wall_bottom, wall_top);
            }
        }

        let roof_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + lift).collect();
        let roof_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + lift).collect())
            .collect();
        builder.push_polygon(
            &roof_outer,
            &roof_holes,
            roof_color(building, index, tinted),
        );
    }
    builder
}

/// Знаковая площадь кольца (shoelace): положительная — обход CCW.
/// `model::ring_area` абсолютная, для определения обхода не годится.
fn signed_area(ring: &[Vec2]) -> f32 {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += ring[j].perp_dot(ring[i]);
        j = i;
    }
    doubled / 2.0
}

/// Рёбра кольца, чья наружная нормаль смотрит по `direction` — силуэт с
/// подветренной стороны. Обход кольца (CW/CCW) учитывается по знаковой
/// площади, так что результат от него не зависит.
fn silhouette_edges(ring: &[Vec2], direction: Vec2) -> Vec<(Vec2, Vec2)> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_area(ring).signum();
    let mut edges = Vec::new();
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let edge = b - a;
        // для CCW-кольца наружная нормаль ребра — правый перпендикуляр
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        if outward.dot(direction) > 0.0 {
            edges.push((a, b));
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::RoadClass;

    fn square() -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ]
    }

    fn building(outer: Vec<Vec2>, height: Option<f32>, kind: AreaKind) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            height,
            entrances: Vec::new(),
        }
    }

    #[test]
    fn silhouette_picks_edges_facing_the_shadow() {
        // свет сверху-слева, тень вправо-вниз: силуэт — нижнее и правое рёбра
        let edges = silhouette_edges(&square(), SHADOW_DIR);
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|(a, b)| {
            let bottom = a.y == 0.0 && b.y == 0.0;
            let right = a.x == 10.0 && b.x == 10.0;
            bottom || right
        }));
    }

    #[test]
    fn silhouette_is_winding_independent() {
        let ccw = square();
        let cw: Vec<Vec2> = square().into_iter().rev().collect();
        let mut ccw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&ccw, SHADOW_DIR);
        let mut cw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&cw, SHADOW_DIR)
            .into_iter()
            .map(|(a, b)| (b, a))
            .collect();
        let key = |(a, b): &(Vec2, Vec2)| (a.x + a.y).min(b.x + b.y);
        ccw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
        cw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
        assert_eq!(ccw_edges.len(), 2);
        for (ccw_edge, cw_edge) in ccw_edges.iter().zip(&cw_edges) {
            let matches = (ccw_edge.0 == cw_edge.0 && ccw_edge.1 == cw_edge.1)
                || (ccw_edge.0 == cw_edge.1 && ccw_edge.1 == cw_edge.0);
            assert!(matches, "{ccw_edge:?} vs {cw_edge:?}");
        }
    }

    #[test]
    fn extrusion_walls_face_south_only() {
        // у квадрата при подъёме строго вверх видима одна южная стена
        let edges = silhouette_edges(&square(), Vec2::NEG_Y);
        assert_eq!(edges.len(), 1);
        let (a, b) = edges[0];
        assert_eq!(a.y, 0.0);
        assert_eq!(b.y, 0.0);
    }

    #[test]
    fn extrusion_sorts_north_first() {
        let north = building(
            square()
                .iter()
                .map(|p| *p + Vec2::new(0.0, 100.0))
                .collect(),
            Some(30.0),
            AreaKind::Building,
        );
        let south = building(square(), Some(3.0), AreaKind::Building);
        let positions = |list: &[PolyArea]| {
            extrusion_builder(list, &[], false)
                .build()
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap()
                .to_vec()
        };
        let sorted = positions(&[south.clone(), north.clone()]);
        let reversed = positions(&[north, south]);
        // порядок входа не важен: painter's sort всегда пишет север первым,
        // поэтому буферы вершин совпадают, а первая вершина — северная
        assert_eq!(sorted, reversed);
        assert!(
            sorted[0][1] >= 100.0,
            "north building must be written first"
        );
    }

    #[test]
    fn shadow_length_scales_with_height() {
        let low = building(square(), Some(6.0), AreaKind::Building);
        let high = building(square(), Some(60.0), AreaKind::Building);
        let reach = |list: &[PolyArea]| {
            let mesh = shadow_builder(list, &[], false).build();
            let positions = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap()
                .to_vec();
            positions
                .iter()
                .map(|p| Vec2::new(p[0], p[1]).dot(SHADOW_DIR))
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert!(reach(std::slice::from_ref(&high)) > reach(std::slice::from_ref(&low)) + 10.0);
    }

    #[test]
    fn every_mode_builds_geometry_for_mixed_input() {
        let mut with_hole = building(square(), Some(20.0), AreaKind::Building);
        with_hole.holes.push(vec![
            Vec2::new(4.0, 4.0),
            Vec2::new(6.0, 4.0),
            Vec2::new(6.0, 6.0),
            Vec2::new(4.0, 6.0),
        ]);
        let list = [
            with_hole,
            building(square(), None, AreaKind::Building),
            building(square(), Some(12.0), AreaKind::Kremlin),
        ];

        let (facades, roofs) = facade_and_roof_builders(&list, &[], true);
        assert!(!facades.is_empty());
        assert!(!roofs.is_empty());
        assert_eq!(facades.skipped_polygons(), 0);

        let shadows = shadow_builder(&list, &[], false);
        assert!(!shadows.is_empty());

        let extruded = extrusion_builder(&list, &[], false);
        assert!(!extruded.is_empty());
        assert_eq!(extruded.skipped_polygons(), 0);
        // комбинированный режим: рампа меняет цвета, но не геометрию
        let tinted = extrusion_builder(&list, &[], true);
        assert!(!tinted.is_empty());
        assert_eq!(tinted.skipped_polygons(), 0);
    }

    /// Сумма площадей треугольников меша — двойное наложение внутри тени
    /// давало бы сумму больше площади самой фигуры.
    fn mesh_area(mesh: &Mesh) -> f32 {
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        let indices: Vec<usize> = match mesh.indices().unwrap() {
            bevy::mesh::Indices::U32(list) => list.iter().map(|&i| i as usize).collect(),
            bevy::mesh::Indices::U16(list) => list.iter().map(|&i| i as usize).collect(),
        };
        indices
            .chunks_exact(3)
            .map(|triangle| {
                let point =
                    |i: usize| Vec2::new(positions[triangle[i]][0], positions[triangle[i]][1]);
                (point(1) - point(0)).perp_dot(point(2) - point(0)).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn square_shadow_is_one_swept_polygon() {
        let list = [building(square(), Some(15.0), AreaKind::Building)];
        let mesh = shadow_builder(&list, &[], false).build();
        // одна цепочка низ+право: свип из 6 вершин, без квадов на ребро
        assert_eq!(mesh.count_vertices(), 6);
    }

    #[test]
    fn staircase_shadow_has_no_double_darkening() {
        // ступенчатый юго-восточный фасад: раньше квады ступеней перекрывались
        // вдоль тени и полупрозрачность складывалась в полосы. Свип монотонной
        // цепочки покрывает ровно |сдвиг| × перп-протяжённость — без нахлёстов
        let staircase = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(6.0, 0.0),
            Vec2::new(6.0, 3.0),
            Vec2::new(9.0, 3.0),
            Vec2::new(9.0, 6.0),
            Vec2::new(12.0, 6.0),
            Vec2::new(12.0, 9.0),
            Vec2::new(0.0, 9.0),
        ];
        let chains = silhouette_chains(&staircase, SHADOW_DIR);
        assert_eq!(chains.len(), 1, "лестница — одна непрерывная цепочка");
        assert_eq!(chains[0].len(), 7);

        let list = [building(staircase, Some(20.0), AreaKind::Building)];
        let mesh = shadow_builder(&list, &[], false).build();
        let offset_length = 20.0 * SHADOW_LENGTH_SCALE;
        let perp_span = Vec2::new(12.0, 9.0).dot(SHADOW_DIR.perp());
        assert!((mesh_area(&mesh) - offset_length * perp_span).abs() < 0.5);
    }

    #[test]
    fn neighbour_shadows_union_without_double_darkening() {
        // два корпуса в ряд: тень левого дотягивается до правого, и без
        // union суммарная площадь меша была бы суммой двух свипов — с
        // перекрытием, читающимся как пятно двойной темноты
        let left = building(square(), Some(15.0), AreaKind::Building);
        let right = building(
            square().iter().map(|p| *p + Vec2::new(12.0, 0.0)).collect(),
            Some(15.0),
            AreaKind::Building,
        );
        let alone =
            |b: &PolyArea| mesh_area(&shadow_builder(std::slice::from_ref(b), &[], false).build());
        let separate = alone(&left) + alone(&right);
        let together = mesh_area(&shadow_builder(&[left, right], &[], false).build());
        assert!(
            together < separate - 1.0,
            "union must remove the overlap: {together} vs {separate}"
        );
    }

    #[test]
    fn roof_tint_darkens_tall_buildings_and_spares_the_kremlin() {
        let base = roof_color(&building(square(), None, AreaKind::Building), 0, true);
        let tall = roof_color(&building(square(), Some(60.0), AreaKind::Building), 0, true);
        assert!(tall.red < base.red);
        assert!(tall.green < base.green);

        let kremlin_flat = roof_color(&building(square(), Some(60.0), AreaKind::Kremlin), 0, true);
        let kremlin_base = roof_color(&building(square(), None, AreaKind::Kremlin), 0, false);
        assert_eq!(kremlin_flat, kremlin_base);
    }

    fn passage(points: Vec<Vec2>, passage: bool) -> RoadLine {
        RoadLine {
            points,
            width: 5.0,
            class: RoadClass::Street,
            bridge: false,
            passage,
        }
    }

    /// Проём режется только под дорогой с флагом `passage`, и только если
    /// она действительно идёт сквозь дом.
    #[test]
    fn only_a_building_passage_cuts_an_arch() {
        let house = vec![building(square(), Some(15.0), AreaKind::Building)];
        let through = vec![passage(
            vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
            true,
        )];
        let alongside = vec![passage(
            vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
            false,
        )];
        let elsewhere = vec![passage(
            vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 10.0)],
            true,
        )];

        let solid = extrusion_builder(&house, &[], false).vertex_count();
        assert!(extrusion_builder(&house, &through, false).vertex_count() > solid);
        assert_eq!(
            extrusion_builder(&house, &alongside, false).vertex_count(),
            solid
        );
        assert_eq!(
            extrusion_builder(&house, &elsewhere, false).vertex_count(),
            solid
        );
    }

    /// Высота проёма задана в настоящих метрах, а рисуется проекция:
    /// трёхметровая арка обязана занять ту же долю нарисованной стены, какую
    /// три метра занимают в настоящей высоте дома.
    #[test]
    fn an_arch_opening_is_three_real_metres_of_the_drawn_wall() {
        // 40 м высоты, подъём 14 м: арка обязана занять 14 × 3/40 = 1.05 м
        let tall = building(square(), Some(40.0), AreaKind::Building);
        let lift = extrusion_lift(&tall, BuildingHeightMode::Extrusion);
        let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);

        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &tall, &[&road], lift);

        let span = |pick: fn(&[f32; 3]) -> f32| {
            let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
            let low = values.iter().copied().fold(f32::INFINITY, f32::min);
            let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            high - low
        };

        let expected = lift.y * ARCH_HEIGHT / 40.0;
        assert!(
            (span(|p| p[1]) - expected).abs() < 0.01,
            "opening is {} m tall, expected {expected} m of a {} m wall",
            span(|p| p[1]),
            lift.y
        );
    }

    /// У низкого дома нарисованный метр стоит других настоящих метров:
    /// подъём обрезан `EXTRUDE_RANGE`, и пересчёт через `EXTRUDE_SCALE` дал бы
    /// не ту долю. Проверяем, что доля считается от высоты самого дома.
    #[test]
    fn a_clamped_wall_still_gets_a_proportional_opening() {
        // 4 м высоты: подъём 4 × 0.35 = 1.4 обрезается снизу до 2.5 м
        let low = building(square(), Some(4.0), AreaKind::Building);
        let lift = extrusion_lift(&low, BuildingHeightMode::Extrusion);
        assert_eq!(lift.y, *EXTRUDE_RANGE.start());

        let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);
        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &low, &[&road], lift);

        let heights: Vec<f32> = builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .collect();
        let opening = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - heights.iter().copied().fold(f32::INFINITY, f32::min);
        // арка выше самого дома (6 > 4) — проём режется по стене целиком
        assert!(
            (opening - lift.y).abs() < 0.01,
            "opening {opening} m, expected the whole {} m wall",
            lift.y
        );
    }

    /// Проём лежит в плоскости стены и шириной с проход, а не растянут вдоль
    /// дороги: у дороги, подходящей к южной грани под углом, вырез всё равно
    /// ровно по грани.
    #[test]
    fn an_arch_is_cut_along_the_wall_not_along_the_road() {
        let house = building(square(), Some(15.0), AreaKind::Building);
        let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
        // дорога идёт наискось и коротка: до стены дотягивается один конец
        let slanted = passage(vec![Vec2::new(4.0, -2.0), Vec2::new(9.0, 20.0)], true);

        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &house, &[&slanted], lift);

        let span = |pick: fn(&[f32; 3]) -> f32| {
            let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
            values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - values.iter().copied().fold(f32::INFINITY, f32::min)
        };
        // ширина дороги, спроецированная углом входа: наклонная дорога
        // дырявит стену уже собственной ширины
        let entry = (Vec2::new(9.0, 20.0) - Vec2::new(4.0, -2.0)).normalize();
        let expected = slanted.width * entry.perp_dot(Vec2::X).abs();
        assert!(
            (span(|p| p[0]) - expected).abs() < 0.01,
            "opening is {} m wide, expected {expected} m",
            span(|p| p[0])
        );
    }

    /// Регресс: конец прохода — общая вершина двух граней (как у любой
    /// OSM-арки). Зажатый в одну грань проём выходил вдвое уже дороги;
    /// теперь куски на обеих гранях продолжают друг друга.
    #[test]
    fn an_arch_at_a_shared_vertex_keeps_the_road_width() {
        // южная сторона из двух граней со стыком в (5, 0)
        let house = building(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(5.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(0.0, 10.0),
            ],
            Some(15.0),
            AreaKind::Building,
        );
        let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
        let road = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.0, 12.0)], true);

        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &house, &[&road], lift);

        let xs: Vec<f32> = builder
            .positions_for_test()
            .iter()
            .map(|position| position[0])
            .collect();
        let width = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - xs.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            width >= road.width * 0.9,
            "opening is {width} m wide against a {} m road",
            road.width
        );
    }

    /// Арка у самого угла дома: проём подрезается по концу грани, а не
    /// повисает половиной квада в воздухе за углом.
    #[test]
    fn an_arch_near_a_corner_is_trimmed_to_the_wall() {
        let house = building(square(), Some(15.0), AreaKind::Building);
        let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
        // дорога упирается в южную грань в метре от юго-западного угла
        let road = passage(vec![Vec2::new(1.0, 0.0), Vec2::new(1.0, 12.0)], true);

        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &house, &[&road], lift);
        assert!(!builder.is_empty());

        let xs: Vec<f32> = builder
            .positions_for_test()
            .iter()
            .map(|position| position[0])
            .collect();
        let west = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let east = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(west >= -0.01, "opening hangs past the corner: {west}");
        // а восточный край — там, куда дотянулась полуширина
        assert!((east - 3.5).abs() < 0.01, "{east}");
    }

    /// Регресс на реальную арку 485488257 (Тула): проход размечен отрезком
    /// **между двумя вершинами контура**, лежит внутри дома и стен касается
    /// только концами. Поиск пересечения дороги с контуром здесь не находит
    /// ничего — вырез обязан появиться от концов.
    #[test]
    fn an_arch_lying_inside_the_outline_still_cuts_an_opening() {
        // упрощённая геометрия того дома: южная грань y = 0, арка — отрезок
        // от вершины (5, 0) вглубь до вершины (5.2, 14) северной грани
        let house = building(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(5.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 14.0),
                Vec2::new(5.2, 14.0),
                Vec2::new(0.0, 14.0),
            ],
            Some(42.0),
            AreaKind::Building,
        );
        let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
        let inner = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.2, 14.0)], true);

        let mut builder = MeshBuilder::default();
        push_arches(&mut builder, &house, &[&inner], lift);
        assert!(
            !builder.is_empty(),
            "an outline-to-outline passage cut nothing"
        );
        // и проём сидит на южной грани — начинается на y = 0
        let bottom = builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::INFINITY, f32::min);
        assert!(bottom.abs() < 0.01, "opening floats at y = {bottom}");
    }

    /// Середина прохода берётся по длине, а не по числу точек: у ломаной с
    /// одним длинным и одним коротким сегментом это разные точки.
    #[test]
    fn the_passage_middle_is_measured_along_its_length() {
        let road = passage(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(90.0, 0.0),
                Vec2::new(100.0, 0.0),
            ],
            true,
        );
        let middle = passage_middle(&road).unwrap();
        assert!((middle.x - 50.0).abs() < 0.01, "{middle:?}");
    }
}
