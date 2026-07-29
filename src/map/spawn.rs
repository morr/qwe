//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (парки, луга, песок,
//! вода, аллеи, улицы, фасады, крыши, стены) + деревья отдельными сущностями.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{AreaKind, MapData, RoadClass};
use crate::map::trees::{self, TreeStyle};
use crate::settings::{
    MAP_SIZE, Z_ALLEY, Z_BUILDING, Z_GRASS, Z_GROUND, Z_PARK, Z_POND, Z_ROAD, Z_SAND, Z_WOOD,
};

const GROUND_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
const PARK_COLOR: Color = Color::srgb(0.769, 0.878, 0.580);
/// Лес внутри парка — темнее парковой подложки (osm-carto `#ADD19E`), под ним
/// и растут кроны; открытая часть парка так читается как поле.
const WOOD_COLOR: Color = Color::srgb(0.678, 0.820, 0.620);
/// Луг — заметно светлее парка (`#DDEFBE`): поле без деревьев обязано читаться
/// поверх парковой заливки, иначе газон сливается с лесом.
const GRASS_COLOR: Color = Color::srgb(0.867, 0.937, 0.745);
/// Песок/пляж (osm-carto `#F5E9C6`).
const SAND_COLOR: Color = Color::srgb(0.961, 0.914, 0.776);
const WATER_COLOR: Color = Color::srgb(0.655, 0.804, 0.910);
const ROAD_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);

const ROOF_COLOR: Color = Color::srgb(0.949, 0.929, 0.878);
const FACADE_COLOR: Color = Color::srgb(0.663, 0.616, 0.529);
const KREMLIN_ROOF_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);
const KREMLIN_FACADE_COLOR: Color = Color::srgb(0.42, 0.18, 0.15);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Высота тёмной полосы фасада — «псевдо-3D» низ здания.
const FACADE_HEIGHT: f32 = 3.0;
/// Фасады чуть ниже крыш: крыша соседа сверху прикрывает полосу.
const Z_FACADE: f32 = Z_BUILDING - 0.1;
/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

pub fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
) {
    commands.spawn((
        Sprite {
            color: GROUND_COLOR,
            custom_size: Some(MAP_SIZE),
            ..default()
        },
        Transform::from_translation((MAP_SIZE / 2.0).extend(Z_GROUND)),
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

    let mut alleys = MeshBuilder::default();
    let mut roads = MeshBuilder::default();
    for road in &map.roads {
        let (builder, color) = match road.class {
            RoadClass::Street => (&mut roads, ROAD_COLOR),
            RoadClass::Alley => (&mut alleys, ALLEY_COLOR),
        };
        builder.push_polyline(&road.points, road.width, color.to_linear());
    }

    let mut facades = MeshBuilder::default();
    let mut roofs = MeshBuilder::default();
    for (index, building) in map.buildings.iter().enumerate() {
        let (roof_base, facade_color) = match building.kind {
            AreaKind::Kremlin => (KREMLIN_ROOF_COLOR, KREMLIN_FACADE_COLOR),
            _ => (ROOF_COLOR, FACADE_COLOR),
        };
        // лёгкая вариация тона крыш, чтобы кварталы не сливались
        let tint = 1.0 - (index % 3) as f32 * 0.025;
        let roof_color = LinearRgba::from(roof_base.to_srgba() * tint);

        // фасад — тот же контур, сдвинутый вниз: тёмная кромка видна
        // только вдоль южных граней любого полигона
        let offset = Vec2::new(0.0, -FACADE_HEIGHT);
        let facade_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + offset).collect();
        let facade_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + offset).collect())
            .collect();
        facades.push_polygon(&facade_outer, &facade_holes, facade_color.to_linear());
        roofs.push_polygon(&building.outer, &building.holes, roof_color);
    }

    let mut walls = MeshBuilder::default();
    for wall in &map.walls {
        walls.push_polyline(&wall.points, wall.width, WALL_COLOR.to_linear());
    }

    let skipped: usize = [&parks, &woods, &grass, &sand, &water, &facades, &roofs]
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
        (alleys, Z_ALLEY, "alleys"),
        (roads, Z_ROAD, "roads"),
        (facades, Z_FACADE, "building_facades"),
        (roofs, Z_BUILDING, "building_roofs"),
        (walls, Z_WALL, "walls"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            Mesh2d(meshes.add(builder.build())),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, z),
            Name::new(name),
        ));
    }

    trees::spawn_trees(
        &mut commands,
        &mut meshes,
        &mut materials,
        &style,
        &map.trees,
    );
}
