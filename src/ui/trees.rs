//! Панель стиля деревьев — вкладка Trees из «Style settings» watabou
//! (форма кроны, листва, чернила деталей, разброс яркости) плюс ползунок
//! плотности посадки. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу; правка `TreeStyle` пересобирает кроны
//! (`map::trees::rebuild_trees`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{
    Activate, Button, Slider, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick,
    ValueChange,
};

use bevy::prelude::*;

use crate::map::{TREE_DENSITY_MAX, TreeShape, TreeStyle};
use crate::settings::{
    TREE_CONIFER_SHARE_MAX, TREE_CONIFER_SHARE_MIN, TREE_CONIFER_SHARE_STEP, TREE_DENSITY_MIN,
    TREE_DENSITY_STEP,
};
use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiOpacity, UiRightColumnSlot, ui_color,
};

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

/// Дорожка и ползунок плотности; высота ползунка задаёт и высоту строки.
const SLIDER_HEIGHT_PX: f32 = 12.0;
const SLIDER_TRACK_PX: f32 = 4.0;
const SLIDER_TRACK_COLOR: Color = Color::srgba(1., 1., 1., 0.18);
const SLIDER_THUMB_COLOR: Color = Color::srgba(1., 1., 1., 0.75);
const SLIDER_THUMB_HOVER_COLOR: Color = Color::WHITE;

/// Какое поле стиля показывает строка — она же адресует подпись и свотч.
/// `ConiferShare` и `Density` — не кнопки, а ползунки, но подпись значения у
/// них общая с остальными строками.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum TreeStyleRow {
    Shape,
    Foliage,
    Details,
    Variance,
    ConiferShare,
    Density,
}

/// Текст значения в строке.
#[derive(Component)]
struct TreeStyleValueLabel(TreeStyleRow);

/// Квадрат-образец цвета в строке (пустой для нецветовых полей).
#[derive(Component)]
struct TreeStyleSwatch(TreeStyleRow);

/// Ползунок строки и его бегунок.
#[derive(Component)]
struct TreeStyleSlider(TreeStyleRow);

#[derive(Component)]
struct TreeStyleThumb;

/// Блок строки доли хвои: доля есть только у формы `Mixed`, и вне неё строка
/// убирается из раскладки целиком (`Display::None`), а не гасится, — иначе в
/// панели остаётся необъяснимая дыра.
#[derive(Component)]
struct ConiferShareRow;

pub struct UiTreeStylePlugin;

impl Plugin for UiTreeStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_tree_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    sync_row_values.run_if(resource_changed::<TreeStyle>),
                    // без run_if: строка одна, а зависеть от того, попал ли
                    // первый кадр в окно `resource_changed`, тут ни к чему
                    sync_conifer_row_visibility,
                    sync_slider_thumbs,
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
            UiRightColumnSlot(0),
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
    // сразу под формой: доля уточняет именно её и только при `Mixed` видна
    let conifer_row = spawn_slider_row(
        &mut commands,
        panel,
        TreeStyleRow::ConiferShare,
        "Conifer share",
        &style,
        (
            TREE_CONIFER_SHARE_MIN,
            TREE_CONIFER_SHARE_MAX,
            TREE_CONIFER_SHARE_STEP,
        ),
        on_conifer_share_change,
    );
    commands.entity(conifer_row).insert(ConiferShareRow);
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
    spawn_slider_row(
        &mut commands,
        panel,
        TreeStyleRow::Density,
        "Density",
        &style,
        (TREE_DENSITY_MIN, TREE_DENSITY_MAX, TREE_DENSITY_STEP),
        on_density_change,
    );
}

/// Строка-ползунок: подпись со значением и под ней сам ползунок. Возвращает
/// блок строки — на него навешивается маркер, если строку кто-то адресует
/// (`ConiferShareRow` управляет её видимостью).
fn spawn_slider_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: TreeStyleRow,
    label: &str,
    style: &TreeStyle,
    (min, max, step): (f32, f32, f32),
    on_change: impl IntoObserverSystem<ValueChange<f32>, (), M>,
) -> Entity {
    let block = commands
        .spawn((
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(6.),
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(6.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![(
                Node {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6.),
                    ..default()
                },
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
                ],
            )],
        ))
        .id();

    let slider = commands
        .spawn((
            TreeStyleSlider(row),
            Slider {
                // клик по дорожке ставит бегунок туда, куда ткнули
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderValue(slider_value(row, style).expect("slider row has a value")),
            SliderRange::new(min, max),
            SliderStep(step),
            Pickable::default(),
            Hovered::default(),
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                height: px(SLIDER_HEIGHT_PX),
                ..default()
            },
            children![
                // дорожка
                (
                    Node {
                        height: px(SLIDER_TRACK_PX),
                        border_radius: BorderRadius::all(px(SLIDER_TRACK_PX / 2.)),
                        ..default()
                    },
                    BackgroundColor(SLIDER_TRACK_COLOR),
                ),
                // невидимая направляющая: она короче дорожки на ширину бегунка,
                // поэтому бегунок ставится простым процентом, без замеров
                (
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(0.),
                        right: px(SLIDER_HEIGHT_PX),
                        top: px(0.),
                        bottom: px(0.),
                        ..default()
                    },
                    children![(
                        TreeStyleThumb,
                        SliderThumb,
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(SLIDER_HEIGHT_PX),
                            height: px(SLIDER_HEIGHT_PX),
                            border_radius: BorderRadius::MAX,
                            ..default()
                        },
                        BackgroundColor(SLIDER_THUMB_COLOR),
                    )],
                ),
            ],
        ))
        .observe(on_change)
        .id();
    commands.entity(block).add_child(slider);
    commands.entity(panel).add_child(block);
    block
}

/// Ползунок ведёт себя дискретно: значение с драга округляется до шага, и
/// стиль правится только когда шаг действительно сменился — иначе каждый
/// пиксель протяжки пересобирал бы все кроны.
fn on_density_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<TreeStyle>,
) {
    let stepped = quantize(
        change.value,
        TREE_DENSITY_MIN,
        TREE_DENSITY_MAX,
        TREE_DENSITY_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.density - stepped).abs() > f32::EPSILON {
        style.density = stepped;
    }
}

/// То же для доли хвои: шаг ползунка меняет породу у целых массивов, так что
/// пересборка на каждый пиксель протяжки тем более не нужна.
fn on_conifer_share_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<TreeStyle>,
) {
    let stepped = quantize(
        change.value,
        TREE_CONIFER_SHARE_MIN,
        TREE_CONIFER_SHARE_MAX,
        TREE_CONIFER_SHARE_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.conifer_share - stepped).abs() > f32::EPSILON {
        style.conifer_share = stepped;
    }
}

fn quantize(value: f32, min: f32, max: f32, step: f32) -> f32 {
    ((value / step).round() * step).clamp(min, max)
}

/// Позиция бегунка по значению плюс подсветка под курсором и при протяжке.
fn sync_slider_thumbs(
    sliders: Query<
        (Entity, &SliderValue, &SliderRange, &Hovered),
        (
            With<TreeStyleSlider>,
            Or<(Changed<SliderValue>, Changed<Hovered>)>,
        ),
    >,
    children: Query<&Children>,
    mut thumbs: Query<(&mut Node, &mut BackgroundColor), With<TreeStyleThumb>>,
) {
    for (slider, value, range, hovered) in &sliders {
        for child in children.iter_descendants(slider) {
            let Ok((mut node, mut background)) = thumbs.get_mut(child) else {
                continue;
            };
            node.left = percent(range.thumb_position(value.0) * 100.);
            background.0 = if hovered.get() {
                SLIDER_THUMB_HOVER_COLOR
            } else {
                SLIDER_THUMB_COLOR
            };
        }
    }
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
        TreeStyleRow::ConiferShare => format!("{:.0}%", style.conifer_share * 100.),
        TreeStyleRow::Density => format!("{:.2}x", style.density),
    }
}

/// Значение ползунка строки — только у строк-ползунков; у строк-кнопок
/// ползунка нет, и `None` рвёт их синхронизацию по значению.
fn slider_value(row: TreeStyleRow, style: &TreeStyle) -> Option<f32> {
    match row {
        TreeStyleRow::ConiferShare => Some(style.conifer_share),
        TreeStyleRow::Density => Some(style.density),
        TreeStyleRow::Shape
        | TreeStyleRow::Foliage
        | TreeStyleRow::Details
        | TreeStyleRow::Variance => None,
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
    sliders: Query<(Entity, &TreeStyleSlider, &SliderValue)>,
    mut commands: Commands,
) {
    for (label, mut text) in &mut labels {
        text.0 = row_value(label.0, &style);
    }
    for (swatch, mut background) in &mut swatches {
        background.0 = swatch_color(swatch.0, &style).unwrap_or(Color::NONE);
    }
    // стиль правится и мимо ползунков (по BRP, из сохранённых настроек) —
    // бегунок обязан переехать; при протяжке значения уже совпадают.
    // `SliderValue` — immutable-компонент, меняется только вставкой
    for (slider, row, value) in &sliders {
        let Some(target) = slider_value(row.0, &style) else {
            continue;
        };
        if (value.0 - target).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(target));
        }
    }
}

/// Доля хвои есть только у смешанного леса — на прочих формах строка уходит из
/// раскладки, и панель становится ниже (её место в правой колонке пересчитает
/// `ui::stack_right_column`).
fn sync_conifer_row_visibility(
    style: Res<TreeStyle>,
    mut rows: Query<&mut Node, With<ConiferShareRow>>,
) {
    let display = if style.shape == TreeShape::Mixed {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut rows {
        if node.display != display {
            node.display = display;
        }
    }
}
