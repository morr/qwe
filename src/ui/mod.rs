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

/// Место панели в правой колонке — **порядок объявления снизу вверх**, от
/// края экрана.
///
/// Перечислением, а не числом: слот прописывается в файле своей панели, и
/// номерами два соседних файла легко ставили одну панель поверх другой или
/// оставляли дыру в колонке. Порядок здесь читается целиком в одном месте, а
/// коллизия и пропуск стали невыразимы.
///
/// Панели абсолютные, `bevy_ui` их не стыкует, поэтому `bottom` каждой считает
/// [`stack_bottom_columns`] по **замеренным** высотам тех, что под ней: высота
/// панели Trees меняется на ходу (строки доли хвои и примеси появляются только
/// у формы `Mixed`), — прошитые константы высот такую колонку уронили бы
/// панелями друг на друга.
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum UiRightColumn {
    TreeRows,
    Trees,
    Buildings,
    Roads,
    /// Справка по хоткеям — не настройка карты, поэтому с зазором под собой.
    Hotkeys,
}

/// То же для левой колонки, снизу вверх.
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum UiLeftColumn {
    /// Ряд дебаг-тумблеров у края экрана.
    DebugToggles,
    /// Панель Noise — целиком живёт при включённом дебаг-слое `noise`.
    Noise,
    Navigation,
}

/// Узел панели настроек в колонке: абсолютная, у своего края экрана, колонка
/// строк в 210 px. `bottom` — заглушка на один кадр, его перестыкует
/// [`stack_bottom_columns`] по замеренным высотам.
///
/// Ряд дебаг-тумблеров и справка по хоткеям сюда не входят: у них своя форма
/// (строка вместо колонки, свои отступы) — общий узел пришлось бы
/// параметризовать ровно тем, чем они и отличаются.
fn panel_node(left: Val, right: Val) -> Node {
    Node {
        position_type: PositionType::Absolute,
        bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
        left,
        right,
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: px(4.),
        padding: UiRect::all(px(10.)),
        width: px(210.),
        ..default()
    }
}

/// Панель настроек правой колонки: её узел и её место в колонке.
pub fn right_panel(slot: UiRightColumn) -> (Node, UiRightColumn) {
    (panel_node(Val::Auto, px(UI_SCREEN_EDGE_PX_OFFSET)), slot)
}

/// Панель настроек левой колонки: её узел и её место в колонке.
pub fn left_panel(slot: UiLeftColumn) -> (Node, UiLeftColumn) {
    (panel_node(px(UI_SCREEN_EDGE_PX_OFFSET), Val::Auto), slot)
}

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
/// ней (см. [`UiRightColumn`] / [`UiLeftColumn`]) плюс зазор за каждую
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
            &UiRightColumn,
            &ComputedNode,
            &mut Node,
            Has<UiPanelGapBelow>,
        ),
        Without<UiLeftColumn>,
    >,
    mut left: Query<
        (
            &UiLeftColumn,
            &ComputedNode,
            &mut Node,
            Has<UiPanelGapBelow>,
        ),
        Without<UiRightColumn>,
    >,
) {
    stack_column(
        right
            .iter_mut()
            .map(|(slot, computed, node, gap)| (*slot as u8, computed, node, gap)),
    );
    stack_column(
        left.iter_mut()
            .map(|(slot, computed, node, gap)| (*slot as u8, computed, node, gap)),
    );
}

fn stack_column<'a>(panels: impl Iterator<Item = (u8, &'a ComputedNode, Mut<'a, Node>, bool)>) {
    let mut panels: Vec<_> = panels.collect();
    // спрятанная панель (Noise при выключенном тумблере) не занимает места —
    // и не по `ComputedNode` с прошлого кадра, а по самому `Display`
    let visible: Vec<Stacked> = panels
        .iter()
        .filter(|(_, _, node, _)| node.display != Display::None)
        .map(|&(slot, computed, _, gap)| Stacked {
            slot,
            height: computed.size.y * computed.inverse_scale_factor,
            gap_below: gap,
        })
        .collect();

    for (slot, _, node, _) in panels.iter_mut() {
        let bottom = px(column_bottom(*slot, &visible));
        if node.bottom != bottom {
            node.bottom = bottom;
        }
    }
}

/// Видимая панель колонки глазами раскладки: только то, из чего считается чужой
/// отступ.
#[derive(Clone, Copy, Debug)]
struct Stacked {
    slot: u8,
    /// Логические пиксели — см. `inverse_scale_factor` у [`stack_bottom_columns`].
    height: f32,
    gap_below: bool,
}

/// Отступ снизу у панели с местом `slot`: отступ от края экрана, высоты видимых
/// панелей под ней и по зазору за каждую помеченную на её уровне и ниже.
///
/// Отдельной функцией от `Mut<Node>`: арифметика колонки — единственное, что
/// здесь можно перепутать, и проверять её через целое приложение с честной
/// раскладкой `bevy_ui` пришлось бы ради трёх сложений.
fn column_bottom(slot: u8, visible: &[Stacked]) -> f32 {
    let below: f32 = visible
        .iter()
        .filter(|other| other.slot < slot)
        .map(|other| other.height)
        .sum();
    // зазор считается по панелям на своём уровне и ниже: помеченная
    // отодвигается от того, что под ней, вместе со всеми над ней
    let gaps = visible
        .iter()
        .filter(|other| other.gap_below && other.slot <= slot)
        .count() as f32;
    UI_SCREEN_EDGE_PX_OFFSET * (1. + gaps) + below
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

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(slot: u8, height: f32) -> Stacked {
        Stacked {
            slot,
            height,
            gap_below: false,
        }
    }

    #[test]
    fn the_bottom_panel_sits_at_the_screen_edge() {
        let column = [panel(0, 100.), panel(1, 40.)];
        assert_eq!(column_bottom(0, &column), UI_SCREEN_EDGE_PX_OFFSET);
    }

    #[test]
    fn a_panel_clears_everything_below_it() {
        let column = [panel(0, 100.), panel(1, 40.), panel(2, 30.)];
        assert_eq!(column_bottom(2, &column), UI_SCREEN_EDGE_PX_OFFSET + 140.);
    }

    /// Панель Noise при выключенном тумблере не попадает в `visible` — и всё,
    /// что над ней, съезжает вниз на её место, а не висит над дырой.
    #[test]
    fn a_hidden_panel_leaves_no_hole() {
        let column = [panel(0, 100.), panel(2, 30.)];
        assert_eq!(column_bottom(2, &column), UI_SCREEN_EDGE_PX_OFFSET + 100.);
    }

    /// Зазор помеченной панели достаётся всем над ней, а не ей самой: он
    /// отделяет её от того, что под ней.
    #[test]
    fn a_marked_panel_pushes_itself_and_everything_above_it_up() {
        let column = [
            panel(0, 100.),
            Stacked {
                slot: 1,
                height: 40.,
                gap_below: true,
            },
            panel(2, 30.),
        ];
        assert_eq!(column_bottom(0, &column), UI_SCREEN_EDGE_PX_OFFSET);
        assert_eq!(
            column_bottom(1, &column),
            UI_SCREEN_EDGE_PX_OFFSET * 2. + 100.
        );
        assert_eq!(
            column_bottom(2, &column),
            UI_SCREEN_EDGE_PX_OFFSET * 2. + 140.
        );
    }
}
