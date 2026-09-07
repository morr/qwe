//! Секция Surfaces вкладки Map — сила процедурной фактуры поверхностей
//! (`map::surface::SurfaceStyle`): земля, зелень, песок, вода, асфальт и
//! тротуар. Один ползунок: ноль — плоские заливки, какими они были до
//! фактуры. Правка меняет юниформы материалов, меши не пересобираются, так
//! что протяжка ползунка ничего не стоит.

use bevy::prelude::*;

use crate::map::{SURFACE_TEXTURE_MAX, SURFACE_TEXTURE_MIN, SURFACE_TEXTURE_STEP, SurfaceStyle};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{UiBuildSet, panel_title};

pub struct UiSurfaceStylePlugin;

impl Plugin for UiSurfaceStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<SurfaceStyle>()
            .add_systems(Startup, build_surfaces_section.in_set(UiBuildSet::Sections));
    }
}

fn build_surfaces_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    style: Res<SurfaceStyle>,
) {
    // без счётчика объектов (`panel_header`): фактура лежит на всей карте,
    // считать тут нечего
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Surfaces,
        panel_title("Surfaces"),
        "surfaces_section",
    );

    spawn_knob(
        &mut commands,
        panel,
        "Texture",
        &*style,
        SliderBinding {
            get: |style| style.texture,
            set: |style, value| style.texture = value,
            range: (
                SURFACE_TEXTURE_MIN,
                SURFACE_TEXTURE_MAX,
                SURFACE_TEXTURE_STEP,
            ),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
}
