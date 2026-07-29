//! Панель скорости симуляции (порт `zxc/src/ui/simulation_state.rs`, без
//! игровой даты): текст в правом верхнем углу, обновляется из `Time<Virtual>`.
//! Там же часы симуляции (`SimClock`) — сколько мир уже прожил.
//! Вторая строка — диагностика pathfinding (порт заголовка
//! `zxc/src/ui/debug/info.rs`), третья — зум и позиция камеры плюс точка под
//! курсором.

use bevy::camera_controller::pan_camera::PanCamera;
use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::cursor_offset;
use crate::diagnostics::{PATHFINDING_DURATION_MS, PATHFINDING_IN_FLIGHT, PATHFINDING_QUEUED};
use crate::sim_time::{SimClock, SimSpeed};
use crate::ui::{GameUiRoot, UI_TEXT_SHADOW, UiOpacity, ui_color};

#[derive(Component, Default)]
struct SpeedTextMarker;

#[derive(Component, Default)]
struct PathfindingTextMarker;

#[derive(Component, Default)]
struct CameraTextMarker;

pub struct UiSpeedPlugin;

impl Plugin for UiSpeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_speed_ui).add_systems(
            Update,
            (
                update_speed_text,
                update_pathfinding_text,
                update_camera_text,
            ),
        );
    }
}

fn render_speed_ui(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    speed: Res<SimSpeed>,
    clock: Res<SimClock>,
) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(0.),
            right: px(0.),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(3.),
            // фиксированная ширина: количество цифр в счётчиках меняется
            // каждый кадр, и авто-ширина заставляла панель дёргаться. Ширины
            // хватает на строку pathfinding целиком — перенос её на вторую
            // строку сдвигал бы всё под ней
            width: px(470.),
            padding: UiRect {
                top: px(10.),
                right: px(16.),
                bottom: px(10.),
                left: px(16.),
            },
            ..default()
        },
        BackgroundColor(ui_color(UiOpacity::Medium)),
        GameUiRoot,
        Visibility::Hidden,
        Name::new("speed_ui"),
        children![
            (
                Text(format_speed_text(&time, &speed, &clock)),
                TextFont {
                    font_size: FontSize::Px(20.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
                SpeedTextMarker,
            ),
            (
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
                PathfindingTextMarker,
            ),
            (
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
                CameraTextMarker,
            ),
        ],
    ));
}

fn update_speed_text(
    text: Single<&mut Text, With<SpeedTextMarker>>,
    time: Res<Time<Virtual>>,
    speed: Res<SimSpeed>,
    clock: Res<SimClock>,
) {
    text.into_inner()
        .set_if_neq(Text(format_speed_text(&time, &speed, &clock)));
}

/// Строка pathfinding-диагностики: в полёте, среднее время поиска, сущности.
fn update_pathfinding_text(
    text: Single<&mut Text, With<PathfindingTextMarker>>,
    diagnostics: Res<DiagnosticsStore>,
) {
    let in_flight = diagnostics
        .get(&PATHFINDING_IN_FLIGHT)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    let queued = diagnostics
        .get(&PATHFINDING_QUEUED)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    let duration_ms = diagnostics
        .get(&PATHFINDING_DURATION_MS)
        .and_then(|diagnostic| diagnostic.average())
        .unwrap_or_default();
    let entities = diagnostics
        .get(&EntityCountDiagnosticsPlugin::ENTITY_COUNT)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();

    // выравнивание цифр по правому краю, чтобы строка не «плясала»
    text.into_inner().set_if_neq(Text(format!(
        "pathfinding: {in_flight:>4.0} in flight, {queued:>5.0} queued, {duration_ms:>5.2} ms avg\nentities: {entities:>6.0}"
    )));
}

/// Где стоит камера и насколько приближена. Строка нужна не игроку, а чтению
/// скриншота со стороны: по ней видно, какой кусок карты в кадре, без запроса
/// к живому миру по BRP.
///
/// Формат `0.41/2374/2703 2510/2880` — сначала камера как `zoom/x/y` (порядок
/// пермалинка slippy-карт), через пробел — точка под курсором как `x/y`.
///
/// Координаты — мировые метры от юго-западного угла карты, та же система, в
/// которой лежат `SimPosition` и `Transform` юнитов; камерные — центр экрана.
/// Зум — метры на экранный пиксель, как их держит `PanCamera::zoom_factor`:
/// меньше — ближе.
///
/// Курсор вне окна координат не даёт — вместо чисел прочерки, чтобы строка не
/// теряла хвост и не выглядела обрезанной.
fn update_camera_text(
    text: Single<&mut Text, With<CameraTextMarker>>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let (transform, controller) = *camera;
    let center = transform.translation.truncate();
    let zoom = controller.zoom_factor;

    let cursor = match window.cursor_position() {
        Some(cursor) => {
            let world = center + cursor_offset(&window, cursor) * zoom;
            format!("{:.0}/{:.0}", world.x, world.y)
        }
        None => "-/-".to_string(),
    };

    // строка последняя в панели, поэтому ширины полей не выравниваем: сдвигать
    // её «пляской» цифр нечему
    text.into_inner().set_if_neq(Text(format!(
        "{zoom:.2}/{:.0}/{:.0} {cursor}",
        center.x, center.y
    )));
}

/// «Speed: 15x» — идём как просили; «Speed: 15x → 9.8x» — машина не тянет,
/// время замедлено (см. `sim_time`). После стрелки — замеренная фактическая
/// скорость, поэтому она бывает и меньше 1x: на просадке (например, пока
/// фоново строится сетка northstar) симуляция отстаёт от реального времени.
///
/// Хвостом — часы симуляции (`SimClock`): в какой момент своей жизни мир
/// сейчас находится. На 15x они бегут в пятнадцать раз быстрее настенных, и
/// смотреть на «сколько идёт прогон» нужно именно по ним.
///
/// Разделителем два пробела: часы стоят на той же строке, что и скорость, и
/// одним пробелом слипались бы с `15x` в одно число.
fn format_speed_text(time: &Time<Virtual>, speed: &SimSpeed, clock: &SimClock) -> String {
    let clock = format_sim_clock(clock.elapsed);
    let requested = format!("{}x", speed.requested);
    if time.is_paused() {
        return format!("Paused ({requested})  {clock}");
    }
    if speed.is_throttled() {
        // ниже 1x одного знака мало: 0.3x и 0.06x — разные истории
        let actual = if speed.actual < 1.0 {
            format!("{:.2}", speed.actual)
        } else {
            format!("{:.1}", speed.actual)
        };
        format!("Speed: {requested} → {actual}x  {clock}")
    } else {
        format!("Speed: {requested}  {clock}")
    }
}

/// Часы симуляции как `T+8130` — секунды и всё: разбивка на часы и сутки пока
/// не нужна, а секунды напрямую сопоставимы с периодами в `settings.rs`.
fn format_sim_clock(elapsed: f64) -> String {
    format!("T+{}", elapsed.max(0.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_clock_counts_whole_seconds() {
        assert_eq!(format_sim_clock(0.0), "T+0");
        assert_eq!(format_sim_clock(8130.4), "T+8130");
    }
}
