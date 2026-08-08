//! Панель стиля аллей (`natural=tree_row`) — отделена от панели Trees так же,
//! как Buildings: тумблер аллей целиком, состав посадки (политика размещения,
//! источник шага) и три ручки зелёной подложки. Каждая строка — кнопка,
//! листающая значение по кругу; правка `TreeRowStyle` пересобирает набор
//! деревьев и подложку (`map::mod` — та же цепочка, что у `TreeStyle`).

use bevy::ecs::system::IntoObserverSystem;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::map::{RoadJoin, RoadSmoothing, TreeRowPlacement, TreeRowStyle};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off, spawn_value_row};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

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
                // и клик по кнопке, и правка по BRP
                sync_row_values.run_if(resource_changed::<TreeRowStyle>),
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

fn spawn_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: TreeRowStyleRow,
    label: &str,
    style: &TreeRowStyle,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let button = spawn_value_row(
        commands,
        panel,
        label,
        ROW_LEFT_PX,
        TreeRowStyleValueLabel(row),
        row_value(row, style),
        on_activate,
    );
    commands.entity(button).insert(row);
}

fn row_value(row: TreeRowStyleRow, style: &TreeRowStyle) -> String {
    match row {
        TreeRowStyleRow::Enabled => on_off(style.enabled).to_string(),
        TreeRowStyleRow::Placement => style.placement.label().to_string(),
        TreeRowStyleRow::Spacing => (if style.osm_spacing { "OSM" } else { "slider" }).to_string(),
        TreeRowStyleRow::Join => style.join.label().to_string(),
        TreeRowStyleRow::Smoothing => style.smoothing.label().to_string(),
        TreeRowStyleRow::Casing => on_off(style.casing).to_string(),
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
