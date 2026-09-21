//! Панель витрины: строки-ползунки на все ручки генерации.
//!
//! Виджеты — те же, что в панелях игры (`qwe::ui::knob`), а не свои: витрина
//! обязана выглядеть и вести себя как настоящая панель, иначе непонятно, чему
//! в ней верить.
//!
//! Строка тут и строка панели игры — один и тот же вызов `spawn_knob`, а за
//! протяжкой, округлением до шага и подписью стоит наблюдатель кита,
//! заведённый `add_knobs::<Tuning>()` по разу на ресурс.
//!
//! **Шрифт панель ставит себе сама.** В игре его вешает `apply_panel_font` по
//! `Added<GameUiRoot>`, но эта система живёт в `UiPlugin`, которого здесь нет —
//! а без `InheritableFont` подписи достаются дефолтному шрифту bevy, где нет
//! кириллицы, и вся панель выходит квадратиками не того кегля.

use bevy::feathers::controls::ButtonVariant;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use qwe::ui::knob::spawn_knob_specs;
use qwe::ui::{
    GROUP_HEADER_PAD_PX, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_font,
    spawn_panel_button, ui_node,
};

use crate::params::{Tuning, specs};

pub(crate) fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>, tuning: Res<Tuning>) {
    let panel = commands
        .spawn((
            // угол делится с меткой агентского запуска — плашка съезжает под неё
            qwe::ui::BelowBrpBadge,
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

#[derive(Component)]
pub(crate) struct ResetButton;

/// Кнопка сброса подсвечивается, пока настройка отличается от дефолта витрины —
/// это игра по геометрии, но `variance` = 0, см. `Tuning::default`.
///
/// Сравнение по ручкам, а не `!=` на весь [`Tuning`], как у соседних витрин:
/// `CrownParams` — не `PartialEq`, а всё, что видно на панели, ручки и так
/// покрывают.
pub(crate) fn sync_reset_button(
    tuning: Res<Tuning>,
    mut button: Query<&mut ButtonVariant, With<ResetButton>>,
) {
    let default = Tuning::default();
    let changed = specs()
        .iter()
        .any(|spec| (spec.binding.get)(&tuning) != (spec.binding.get)(&default));
    for mut variant in &mut button {
        *variant = qwe::ui::button_variant(changed);
    }
}
