//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант.
//! Полей ввода в `bevy_ui` нет, поэтому каждая строка — кнопка, листающая
//! значение по кругу (как у панели деревьев); правка `RoadStyle` пересобирает
//! дорожные слои (`map::roads::rebuild_roads`).

use bevy::ecs::system::IntoObserverSystem;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::map::{RoadJoin, RoadSmoothing, RoadStyle};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off, spawn_value_row};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

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
                // и клик по кнопке, и правка по BRP
                sync_row_values.run_if(resource_changed::<RoadStyle>),
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

fn spawn_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: RoadStyleRow,
    label: &str,
    style: &RoadStyle,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let button = spawn_value_row(
        commands,
        panel,
        label,
        ROW_LEFT_PX,
        RoadStyleValueLabel(row),
        row_value(row, style),
        on_activate,
    );
    commands.entity(button).insert(row);
}

fn row_value(row: RoadStyleRow, style: &RoadStyle) -> String {
    match row {
        RoadStyleRow::Join => style.join.label().to_string(),
        RoadStyleRow::Smoothing => style.smoothing.label().to_string(),
        RoadStyleRow::Casing => on_off(style.casing).to_string(),
    }
}

/// Актуализация подписей после смены стиля (кликом или по BRP).
fn sync_row_values(style: Res<RoadStyle>, mut labels: Query<(&RoadStyleValueLabel, &mut Text)>) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
}
