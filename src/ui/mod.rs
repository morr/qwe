//! Игровой UI (порт идиом из `zxc/src/ui`): панель скорости симуляции и
//! дебаг-тумблеры. Обычные `bevy_ui`-ноды + первопартийный
//! `bevy_ui_widgets::Button`.

mod buildings;
mod city;
mod debug;
mod hotkeys;
mod roads;
mod speed;
mod tram;
mod tree_rows;
mod trees;

use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

pub use self::debug::{DebugConiferNoise, DebugDoors, DebugGrid, DebugNavmesh};
use crate::loading::{AppState, PlayPhase};
use crate::map::osm::{MapData, RailKind};
use crate::map::trees::visible_count;
use crate::map::{TreeRowStyle, TreeStyle};

pub const UI_SCREEN_EDGE_PX_OFFSET: f32 = 8.0;

/// Место панели в правой колонке, снизу вверх: 0 — Tree rows у края экрана,
/// дальше Trees, Buildings, Roads, Tram, справка по хоткеям. Панели абсолютные,
/// `bevy_ui` их не стыкует, поэтому `bottom` каждой считает
/// [`stack_right_column`] по **замеренным** высотам тех, что под ней: высота
/// панели Trees меняется на ходу (строка доли хвои появляется только у формы
/// `Mixed`), и прошитые константы высот такую панель уронили бы под соседнюю.
#[derive(Component)]
pub struct UiRightColumnSlot(pub u8);

/// Тип объектов мира, чьё число стоит в заголовке панели (см.
/// [`panel_header`]); компонент висит на тексте счётчика.
#[derive(Component, Clone, Copy)]
pub enum PanelCount {
    Trees,
    TreeRows,
    Buildings,
    Roads,
    Trams,
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
            PanelCount::Trams => map
                .rails
                .iter()
                .filter(|rail| rail.kind == RailKind::Tram)
                .count(),
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
            debug::UiDebugTogglesPlugin,
            trees::UiTreeStylePlugin,
            tree_rows::UiTreeRowStylePlugin,
            buildings::UiBuildingStylePlugin,
            roads::UiRoadStylePlugin,
            tram::UiTramStylePlugin,
            city::UiCityPlugin,
            hotkeys::UiHotkeysPlugin,
        ))
        .add_systems(Update, stack_right_column)
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

/// Отступ снизу у каждой панели правой колонки — сумма высот панелей под ней
/// (см. [`UiRightColumnSlot`]). Панели стоят вплотную, без зазора.
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
fn stack_right_column(mut panels: Query<(&UiRightColumnSlot, &ComputedNode, &mut Node)>) {
    let mut heights: Vec<(u8, f32)> = panels
        .iter()
        .map(|(slot, computed, _)| (slot.0, computed.size.y * computed.inverse_scale_factor))
        .collect();
    heights.sort_unstable_by_key(|&(slot, _)| slot);

    for (slot, _, mut node) in &mut panels {
        let below: f32 = heights
            .iter()
            .filter(|&&(other, _)| other < slot.0)
            .map(|&(_, height)| height)
            .sum();
        let bottom = px(UI_SCREEN_EDGE_PX_OFFSET + below);
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
