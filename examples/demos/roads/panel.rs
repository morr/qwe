//! Панель витрины: переключатель города, ручки стиля дорог и подсказка.
//!
//! Виджеты — игровые киты (`qwe::ui`), а ручки — те же строки, что в секциях
//! Roads и Road paint панели игры, привязанные к тем же ресурсам: ползунки
//! формы (`RoadShape`, таблица `qwe::ui::shape_knobs`) и тумблеры слоёв
//! (`RoadStyle`) пересобирают все примеры, так что форму и краску можно
//! сравнивать на одном и том же узле. Ползунки Paint, Wear и Turn wear —
//! `RoadPaintStyle`, юниформы материалов: протяжка ничего не пересобирает.
//!
//! **Шрифт панель ставит себе сама** — `apply_panel_font` живёт в `UiPlugin`,
//! которого здесь нет, а во встроенном шрифте bevy нет кириллицы.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use qwe::city::City;
use qwe::map::{
    CrossingMode, PAINT_MAX, PAINT_MIN, PAINT_STEP, RoadPaintStyle, RoadShape, RoadStyle,
    TURN_WEAR_MAX, TURN_WEAR_MIN, TURN_WEAR_STEP, WEAR_MAX, WEAR_MIN, WEAR_STEP,
};
use qwe::ui::knob::{CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};

use crate::overlay::NetworkOverlay;
use qwe::ui::{
    GROUP_HEADER_PAD_PX, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, button_variant,
    panel_background, panel_block_background, panel_font, panel_title, row_label, shape_knobs,
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
    shape: Res<RoadShape>,
    style: Res<RoadStyle>,
    paint: Res<RoadPaintStyle>,
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
    // форма — те же ползунки, что в игре; карта витрины следует за ними после
    // паузы, как игра (`RoadShapeOnMap`)
    for (label, binding) in shape_knobs() {
        spawn_knob(&mut commands, panel, label, &*shape, binding);
    }
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

    header(&mut commands, "Road paint");
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
    // краска и колея — те же ползунки, что в игре, и тоже без пересборки
    spawn_knob(
        &mut commands,
        panel,
        "Paint",
        &*paint,
        SliderBinding {
            get: |paint: &RoadPaintStyle| paint.paint,
            set: |paint, value| paint.paint = value,
            range: (PAINT_MIN, PAINT_MAX, PAINT_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Crossings",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| {
                style.crossings = next_in(&CrossingMode::ALL, style.crossings);
            },
            text: |style| style.crossings.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Stop lines",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.stop_lines = !style.stop_lines,
            text: |style| on_off(style.stop_lines),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Arrows",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style: &mut RoadStyle| style.arrows = !style.arrows,
            text: |style| on_off(style.arrows),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Wear",
        &*paint,
        SliderBinding {
            get: |paint: &RoadPaintStyle| paint.wear,
            set: |paint, value| paint.wear = value,
            range: (WEAR_MIN, WEAR_MAX, WEAR_STEP),
            text: |value| format!("{:.1}%", value * 100.),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Turn wear",
        &*paint,
        SliderBinding {
            get: |paint: &RoadPaintStyle| paint.turn_wear,
            set: |paint, value| paint.turn_wear = value,
            range: (TURN_WEAR_MIN, TURN_WEAR_MAX, TURN_WEAR_STEP),
            text: |value| format!("{:.1}%", value * 100.),
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
