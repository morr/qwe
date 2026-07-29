//! Слои зданий и режимы отображения их высоты из OSM. Пять режимов,
//! переключаемых на лету (панель Buildings, `ui/buildings.rs`): фасадная
//! полоса (статус-кво), длинные тени как у деревьев, тени с тонировкой крыш
//! по высоте, 2.5D-экструзия в стиле watabou и всё разом. Правка
//! `BuildingHeightMode` пересобирает только зданиевые слои
//! (`rebuild_buildings`).

use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::loading::AppState;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::ring_bounds;
use crate::map::osm::{AreaKind, MapData, PolyArea};
use crate::settings::{Z_BUILDING, Z_TREE_SHADOW};

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
/// Тени зданий — как тени крон: над юнитами и крышами, под самими кронами.
/// Юнит, улица и крыша соседа в тени затемняются, как под деревом.
const Z_BUILDING_SHADOW: f32 = Z_TREE_SHADOW - 0.1;

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
                extrusion_builder(buildings, tinted),
                Z_BUILDING,
                "building_extruded",
            );
        }
        BuildingHeightMode::Facade
        | BuildingHeightMode::Shadows
        | BuildingHeightMode::ShadowsTint => {
            let tinted = mode == BuildingHeightMode::ShadowsTint;
            let (facades, roofs) = facade_and_roof_builders(buildings, tinted);
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
        let shadows = shadow_builder(buildings);
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
fn facade_and_roof_builders(buildings: &[PolyArea], tinted: bool) -> (MeshBuilder, MeshBuilder) {
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
        roofs.push_polygon(
            &building.outer,
            &building.holes,
            roof_color(building, index, tinted),
        );
    }
    (facades, roofs)
}

/// Тени зданий: на каждое ребро-силуэт внешнего кольца — квад, вытянутый по
/// `SHADOW_DIR` на длину, пропорциональную высоте. Часть тени под самим
/// зданием не рисуется — её всё равно закрывает непрозрачная крыша, а у
/// выпуклых контуров квады силуэта не перекрываются, так что полупрозрачная
/// тень не двоится внутри одного здания. Дыры (дворы) пропускаются: их тень
/// падает внутрь футпринта, под крышу.
fn shadow_builder(buildings: &[PolyArea]) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    for building in buildings {
        let length = (height_or_default(building) * SHADOW_LENGTH_SCALE)
            .clamp(*SHADOW_LENGTH_RANGE.start(), *SHADOW_LENGTH_RANGE.end());
        let offset = SHADOW_DIR * length;
        for (a, b) in silhouette_edges(&building.outer, SHADOW_DIR) {
            builder.push_quad([a, b, b + offset, a + offset], color);
        }
    }
    builder
}

/// 2.5D-экструзия: painter's algorithm внутри одного меша — треугольники
/// растеризуются в порядке index-буфера, поэтому здания пишутся с севера на
/// юг (южное поверх), на здание сначала стены, потом крыша. Фасадной полосы
/// в этом режиме нет — её заменяют настоящие стены; `tinted` включает рампу
/// тона крыш, как в `ShadowsTint`.
fn extrusion_builder(buildings: &[PolyArea], tinted: bool) -> MeshBuilder {
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
        // при сдвиге крыши строго вверх видимы только стены южных рёбер
        for (a, b) in silhouette_edges(&building.outer, Vec2::NEG_Y) {
            builder.push_quad_gradient(
                [a, b, b + lift, a + lift],
                [wall_bottom, wall_bottom, wall_top, wall_top],
            );
        }
        // двор: видима внутренняя стена его северной стороны — та, чья
        // наружная (для кольца дыры) нормаль смотрит вверх
        for hole in &building.holes {
            for (a, b) in silhouette_edges(hole, Vec2::Y) {
                builder.push_quad_gradient(
                    [a, b, b + lift, a + lift],
                    [wall_bottom, wall_bottom, wall_top, wall_top],
                );
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
            extrusion_builder(list, false)
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
            let mesh = shadow_builder(list).build();
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

        let (facades, roofs) = facade_and_roof_builders(&list, true);
        assert!(!facades.is_empty());
        assert!(!roofs.is_empty());
        assert_eq!(facades.skipped_polygons(), 0);

        let shadows = shadow_builder(&list);
        assert!(!shadows.is_empty());

        let extruded = extrusion_builder(&list, false);
        assert!(!extruded.is_empty());
        assert_eq!(extruded.skipped_polygons(), 0);
        // комбинированный режим: рампа меняет цвета, но не геометрию
        let tinted = extrusion_builder(&list, true);
        assert!(!tinted.is_empty());
        assert_eq!(tinted.skipped_polygons(), 0);
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
}
