use bevy::app::AppExit;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

fn main() {
    App::new()
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
        .add_systems(Startup, (spawn_camera, spawn_sprite))
        .add_systems(
            Update,
            (
                rotate_sprite,
                close_on_esc.run_if(input_just_pressed(KeyCode::Escape)),
            ),
        )
        .run();
}

#[derive(Component)]
struct Spinner;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Msaa::Off,
        Name::new("main_camera"),
    ));
}

fn spawn_sprite(mut commands: Commands) {
    commands.spawn((
        Sprite {
            color: Color::srgb(0.3, 0.6, 0.9),
            custom_size: Some(Vec2::splat(120.0)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
        Spinner,
        Name::new("spinner"),
    ));
}

fn rotate_sprite(time: Res<Time>, mut query: Query<&mut Transform, With<Spinner>>) {
    for mut transform in query.iter_mut() {
        transform.rotate_z(time.delta_secs());
    }
}

/// Gated by `input_just_pressed(Escape)` in the schedule — the window-focus
/// check stays here so Esc in another app's window doesn't quit this one.
fn close_on_esc(focused_windows: Query<&Window>, mut event_writer: MessageWriter<AppExit>) {
    if focused_windows.iter().any(|window| window.focused) {
        event_writer.write(AppExit::Success);
    }
}
