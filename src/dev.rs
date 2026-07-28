//! Инструменты для отладочных сессий (skill `live-app`): скриншот в файл по
//! клавише F12 или BRP-событию `TakeScreenshotEvent`.

use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::grid::world_to_tile;
use crate::movement::Movable;
use crate::navigation::ArcNavmesh;
use crate::settings::unit_z;

const SCREENSHOT_PATH: &str = "screenshot.png";

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct TakeScreenshotEvent;

/// Тестовый агент навигации: спавнится в `from` и идёт в `to` (метры).
/// Триггерится по BRP: `brp event SpawnTestWalkerEvent '{"from":[..],"to":[..]}'`.
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct SpawnTestWalkerEvent {
    pub from: Vec2,
    pub to: Vec2,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct TestWalker;

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin::default(),
        ))
        .register_type::<TakeScreenshotEvent>()
        .register_type::<SpawnTestWalkerEvent>()
        .register_type::<TestWalker>()
        .add_observer(on_take_screenshot)
        .add_observer(on_spawn_test_walker)
        .add_systems(
            Update,
            trigger_screenshot.run_if(bevy::input::common_conditions::input_just_pressed(
                KeyCode::F12,
            )),
        );
    }
}

fn on_spawn_test_walker(
    event: On<SpawnTestWalkerEvent>,
    mut commands: Commands,
    arc_navmesh: Res<ArcNavmesh>,
) {
    let mut movable = Movable::new(4.0);
    let entity = commands
        .spawn((
            Sprite {
                color: Color::srgb(0.1, 0.1, 0.9),
                custom_size: Some(Vec2::splat(2.0)),
                ..default()
            },
            Transform::from_translation(event.from.extend(unit_z(event.from.y))),
            TestWalker,
            Name::new("test_walker"),
        ))
        .id();
    movable.to_pathfinding(
        entity,
        world_to_tile(event.from),
        world_to_tile(event.to),
        &arc_navmesh,
        &mut commands,
    );
    commands.entity(entity).insert(movable);
}

fn trigger_screenshot(mut commands: Commands) {
    commands.trigger(TakeScreenshotEvent);
}

fn on_take_screenshot(_event: On<TakeScreenshotEvent>, mut commands: Commands) {
    info!("saving screenshot to {SCREENSHOT_PATH}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(SCREENSHOT_PATH));
}
