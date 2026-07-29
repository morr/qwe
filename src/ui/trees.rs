//! Панель стиля деревьев — вкладка Trees из «Style settings» watabou
//! (форма кроны, листва, чернила деталей, разброс яркости). Полей ввода в
//! `bevy_ui` нет, поэтому каждая строка — кнопка, листающая значение по кругу;
//! правка `TreeStyle` пересобирает кроны (`map::trees::rebuild_trees`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use bevy::prelude::*;

use crate::map::{TreeShape, TreeStyle};
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, ui_color};

/// Палитра листвы: зелень watabou плюс осенние и хвойные оттенки.
const FOLIAGE_PALETTE: [Color; 5] = [
    Color::srgb(0.42, 0.60, 0.33),
    Color::srgb(0.29, 0.40, 0.39),
    Color::srgb(0.51, 0.63, 0.24),
    Color::srgb(0.67, 0.55, 0.24),
    Color::srgb(0.35, 0.52, 0.45),
];

/// Палитра чернил: от почти чёрного до «деталей в цвет листвы» (белый вырез).
const DETAILS_PALETTE: [Color; 4] = [
    Color::srgb(0.004, 0.008, 0.024),
    Color::srgb(0.16, 0.22, 0.14),
    Color::srgb(0.30, 0.24, 0.14),
    Color::srgb(0.85, 0.90, 0.80),
];

/// Ступени разброса яркости (`treeVariance`).
const VARIANCE_STEPS: [f32; 5] = [0.0, 0.1, 0.2, 0.35, 0.5];

/// Строки — как у дебаг-тумблеров: плотный фон поверх полупрозрачной панели.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Какое поле стиля листает кнопка — она же адресует подпись и свотч.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum TreeStyleRow {
    Shape,
    Foliage,
    Details,
    Variance,
}

/// Текст значения в строке.
#[derive(Component)]
struct TreeStyleValueLabel(TreeStyleRow);

/// Квадрат-образец цвета в строке (пустой для нецветовых полей).
#[derive(Component)]
struct TreeStyleSwatch(TreeStyleRow);

pub struct UiTreeStylePlugin;

impl Plugin for UiTreeStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_tree_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    sync_row_values.run_if(resource_changed::<TreeStyle>),
                ),
            );
    }
}

fn render_tree_style_panel(mut commands: Commands, style: Res<TreeStyle>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
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
            Name::new("tree_style_panel"),
            children![(
                Text::new("Trees"),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            )],
        ))
        .id();

    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Shape,
        "Shape",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            let next = TreeShape::ALL
                .iter()
                .position(|&shape| shape == style.shape)
                .map_or(0, |index| (index + 1) % TreeShape::ALL.len());
            style.shape = TreeShape::ALL[next];
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Foliage,
        "Foliage",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            style.foliage = next_in(&FOLIAGE_PALETTE, style.foliage);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Details,
        "Crown details",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            style.details = next_in(&DETAILS_PALETTE, style.details);
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Variance,
        "Color variance",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            let current = VARIANCE_STEPS
                .iter()
                .position(|step| (step - style.variance).abs() < 1e-3)
                .map_or(0, |index| (index + 1) % VARIANCE_STEPS.len());
            style.variance = VARIANCE_STEPS[current];
        },
    );
}

/// Следующий цвет палитры за текущим; незнакомый цвет откатывается к первому.
fn next_in(palette: &[Color], current: Color) -> Color {
    let index = palette
        .iter()
        .position(|color| color.to_linear() == current.to_linear())
        .map_or(0, |index| (index + 1) % palette.len());
    palette[index]
}

fn spawn_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: TreeStyleRow,
    label: &str,
    style: &TreeStyle,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    // у нецветовых строк место под свотч остаётся (колонки не разъезжаются),
    // но и заливка, и рамка прозрачны — пустая рамка читалась как недоделка
    let swatch = swatch_color(row, style);
    let swatch_border = if swatch.is_some() {
        Color::srgba(1., 1., 1., 0.35)
    } else {
        Color::NONE
    };
    let button = commands
        .spawn((
            Button,
            row,
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
                    Text::new(label),
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
                    TreeStyleValueLabel(row),
                    Text::new(row_value(row, style)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
                (
                    TreeStyleSwatch(row),
                    Node {
                        width: px(14.),
                        height: px(14.),
                        border: UiRect::all(px(1.)),
                        ..default()
                    },
                    BorderColor::all(swatch_border),
                    BackgroundColor(swatch.unwrap_or(Color::NONE)),
                ),
            ],
        ))
        .observe(on_activate)
        .id();
    commands.entity(panel).add_child(button);
}

/// Текст значения строки: имя формы, hex цвета или число разброса.
fn row_value(row: TreeStyleRow, style: &TreeStyle) -> String {
    match row {
        TreeStyleRow::Shape => style.shape.label().to_string(),
        TreeStyleRow::Foliage => hex(style.foliage),
        TreeStyleRow::Details => hex(style.details),
        TreeStyleRow::Variance => format!("{:.2}", style.variance),
    }
}

/// Цвет свотча; у нецветовых строк свотча нет.
fn swatch_color(row: TreeStyleRow, style: &TreeStyle) -> Option<Color> {
    match row {
        TreeStyleRow::Foliage => Some(style.foliage),
        TreeStyleRow::Details => Some(style.details),
        _ => None,
    }
}

fn hex(color: Color) -> String {
    let srgba = color.to_srgba();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.0).round() as u8;
    format!(
        "{:02X}{:02X}{:02X}",
        channel(srgba.red),
        channel(srgba.green),
        channel(srgba.blue)
    )
}

/// Подсветка строки под курсором и при нажатии (как у дебаг-тумблеров).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<TreeStyleRow>>,
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

/// Актуализация подписей и свотчей после правки стиля (кликом или по BRP).
fn sync_row_values(
    style: Res<TreeStyle>,
    mut labels: Query<(&TreeStyleValueLabel, &mut Text)>,
    mut swatches: Query<(&TreeStyleSwatch, &mut BackgroundColor)>,
) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
    for (swatch, mut background) in &mut swatches {
        background.0 = swatch_color(swatch.0, &style).unwrap_or(Color::NONE);
    }
}
