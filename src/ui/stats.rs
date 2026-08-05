//! Панель World в левом верхнем углу: сколько пешек ещё живо, сколько демонов
//! ходит по городу, сколько душ съедено, — и два ползунка спавна демонов.
//!
//! Счётчики до этого жили только в BRP (`count Human`, `res get Telemetry`), то
//! есть смотреть на симуляцию без агентского клиента рядом было нечем.

use bevy::prelude::*;
use bevy::ui_widgets::{SliderValue, ValueChange};

use super::brp::{AgentBrpSession, BrpBadge};
use super::slider::{SliderRow, quantize, spawn_slider_row};
use super::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};
use crate::demon::{Demon, DemonSpawnStyle};
use crate::human::Human;
use crate::settings::{
    DEMON_CAP_MAX, DEMON_CAP_MIN, DEMON_CAP_STEP, DEMON_SPAWN_INTERVAL_MAX,
    DEMON_SPAWN_INTERVAL_MIN, DEMON_SPAWN_INTERVAL_STEP,
};
use crate::telemetry::Telemetry;

/// Ширина панели — как у остальных панелей с ползунками.
const PANEL_WIDTH_PX: f32 = 210.0;
/// Подпись счётчика. Светлее тусклой подписи строк-ползунков
/// (`slider.rs`, 0.75): те лежат на своей плотной подложке, а счётчики
/// читаются на фоне карты, и на бежевой Туле серый на сером пропадал.
const LABEL_COLOR: Color = Color::srgb(0.88, 0.91, 0.88);

/// Корень панели: по нему система развода с меткой BRP правит `top`.
#[derive(Component)]
struct WorldStatsPanel;

/// Какой счётчик показывает строка; компонент висит на тексте значения.
#[derive(Component, Clone, Copy)]
enum StatRow {
    Pawns,
    Demons,
    Souls,
}

/// Какой параметр спавна крутит строка-ползунок.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum SpawnRow {
    Cap,
    Interval,
}

/// Текст значения в строке-ползунке.
#[derive(Component)]
struct SpawnValueLabel(SpawnRow);

/// Ползунок строки.
#[derive(Component)]
struct SpawnSlider(SpawnRow);

pub struct UiStatsPlugin;

impl Plugin for UiStatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_stats_panel).add_systems(
            Update,
            (
                sync_world_counts,
                sync_spawn_values.run_if(resource_changed::<DemonSpawnStyle>),
                // метка BRP стоит только в агентских запусках, и только тогда
                // панели есть что обходить
                offset_below_brp_badge.run_if(resource_exists::<AgentBrpSession>),
            ),
        );
    }
}

/// Строка-счётчик: подпись слева, белое число справа — та же разметка,
/// что у шапки строки-ползунка (`slider::spawn_slider_row`).
fn count_row(label: &str, row: StatRow) -> impl Bundle {
    (
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6.),
            ..default()
        },
        children![
            (
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(LABEL_COLOR),
                UI_TEXT_SHADOW,
                // распорка: подпись забирает всю ширину, число прижимается
                // к правому краю строки
                Node {
                    flex_grow: 1.,
                    ..default()
                },
            ),
            (
                row,
                Text::new("0"),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            ),
        ],
    )
}

fn render_stats_panel(mut commands: Commands, style: Res<DemonSpawnStyle>) {
    let panel = commands
        .spawn((
            WorldStatsPanel,
            Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                // единственный свободный угол: снизу обе колонки заняты
                // панелями стилей, сверху справа — телеметрия и кнопка скорости
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(PANEL_WIDTH_PX),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("world_stats_panel"),
            // без `panel_header`: тот считает объекты карты по `MapData`,
            // а здесь счётчики свои и живут покадрово
            children![
                (
                    Text::new("World"),
                    TextFont {
                        font_size: FontSize::Px(14.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    UI_TEXT_SHADOW,
                ),
                // счётчики на своей плотной подложке, как строки-ползунки:
                // на полупрозрачном фоне панели поверх светлой карты они
                // читались заметно хуже соседних строк
                (
                    Node {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2.),
                        padding: UiRect {
                            top: px(4.),
                            right: px(8.),
                            bottom: px(6.),
                            left: px(8.),
                        },
                        ..default()
                    },
                    BackgroundColor(ui_color(UiOpacity::Heavy)),
                    children![
                        count_row("Pawns", StatRow::Pawns),
                        count_row("Demons", StatRow::Demons),
                        count_row("Souls reaped", StatRow::Souls),
                    ],
                ),
            ],
        ))
        .id();

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Max demons",
            value: slider_value(SpawnRow::Cap, &style),
            value_text: row_value(SpawnRow::Cap, &style),
            range: (DEMON_CAP_MIN, DEMON_CAP_MAX, DEMON_CAP_STEP),
        },
        SpawnValueLabel(SpawnRow::Cap),
        SpawnSlider(SpawnRow::Cap),
        on_cap_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Spawn every",
            value: slider_value(SpawnRow::Interval, &style),
            value_text: row_value(SpawnRow::Interval, &style),
            range: (
                DEMON_SPAWN_INTERVAL_MIN,
                DEMON_SPAWN_INTERVAL_MAX,
                DEMON_SPAWN_INTERVAL_STEP,
            ),
        },
        SpawnValueLabel(SpawnRow::Interval),
        SpawnSlider(SpawnRow::Interval),
        on_interval_change,
    );
}

/// Счётчики панели. `Human` снимается с человека в момент смерти, а с трупа не
/// висит вовсе, — так что `With<Human>` и есть «жив».
///
/// `iter().len()`, а не `count()`: `QueryIter` при чисто архетипном фильтре
/// (`With<_>`) — `ExactSizeIterator`, и длина берётся суммой размеров
/// архетипов, а не проходом по двадцати тысячам сущностей каждый кадр.
fn sync_world_counts(
    humans: Query<(), With<Human>>,
    demons: Query<(), With<Demon>>,
    telemetry: Res<Telemetry>,
    mut labels: Query<(&StatRow, &mut Text)>,
) {
    for (row, mut text) in &mut labels {
        let value = match row {
            StatRow::Pawns => humans.iter().len(),
            StatRow::Demons => demons.iter().len(),
            StatRow::Souls => telemetry.killed,
        };
        text.set_if_neq(Text(value.to_string()));
    }
}

/// Подписи и бегунки вслед за ресурсом — правка извне (BRP, восстановленные
/// настройки) должна двигать ползунок, а не только менять поведение.
fn sync_spawn_values(
    style: Res<DemonSpawnStyle>,
    mut commands: Commands,
    mut labels: Query<(&SpawnValueLabel, &mut Text)>,
    sliders: Query<(Entity, &SpawnSlider, &SliderValue)>,
) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
    for (slider, row, value) in &sliders {
        let target = slider_value(row.0, &style);
        if (value.0 - target).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(target));
        }
    }
}

/// Ползунки дискретные: ресурс правится только на реальной смене шага.
fn on_cap_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<DemonSpawnStyle>,
) {
    let stepped = quantize(change.value, DEMON_CAP_MIN, DEMON_CAP_MAX, DEMON_CAP_STEP);
    commands.entity(change.source).insert(SliderValue(stepped));
    if style.cap != stepped as usize {
        style.cap = stepped as usize;
    }
}

fn on_interval_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<DemonSpawnStyle>,
) {
    let stepped = quantize(
        change.value,
        DEMON_SPAWN_INTERVAL_MIN,
        DEMON_SPAWN_INTERVAL_MAX,
        DEMON_SPAWN_INTERVAL_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.interval - stepped).abs() > f32::EPSILON {
        style.interval = stepped;
    }
}

/// Текст значения строки-ползунка.
fn row_value(row: SpawnRow, style: &DemonSpawnStyle) -> String {
    match row {
        SpawnRow::Cap => style.cap.to_string(),
        SpawnRow::Interval => format!("{:.1} s", style.interval),
    }
}

/// Значение ползунка строки.
fn slider_value(row: SpawnRow, style: &DemonSpawnStyle) -> f32 {
    match row {
        SpawnRow::Cap => style.cap as f32,
        SpawnRow::Interval => style.interval,
    }
}

/// Метка BRP занимает тот же угол, но только в агентских запусках — панель
/// уступает ей место и съезжает под неё.
///
/// Высота читается из `ComputedNode`, то есть с прошлого кадра, и она в
/// **физических** пикселях, тогда как `Node::top` — в логических: без
/// `inverse_scale_factor` на retina зазор удваивается (та же ловушка, что в
/// `stack_bottom_columns`). `top` пишется только когда реально изменился —
/// `Node` не `set_if_neq`-компонент, и безусловная запись метила бы его
/// изменённым каждый кадр, заставляя `bevy_ui` пересчитывать раскладку зря.
fn offset_below_brp_badge(
    badge: Single<&ComputedNode, With<BrpBadge>>,
    panel: Single<&mut Node, With<WorldStatsPanel>>,
) {
    let top = px(UI_SCREEN_EDGE_PX_OFFSET * 2.0 + badge.size.y * badge.inverse_scale_factor);
    let mut panel = panel.into_inner();
    if panel.top != top {
        panel.top = top;
    }
}
