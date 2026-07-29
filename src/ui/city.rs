//! Панель выбора города — снизу по центру: кнопка на город, активна кнопка
//! текущего. Клик пишет ресурс `City`, а перезагрузку мира делает
//! `city::reload_world_on_city_change`.

use bevy::color::Mix;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::Activate;

use crate::city::City;
use crate::ui::{
    GameUiRoot, TOGGLE_ACTIVE_COLOR, TOGGLE_HOVER_LIGHTEN, TOGGLE_PRESSED_LIGHTEN,
    UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, spawn_panel_button, ui_color,
};

/// Какой город выбирает кнопка.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct CityButton(City);

pub struct UiCityPlugin;

impl Plugin for UiCityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_city_panel)
            .add_systems(Update, update_city_buttons);
    }
}

fn render_city_panel(mut commands: Commands) {
    let row = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                // центрирование абсолютной ноды: левый край в середину
                // экрана, затем сдвиг на половину собственной ширины
                left: percent(50.),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                column_gap: px(6.),
                padding: UiRect::all(px(10.)),
                ..default()
            },
            UiTransform::from_translation(Val2::percent(-50., 0.)),
            BackgroundColor(ui_color(UiOpacity::Medium)),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("city_panel"),
        ))
        .id();

    for city in City::ALL {
        spawn_panel_button(
            &mut commands,
            row,
            CityButton(city),
            city.label(),
            move |_activate: On<Activate>, mut current: ResMut<City>| {
                // set_if_neq: повторный клик по текущему городу не должен
                // перезагружать мир
                current.set_if_neq(city);
            },
        );
    }
}

/// Подсветка: активен город из ресурса, плюс hover/press как у тумблеров.
fn update_city_buttons(
    city: Res<City>,
    mut buttons: Query<(&CityButton, &Hovered, Has<Pressed>, &mut BackgroundColor)>,
) {
    for (button, hovered, is_pressed, mut background) in &mut buttons {
        let base = if button.0 == *city {
            TOGGLE_ACTIVE_COLOR
        } else {
            ui_color(UiOpacity::Heavy)
        };
        let lighten = if is_pressed {
            TOGGLE_PRESSED_LIGHTEN
        } else if hovered.get() {
            TOGGLE_HOVER_LIGHTEN
        } else {
            0.0
        };
        background.set_if_neq(BackgroundColor(base.mix(&Color::WHITE, lighten)));
    }
}
