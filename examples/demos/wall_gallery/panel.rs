//! Панель витрины: строки-ползунки на все ручки, значения констант стены под
//! ними — проёмы из `roof.wgsl` и пороги из `layers.rs` — и плашка с масштабом
//! внизу справа.
//!
//! Устройство повторяет `roof_gallery/panel.rs` слово в слово, и намеренно:
//! витрины читаются сверху вниз как самостоятельные примеры, а виджеты в обеих
//! — те же, что в панелях игры (`qwe::ui::knob`), иначе непонятно, чему в
//! картинке верить.
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
use bevy::window::PrimaryWindow;
use qwe::ui::knob::spawn_knob_specs;
use qwe::ui::{
    GROUP_HEADER_PAD_PX, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_font,
    row_label, row_value, spawn_group_header, spawn_panel_button, ui_node,
};

use crate::constants::{rule_constants, shader_constants};
use crate::params::{Tuning, specs};

/// Отступ строки-константы от краёв панели — `ROW_LEFT_PX` строк игры,
/// который сама она наружу не отдаёт.
const ROW_PAD_PX: f32 = 8.0;

/// Строка с масштабом: сколько метров в пикселе и какой рисунок на нём ещё жив.
#[derive(Component)]
pub(crate) struct ScaleReadout;

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

    spawn_knob_specs(&mut commands, panel, &*tuning, &specs());

    spawn_panel_button(
        &mut commands,
        panel,
        ResetButton,
        "Сброс",
        false,
        |_: On<Activate>, mut tuning: ResMut<Tuning>| *tuning = Tuning::default(),
    );

    spawn_group_header(&mut commands, panel, "Проёмы, roof.wgsl");
    for (name, value) in shader_constants() {
        spawn_constant_row(&mut commands, panel, name, value);
    }

    spawn_group_header(&mut commands, panel, "Пороги, layers.rs");
    for (name, value) in rule_constants() {
        spawn_constant_row(&mut commands, panel, name, value);
    }
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
/// клавишам, а измерение: правило гашения в `noise.wgsl` задано в пикселях, и
/// без числа на экране «окна пропали» не отличить от «фактура выключена».
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
/// отсюда деление на `scale_factor`.
///
/// Стена, в отличие от кровли, считается не в метрах, а в **ячейках**, и
/// пороги гашения приходятся на них: этаж это `STOREY_HEIGHT` метров, сжатых
/// подъёмом (`EXTRUDE_SCALE`), поэтому плашка печатает и метры, и экранный
/// размер этажа — по нему и видно, когда окну пора исчезнуть.
pub(crate) fn update_readout(
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut readout: Single<&mut Text, With<ScaleReadout>>,
) {
    let metres_per_pixel = camera.scale.x / window.scale_factor();
    // нарисованная высота этажа: настоящие три метра, сжатые подъёмом
    let storey_px = crate::drawn_storey() / metres_per_pixel;
    readout.0 = format!(
        "{metres_per_pixel:.3} м/пиксель  ·  этаж ≈ {storey_px:.1} пикселя  ·  \
         рисунок крупнее 4 ячеек на пиксель гаснет целиком",
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
