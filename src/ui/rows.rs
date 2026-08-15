//! Строка-значение — общий каркас строк всех панелей стиля.
//!
//! Панелей стиля шесть (Navigation, Trees, Tree rows, Roads, Buildings, Stats),
//! и все они устроены одинаково: полей ввода в `bevy_ui` нет, поэтому строка —
//! это кнопка, листающая значение по кругу, с серой подписью слева и белым
//! значением справа. Каркас был скопирован в каждую из них: свои
//! `ROW_LIGHTEN`/`HOVER_LIGHTEN`/`PRESSED_LIGHTEN`, свой `row_color`, свой
//! `spawn_row` на полсотни строк и своя система подсветки.
//!
//! Копии успели разъехаться в мелочах, и одна из них была не косметической:
//! пять систем из шести писали `BackgroundColor` **безусловно, каждый кадр**,
//! то есть помечали изменившимися все строки всех панелей независимо от того,
//! двигалась мышь или нет. Сегодня подсветки здесь нет вовсе: строка — это
//! первопартийная кнопка feathers, и цвет по наведению и нажатию ведёт она
//! сама, системой с фильтром `Changed` (см. `ui/theme.rs`).
//!
//! Строка, клик по которой сейчас ничего не делает, получает от своей панели
//! `bevy::ui::InteractionDisabled`: feathers перестаёт её подсвечивать и не шлёт
//! `Activate` — обещать реакцию на курсор там, где клик ничего не сделает, хуже,
//! чем не подсвечивать. Единственный носитель на сегодня — тумблер расталкивания
//! в панели Navigation.

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::feathers::controls::FeathersButton;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use super::{ROW_LABEL_COLOR, UiOpacity, ui_color};

/// Фон строки в покое: плотный, поверх полупрозрачной панели.
pub(super) const ROW_LIGHTEN: f32 = 0.0;

/// Отступ слева у обычной строки. Вложенная (панель Navigation) прибавляет к
/// нему свой `NESTED_ROW_INDENT_PX`.
pub(super) const ROW_LEFT_PX: f32 = 8.0;

pub(super) fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Кнопка-строка: серая подпись слева, белое значение справа, клик — на
/// `on_activate`. Возвращает строку, чтобы вызывающая панель довесила свои
/// метки и, если надо, свотч.
///
/// Своя сцена, а не общий `super::spawn_panel_button_with`: у строки другая
/// геометрия — она во всю ширину панели, с отступом слева по вложенности, и
/// подпись растягивается, отжимая значение вправо. Общего у них ровно цвет, и
/// он приходит из темы (`ui/theme.rs`), а не отсюда.
pub(super) fn spawn_value_row<M>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    left_px: f32,
    value_marker: impl Bundle,
    value: String,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let padding = UiRect {
        top: px(4.),
        right: px(8.),
        bottom: px(4.),
        left: px(left_px),
    };
    let row = commands
        .spawn_scene(bsn! {
            @FeathersButton
            Node {
                height: Val::Auto,
                justify_content: JustifyContent::FlexStart,
                column_gap: px(6),
                padding: {padding},
                border_radius: BorderRadius::ZERO,
            }
        })
        .observe(on_activate)
        .with_child((
            Text::new(label),
            TextFont {
                font_size: FontSize::Px(12.),
                ..default()
            },
            TextColor(ROW_LABEL_COLOR),
            // распорка: подпись забирает всю свободную ширину, значение
            // прижимается к правому краю строки
            Node {
                flex_grow: 1.,
                ..default()
            },
        ))
        .with_child((
            value_marker,
            Text::new(value),
            TextFont {
                font_size: FontSize::Px(12.),
                ..default()
            },
            TextColor(Color::WHITE),
        ))
        .id();
    commands.entity(panel).add_child(row);
    row
}

/// Следующее значение по кругу; незнакомое откатывается к первому.
pub(super) fn next_in<T: Copy + PartialEq>(values: &[T], current: T) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .map_or(0, |index| (index + 1) % values.len());
    values[index]
}

/// Значение тумблера в строке. С заглавной — как во всех панелях стиля;
/// строчное `on`/`off` панели Stats к ней не относится, там это часть строки
/// состояния, а не значение тумблера.
pub(super) fn on_off(enabled: bool) -> &'static str {
    if enabled { "On" } else { "Off" }
}

#[cfg(test)]
mod tests;
