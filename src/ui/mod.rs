//! Игровой UI (порт идиом из `zxc/src/ui`): панель скорости симуляции и
//! дебаг-тумблеры. Обычные `bevy_ui`-ноды + первопартийный
//! `bevy_ui_widgets::Button`.

mod buildings;
mod city;
mod debug;
mod hotkeys;
mod speed;
mod trees;

use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

pub use self::debug::{DebugDoors, DebugGrid, DebugNavmesh};
use crate::loading::{AppState, PlayPhase};

pub const UI_SCREEN_EDGE_PX_OFFSET: f32 = 8.0;

/// Подсветка «кнопка активна» и осветление под курсором / при нажатии —
/// общие для тумблеров и панели городов.
pub const TOGGLE_ACTIVE_COLOR: Color = Color::srgba(0.16, 0.5, 0.2, 0.9);
pub const TOGGLE_HOVER_LIGHTEN: f32 = 0.12;
pub const TOGGLE_PRESSED_LIGHTEN: f32 = 0.24;

const UI_COLOR: Color = Color::srgb(0.094, 0.102, 0.11);

/// Тень под белым текстом панелей: фон у них полупрозрачный, и поверх светлой
/// карты буквы без тени сливаются с ней. Смещение в пиксель — дефолтные четыре
/// на 11–20 px шрифте читаются как вторая строка текста.
pub const UI_TEXT_SHADOW: TextShadow = TextShadow {
    offset: Vec2::splat(1.0),
    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
};

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

/// Кнопка панели: тёмный прямоугольник с подписью 12 px, подсвечиваемый по
/// наведению и нажатию. `marker` — компонент, по которому система подсветки
/// находит эту кнопку среди прочих (`DebugToggleButton`, `CityButton`, …).
///
/// `Hovered` кормит UI-picking-бэкенд, `Pressed` вставляет
/// `bevy_ui_widgets::Button` — для подсветки нужны оба.
pub fn spawn_panel_button<M>(
    commands: &mut Commands,
    parent: Entity,
    marker: impl Bundle,
    label: &str,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let button = commands
        .spawn((
            Button,
            marker,
            Pickable::default(),
            Hovered::default(),
            Node {
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
            children![(
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .observe(on_activate)
        .id();
    commands.entity(parent).add_child(button);
    button
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
            buildings::UiBuildingStylePlugin,
            city::UiCityPlugin,
            hotkeys::UiHotkeysPlugin,
        ))
        .add_systems(OnEnter(PlayPhase::Live), show_game_ui)
        // смена города возвращает приложение на экран загрузки — панели
        // прячутся до конца следующего прогрева
        .add_systems(OnExit(AppState::Playing), hide_game_ui);
    }
}

fn show_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Inherited;
    }
}

fn hide_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Hidden;
    }
}
