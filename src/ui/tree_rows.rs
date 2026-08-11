//! Панель стиля аллей (`natural=tree_row`) — отделена от панели Trees так же,
//! как Buildings: тумблер аллей целиком, состав посадки (политика размещения,
//! источник шага) и три ручки зелёной подложки. Каждая строка — кнопка,
//! листающая значение по кругу; правка `TreeRowStyle` пересобирает набор
//! деревьев и подложку (`map::mod` — та же цепочка, что у `TreeStyle`).

use bevy::prelude::*;

use crate::map::{RoadJoin, RoadSmoothing, TreeRowPlacement, TreeRowStyle};
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::{
    GameUiRoot, PanelCount, UI_SCREEN_EDGE_PX_OFFSET, UiOpacity, UiRightColumnSlot, panel_header,
    ui_color,
};

pub struct UiTreeRowStylePlugin;

impl Plugin for UiTreeRowStylePlugin {
    fn build(&self, app: &mut App) {
        // подписи вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<TreeRowStyle>()
            .add_systems(Startup, render_tree_row_style_panel);
    }
}

fn render_tree_row_style_panel(mut commands: Commands, style: Res<TreeRowStyle>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(210.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            UiRightColumnSlot(0),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("tree_row_style_panel"),
            children![panel_header("Tree rows", PanelCount::TreeRows)],
        ))
        .id();

    spawn_cycle_row(
        &mut commands,
        panel,
        "Rows",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.enabled = !style.enabled,
            text: |style| on_off(style.enabled).to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Placement",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.placement = next_in(&TreeRowPlacement::ALL, style.placement),
            text: |style| style.placement.label().to_string(),
        },
    );
    // откуда берётся шаг посадки ряда. `OSM` — из тегов `spacing`/`count`, и
    // такой ряд ползунок плотности не трогает; `slider` — теги игнорируются, и
    // ряд подчиняется ползунку наравне с лесом
    spawn_cycle_row(
        &mut commands,
        panel,
        "Spacing",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.osm_spacing = !style.osm_spacing,
            text: |style| (if style.osm_spacing { "OSM" } else { "slider" }).to_string(),
        },
    );
    // те же три ручки, что у панели Roads, но **свои**: ломаная аллеи и ломаная
    // улицы приходят из разных данных, и подложка обязана выглядеть лесом даже
    // там, где дороги оставлены нетронутыми
    spawn_cycle_row(
        &mut commands,
        panel,
        "Joins",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.join = next_in(&RoadJoin::ALL, style.join),
            text: |style| style.join.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Smoothing",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.smoothing = next_in(&RoadSmoothing::ALL, style.smoothing),
            text: |style| style.smoothing.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Casing",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.casing = !style.casing,
            text: |style| on_off(style.casing).to_string(),
        },
    );
}
