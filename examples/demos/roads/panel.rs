//! Панель витрины: переключатель города, ручки стиля дорог и подсказка.
//!
//! Виджеты — игровые киты (`qwe::ui`), а ручки стиля — те же пять строк, что в
//! секции Roads панели игры, привязанные к тому же `RoadStyle`: правка
//! пересобирает все примеры, так что стык, сглаживание, кант, тротуары и
//! разметку можно сравнивать на одном и том же узле.
//!
//! **Шрифт панель ставит себе сама** — `apply_panel_font` живёт в `UiPlugin`,
//! которого здесь нет, а во встроенном шрифте bevy нет кириллицы.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use qwe::city::City;
use qwe::map::{RoadJoin, RoadStyle, Smoothing};
use qwe::ui::knob::{CycleBinding, spawn_cycle_row};

use crate::overlay::NetworkOverlay;
use qwe::ui::{
    GROUP_HEADER_PAD_PX, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, button_variant,
    panel_background, panel_block_background, panel_font, panel_title, row_label,
    spawn_panel_button, ui_node,
};

/// Отступ строки-значения слева — как у строк панели игры.
const ROW_LEFT_PX: f32 = 8.0;

const HOTKEYS: &str = "колесо — зум, ЛКМ / WASD — панорама\n\
    ↑ ↓ — к соседнему примеру\n\
    F5 — нарезать срезы OSM заново";

/// Кнопка города: подсвечена у выбранного.
#[derive(Component)]
pub(crate) struct CityButton(City);

/// Строка состояния под кнопками: сколько примеров собрано или почему их нет.
#[derive(Component)]
pub(crate) struct StatusLine;

fn next_in<T: Copy + PartialEq>(all: &[T], current: T) -> T {
    let index = all.iter().position(|item| *item == current).unwrap_or(0);
    all[(index + 1) % all.len()]
}

fn on_off(value: bool) -> String {
    if value { "On" } else { "Off" }.to_string()
}

pub(crate) fn spawn_panel(
    mut commands: Commands,
    assets: Res<AssetServer>,
    city: Res<City>,
    style: Res<RoadStyle>,
    overlay: Res<NetworkOverlay>,
) {
    let panel = commands
        .spawn((
            // угол делится с меткой агентского запуска — плашка съезжает под неё
            qwe::ui::BelowBrpBadge,
            ui_node(Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                width: px(PANEL_WIDTH_PX),
                flex_direction: FlexDirection::Column,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                padding: UiRect::all(px(GROUP_HEADER_PAD_PX)),
                ..default()
            }),
            panel_background(),
            panel_font(&assets),
            Name::new("roads_panel"),
        ))
        .id();

    let header = |commands: &mut Commands, title: &str| {
        let header = commands
            .spawn((
                ui_node(Node {
                    padding: UiRect::all(px(GROUP_HEADER_PAD_PX)),
                    ..default()
                }),
                panel_block_background(),
                children![panel_title(title)],
            ))
            .id();
        commands.entity(panel).add_child(header);
    };

    header(&mut commands, "Город");
    for option in City::ALL {
        spawn_panel_button(
            &mut commands,
            panel,
            CityButton(option),
            option.label(),
            option == *city,
            move |_: On<Activate>, mut city: ResMut<City>| {
                // `set_if_neq`: повторный клик по выбранному городу не должен
                // пересобирать витрину
                city.set_if_neq(option);
            },
        );
    }
    let status = commands.spawn((row_label(""), StatusLine)).id();
    commands.entity(panel).add_child(status);

    header(&mut commands, "Roads");
    spawn_cycle_row(
        &mut commands,
        panel,
        "Joins",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.join = next_in(&RoadJoin::ALL, style.join),
            text: |style| style.join.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Smoothing",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| {
                style.smoothing = next_in(&Smoothing::ALL, style.smoothing);
            },
            text: |style| style.smoothing.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Casing",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.casing = !style.casing,
            text: |style| on_off(style.casing),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Sidewalks",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.sidewalks = !style.sidewalks,
            text: |style| on_off(style.sidewalks),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Markings",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.markings = !style.markings,
            text: |style| on_off(style.markings),
        },
    );
    // не ручка стиля, а взгляд на данные: улицы сети и их сечения
    spawn_cycle_row(
        &mut commands,
        panel,
        "Network",
        ROW_LEFT_PX,
        &*overlay,
        CycleBinding {
            cycle: |overlay: &mut NetworkOverlay| overlay.visible = !overlay.visible,
            text: |overlay| on_off(overlay.visible),
        },
    );

    let hotkeys = commands.spawn(row_label(HOTKEYS)).id();
    commands.entity(panel).add_child(hotkeys);
}

/// Подсветка кнопки выбранного города — вслед за ресурсом.
pub(crate) fn sync_city_buttons(
    city: Res<City>,
    mut buttons: Query<(&CityButton, &mut ButtonVariant)>,
) {
    for (button, mut variant) in &mut buttons {
        *variant = button_variant(button.0 == *city);
    }
}
