//! Дебаг-тумблеры (порт `zxc/src/ui/debug/toggles.rs` на ресурсах вместо
//! стейтов): кнопки grid / navmesh / movepath в левом нижнем углу.
//!
//! - grid — сетка navtiles гизмо-линиями;
//! - navmesh — заливка непроходимых тайлов (Mesh2d, спавнится по включению);
//! - movepath — существующий `DrawMovePaths` (он же на клавише P).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::grid::tile_center;
use crate::movement::DrawMovePaths;
use crate::navigation::{ArcNavmesh, PathfindingAlgorithm};
use crate::settings::{GRID_SIZE, MAP_SIZE, NAVTILE_SIZE};
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, ui_color};

#[derive(Resource, Default)]
pub struct DebugGrid(pub bool);

#[derive(Resource, Default)]
pub struct DebugNavmesh(pub bool);

/// Какой слой переключает кнопка; определяет подсветку «активна».
#[derive(Component, Clone, Copy)]
enum DebugToggleButton {
    Grid,
    Navmesh,
    Movepath,
}

#[derive(Component)]
struct NavmeshOverlayMarker;

/// Подпись на кнопке-переключателе алгоритма поиска пути.
#[derive(Component)]
struct PathfindingMethodLabel;

const TOGGLE_ACTIVE_COLOR: Color = Color::srgba(0.16, 0.5, 0.2, 0.9);
/// Насколько светлее становится кнопка под курсором и при нажатии.
const TOGGLE_HOVER_LIGHTEN: f32 = 0.12;
const TOGGLE_PRESSED_LIGHTEN: f32 = 0.24;

/// Z заливки navmesh: над зданиями (5.0), под юнитами (5.5+).
const NAVMESH_OVERLAY_Z: f32 = 5.2;

pub struct UiDebugTogglesPlugin;

impl Plugin for UiDebugTogglesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugGrid>()
            .init_resource::<DebugNavmesh>()
            .add_systems(Startup, render_debug_toggles)
            .add_systems(
                Update,
                (
                    update_toggle_buttons,
                    render_grid.run_if(|grid: Res<DebugGrid>| grid.0),
                    sync_navmesh_overlay.run_if(resource_changed::<DebugNavmesh>),
                    sync_pathfinding_method_label.run_if(resource_changed::<PathfindingAlgorithm>),
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
        Name::new("navmesh_overlay"),
    ));
}
