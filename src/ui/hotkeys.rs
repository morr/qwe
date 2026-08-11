//! Справка по хоткеям: неинтерактивный блок в правом нижнем углу, над панелью
//! Buildings. Фон — общий `UiOpacity::Medium`, как у рабочих панелей: на
//! `Light` (0.25) контраста поверх бежевой карты не хватало даже с тенью под
//! буквами, и справка читалась хуже всего на экране. Тень (`UI_TEXT_SHADOW`)
//! остаётся — панель узкая, и её край нередко приходится на светлое пятно.
//!
//! Список — единственное место, где хоткеи перечислены целиком; клавиши сами
//! живут в своих плагинах (`restart`, `ui::debug`, `movement`). Добавил
//! клавишу — допиши строку сюда.
//!
//! Только ASCII: `default_font` — минимальный шрифт без кириллицы и без
//! типографских символов вроде `−`, они рисуются пустотой или квадратом.

use bevy::prelude::*;

use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, UiPanelGapBelow,
    UiRightColumn, ui_color,
};

/// Мелкий шрифт: справку читают один раз, места она занимать не должна.
const FONT_PX: f32 = 11.0;
/// Ширина колонки с клавишей — все подписи в один символ.
const KEY_COLUMN_PX: f32 = 12.0;

const KEY_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const ACTION_COLOR: Color = Color::srgb(0.92, 0.94, 0.92);

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
            BackgroundColor(ui_color(UiOpacity::Medium)),
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
