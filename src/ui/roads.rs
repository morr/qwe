//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант.
//! Полей ввода в `bevy_ui` нет, поэтому каждая строка — кнопка, листающая
//! значение по кругу (как у панели деревьев); правка `RoadStyle` пересобирает
//! дорожные слои (`map::roads::rebuild_roads`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, RoadStyle};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

/// Строки — как у панелей деревьев и зданий: плотный фон поверх полупрозрачной
/// панели.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Какое поле стиля листает кнопка — она же адресует подпись значения.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum RoadStyleRow {
    Join,
    Smoothing,
    Casing,
}

/// Текст значения в строке.
#[derive(Component)]
struct RoadStyleValueLabel(RoadStyleRow);

pub struct UiRoadStylePlugin;

impl Plugin for UiRoadStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_road_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    // и клик по кнопке, и правка по BRP
                    sync_row_values.run_if(resource_changed::<RoadStyle>),
                ),
            );
    }
}

fn render_road_style_panel(mut commands: Commands, style: Res<RoadStyle>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // `bottom` доедет от stack_right_column: под панелью стоят
                // Buildings и Trees, а Trees меняет высоту на ходу
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
            UiRightColumnSlot(3),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("road_style_panel"),
            children![panel_header("Roads", PanelCount::Roads)],
        ))
        .id();

    spawn_row(
        &mut commands,
        panel,
        RoadStyleRow::Join,
        "Joins",
        &style,
        |_activate: On<Activate>, mut style: ResMut<RoadStyle>| {
            style.join = next_in(&RoadJoin::ALL, style.join);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        RoadStyleRow::Smoothing,
        "Smoothing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<RoadStyle>| {
            style.smoothing = next_in(&RoadSmoothing::ALL, style.smoothing);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        RoadStyleRow::Casing,
        "Casing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<RoadStyle>| {
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
    row: RoadStyleRow,
    label: &str,
    style: &RoadStyle,
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
                    RoadStyleValueLabel(row),
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

fn row_value(row: RoadStyleRow, style: &RoadStyle) -> String {
    match row {
        RoadStyleRow::Join => style.join.label().to_string(),
        RoadStyleRow::Smoothing => style.smoothing.label().to_string(),
        RoadStyleRow::Casing => if style.casing { "On" } else { "Off" }.to_string(),
    }
}

/// Подсветка строки под курсором и при нажатии (как у панели деревьев).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<RoadStyleRow>>,
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
fn sync_row_values(style: Res<RoadStyle>, mut labels: Query<(&RoadStyleValueLabel, &mut Text)>) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
}
