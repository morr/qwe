//! Панель стиля деревьев — вкладка Trees из «Style settings» watabou
//! (форма кроны, листва, чернила деталей, разброс яркости) плюс ползунок
//! плотности посадки. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу; правка `TreeStyle` пересобирает кроны
//! (`map::trees::rebuild_trees`).

use bevy::ecs::system::IntoObserverSystem;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::map::{TREE_DENSITY_MAX, TreeShape, TreeStyle};
use crate::settings::{
    TREE_CONIFER_SHARE_MAX, TREE_CONIFER_SHARE_MIN, TREE_CONIFER_SHARE_STEP, TREE_DENSITY_MIN,
    TREE_DENSITY_STEP, TREE_NOISE_MIX_MAX, TREE_NOISE_MIX_MIN, TREE_NOISE_MIX_STEP,
};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, on_off, spawn_value_row};
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

/// Какое поле стиля показывает строка-кнопка — она же адресует подпись и
/// свотч. Ползунки панели (доля хвои, примесь, плотность) сюда не входят:
/// их подписи ведёт кит ручек по своей привязке.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum TreeStyleRow {
    Woods,
    Standalone,
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

/// Блок строки, живущей только у формы `Mixed` (доля хвои и примесь): вне неё
/// строка убирается из раскладки целиком (`Display::None`), а не гасится, —
/// иначе в панели остаётся необъяснимая дыра.
#[derive(Component)]
struct MixedOnlyRow;

pub struct UiTreeStylePlugin;

impl Plugin for UiTreeStylePlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<TreeStyle>()
            .add_systems(Startup, render_tree_style_panel)
            .add_systems(
                Update,
                (
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
    let share_row = spawn_knob(
        &mut commands,
        panel,
        "Conifer share",
        &*style,
        SliderBinding {
            get: |style| style.conifer_share,
            set: |style, value| style.conifer_share = value,
            range: (
                TREE_CONIFER_SHARE_MIN,
                TREE_CONIFER_SHARE_MAX,
                TREE_CONIFER_SHARE_STEP,
            ),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
    commands.entity(share_row).insert(MixedOnlyRow);
    let mix_row = spawn_knob(
        &mut commands,
        panel,
        "Noise mix",
        &*style,
        SliderBinding {
            get: |style| style.noise_mix,
            set: |style, value| style.noise_mix = value,
            range: (TREE_NOISE_MIX_MIN, TREE_NOISE_MIX_MAX, TREE_NOISE_MIX_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
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
    spawn_knob(
        &mut commands,
        panel,
        "Density",
        &*style,
        SliderBinding {
            get: |style| style.density,
            set: |style, value| style.density = value,
            range: (TREE_DENSITY_MIN, TREE_DENSITY_MAX, TREE_DENSITY_STEP),
            text: |value| format!("{value:.2}x"),
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
    let button = spawn_value_row(
        commands,
        panel,
        label,
        ROW_LEFT_PX,
        TreeStyleValueLabel(row),
        row_value(row, style),
        on_activate,
    );
    // у нецветовых строк место под свотч остаётся (колонки не разъезжаются),
    // но и заливка, и рамка прозрачны — пустая рамка читалась как недоделка.
    // Третьим ребёнком, после значения: `with_child` дописывает в конец
    let swatch = swatch_color(row, style);
    let swatch_border = if swatch.is_some() {
        Color::srgba(1., 1., 1., 0.35)
    } else {
        Color::NONE
    };
    commands.entity(button).insert(row).with_child((
        TreeStyleSwatch(row),
        Node {
            width: px(14.),
            height: px(14.),
            border: UiRect::all(px(1.)),
            ..default()
        },
        BorderColor::all(swatch_border),
        BackgroundColor(swatch.unwrap_or(Color::NONE)),
    ));
}

/// Текст значения строки: имя формы, hex цвета или число разброса.
fn row_value(row: TreeStyleRow, style: &TreeStyle) -> String {
    match row {
        TreeStyleRow::Woods => on_off(style.woods).to_string(),
        TreeStyleRow::Standalone => on_off(style.standalone).to_string(),
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

/// Актуализация подписей и свотчей строк-кнопок после правки стиля (кликом
/// или по BRP). Подписи и бегунки ползунков ведёт кит ручек.
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
