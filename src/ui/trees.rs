//! Панель стиля деревьев — вкладка Trees из «Style settings» watabou
//! (форма кроны, листва, чернила деталей, разброс яркости) плюс ползунок
//! плотности посадки. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу; правка `TreeStyle` пересобирает кроны
//! (`map::trees::rebuild_trees`).

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, SliderValue, ValueChange};

use bevy::prelude::*;

use crate::map::{TREE_DENSITY_MAX, TreeShape, TreeStyle};
use crate::settings::{
    CONIFER_MIX_MAX, CONIFER_MIX_MIN, CONIFER_MIX_STEP, TREE_CONIFER_SHARE_MAX,
    TREE_CONIFER_SHARE_MIN, TREE_CONIFER_SHARE_STEP, TREE_DENSITY_MIN, TREE_DENSITY_STEP,
};
use crate::ui::slider::{SliderRow, quantize, spawn_slider_row};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
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

/// Какое поле стиля показывает строка — она же адресует подпись и свотч.
/// `ConiferShare`, `ConiferMix` и `Density` — не кнопки, а ползунки, но подпись
/// значения у них общая с остальными строками.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum TreeStyleRow {
    Woods,
    Standalone,
    Shape,
    Foliage,
    Details,
    Variance,
    ConiferShare,
    ConiferMix,
    Density,
}

/// Текст значения в строке.
#[derive(Component)]
struct TreeStyleValueLabel(TreeStyleRow);

/// Квадрат-образец цвета в строке (пустой для нецветовых полей).
#[derive(Component)]
struct TreeStyleSwatch(TreeStyleRow);

/// Ползунок строки.
#[derive(Component)]
struct TreeStyleSlider(TreeStyleRow);

/// Блок строки, живущей только у формы `Mixed` (доля хвои и примесь): вне неё
/// строка убирается из раскладки целиком (`Display::None`), а не гасится, —
/// иначе в панели остаётся необъяснимая дыра.
#[derive(Component)]
struct MixedOnlyRow;

pub struct UiTreeStylePlugin;

impl Plugin for UiTreeStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_tree_style_panel)
            .add_systems(
                Update,
                (
                    highlight_rows,
                    sync_row_values.run_if(resource_changed::<TreeStyle>),
                    // без run_if: строк две, а зависеть от того, попал ли
                    // первый кадр в окно `resource_changed`, тут ни к чему
                    sync_mixed_row_visibility,
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
            UiRightColumnSlot(1),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("tree_style_panel"),
            children![panel_header("Trees", PanelCount::Trees)],
        ))
        .id();

    // тумблеры источников — первыми: они решают, есть ли на карте лес и
    // одиночные деревья вообще, остальные ручки правят вид уже стоящих крон
    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Woods,
        "Woods",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            style.woods = !style.woods;
        },
    );
    spawn_row(
        &mut commands,
        panel,
        TreeStyleRow::Standalone,
        "Individual",
        &style,
        |_activate: On<Activate>, mut style: ResMut<TreeStyle>| {
            style.standalone = !style.standalone;
        },
    );
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
    // сразу под формой: доля и примесь уточняют именно её и только при
    // `Mixed` видны
    let share_row = spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Conifer share",
            value: style.conifer_share,
            value_text: row_value(TreeStyleRow::ConiferShare, &style),
            range: (
                TREE_CONIFER_SHARE_MIN,
                TREE_CONIFER_SHARE_MAX,
                TREE_CONIFER_SHARE_STEP,
            ),
        },
        TreeStyleValueLabel(TreeStyleRow::ConiferShare),
        TreeStyleSlider(TreeStyleRow::ConiferShare),
        on_conifer_share_change,
    );
    commands.entity(share_row).insert(MixedOnlyRow);
    let mix_row = spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Mix",
            value: style.conifer_mix,
            value_text: row_value(TreeStyleRow::ConiferMix, &style),
            range: (CONIFER_MIX_MIN, CONIFER_MIX_MAX, CONIFER_MIX_STEP),
        },
        TreeStyleValueLabel(TreeStyleRow::ConiferMix),
        TreeStyleSlider(TreeStyleRow::ConiferMix),
        on_conifer_mix_change,
    );
    commands.entity(mix_row).insert(MixedOnlyRow);
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
        SliderRow {
            label: "Density",
            value: style.density,
            value_text: row_value(TreeStyleRow::Density, &style),
            range: (TREE_DENSITY_MIN, TREE_DENSITY_MAX, TREE_DENSITY_STEP),
        },
        TreeStyleValueLabel(TreeStyleRow::Density),
        TreeStyleSlider(TreeStyleRow::Density),
        on_density_change,
    );
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

/// Примесь пород: тот же дискретный шаг — каждый шаг пересемплирует поле хвои
/// и пересобирает кроны.
fn on_conifer_mix_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut style: ResMut<TreeStyle>,
) {
    let stepped = quantize(
        change.value,
        CONIFER_MIX_MIN,
        CONIFER_MIX_MAX,
        CONIFER_MIX_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (style.conifer_mix - stepped).abs() > f32::EPSILON {
        style.conifer_mix = stepped;
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
        TreeStyleRow::Woods => (if style.woods { "On" } else { "Off" }).to_string(),
        TreeStyleRow::Standalone => (if style.standalone { "On" } else { "Off" }).to_string(),
        TreeStyleRow::Shape => style.shape.label().to_string(),
        TreeStyleRow::Foliage => hex(style.foliage),
        TreeStyleRow::Details => hex(style.details),
        TreeStyleRow::Variance => format!("{:.2}", style.variance),
        TreeStyleRow::ConiferShare => format!("{:.0}%", style.conifer_share * 100.),
        TreeStyleRow::ConiferMix => format!("{:.0}%", style.conifer_mix * 100.),
        TreeStyleRow::Density => format!("{:.2}x", style.density),
    }
}

/// Значение ползунка строки — только у строк-ползунков; у строк-кнопок
/// ползунка нет, и `None` рвёт их синхронизацию по значению.
fn slider_value(row: TreeStyleRow, style: &TreeStyle) -> Option<f32> {
    match row {
        TreeStyleRow::ConiferShare => Some(style.conifer_share),
        TreeStyleRow::ConiferMix => Some(style.conifer_mix),
        TreeStyleRow::Density => Some(style.density),
        TreeStyleRow::Woods
        | TreeStyleRow::Standalone
        | TreeStyleRow::Shape
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

/// Доля хвои и примесь есть только у смешанного леса — на прочих формах их
/// строки уходят из раскладки, и панель становится ниже (её место в правой
/// колонке пересчитает `ui::stack_right_column`).
fn sync_mixed_row_visibility(
    style: Res<TreeStyle>,
    mut rows: Query<&mut Node, With<MixedOnlyRow>>,
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
