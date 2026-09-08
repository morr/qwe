//! Секция Photo вкладки Map — сила фотографического прохода
//! (`photo::PhotoStyle`): зерно сенсора, дымка, хроматическая аберрация,
//! ореол шарпинга и кривая контраста разом. Один ползунок: ноль — прежний
//! чистый кадр, каким он был до прохода. Правка меняет юниформ прохода, ни
//! одного меша не пересобирая.

use bevy::prelude::*;

use crate::photo::PhotoStyle;
use crate::settings::{PHOTO_AMOUNT_MAX, PHOTO_AMOUNT_MIN, PHOTO_AMOUNT_STEP};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{UiBuildSet, panel_title};

pub struct UiPhotoStylePlugin;

impl Plugin for UiPhotoStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<PhotoStyle>()
            .add_systems(Startup, build_photo_section.in_set(UiBuildSet::Sections));
    }
}

fn build_photo_section(mut commands: Commands, panes: Res<SettingsPanes>, style: Res<PhotoStyle>) {
    // без счётчика объектов, как у Surfaces: проход лежит на всём кадре
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Photo,
        panel_title("Photo"),
        "photo_section",
    );

    spawn_knob(
        &mut commands,
        panel,
        "Film",
        &*style,
        SliderBinding {
            get: |style| style.amount,
            set: |style, value| style.amount = value,
            range: (PHOTO_AMOUNT_MIN, PHOTO_AMOUNT_MAX, PHOTO_AMOUNT_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
}
