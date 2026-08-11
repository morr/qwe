//! Игровой UI (порт идиом из `zxc/src/ui`): панель скорости симуляции и
//! дебаг-тумблеры. Обычные `bevy_ui`-ноды + первопартийный
//! `bevy_ui_widgets::Button`.

mod brp;
mod buildings;
mod city;
mod debug;
mod hotkeys;
// `pub` по той же причине, что и `slider`: демо-сцена расталкивания ставит
// свои ручки тем же китом
pub mod knob;
mod navigation;
mod noise;
mod roads;
mod rows;
// `pub` ради демо-сцены расталкивания (`examples/demos/crowd_demo.rs`): ей
// нужна та же строка-ползунок, что и панелям игры, а весь `UiPlugin` она
// поднять не может — он тянет панели, карту и настройки
pub mod slider;
mod speed;
mod stats;
mod tree_rows;
mod trees;

use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

pub use self::brp::AgentBrpSession;
pub use self::debug::{DebugConiferNoise, DebugDoors, DebugGrid, DebugNavmesh};
use crate::loading::{AppState, PlayPhase};
use crate::map::osm::MapData;
use crate::map::trees::visible_count;
use crate::map::{TreeRowStyle, TreeStyle};

pub const UI_SCREEN_EDGE_PX_OFFSET: f32 = 8.0;

/// Место панели в правой колонке, снизу вверх: 0 — Tree rows у края экрана,
/// дальше Trees, Buildings, Roads, справка по хоткеям. Панели
/// абсолютные, `bevy_ui` их не стыкует, поэтому `bottom` каждой считает
/// [`stack_bottom_columns`] по **замеренным** высотам тех, что под ней: высота
/// панели Trees меняется на ходу (строки доли хвои и примеси появляются только
/// у формы `Mixed`), — прошитые константы высот такую колонку уронили бы
/// панелями друг на друга.
#[derive(Component)]
pub struct UiRightColumnSlot(pub u8);

/// То же для левой колонки: 0 — ряд дебаг-тумблеров у края экрана, 1 — панель
/// Noise над ним (целиком живёт при включённом дебаг-слое `noise`), 2 —
/// панель Navigation.
#[derive(Component)]
pub struct UiLeftColumnSlot(pub u8);

/// «Оставить зазор под этой панелью». Колонка по умолчанию стоит вплотную —
/// панели настроек карты читаются как один блок, и щели между ними лишние.
/// Зазор нужен там, где над блоком начинается **другой род** UI: ряд кнопок
/// под панелью Navigation, справка по хоткеям над панелями OSM. Зазор один на
/// всё и равен отступу от края экрана ([`UI_SCREEN_EDGE_PX_OFFSET`]).
#[derive(Component)]
pub struct UiPanelGapBelow;

/// Тип объектов мира, чьё число стоит в заголовке панели (см.
/// [`panel_header`]); компонент висит на тексте счётчика.
#[derive(Component, Clone, Copy)]
pub enum PanelCount {
    Trees,
    TreeRows,
    Buildings,
    Roads,
}

/// Заголовок панели: название и у правого края блока мелким шрифтом число
/// объектов этого типа в мире. Число пустое до загрузки карты — его заполняет
/// и дальше ведёт [`sync_panel_counts`].
pub fn panel_header(title: &str, count: PanelCount) -> impl Bundle {
    (
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Baseline,
            column_gap: px(6.),
            ..default()
        },
        children![
            (
                Text::new(title),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
                // распорка: заголовок забирает всю ширину, число прижимается
                // к правому краю блока
                Node {
                    flex_grow: 1.,
                    ..default()
                },
            ),
            (
                count,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(10.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            ),
        ],
    )
}

/// Счётчики в заголовках панелей: сколько объектов этого типа сейчас в мире.
/// Деревья считаются по собранному набору с учётом ползунка плотности и
/// тумблеров источников — то есть ровно столько крон и стоит на карте.
fn sync_panel_counts(
    map: Res<MapData>,
    style: Res<TreeStyle>,
    mut labels: Query<(&PanelCount, &mut Text)>,
) {
    for (count, mut text) in &mut labels {
        let total = match count {
            PanelCount::Trees => visible_count(&map.tree_appears_at, style.density),
            PanelCount::TreeRows => map.tree_rows.len(),
            PanelCount::Buildings => map.buildings.len(),
            PanelCount::Roads => map.roads.len(),
        };
        text.0 = total.to_string();
    }
}

/// Подсветка «кнопка активна» и осветление под курсором / при нажатии —
/// общие для тумблеров и панели городов.
pub const TOGGLE_ACTIVE_COLOR: Color = Color::srgba(0.16, 0.5, 0.2, 0.9);
pub const TOGGLE_HOVER_LIGHTEN: f32 = 0.12;
pub const TOGGLE_PRESSED_LIGHTEN: f32 = 0.24;

const UI_COLOR: Color = Color::srgb(0.094, 0.102, 0.11);

/// Тень под белым текстом панелей: фон у них полупрозрачный, и поверх светлой
/// карты буквы без тени сливаются с ней. Смещение в пиксель — дефолтные четыре
/// на 11–20 px шрифте читаются как вторая строка текста.
pub const UI_TEXT_SHADOW: TextShadow = TextShadow {
    offset: Vec2::splat(1.0),
    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
};

pub enum UiOpacity {
    Light,
    /// Фон панелей: сквозь `Light` просвечивали пешки и кроны, `Heavy` глушит
    /// карту под панелью.
    Medium,
    Heavy,
}

pub fn ui_color(opacity: UiOpacity) -> Color {
    UI_COLOR.with_alpha(match opacity {
        UiOpacity::Light => 0.25,
        UiOpacity::Medium => 0.55,
        UiOpacity::Heavy => 0.85,
    })
}

/// Кнопка панели: тёмный прямоугольник с подписью 12 px, подсвечиваемый по
/// наведению и нажатию. `marker` — компонент, по которому система подсветки
/// находит эту кнопку среди прочих (`DebugToggleButton`, `CityButton`, …).
///
/// `Hovered` кормит UI-picking-бэкенд, `Pressed` вставляет
/// `bevy_ui_widgets::Button` — для подсветки нужны оба.
pub fn spawn_panel_button<M>(
    commands: &mut Commands,
    parent: Entity,
    marker: impl Bundle,
    label: &str,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let button = commands
        .spawn((
            Button,
            marker,
            Pickable::default(),
            Hovered::default(),
            Node {
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
            children![(
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .observe(on_activate)
        .id();
    commands.entity(parent).add_child(button);
    button
}

/// «Курсор стоит в текстовом поле» — условие расписания для КАЖДОГО хоткея.
///
/// Без него набор seed'а разговаривает с игрой: пробел ставит симуляцию на
/// паузу, `-` и `=` крутят скорость, R перезапускает мир прямо посреди ввода.
/// Это тот же закон, что и «клик по панели не доходит до мира»
/// (`camera.rs::pointer_over_ui`), только для клавиатуры: событие, адресованное
/// UI, миру не принадлежит.
///
/// `Option` у фокуса: `InputFocus` ставит `bevy_ui_widgets`, и в тестовых
/// сборках без него условие должно отвечать «не печатаем», а не валить
/// валидацию параметров.
pub fn typing_in_text_input(
    focus: Option<Res<bevy::input_focus::InputFocus>>,
    fields: Query<(), With<bevy::text::EditableText>>,
) -> bool {
    focus
        .and_then(|focus| focus.get())
        .is_some_and(|entity| fields.contains(entity))
}

/// Корень игровой панели. Панели спавнятся в `Startup`, но поверх экрана
/// загрузки им делать нечего — до конца прогрева (`PlayPhase::Live`) они
/// скрыты (спавнятся с `Visibility::Hidden`).
#[derive(Component)]
pub struct GameUiRoot;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            speed::UiSpeedPlugin,
            stats::UiStatsPlugin,
            debug::UiDebugTogglesPlugin,
            trees::UiTreeStylePlugin,
            tree_rows::UiTreeRowStylePlugin,
            noise::UiConiferNoisePlugin,
            navigation::UiNavigationPlugin,
            buildings::UiBuildingStylePlugin,
            roads::UiRoadStylePlugin,
            city::UiCityPlugin,
            hotkeys::UiHotkeysPlugin,
            brp::UiBrpBadgePlugin,
        ))
        // бегунки всех панелей ведёт одна система — ползунки помечены общим
        // `slider::UiSlider`, строки — общим `rows::ValueRow`
        .add_systems(
            Update,
            (
                stack_bottom_columns,
                slider::sync_slider_thumbs,
                rows::highlight_value_rows,
            ),
        )
        .add_systems(
            Update,
            // `resource_changed` без `resource_exists` паникует до загрузки
            // карты, а `and_then` не вычисляет правую часть зря
            sync_panel_counts.run_if(
                resource_exists::<MapData>.and_then(
                    resource_changed::<MapData>
                        .or_else(resource_changed::<TreeStyle>)
                        .or_else(resource_changed::<TreeRowStyle>),
                ),
            ),
        )
        .add_systems(OnEnter(PlayPhase::Live), show_game_ui)
        // смена города возвращает приложение на экран загрузки — панели
        // прячутся до конца следующего прогрева
        .add_systems(OnExit(AppState::Playing), hide_game_ui);
    }
}

/// Отступ снизу у каждой панели обеих нижних колонок — сумма высот панелей под
/// ней (см. [`UiRightColumnSlot`] / [`UiLeftColumnSlot`]) плюс зазор за каждую
/// помеченную [`UiPanelGapBelow`] панель на её уровне и ниже.
///
/// `ComputedNode::size` — **физические** пиксели, а `Node::bottom` — логические:
/// без `inverse_scale_factor` на retina-экране каждая высота удваивалась и между
/// панелями зияла дыра в их собственный размер.
///
/// `bottom` пишется только когда он реально поехал: `Node` — не
/// `set_if_neq`-компонент, и безусловная запись метила бы его изменённым каждый
/// кадр, заставляя `bevy_ui` пересчитывать раскладку на пустом месте. Высота
/// читается из `ComputedNode`, то есть с прошлого кадра: панель, у которой
/// появилась строка, доезжает на кадр позже — заметить это можно только на
/// смене формы кроны.
fn stack_bottom_columns(
    mut right: Query<
        (
            &UiRightColumnSlot,
            &ComputedNode,
            &mut Node,
            Has<UiPanelGapBelow>,
        ),
        Without<UiLeftColumnSlot>,
    >,
    mut left: Query<
        (
            &UiLeftColumnSlot,
            &ComputedNode,
            &mut Node,
            Has<UiPanelGapBelow>,
        ),
        Without<UiRightColumnSlot>,
    >,
) {
    let mut right: Vec<_> = right
        .iter_mut()
        .map(|(slot, computed, node, gap)| (slot.0, computed, node, gap))
        .collect();
    stack_column(&mut right);
    let mut left: Vec<_> = left
        .iter_mut()
        .map(|(slot, computed, node, gap)| (slot.0, computed, node, gap))
        .collect();
    stack_column(&mut left);
}

fn stack_column(panels: &mut [(u8, &ComputedNode, Mut<Node>, bool)]) {
    // спрятанная панель (Noise при выключенном тумблере) не занимает места —
    // и не по `ComputedNode` с прошлого кадра, а по самому `Display`
    let visible: Vec<(u8, f32, bool)> = panels
        .iter()
        .filter(|(_, _, node, _)| node.display != Display::None)
        .map(|&(slot, computed, _, gap)| {
            (slot, computed.size.y * computed.inverse_scale_factor, gap)
        })
        .collect();

    for (slot, _, node, _) in panels.iter_mut() {
        let below: f32 = visible
            .iter()
            .filter(|&&(other, _, _)| other < *slot)
            .map(|&(_, height, _)| height)
            .sum();
        // зазор считается по панелям на своём уровне и ниже: помеченная
        // отодвигается от того, что под ней, вместе со всеми над ней
        let gaps = visible
            .iter()
            .filter(|&&(other, _, gap)| gap && other <= *slot)
            .count() as f32;
        let bottom = px(UI_SCREEN_EDGE_PX_OFFSET * (1. + gaps) + below);
        if node.bottom != bottom {
            node.bottom = bottom;
        }
    }
}

fn show_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Inherited;
    }
}

fn hide_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Hidden;
    }
}
