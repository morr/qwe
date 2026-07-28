use bevy::camera_controller::pan_camera::{MousePanSettings, PanCamera, PanCameraPlugin};
use bevy::prelude::*;

use crate::settings::PORTAL_POS;

/// Зум = масштаб трансформа камеры: мировых метров на экранный пиксель.
/// 0.0625 (= 1/16) — «крупный план», нативный пиксель ассетов (16 px = 1 м);
/// ~1.0 — вся карта (1200 м) в кадре при окне 1280.
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 1.1;
const START_ZOOM: f32 = 0.4;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PanCameraPlugin)
            .add_systems(Startup, spawn_camera);
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(PORTAL_POS.extend(0.0)),
        Msaa::Off,
        PanCamera {
            zoom_factor: START_ZOOM,
            min_zoom: MIN_ZOOM,
            max_zoom: MAX_ZOOM,
            zoom_speed: 0.05,
            // `=`/`-` отданы скорости симуляции (sim_time); зум — колесом
            key_zoom_in: None,
            key_zoom_out: None,
            pan_speed: 600.0,
            // без поворота камеры
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            mouse_pan_settings: MousePanSettings {
                enabled: true,
                button: MouseButton::Left,
            },
            ..default()
        },
        Name::new("main_camera"),
    ));
}
