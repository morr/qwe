use bevy::prelude::*;

use crate::map::data::{
    self, Building, BuildingKind, PARK_PLAZA, PARK_PLAZA_RADIUS, POND_CENTER, POND_RADII, Road,
    RoadKind,
};
use crate::settings::{MAP_SIZE, Z_ALLEY, Z_BUILDING, Z_GROUND, Z_PARK, Z_POND, Z_ROAD, Z_TREE};

const GROUND_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
const PARK_COLOR: Color = Color::srgb(0.769, 0.878, 0.580);
const POND_COLOR: Color = Color::srgb(0.655, 0.804, 0.910);
const ROAD_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);

/// Высота тёмной полосы фасада — «псевдо-3D» низ здания.
const FACADE_HEIGHT: f32 = 3.0;

pub fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    spawn_rect(
        &mut commands,
        Vec2::ZERO,
        MAP_SIZE,
        GROUND_COLOR,
        Z_GROUND,
        "ground",
    );

    for (min, size) in data::PARK_RECTS {
        spawn_rect(&mut commands, *min, *size, PARK_COLOR, Z_PARK, "park");
    }

    // Пруд
    commands.spawn((
        Mesh2d(meshes.add(Ellipse::new(POND_RADII.x, POND_RADII.y))),
        MeshMaterial2d(materials.add(POND_COLOR)),
        Transform::from_translation(POND_CENTER.extend(Z_POND)),
        Name::new("pond"),
    ));

    // Парковая площадь
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(PARK_PLAZA_RADIUS))),
        MeshMaterial2d(materials.add(ALLEY_COLOR)),
        Transform::from_translation(PARK_PLAZA.extend(Z_ALLEY)),
        Name::new("park_plaza"),
    ));

    for road in data::ROADS {
        spawn_road(&mut commands, road);
    }

    for (index, building) in data::buildings().iter().enumerate() {
        spawn_building(&mut commands, building, index);
    }

    // Деревья: несколько общих мешей/материалов на все кроны
    let tree_mesh = meshes.add(Circle::new(1.0));
    let tree_materials: Vec<Handle<ColorMaterial>> = [
        Color::srgb(0.435, 0.682, 0.333),
        Color::srgb(0.478, 0.722, 0.369),
        Color::srgb(0.392, 0.633, 0.302),
    ]
    .into_iter()
    .map(|color| materials.add(color))
    .collect();

    for (index, (pos, radius)) in data::trees().into_iter().enumerate() {
        commands.spawn((
            Mesh2d(tree_mesh.clone()),
            MeshMaterial2d(tree_materials[index % tree_materials.len()].clone()),
            Transform::from_translation(pos.extend(Z_TREE)).with_scale(Vec3::splat(radius)),
            Name::new("tree"),
        ));
    }
}

fn spawn_rect(commands: &mut Commands, min: Vec2, size: Vec2, color: Color, z: f32, name: &str) {
    commands.spawn((
        Sprite {
            color,
            custom_size: Some(size),
            ..default()
        },
        Transform::from_translation((min + size / 2.0).extend(z)),
        Name::new(name.to_string()),
    ));
}

/// Дорога — повёрнутая полоса; длина продлена на ширину, чтобы стыки
/// перекрывались.
fn spawn_road(commands: &mut Commands, road: &Road) {
    let delta = road.to - road.from;
    let (color, z) = match road.kind {
        RoadKind::Street => (ROAD_COLOR, Z_ROAD),
        RoadKind::Alley => (ALLEY_COLOR, Z_ALLEY),
    };
    commands.spawn((
        Sprite {
            color,
            custom_size: Some(Vec2::new(delta.length() + road.width, road.width)),
            ..default()
        },
        Transform::from_translation(((road.from + road.to) / 2.0).extend(z))
            .with_rotation(Quat::from_rotation_z(delta.to_angle())),
        Name::new("road"),
    ));
}

/// Здание: крыша + тёмная полоса фасада по южной кромке (псевдо-3D).
fn spawn_building(commands: &mut Commands, building: &Building, index: usize) {
    let (roof_base, facade_color) = match building.kind {
        BuildingKind::Slab => (
            Color::srgb(0.949, 0.929, 0.878),
            Color::srgb(0.663, 0.616, 0.529),
        ),
        BuildingKind::House => (
            Color::srgb(0.894, 0.839, 0.722),
            Color::srgb(0.612, 0.557, 0.463),
        ),
    };
    // лёгкая вариация тона крыш, чтобы кварталы не сливались
    let tint = 1.0 - (index % 3) as f32 * 0.025;
    let roof_color = roof_base.to_srgba() * tint;

    let facade_height = FACADE_HEIGHT.min(building.size.y / 3.0);
    let facade_size = Vec2::new(building.size.x, facade_height);
    let roof_min = building.min + Vec2::new(0.0, facade_height);
    let roof_size = building.size - Vec2::new(0.0, facade_height);

    commands.spawn((
        Sprite {
            color: roof_color.into(),
            custom_size: Some(roof_size),
            ..default()
        },
        Transform::from_translation((roof_min + roof_size / 2.0).extend(Z_BUILDING)),
        Name::new("building_roof"),
    ));
    commands.spawn((
        Sprite {
            color: facade_color,
            custom_size: Some(facade_size),
            ..default()
        },
        Transform::from_translation((building.min + facade_size / 2.0).extend(Z_BUILDING)),
        Name::new("building_facade"),
    ));
}
