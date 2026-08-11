//! Панель Noise — параметры fbm поля хвои (`map::trees::ConiferNoiseStyle`).
//! Дебажная по назначению, поэтому видна только пока включён дебаг-слой
//! `noise` (как окно шума в zxc живёт при включённом оверлее): настраивать
//! рельеф поля вслепую, без подсвеченных массивов, бессмысленно. Примесь пород
//! (`Mix`) — не здесь, а в панели Trees: она игровая ручка вида леса, а не
//! отладочная.

use bevy::prelude::*;

use crate::map::ConiferNoiseStyle;
use crate::settings::{
    CONIFER_NOISE_LACUNARITY_MAX, CONIFER_NOISE_LACUNARITY_MIN, CONIFER_NOISE_LACUNARITY_STEP,
    CONIFER_NOISE_OCTAVES_MAX, CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_PERSISTENCE_MAX,
    CONIFER_NOISE_PERSISTENCE_MIN, CONIFER_NOISE_PERSISTENCE_STEP, CONIFER_NOISE_WAVELENGTH_MAX,
    CONIFER_NOISE_WAVELENGTH_MIN, CONIFER_NOISE_WAVELENGTH_STEP,
};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::{
    DebugConiferNoise, GameUiRoot, UI_TEXT_SHADOW, UiLeftColumn, UiOpacity, left_panel, ui_color,
};

/// Корень панели — по нему видимость следует за тумблером `noise`.
#[derive(Component)]
struct ConiferNoisePanel;

pub struct UiConiferNoisePlugin;

impl Plugin for UiConiferNoisePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<ConiferNoiseStyle>()
            .add_systems(Startup, render_noise_panel)
            .add_systems(
                Update,
                // без run_if: одна query и сравнение — дешевле, чем следить за
                // окном `resource_changed` на первом кадре
                sync_noise_panel_visibility,
            );
    }
}

fn render_noise_panel(
    mut commands: Commands,
    noise: Res<ConiferNoiseStyle>,
    enabled: Res<DebugConiferNoise>,
) {
    // левая колонка: правая панелями стилей забита до самого верха
    let (mut node, slot) = left_panel(UiLeftColumn::Noise);
    // тумблер восстановлен из настроек до Startup — панель сразу спавнится в
    // согласии с ним, без мигания на первом кадре
    node.display = if enabled.0 {
        Display::Flex
    } else {
        Display::None
    };

    let panel = commands
        .spawn((
            ConiferNoisePanel,
            node,
            BackgroundColor(ui_color(UiOpacity::Medium)),
            slot,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("conifer_noise_panel"),
            // без счётчика объектов (`panel_header`): поле определено на всей
            // карте, считать нечего
            children![(
                Text::new("Noise"),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            )],
        ))
        .id();

    spawn_knob(
        &mut commands,
        panel,
        "Wavelength",
        &*noise,
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
    spawn_knob(
        &mut commands,
        panel,
        "Octaves",
        &*noise,
        SliderBinding {
            get: |noise| noise.octaves as f32,
            set: |noise, value| noise.octaves = value as u32,
            range: (CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_OCTAVES_MAX, 1.0),
            text: |value| format!("{value:.0}"),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Lacunarity",
        &*noise,
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
    spawn_knob(
        &mut commands,
        panel,
        "Persistence",
        &*noise,
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

/// Панель живёт при включённом дебаг-слое `noise` и уходит из раскладки вместе
/// с ним; левую колонку перестыкует `ui::stack_bottom_columns`.
fn sync_noise_panel_visibility(
    enabled: Res<DebugConiferNoise>,
    mut panels: Query<&mut Node, With<ConiferNoisePanel>>,
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
