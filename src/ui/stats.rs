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
use crate::movement::{
    SeparationLab, SeparationStyle, SlotLab, SlotSearch, separation_allowed_by_mode,
};
use crate::navigation::PolymeshDebug;
use crate::rng::{MAX_SEED, SEED_ROLL_RANGE, WorldSeed};
use crate::settings::{
    CLAIM_SEARCH_MAX, CLAIM_SEARCH_MIN, CLAIM_SEARCH_STEP, DEMON_CAP_MAX, DEMON_CAP_MIN,
    DEMON_CAP_STEP, DEMON_LUNGE_BOOST_MAX, DEMON_LUNGE_BOOST_MIN, DEMON_LUNGE_BOOST_STEP,
    DEMON_SPAWN_INTERVAL_MAX, DEMON_SPAWN_INTERVAL_MIN, DEMON_SPAWN_INTERVAL_STEP,
    DEMON_SPEED_FACTOR_MAX, DEMON_SPEED_FACTOR_MIN, DEMON_SPEED_FACTOR_STEP, HUMAN_BODY_RADIUS_MAX,
    HUMAN_BODY_RADIUS_MIN, HUMAN_BODY_RADIUS_STEP, HUMAN_SPEED_SPREAD_MAX, HUMAN_SPEED_SPREAD_MIN,
    HUMAN_SPEED_SPREAD_STEP, SEPARATION_LEFT_SHARE_MAX, SEPARATION_LEFT_SHARE_MIN,
    SEPARATION_LEFT_SHARE_STEP, SEPARATION_PASS_SQUEEZE_MAX, SEPARATION_PASS_SQUEEZE_MIN,
    SEPARATION_PASS_SQUEEZE_STEP, SLOT_REGROUP_MAX, SLOT_REGROUP_MIN, SLOT_REGROUP_STEP,
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

/// Панель Human: разброс скоростей и радиус тела. Строк пока две, поэтому без
/// enum'а — по паре маркеров на строку; появится третья, заводить `HumanRow`.
#[derive(Component)]
struct SpreadValueLabel;

#[derive(Component)]
struct SpreadSlider;

#[derive(Component)]
struct BodyRadiusValueLabel;

#[derive(Component)]
struct BodyRadiusSlider;

/// Строка радиуса поиска слота назначения — в панели World: механизм
/// видо-независимый, как и расталкивание.
#[derive(Component)]
struct SlotSearchValueLabel;

#[derive(Component)]
struct SlotSearchSlider;

/// Строки трёх механизмов, которые замер вывел в дефолты
/// (`tools/crowd_tuning_lab/REPORT.md`): протискивание мимо стоящих, доля
/// левшей и возврат на свой слот. Все три — свойства мира, а не вида, поэтому
/// стоят в панели World рядом с тумблером расталкивания.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CrowdRow {
    PassSqueeze,
    LeftShare,
    Regroup,
}

/// Подпись значения такой строки.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct CrowdValueLabel(CrowdRow);

/// Её ползунок.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct CrowdSlider(CrowdRow);

impl CrowdRow {
    const ALL: [Self; 3] = [Self::PassSqueeze, Self::LeftShare, Self::Regroup];

    fn label(self) -> &'static str {
        match self {
            Self::PassSqueeze => "Pass squeeze",
            Self::LeftShare => "Left share",
            Self::Regroup => "Regroup",
        }
    }

    /// `(min, max, шаг)` — из `settings.rs`, как у остальных ползунков.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::PassSqueeze => (
                SEPARATION_PASS_SQUEEZE_MIN,
                SEPARATION_PASS_SQUEEZE_MAX,
                SEPARATION_PASS_SQUEEZE_STEP,
            ),
            Self::LeftShare => (
                SEPARATION_LEFT_SHARE_MIN,
                SEPARATION_LEFT_SHARE_MAX,
                SEPARATION_LEFT_SHARE_STEP,
            ),
            Self::Regroup => (SLOT_REGROUP_MIN, SLOT_REGROUP_MAX, SLOT_REGROUP_STEP),
        }
    }

    fn get(self, lab: &SeparationLab, slots: &SlotLab) -> f32 {
        match self {
            Self::PassSqueeze => lab.pass_squeeze,
            Self::LeftShare => lab.left_share,
            Self::Regroup => slots.regroup,
        }
    }

    fn set(self, lab: &mut SeparationLab, slots: &mut SlotLab, value: f32) {
        match self {
            Self::PassSqueeze => lab.pass_squeeze = value,
            Self::LeftShare => lab.left_share = value,
            Self::Regroup => slots.regroup = value,
        }
    }

    /// Единица измерения в подписи: у возврата это метры, у двух других — доля.
    fn value_text(self, lab: &SeparationLab, slots: &SlotLab) -> String {
        let value = self.get(lab, slots);
        match self {
            Self::Regroup => format!("{value:.2} m"),
            _ => format!("{value:.2}"),
        }
    }
}

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
                sync_separation_value.run_if(
                    resource_changed::<SeparationStyle>
                        .or_else(resource_changed::<Determinism>)
                        // строка гаснет и от смены бэкенда навигации
                        .or_else(resource_changed::<PolymeshDebug>),
                ),
                sync_demon_values.run_if(resource_changed::<DemonStyle>),
                sync_human_values.run_if(resource_changed::<HumanStyle>),
                sync_slot_search_value.run_if(resource_changed::<SlotSearch>),
                sync_crowd_values
                    .run_if(resource_changed::<SeparationLab>.or_else(resource_changed::<SlotLab>)),
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

#[allow(clippy::too_many_arguments)]
fn render_stats_panel(
    mut commands: Commands,
    style: Res<DemonStyle>,
    human_style: Res<HumanStyle>,
    separation: Res<SeparationStyle>,
    separation_lab: Res<SeparationLab>,
    slot_lab: Res<SlotLab>,
    slot_search: Res<SlotSearch>,
    determinism: Res<Determinism>,
    polymesh: Res<PolymeshDebug>,
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
                    Text::new(separation_value(&separation, &determinism, &polymesh)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(separation_value_color(&determinism, &polymesh)),
                ),
            ],
        ))
        .observe(
            |_activate: On<Activate>,
             mut style: ResMut<SeparationStyle>,
             determinism: Res<Determinism>,
             polymesh: Res<PolymeshDebug>| {
                // под детерминизмом и на сеточной навигации расталкивания нет
                // вовсе — тумблер не должен молча переключать то, что всё
                // равно не работает
                if !separation_allowed_by_mode(determinism.0, polymesh.enabled) {
                    return;
                }
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

    // радиус тела — здесь, а не в World рядом с тумблером расталкивания: это
    // свойство человека, и читает его не только расталкивание, но и слоты
    // назначения, которые работают даже когда тумблер выключен
    spawn_slider_row(
        &mut commands,
        human_panel,
        SliderRow {
            label: "Body radius",
            value: human_style.body_radius,
            value_text: body_radius_value(&human_style),
            range: (
                HUMAN_BODY_RADIUS_MIN,
                HUMAN_BODY_RADIUS_MAX,
                HUMAN_BODY_RADIUS_STEP,
            ),
        },
        BodyRadiusValueLabel,
        BodyRadiusSlider,
        on_body_radius_change,
    );

    spawn_slider_row(
        &mut commands,
        world_panel,
        SliderRow {
            label: "Slot search",
            value: slot_search.0,
            value_text: slot_search_value(&slot_search),
            range: (CLAIM_SEARCH_MIN, CLAIM_SEARCH_MAX, CLAIM_SEARCH_STEP),
        },
        SlotSearchValueLabel,
        SlotSearchSlider,
        on_slot_search_change,
    );

    for row in CrowdRow::ALL {
        spawn_slider_row(
            &mut commands,
            world_panel,
            SliderRow {
                label: row.label(),
                value: row.get(&separation_lab, &slot_lab),
                value_text: row.value_text(&separation_lab, &slot_lab),
                range: row.range(),
            },
            CrowdValueLabel(row),
            CrowdSlider(row),
            move |change: On<ValueChange<f32>>,
                  mut commands: Commands,
                  mut lab: ResMut<SeparationLab>,
                  mut slots: ResMut<SlotLab>| {
                let (min, max, step) = row.range();
                let stepped = quantize(change.value, min, max, step);
                commands.entity(change.source).insert(SliderValue(stepped));
                if (row.get(&lab, &slots) - stepped).abs() > f32::EPSILON {
                    row.set(&mut lab, &mut slots, stepped);
                }
            },
        );
    }
}

/// Подтянуть строки механизмов к ресурсам — их правят не только эти ползунки
/// (BRP, панель демо-сцены), а расходиться показанному и настоящему нельзя.
fn sync_crowd_values(
    mut commands: Commands,
    lab: Res<SeparationLab>,
    slots: Res<SlotLab>,
    mut labels: Query<(&CrowdValueLabel, &mut Text)>,
    sliders: Query<(Entity, &CrowdSlider, &SliderValue)>,
) {
    for (label, mut text) in &mut labels {
        let next = label.0.value_text(&lab, &slots);
        if text.0 != next {
            text.0 = next;
        }
    }
    for (entity, slider, value) in &sliders {
        let next = slider.0.get(&lab, &slots);
        if (value.0 - next).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(next));
        }
    }
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
///
/// Под детерминизмом и на сеточной навигации строка не подсвечивается вовсе:
/// расталкивание там выключено расписанием (`movement/mod.rs`), нажимать
/// нечего, и реакция на курсор обещала бы работающую кнопку.
fn highlight_separation_row(
    determinism: Res<Determinism>,
    polymesh: Res<PolymeshDebug>,
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<SeparationRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if !separation_allowed_by_mode(determinism.0, polymesh.enabled) {
            ROW_LIGHTEN
        } else if pressed {
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
    determinism: Res<Determinism>,
    polymesh: Res<PolymeshDebug>,
    mut labels: Query<(&mut Text, &mut TextColor), With<SeparationValueLabel>>,
) {
    for (mut text, mut color) in &mut labels {
        text.0 = separation_value(&style, &determinism, &polymesh);
        color.0 = separation_value_color(&determinism, &polymesh);
    }
}

/// Подпись тумблера расталкивания.
///
/// Под детерминизмом и на сеточной навигации — всегда `off`, каким бы ни был
/// `SeparationStyle`: система выключена run-условием
/// (`movement::separation_runs` — расталкивание завязано на камеру, зум и
/// `FrameCount`, то есть на всё, от чего повтор обязан не зависеть, а на
/// сетке waypoint'ы стоят в центрах навтайлов и разводить пешки некуда).
/// Собственное значение стиля при этом сохраняется — возврат режима вернёт
/// панель к нему.
fn separation_value(
    style: &SeparationStyle,
    determinism: &Determinism,
    polymesh: &PolymeshDebug,
) -> String {
    if !separation_allowed_by_mode(determinism.0, polymesh.enabled) {
        return "off".to_string();
    }
    if style.enabled { "on" } else { "off" }.to_string()
}

/// Приглушённая подпись — тем же способом, каким панели показывают
/// неактивное: цветом, а не отдельной иконкой.
fn separation_value_color(determinism: &Determinism, polymesh: &PolymeshDebug) -> Color {
    if separation_allowed_by_mode(determinism.0, polymesh.enabled) {
        Color::WHITE
    } else {
        Color::srgb(0.45, 0.45, 0.45)
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
    mut spread_label: Query<&mut Text, (With<SpreadValueLabel>, Without<BodyRadiusValueLabel>)>,
    spread_slider: Query<(Entity, &SliderValue), With<SpreadSlider>>,
    mut radius_label: Query<&mut Text, (With<BodyRadiusValueLabel>, Without<SpreadValueLabel>)>,
    radius_slider: Query<(Entity, &SliderValue), With<BodyRadiusSlider>>,
) {
    for mut text in &mut spread_label {
        text.0 = spread_value(&style);
    }
    for (entity, value) in &spread_slider {
        if (value.0 - style.spread).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(style.spread));
        }
    }
    for mut text in &mut radius_label {
        text.0 = body_radius_value(&style);
    }
    for (entity, value) in &radius_slider {
        if (value.0 - style.body_radius).abs() > f32::EPSILON {
            commands
                .entity(entity)
                .insert(SliderValue(style.body_radius));
        }
    }
}

/// Подпись и бегунок радиуса поиска слота вслед за ресурсом.
fn sync_slot_search_value(
    search: Res<SlotSearch>,
    mut commands: Commands,
    mut label: Query<&mut Text, With<SlotSearchValueLabel>>,
    slider: Query<(Entity, &SliderValue), With<SlotSearchSlider>>,
) {
    for mut text in &mut label {
        text.0 = slot_search_value(&search);
    }
    for (entity, value) in &slider {
        if (value.0 - search.0).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(search.0));
        }
    }
}

fn on_body_radius_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<HumanStyle>,
) {
    let stepped = quantize(
        change.value,
        HUMAN_BODY_RADIUS_MIN,
        HUMAN_BODY_RADIUS_MAX,
        HUMAN_BODY_RADIUS_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.body_radius - stepped).abs() > f32::EPSILON {
        style.body_radius = stepped;
    }
}

fn on_slot_search_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut search: ResMut<SlotSearch>,
) {
    let stepped = quantize(
        change.value,
        CLAIM_SEARCH_MIN,
        CLAIM_SEARCH_MAX,
        CLAIM_SEARCH_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (search.0 - stepped).abs() > f32::EPSILON {
        search.0 = stepped;
    }
}

/// Значение строки радиуса тела. В метрах: «личное пространство» — это
/// дистанция покоя, вдвое больше, и она читается по спрайту (1 м).
fn body_radius_value(style: &HumanStyle) -> String {
    format!("{:.2} m", style.body_radius)
}

fn slot_search_value(search: &SlotSearch) -> String {
    format!("{:.0} m", search.0)
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
