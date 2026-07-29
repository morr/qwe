use bevy::camera_controller::pan_camera::{MousePanSettings, PanCamera, PanCameraPlugin};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::city::City;
use crate::loading::{AppState, WorldInitSet};
use crate::portal::PortalPos;

/// Зум = масштаб трансформа камеры: мировых метров на экранный пиксель.
/// 0.0625 (= 1/16) — «крупный план», нативный пиксель ассетов (16 px = 1 м);
/// ~4.4 — вся карта (5600 м) в кадре при окне 1280.
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 4.5;
const START_ZOOM: f32 = 0.4;
/// Множитель зума на один щелчок колеса.
const ZOOM_STEP: f32 = 1.12;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PanCameraPlugin)
            .add_systems(Startup, spawn_camera)
            // карта нового города лежит в тех же координатах, но портал
            // переезжает — камеру возвращаем к нему на каждой загрузке
            .add_systems(
                OnEnter(AppState::Playing),
                center_camera_on_portal.in_set(WorldInitSet::Spawn),
            )
            // Мышь ведём сами (см. ниже); у PanCamera остаются WASD-пан и
            // применение zoom_factor к масштабу трансформа.
            .add_systems(Update, (zoom_to_cursor, drag_pan));
    }
}

fn spawn_camera(mut commands: Commands, city: Res<City>) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(city.portal_hint().extend(0.0)),
        Msaa::Off,
        PanCamera {
            zoom_factor: START_ZOOM,
            min_zoom: MIN_ZOOM,
            max_zoom: MAX_ZOOM,
            // колесо обрабатывает `zoom_to_cursor`, а не PanCamera
            zoom_speed: 0.0,
            // `=`/`-` отданы скорости симуляции (sim_time)
            key_zoom_in: None,
            key_zoom_out: None,
            pan_speed: 600.0,
            // без поворота камеры
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            // drag ведёт `drag_pan`: якорит мир к курсору (1:1, как
            // bevy_pancam в zxc) и не зависит от масштаба ретины
            mouse_pan_settings: MousePanSettings {
                enabled: false,
                button: MouseButton::Left,
            },
            ..default()
        },
        Name::new("main_camera"),
    ));
}

/// Камера на портал: позиция снапится по navmesh уже загруженного города,
/// так что она известна только к входу в `Playing`.
fn center_camera_on_portal(
    portal: Res<PortalPos>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
) {
    camera.translation = portal.0.extend(camera.translation.z);
}

/// Смещение курсора от центра окна в мировых осях (экранный y — вниз).
fn cursor_offset(window: &Window, cursor: Vec2) -> Vec2 {
    (cursor - window.size() / 2.0) * Vec2::new(1.0, -1.0)
}

/// Зум колесом к точке под курсором: мировая точка под курсором остаётся
/// на месте, а не уезжает к центру экрана.
fn zoom_to_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut query: Query<(&mut Transform, &mut PanCamera), With<Camera>>,
) {
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
    };
    if lines == 0.0 {
        return;
    }
    let Ok((mut transform, mut controller)) = query.single_mut() else {
        return;
    };

    let old_zoom = controller.zoom_factor;
    let new_zoom =
        (old_zoom * ZOOM_STEP.powf(-lines)).clamp(controller.min_zoom, controller.max_zoom);
    if new_zoom == old_zoom {
        return;
    }

    if let Some(cursor) = window.cursor_position() {
        let offset = cursor_offset(&window, cursor);
        let world_under_cursor = transform.translation.truncate() + offset * old_zoom;
        let translation = world_under_cursor - offset * new_zoom;
        transform.translation = translation.extend(transform.translation.z);
    }
    controller.zoom_factor = new_zoom;
    transform.scale = Vec3::splat(new_zoom);
}

/// Пан зажатой левой кнопкой: точка мира «схвачена» курсором и движется с
/// ним один в один (по логическим px, поэтому ретина-масштаб не удваивает
/// скорость, как это делал экранный `delta` у PanCamera).
fn drag_pan(
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut last_cursor: Local<Option<Vec2>>,
    mut query: Query<(&mut Transform, &PanCamera), With<Camera>>,
) {
    if !buttons.pressed(MouseButton::Left) {
        *last_cursor = None;
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((mut transform, controller)) = query.single_mut() else {
        return;
    };

    if let Some(last) = *last_cursor {
        let delta = (cursor - last) * Vec2::new(1.0, -1.0) * controller.zoom_factor;
        transform.translation -= delta.extend(0.0);
    }
    *last_cursor = Some(cursor);
}
