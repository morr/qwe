//! Три панели левого верхнего угла: World — сколько пешек ещё живо, сколько
//! демонов ходит по городу, сколько душ съедено; Demon под ней — ползунки
//! `DemonStyle` (кап, интервал спавна, скорость и надбавка на бросок); Human
//! под ними — разброс личных скоростей (`HumanStyle`).
//!
//! Счётчики до этого жили только в BRP (`count Human`, `res get Telemetry`), то
//! есть смотреть на симуляцию без агентского клиента рядом было нечем.

use bevy::input_focus::InputFocus;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use bevy::ui_widgets::Activate;
use rand::Rng;

use super::brp::{AgentBrpSession, BrpBadge};
use super::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use super::rows::{ROW_LEFT_PX, ROW_LIGHTEN, row_color};
use super::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};
use crate::demon::{Demon, DemonStyle};
use crate::determinism::Determinism;
use crate::human::{Human, HumanStyle};
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

/// Поле ввода seed'а мира.
#[derive(Component)]
struct SeedField;

pub struct UiStatsPlugin;

impl Plugin for UiStatsPlugin {
    fn build(&self, app: &mut App) {
        // ручки панелей World, Demon и Human — их подписи и бегунки ведёт кит.
        // `HumanStyle` регистрирует ещё и панель Navigation (радиус тела);
        // повторный вызов кит отбрасывает сам
        app.add_knobs::<DemonStyle>()
            .add_knobs::<HumanStyle>()
            .add_knobs::<Determinism>()
            .add_systems(Startup, render_stats_panel)
            .add_systems(
                Update,
                (
                    sync_world_counts,
                    apply_seed_on_enter,
                    sync_seed_field.run_if(resource_changed::<WorldSeed>),
                    // метка BRP стоит только в агентских запусках, и только
                    // тогда панели есть что обходить
                    offset_below_brp_badge.run_if(resource_exists::<AgentBrpSession>),
                ),
            );
    }
}

/// Строка-счётчик: подпись слева, белое число справа — та же разметка,
/// что у шапки строки-ползунка (`ui::knob::spawn_knob`).
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

    // тумблер детерминированного режима и поле seed'а — свойства мира целиком,
    // а не вида. Расталкивание и слоты стояли здесь по той же логике, но
    // уехали в подвкладки панели Navigation: они про перемещение, и ручек у
    // них столько, что World переставал читаться как сводка прогона
    spawn_cycle_row(
        &mut commands,
        world_panel,
        "Deterministic",
        ROW_LEFT_PX,
        &*determinism,
        CycleBinding {
            // рестарт заказывает наблюдатель в `determinism.rs`: прогон
            // детерминирован или нет с тика 0, переключить его на ходу нельзя
            cycle: |mode| mode.0 = !mode.0,
            text: |mode| on_off(mode.0).to_string(),
        },
    );
    let seed_row = spawn_seed_row(&mut commands, seed.0);
    commands.entity(world_panel).add_child(seed_row);

    // панель Demon отдельной сущностью, а не внутри `children!`:
    // `spawn_knob` берёт родителя сущностью, а там её ещё нет
    let panel = commands
        .spawn((
            panel_node(),
            BackgroundColor(ui_color(UiOpacity::Medium)),
            Name::new("demon_style_panel"),
            children![panel_title("Demon")],
        ))
        .id();
    commands.entity(column).add_child(panel);

    // кап — единственная целочисленная ручка панелей: шаг целый, и текст без
    // дробной части, так что округление ползунка и есть само значение
    spawn_knob(
        &mut commands,
        panel,
        "Max demons",
        &*style,
        SliderBinding {
            get: |style| style.cap as f32,
            set: |style, value| style.cap = value as usize,
            range: (DEMON_CAP_MIN, DEMON_CAP_MAX, DEMON_CAP_STEP),
            text: |value| format!("{value:.0}"),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Spawn every",
        &*style,
        SliderBinding {
            get: |style| style.interval,
            set: |style, value| style.interval = value,
            range: (
                DEMON_SPAWN_INTERVAL_MIN,
                DEMON_SPAWN_INTERVAL_MAX,
                DEMON_SPAWN_INTERVAL_STEP,
            ),
            text: |value| format!("{value:.1} s"),
        },
    );
    // скорость и бросок — проценты: множитель «1.3» на панели ничего не
    // сообщает, «130%» и «+30%» читаются сразу
    spawn_knob(
        &mut commands,
        panel,
        "Speed",
        &*style,
        SliderBinding {
            get: |style| style.speed,
            set: |style, value| style.speed = value,
            range: (
                DEMON_SPEED_FACTOR_MIN,
                DEMON_SPEED_FACTOR_MAX,
                DEMON_SPEED_FACTOR_STEP,
            ),
            text: |value| format!("{:.0}%", value * 100.0),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Lunge boost",
        &*style,
        SliderBinding {
            get: |style| style.lunge,
            set: |style, value| style.lunge = value,
            range: (
                DEMON_LUNGE_BOOST_MIN,
                DEMON_LUNGE_BOOST_MAX,
                DEMON_LUNGE_BOOST_STEP,
            ),
            text: |value| format!("+{:.0}%", value * 100.0),
        },
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

    // со знаком, потому что это полуширина: «15%» читалось бы как «все на 15%
    // быстрее». Знак — ASCII `+/-`, а не «±»: встроенный шрифт (фича
    // `default_font`) — узкая подвыборка, и всё вне ASCII рисуется квадратом
    spawn_knob(
        &mut commands,
        human_panel,
        "Speed spread",
        &*human_style,
        SliderBinding {
            get: |style| style.spread,
            set: |style, value| style.spread = value,
            range: (
                HUMAN_SPEED_SPREAD_MIN,
                HUMAN_SPEED_SPREAD_MAX,
                HUMAN_SPEED_SPREAD_STEP,
            ),
            text: |value| format!("+/-{:.0}%", value * 100.0),
        },
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
