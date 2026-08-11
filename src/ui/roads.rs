//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант.
//! Полей ввода в `bevy_ui` нет, поэтому каждая строка — кнопка, листающая
//! значение по кругу (как у панели деревьев); правка `RoadStyle` пересобирает
//! дорожные слои (`map::roads::rebuild_roads`).

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, RoadStyle};
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

pub struct UiRoadStylePlugin;

impl Plugin for UiRoadStylePlugin {
    fn build(&self, app: &mut App) {
        // подписи вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<RoadStyle>()
            .add_systems(Startup, render_road_style_panel);
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

    spawn_cycle_row(
        &mut commands,
        panel,
        "Joins",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.join = next_in(&RoadJoin::ALL, style.join),
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
            cycle: |style| style.smoothing = next_in(&RoadSmoothing::ALL, style.smoothing),
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
            cycle: |style| style.casing = !style.casing,
            text: |style| on_off(style.casing).to_string(),
        },
    );
}
