//! Строка-ползунок для панелей правой колонки — общий кит для Trees и Noise.
//! Полей ввода в `bevy_ui` нет, поэтому числовые ручки — ползунки
//! `bevy_ui_widgets::Slider` с дискретным шагом (см. [`quantize`]).

use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::ui_widgets::{
    Slider, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick, ValueChange,
};

use bevy::prelude::*;

use crate::ui::{UiOpacity, ui_color};

/// Дорожка и бегунок; высота бегунка задаёт и высоту строки.
const SLIDER_HEIGHT_PX: f32 = 12.0;
const SLIDER_TRACK_PX: f32 = 4.0;
const SLIDER_TRACK_COLOR: Color = Color::srgba(1., 1., 1., 0.18);
const SLIDER_THUMB_COLOR: Color = Color::srgba(1., 1., 1., 0.75);
const SLIDER_THUMB_HOVER_COLOR: Color = Color::WHITE;

/// Ползунок, поставленный [`spawn_slider_row`], — по нему [`sync_slider_thumbs`]
/// находит все ползунки всех панелей.
#[derive(Component)]
pub struct UiSlider;

/// Бегунок такого ползунка.
#[derive(Component)]
pub struct UiSliderThumb;

/// Что показывает строка-ползунок: подпись, стартовое значение с его текстом
/// и диапазон с шагом.
pub struct SliderRow<'a> {
    pub label: &'a str,
    pub value: f32,
    pub value_text: String,
    /// `(min, max, step)` — как у констант `settings.rs`.
    pub range: (f32, f32, f32),
}

/// Строка-ползунок: подпись со значением и под ней сам ползунок. Возвращает
/// блок строки — на него навешивается маркер, если строку кто-то адресует
/// (`MixedOnlyRow` управляет видимостью строк панели Trees).
///
/// `value_label_marker` встаёт на текст значения, `slider_marker` — на ползунок:
/// по ним панель-хозяйка актуализирует подпись и бегунок после правки ресурса
/// извне (BRP, сохранённые настройки).
pub fn spawn_slider_row<M>(
    commands: &mut Commands,
    panel: Entity,
    row: SliderRow,
    value_label_marker: impl Bundle,
    slider_marker: impl Bundle,
    on_change: impl IntoObserverSystem<ValueChange<f32>, (), M>,
) -> Entity {
    let SliderRow {
        label,
        value,
        value_text,
        range: (min, max, step),
    } = row;
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
            BackgroundColor(ui_color(UiOpacity::Heavy)),
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
                        value_label_marker,
                        Text::new(value_text),
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
            UiSlider,
            slider_marker,
            Slider {
                // клик по дорожке ставит бегунок туда, куда ткнули
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderValue(value),
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
                        UiSliderThumb,
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

/// Дискретный шаг ползунка: значение с драга округляется до шага, наблюдатели
/// правят ресурс только когда шаг действительно сменился — иначе каждый
/// пиксель протяжки запускал бы пересборку.
pub fn quantize(value: f32, min: f32, max: f32, step: f32) -> f32 {
    ((value / step).round() * step).clamp(min, max)
}

/// Позиция бегунка по значению плюс подсветка под курсором и при протяжке.
pub fn sync_slider_thumbs(
    sliders: Query<
        (Entity, &SliderValue, &SliderRange, &Hovered),
        (With<UiSlider>, Or<(Changed<SliderValue>, Changed<Hovered>)>),
    >,
    children: Query<&Children>,
    mut thumbs: Query<(&mut Node, &mut BackgroundColor), With<UiSliderThumb>>,
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
