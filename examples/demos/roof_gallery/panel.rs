//! Панель витрины: строки-ползунки на все ручки, значения констант кровель
//! под ними — фактуры из `roof.wgsl` и формы из `roofs.rs` — и плашка с
//! масштабом внизу справа.
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

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::font_styles::InheritableFont;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::Activate;
use bevy::window::PrimaryWindow;
use qwe::ui::knob::spawn_knob;
use qwe::ui::{
    PANEL_FONT, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_block_background,
    panel_title, row_label, row_value, spawn_panel_button, ui_node,
};

use crate::constants::{shader_constants, shape_constants};
use crate::params::{Tuning, specs};

/// Отступ заголовка группы от края плашки — как у заголовка секции в панели
/// настроек игры.
const GROUP_HEADER_PAD_PX: f32 = 6.0;

/// Отступ строки-константы от краёв панели — `ROW_LEFT_PX` строк игры,
/// который сама она наружу не отдаёт.
const ROW_PAD_PX: f32 = 8.0;

/// Строка с масштабом: сколько метров в пикселе и какая фактура на нём ещё
/// жива.
#[derive(Component)]
pub(crate) struct ScaleReadout;

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
                // до низа экрана: телу нужен потолок высоты, иначе строки
                // растут за край и прокручивать нечего
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

    for spec in specs() {
        if let Some(group) = spec.group {
            spawn_group_header(&mut commands, panel, group);
        }
        spawn_knob(&mut commands, panel, spec.label, &*tuning, spec.binding);
    }

    spawn_panel_button(
        &mut commands,
        panel,
        ResetButton,
        "Сброс",
        false,
        |_: On<Activate>, mut tuning: ResMut<Tuning>| *tuning = Tuning::default(),
    );

    spawn_group_header(&mut commands, panel, "Константы roof.wgsl");
    for (name, value) in shader_constants() {
        spawn_constant_row(&mut commands, panel, name, value);
    }

    spawn_group_header(&mut commands, panel, "Константы roofs.rs");
    for (name, value) in shape_constants() {
        spawn_constant_row(&mut commands, panel, name, value);
    }
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

/// Константа шейдера: имя слева, значение справа — та же геометрия, что у
/// строки-значения игры (`ui/rows.rs`), но **не кнопка**: крутить константу
/// отсюда нельзя, а подсвечивать под курсором строку, клик по которой ничего
/// не сделает, панель игры себе не позволяет.
fn spawn_constant_row(commands: &mut Commands, panel: Entity, name: &str, value: &str) {
    commands.spawn((
        ui_node(Node {
            column_gap: px(6),
            padding: UiRect::axes(px(ROW_PAD_PX), px(1)),
            ..default()
        }),
        children![
            (
                row_label(name),
                // распорка: имя забирает свободную ширину, значение уходит
                // к правому краю строки
                Node {
                    flex_grow: 1.,
                    ..default()
                },
            ),
            row_value(value),
        ],
        ChildOf(panel),
    ));
}

/// Плашка с масштабом — внизу справа, подальше от панели. Не подсказка по
/// клавишам, а измерение: правило гашения октав в `noise.wgsl` задано в
/// пикселях, и без числа на экране «фактура пропала» не отличить от «фактура
/// выключена».
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
/// **логических** точках, а `fwidth` в шейдере меряет физический пиксель,
/// отсюда деление на `scale_factor`. Всё, что короче полутора пикселей,
/// шейдер гасит полностью, всё, что длиннее четырёх, показывает целиком
/// (`visible` в `noise.wgsl`) — эти два числа плашка и печатает.
pub(crate) fn update_readout(
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut readout: Single<&mut Text, With<ScaleReadout>>,
) {
    let metres_per_pixel = camera.scale.x / window.scale_factor();
    readout.0 = format!(
        "{metres_per_pixel:.3} м/пиксель  ·  фактура крупнее {:.2} м видна целиком, мельче {:.2} м погашена",
        metres_per_pixel * 4.0,
        metres_per_pixel * 1.5,
    );
}

#[derive(Component)]
pub(crate) struct ResetButton;

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
