//! Игровой UI (порт идиом из `zxc/src/ui`): панель скорости симуляции и
//! дебаг-тумблеры. Обычные `bevy_ui`-ноды + первопартийный
//! `bevy_ui_widgets::Button`.

mod debug;
mod speed;
mod trees;

use bevy::prelude::*;

pub use self::debug::{DebugGrid, DebugNavmesh};
use crate::loading::PlayPhase;

pub const UI_SCREEN_EDGE_PX_OFFSET: f32 = 8.0;

const UI_COLOR: Color = Color::srgb(0.094, 0.102, 0.11);

pub enum UiOpacity {
    Light,
    /// Фон панелей: сквозь `Light` просвечивали пешки и кроны, `Heavy` глушит
    /// карту под панелью.
    Medium,
    Heavy,
}

pub fn ui_color(opacity: UiOpacity) -> Color {
    UI_COLOR.with_alpha(match opacity {
        UiOpacity::Light => 0.25,
        UiOpacity::Medium => 0.55,
        UiOpacity::Heavy => 0.85,
    })
}

/// Корень игровой панели. Панели спавнятся в `Startup`, но поверх экрана
/// загрузки им делать нечего — до конца прогрева (`PlayPhase::Live`) они
/// скрыты (спавнятся с `Visibility::Hidden`).
#[derive(Component)]
pub struct GameUiRoot;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            speed::UiSpeedPlugin,
            debug::UiDebugTogglesPlugin,
            trees::UiTreeStylePlugin,
        ))
        .add_systems(OnEnter(PlayPhase::Live), show_game_ui);
    }
}

fn show_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Inherited;
    }
}
