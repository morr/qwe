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
//! двигалась мышь или нет. Здесь система одна и пишет через `set_if_neq`.

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use super::{ROW_LABEL_COLOR, TOGGLE_HOVER_LIGHTEN, TOGGLE_PRESSED_LIGHTEN, UiOpacity, ui_color};

/// Фон строки в покое: плотный, поверх полупрозрачной панели.
pub(super) const ROW_LIGHTEN: f32 = 0.0;

/// Отступ слева у обычной строки. Вложенная (панель Navigation) прибавляет к
/// нему свой `NESTED_ROW_INDENT_PX`.
pub(super) const ROW_LEFT_PX: f32 = 8.0;

pub(super) fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Строка, которую подсвечивает [`highlight_value_rows`]. Своя метка строки у
/// панели остаётся — она адресует значение при синхронизации; эта отвечает
/// только за подсветку и потому общая.
#[derive(Component)]
pub(super) struct ValueRow;

/// Строка, клик по которой сейчас ничего не делает: подсвечивать её нельзя —
/// подсветка обещала бы, что клик что-то сделает.
///
/// Ставится и снимается панелью (единственный носитель на сегодня — тумблер
/// расталкивания в Navigation, недоступный в детерминированном режиме и на
/// сеточной навигации). Метка, а не проверка ресурсов внутри подсветки:
/// иначе общая система знала бы про режимы мира, до которых ей нет дела.
#[derive(Component)]
pub(super) struct RowInert;

/// Кнопка-строка: серая подпись слева, белое значение справа, клик — на
/// `on_activate`. Возвращает строку, чтобы вызывающая панель довесила свои
/// метки и, если надо, свотч.
///
/// `Hovered` кормит UI-picking, `Pressed` ставит виджет — оба нужны.
pub(super) fn spawn_value_row<M>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    left_px: f32,
    value_marker: impl Bundle,
    value: String,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let row = commands
        .spawn((
            Button,
            ValueRow,
            Pickable::default(),
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
                    left: px(left_px),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                (
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(ROW_LABEL_COLOR),
                    Node {
                        flex_grow: 1.,
                        ..default()
                    },
                ),
                (
                    value_marker,
                    Text::new(value),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(on_activate)
        .id();
    commands.entity(panel).add_child(row);
    row
}

/// Осветление строки под курсором и при нажатии — одна система на все панели.
///
/// `set_if_neq`, а не присваивание: система крутится каждый кадр по всем
/// строкам всех панелей, а меняется из них максимум одна — та, что под
/// курсором.
pub(super) fn highlight_value_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, Has<RowInert>, &mut BackgroundColor), With<ValueRow>>,
) {
    for (hovered, pressed, inert, mut background) in &mut rows {
        let lighten = if inert {
            ROW_LIGHTEN
        } else if pressed {
            TOGGLE_PRESSED_LIGHTEN
        } else if hovered.get() {
            TOGGLE_HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.set_if_neq(BackgroundColor(row_color(lighten)));
    }
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
