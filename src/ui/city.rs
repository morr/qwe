//! Выбор города — снизу по центру: кнопка с текущим городом и выпадающий
//! список остальных. Клик по пункту пишет ресурс `City`, а перезагрузку мира
//! делает `city::reload_world_on_city_change`.
//!
//! Список, а не ряд кнопок на город: городов семь, ряд занимал треть нижнего
//! края экрана и рос с каждым новым, а выбран из них всегда ровно один — это
//! и есть select. Виджет первопартийный (`FeathersMenu` + `FeathersMenuPopup`),
//! всплывающая часть сама переворачивается вверх, когда снизу нет места.

use bevy::feathers::controls::{
    FeathersMenu, FeathersMenuButton, FeathersMenuItem, FeathersMenuPopup,
};
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::ThemedText;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::city::City;
use crate::ui::{GameUiRoot, PANEL_FONT, UI_SCREEN_EDGE_PX_OFFSET, panel_background};

/// Подпись на кнопке списка — по ней синхронизация находит текст, который надо
/// перечитать из ресурса.
#[derive(Component, Default, Clone)]
struct CityLabel;

pub struct UiCityPlugin;

impl Plugin for UiCityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_city_panel)
            .add_systems(Update, sync_city_label.run_if(resource_changed::<City>));
    }
}

fn render_city_panel(mut commands: Commands, current: Res<City>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                // центрирование абсолютной ноды: левый край в середину
                // экрана, затем сдвиг на половину собственной ширины
                left: percent(50.),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                padding: UiRect::all(px(8.)),
                ..default()
            },
            UiTransform::from_translation(Val2::percent(-50., 0.)),
            panel_background(),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("city_panel"),
        ))
        .id();

    let menu = commands.spawn_scene(bsn! { @FeathersMenu }).id();
    commands.entity(panel).add_child(menu);

    let label = current.label().to_string();
    let button = commands
        .spawn_scene(bsn! {
            @FeathersMenuButton {
                @caption: bsn! { Text({label}) ThemedText CityLabel }
            }
            // ширина прибита: названия городов разной длины, и по авто-ширине
            // кнопка (а с ней и вся панель) прыгала бы при каждом выборе
            Node { width: px(140) }
            InheritableFont { font_size: {PANEL_FONT} }
        })
        .id();
    commands.entity(menu).add_child(button);

    let popup = commands.spawn_scene(bsn! { @FeathersMenuPopup }).id();
    commands.entity(menu).add_child(popup);

    for city in City::ALL {
        let name = city.label().to_string();
        let item = commands
            .spawn_scene(bsn! {
                @FeathersMenuItem {
                    @caption: bsn! { Text({name}) ThemedText }
                }
                InheritableFont { font_size: {PANEL_FONT} }
            })
            .observe(move |_activate: On<Activate>, mut current: ResMut<City>| {
                // set_if_neq: повторный выбор текущего города не должен
                // перезагружать мир
                current.set_if_neq(city);
            })
            .id();
        commands.entity(popup).add_child(item);
    }
}

/// Подпись кнопки следует за ресурсом: город меняют не только эти пункты, но и
/// `reset` настроек, и BRP.
fn sync_city_label(city: Res<City>, mut labels: Query<&mut Text, With<CityLabel>>) {
    for mut text in &mut labels {
        text.set_if_neq(Text(city.label().to_string()));
    }
}
