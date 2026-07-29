//! Дебаг-тумблеры (порт `zxc/src/ui/debug/toggles.rs` на ресурсах вместо
//! стейтов): кнопки grid / navmesh / doors / movepath в левом нижнем углу.
//!
//! - grid — сетка navtiles гизмо-линиями;
//! - navmesh — заливка непроходимых тайлов (Mesh2d, спавнится по включению);
//! - doors — входы в здания, свои и досочинённые (`map/osm/entrances.rs`);
//! - movepath — существующий `DrawMovePaths` (он же на клавише M).
//!
//! Хоткеи: N — navmesh, M — movepath (в `movement`), G — «гизмо» одной
//! клавишей, то есть doors и movepath вместе. У grid хоткея нет: сетка нужна
//! редко и только вблизи, кнопки в панели достаточно.

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::input::common_conditions::input_just_pressed;
use bevy::picking::hover::Hovered;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;

use bevy::prelude::*;

use crate::grid::tile_center;
use crate::loading::{AppState, WorldInitSet};
use crate::map::osm::MapData;
use crate::movement::DrawMovePaths;
use crate::navigation::{ArcNavmesh, PathfindingAlgorithm};
use crate::settings::{GRID_SIZE, MAP_SIZE, NAVTILE_SIZE};
use crate::ui::{
    GameUiRoot, TOGGLE_ACTIVE_COLOR, TOGGLE_HOVER_LIGHTEN, TOGGLE_PRESSED_LIGHTEN,
    UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, ui_color,
};

// оба тумблера — группы настроек (`prefs`), поэтому Reflect + SettingsGroup
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "grid")]
pub struct DebugGrid(pub bool);

#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "navmesh")]
pub struct DebugNavmesh(pub bool);

#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "doors")]
pub struct DebugDoors(pub bool);

/// Какой слой переключает кнопка; определяет подсветку «активна».
#[derive(Component, Clone, Copy)]
enum DebugToggleButton {
    Grid,
    Navmesh,
    Doors,
    Movepath,
}

#[derive(Component)]
struct NavmeshOverlayMarker;

/// Подпись на кнопке-переключателе алгоритма поиска пути.
#[derive(Component)]
struct PathfindingMethodLabel;

/// Z заливки navmesh: над зданиями (5.0), под юнитами (5.5+).
const NAVMESH_OVERLAY_Z: f32 = 5.2;

/// Метка двери, м: с высоты, на которой видно квартал, кружок меньше метра
/// уже не читается.
const DOOR_MARKER_RADIUS: f32 = 1.5;
const DOOR_COLOR: Color = Color::srgba(0.15, 0.85, 1.0, 0.9);
/// Сколько экранов вокруг камеры рисовать двери. Как и у movepath: на всю
/// карту это десять тысяч гизмо за кадр, а за соседним экраном их не видно.
const DOORS_VIEW_SCREENS: f32 = 1.5;

pub struct UiDebugTogglesPlugin;

impl Plugin for UiDebugTogglesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugGrid>()
            .init_resource::<DebugNavmesh>()
            .init_resource::<DebugDoors>()
            .register_type::<DebugGrid>()
            .register_type::<DebugNavmesh>()
            .register_type::<DebugDoors>()
            .add_systems(Startup, render_debug_toggles)
            // тумблер, восстановленный из настроек, менялся до того, как
            // navmesh был заполнен, — красим заливку ещё раз по спавну мира
            .add_systems(
                OnEnter(AppState::Playing),
                sync_navmesh_overlay.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    update_toggle_buttons,
                    render_grid.run_if(|grid: Res<DebugGrid>| grid.0),
                    // MapData появляется только под Playing
                    render_doors
                        .run_if(|doors: Res<DebugDoors>| doors.0)
                        .run_if(in_state(AppState::Playing)),
                    sync_navmesh_overlay.run_if(resource_changed::<DebugNavmesh>),
                    sync_pathfinding_method_label.run_if(resource_changed::<PathfindingAlgorithm>),
                    toggle_navmesh.run_if(input_just_pressed(KeyCode::KeyN)),
                    toggle_gizmos.run_if(input_just_pressed(KeyCode::KeyG)),
                ),
            );
    }
}

fn render_debug_toggles(mut commands: Commands) {
    let row = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                column_gap: px(6.),
                padding: UiRect::all(px(10.)),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("debug_toggles"),
        ))
        .id();

    spawn_toggle(
        &mut commands,
        row,
        "grid",
        DebugToggleButton::Grid,
        |_activate: On<Activate>, mut grid: ResMut<DebugGrid>| {
            grid.0 = !grid.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "navmesh",
        DebugToggleButton::Navmesh,
        |_activate: On<Activate>, mut navmesh: ResMut<DebugNavmesh>| {
            navmesh.0 = !navmesh.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "doors",
        DebugToggleButton::Doors,
        |_activate: On<Activate>, mut doors: ResMut<DebugDoors>| {
            doors.0 = !doors.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "movepath",
        DebugToggleButton::Movepath,
        |_activate: On<Activate>, mut movepaths: ResMut<DrawMovePaths>| {
            movepaths.0 = !movepaths.0;
        },
    );

    // переключатель алгоритма поиска пути — клик листает по циклу
    let method_button = commands
        .spawn((
            Button,
            Pickable::default(),
            Hovered::default(),
            Node {
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
            children![(
                PathfindingMethodLabel,
                Text::new(PathfindingAlgorithm::default().label()),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .observe(
            |_activate: On<Activate>, mut algorithm: ResMut<PathfindingAlgorithm>| {
                *algorithm = algorithm.next();
            },
        )
        .id();
    commands.entity(row).add_child(method_button);
}

fn toggle_navmesh(mut navmesh: ResMut<DebugNavmesh>) {
    navmesh.0 = !navmesh.0;
}

/// G — общий тумблер «гизмо»: doors и movepath разом. Гасит всё, если горит
/// хоть один слой, иначе зажигает оба, — чтобы одно нажатие всегда очищало
/// экран, в каком бы состоянии слои ни разошлись поодиночке.
fn toggle_gizmos(mut doors: ResMut<DebugDoors>, mut movepaths: ResMut<DrawMovePaths>) {
    let on = !(doors.0 || movepaths.0);
    doors.0 = on;
    movepaths.0 = on;
}

/// Актуализация подписи при смене алгоритма (кнопкой или через BRP).
fn sync_pathfinding_method_label(
    algorithm: Res<PathfindingAlgorithm>,
    mut labels: Query<&mut Text, With<PathfindingMethodLabel>>,
) {
    for mut text in &mut labels {
        text.0 = algorithm.label().to_string();
    }
}

fn spawn_toggle<M>(
    commands: &mut Commands,
    row: Entity,
    label: &str,
    kind: DebugToggleButton,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let button = commands
        .spawn((
            Button,
            kind,
            Pickable::default(),
            // `Hovered` обновляет UI-picking-бэкенд; `Pressed` вставляет
            // `bevy_ui_widgets::Button` — оба нужны для подсветки.
            Hovered::default(),
            Node {
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
            children![(
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .observe(on_activate)
        .id();
    commands.entity(row).add_child(button);
}

fn update_toggle_buttons(
    grid: Res<DebugGrid>,
    navmesh: Res<DebugNavmesh>,
    doors: Res<DebugDoors>,
    movepaths: Res<DrawMovePaths>,
    mut buttons: Query<(
        &DebugToggleButton,
        &Hovered,
        Has<Pressed>,
        &mut BackgroundColor,
    )>,
) {
    for (toggle, hovered, is_pressed, mut background) in &mut buttons {
        let is_active = match toggle {
            DebugToggleButton::Grid => grid.0,
            DebugToggleButton::Navmesh => navmesh.0,
            DebugToggleButton::Doors => doors.0,
            DebugToggleButton::Movepath => movepaths.0,
        };
        let base = if is_active {
            TOGGLE_ACTIVE_COLOR
        } else {
            ui_color(UiOpacity::Heavy)
        };
        let lighten = if is_pressed {
            TOGGLE_PRESSED_LIGHTEN
        } else if hovered.get() {
            TOGGLE_HOVER_LIGHTEN
        } else {
            0.0
        };
        background.set_if_neq(BackgroundColor(base.mix(&Color::WHITE, lighten)));
    }
}

/// Сетка navtiles гизмо-линиями по краям тайлов.
fn render_grid(mut gizmos: Gizmos) {
    let color = Color::srgba(0.2, 0.2, 0.2, 0.3);
    for x in 0..=GRID_SIZE.x {
        let world_x = x as f32 * NAVTILE_SIZE;
        gizmos.line_2d(
            Vec2::new(world_x, 0.0),
            Vec2::new(world_x, MAP_SIZE.y),
            color,
        );
    }
    for y in 0..=GRID_SIZE.y {
        let world_y = y as f32 * NAVTILE_SIZE;
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
fn render_doors(
    map: Res<MapData>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut gizmos: Gizmos,
) {
    let camera_position = camera.translation.truncate();
    let half_view =
        Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * DOORS_VIEW_SCREENS;

    for building in &map.buildings {
        for &door in &building.entrances {
            let offset = (door - camera_position).abs();
            if offset.x > half_view.x || offset.y > half_view.y {
                continue;
            }
            gizmos.circle_2d(door, DOOR_MARKER_RADIUS, DOOR_COLOR);
        }
    }
}

/// Спавн/despawn заливки непроходимых тайлов при переключении тумблера.
/// Один слитый меш на все тайлы: на OSM-карте их сотни тысяч, отдельные
/// entity на каждый укладывали кадр.
fn sync_navmesh_overlay(
    mut commands: Commands,
    navmesh_enabled: Res<DebugNavmesh>,
    arc_navmesh: Res<ArcNavmesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    overlay: Query<Entity, With<NavmeshOverlayMarker>>,
) {
    if !navmesh_enabled.0 {
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        return;
    }

    let color = Color::srgba(0.9, 0.15, 0.15, 0.35).to_linear();
    let mut builder = crate::map::MeshBuilder::default();
    let navmesh = arc_navmesh.read();
    for x in 0..GRID_SIZE.x {
        for y in 0..GRID_SIZE.y {
            if navmesh.is_passable(x, y) {
                continue;
            }
            let center = tile_center(IVec2::new(x, y));
            builder.push_rect(
                center - NAVTILE_SIZE / 2.0,
                center + NAVTILE_SIZE / 2.0,
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
