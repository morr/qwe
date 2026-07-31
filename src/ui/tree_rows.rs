//! Панель стиля аллей (`natural=tree_row`) — отделена от панели Trees так же,
//! как Buildings: тумблер аллей целиком, состав посадки (политика размещения,
//! источник шага) и три ручки зелёной подложки. Каждая строка — кнопка,
//! листающая значение по кругу; правка `TreeRowStyle` пересобирает набор
//! деревьев и подложку (`map::mod` — та же цепочка, что у `TreeStyle`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, TreeRowPlacement, TreeRowStyle};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

/// Строки — как у соседних панелей: плотный фон поверх полупрозрачной панели.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Какое поле стиля листает кнопка — она же адресует подпись значения.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum TreeRowStyleRow {
    Enabled,
    Placement,
    Spacing,
    Join,
    Smoothing,
    Casing,
}

/// Текст значения в строке.
#[derive(Component)]
struct TreeRowStyleValueLabel(TreeRowStyleRow);

pub struct UiTreeRowStylePlugin;

impl Plugin for UiTreeRowStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_tree_row_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    // и клик по кнопке, и правка по BRP
                    sync_row_values.run_if(resource_changed::<TreeRowStyle>),
                ),
            );
    }
}

fn render_tree_row_style_panel(mut commands: Commands, style: Res<TreeRowStyle>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(210.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            UiRightColumnSlot(0),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("tree_row_style_panel"),
            children![panel_header("Tree rows", PanelCount::TreeRows)],
        ))
        .id();

    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Enabled,
        "Rows",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.enabled = !style.enabled;
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Placement,
        "Placement",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.placement = next_in(&TreeRowPlacement::ALL, style.placement);
        },
    );
    // откуда берётся шаг посадки ряда. `OSM` — из тегов `spacing`/`count`, и
    // такой ряд ползунок плотности не трогает; `slider` — теги игнорируются, и
    // ряд подчиняется ползунку наравне с лесом
    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Spacing,
        "Spacing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.osm_spacing = !style.osm_spacing;
        },
    );
    // те же три ручки, что у панели Roads, но **свои**: ломаная аллеи и ломаная
    // улицы приходят из разных данных, и подложка обязана выглядеть лесом даже
    // там, где дороги оставлены нетронутыми
    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Join,
        "Joins",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.join = next_in(&RoadJoin::ALL, style.join);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Smoothing,
        "Smoothing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.smoothing = next_in(&RoadSmoothing::ALL, style.smoothing);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeRowStyleRow::Casing,
        "Casing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeRowStyle>| {
            style.casing = !style.casing;
        },
    );
}

/// Следующее значение по кругу; незнакомое откатывается к первому.
fn next_in<T: Copy + PartialEq>(values: &[T], current: T) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .map_or(0, |index| (index + 1) % values.len());
    values[index]
}

fn spawn_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: TreeRowStyleRow,
    label: &str,
    style: &TreeRowStyle,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let button = commands
        .spawn((
            Button,
            row,
            Pickable::default(),
            // `Hovered` кормит UI-picking, `Pressed` ставит виджет — оба нужны
            Hovered::default(),
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6.),
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                (
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.78, 0.75)),
                    Node {
                        flex_grow: 1.,
                        ..default()
                    },
                ),
                (
                    TreeRowStyleValueLabel(row),
                    Text::new(row_value(row, style)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(on_activate)
        .id();
    commands.entity(panel).add_child(button);
}

fn row_value(row: TreeRowStyleRow, style: &TreeRowStyle) -> String {
    let on_off = |value: bool| (if value { "On" } else { "Off" }).to_string();
    match row {
        TreeRowStyleRow::Enabled => on_off(style.enabled),
        TreeRowStyleRow::Placement => style.placement.label().to_string(),
        TreeRowStyleRow::Spacing => (if style.osm_spacing { "OSM" } else { "slider" }).to_string(),
        TreeRowStyleRow::Join => style.join.label().to_string(),
        TreeRowStyleRow::Smoothing => style.smoothing.label().to_string(),
        TreeRowStyleRow::Casing => on_off(style.casing),
    }
}

/// Подсветка строки под курсором и при нажатии (как у соседних панелей).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<TreeRowStyleRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.0 = row_color(lighten);
    }
}

/// Актуализация подписей после смены стиля (кликом или по BRP).
fn sync_row_values(
    style: Res<TreeRowStyle>,
    mut labels: Query<(&TreeRowStyleValueLabel, &mut Text)>,
) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
}
