//! Первопартийные виджеты панелей (`bevy_feathers`) и их тема.
//!
//! Виджеты — библиотечные целиком: серые кнопки со скруглением 4 px, синий у
//! «активных» (`ButtonVariant::Primary`), FiraSans, строка ростом
//! `size::ROW_HEIGHT`. А вот **плашки под ними — полупрозрачные**: панели лежат
//! поверх карты, и глухая заливка `create_dark_theme()` превращала их в стену,
//! за которой города не видно. Тема поэтому берётся не «как есть», а с
//! поправками — все они в [`create_panel_theme`] и все до одной цветовые.
//!
//! Цвет виджету задаёт не константа, а **дизайн-токен**: контрол носит
//! `ThemeBackgroundColor(tokens::BUTTON_BG)`, а какой это цвет — решает ресурс
//! `UiTheme`. Значит и собственные плашки панелей красятся токенами
//! (`tokens::PANE_BODY_BG`, `tokens::GROUP_BG`), а не своими цветами: иначе
//! смена темы перекрасила бы половину экрана и оставила вторую. Правка альфы в
//! одном месте — прямое следствие: прозрачной становится вся игровая плашка,
//! где бы она ни стояла.
//!
//! Соответствие состояний:
//!
//! | qwe | feathers |
//! |---|---|
//! | строка-значение в покое | `ButtonVariant::Plain` (фон `Color::NONE`) |
//! | отдельная кнопка в покое | `ButtonVariant::Normal` |
//! | «активна» (тумблер включён, город выбран, пауза) | `ButtonVariant::Primary` |
//! | инертная строка | `InteractionDisabled` |

use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::{SemanticToken, ThemeProps, UiTheme};
use bevy::feathers::{FeathersCorePlugin, palette, tokens};
use bevy::prelude::*;

/// Заместить цвет токена в теме. В 0.20 цвет живёт на **семантическом**
/// уровне (`semantic_base`), и у большинства токенов семантика общая с
/// другими (`PANE_HEADER_TEXT` и `TEXT_MAIN` сидят на `TEXT_DEFAULT`) —
/// красим семантику, как это делает сама библиотека. Отдельный семантический
/// токен (имя произвольное) нужен там, где наш цвет обязан остаться частным:
/// гасить `TEXT_ON_ACCENT` целиком нельзя — это цвет надписей на Primary-кнопках.
fn set_token(
    props: &mut ThemeProps,
    token: bevy::feathers::theme::ThemeToken,
    semantic: SemanticToken,
    color: Color,
) {
    props.token_assignments.insert(token, semantic.clone());
    props.semantic_base.insert(semantic, color);
}

/// Цвет всех игровых плашек — почти чёрный, чтобы светлая карта под ним не
/// поднимала текст. Прозрачность добавляют токены ниже; сам цвет один на все
/// слои, иначе полупрозрачные плашки разных оттенков дают на карте грязь.
const UI_COLOR: Color = Color::srgb(0.094, 0.102, 0.11);

/// Плотность плашки: сквозь неё виден город. Читаемость держит не она, а
/// яркость текста ([`MUTED_TEXT`]) — иначе плашку пришлось бы делать глухой,
/// то есть ровно тем, чем она быть не должна.
const PANEL_ALPHA: f32 = 0.72;

/// Шапка панели — плотнее тела: полоска вкладок отделяет себя от содержимого
/// не рамкой, а весом.
const HEADER_ALPHA: f32 = 0.85;

/// Вложенный блок внутри панели (блок счётчиков, заголовок секции) — легче
/// тела: он лежит НА плашке, и две одинаковые альфы сложились бы в глухую.
const BLOCK_ALPHA: f32 = 0.50;

/// Приглушённая подпись. Библиотечный `TEXT_DIM` — `LIGHT_GRAY_2` (oklch L .611),
/// то есть рассчитан на глухую плашку инспектора; на полупрозрачной поверх
/// светлого города он читался как серое на сером. «Приглушённость» здесь — на
/// ступень ниже белого, а не в середину шкалы.
const MUTED_TEXT: Color = Color::oklcha(0.90, 0.0015, 286.3, 1.0);

/// Тёмная тема feathers с игровыми поправками (см. [`PanelWidgetsPlugin`]).
fn create_panel_theme() -> ThemeProps {
    let mut props = create_dark_theme();
    use tokens::semantic;

    // --- плашки: полупрозрачные, потому что лежат поверх карты ---
    set_token(
        &mut props,
        tokens::PANE_BODY_BG,
        semantic::SURFACE_PANE_BODY,
        UI_COLOR.with_alpha(PANEL_ALPHA),
    );
    set_token(
        &mut props,
        tokens::SUBPANE_BODY_BG,
        semantic::SURFACE_SUBPANE_BODY,
        UI_COLOR.with_alpha(PANEL_ALPHA),
    );
    set_token(
        &mut props,
        tokens::PANE_HEADER_BG,
        semantic::SURFACE_PANE_HEADER,
        UI_COLOR.with_alpha(HEADER_ALPHA),
    );
    set_token(
        &mut props,
        tokens::SUBPANE_HEADER_BG,
        semantic::SURFACE_SUBPANE_HEADER,
        UI_COLOR.with_alpha(HEADER_ALPHA),
    );
    set_token(
        &mut props,
        tokens::GROUP_BG,
        semantic::SURFACE_GROUP,
        UI_COLOR.with_alpha(BLOCK_ALPHA),
    );

    // --- текст: обе роли ярче библиотечных ---
    // Панель полупрозрачна, и то, что под подписью, — не ровный серый фон
    // инспектора, а город: белые дома, зелёные парки, светлый асфальт. Текст
    // держит читаемость сам. `TEXT_DEFAULT` — это и `TEXT_MAIN`, и заголовки
    // секций (`PANE_HEADER_TEXT`): белый против приглушённого — единственное,
    // чем заголовок себя объявляет.
    set_token(
        &mut props,
        tokens::TEXT_MAIN,
        semantic::TEXT_DEFAULT,
        palette::WHITE,
    );
    set_token(&mut props, tokens::TEXT_DIM, semantic::TEXT_DIM, MUTED_TEXT);

    // Ползунок рисует своё значение внутри полосы — голым числом, без единиц.
    // «0.9» вместо «0.90 m» и «0.1» вместо «10%» на панели не сообщают ничего,
    // поэтому число там пишет кит (`ui/slider.rs`), а собственное feathers
    // гасится прозрачным цветом: погасить текст можно только его цветом —
    // маркер `SliderValueText` приватный, а формат зашит в `update_slider_pos`.
    // В 0.20 токены ползунка сидят на общих семантиках (`TEXT_ON_ACCENT` и
    // `TEXT_DISABLED`), поэтому им выдаются собственные.
    set_token(
        &mut props,
        tokens::SLIDER_TEXT,
        SemanticToken::new_static("qwe.slider.text"),
        Color::NONE,
    );
    set_token(
        &mut props,
        tokens::SLIDER_TEXT_DISABLED,
        SemanticToken::new_static("qwe.slider.text.disabled"),
        Color::NONE,
    );
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
