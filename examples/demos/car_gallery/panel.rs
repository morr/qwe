//! Панель витрины: строки-ползунки на ручки и плашка с масштабом внизу
//! справа.
//!
//! Виджеты — те же, что в панелях игры (`qwe::ui::knob`), а не свои: витрина
//! обязана выглядеть и вести себя как настоящая панель, иначе непонятно, чему
//! в ней верить. Строка тут и строка панели игры — один и тот же вызов
//! `spawn_knob`, а за протяжкой, округлением до шага и подписью стоит
//! наблюдатель кита, заведённый `add_knobs::<Tuning>()` по разу на ресурс.
//!
//! **Шрифт панель ставит себе сама.** В игре его вешает `apply_panel_font` по
//! `Added<GameUiRoot>`, но эта система живёт в `UiPlugin`, которого здесь нет —
//! а без `InheritableFont` подписи достаются дефолтному шрифту bevy, где нет
//! кириллицы.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use bevy::window::PrimaryWindow;
use qwe::settings::CAR_MAX_ZOOM;
use qwe::ui::knob::spawn_knob_specs;
use qwe::ui::{
    GROUP_HEADER_PAD_PX, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_font,
    row_value, spawn_panel_button, ui_node,
};

use crate::params::{Tuning, specs};

/// Строка с масштабом: сколько метров в пикселе и жив ли на нём слой машин.
#[derive(Component)]
pub(crate) struct ScaleReadout;

#[derive(Component)]
pub(crate) struct ResetButton;

pub(crate) fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>, tuning: Res<Tuning>) {
    let panel = commands
        .spawn((
            // угол делится с меткой агентского запуска — плашка съезжает под неё
            qwe::ui::BelowBrpBadge,
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

    spawn_knob_specs(&mut commands, panel, &*tuning, &specs());

    spawn_panel_button(
        &mut commands,
        panel,
        ResetButton,
        "Сброс",
        false,
        |_: On<Activate>, mut tuning: ResMut<Tuning>| *tuning = Tuning::default(),
    );
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
