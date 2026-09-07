//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (земля, парки, луга,
//! песок, вода площадная и линейная) + дороги, аллеи и стены (`map/roads.rs`,
//! стиль ленты переключается панелью Roads) + здания (`map/buildings/`, режим
//! отображения высоты переключается панелью Buildings) + деревья отдельными
//! сущностями. Поверхности красит фактурный материал (`map/surface.rs`):
//! цвет слоя по-прежнему вершинный, шум кладёт шейдер.

use bevy::prelude::*;

use crate::map::buildings::{self, BuildingHeightMode};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::{MapData, TreeRow, WaterLine, water_line_caps};
use crate::map::roads::{self, RoadSmoothing, RoadStyle};
use crate::map::surface::{LayerMaterial, SurfaceKind, SurfaceMaterials, spawn_layer};
use crate::map::trees::TreeRowStyle;
use crate::settings::{
    MAP_SIZE, Z_GRASS, Z_GROUND, Z_PARK, Z_POND, Z_SAND, Z_TREE_ROW_BAND, Z_TREE_ROW_BAND_CASING,
    Z_WATERWAY, Z_WOOD,
};

pub const GROUND_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
pub const PARK_COLOR: Color = Color::srgb(0.769, 0.878, 0.580);
/// Лес внутри парка — темнее парковой подложки (osm-carto `#ADD19E`), под ним
/// и растут кроны; открытая часть парка так читается как поле.
pub const WOOD_COLOR: Color = Color::srgb(0.678, 0.820, 0.620);
/// Ширина зелёной полосы под аллеей (`natural=tree_row`), м. Аллея — тот же лес,
/// только вытянутый в линию, поэтому и подложка у неё лесная: без неё ряд крон
/// висит на голом асфальте, тогда как каждое дерево в парке стоит на зелени.
///
/// Чуть уже полного вылета кроны (2 · 4 · `TREE_CROWN_REACH` = 12 м): кроны
/// должны свешиваться за край полосы, как свешиваются за контур лесного
/// полигона, иначе видно саму полосу, а не деревья на ней.
const TREE_ROW_BAND_WIDTH: f32 = 10.0;
/// Кант подложки аллеи — та же зелень на пару тонов темнее. У дорожного канта
/// роль «отделить полотно от фона», здесь — «показать край зарослей», поэтому
/// цвет берётся из семейства леса, а не серый, как у улицы.
const TREE_ROW_CASING_COLOR: Color = Color::srgb(0.565, 0.729, 0.510);
/// Луг — заметно светлее парка (`#DDEFBE`): поле без деревьев обязано читаться
/// поверх парковой заливки, иначе газон сливается с лесом.
const GRASS_COLOR: Color = Color::srgb(0.867, 0.937, 0.745);
/// Песок/пляж (osm-carto `#F5E9C6`).
const SAND_COLOR: Color = Color::srgb(0.961, 0.914, 0.776);
const WATER_COLOR: Color = Color::srgb(0.655, 0.804, 0.910);

pub fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    surfaces: Res<SurfaceMaterials>,
    map: Res<MapData>,
    height_mode: Res<BuildingHeightMode>,
    road_style: Res<RoadStyle>,
) {
    // земля — квад на всю карту тем же фактурным материалом, что и прочие
    // поверхности: спрайту с плоским цветом фактуру не положить
    let mut ground = MeshBuilder::with_surface_coords();
    ground.push_rect(Vec2::ZERO, MAP_SIZE, GROUND_COLOR.to_linear());

    let mut parks = MeshBuilder::with_surface_coords();
    for park in &map.parks {
        parks.push_polygon(&park.outer, &park.holes, PARK_COLOR.to_linear());
    }

    let mut woods = MeshBuilder::with_surface_coords();
    for area in &map.woods {
        woods.push_polygon(&area.outer, &area.holes, WOOD_COLOR.to_linear());
    }

    let mut grass = MeshBuilder::with_surface_coords();
    for area in &map.grass {
        grass.push_polygon(&area.outer, &area.holes, GRASS_COLOR.to_linear());
    }

    let mut sand = MeshBuilder::with_surface_coords();
    for area in &map.sand {
        sand.push_polygon(&area.outer, &area.holes, SAND_COLOR.to_linear());
    }

    let mut water = MeshBuilder::with_surface_coords();
    for area in &map.water {
        water.push_polygon(&area.outer, &area.holes, WATER_COLOR.to_linear());
    }

    let waterways = mesh_water_lines(&map.water_lines);

    let skipped: usize = [&parks, &woods, &grass, &sand, &water]
        .iter()
        .map(|builder| builder.skipped_polygons())
        .sum();
    if skipped > 0 {
        warn!("map meshing: {skipped} degenerate polygons skipped");
    }

    for (builder, z, name, kind) in [
        (ground, Z_GROUND, "ground", SurfaceKind::Ground),
        (parks, Z_PARK, "parks", SurfaceKind::Park),
        (woods, Z_WOOD, "woods", SurfaceKind::Wood),
        (grass, Z_GRASS, "grass", SurfaceKind::Grass),
        (sand, Z_SAND, "sand", SurfaceKind::Sand),
        (water, Z_POND, "water", SurfaceKind::Water),
        (waterways, Z_WATERWAY, "waterways", SurfaceKind::Water),
    ] {
        spawn_layer(
            &mut commands,
            &mut meshes,
            builder,
            z,
            name,
            LayerMaterial::Surface(surfaces.handle(kind)),
            (),
        );
    }

    roads::spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        &surfaces,
        *road_style,
        &map,
    );

    buildings::spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        *height_mode,
        &map.buildings,
        &map.roads,
    );
}

/// Лента открытых русел одним мешем. **Трубы не рисуются вовсе**: под землёй
/// воды не видно, а пунктир вдоль улицы читался как ручей поверх неё. Тем, что
/// человек проходит там, где на карте «ручей», управляет не эта отрисовка, а
/// её отсутствие: русло обрывается на портале культверта и продолжается за ним
/// (`water_line_caps`), и между порталами воды на карте просто нет.
fn mesh_water_lines(lines: &[WaterLine]) -> MeshBuilder {
    let color = WATER_COLOR.to_linear();
    let mut open = MeshBuilder::with_surface_coords();

    for line in lines.iter().filter(|line| !line.tunnel) {
        // сглаживание как у дорог: русло в OSM — ломаная по точкам съёмки, и на
        // её изломах лента без сглаживания заметно гранёная
        let points = roads::smooth_path(&line.points, line.width, RoadSmoothing::Light);
        // круглые стыки, и круглые торцы там, где вода продолжается: два way
        // одного русла встречаются в общем узле, и полудиски на торцах
        // сливаются в непрерывную реку. Портал культверта — исключение: за ним
        // воды нет, и полудиск торчал бы на полуширину русла в сухую землю
        let caps = water_line_caps(line, lines).map(|round| {
            if round {
                RibbonCap::Round
            } else {
                RibbonCap::Butt
            }
        });
        open.push_ribbon_capped(&points, false, line.width, color, RibbonJoin::Round, caps);
    }

    open
}

/// Зелёная полоса под аллеей — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
pub struct TreeRowBandTag;

/// Подложка аллей: лента лесного цвета вдоль каждого `natural=tree_row`, со
/// своими **тремя** ручками — стык, сглаживание и кант, — теми же самыми, что у
/// дорожных лент (`RoadJoin` / `RoadSmoothing` / `casing`), но своими: ломаная
/// аллеи и ломаная улицы приходят из разных данных, и подложка обязана выглядеть
/// лесом даже там, где дороги оставлены нетронутыми.
///
/// Отдельная сущность, а не часть слитого меша лесов, ровно потому, что эти
/// ручки переключаются на лету, а слой лесов собирается один раз на город и
/// пересобирать его на каждый клик незачем.
pub fn spawn_tree_row_band(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    surfaces: &SurfaceMaterials,
    rows: &[TreeRow],
    style: &TreeRowStyle,
) {
    let mut casing = MeshBuilder::default();
    // заливка — лесная поверхность, с той же фактурой, что лес под кронами
    let mut fill = MeshBuilder::with_surface_coords();
    // тумблер панели выключает аллеи целиком: без деревьев ряда полоса под
    // ними — просто зелёная линия поперёк города
    let rows = if style.enabled { rows } else { &[] };
    for row in rows {
        // Chaikin тот же, что у дорог: ломаная из OSM на повороте даёт полосе
        // заметный угол, которого у лесного контура не бывает
        let path = roads::smooth_path(&row.points, TREE_ROW_BAND_WIDTH, style.smoothing);
        if style.casing {
            let width = TREE_ROW_BAND_WIDTH
                + 2.0 * crate::map::footprint::casing_width(TREE_ROW_BAND_WIDTH);
            roads::push_ribbon(
                &mut casing,
                &path,
                width,
                TREE_ROW_CASING_COLOR.to_linear(),
                style.join,
            );
        }
        roads::push_ribbon(
            &mut fill,
            &path,
            TREE_ROW_BAND_WIDTH,
            WOOD_COLOR.to_linear(),
            style.join,
        );
    }

    let flat = materials.add(Color::WHITE);
    for (builder, z, name, material) in [
        (
            casing,
            Z_TREE_ROW_BAND_CASING,
            "tree_row_band_casing",
            LayerMaterial::Flat(flat),
        ),
        (
            fill,
            Z_TREE_ROW_BAND,
            "tree_row_band",
            LayerMaterial::Surface(surfaces.handle(SurfaceKind::Wood)),
        ),
    ] {
        spawn_layer(commands, meshes, builder, z, name, material, TreeRowBandTag);
    }
}

/// Пересборка подложки аллей после правки её настроек из UI.
pub fn rebuild_tree_row_band(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    surfaces: Res<SurfaceMaterials>,
    style: Res<TreeRowStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<TreeRowBandTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_tree_row_band(
        &mut commands,
        &mut meshes,
        &mut materials,
        &surfaces,
        &map.tree_rows,
        &style,
    );
}
