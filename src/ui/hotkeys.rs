//! Справка по хоткеям: неинтерактивный блок в правом нижнем углу. Настройкой
//! не является и потому стоит поверх карты, а не во вкладке — читают её как раз
//! тогда, когда до панели настроек дела нет. Плашка — та же, что у панелей
//! (`panel_background`), и по той же причине: сквозь что-то более прозрачное
//! бежевая карта пробивала текст даже с тенью, и справка читалась хуже всего на
//! экране.
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

use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, panel_background};

/// Ширина колонки с клавишей — по самой длинной подписи (`Tab`), чтобы
/// описания стояли в одну вертикаль.
const KEY_COLUMN_PX: f32 = 28.0;

/// `(клавиша, что делает)`.
const HOTKEYS: &[(&str, &str)] = &[
    ("R", "restart (RR - to portal)"),
    ("G", "gizmos"),
    ("N", "navmesh"),
    ("M", "movepath"),
    ("Tab", "settings panel"),
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
            GameUiRoot,
            Visibility::Hidden,
            Name::new("hotkeys_panel"),
        ))
        .id();

    for (key, action) in HOTKEYS {
        let row = commands
            .spawn((
                crate::ui::ui_row(6.),
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
