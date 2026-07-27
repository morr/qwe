use bevy::app::AppExit;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

use qwe::{camera, demon, dev, map, movement, navigation, portal};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.72, 0.71, 0.68)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe".to_string(),
                        position: WindowPosition::Automatic,
                        mode: bevy::window::WindowMode::Windowed,
                        present_mode: bevy::window::PresentMode::AutoVsync,
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    level: bevy::log::Level::TRACE,
                    filter: "info,qwe=trace".to_string(),
                    ..default()
                }),
        )
        .add_plugins((RemotePlugin::default(), RemoteHttpPlugin::default()))
        .add_plugins((
            camera::CameraPlugin,
            map::MapPlugin,
            navigation::NavigationPlugin,
            movement::MovementPlugin,
            portal::PortalPlugin,
            demon::DemonPlugin,
            dev::DevPlugin,
        ))
        .add_systems(
            Update,
            close_on_esc.run_if(input_just_pressed(KeyCode::Escape)),
        )
        .run();
}

/// Gated by `input_just_pressed(Escape)` in the schedule — the window-focus
/// check stays here so Esc in another app's window doesn't quit this one.
fn close_on_esc(focused_windows: Query<&Window>, mut event_writer: MessageWriter<AppExit>) {
    if focused_windows.iter().any(|window| window.focused) {
        event_writer.write(AppExit::Success);
    }
}
