//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (парки, луга, песок,
//! вода) + дороги, аллеи и стены (`map/roads.rs`, стиль ленты переключается
//! панелью Roads) + здания (`map/buildings/`, режим отображения высоты
//! переключается панелью Buildings) + деревья отдельными сущностями.

use bevy::prelude::*;

use crate::loading::AppState;
use crate::map::buildings::{self, BuildingHeightMode};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::MapData;
use crate::map::roads::{self, RoadStyle};
use crate::settings::{MAP_SIZE, Z_GRASS, Z_GROUND, Z_PARK, Z_POND, Z_SAND, Z_WOOD};

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
