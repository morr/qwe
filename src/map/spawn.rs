//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (парки, луга, песок,
//! вода) + дороги, аллеи и стены (`map/roads.rs`, стиль ленты переключается
//! панелью Roads) + здания (`map/buildings/`, режим отображения высоты
//! переключается панелью Buildings) + деревья отдельными сущностями.

use bevy::prelude::*;

use crate::loading::AppState;
use crate::map::buildings::{self, BuildingHeightMode};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, TreeRow};
use crate::map::roads::{self, RoadStyle};
use crate::map::trees::TreeRowStyle;
use crate::settings::{
    MAP_SIZE, Z_GRASS, Z_GROUND, Z_PARK, Z_POND, Z_SAND, Z_TREE_ROW_BAND, Z_TREE_ROW_BAND_CASING,
    Z_WOOD,
};

const GROUND_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
const PARK_COLOR: Color = Color::srgb(0.769, 0.878, 0.580);
/// Лес внутри парка — темнее парковой подложки (osm-carto `#ADD19E`), под ним
/// и растут кроны; открытая часть парка так читается как поле.
const WOOD_COLOR: Color = Color::srgb(0.678, 0.820, 0.620);
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
    map: Res<MapData>,
    height_mode: Res<BuildingHeightMode>,
    road_style: Res<RoadStyle>,
) {
    commands.spawn((
        Sprite {
            color: GROUND_COLOR,
            custom_size: Some(MAP_SIZE),
            ..default()
        },
        Transform::from_translation((MAP_SIZE / 2.0).extend(Z_GROUND)),
        DespawnOnExit(AppState::Playing),
        Name::new("ground"),
    ));

    // вершинные цвета — материал один, белый
    let material = materials.add(Color::WHITE);

    let mut parks = MeshBuilder::default();
    for park in &map.parks {
        parks.push_polygon(&park.outer, &park.holes, PARK_COLOR.to_linear());
    }

    let mut woods = MeshBuilder::default();
    for area in &map.woods {
        woods.push_polygon(&area.outer, &area.holes, WOOD_COLOR.to_linear());
    }

    let mut grass = MeshBuilder::default();
    for area in &map.grass {
        grass.push_polygon(&area.outer, &area.holes, GRASS_COLOR.to_linear());
    }

    let mut sand = MeshBuilder::default();
    for area in &map.sand {
        sand.push_polygon(&area.outer, &area.holes, SAND_COLOR.to_linear());
    }

    let mut water = MeshBuilder::default();
    for area in &map.water {
        water.push_polygon(&area.outer, &area.holes, WATER_COLOR.to_linear());
    }

    let skipped: usize = [&parks, &woods, &grass, &sand, &water]
        .iter()
        .map(|builder| builder.skipped_polygons())
        .sum();
    if skipped > 0 {
        warn!("map meshing: {skipped} degenerate polygons skipped");
    }

    for (builder, z, name) in [
        (parks, Z_PARK, "parks"),
        (woods, Z_WOOD, "woods"),
        (grass, Z_GRASS, "grass"),
        (sand, Z_SAND, "sand"),
        (water, Z_POND, "water"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            Mesh2d(meshes.add(builder.build())),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, z),
            DespawnOnExit(AppState::Playing),
            Name::new(name),
        ));
    }

    roads::spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        *road_style,
        &map.roads,
        &map.rails,
        &map.walls,
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
    rows: &[TreeRow],
    style: &TreeRowStyle,
) {
    let mut casing = MeshBuilder::default();
    let mut fill = MeshBuilder::default();
    // тумблер панели выключает аллеи целиком: без деревьев ряда полоса под
    // ними — просто зелёная линия поперёк города
    let rows = if style.enabled { rows } else { &[] };
    for row in rows {
        // Chaikin тот же, что у дорог: ломаная из OSM на повороте даёт полосе
        // заметный угол, которого у лесного контура не бывает
        let path = roads::smooth_path(&row.points, TREE_ROW_BAND_WIDTH, style.smoothing);
        if style.casing {
            let width = TREE_ROW_BAND_WIDTH + 2.0 * roads::casing_width(TREE_ROW_BAND_WIDTH);
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

    let material = materials.add(Color::WHITE);
    for (builder, z, name) in [
        (casing, Z_TREE_ROW_BAND_CASING, "tree_row_band_casing"),
        (fill, Z_TREE_ROW_BAND, "tree_row_band"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            TreeRowBandTag,
            Mesh2d(meshes.add(builder.build())),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, z),
            DespawnOnExit(AppState::Playing),
            Name::new(name),
        ));
    }
}

/// Пересборка подложки аллей после правки её настроек из UI.
pub fn rebuild_tree_row_band(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
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
        &map.tree_rows,
        &style,
    );
}
