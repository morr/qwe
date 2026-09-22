//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант,
//! тротуары, разметка, зебры и стоп-линии узлов. Полей ввода в `bevy_ui` нет,
//! поэтому каждая строка — кнопка, листающая значение по кругу (как у панели
//! деревьев); правка `RoadStyle` пересобирает дорожные слои
//! (`map::roads::rebuild_roads`).
//!
//! Под разметкой — два ползунка `RoadPaintStyle`: свежесть краски (Paint) и
//! колея асфальта (Wear). Оба — юниформы материалов, протяжка не пересобирает
//! ничего, поэтому ресурс свой, а не поля `RoadStyle`.
//!
//! Последние строки секции — трамвай (`TramStyle`) и машины (`CarStyle`,
//! тумблер плюс ползунок занятости): у каждого свой ресурс со своей
//! пересборкой (`map::tram::rebuild_tram`, `map::cars::rebuild_cars`). И путь,
//! и припаркованный ряд стоят на той же проезжей части, так что читаются они
//! вместе с дорогами, а не отдельными секциями на одну-две строки.
//!
//! Ползунок занятости виден, только пока включён слой машин: при `Cars = Off`
//! ряда на карте нет, и протяжка ползунка гоняла бы пересборку слоя вхолостую
//! (`rebuild_cars` деспавнит и выходит) и писала бы настройки — то же правило,
//! что прячет ручки Separation в Nav и ручки поля хвои в Noise.

use bevy::prelude::*;

use crate::map::cars::{CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_STEP};
use crate::map::{
    CarStyle, CrossingMode, PAINT_MAX, PAINT_MIN, PAINT_STEP, RoadJoin, RoadPaintStyle, RoadStyle,
    Smoothing, TramStyle, WEAR_MAX, WEAR_MIN, WEAR_STEP,
};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, next_in, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{PanelCount, UiBuildSet, panel_header};

/// Строка ползунка занятости — по ней видимость следует за тумблером `Cars`.
#[derive(Component)]
struct CarOccupancyRow;

pub struct UiRoadStylePlugin;

impl Plugin for UiRoadStylePlugin {
    fn build(&self, app: &mut App) {
        // подписи вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<RoadStyle>()
            .add_knobs::<RoadPaintStyle>()
            .add_knobs::<TramStyle>()
            .add_knobs::<CarStyle>()
            .add_systems(Startup, build_roads_section.in_set(UiBuildSet::Sections))
            .add_systems(
                Update,
                // без run_if: одна query и сравнение — дешевле, чем следить за
                // окном `resource_changed` на первом кадре
                sync_occupancy_row_visibility,
            );
    }
}

fn build_roads_section(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    style: Res<RoadStyle>,
    paint: Res<RoadPaintStyle>,
    tram: Res<TramStyle>,
    cars: Res<CarStyle>,
) {
    let panel = spawn_section(
        &mut commands,
        panes.pane(SettingsTab::Map),
        SectionSlot::Roads,
        panel_header("Roads", PanelCount::Roads),
        "roads_section",
    );

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
            cycle: |style| style.smoothing = next_in(&Smoothing::ALL, style.smoothing),
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
    spawn_cycle_row(
        &mut commands,
        panel,
        "Sidewalks",
        ROW_LEFT_PX,
        &*style,
        CycleBinding {
            cycle: |style| style.sidewalks = !style.sidewalks,
            text: |style| on_off(style.sidewalks).to_string(),
        },
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
    // краска и колея — юниформы, протяжка ничего не пересобирает
    spawn_knob(
        &mut commands,
        panel,
        "Paint",
        &*paint,
        SliderBinding {
            get: |paint| paint.paint,
            set: |paint, value| paint.paint = value,
            range: (PAINT_MIN, PAINT_MAX, PAINT_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
    spawn_knob(
        &mut commands,
        panel,
        "Wear",
        &*paint,
        SliderBinding {
            get: |paint| paint.wear,
            set: |paint, value| paint.wear = value,
            range: (WEAR_MIN, WEAR_MAX, WEAR_STEP),
            text: |value| format!("{:.1}%", value * 100.),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Tram",
        ROW_LEFT_PX,
        &*tram,
        CycleBinding {
            cycle: |tram| tram.visible = !tram.visible,
            text: |tram| on_off(tram.visible).to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        panel,
        "Cars",
        ROW_LEFT_PX,
        &*cars,
        CycleBinding {
            cycle: |cars| cars.visible = !cars.visible,
            text: |cars| on_off(cars.visible).to_string(),
        },
    );
    let occupancy = spawn_knob(
        &mut commands,
        panel,
        "Occupancy",
        &*cars,
        SliderBinding {
            get: |cars| cars.occupancy,
            set: |cars, value| cars.occupancy = value,
            range: (CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_STEP),
            text: |value| format!("{:.0}%", value * 100.),
        },
    );
    commands.entity(occupancy).insert(CarOccupancyRow);
}

/// Занятость живёт при включённом слое машин и уходит из раскладки вместе с
/// ним: настраивать долю занятых мест, пока ряда на карте нет, нечем — то же
/// правило, что убирает ручки невыбранного бэкенда в Nav. Строка `Cars`
/// остаётся: ею слой и возвращают.
fn sync_occupancy_row_visibility(
    cars: Res<CarStyle>,
    mut rows: Query<&mut Node, With<CarOccupancyRow>>,
) {
    let display = if cars.visible {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut rows {
        if node.display != display {
            node.display = display;
        }
    }
}
