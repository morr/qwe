//! Панель режима отображения высоты зданий: одна строка-кнопка, листающая
//! `BuildingHeightMode` по кругу (фасадная полоса → тени → тени+тон → 2.5D).
//! Правка ресурса пересобирает зданиевые слои (`map::buildings::rebuild_buildings`).

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::map::BuildingHeightMode;
use crate::ui::rows::{ROW_LEFT_PX, spawn_value_row};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

/// Текст значения в строке.
#[derive(Component)]
struct BuildingHeightValueLabel;

pub struct UiBuildingStylePlugin;

impl Plugin for UiBuildingStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_building_style_panel)
            .add_systems(
                Update,
                // и клик по кнопке, и правка по BRP
                sync_row_value.run_if(resource_changed::<BuildingHeightMode>),
            );
    }
}

fn render_building_style_panel(mut commands: Commands, mode: Res<BuildingHeightMode>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // `bottom` доедет от stack_right_column по высоте панели Trees
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
            UiRightColumnSlot(2),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("building_style_panel"),
            children![panel_header("Buildings", PanelCount::Buildings)],
        ))
        .id();

    spawn_value_row(
        &mut commands,
        panel,
        "Height",
        ROW_LEFT_PX,
        BuildingHeightValueLabel,
        mode.label().to_string(),
        |_activate: On<Activate>, mut mode: ResMut<BuildingHeightMode>| {
            *mode = mode.next();
        },
    );
}

/// Актуализация подписи после смены режима (кликом или по BRP).
fn sync_row_value(
    mode: Res<BuildingHeightMode>,
    mut labels: Query<&mut Text, With<BuildingHeightValueLabel>>,
) {
    for mut text in &mut labels {
        text.0 = mode.label().to_string();
    }
}
