//! Панель режима отображения высоты зданий: одна строка-кнопка, листающая
//! `BuildingHeightMode` по кругу (фасадная полоса → тени → тени+тон → 2.5D).
//! Правка ресурса пересобирает зданиевые слои (`map::buildings::rebuild_buildings`).

use bevy::prelude::*;

use crate::map::{BuildingHeightMode, BuildingLean, RoofStyle};
use crate::settings::{ROOF_TEXTURE_MAX, ROOF_TEXTURE_MIN, ROOF_TEXTURE_STEP};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::ROW_LEFT_PX;
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{PanelCount, UiBuildSet, panel_header};

pub struct UiBuildingStylePlugin;

impl Plugin for UiBuildingStylePlugin {
    fn build(&self, app: &mut App) {
        // подпись вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<BuildingHeightMode>()
            .add_knobs::<BuildingLean>()
            .add_knobs::<RoofStyle>()
            .add_systems(
                Startup,
                build_buildings_section.in_set(UiBuildSet::Sections),
            );
    }
}

fn build_buildings_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    mode: Res<BuildingHeightMode>,
    lean: Res<BuildingLean>,
    roofs: Res<RoofStyle>,
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

    // куда «падает» верх дома: постоянный косой сдвиг (спутниковый кадр) или
    // лучами от надира (кадр с самолёта)
    spawn_cycle_row(
        &mut commands,
        panel,
        "Lean",
        ROW_LEFT_PX,
        &*lean,
        CycleBinding {
            cycle: |lean| *lean = lean.next(),
            text: |lean| lean.label().to_string(),
        },
    );

    // фактура кровель — юниформ материала, так что протяжка ползунка ничего
    // не пересобирает, как и Texture у поверхностей
    spawn_knob(
        &mut commands,
        panel,
        "Roof texture",
        &*roofs,
        SliderBinding {
            get: |style| style.texture,
            set: |style, value| style.texture = value,
            range: (ROOF_TEXTURE_MIN, ROOF_TEXTURE_MAX, ROOF_TEXTURE_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
}
