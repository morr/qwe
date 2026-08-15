//! Панель выбора города — снизу по центру: кнопка на город, активна кнопка
//! текущего. Клик пишет ресурс `City`, а перезагрузку мира делает
//! `city::reload_world_on_city_change`.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::city::City;
use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, button_variant, spawn_panel_button, ui_color,
};

/// Какой город выбирает кнопка.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct CityButton(City);

pub struct UiCityPlugin;

impl Plugin for UiCityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_city_panel)
            .add_systems(Update, sync_city_buttons.run_if(resource_changed::<City>));
    }
}

fn render_city_panel(mut commands: Commands, current: Res<City>) {
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
            city == *current,
            move |_activate: On<Activate>, mut current: ResMut<City>| {
                // set_if_neq: повторный клик по текущему городу не должен
                // перезагружать мир
                current.set_if_neq(city);
            },
        );
    }
}

/// Зелёной стоит кнопка текущего города. Наведение и нажатие ведёт feathers
/// сама — здесь остаётся только «активность», и та меняется лишь вместе с
/// ресурсом.
fn sync_city_buttons(city: Res<City>, mut buttons: Query<(&CityButton, &mut ButtonVariant)>) {
    for (button, mut variant) in &mut buttons {
        variant.set_if_neq(button_variant(button.0 == *city));
    }
}
