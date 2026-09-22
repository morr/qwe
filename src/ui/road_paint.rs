//! Секция Road paint: всё, что лежит **краской** поверх асфальта, — линии
//! полос, зебры и стоп-линии узлов, стрелки на полосах, — и износ асфальта.
//!
//! Четыре тумблера — поля `RoadStyle`: каждый снимает или кладёт свой слой
//! краски, и правка пересобирает дорожные слои (`map::roads::rebuild_roads`).
//! Три ползунка — `RoadPaintStyle`: свежесть краски (Paint), колея асфальта
//! (Wear) и колея траекторий узла (Turn wear). Все три — юниформы
//! материалов, протяжка не пересобирает ничего, поэтому ресурс свой, а не поля
//! `RoadStyle`.
//!
//! Отдельной секцией, а не хвостом Roads: с ручками формы (этап 8 плана
//! переработки дорог) секция Roads выросла бы до полутора десятков строк, а
//! краска и форма настраиваются порознь — одна по картинке узла, другая по
//! картинке улицы.

use bevy::prelude::*;

use crate::map::{
    CrossingMode, PAINT_MAX, PAINT_MIN, PAINT_STEP, RoadPaintStyle, RoadStyle, TURN_WEAR_MAX,
    TURN_WEAR_MIN, TURN_WEAR_STEP, WEAR_MAX, WEAR_MIN, WEAR_STEP,
};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{UiBuildSet, panel_title};

pub struct UiRoadPaintPlugin;

impl Plugin for UiRoadPaintPlugin {
    fn build(&self, app: &mut App) {
        // `add_knobs` идемпотентен: `RoadStyle` вяжет и секция Roads
        app.add_knobs::<RoadStyle>()
            .add_knobs::<RoadPaintStyle>()
            .add_systems(
                Startup,
                build_road_paint_section.in_set(UiBuildSet::Sections),
            );
    }
}

fn build_road_paint_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    style: Res<RoadStyle>,
    paint: Res<RoadPaintStyle>,
) {
    let [paint_knob, wear_knob, turn_wear_knob] = paint_knobs();
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::RoadPaint,
        panel_title("Road paint"),
        "road_paint_section",
    );

    spawn_cycle_row(
        &mut commands,
        panel,
        "Markings",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.markings = !style.markings,
            text: |style| on_off(style.markings).to_string(),
        },
    );
    spawn_knob(&mut commands, panel, paint_knob.0, &*paint, paint_knob.1);
    // краска узла (`map::roads::node_paint`): зебры и стоп-линии на плечах
    spawn_cycle_row(
        &mut commands,
        panel,
        "Crossings",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.crossings = next_in(&CrossingMode::ALL, style.crossings),
            text: |style| style.crossings.label().to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Stop lines",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.stop_lines = !style.stop_lines,
            text: |style| on_off(style.stop_lines).to_string(),
        },
    );
    // стрелки на полосах подходов (`map::roads::turns`)
    spawn_cycle_row(
        &mut commands,
        panel,
        "Arrows",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.arrows = !style.arrows,
            text: |style| on_off(style.arrows).to_string(),
        },
    );
    // колея — юниформы, протяжка ничего не пересобирает
    spawn_knob(&mut commands, panel, wear_knob.0, &*paint, wear_knob.1);
    // колея траекторий узла (`map::roads::turns`) — тоже юниформ
    spawn_knob(
        &mut commands,
        panel,
        turn_wear_knob.0,
        &*paint,
        turn_wear_knob.1,
    );
}

/// Ползунки краски — подпись и привязка к полю `RoadPaintStyle`: Paint, Wear,
/// Turn wear. Одна таблица на панель игры и панель витрины `roads`, как
/// `shape_knobs`: шкалы у двух панелей не должны разойтись. Порядок — порядок
/// строк; между ними стоят тумблеры `RoadStyle`, поэтому вызывающий
/// разбирает массив, а не обходит его циклом.
pub fn paint_knobs() -> [(&'static str, SliderBinding<RoadPaintStyle>); 3] {
    [
        (
            "Paint",
            SliderBinding {
                get: |paint| paint.paint,
                set: |paint, value| paint.paint = value,
                range: (PAINT_MIN, PAINT_MAX, PAINT_STEP),
                text: |value| format!("{:.0}%", value * 100.),
            },
        ),
        (
            "Wear",
            SliderBinding {
                get: |paint| paint.wear,
                set: |paint, value| paint.wear = value,
                range: (WEAR_MIN, WEAR_MAX, WEAR_STEP),
                text: |value| format!("{:.1}%", value * 100.),
            },
        ),
        (
            "Turn wear",
            SliderBinding {
                get: |paint| paint.turn_wear,
                set: |paint, value| paint.turn_wear = value,
                range: (TURN_WEAR_MIN, TURN_WEAR_MAX, TURN_WEAR_STEP),
                text: |value| format!("{:.1}%", value * 100.),
            },
        ),
    ]
}
