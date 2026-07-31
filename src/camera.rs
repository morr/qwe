use bevy::camera_controller::pan_camera::{MousePanSettings, PanCamera, PanCameraPlugin};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
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
/// Публичный: от него считается стартовая ступень зум-LOD трамвая
/// (`map::tram::TramZoomBucket`).
pub const START_ZOOM: f32 = 0.4;
/// Множитель зума на один щелчок колеса.
const ZOOM_STEP: f32 = 1.12;
/// Скорость WASD-пана в *экранных* логических пикселях в секунду — как у
/// `drag_pan`, поэтому на любом масштабе карта уезжает одинаково быстро.
/// (`pan_speed` у PanCamera задаётся в мировых метрах и на крупном плане
/// швыряет камеру, а на общем — еле тащит.)
const PAN_SPEED: f32 = 1125.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PanCameraPlugin)
            .add_systems(Startup, spawn_camera)
            // карта нового города лежит в тех же координатах, но портал
            // переезжает — камеру возвращаем к нему на каждой загрузке
            .add_systems(
                OnEnter(AppState::Playing),
                reset_camera_to_portal.in_set(WorldInitSet::Spawn),
            )
            // Мышь и WASD ведём сами (см. ниже); у PanCamera остаётся только
            // применение zoom_factor к масштабу трансформа.
            .add_systems(Update, (zoom_to_cursor, drag_pan, key_pan));
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
            // WASD ведёт `key_pan`: шаг PanCamera задан в мировых метрах и
            // потому зависит от масштаба
            pan_speed: 0.0,
            key_up: None,
            key_down: None,
            key_left: None,
            key_right: None,
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

/// Камера на портал в стартовом зуме: позиция портала снапится по navmesh
/// уже загруженного города, так что известна только к входу в `Playing`.
///
/// Зум сбрасывается вместе с позицией: разглядывать новый город с того
/// приближения, на котором бросили предыдущий, незачем — да и на общем плане
/// (4.5) отъезд после смены города читается как «карта не загрузилась».
fn reset_camera_to_portal(
    portal: Res<PortalPos>,
    mut camera: Single<(&mut Transform, &mut PanCamera), With<Camera2d>>,
) {
    let (transform, controller) = &mut *camera;
    transform.translation = portal.0.extend(transform.translation.z);
    // масштаб трансформа PanCamera держит сам, по zoom_factor
    controller.zoom_factor = START_ZOOM;
}

/// Смещение курсора от центра окна в мировых осях (экранный y — вниз).
pub fn cursor_offset(window: &Window, cursor: Vec2) -> Vec2 {
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

/// Пан на WASD в экранной скорости: шаг умножается на `zoom_factor`, поэтому
/// на крупном плане камера проходит меньше метров, а на общем — больше, и на
/// экране карта в обоих случаях едет с одной скоростью.
///
/// Время реальное: пан не должен замирать вместе с паузой симуляции.
fn key_pan(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&mut Transform, &PanCamera), With<Camera>>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    let Some(dir) = dir.try_normalize() else {
        return;
    };
    let Ok((mut transform, controller)) = query.single_mut() else {
        return;
    };

    let delta = dir * PAN_SPEED * controller.zoom_factor * time.delta_secs();
    transform.translation += delta.extend(0.0);
}

/// Состояние протяжки левой кнопкой. Решение «камера или UI» принимается один
/// раз, в кадре нажатия, и держится до отпускания: протяжка ползунка плотности
/// уводит курсор с панели, и покадровая проверка «курсор над UI» отдала бы
/// остаток протяжки камере.
#[derive(Default, Clone, Copy)]
enum DragPan {
    #[default]
    Idle,
    /// Зажатие началось над панелью — камера в нём не участвует.
    OverUi,
    /// Зажатие началось над картой; хранится позиция курсора в прошлом кадре.
    Dragging(Vec2),
}

/// Курсор над каким-нибудь узлом `bevy_ui` (идиома из `zxc/src/input.rs`):
/// `HoverMap` собирает UI-пикинг, мировой ввод под панелью обрабатывать нельзя.
fn pointer_over_ui(hover_map: &HoverMap, ui_nodes: &Query<(), With<Node>>) -> bool {
    hover_map
        .values()
        .flat_map(|pointer| pointer.keys())
        .any(|entity| ui_nodes.contains(*entity))
}

/// Пан зажатой левой кнопкой: точка мира «схвачена» курсором и движется с
/// ним один в один (по логическим px, поэтому ретина-масштаб не удваивает
/// скорость, как это делал экранный `delta` у PanCamera).
fn drag_pan(
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut drag: Local<DragPan>,
    mut query: Query<(&mut Transform, &PanCamera), With<Camera>>,
) {
    if !buttons.pressed(MouseButton::Left) {
        *drag = DragPan::Idle;
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    if matches!(*drag, DragPan::Idle) {
        *drag = if pointer_over_ui(&hover_map, &ui_nodes) {
            DragPan::OverUi
        } else {
            DragPan::Dragging(cursor)
        };
        return;
    }
    let DragPan::Dragging(last) = *drag else {
        return;
    };
    let Ok((mut transform, controller)) = query.single_mut() else {
        return;
    };

    let delta = (cursor - last) * Vec2::new(1.0, -1.0) * controller.zoom_factor;
    transform.translation -= delta.extend(0.0);
    *drag = DragPan::Dragging(cursor);
}
