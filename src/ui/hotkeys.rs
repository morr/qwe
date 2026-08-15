//! Справка по хоткеям: неинтерактивный блок в правом нижнем углу, над панелью
//! Buildings. Плашка — та же, что у рабочих панелей (`panel_background`), и по
//! той же причине: сквозь что-то более прозрачное бежевая карта пробивала текст
//! даже с тенью, и справка читалась хуже всего на экране.
//!
//! Список — единственное место, где хоткеи перечислены целиком; клавиши сами
//! живут в своих плагинах (`restart`, `ui::debug`, `movement`). Добавил
//! клавишу — допиши строку сюда.
//!
//! Только ASCII: `default_font` — минимальный шрифт без кириллицы и без
//! типографских символов вроде `−`, они рисуются пустотой или квадратом.

use bevy::prelude::*;

use bevy::feathers::theme::ThemeTextColor;
use bevy::feathers::tokens;

use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiPanelGapBelow, UiRightColumn, panel_background,
};

/// Ширина колонки с клавишей — все подписи в один символ.
const KEY_COLUMN_PX: f32 = 14.0;

/// `(клавиша, что делает)`.
const HOTKEYS: &[(&str, &str)] = &[
    ("R", "restart (RR - to portal)"),
    ("G", "gizmos"),
    ("N", "navmesh"),
    ("M", "movepath"),
];

pub struct UiHotkeysPlugin;

impl Plugin for UiHotkeysPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_hotkeys_panel);
    }
}

fn render_hotkeys_panel(mut commands: Commands) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // последняя в правой колонке: `bottom` доедет от
                // stack_right_column по высотам всех панелей под ней
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(2.),
                padding: UiRect::all(px(8.)),
                ..default()
            },
            panel_background(),
            // справка ничего не принимает: клики сквозь неё уходят на карту
            Pickable::IGNORE,
            UiRightColumn::Hotkeys,
            // справка — не настройка карты: от блока панелей OSM её отделяет
            // зазор в отступ от края экрана
            UiPanelGapBelow,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("hotkeys_panel"),
        ))
        .id();

    for (key, action) in HOTKEYS {
        let row = commands
            .spawn((
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    column_gap: px(6.),
                    ..default()
                },
                crate::ui::text_container(),
                children![
                    (
                        Text::new(*key),
                        ThemeTextColor(tokens::TEXT_MAIN),
                        Node {
                            width: px(KEY_COLUMN_PX),
                            ..default()
                        },
                    ),
                    (Text::new(*action), ThemeTextColor(tokens::TEXT_DIM)),
                ],
            ))
            .id();
        commands.entity(panel).add_child(row);
    }
}
