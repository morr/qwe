//! Панель витрины: строки-ползунки на ручки и плашка с масштабом внизу
//! справа.
//!
//! Виджеты — те же, что в панелях игры (`qwe::ui::slider`), а не свои: витрина
//! обязана выглядеть и вести себя как настоящая панель, иначе непонятно, чему
//! в ней верить. Устройство целиком повторяет `roof_gallery/panel.rs` — один
//! наблюдатель протяжки на все ручки, номер ручки на самом ползунке.
//!
//! **Шрифт панель ставит себе сама.** В игре его вешает `apply_panel_font` по
//! `Added<GameUiRoot>`, но эта система живёт в `UiPlugin`, которого здесь нет —
//! а без `InheritableFont` подписи достаются дефолтному шрифту bevy, где нет
//! кириллицы.

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::font_styles::InheritableFont;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::{Activate, ValueChange};
use bevy::window::PrimaryWindow;
use qwe::settings::CAR_MAX_ZOOM;
use qwe::ui::slider::{SliderRow, apply_step, retarget, spawn_slider_row};
use qwe::ui::{
    PANEL_FONT, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_block_background,
    panel_title, row_value, spawn_panel_button, ui_node,
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

/// Строка с масштабом: сколько метров в пикселе и жив ли на нём слой машин.
#[derive(Component)]
pub(crate) struct ScaleReadout;

#[derive(Component)]
pub(crate) struct ResetButton;

fn panel_font(assets: &AssetServer) -> InheritableFont {
    InheritableFont {
        font: assets.load(fonts::REGULAR),
        font_size: PANEL_FONT,
        weight: FontWeight::NORMAL,
    }
}

pub(crate) fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>, tuning: Res<Tuning>) {
    let panel = commands
        .spawn((
            ui_node(Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                width: px(PANEL_WIDTH_PX),
                flex_direction: FlexDirection::Column,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                padding: UiRect::all(px(GROUP_HEADER_PAD_PX)),
                ..default()
            }),
            panel_background(),
            panel_font(&assets),
            Name::new("gallery_panel"),
        ))
        .id();

    for (index, spec) in specs().into_iter().enumerate() {
        if let Some(group) = spec.group {
            spawn_group_header(&mut commands, panel, group);
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

/// Заголовок группы строк — плашка с названием, как секция в панели игры.
fn spawn_group_header(commands: &mut Commands, panel: Entity, title: &str) {
    commands.spawn((
        ui_node(Node {
            padding: UiRect::axes(px(GROUP_HEADER_PAD_PX), px(2)),
            ..default()
        }),
        panel_block_background(),
        children![panel_title(title)],
        ChildOf(panel),
    ));
}

/// Плашка с масштабом — внизу справа. Не подсказка, а измерение: в игре слой
/// машин снимается целиком за `CAR_MAX_ZOOM`, и без числа на экране «мелко, но
/// видно» не отличить от «в игре здесь уже пусто».
pub(crate) fn spawn_readout(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        ui_node(Node {
            position_type: PositionType::Absolute,
            bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
            right: px(UI_SCREEN_EDGE_PX_OFFSET),
            padding: UiRect::axes(px(10), px(6)),
            ..default()
        }),
        panel_background(),
        panel_font(&assets),
        Name::new("scale_readout"),
        children![(row_value(""), ScaleReadout)],
    ));
}

/// Метров на пиксель — по камере, а не по окну: масштаб проекции считается в
/// **логических** точках.
pub(crate) fn update_readout(
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut readout: Single<&mut Text, With<ScaleReadout>>,
) {
    let metres_per_pixel = camera.scale.x / window.scale_factor();
    let in_game = if metres_per_pixel < CAR_MAX_ZOOM {
        "слой машин в игре здесь рисуется"
    } else {
        "в игре на таком зуме слоя машин уже нет"
    };
    readout.0 = format!("{metres_per_pixel:.3} м/пиксель  ·  {in_game} (порог {CAR_MAX_ZOOM})");
}

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
        // ресурс правится только на смене шага: пересборка витрины стоит всех
        // клеток разом, и платить ею за каждый пиксель протяжки нельзя
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

/// Кнопка сброса подсвечивается, пока настройка отличается от игровой.
pub(crate) fn sync_reset_button(
    tuning: Res<Tuning>,
    mut button: Query<&mut ButtonVariant, With<ResetButton>>,
) {
    let changed = *tuning != Tuning::default();
    for mut variant in &mut button {
        *variant = qwe::ui::button_variant(changed);
    }
}
