//! Панель режима отображения высоты зданий: одна строка-кнопка, листающая
//! `BuildingHeightMode` по кругу (фасадная полоса → тени → тени+тон → 2.5D).
//! Правка ресурса пересобирает зданиевые слои (`map::buildings::rebuild_buildings`).

use bevy::prelude::*;

use crate::map::BuildingHeightMode;
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::ROW_LEFT_PX;
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{PanelCount, UiBuildSet, panel_header};

pub struct UiBuildingStylePlugin;

impl Plugin for UiBuildingStylePlugin {
    fn build(&self, app: &mut App) {
        // подпись вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<BuildingHeightMode>().add_systems(
            Startup,
            build_buildings_section.in_set(UiBuildSet::Sections),
        );
    }
}

fn build_buildings_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    mode: Res<BuildingHeightMode>,
) {
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Buildings,
        panel_header("Buildings", PanelCount::Buildings),
        "buildings_section",
    );

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
