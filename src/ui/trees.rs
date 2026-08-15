//! Панель стиля деревьев — вкладка Trees из «Style settings» watabou
//! (форма кроны, листва, чернила деталей, разброс яркости) плюс ползунок
//! плотности посадки. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу; правка `TreeStyle` пересобирает кроны
//! (`map::trees::rebuild_trees`).

use bevy::prelude::*;

use crate::map::{TREE_DENSITY_MAX, TreeShape, TreeStyle};
use crate::settings::{
    TREE_CONIFER_SHARE_MAX, TREE_CONIFER_SHARE_MIN, TREE_CONIFER_SHARE_STEP, TREE_DENSITY_MIN,
    TREE_DENSITY_STEP, TREE_NOISE_MIX_MAX, TREE_NOISE_MIX_MIN, TREE_NOISE_MIX_STEP,
};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::{GameUiRoot, PanelCount, UiRightColumn, panel_header, right_panel};

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

/// Квадрат-образец цвета в строке — на строках, которые цвет и правят;
/// у остальных свотч есть, но прозрачный, и этого компонента не несёт.
#[derive(Component)]
struct TreeStyleSwatch(fn(&TreeStyle) -> Color);

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
                    sync_swatches.run_if(resource_changed::<TreeStyle>),
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
            right_panel(UiRightColumn::Trees),
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
        None,
        "Woods",
        &style,
        CycleBinding {
            cycle: |style| style.woods = !style.woods,
            text: |style| on_off(style.woods).to_string(),
        },
    );
    spawn_row(
        &mut commands,
        panel,
        None,
        "Individual",
        &style,
        CycleBinding {
            cycle: |style| style.standalone = !style.standalone,
            text: |style| on_off(style.standalone).to_string(),
        },
    );
    spawn_row(
        &mut commands,
        panel,
        None,
        "Shape",
        &style,
        CycleBinding {
            cycle: |style| style.shape = next_in(&TreeShape::ALL, style.shape),
            text: |style| style.shape.label().to_string(),
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
        Some(|style: &TreeStyle| style.foliage),
        "Foliage",
        &style,
        CycleBinding {
            cycle: |style| style.foliage = next_color(&FOLIAGE_PALETTE, style.foliage),
            text: |style| hex(style.foliage),
        },
    );
    spawn_row(
        &mut commands,
        panel,
        Some(|style: &TreeStyle| style.details),
        "Crown details",
        &style,
        CycleBinding {
            cycle: |style| style.details = next_color(&DETAILS_PALETTE, style.details),
            text: |style| hex(style.details),
        },
    );
    spawn_row(
        &mut commands,
        panel,
        None,
        "Color variance",
        &style,
        CycleBinding {
            // ступени сравниваются с допуском: значение приходит и из
            // сохранённых настроек, где оно уже прошло через toml
            cycle: |style| {
                let next = VARIANCE_STEPS
                    .iter()
                    .position(|step| (step - style.variance).abs() < 1e-3)
                    .map_or(0, |index| (index + 1) % VARIANCE_STEPS.len());
                style.variance = VARIANCE_STEPS[next];
            },
            text: |style| format!("{:.2}", style.variance),
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
/// Свой, а не общий `rows::next_in`: цвета сравниваются в линейном
/// пространстве, иначе тот же оттенок из настроек не нашёлся бы в палитре.
fn next_color(palette: &[Color], current: Color) -> Color {
    let index = palette
        .iter()
        .position(|color| color.to_linear() == current.to_linear())
        .map_or(0, |index| (index + 1) % palette.len());
    palette[index]
}

/// Строка-кнопка панели: ручка кита плюс квадрат-образец цвета у тех строк,
/// что цвет и правят. Свотч адресуется той же функцией-геттером, что и сам
/// цвет, — своего перечисления строк панели больше нет.
fn spawn_row(
    commands: &mut Commands,
    panel: Entity,
    swatch: Option<fn(&TreeStyle) -> Color>,
    label: &str,
    style: &TreeStyle,
    binding: CycleBinding<TreeStyle>,
) {
    let button = spawn_cycle_row(commands, panel, label, ROW_LEFT_PX, style, binding);
    // у нецветовых строк место под свотч остаётся (колонки не разъезжаются),
    // но и заливка, и рамка прозрачны — пустая рамка читалась как недоделка.
    // Третьим ребёнком, после значения: `with_child` дописывает в конец
    let color = swatch.map(|get| get(style));
    let border = if color.is_some() {
        Color::srgba(1., 1., 1., 0.35)
    } else {
        Color::NONE
    };
    let mut swatch_entity = commands.spawn((
        Node {
            width: px(14.),
            height: px(14.),
            border: UiRect::all(px(1.)),
            ..default()
        },
        BorderColor::all(border),
        BackgroundColor(color.unwrap_or(Color::NONE)),
    ));
    if let Some(get) = swatch {
        swatch_entity.insert(TreeStyleSwatch(get));
    }
    let swatch_entity = swatch_entity.id();
    commands.entity(button).add_child(swatch_entity);
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

/// Свотчи вслед за стилем (клик или правка по BRP); подписи строк ведёт кит.
fn sync_swatches(
    style: Res<TreeStyle>,
    mut swatches: Query<(&TreeStyleSwatch, &mut BackgroundColor)>,
) {
    for (swatch, mut background) in &mut swatches {
        background.0 = (swatch.0)(&style);
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
