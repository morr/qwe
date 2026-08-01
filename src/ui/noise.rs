//! Панель Noise — параметры fbm поля хвои (`map::trees::ConiferNoiseStyle`).
//! Дебажная по назначению, поэтому видна только пока включён дебаг-слой
//! `noise` (как окно шума в zxc живёт при включённом оверлее): настраивать
//! рельеф поля вслепую, без подсвеченных массивов, бессмысленно. Примесь пород
//! (`Mix`) — не здесь, а в панели Trees: она игровая ручка вида леса, а не
//! отладочная.

use bevy::ui_widgets::{SliderValue, ValueChange};

use bevy::prelude::*;

use crate::map::ConiferNoiseStyle;
use crate::settings::{
    CONIFER_NOISE_LACUNARITY_MAX, CONIFER_NOISE_LACUNARITY_MIN, CONIFER_NOISE_LACUNARITY_STEP,
    CONIFER_NOISE_OCTAVES_MAX, CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_PERSISTENCE_MAX,
    CONIFER_NOISE_PERSISTENCE_MIN, CONIFER_NOISE_PERSISTENCE_STEP, CONIFER_NOISE_WAVELENGTH_MAX,
    CONIFER_NOISE_WAVELENGTH_MIN, CONIFER_NOISE_WAVELENGTH_STEP,
};
use crate::ui::slider::{SliderRow, quantize, spawn_slider_row};
use crate::ui::{
    DebugConiferNoise, GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity,
    UiRightColumnSlot, ui_color,
};

/// Корень панели — по нему видимость следует за тумблером `noise`.
#[derive(Component)]
struct ConiferNoisePanel;

/// Какой параметр шума показывает строка.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum NoiseRow {
    Wavelength,
    Octaves,
    Lacunarity,
    Persistence,
}

/// Текст значения в строке.
#[derive(Component)]
struct NoiseValueLabel(NoiseRow);

/// Ползунок строки.
#[derive(Component)]
struct NoiseSlider(NoiseRow);

pub struct UiConiferNoisePlugin;

impl Plugin for UiConiferNoisePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_noise_panel).add_systems(
            Update,
            (
                sync_noise_values.run_if(resource_changed::<ConiferNoiseStyle>),
                // без run_if: одна query и сравнение — дешевле, чем следить за
                // окном `resource_changed` на первом кадре
                sync_noise_panel_visibility,
            ),
        );
    }
}

fn render_noise_panel(
    mut commands: Commands,
    noise: Res<ConiferNoiseStyle>,
    enabled: Res<DebugConiferNoise>,
) {
    let panel = commands
        .spawn((
            ConiferNoisePanel,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                // тумблер восстановлен из настроек до Startup — панель сразу
                // спавнится в согласии с ним, без мигания на первом кадре
                display: if enabled.0 {
                    Display::Flex
                } else {
                    Display::None
                },
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(210.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            UiRightColumnSlot(2),
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

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Wavelength",
            value: noise.wavelength,
            value_text: row_value(NoiseRow::Wavelength, &noise),
            range: (
                CONIFER_NOISE_WAVELENGTH_MIN,
                CONIFER_NOISE_WAVELENGTH_MAX,
                CONIFER_NOISE_WAVELENGTH_STEP,
            ),
        },
        NoiseValueLabel(NoiseRow::Wavelength),
        NoiseSlider(NoiseRow::Wavelength),
        on_wavelength_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Octaves",
            value: noise.octaves as f32,
            value_text: row_value(NoiseRow::Octaves, &noise),
            range: (CONIFER_NOISE_OCTAVES_MIN, CONIFER_NOISE_OCTAVES_MAX, 1.0),
        },
        NoiseValueLabel(NoiseRow::Octaves),
        NoiseSlider(NoiseRow::Octaves),
        on_octaves_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Lacunarity",
            value: noise.lacunarity,
            value_text: row_value(NoiseRow::Lacunarity, &noise),
            range: (
                CONIFER_NOISE_LACUNARITY_MIN,
                CONIFER_NOISE_LACUNARITY_MAX,
                CONIFER_NOISE_LACUNARITY_STEP,
            ),
        },
        NoiseValueLabel(NoiseRow::Lacunarity),
        NoiseSlider(NoiseRow::Lacunarity),
        on_lacunarity_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Persistence",
            value: noise.persistence,
            value_text: row_value(NoiseRow::Persistence, &noise),
            range: (
                CONIFER_NOISE_PERSISTENCE_MIN,
                CONIFER_NOISE_PERSISTENCE_MAX,
                CONIFER_NOISE_PERSISTENCE_STEP,
            ),
        },
        NoiseValueLabel(NoiseRow::Persistence),
        NoiseSlider(NoiseRow::Persistence),
        on_persistence_change,
    );
}

/// Ползунки дискретные, как в панели Trees: ресурс правится только на реальной
/// смене шага — каждый шаг пересемплирует поле и пересобирает кроны.
fn on_wavelength_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut noise: ResMut<ConiferNoiseStyle>,
) {
    let stepped = quantize(
        change.value,
        CONIFER_NOISE_WAVELENGTH_MIN,
        CONIFER_NOISE_WAVELENGTH_MAX,
        CONIFER_NOISE_WAVELENGTH_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (noise.wavelength - stepped).abs() > f32::EPSILON {
        noise.wavelength = stepped;
    }
}

fn on_octaves_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut noise: ResMut<ConiferNoiseStyle>,
) {
    let stepped = quantize(
        change.value,
        CONIFER_NOISE_OCTAVES_MIN,
        CONIFER_NOISE_OCTAVES_MAX,
        1.0,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if noise.octaves != stepped as u32 {
        noise.octaves = stepped as u32;
    }
}

fn on_lacunarity_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut noise: ResMut<ConiferNoiseStyle>,
) {
    let stepped = quantize(
        change.value,
        CONIFER_NOISE_LACUNARITY_MIN,
        CONIFER_NOISE_LACUNARITY_MAX,
        CONIFER_NOISE_LACUNARITY_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (noise.lacunarity - stepped).abs() > f32::EPSILON {
        noise.lacunarity = stepped;
    }
}

fn on_persistence_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut noise: ResMut<ConiferNoiseStyle>,
) {
    let stepped = quantize(
        change.value,
        CONIFER_NOISE_PERSISTENCE_MIN,
        CONIFER_NOISE_PERSISTENCE_MAX,
        CONIFER_NOISE_PERSISTENCE_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (noise.persistence - stepped).abs() > f32::EPSILON {
        noise.persistence = stepped;
    }
}

/// Текст значения строки.
fn row_value(row: NoiseRow, noise: &ConiferNoiseStyle) -> String {
    match row {
        NoiseRow::Wavelength => format!("{:.0} m", noise.wavelength),
        NoiseRow::Octaves => noise.octaves.to_string(),
        NoiseRow::Lacunarity => format!("{:.1}", noise.lacunarity),
        NoiseRow::Persistence => format!("{:.2}", noise.persistence),
    }
}

/// Значение ползунка строки.
fn slider_value(row: NoiseRow, noise: &ConiferNoiseStyle) -> f32 {
    match row {
        NoiseRow::Wavelength => noise.wavelength,
        NoiseRow::Octaves => noise.octaves as f32,
        NoiseRow::Lacunarity => noise.lacunarity,
        NoiseRow::Persistence => noise.persistence,
    }
}

/// Актуализация подписей и бегунков после правки ресурса (ползунком или по
/// BRP). `SliderValue` — immutable-компонент, меняется только вставкой.
fn sync_noise_values(
    noise: Res<ConiferNoiseStyle>,
    mut labels: Query<(&NoiseValueLabel, &mut Text)>,
    sliders: Query<(Entity, &NoiseSlider, &SliderValue)>,
    mut commands: Commands,
) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &noise);
    }
    for (slider, row, value) in &sliders {
        let target = slider_value(row.0, &noise);
        if (value.0 - target).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(target));
        }
    }
}

/// Панель живёт при включённом дебаг-слое `noise` и уходит из раскладки вместе
/// с ним; правую колонку перестыкует `ui::stack_right_column`.
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
