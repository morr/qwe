//! Колонка правого края и панель телеметрии в ней (порт
//! `zxc/src/ui/simulation_state.rs`, без игровой даты): первая строка — часы
//! симуляции (`SimClock`), сколько мир уже прожил, вторая — диагностика
//! pathfinding (порт заголовка `zxc/src/ui/debug/info.rs`). Строка камеры с
//! точкой под курсором живёт теперь в инспекторе точки под этой панелью
//! (`ui/inspector.rs`) — тоже ребёнок колонки, раскладку ведёт её флекс.
//!
//! Скорость времени вынесена левее колонки в отдельную кнопку: она не только
//! показывает `SimSpeed`, но и крутит лесенку кликом.

use bevy::app::PropagateOver;
use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin};
use bevy::feathers::constants::{fonts, size};
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::ThemeTextColor;
use bevy::feathers::tokens;
use bevy::picking::Pickable;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::Activate;

use crate::diagnostics::{
    PATHFINDING_ANSWERED, PATHFINDING_DURATION_MS, PATHFINDING_FAILED, PATHFINDING_IN_FLIGHT,
    PATHFINDING_QUEUED, SIM_TICK_MS, SIM_WAIT_MS, SIM_WAIT_PEAK_MS,
};
use crate::sim_time::{SimClock, SimSpeed, cycle_time_scale, previous_time_scale};
use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiBuildSet, button_variant, panel_background,
    panel_button_label, row_label, spawn_panel_button_with, ui_node,
};

/// Ширина колонки правого края и телеметрии в ней; от неё же отсчитывается
/// место кнопки скорости. Инспектор точки (`ui/inspector.rs`) — второй ребёнок
/// той же колонки, так что одна ширина на обе панели выходит сама.
pub(crate) const PANEL_WIDTH_PX: f32 = 470.0;
/// Зазор между телеметрией и инспектором точки внутри колонки, px.
const INSPECTOR_GAP_PX: f32 = 6.0;
/// Зазор между кнопкой скорости и левым краем панели.
const SPEED_BUTTON_GAP_PX: f32 = 6.0;
/// Ширина кнопки скорости: хватает на самое длинное значение
/// `Paused (30x)` / `15x → 8.4x` без переноса. 12 знаков моноширинного
/// FiraMono на 14 px — это 12 × 8.4 ≈ 101 px, плюс подпись `Speed:` (6 × 8.4 ≈
/// 50), зазор 8 и горизонтальные отступы 16: 175 px, откуда и запас до 190.
const SPEED_BUTTON_WIDTH_PX: f32 = 190.0;
/// Период обновления строк телеметрии, с. Сглаженные диагнозы (tick/wait,
/// in-flight) меняются каждый кадр — без троттлинга строка отличалась бы от
/// прошлой и перерисовывалась 60 раз в секунду; 8 Гц достаточно, чтобы
/// читать. Тикает на `Time<Real>`: на паузе виртуальное время стоит, и метка
/// `Paused (…)` на кнопке иначе не доезжала бы до экрана вовсе.
const TELEMETRY_REFRESH_SECS: f32 = 0.125;

#[derive(Component, Default)]
struct ClockTextMarker;

#[derive(Component, Default)]
struct PathfindingTextMarker;

/// Колонка правого края: инспектор точки (`ui/inspector.rs`) добавляет в неё
/// свою панель ребёнком после телеметрии. Вставляется при спавне колонки
/// (`render_speed_ui`, `UiBuildSet::Shell`) — инспектор спавнится позже, в
/// `UiBuildSet::Sections`, и находит ресурс гарантированно.
#[derive(Resource)]
pub(crate) struct RightHudColumn(pub(crate) Entity);

/// Кнопка скорости — адресует и подсветку, и подпись значения.
#[derive(Component, Default)]
struct SpeedButton;

/// Текст значения на кнопке (правая половина строки).
#[derive(Component, Default)]
struct SpeedValueLabel;

pub struct UiSpeedPlugin;

impl Plugin for UiSpeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_speed_ui.in_set(UiBuildSet::Shell))
            .add_systems(Startup, render_speed_button)
            .add_systems(
                Update,
                (
                    update_clock_text,
                    update_speed_button,
                    update_pathfinding_text,
                ),
            );
    }
}

/// Колонка правого края с телеметрией внутри. Один флекс вместо двух
/// абсолютных панелей: ребёнку не нужно знать рост соседа, чтобы встать под
/// ним, — тот же приём, что у колонки левого края (`ui/shell.rs`). Пиксельно
/// то же самое: телеметрия стояла `top: EDGE`, инспектор —
/// `EDGE + высота телеметрии + INSPECTOR_GAP_PX`; колонка с `row_gap:
/// INSPECTOR_GAP_PX` складывает их ровно так же.
fn render_speed_ui(mut commands: Commands, clock: Res<SimClock>, assets: Res<AssetServer>) {
    let mono: Handle<Font> = assets.load(fonts::MONO);
    let column = commands
        .spawn((
            ui_node(Node {
                position_type: PositionType::Absolute,
                // от края экрана — тот же отступ, что у нижних панелей
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: px(INSPECTOR_GAP_PX),
                width: px(PANEL_WIDTH_PX),
                ..default()
            }),
            // колонка шире своих панелей ровно на зазор между ними, и невидимая
            // полоса не должна отбирать у карты клики (см. колонку левого края)
            Pickable::IGNORE,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("ui_right_column"),
        ))
        .id();

    let telemetry = commands
        .spawn((
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(3.),
                // фиксированная ширина: количество цифр в счётчиках меняется
                // несколько раз в секунду, и авто-ширина заставляла панель
                // дёргаться. Ширины хватает на строку pathfinding целиком —
                // перенос её на вторую строку сдвигал бы всё под ней
                width: px(PANEL_WIDTH_PX),
                padding: UiRect {
                    top: px(10.),
                    right: px(16.),
                    bottom: px(10.),
                    left: px(16.),
                },
                ..default()
            },
            panel_background(),
            Name::new("speed_ui"),
            // моноширинный и покрупнее: часы и счётчики меняются несколько раз в
            // секунду, и на пропорциональном шрифте цифры под собой пляшут
            InheritableFont {
                font: mono.clone(),
                font_size: size::SMALL_FONT,
                weight: FontWeight::NORMAL,
            },
            children![
                (
                    Text(format_sim_clock(clock.elapsed)),
                    // часы крупнее остальной телеметрии. `PropagateOver` — чтобы
                    // свой кегль пережил шрифт панели: без него `InheritableFont`
                    // сверху перезаписал бы `TextFont` целиком
                    TextFont {
                        font: mono.clone().into(),
                        font_size: FontSize::Px(20.),
                        ..default()
                    },
                    PropagateOver::<TextFont>::default(),
                    ThemeTextColor(tokens::TEXT_MAIN),
                    ClockTextMarker,
                ),
                (
                    Text::default(),
                    ThemeTextColor(tokens::TEXT_DIM),
                    PathfindingTextMarker,
                ),
            ],
        ))
        .id();
    commands.entity(column).add_child(telemetry);

    commands.insert_resource(RightHudColumn(column));
}

fn update_clock_text(
    mut due: Local<f32>,
    time: Res<Time<Real>>,
    text: Single<&mut Text, With<ClockTextMarker>>,
    clock: Res<SimClock>,
) {
    *due -= time.delta_secs();
    if *due > 0.0 {
        return;
    }
    *due = TELEMETRY_REFRESH_SECS;
    text.into_inner()
        .set_if_neq(Text(format_sim_clock(clock.elapsed)));
}

/// Кнопка скорости — слева от панели телеметрии. Устроена как панель
/// Buildings: полупрозрачная подложка, внутри строка-кнопка с плотным фоном,
/// тусклой подписью `Speed:` слева и белым значением справа. Своя нода, а не
/// строка панели телеметрии: та фиксированной ширины и прибита к правому краю.
///
/// Клик крутит лесенку скоростей (`cycle_time_scale`), правый клик — назад
/// (`previous_time_scale`); хоткеи `=`/`-` и Space продолжают работать.
///
/// Кнопка не берёт событие `Activate` кита: оно приходит на любую кнопку мыши,
/// и правый клик срабатывал бы дважды — вперёд и назад. Виджет остаётся ради
/// подсветки и `ButtonVariant`, а решение принимается в своём наблюдателе по
/// `PointerButton`.
fn render_speed_button(mut commands: Commands, time: Res<Time<Virtual>>, speed: Res<SimSpeed>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                // панель телеметрии сама отодвинута от края, поэтому её отступ
                // входит в смещение кнопки
                right: px(UI_SCREEN_EDGE_PX_OFFSET + PANEL_WIDTH_PX + SPEED_BUTTON_GAP_PX),
                padding: UiRect::all(px(10.)),
                ..default()
            },
            panel_background(),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("speed_panel"),
        ))
        .id();

    let caption = (
        crate::ui::ui_node(Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8.),
            // ширина фиксирована: значение меняется с «1x» на «15x → 8.4x»,
            // и по авто-ширине кнопка прыгала бы при каждом клике
            width: px(SPEED_BUTTON_WIDTH_PX),
            ..default()
        }),
        children![
            (
                row_label("Speed:"),
                Node {
                    flex_grow: 1.,
                    ..default()
                },
            ),
            (
                SpeedValueLabel,
                panel_button_label(&format_speed_label(&time, &speed)),
                // значение живёт в строке кнопки фиксированной ширины: без
                // запрета переноса `Paused (30x)` ломается по пробелу на две
                // строки и кнопка вырастает вдвое
                TextLayout::no_wrap(),
            ),
        ],
    );
    // `Activate` намеренно пустой — см. док функции; шаг лесенки решает
    // наблюдатель клика ниже, он один знает, какая кнопка нажата
    let button = spawn_panel_button_with(
        &mut commands,
        panel,
        SpeedButton,
        caption,
        time.is_paused(),
        |_activate: On<Activate>| {},
    );
    commands
        .entity(button)
        .observe(
            |click: On<PointerClick>, mut speed: ResMut<SimSpeed>| match click.button {
                PointerButton::Primary => speed.requested = cycle_time_scale(speed.requested),
                PointerButton::Secondary => speed.requested = previous_time_scale(speed.requested),
                PointerButton::Middle => {}
            },
        );
}

/// Подпись и подсветка кнопки. На паузе кнопка горит зелёным, как активный
/// тумблер: пауза — состояние, а не мгновенное действие. Подпись тикает на
/// `Time<Real>` с общим периодом телеметрии — замеренная скорость меняется
/// каждый кадр, и без троттлинга строка перерисовывалась бы на весь кадр;
/// сама пауза-подсветка дешёвая и честно сравнивается каждый кадр.
fn update_speed_button(
    mut due: Local<f32>,
    real: Res<Time<Real>>,
    mut variant: Single<&mut ButtonVariant, With<SpeedButton>>,
    value: Single<&mut Text, With<SpeedValueLabel>>,
    time: Res<Time<Virtual>>,
    speed: Res<SimSpeed>,
) {
    variant.set_if_neq(button_variant(time.is_paused()));

    *due -= real.delta_secs();
    if *due > 0.0 {
        return;
    }
    *due = TELEMETRY_REFRESH_SECS;
    value
        .into_inner()
        .set_if_neq(Text(format_speed_label(&time, &speed)));
}

/// Строка pathfinding-диагностики: в полёте, среднее время поиска, доля
/// отказов со своим знаменателем, сущности. Сглаженные значения в ней меняются
/// каждый кадр — троттлинг общего периода телеметрии держит каскад
/// `text → measure → layout` от перерисовки 60 раз в секунду.
fn update_pathfinding_text(
    mut due: Local<f32>,
    time: Res<Time<Real>>,
    text: Single<&mut Text, With<PathfindingTextMarker>>,
    diagnostics: Res<DiagnosticsStore>,
) {
    *due -= time.delta_secs();
    if *due > 0.0 {
        return;
    }
    *due = TELEMETRY_REFRESH_SECS;
    let in_flight = diagnostics
        .get(&PATHFINDING_IN_FLIGHT)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    let queued = diagnostics
        .get(&PATHFINDING_QUEUED)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    let duration_ms = diagnostics
        .get(&PATHFINDING_DURATION_MS)
        .and_then(|diagnostic| diagnostic.average())
        .unwrap_or_default();
    // отказы на ответ за окно истории: отношение средних, а не среднее
    // подолей — кадр с одним ответом иначе весит как кадр с сотней
    let answered = diagnostics
        .get(&PATHFINDING_ANSWERED)
        .and_then(|diagnostic| diagnostic.average())
        .unwrap_or_default();
    let failed = diagnostics
        .get(&PATHFINDING_FAILED)
        .and_then(|diagnostic| diagnostic.average())
        .unwrap_or_default();
    let failed = if answered > 0.0 {
        failed / answered * 100.0
    } else {
        0.0
    };
    let entities = diagnostics
        .get(&EntityCountDiagnosticsPlugin::ENTITY_COUNT)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    // цена тика — то, из чего регулятор считает посильную скорость, так что
    // «почему стоим на 4x» читается прямо с экрана
    let tick_ms = diagnostics
        .get(&SIM_TICK_MS)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    // ожидание конвейера показано рядом с работой: это два разных диагноза —
    // «мир дорогой» и «поиск пути не успевает к сроку»
    let wait_ms = diagnostics
        .get(&SIM_WAIT_MS)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();
    // пик — то, по чему на самом деле держится потолок конвейера: скорость,
    // стоящая ниже, чем обещает среднее ожидание, объясняется этим числом
    let peak_ms = diagnostics
        .get(&SIM_WAIT_PEAK_MS)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default();

    // выравнивание цифр по правому краю, чтобы строка не «плясала»;
    // знаменатель доли отказов показан рядом с ней: «100 % отказов» на
    // ручейке в один ответ за кадр и на сотне ответов — разные новости, а
    // само число одинаковое
    text.into_inner().set_if_neq(Text(format!(
        "pathfinding: {in_flight:>4.0} in flight, {queued:>5.0} queued, {duration_ms:>5.2} ms avg\nanswers: {answered:>6.1}/frame, {failed:>5.1}% failed\nentities: {entities:>6.0}, tick {tick_ms:>5.2} + {wait_ms:>5.2} ms wait (pk {peak_ms:>5.2})"
    )));
}

/// Значение на кнопке. «15x» — идём как просили; «15x → 9.8x» — машина не
/// тянет, время замедлено (см. `sim_time`). После стрелки — замеренная
/// фактическая скорость, поэтому она бывает и меньше 1x: на просадке
/// (например, пока фоново строится сетка northstar) симуляция отстаёт от
/// реального времени.
fn format_speed_label(time: &Time<Virtual>, speed: &SimSpeed) -> String {
    let requested = format!("{}x", speed.requested);
    if time.is_paused() {
        return format!("Paused ({requested})");
    }
    if speed.is_throttled() {
        // ниже 1x одного знака мало: 0.3x и 0.06x — разные истории
        let actual = if speed.actual < 1.0 {
            format!("{:.2}", speed.actual)
        } else {
            format!("{:.1}", speed.actual)
        };
        format!("{requested} → {actual}x")
    } else {
        requested
    }
}

/// Часы симуляции как `T+8130` — секунды и всё: разбивка на часы и сутки пока
/// не нужна, а секунды напрямую сопоставимы с периодами в `sim_time.rs`.
fn format_sim_clock(elapsed: f64) -> String {
    format!("T+{}", elapsed.max(0.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Подпись `Speed:` — отдельная нода кнопки, в значении её быть не должно:
    /// иначе на экране `Speed: Speed: 1x`.
    #[test]
    fn speed_value_carries_no_label() {
        let label = format_speed_label(&Time::<Virtual>::default(), &SimSpeed::default());
        assert_eq!(label, "1x");
    }

    #[test]
    fn sim_clock_counts_whole_seconds() {
        assert_eq!(format_sim_clock(0.0), "T+0");
        assert_eq!(format_sim_clock(8130.4), "T+8130");
    }
}
