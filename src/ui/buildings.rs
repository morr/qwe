//! Панель режима отображения высоты зданий: одна строка-кнопка, листающая
//! `BuildingHeightMode` по кругу (фасадная полоса → тени → тени+тон → 2.5D).
//! Правка ресурса пересобирает зданиевые слои (`map::buildings::rebuild_buildings`).

use bevy::prelude::*;

use crate::map::BuildingHeightMode;
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::ROW_LEFT_PX;
use crate::ui::{GameUiRoot, PanelCount, UiRightColumn, panel_header, right_panel};

pub struct UiBuildingStylePlugin;

impl Plugin for UiBuildingStylePlugin {
    fn build(&self, app: &mut App) {
        // подпись вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<BuildingHeightMode>()
            .add_systems(Startup, render_building_style_panel);
    }
}

fn render_building_style_panel(mut commands: Commands, mode: Res<BuildingHeightMode>) {
    let panel = commands
        .spawn((
            right_panel(UiRightColumn::Buildings),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("building_style_panel"),
            children![panel_header("Buildings", PanelCount::Buildings)],
        ))
        .id();

    spawn_cycle_row(
        &mut commands,
        panel,
        "Height",
        ROW_LEFT_PX,
        &*mode,
        CycleBinding {
            cycle: |mode| *mode = mode.next(),
            text: |mode| mode.label().to_string(),
        },
    );
}
