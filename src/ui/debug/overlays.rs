//! Мировые слои дебаг-панели: то, что она рисует НА КАРТЕ, а не в UI —
//! сетка навтайлов и двери гизмо-линиями, заливка непроходимости и поле
//! хвои отдельными мешами.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

use super::{DebugConiferNoise, DebugNavmesh};
use crate::camera::Viewport;
use crate::grid::tile_center;
use crate::loading::AppState;
use crate::map::ConiferField;
use crate::map::osm::MapData;
use crate::navigation::{ArcNavmesh, PolymeshDebug};
use crate::settings::{MAP_SIZE, Z_CONIFER_NOISE_OVERLAY, grid_size, navtile_size};

#[derive(Component)]
pub(super) struct NavmeshOverlayMarker;

/// Слой поля хвои и то, под что он нарисован: порог и поколение поля.
/// Пересобирать текстуру, пока оба те же, незачем — правка любого другого поля
/// `TreeStyle` (тот же ползунок плотности) иначе перерисовывала бы её на
/// каждом шаге. Одного порога мало: правка параметров шума (панель Noise)
/// меняет рельеф поля, а порог-квантиль при этом может совпасть числом.
#[derive(Component)]
pub(super) struct ConiferNoiseOverlayMarker {
    threshold: f32,
    generation: u32,
}

/// Z заливки navmesh: над зданиями (5.0), под юнитами (5.5+).
const NAVMESH_OVERLAY_Z: f32 = 5.2;

/// Сторона текстуры поля хвои, тексели: 512 на 5.6 км карты — тексель ~11 м,
/// вдвое мельче кроны, то есть контур массива читается точно, а пересборка
/// слоя остаётся четвертью миллиона выборок, а не миллионами.
const CONIFER_NOISE_OVERLAY_PX: u32 = 512;
/// Прозрачность поля: под порогом слой только подсвечивает рельеф шума и не
/// должен скрывать лес, над порогом — показывает будущий массив, и его видно.
const CONIFER_NOISE_ALPHA: f32 = 0.30;
const CONIFER_STAND_ALPHA: f32 = 0.55;
/// Цвет хвойной области — холодная зелень ели, а не листвы.
const CONIFER_STAND_COLOR: Vec3 = Vec3::new(0.15, 0.90, 0.35);

/// Метка двери, м: с высоты, на которой видно квартал, кружок меньше метра
/// уже не читается.
const DOOR_MARKER_RADIUS: f32 = 1.5;
const DOOR_COLOR: Color = Color::srgba(0.15, 0.85, 1.0, 0.9);
/// Сколько экранов вокруг камеры рисовать двери. Как и у movepath: на всю
/// карту это десять тысяч гизмо за кадр, а за соседним экраном их не видно.
const DOORS_VIEW_SCREENS: f32 = 1.5;

/// Сетка navtiles гизмо-линиями по краям тайлов.
pub(super) fn render_grid(mut gizmos: Gizmos) {
    let color = Color::srgba(0.2, 0.2, 0.2, 0.3);
    let (grid_size, tile_size) = (grid_size(), navtile_size());
    for x in 0..=grid_size.x {
        let world_x = x as f32 * tile_size;
        gizmos.line_2d(
            Vec2::new(world_x, 0.0),
            Vec2::new(world_x, MAP_SIZE.y),
            color,
        );
    }
    for y in 0..=grid_size.y {
        let world_y = y as f32 * tile_size;
        gizmos.line_2d(
            Vec2::new(0.0, world_y),
            Vec2::new(MAP_SIZE.x, world_y),
            color,
        );
    }
}

/// Входы в здания — кружок на каждую дверь. Как и у movepath, рисуется
/// гизмо каждый кадр и отсекается по вьюпорту: дверей на карте под десять
/// тысяч, и гизмо на всю карту разом кладёт кадр.
pub(super) fn render_doors(
    map: Res<MapData>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut gizmos: Gizmos,
) {
    let view = Viewport::of(&window, &camera, DOORS_VIEW_SCREENS);

    for building in &map.buildings {
        for &door in &building.entrances {
            if !view.contains(door) {
                continue;
            }
            gizmos.circle_2d(door, DOOR_MARKER_RADIUS, DOOR_COLOR);
        }
    }
}

/// Спавн/despawn заливки непроходимых тайлов при переключении тумблера.
/// Один слитый меш на все тайлы: на OSM-карте их сотни тысяч, отдельные
/// entity на каждый укладывали кадр.
pub(super) fn sync_navmesh_overlay(
    mut commands: Commands,
    navmesh_show: Res<DebugNavmesh>,
    polymesh: Res<PolymeshDebug>,
    arc_navmesh: Res<ArcNavmesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    overlay: Query<Entity, With<NavmeshOverlayMarker>>,
) {
    if polymesh.enabled || !navmesh_show.0 {
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        return;
    }

    let color = Color::srgba(0.9, 0.15, 0.15, 0.35).to_linear();
    let mut builder = crate::map::MeshBuilder::default();
    let navmesh = arc_navmesh.read();
    for x in 0..navmesh.grid_size.x {
        for y in 0..navmesh.grid_size.y {
            if navmesh.is_passable(x, y) {
                continue;
            }
            let center = tile_center(IVec2::new(x, y));
            builder.push_rect(
                center - navmesh.tile_size / 2.0,
                center + navmesh.tile_size / 2.0,
                color,
            );
        }
    }
    if builder.is_empty() {
        return;
    }

    commands.spawn((
        NavmeshOverlayMarker,
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(materials.add(ColorMaterial {
            alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, NAVMESH_OVERLAY_Z),
        DespawnOnExit(AppState::Playing),
        Name::new("navmesh_overlay"),
    ));
}

/// Спавн/despawn слоя поля хвои. Слой — один спрайт на всю карту с текстурой,
/// посчитанной на CPU: заливать поле мешем в четверть миллиона квадов незачем,
/// оно и так гладкое.
///
/// Серая рампа — само значение поля, зелёным залито всё, что не ниже текущего
/// порога, то есть **будущий хвойный массив**: так видно и рельеф шума, и что
/// из него отберёт ползунок доли. Зелень покрывает и застройку — поле
/// определено на всей карте, а деревья стоят только в лесах.
///
/// Слой рисует поле **без примеси** (`TreeStyle::noise_mix` живёт только в
/// деревьях): при mix > 0 одиночные кроны намеренно стоят «не с той стороны»
/// зелёной границы — это вкрапления, а не рассинхрон слоя.
pub(super) fn sync_conifer_noise_overlay(
    mut commands: Commands,
    enabled: Res<DebugConiferNoise>,
    field: Res<ConiferField>,
    mut images: ResMut<Assets<Image>>,
    overlay: Query<(Entity, &ConiferNoiseOverlayMarker)>,
) {
    let threshold = field.threshold();
    if enabled.0
        && overlay.iter().any(|(_, drawn)| {
            drawn.threshold.to_bits() == threshold.to_bits()
                && drawn.generation == field.generation()
        })
    {
        return;
    }
    for (entity, _) in &overlay {
        commands.entity(entity).despawn();
    }
    if !enabled.0 {
        return;
    }

    let side = CONIFER_NOISE_OVERLAY_PX;
    let mut data = Vec::with_capacity((side * side * 4) as usize);
    for row in 0..side {
        for column in 0..side {
            // строка 0 текстуры — верх спрайта, то есть максимальный мировой y
            let position = Vec2::new(
                (column as f32 + 0.5) / side as f32 * MAP_SIZE.x,
                (side - 1 - row) as f32 / side as f32 * MAP_SIZE.y,
            );
            let value = field.sample(position);
            let (color, alpha) = if value >= threshold {
                (CONIFER_STAND_COLOR * value, CONIFER_STAND_ALPHA)
            } else {
                (Vec3::splat(value), CONIFER_NOISE_ALPHA)
            };
            let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0) as u8;
            data.extend_from_slice(&[byte(color.x), byte(color.y), byte(color.z), byte(alpha)]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // приложение целиком на `ImagePlugin::default_nearest()` (пиксель-арт
    // спрайтов), но поле гладкое: с ближайшим соседом тексель в 11 м вылезал
    // на карту квадратом в пол-кроны, и слой читался как сетка, а не как шум
    image.sampler = ImageSampler::linear();

    commands.spawn((
        ConiferNoiseOverlayMarker {
            threshold,
            generation: field.generation(),
        },
        Sprite {
            image: images.add(image),
            custom_size: Some(MAP_SIZE),
            ..default()
        },
        Transform::from_translation((MAP_SIZE / 2.0).extend(Z_CONIFER_NOISE_OVERLAY)),
        DespawnOnExit(AppState::Playing),
        Name::new("conifer_noise_overlay"),
    ));
}
