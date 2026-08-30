//! Секция Noise вкладки Map — параметры fbm поля хвои
//! (`map::trees::ConiferNoiseStyle`).
//!
//! Во вкладке Map, а не Debug: поле задаёт, где на карте встанут хвойные
//! массивы, то есть это вид карты — и подбирают его рядом с секцией Trees, чья
//! доля хвои по этому полю и раскладывается.
//!
//! Ползунки видны, только пока включён слой `noise`: настраивать рельеф поля
//! вслепую, без подсвеченных массивов, нечем. Включает его первая строка секции
//! — та же, что в секции Overlays вкладки Debug, на том же ресурсе: слой
//! перечислен среди отладочных, но включать его приходится именно здесь, и
//! отсылать за этим в другую вкладку значило бы оставить секцию без
//! выключателя. Обе строки ведёт кит (`ui/knob.rs`), разойтись им негде.
//!
//! Примесь пород (`Noise mix`) — не здесь, а в секции Trees: она игровая ручка
//! вида леса, а не отладочная.

use bevy::prelude::*;

use crate::map::ConiferNoiseStyle;
use crate::settings::{
    CONIFER_NOISE_LACUNARITY_MAX, CONIFER_NOISE_LACUNARITY_MIN, CONIFER_NOISE_LACUNARITY_STEP,
    CONIFER_NOISE_OCTAVES_MAX, CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_PERSISTENCE_MAX,
    CONIFER_NOISE_PERSISTENCE_MIN, CONIFER_NOISE_PERSISTENCE_STEP, CONIFER_NOISE_WAVELENGTH_MAX,
    CONIFER_NOISE_WAVELENGTH_MIN, CONIFER_NOISE_WAVELENGTH_STEP,
};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{DebugConiferNoise, UiBuildSet, panel_title};

/// Строка-ползунок секции — по ней видимость следует за тумблером слоя.
#[derive(Component)]
struct ConiferNoiseKnobRow;

pub struct UiConiferNoisePlugin;

impl Plugin for UiConiferNoisePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<ConiferNoiseStyle>()
            .add_knobs::<DebugConiferNoise>()
            .add_systems(Startup, build_noise_section.in_set(UiBuildSet::Sections))
            .add_systems(
                Update,
                // без run_if: одна query и сравнение — дешевле, чем следить за
                // окном `resource_changed` на первом кадре
                sync_noise_knob_visibility,
            );
    }
}

fn build_noise_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    noise: Res<ConiferNoiseStyle>,
    enabled: Res<DebugConiferNoise>,
) {
    // без счётчика объектов (`panel_header`): поле определено на всей карте,
    // считать нечего
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Noise,
        panel_title("Noise"),
        "noise_section",
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Show",
        ROW_LEFT_PX,
        &*enabled,
        CycleBinding {
            cycle: |enabled: &mut DebugConiferNoise| enabled.0 = !enabled.0,
            text: |enabled| on_off(enabled.0).to_string(),
        },
    );

    spawn_noise_knob(
        &mut commands,
        panel,
        "Wavelength",
        &noise,
        SliderBinding {
            get: |noise| noise.wavelength,
            set: |noise, value| noise.wavelength = value,
            range: (
                CONIFER_NOISE_WAVELENGTH_MIN,
                CONIFER_NOISE_WAVELENGTH_MAX,
                CONIFER_NOISE_WAVELENGTH_STEP,
            ),
            text: |value| format!("{value:.0} m"),
        },
    );
    // единственная целочисленная ручка панели: и шаг, и текст без дробной
    // части, так что округление ползунка и есть само значение
    spawn_noise_knob(
        &mut commands,
        panel,
        "Octaves",
        &noise,
        SliderBinding {
            get: |noise| noise.octaves as f32,
            set: |noise, value| noise.octaves = value as u32,
            range: (CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_OCTAVES_MAX, 1.0),
            text: |value| format!("{value:.0}"),
        },
    );
    spawn_noise_knob(
        &mut commands,
        panel,
        "Lacunarity",
        &noise,
        SliderBinding {
            get: |noise| noise.lacunarity,
            set: |noise, value| noise.lacunarity = value,
            range: (
                CONIFER_NOISE_LACUNARITY_MIN,
                CONIFER_NOISE_LACUNARITY_MAX,
                CONIFER_NOISE_LACUNARITY_STEP,
            ),
            text: |value| format!("{value:.1}"),
        },
    );
    spawn_noise_knob(
        &mut commands,
        panel,
        "Persistence",
        &noise,
        SliderBinding {
            get: |noise| noise.persistence,
            set: |noise, value| noise.persistence = value,
            range: (
                CONIFER_NOISE_PERSISTENCE_MIN,
                CONIFER_NOISE_PERSISTENCE_MAX,
                CONIFER_NOISE_PERSISTENCE_STEP,
            ),
            text: |value| format!("{value:.2}"),
        },
    );
}

/// Ползунок секции: ручка кита плюс метка видимости. Кит возвращает блок
/// строки, чтобы метки вешала панель, — но здесь их носят **все** ручки
/// секции без исключения, и вешать её у каждого вызова значит четыре раза
/// написать одно и то же. Та же роль, что у `ui/trees.rs::spawn_row`,
/// довешивающего свотч.
fn spawn_noise_knob(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    noise: &ConiferNoiseStyle,
    binding: SliderBinding<ConiferNoiseStyle>,
) {
    let row = spawn_knob(commands, panel, label, noise, binding);
    commands.entity(row).insert(ConiferNoiseKnobRow);
}

/// Ползунки живут при включённом дебаг-слое `noise` и уходят из раскладки
/// вместе с ним — та же логика, что у настроек невыбранного бэкенда в Nav:
/// прячем то, что при нынешних настройках ни на что не влияет. Строка `Show`
/// остаётся: ею слой и возвращают.
fn sync_noise_knob_visibility(
    enabled: Res<DebugConiferNoise>,
    mut panels: Query<&mut Node, With<ConiferNoiseKnobRow>>,
) {
    let display = if enabled.0 {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut panels {
        if node.display != display {
            node.display = display;
        }
    }
}
