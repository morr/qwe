//! Строка-ползунок — общий кит числовых ручек панелей.
//!
//! Сам ползунок первопартийный (`bevy_feathers::controls::FeathersSlider`):
//! полоса с заливкой по значению, скруглённая, с курсором `EwResize`. Шаг
//! дискретный (см. [`quantize`]) — ресурс правится только на смене шага, а не
//! на каждом пикселе протяжки.
//!
//! Значение написано **внутри полосы**, как в галерее feathers, но пишет его
//! кит, а не библиотека: её собственное число — голое `SliderValue` без единиц,
//! а «0.9» вместо «0.90 m» и «0.1» вместо «10%» на панели ничего не сообщают.
//! Своё число — обычный ребёнок ползунка (узел полосы центрирует детей сам), а
//! библиотечное погашено токеном `SLIDER_TEXT` в `ui/theme.rs`.

use bevy::ecs::system::IntoObserverSystem;
use bevy::feathers::controls::FeathersSlider;
use bevy::feathers::theme::ThemedText;
use bevy::ui_widgets::{Slider, SliderPrecision, SliderStep, SliderValue, TrackClick, ValueChange};

use bevy::prelude::*;

use crate::ui::{row_label, row_value};

/// Что показывает строка-ползунок: подпись, стартовое значение с его текстом
/// и диапазон с шагом.
pub struct SliderRow<'a> {
    pub label: &'a str,
    pub value: f32,
    pub value_text: String,
    /// `(min, max, step)` — как у констант `settings.rs`.
    pub range: (f32, f32, f32),
}

/// Строка-ползунок: подпись и под ней полоса с числом внутри. Возвращает блок
/// строки — на него навешивается маркер, если строку кто-то адресует
/// (`MixedOnlyRow` управляет видимостью строк панели Trees).
///
/// `value_label_marker` встаёт на текст значения, `slider_marker` — на ползунок:
/// по ним панель-хозяйка актуализирует подпись и положение после правки ресурса
/// извне (BRP, сохранённые настройки).
pub fn spawn_slider_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: SliderRow,
    value_label_marker: impl Bundle,
    slider_marker: impl Bundle,
    on_change: impl IntoObserverSystem<ValueChange<f32>, (), M>,
) -> Entity {
    let SliderRow {
        label,
        value,
        value_text,
        range: (min, max, step),
    } = row;
    let block = commands
        .spawn((
            // одна строка, а не подпись над полосой: у feathers строка ростом
            // `size::ROW_HEIGHT`, и двухэтажные ползунки выгоняли левую колонку
            // панелей за верх экрана — она наезжала на панели World/Demon/Human
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6.),
                padding: UiRect::horizontal(px(8.)),
                ..default()
            },
            crate::ui::text_container(),
            // подпись не жмётся и не переносится: «Conifer share» и «Max
            // demons» на двух строках ломали ровный рост строк панели
            children![(
                row_label(label),
                TextLayout::no_wrap(),
                Node {
                    flex_shrink: 0.,
                    ..default()
                },
            )],
        ))
        .id();

    let slider = commands
        .spawn_scene(bsn! {
            @FeathersSlider { @value: {value}, @min: {min}, @max: {max} }
            // клик по полосе ставит значение туда, куда ткнули: у feathers по
            // умолчанию `Drag`, то есть клик мимо бегунка не делает ничего
            Slider { track_click: TrackClick::Snap }
            SliderStep({step})
            // число мы рисуем своё, но без этого компонента `update_slider_pos`
            // не выбирает ползунок вовсе — и заливка перестала бы ездить
            SliderPrecision(2)
            // полоса забирает всю ширину, оставшуюся от подписи: без нулевого
            // базиса flex делит строку по содержимому, и полоса выходила у́же
            // собственного числа
            Node { flex_basis: px(0), min_width: px(70) }
            // метка-проводник шрифта: наше число — ребёнок полосы, а сама она
            // `ThemedText` не носит, и распространение обрывалось на ней
            ThemedText
        })
        .insert(slider_marker)
        .observe(on_change)
        .with_child((value_label_marker, row_value(value_text)))
        .id();
    commands.entity(block).add_child(slider);
    commands.entity(panel).add_child(block);
    block
}

/// Дискретный шаг ползунка: значение с драга округляется до шага, наблюдатели
/// правят ресурс только когда шаг действительно сменился — иначе каждый
/// пиксель протяжки запускал бы пересборку.
pub fn quantize(value: f32, min: f32, max: f32, step: f32) -> f32 {
    ((value / step).round() * step).clamp(min, max)
}

/// Первая половина каждого наблюдателя протяжки: округлить до шага и вернуть
/// бегунок на округлённое место. Возвращает шаг — правку своего ресурса
/// вызывающий делает сам, потому что поля у всех разного типа (`f32`, `u32`,
/// `usize`) и сравнивать их с `f32::EPSILON` можно только на месте.
///
/// `SliderValue` — immutable-компонент, меняется только вставкой.
pub fn apply_step(
    change: &On<ValueChange<f32>>,
    commands: &mut Commands,
    (min, max, step): (f32, f32, f32),
) -> f32 {
    let stepped = quantize(change.value, min, max, step);
    commands.entity(change.source).insert(SliderValue(stepped));
    stepped
}

/// Переставить бегунок под значение, пришедшее мимо него — по BRP, из
/// сохранённых настроек, из пресета стенда. При протяжке значения уже
/// совпадают, и вставка не делается.
pub fn retarget(commands: &mut Commands, slider: Entity, current: f32, target: f32) {
    if (current - target).abs() > f32::EPSILON {
        commands.entity(slider).insert(SliderValue(target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Округление и кламп — то, ради чего ползунок «дискретный»: ресурс
    /// правится только на смене шага, и промежуточные значения протяжки
    /// обязаны схлопываться в одно.
    #[test]
    fn quantize_rounds_to_the_step_and_clamps_to_the_range() {
        // `3.0 * 0.1`, а не `0.3`: шаг умножается, и в f32 это разные числа
        assert_eq!(quantize(0.34, 0.0, 1.0, 0.1), 3.0 * 0.1);
        assert_eq!(quantize(0.36, 0.0, 1.0, 0.1), 4.0 * 0.1);
        assert_eq!(quantize(-5.0, 0.2, 1.0, 0.1), 0.2);
        assert_eq!(quantize(99.0, 0.2, 1.0, 0.1), 1.0);
    }

    /// Кламп идёт ПОСЛЕ округления: иначе шаг мог бы вынести значение за
    /// границу, которую ползунок показывает как предел.
    #[test]
    fn quantize_never_leaves_the_range_after_rounding() {
        let (min, max, step) = (0.0, 0.95, 0.1);
        for raw in [0.94_f32, 0.951, 1.5] {
            let stepped = quantize(raw, min, max, step);
            assert!((min..=max).contains(&stepped), "{raw} -> {stepped}");
        }
    }
}
