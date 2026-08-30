//! Панель телеметрии в правом верхнем углу (порт
//! `zxc/src/ui/simulation_state.rs`, без игровой даты): первая строка — часы
//! симуляции (`SimClock`), сколько мир уже прожил, вторая — диагностика
//! pathfinding (порт заголовка `zxc/src/ui/debug/info.rs`), третья — зум и
//! позиция камеры плюс точка под курсором.
//!
//! Скорость времени вынесена левее панели в отдельную кнопку: она не только
//! показывает `SimSpeed`, но и крутит лесенку кликом.

use bevy::app::PropagateOver;
use bevy::camera_controller::pan_camera::PanCamera;
use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin};
use bevy::feathers::constants::{fonts, size};
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::ThemeTextColor;
use bevy::feathers::tokens;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::Activate;
use bevy::window::PrimaryWindow;

use crate::camera::cursor_offset;
use crate::diagnostics::{
    PATHFINDING_ANSWERED, PATHFINDING_DURATION_MS, PATHFINDING_FAILED, PATHFINDING_IN_FLIGHT,
    PATHFINDING_QUEUED, SIM_TICK_MS, SIM_WAIT_MS, SIM_WAIT_PEAK_MS,
};
use crate::sim_time::{SimClock, SimSpeed, cycle_time_scale, previous_time_scale};
use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, button_variant, panel_background, panel_button_label,
    row_label, spawn_panel_button_with,
};

/// Ширина панели телеметрии; от неё же отсчитывается место кнопки скорости.
const PANEL_WIDTH_PX: f32 = 470.0;
/// Зазор между кнопкой скорости и левым краем панели.
const SPEED_BUTTON_GAP_PX: f32 = 6.0;
/// Ширина кнопки скорости: хватает на самое длинное значение
/// `Paused (30x)` / `15x → 8.4x` без переноса. 12 знаков моноширинного
/// FiraMono на 14 px — это 12 × 8.4 ≈ 101 px, плюс подпись `Speed:` (6 × 8.4 ≈
/// 50), зазор 8 и горизонтальные отступы 16: 175 px, откуда и запас до 190.
const SPEED_BUTTON_WIDTH_PX: f32 = 190.0;

#[derive(Component, Default)]
struct ClockTextMarker;

#[derive(Component, Default)]
struct PathfindingTextMarker;

#[derive(Component, Default)]
struct CameraTextMarker;

/// Кнопка скорости — адресует и подсветку, и подпись значения.
#[derive(Component, Default)]
struct SpeedButton;

/// Текст значения на кнопке (правая половина строки).
#[derive(Component, Default)]
struct SpeedValueLabel;

pub struct UiSpeedPlugin;

impl Plugin for UiSpeedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (render_speed_ui, render_speed_button))
            .add_systems(
                Update,
                (
                    update_clock_text,
                    update_speed_button,
                    update_pathfinding_text,
                    update_camera_text,
                ),
            );
    }
}

fn render_speed_ui(mut commands: Commands, clock: Res<SimClock>, assets: Res<AssetServer>) {
    let mono: Handle<Font> = assets.load(fonts::MONO);
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            // от края экрана — тот же отступ, что у нижних панелей
            top: px(UI_SCREEN_EDGE_PX_OFFSET),
            right: px(UI_SCREEN_EDGE_PX_OFFSET),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(3.),
            // фиксированная ширина: количество цифр в счётчиках меняется
            // каждый кадр, и авто-ширина заставляла панель дёргаться. Ширины
            // хватает на строку pathfinding целиком — перенос её на вторую
            // строку сдвигал бы всё под ней
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
        GameUiRoot,
        Visibility::Hidden,
        Name::new("speed_ui"),
        // моноширинный и покрупнее: часы и счётчики меняются каждый кадр, и на
        // пропорциональном шрифте цифры под собой пляшут
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
            (
                Text::default(),
                ThemeTextColor(tokens::TEXT_DIM),
                CameraTextMarker,
            ),
        ],
    ));
}

fn update_clock_text(text: Single<&mut Text, With<ClockTextMarker>>, clock: Res<SimClock>) {
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
    // наблюдатель `Pointer<Click>` ниже, он один знает, какая кнопка нажата
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
            |click: On<Pointer<Click>>, mut speed: ResMut<SimSpeed>| match click.button {
                PointerButton::Primary => speed.requested = cycle_time_scale(speed.requested),
                PointerButton::Secondary => speed.requested = previous_time_scale(speed.requested),
                PointerButton::Middle => {}
            },
        );
}

/// Подпись и подсветка кнопки. На паузе кнопка горит зелёным, как активный
/// тумблер: пауза — состояние, а не мгновенное действие.
fn update_speed_button(
    mut variant: Single<&mut ButtonVariant, With<SpeedButton>>,
    value: Single<&mut Text, With<SpeedValueLabel>>,
    time: Res<Time<Virtual>>,
    speed: Res<SimSpeed>,
) {
    variant.set_if_neq(button_variant(time.is_paused()));

    value
        .into_inner()
        .set_if_neq(Text(format_speed_label(&time, &speed)));
}

/// Строка pathfinding-диагностики: в полёте, среднее время поиска, доля
/// отказов со своим знаменателем, сущности.
fn update_pathfinding_text(
    text: Single<&mut Text, With<PathfindingTextMarker>>,
    diagnostics: Res<DiagnosticsStore>,
) {
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

/// Где стоит камера и насколько приближена. Строка нужна не игроку, а чтению
/// скриншота со стороны: по ней видно, какой кусок карты в кадре, без запроса
/// к живому миру по BRP.
///
/// Формат `0.41/2374/2703 2510/2880` — сначала камера как `zoom/x/y` (порядок
/// пермалинка slippy-карт), через пробел — точка под курсором как `x/y`.
///
/// Координаты — мировые метры от юго-западного угла карты, та же система, в
/// которой лежат `SimPosition` и `Transform` юнитов; камерные — центр экрана.
/// Зум — метры на экранный пиксель, как их держит `PanCamera::zoom_factor`:
/// меньше — ближе.
///
/// Курсор вне окна координат не даёт — вместо чисел прочерки, чтобы строка не
/// теряла хвост и не выглядела обрезанной.
fn update_camera_text(
    text: Single<&mut Text, With<CameraTextMarker>>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let (transform, controller) = *camera;
    let center = transform.translation.truncate();
    let zoom = controller.zoom_factor;

    let cursor = match window.cursor_position() {
        Some(cursor) => {
            let world = center + cursor_offset(&window, cursor) * zoom;
            format!("{:.0}/{:.0}", world.x, world.y)
        }
        None => "-/-".to_string(),
    };

    // строка последняя в панели, поэтому ширины полей не выравниваем: сдвигать
    // её «пляской» цифр нечему
    text.into_inner().set_if_neq(Text(format!(
        "{zoom:.2}/{:.0}/{:.0} {cursor}",
        center.x, center.y
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
/// не нужна, а секунды напрямую сопоставимы с периодами в `settings.rs`.
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
