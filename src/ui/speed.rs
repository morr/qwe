//! Панель скорости симуляции (порт `zxc/src/ui/simulation_state.rs`, без
//! игровой даты): текст в правом верхнем углу, обновляется из `Time<Virtual>`.

use bevy::prelude::*;

use crate::ui::{UiOpacity, ui_color};

#[derive(Component, Default)]
struct SpeedTextMarker;

pub struct UiSpeedPlugin;

impl Plugin for UiSpeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_speed_ui)
            .add_systems(Update, update_speed_text);
    }
}

fn render_speed_ui(mut commands: Commands, time: Res<Time<Virtual>>) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(0.),
            right: px(0.),
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
        children![(
            Text(format_speed_text(&time)),
            TextFont {
                font_size: FontSize::Px(20.),
                ..default()
            },
            TextColor(Color::WHITE),
            SpeedTextMarker,
        )],
    ));
}

fn update_speed_text(text: Single<&mut Text, With<SpeedTextMarker>>, time: Res<Time<Virtual>>) {
    text.into_inner().set_if_neq(Text(format_speed_text(&time)));
}

fn format_speed_text(time: &Time<Virtual>) -> String {
    if time.is_paused() {
        format!("Paused ({}x)", time.relative_speed())
    } else {
        format!("Speed: {}x", time.relative_speed())
    }
}
