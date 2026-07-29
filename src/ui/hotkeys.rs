//! Справка по хоткеям: неинтерактивный блок в правом нижнем углу, над панелью
//! Buildings. Единственная панель на `UiOpacity::Light` — она ничего не
//! переключает, поэтому не должна глушить карту под собой так же, как рабочие
//! панели. Светлый фон почти не даёт контраста поверх бежевой карты, поэтому
//! буквы идут с общей `UI_TEXT_SHADOW` — без неё справка нечитаема.
//!
//! Список — единственное место, где хоткеи перечислены целиком; клавиши сами
//! живут в своих плагинах (`restart`, `ui::debug`, `movement`). Добавил
//! клавишу — допиши строку сюда.
//!
//! Только ASCII: `default_font` — минимальный шрифт без кириллицы и без
//! типографских символов вроде `−`, они рисуются пустотой или квадратом.

use bevy::prelude::*;

use crate::ui::{
    GameUiRoot, UI_BUILDINGS_PANEL_PX, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW,
    UI_TREES_PANEL_PX, UiOpacity, ui_color,
};

/// Отступ снизу: блок стоит над панелью Buildings в правой колонке панелей
/// (Trees → Buildings → справка).
const PANEL_BOTTOM_PX: f32 =
    UI_SCREEN_EDGE_PX_OFFSET + UI_TREES_PANEL_PX + UI_BUILDINGS_PANEL_PX;

/// Мелкий шрифт: справку читают один раз, места она занимать не должна.
const FONT_PX: f32 = 11.0;
/// Ширина колонки с клавишей — все подписи в один символ.
const KEY_COLUMN_PX: f32 = 12.0;

const KEY_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const ACTION_COLOR: Color = Color::srgb(0.86, 0.89, 0.86);

/// `(клавиша, что делает)`.
const HOTKEYS: &[(&str, &str)] = &[
    ("R", "restart"),
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
                bottom: px(PANEL_BOTTOM_PX),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(2.),
                padding: UiRect::all(px(8.)),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Light)),
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
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    column_gap: px(6.),
                    ..default()
                },
                children![
                    (
                        Text::new(*key),
                        TextFont {
                            font_size: FontSize::Px(FONT_PX),
                            ..default()
                        },
                        TextColor(KEY_COLOR),
                        UI_TEXT_SHADOW,
                        Node {
                            width: px(KEY_COLUMN_PX),
                            ..default()
                        },
                    ),
                    (
                        Text::new(*action),
                        TextFont {
                            font_size: FontSize::Px(FONT_PX),
                            ..default()
                        },
                        TextColor(ACTION_COLOR),
                        UI_TEXT_SHADOW,
                    ),
                ],
            ))
            .id();
        commands.entity(panel).add_child(row);
    }
}
