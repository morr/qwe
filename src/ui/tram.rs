//! Панель стиля трамвайного пути: стык на изломе, сглаживание осевой и
//! плотность шпал. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу (как у панели дорог); правка `TramStyle`
//! пересобирает трамвайный меш (`map::tram::rebuild_tram`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, TieDensity, TramStyle};
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
enum TramStyleRow {
    Join,
    Smoothing,
    Ties,
}

/// Текст значения в строке.
#[derive(Component)]
struct TramStyleValueLabel(TramStyleRow);

pub struct UiTramStylePlugin;

impl Plugin for UiTramStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_tram_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    // и клик по кнопке, и правка по BRP
                    sync_row_values.run_if(resource_changed::<TramStyle>),
                ),
            );
    }
}

fn render_tram_style_panel(mut commands: Commands, style: Res<TramStyle>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // `bottom` доедет от stack_right_column: под панелью стоят
                // Roads, Buildings и Trees, а Trees меняет высоту на ходу
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
            UiRightColumnSlot(5),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("tram_style_panel"),
            children![panel_header("Tram", PanelCount::Trams)],
        ))
        .id();

    spawn_row(
        &mut commands,
        panel,
        TramStyleRow::Join,
        "Joins",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TramStyle>| {
            style.join = next_in(&RoadJoin::ALL, style.join);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TramStyleRow::Smoothing,
        "Smoothing",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TramStyle>| {
            style.smoothing = next_in(&RoadSmoothing::ALL, style.smoothing);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TramStyleRow::Ties,
        "Ties",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TramStyle>| {
            style.ties = next_in(&TieDensity::ALL, style.ties);
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
    row: TramStyleRow,
    label: &str,
    style: &TramStyle,
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
                    TramStyleValueLabel(row),
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

fn row_value(row: TramStyleRow, style: &TramStyle) -> String {
    match row {
        TramStyleRow::Join => style.join.label().to_string(),
        TramStyleRow::Smoothing => style.smoothing.label().to_string(),
        TramStyleRow::Ties => style.ties.label().to_string(),
    }
}

/// Подсветка строки под курсором и при нажатии (как у панели дорог).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<TramStyleRow>>,
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
fn sync_row_values(style: Res<TramStyle>, mut labels: Query<(&TramStyleValueLabel, &mut Text)>) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
}
