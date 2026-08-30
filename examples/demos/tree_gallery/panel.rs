//! Панель витрины: строки-ползунки на все ручки генерации.
//!
//! Виджеты — те же, что в панелях игры (`qwe::ui::slider`), а не свои: витрина
//! обязана выглядеть и вести себя как настоящая панель, иначе непонятно, чему
//! в ней верить.
//!
//! Обработчик протяжки **один на все ручки**. Ползунок носит свой номер в
//! таблице [`specs`], по номеру достаётся `set` — и шестнадцать почти
//! одинаковых наблюдателей сворачиваются в один.
//!
//! **Шрифт панель ставит себе сама.** В игре его вешает `apply_panel_font` по
//! `Added<GameUiRoot>`, но эта система живёт в `UiPlugin`, которого здесь нет —
//! а без `InheritableFont` подписи достаются дефолтному шрифту bevy, где нет
//! кириллицы, и вся панель выходит квадратиками не того кегля.

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::font_styles::InheritableFont;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::{Activate, ValueChange};
use qwe::ui::slider::{SliderRow, apply_step, retarget, spawn_slider_row};
use qwe::ui::{
    PANEL_FONT, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_block_background,
    panel_title, spawn_panel_button, ui_node,
};

use crate::params::{ParamSpec, Tuning, specs};

/// Отступ заголовка группы от края плашки — как у заголовка секции в панели
/// настроек игры.
const GROUP_HEADER_PAD_PX: f32 = 6.0;

/// Номер ручки в [`specs`] — на ползунке и на его числе.
#[derive(Component, Clone, Copy)]
pub(crate) struct ParamSlider(usize);

#[derive(Component, Clone, Copy)]
pub(crate) struct ParamValue(usize);

pub(crate) fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>, tuning: Res<Tuning>) {
    let panel = commands
        .spawn((
            ui_node(Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                // до низа экрана: телу нужен потолок высоты, иначе шестнадцать
                // строк растут за край и прокручивать нечего
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                width: px(PANEL_WIDTH_PX),
                flex_direction: FlexDirection::Column,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                padding: UiRect::all(px(GROUP_HEADER_PAD_PX)),
                overflow: Overflow::scroll_y(),
                flex_shrink: 1.,
                min_height: px(0),
                ..default()
            }),
            panel_background(),
            // тот же шрифт, которым игра пишет свои панели: своей
            // `apply_panel_font` тут нет, см. шапку модуля
            InheritableFont {
                font: assets.load(fonts::REGULAR),
                font_size: PANEL_FONT,
                weight: FontWeight::NORMAL,
            },
            Name::new("gallery_panel"),
        ))
        .id();

    for (index, spec) in specs().into_iter().enumerate() {
        if let Some(group) = spec.group {
            commands.spawn((
                ui_node(Node {
                    padding: UiRect::axes(px(GROUP_HEADER_PAD_PX), px(2)),
                    ..default()
                }),
                panel_block_background(),
                children![panel_title(group)],
                ChildOf(panel),
            ));
        }
        let value = (spec.get)(&tuning);
        spawn_slider_row(
            &mut commands,
            panel,
            SliderRow {
                label: spec.label,
                value,
                value_text: (spec.format)(value),
                range: spec.range,
            },
            ParamValue(index),
            ParamSlider(index),
            on_param_change,
        );
    }

    spawn_panel_button(
        &mut commands,
        panel,
        ResetButton,
        "Сброс",
        false,
        |_: On<Activate>, mut tuning: ResMut<Tuning>| *tuning = Tuning::default(),
    );
}

#[derive(Component)]
pub(crate) struct ResetButton;

/// Протяжка любой ручки. Округляет до шага, возвращает бегунок на округлённое
/// место и пишет поле — какое именно, знает [`ParamSpec::set`] под номером,
/// который носит сам ползунок.
fn on_param_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    sliders: Query<&ParamSlider>,
    mut tuning: ResMut<Tuning>,
    mut values: Query<(&ParamValue, &mut Text)>,
) {
    let Ok(&ParamSlider(index)) = sliders.get(change.source) else {
        return;
    };
    let specs = specs();
    let spec = &specs[index];
    let stepped = apply_step(&change, &mut commands, spec.range);
    if (spec.get)(&tuning) == stepped {
        // ресурс правится только на смене шага: пересборка витрины стоит
        // тридцати шести крон, и платить ею за каждый пиксель протяжки нельзя
        return;
    }
    (spec.set)(&mut tuning, stepped);
    write_value(&mut values, index, spec, stepped);
}

/// Вернуть бегунки и числа под значение, пришедшее мимо них, — сейчас это
/// кнопка сброса.
pub(crate) fn sync_param_rows(
    mut commands: Commands,
    tuning: Res<Tuning>,
    sliders: Query<(Entity, &ParamSlider, &bevy::ui_widgets::SliderValue)>,
    mut values: Query<(&ParamValue, &mut Text)>,
) {
    let specs = specs();
    for (entity, &ParamSlider(index), current) in &sliders {
        let spec = &specs[index];
        let target = (spec.get)(&tuning);
        retarget(&mut commands, entity, current.0, target);
        write_value(&mut values, index, spec, target);
    }
}

fn write_value(
    values: &mut Query<(&ParamValue, &mut Text)>,
    index: usize,
    spec: &ParamSpec,
    value: f32,
) {
    for (&ParamValue(owner), mut text) in values.iter_mut() {
        if owner == index {
            text.0 = (spec.format)(value);
        }
    }
}

/// Кнопка сброса подсвечивается, пока настройка отличается от дефолта витрины —
/// это игра по геометрии, но `variance` = 0, см. `Tuning::default`.
pub(crate) fn sync_reset_button(
    tuning: Res<Tuning>,
    mut button: Query<&mut ButtonVariant, With<ResetButton>>,
) {
    let default = Tuning::default();
    let changed = specs()
        .iter()
        .any(|spec| (spec.get)(&tuning) != (spec.get)(&default));
    for mut variant in &mut button {
        *variant = qwe::ui::button_variant(changed);
    }
}
