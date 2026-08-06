//! Три панели левого верхнего угла: World — сколько пешек ещё живо, сколько
//! демонов ходит по городу, сколько душ съедено; Demon под ней — ползунки
//! `DemonStyle` (кап, интервал спавна, скорость и надбавка на бросок); Human
//! под ними — разброс личных скоростей (`HumanStyle`).
//!
//! Счётчики до этого жили только в BRP (`count Human`, `res get Telemetry`), то
//! есть смотреть на симуляцию без агентского клиента рядом было нечем.

use bevy::prelude::*;
use bevy::ui_widgets::{SliderValue, ValueChange};

use super::brp::{AgentBrpSession, BrpBadge};
use super::slider::{SliderRow, quantize, spawn_slider_row};
use super::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};
use crate::demon::{Demon, DemonStyle};
use crate::human::{Human, HumanStyle};
use crate::settings::{
    DEMON_CAP_MAX, DEMON_CAP_MIN, DEMON_CAP_STEP, DEMON_LUNGE_BOOST_MAX, DEMON_LUNGE_BOOST_MIN,
    DEMON_LUNGE_BOOST_STEP, DEMON_SPAWN_INTERVAL_MAX, DEMON_SPAWN_INTERVAL_MIN,
    DEMON_SPAWN_INTERVAL_STEP, DEMON_SPEED_FACTOR_MAX, DEMON_SPEED_FACTOR_MIN,
    DEMON_SPEED_FACTOR_STEP, HUMAN_SPEED_SPREAD_MAX, HUMAN_SPEED_SPREAD_MIN,
    HUMAN_SPEED_SPREAD_STEP,
};
use crate::telemetry::Telemetry;

/// Ширина панелей — как у остальных панелей с ползунками.
const PANEL_WIDTH_PX: f32 = 210.0;
/// Подпись счётчика. Светлее тусклой подписи строк-ползунков
/// (`slider.rs`, 0.75): те лежат на своей плотной подложке, а счётчики
/// читаются на фоне карты, и на бежевой Туле серый на сером пропадал.
const LABEL_COLOR: Color = Color::srgb(0.88, 0.91, 0.88);

/// Колонка обеих панелей: по ней система развода с меткой BRP правит `top`.
/// Панели внутри неё стыкует обычный флекс — в отличие от нижних колонок
/// (`stack_bottom_columns`), которым приходится считать высоты вручную, потому
/// что растут они вверх, от края экрана.
#[derive(Component)]
struct TopLeftColumn;

/// Какой счётчик показывает строка; компонент висит на тексте значения.
#[derive(Component, Clone, Copy)]
enum StatRow {
    Pawns,
    Demons,
    Souls,
}

/// Какое поле `DemonStyle` крутит строка-ползунок.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum DemonRow {
    Cap,
    Interval,
    Speed,
    Lunge,
}

/// Текст значения в строке-ползунке.
#[derive(Component)]
struct DemonValueLabel(DemonRow);

/// Ползунок строки.
#[derive(Component)]
struct DemonSlider(DemonRow);

/// Панель Human — одна строка, поэтому без enum'а строк: пара маркеров на
/// текст значения и на сам бегунок. Появится вторая — заводить `HumanRow`.
#[derive(Component)]
struct SpreadValueLabel;

#[derive(Component)]
struct SpreadSlider;

pub struct UiStatsPlugin;

impl Plugin for UiStatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_stats_panel).add_systems(
            Update,
            (
                sync_world_counts,
                sync_demon_values.run_if(resource_changed::<DemonStyle>),
                sync_human_values.run_if(resource_changed::<HumanStyle>),
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

/// Тело панели этой колонки: столбец на полупрозрачной подложке.
fn panel_node() -> Node {
    Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: px(4.),
        padding: UiRect::all(px(10.)),
        ..default()
    }
}

/// Заголовок панели. Не `super::panel_header`: тот считает объекты карты по
/// `MapData`, а этим панелям считать в заголовке нечего.
fn panel_title(title: &str) -> impl Bundle {
    (
        Text::new(title),
        TextFont {
            font_size: FontSize::Px(14.),
            ..default()
        },
        TextColor(Color::WHITE),
        UI_TEXT_SHADOW,
    )
}

fn render_stats_panel(
    mut commands: Commands,
    style: Res<DemonStyle>,
    human_style: Res<HumanStyle>,
) {
    let column = commands
        .spawn((
            TopLeftColumn,
            Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                // единственный свободный угол: снизу обе колонки заняты
                // панелями стилей, сверху справа — телеметрия и кнопка скорости
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                width: px(PANEL_WIDTH_PX),
                ..default()
            },
            GameUiRoot,
            Visibility::Hidden,
            Name::new("world_panels"),
            children![(
                panel_node(),
                BackgroundColor(ui_color(UiOpacity::Medium)),
                Name::new("world_stats_panel"),
                children![
                    panel_title("World"),
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
            )],
        ))
        .id();

    // панель Demon отдельной сущностью, а не внутри `children!`:
    // `spawn_slider_row` берёт родителя сущностью, а там её ещё нет
    let panel = commands
        .spawn((
            panel_node(),
            BackgroundColor(ui_color(UiOpacity::Medium)),
            Name::new("demon_style_panel"),
            children![panel_title("Demon")],
        ))
        .id();
    commands.entity(column).add_child(panel);

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Max demons",
            value: slider_value(DemonRow::Cap, &style),
            value_text: row_value(DemonRow::Cap, &style),
            range: (DEMON_CAP_MIN, DEMON_CAP_MAX, DEMON_CAP_STEP),
        },
        DemonValueLabel(DemonRow::Cap),
        DemonSlider(DemonRow::Cap),
        on_cap_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Spawn every",
            value: slider_value(DemonRow::Interval, &style),
            value_text: row_value(DemonRow::Interval, &style),
            range: (
                DEMON_SPAWN_INTERVAL_MIN,
                DEMON_SPAWN_INTERVAL_MAX,
                DEMON_SPAWN_INTERVAL_STEP,
            ),
        },
        DemonValueLabel(DemonRow::Interval),
        DemonSlider(DemonRow::Interval),
        on_interval_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Speed",
            value: slider_value(DemonRow::Speed, &style),
            value_text: row_value(DemonRow::Speed, &style),
            range: (
                DEMON_SPEED_FACTOR_MIN,
                DEMON_SPEED_FACTOR_MAX,
                DEMON_SPEED_FACTOR_STEP,
            ),
        },
        DemonValueLabel(DemonRow::Speed),
        DemonSlider(DemonRow::Speed),
        on_speed_change,
    );
    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Lunge boost",
            value: slider_value(DemonRow::Lunge, &style),
            value_text: row_value(DemonRow::Lunge, &style),
            range: (
                DEMON_LUNGE_BOOST_MIN,
                DEMON_LUNGE_BOOST_MAX,
                DEMON_LUNGE_BOOST_STEP,
            ),
        },
        DemonValueLabel(DemonRow::Lunge),
        DemonSlider(DemonRow::Lunge),
        on_lunge_change,
    );

    let human_panel = commands
        .spawn((
            panel_node(),
            BackgroundColor(ui_color(UiOpacity::Medium)),
            Name::new("human_style_panel"),
            children![panel_title("Human")],
        ))
        .id();
    commands.entity(column).add_child(human_panel);

    spawn_slider_row(
        &mut commands,
        human_panel,
        SliderRow {
            label: "Speed spread",
            value: human_style.spread,
            value_text: spread_value(&human_style),
            range: (
                HUMAN_SPEED_SPREAD_MIN,
                HUMAN_SPEED_SPREAD_MAX,
                HUMAN_SPEED_SPREAD_STEP,
            ),
        },
        SpreadValueLabel,
        SpreadSlider,
        on_spread_change,
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
fn sync_demon_values(
    style: Res<DemonStyle>,
    mut commands: Commands,
    mut labels: Query<(&DemonValueLabel, &mut Text)>,
    sliders: Query<(Entity, &DemonSlider, &SliderValue)>,
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
    mut style: ResMut<DemonStyle>,
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
    mut style: ResMut<DemonStyle>,
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

fn on_speed_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<DemonStyle>,
) {
    let stepped = quantize(
        change.value,
        DEMON_SPEED_FACTOR_MIN,
        DEMON_SPEED_FACTOR_MAX,
        DEMON_SPEED_FACTOR_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.speed - stepped).abs() > f32::EPSILON {
        style.speed = stepped;
    }
}

fn on_lunge_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<DemonStyle>,
) {
    let stepped = quantize(
        change.value,
        DEMON_LUNGE_BOOST_MIN,
        DEMON_LUNGE_BOOST_MAX,
        DEMON_LUNGE_BOOST_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.lunge - stepped).abs() > f32::EPSILON {
        style.lunge = stepped;
    }
}

/// То же для панели Human: подпись и бегунок вслед за ресурсом.
fn sync_human_values(
    style: Res<HumanStyle>,
    mut commands: Commands,
    mut label: Query<&mut Text, With<SpreadValueLabel>>,
    slider: Query<(Entity, &SliderValue), With<SpreadSlider>>,
) {
    for mut text in &mut label {
        text.0 = spread_value(&style);
    }
    for (entity, value) in &slider {
        if (value.0 - style.spread).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(style.spread));
        }
    }
}

fn on_spread_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<HumanStyle>,
) {
    let stepped = quantize(
        change.value,
        HUMAN_SPEED_SPREAD_MIN,
        HUMAN_SPEED_SPREAD_MAX,
        HUMAN_SPEED_SPREAD_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.spread - stepped).abs() > f32::EPSILON {
        style.spread = stepped;
    }
}

/// Значение строки разброса. Со знаком, потому что это полуширина: «15%»
/// читалось бы как «все на 15% быстрее». Знак пишется как ASCII `+/-`, а не
/// «±»: встроенный шрифт (фича `default_font`) — узкая подвыборка, и всё за
/// пределами ASCII рисуется на панели пустым квадратом.
fn spread_value(style: &HumanStyle) -> String {
    format!("+/-{:.0}%", style.spread * 100.0)
}

/// Текст значения строки-ползунка. Скорость и бросок — проценты: множитель
/// «1.3» на панели ничего не сообщает, «130%» и «+30%» читаются сразу.
fn row_value(row: DemonRow, style: &DemonStyle) -> String {
    match row {
        DemonRow::Cap => style.cap.to_string(),
        DemonRow::Interval => format!("{:.1} s", style.interval),
        DemonRow::Speed => format!("{:.0}%", style.speed * 100.0),
        DemonRow::Lunge => format!("+{:.0}%", style.lunge * 100.0),
    }
}

/// Значение ползунка строки.
fn slider_value(row: DemonRow, style: &DemonStyle) -> f32 {
    match row {
        DemonRow::Cap => style.cap as f32,
        DemonRow::Interval => style.interval,
        DemonRow::Speed => style.speed,
        DemonRow::Lunge => style.lunge,
    }
}

/// Метка BRP занимает тот же угол, но только в агентских запусках — колонка
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
    panel: Single<&mut Node, With<TopLeftColumn>>,
) {
    let top = px(UI_SCREEN_EDGE_PX_OFFSET * 2.0 + badge.size.y * badge.inverse_scale_factor);
    let mut panel = panel.into_inner();
    if panel.top != top {
        panel.top = top;
    }
}
