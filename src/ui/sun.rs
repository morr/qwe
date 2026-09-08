//! Секция Sun вкладки Map — час съёмки (`map::SunStyle`): азимут и высота
//! солнца. Оба ползунка пересобирают всё освещённое разом — зданиевые слои с
//! тенями, кроны деревьев, машины и юниформ материала кровель, — поэтому
//! протяжка стоит куда дороже соседних секций и шаг у неё крупный.

use bevy::prelude::*;

use crate::map::SunStyle;
use crate::settings::{
    SUN_AZIMUTH_MAX, SUN_AZIMUTH_MIN, SUN_AZIMUTH_STEP, SUN_ELEVATION_MAX, SUN_ELEVATION_MIN,
    SUN_ELEVATION_STEP,
};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{UiBuildSet, panel_title};

pub struct UiSunStylePlugin;

impl Plugin for UiSunStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<SunStyle>()
            .add_systems(Startup, build_sun_section.in_set(UiBuildSet::Sections));
    }
}

fn build_sun_section(mut commands: Commands, panes: Res<SettingsPanes>, sun: Res<SunStyle>) {
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Sun,
        panel_title("Sun"),
        "sun_section",
    );

    spawn_knob(
        &mut commands,
        panel,
        "Azimuth",
        &*sun,
        SliderBinding {
            get: |sun| sun.azimuth,
            set: |sun, value| sun.azimuth = value,
            range: (SUN_AZIMUTH_MIN, SUN_AZIMUTH_MAX, SUN_AZIMUTH_STEP),
            text: |value| format!("{value:.0}°"),
        },
    );

    spawn_knob(
        &mut commands,
        panel,
        "Elevation",
        &*sun,
        SliderBinding {
            get: |sun| sun.elevation,
            set: |sun, value| sun.elevation = value,
            range: (SUN_ELEVATION_MIN, SUN_ELEVATION_MAX, SUN_ELEVATION_STEP),
            text: |value| format!("{value:.0}°"),
        },
    );
}
