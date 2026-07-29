//! Панель режима отображения высоты зданий: одна строка-кнопка, листающая
//! `BuildingHeightMode` по кругу (фасадная полоса → тени → тени+тон → 2.5D).
//! Правка ресурса пересобирает зданиевые слои (`map::buildings::rebuild_buildings`).

use bevy::color::Mix;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::map::BuildingHeightMode;
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};

/// Строки — как у панели деревьев: плотный фон поверх полупрозрачной панели.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

/// Отступ снизу: панель стоит над панелью Trees (та ~150 px высотой).
const PANEL_BOTTOM_PX: f32 = UI_SCREEN_EDGE_PX_OFFSET + 158.0;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Кнопка-строка режима высоты — адресует и подсветку, и подпись значения.
#[derive(Component)]
struct BuildingHeightRow;

/// Текст значения в строке.
#[derive(Component)]
struct BuildingHeightValueLabel;

pub struct UiBuildingStylePlugin;

impl Plugin for UiBuildingStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_building_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    // и клик по кнопке, и правка по BRP
                    sync_row_value.run_if(resource_changed::<BuildingHeightMode>),
                ),
            );
    }
}

fn render_building_style_panel(mut commands: Commands, mode: Res<BuildingHeightMode>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(PANEL_BOTTOM_PX),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(210.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("building_style_panel"),
            children![(
                Text::new("Buildings"),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            )],
        ))
        .id();

    let button = commands
        .spawn((
            Button,
            BuildingHeightRow,
            Pickable::default(),
            // `Hovered` кормит UI-picking, `Pressed` ставит виджет — оба нужны
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
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                (
                    Text::new("Height"),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.78, 0.75)),
                    Node {
                        flex_grow: 1.,
                        ..default()
                    },
                ),
                (
                    BuildingHeightValueLabel,
                    Text::new(mode.label()),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(
            |_activate: On<Activate>, mut mode: ResMut<BuildingHeightMode>| {
                *mode = mode.next();
            },
        )
        .id();
    commands.entity(panel).add_child(button);
}

/// Подсветка строки под курсором и при нажатии (как у панели деревьев).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<BuildingHeightRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.0 = row_color(lighten);
    }
}

/// Актуализация подписи после смены режима (кликом или по BRP).
fn sync_row_value(
    mode: Res<BuildingHeightMode>,
    mut labels: Query<&mut Text, With<BuildingHeightValueLabel>>,
) {
    for mut text in &mut labels {
        text.0 = mode.label().to_string();
    }
}
