//! Панель скорости симуляции (порт `zxc/src/ui/simulation_state.rs`, без
//! игровой даты): текст в правом верхнем углу, обновляется из `Time<Virtual>`.
//! Вторая строка — диагностика pathfinding (порт заголовка
//! `zxc/src/ui/debug/info.rs`).

use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin};
use bevy::prelude::*;

use crate::diagnostics::{PATHFINDING_DURATION_MS, PATHFINDING_IN_FLIGHT};
use crate::ui::{UiOpacity, ui_color};

#[derive(Component, Default)]
struct SpeedTextMarker;

#[derive(Component, Default)]
struct PathfindingTextMarker;

pub struct UiSpeedPlugin;

impl Plugin for UiSpeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_speed_ui)
            .add_systems(Update, (update_speed_text, update_pathfinding_text));
    }
}

fn render_speed_ui(mut commands: Commands, time: Res<Time<Virtual>>) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(0.),
            right: px(0.),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(3.),
            // фиксированная ширина: количество цифр в счётчиках меняется
            // каждый кадр, и авто-ширина заставляла панель дёргаться
            width: px(340.),
            padding: UiRect {
                top: px(10.),
                right: px(16.),
                bottom: px(10.),
                left: px(16.),
            },
            ..default()
        },
        BackgroundColor(ui_color(UiOpacity::Light)),
        Name::new("speed_ui"),
        children![
            (
                Text(format_speed_text(&time)),
                TextFont {
                    font_size: FontSize::Px(20.),
                    ..default()
                },
                TextColor(Color::WHITE),
                SpeedTextMarker,
            ),
            (
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
                PathfindingTextMarker,
            ),
        ],
    ));
}

fn update_speed_text(text: Single<&mut Text, With<SpeedTextMarker>>, time: Res<Time<Virtual>>) {
    text.into_inner().set_if_neq(Text(format_speed_text(&time)));
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
        "pathfinding: {in_flight:>4.0} in flight, {duration_ms:>5.2} ms avg\nentities: {entities:>6.0}"
    )));
}

fn format_speed_text(time: &Time<Virtual>) -> String {
    if time.is_paused() {
        format!("Paused ({}x)", time.relative_speed())
    } else {
        format!("Speed: {}x", time.relative_speed())
    }
}
