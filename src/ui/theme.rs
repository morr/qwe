//! Тема первопартийных виджетов (`bevy_feathers`) под облик панелей qwe.
//!
//! Feathers красит свои контролы не константами, а **дизайн-токенами**: виджет
//! носит `ThemeBackgroundColor(tokens::BUTTON_BG)`, а какой это цвет — решает
//! ресурс `UiTheme`. Поэтому перевести панели на первопартийные кнопки и
//! ползунки можно, не принимая редакторский вид feathers: берём её тёмную тему
//! и переопределяем те токены, что панели видно.
//!
//! Значения не выписаны заново, а посчитаны тем же
//! [`super::button_background`], которым красились самодельные кнопки, — иначе
//! у формулы «активная зелёная, под курсором светлее, под нажатием ещё светлее»
//! появилась бы вторая копия, и разъехаться они могли бы молча.
//!
//! Соответствие состояний:
//!
//! | qwe | feathers |
//! |---|---|
//! | кнопка в покое | `ButtonVariant::Normal` |
//! | «активна» (тумблер включён, город выбран, пауза) | `ButtonVariant::Primary` |
//! | инертная строка (`RowInert`) | `InteractionDisabled` |

use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::{ThemeProps, ThemeToken, UiTheme};
use bevy::feathers::{FeathersCorePlugin, tokens};
use bevy::prelude::*;

use super::{UiOpacity, button_background, ui_color};

/// Первопартийные виджеты панелей вместе с их темой — всё, что нужно, чтобы
/// кит кнопок и кит ползунков выглядели как в игре.
///
/// Отдельным плагином от `UiPlugin`: демо расталкивания
/// (`examples/demos/crowd_demo`) зовёт те же киты, а весь `UiPlugin` поднять не
/// может — он тянет панели, карту и настройки.
///
/// Именно `FeathersCorePlugin`, а не группа `FeathersPlugins`: группа добавляет
/// ещё `TabNavigationPlugin`, а с ним Tab фокусирует любой виджет панели (у всех
/// контролов `TabIndex(0)`), после чего пробел «нажимает» сфокусированную кнопку
/// И ставит симуляцию на паузу. Это тот же закон, что и «клик по панели не
/// доходит до мира», и `super::typing_in_text_input` его не ловит — фокус не на
/// текстовом поле. Без плагина `TabIndex`/`FocusIndicator` просто ничего не
/// делают.
pub struct PanelWidgetsPlugin;

impl Plugin for PanelWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FeathersCorePlugin)
            .insert_resource(UiTheme(create_qwe_theme()));
    }
}

/// Фон панели настроек — сквозь него видно карту.
pub const PANEL_BG: ThemeToken = ThemeToken::new_static("qwe.panel.bg");

/// Фон подложки внутри панели (строка-счётчик, ползунок): глушит карту сильнее.
pub const PANEL_BG_HEAVY: ThemeToken = ThemeToken::new_static("qwe.panel.bg.heavy");

/// Тёмная тема feathers с цветами панелей qwe поверх неё.
pub fn create_qwe_theme() -> ThemeProps {
    let mut props = create_dark_theme();
    let color = &mut props.color;

    // Обычная кнопка — тёмный прямоугольник панели.
    color.insert(tokens::BUTTON_BG, button_background(false, false, false));
    color.insert(
        tokens::BUTTON_BG_HOVER,
        button_background(false, false, true),
    );
    color.insert(
        tokens::BUTTON_BG_PRESSED,
        button_background(false, true, true),
    );
    // «Выключенная» — это покой, а не тусклость: инертная строка панели
    // перестаёт подсвечиваться под курсором, но выглядит как обычная.
    color.insert(
        tokens::BUTTON_BG_DISABLED,
        button_background(false, false, false),
    );

    // «Активная» кнопка — зелёная: тумблер включён, город выбран, пауза.
    color.insert(
        tokens::BUTTON_PRIMARY_BG,
        button_background(true, false, false),
    );
    color.insert(
        tokens::BUTTON_PRIMARY_BG_HOVER,
        button_background(true, false, true),
    );
    color.insert(
        tokens::BUTTON_PRIMARY_BG_PRESSED,
        button_background(true, true, true),
    );
    color.insert(
        tokens::BUTTON_PRIMARY_BG_DISABLED,
        button_background(true, false, false),
    );

    color.insert(PANEL_BG, ui_color(UiOpacity::Medium));
    color.insert(PANEL_BG_HEAVY, ui_color(UiOpacity::Heavy));

    props
}

#[cfg(test)]
mod tests {
    use bevy::color::Luminance;

    use super::*;

    /// Незнакомый токен feathers не роняет, а красит виджет в фуксию и пишет
    /// `warn_once` — то есть опечатка в теме видна только глазами на запущенной
    /// игре. Тест перечисляет всё, что панели у темы спрашивают.
    #[test]
    fn the_theme_answers_every_token_the_panels_ask_for() {
        let theme = create_qwe_theme();
        for token in [
            tokens::BUTTON_BG,
            tokens::BUTTON_BG_HOVER,
            tokens::BUTTON_BG_PRESSED,
            tokens::BUTTON_BG_DISABLED,
            tokens::BUTTON_TEXT,
            tokens::BUTTON_TEXT_DISABLED,
            tokens::BUTTON_PRIMARY_BG,
            tokens::BUTTON_PRIMARY_BG_HOVER,
            tokens::BUTTON_PRIMARY_BG_PRESSED,
            tokens::BUTTON_PRIMARY_BG_DISABLED,
            tokens::BUTTON_PRIMARY_TEXT,
            tokens::BUTTON_PRIMARY_TEXT_DISABLED,
            PANEL_BG,
            PANEL_BG_HEAVY,
        ] {
            assert!(
                theme.color.contains_key(&token),
                "в теме нет токена {token}"
            );
        }
    }

    /// Тот же инвариант, что у самодельных кнопок, но уже на токенах: наведение
    /// светлее покоя, нажатие светлее наведения — и у обычной кнопки, и у
    /// зелёной.
    #[test]
    fn the_button_tokens_lighten_from_rest_to_hover_to_press() {
        let theme = create_qwe_theme();
        let luminance = |token: &ThemeToken| theme.color[token].to_linear().luminance();

        for (rest, hover, pressed) in [
            (
                tokens::BUTTON_BG,
                tokens::BUTTON_BG_HOVER,
                tokens::BUTTON_BG_PRESSED,
            ),
            (
                tokens::BUTTON_PRIMARY_BG,
                tokens::BUTTON_PRIMARY_BG_HOVER,
                tokens::BUTTON_PRIMARY_BG_PRESSED,
            ),
        ] {
            assert!(
                luminance(&rest) < luminance(&hover),
                "наведение не осветлило {rest}"
            );
            assert!(
                luminance(&hover) < luminance(&pressed),
                "нажатие не осветлило {hover}"
            );
        }
    }

    /// Зелёная кнопка отличается от обычной не яркостью, а цветом: сравнивать
    /// их светимостями бессмысленно, а вот перепутать местами — легко.
    #[test]
    fn the_primary_button_is_the_toggle_green() {
        let theme = create_qwe_theme();
        assert_eq!(
            theme.color[&tokens::BUTTON_PRIMARY_BG],
            super::super::TOGGLE_ACTIVE_COLOR
        );
        assert_eq!(
            theme.color[&tokens::BUTTON_BG],
            ui_color(UiOpacity::Heavy),
            "обычная кнопка — фон панели"
        );
    }
}
