//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант.
//! Полей ввода в `bevy_ui` нет, поэтому каждая строка — кнопка, листающая
//! значение по кругу (как у панели деревьев); правка `RoadStyle` пересобирает
//! дорожные слои (`map::roads::rebuild_roads`).

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, RoadStyle};
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{PanelCount, UiBuildSet, panel_header};

pub struct UiRoadStylePlugin;

impl Plugin for UiRoadStylePlugin {
    fn build(&self, app: &mut App) {
        // подписи вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<RoadStyle>()
            .add_systems(Startup, build_roads_section.in_set(UiBuildSet::Sections));
    }
}

fn build_roads_section(mut commands: Commands, panes: Res<SettingsPanes>, style: Res<RoadStyle>) {
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Roads,
        panel_header("Roads", PanelCount::Roads),
        "roads_section",
    );

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
