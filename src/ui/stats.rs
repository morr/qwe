//! Три панели левого верхнего угла: World — сколько пешек ещё живо, сколько
//! демонов ходит по городу, сколько душ съедено; Demon под ней — ползунки
//! `DemonStyle` (кап, интервал спавна, скорость и надбавка на бросок); Human
//! под ними — разброс личных скоростей (`HumanStyle`).
//!
//! Счётчики до этого жили только в BRP (`count Human`, `res get Telemetry`), то
//! есть смотреть на симуляцию без агентского клиента рядом было нечем.

use bevy::color::Mix;
use bevy::input_focus::InputFocus;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, SliderValue, ValueChange};
use rand::Rng;

use super::brp::{AgentBrpSession, BrpBadge};
use super::slider::{SliderRow, quantize, spawn_slider_row};
use super::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};
use crate::demon::{Demon, DemonStyle};
use crate::determinism::Determinism;
use crate::human::{Human, HumanStyle};
use crate::movement::SeparationStyle;
use crate::rng::{MAX_SEED, SEED_ROLL_RANGE, WorldSeed};
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

/// Подсветка строки-кнопки — как у панелей Buildings/Trees.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

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

/// Строка-кнопка тумблера расталкивания — адресует и подсветку, и подпись.
#[derive(Component)]
struct SeparationRow;

/// Текст значения в строке тумблера.
#[derive(Component)]
struct SeparationValueLabel;

/// Строка-кнопка тумблера детерминированного режима.
#[derive(Component)]
struct DeterminismRow;

#[derive(Component)]
struct DeterminismValueLabel;

/// Поле ввода seed'а мира.
#[derive(Component)]
struct SeedField;

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
                highlight_separation_row,
                highlight_determinism_row,
                apply_seed_on_enter,
                sync_seed_field.run_if(resource_changed::<WorldSeed>),
                sync_determinism_value.run_if(resource_changed::<Determinism>),
                sync_separation_value.run_if(resource_changed::<SeparationStyle>),
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

/// Разметка строки-тумблера: подпись слева, значение справа.
fn toggle_row_node() -> Node {
    Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(6.),
        padding: UiRect {
            top: px(4.),
            right: px(8.),
            bottom: px(4.),
            left: px(8.),
        },
        ..default()
    }
}

/// Подпись строки-тумблера — распорка, прижимающая значение к правому краю.
fn toggle_row_label(text: &str) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(12.),
            ..default()
        },
        TextColor(Color::srgb(0.75, 0.78, 0.75)),
        Node {
            flex_grow: 1.,
            ..default()
        },
    )
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

/// Строка seed'а: подпись, поле ввода и кнопка перегенерации.
///
/// Ввод применяется по Enter, а не на каждое нажатие: смена seed'а
/// перезапускает мир, и перезапуск на каждой набранной цифре был бы
/// невыносим.
fn spawn_seed_row(commands: &mut Commands, seed: u64) -> Entity {
    let field = commands
        .spawn((
            SeedField,
            EditableText {
                // видимая ширина текста чуть у́же ноды — под её padding
                visible_width: Some(86.),
                allow_newlines: false,
                ..EditableText::new(seed.to_string())
            },
            TextLayout::no_wrap(),
            TextFont {
                font_size: FontSize::Px(12.),
                ..default()
            },
            TextCursorStyle::default(),
            TabIndex(0),
            Node {
                // ширина прибита, а не `flex_grow`: растущее поле выпихивало
                // кнопку перегенерации за правый край панели (панель — 210 px)
                width: px(90.),
                padding: UiRect::all(px(2.)),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
        ))
        .id();

    let row = commands
        .spawn((
            toggle_row_node(),
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![toggle_row_label("Seed")],
        ))
        .id();
    commands.entity(row).add_child(field);

    let reroll = super::spawn_panel_button(
        commands,
        row,
        (),
        "new",
        |_activate: On<Activate>, mut seed: ResMut<WorldSeed>| {
            // единственное место, где ещё нужна системная энтропия: сам
            // жребий нового мира. Диапазон девятизначный — seed должен
            // читаться с экрана и набираться руками
            seed.0 = rand::rng().random_range(0..SEED_ROLL_RANGE);
        },
    );
    let _ = reroll;
    row
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
    separation: Res<SeparationStyle>,
    determinism: Res<Determinism>,
    seed: Res<WorldSeed>,
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
        ))
        .id();

    // панель World отдельной сущностью по той же причине, что Demon ниже:
    // строке-тумблеру нужен `.observe()`, а он вешается на готовую сущность
    let world_panel = commands
        .spawn((
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
        ))
        .id();
    commands.entity(column).add_child(world_panel);

    // тумблер расталкивания пешек в кадре (`movement/separation.rs`) — в
    // World, а не в Demon/Human: механизм видо-независимый
    let separation_row = commands
        .spawn((
            Button,
            SeparationRow,
            Pickable::default(),
            // `Hovered` кормит UI-picking, `Pressed` ставит виджет — оба нужны
            Hovered::default(),
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6.),
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                (
                    Text::new("Separation"),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.78, 0.75)),
                    Node {
                        flex_grow: 1.,
                        ..default()
                    },
                ),
                (
                    SeparationValueLabel,
                    Text::new(separation_value(&separation)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(
            |_activate: On<Activate>, mut style: ResMut<SeparationStyle>| {
                style.enabled = !style.enabled;
            },
        )
        .id();
    commands.entity(world_panel).add_child(separation_row);

    // тумблер детерминированного режима и поле seed'а — под расталкиванием:
    // это тоже свойства мира целиком, а не вида
    let determinism_row = commands
        .spawn((
            Button,
            DeterminismRow,
            Pickable::default(),
            Hovered::default(),
            toggle_row_node(),
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                toggle_row_label("Deterministic"),
                (
                    DeterminismValueLabel,
                    Text::new(on_off(determinism.0)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(|_activate: On<Activate>, mut mode: ResMut<Determinism>| {
            // рестарт заказывает наблюдатель в `determinism.rs`: прогон
            // детерминирован или нет с тика 0, переключить его на ходу нельзя
            mode.0 = !mode.0;
        })
        .id();
    commands.entity(world_panel).add_child(determinism_row);
    let seed_row = spawn_seed_row(&mut commands, seed.0);
    commands.entity(world_panel).add_child(seed_row);

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

/// Подсветка строки тумблера под курсором и при нажатии (как у Buildings).
fn highlight_separation_row(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<SeparationRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.0 = row_color(lighten);
    }
}

fn highlight_determinism_row(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<DeterminismRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.0 = row_color(lighten);
    }
}

fn sync_determinism_value(
    mode: Res<Determinism>,
    mut labels: Query<&mut Text, With<DeterminismValueLabel>>,
) {
    for mut text in &mut labels {
        text.0 = on_off(mode.0).to_string();
    }
}

/// Enter в поле seed'а применяет набранное. Дальше всё делает наблюдатель за
/// `WorldSeed` в `determinism.rs`: он заказывает рестарт.
///
/// Неразобранный ввод не молчит, а откатывается к текущему seed'у — иначе
/// опечатка выглядела бы как «поле приняло, а мир не перезапустился».
fn apply_seed_on_enter(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    mut seed: ResMut<WorldSeed>,
    mut fields: Query<&mut EditableText, With<SeedField>>,
) {
    if !keys.just_pressed(KeyCode::Enter) {
        return;
    }
    let Some(focused) = focus.get() else {
        return;
    };
    let Ok(mut field) = fields.get_mut(focused) else {
        return;
    };
    match field.value().to_string().trim().parse::<u64>() {
        // потолок — `i64`: `toml` не умеет хранить больше, и seed не пережил
        // бы перезапуск приложения
        Ok(value) if value <= MAX_SEED => {
            seed.set_if_neq(WorldSeed(value));
        }
        _ => set_field_text(&mut field, seed.0),
    }
}

/// Записать в поле текст: у `EditableText` нет сеттера значения, а
/// пересоздавать компонент нельзя — потеряется настройка ширины.
fn set_field_text(field: &mut EditableText, seed: u64) {
    field.editor_mut().set_text(&seed.to_string());
    // курсор в конец — как это делает сам `EditableText::new`
    field.queue_edit(TextEdit::TextEnd(false));
}

/// Поле вслед за ресурсом: кнопка перегенерации, BRP, восстановленные
/// настройки. Пока поле в фокусе — не трогаем: иначе синхронизация затирала
/// бы то, что человек набирает.
fn sync_seed_field(
    seed: Res<WorldSeed>,
    focus: Res<InputFocus>,
    mut fields: Query<(Entity, &mut EditableText), With<SeedField>>,
) {
    for (entity, mut field) in &mut fields {
        if focus.get() == Some(entity) {
            continue;
        }
        set_field_text(&mut field, seed.0);
    }
}

/// Подпись тумблера вслед за ресурсом — клик, BRP или восстановленные
/// настройки одинаково двигают текст.
fn sync_separation_value(
    style: Res<SeparationStyle>,
    mut labels: Query<&mut Text, With<SeparationValueLabel>>,
) {
    for mut text in &mut labels {
        text.0 = separation_value(&style);
    }
}

fn separation_value(style: &SeparationStyle) -> String {
    if style.enabled { "on" } else { "off" }.to_string()
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
