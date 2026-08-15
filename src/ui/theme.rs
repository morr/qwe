//! Первопартийные виджеты панелей (`bevy_feathers`) и их тема.
//!
//! Тема принята **как есть** — `create_dark_theme()` библиотеки, с одним
//! погашенным токеном (см. [`create_panel_theme`]). Панели поэтому выглядят так
//! же, как галерея feathers: серые кнопки со скруглением 4 px, синий у
//! «активных» (`ButtonVariant::Primary`), FiraSans 14 px, строка ростом
//! `size::ROW_HEIGHT`.
//!
//! Цвет виджету задаёт не константа, а **дизайн-токен**: контрол носит
//! `ThemeBackgroundColor(tokens::BUTTON_BG)`, а какой это цвет — решает ресурс
//! `UiTheme`. Значит и собственные плашки панелей красятся токенами
//! (`tokens::PANE_BODY_BG`, `tokens::GROUP_BODY_BG`), а не своими цветами: иначе
//! смена темы перекрасила бы половину экрана и оставила вторую.
//!
//! Соответствие состояний:
//!
//! | qwe | feathers |
//! |---|---|
//! | кнопка в покое | `ButtonVariant::Normal` |
//! | «активна» (тумблер включён, город выбран, пауза) | `ButtonVariant::Primary` |
//! | инертная строка | `InteractionDisabled` |

use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::{ThemeProps, UiTheme};
use bevy::feathers::{FeathersCorePlugin, tokens};
use bevy::prelude::*;

/// Тёмная тема feathers с одной поправкой (см. [`PanelWidgetsPlugin`]).
fn create_panel_theme() -> ThemeProps {
    let mut props = create_dark_theme();
    // Ползунок рисует своё значение внутри полосы — голым числом, без единиц.
    // «0.9» вместо «0.90 m» и «0.1» вместо «10%» на панели не сообщают ничего,
    // поэтому число там пишет кит (`ui/slider.rs`), а собственное feathers
    // гасится прозрачным цветом. Это единственный токен, который мы у темы
    // отбираем: погасить текст можно только его цветом — маркер `SliderValueText`
    // приватный, а формат зашит в `update_slider_pos`.
    props.color.insert(tokens::SLIDER_TEXT, Color::NONE);
    props
        .color
        .insert(tokens::SLIDER_TEXT_DISABLED, Color::NONE);
    props
}

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
            .insert_resource(UiTheme(create_panel_theme()));
    }
}
