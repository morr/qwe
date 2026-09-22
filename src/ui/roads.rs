//! Секция Roads: форма дорог и то, что на них стоит. Сверху — ползунки
//! `RoadShape` (`map::roads::shape`): ширина полосы, длина клина между
//! сечениями, допуск оси, зазор разделительной и радиус скругления бордюра;
//! под ними — тумблер тротуаров (`RoadStyle`). Краска и износ ушли в свою
//! секцию, Road paint (`ui/road_paint.rs`).
//!
//! Ползунки формы пишут `RoadShape` на каждом делении, но карта следует за
//! `RoadShapeOnMap` — копией, которая доезжает до неё после паузы
//! (`shape::settle_road_shape`): пересборка осей, пар, узлов и ряда машин
//! на каждое пройденное деление стоила бы сотни миллисекунд. Ширина полосы,
//! осев, перезагружает мир (`city::reload_world`): её читает разбор.
//! Прежние строки Joins, Smoothing и Casing ушли вместе со старым путём
//! рисования (`map::roads::RoadStyle`).
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
    CORNER_RADIUS_MAX, CORNER_RADIUS_MIN, CORNER_RADIUS_STEP, CURVE_TOLERANCE_MAX,
    CURVE_TOLERANCE_MIN, CURVE_TOLERANCE_STEP, CarStyle, LANE_WIDTH_MAX, LANE_WIDTH_MIN,
    LANE_WIDTH_STEP, MEDIAN_GAP_MAX, MEDIAN_GAP_MIN, MEDIAN_GAP_STEP, RoadShape, RoadStyle,
    TAPER_MAX, TAPER_MIN, TAPER_STEP, TramStyle,
};
use crate::ui::knob::{AddKnobsExt, CycleBinding, SliderBinding, spawn_cycle_row, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_section};
use crate::ui::{PanelCount, UiBuildSet, panel_header};

/// Строка ползунка занятости — по ней видимость следует за тумблером `Cars`.
#[derive(Component)]
struct CarOccupancyRow;

pub struct UiRoadStylePlugin;

impl Plugin for UiRoadStylePlugin {
    fn build(&self, app: &mut App) {
        // подписи вслед за ресурсом — и на клик по кнопке, и на правку по BRP
        app.add_knobs::<RoadShape>()
            .add_knobs::<RoadStyle>()
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
    shape: Res<RoadShape>,
    style: Res<RoadStyle>,
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

    for (label, binding) in shape_knobs() {
        spawn_knob(&mut commands, panel, label, &*shape, binding);
    }
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

/// Ползунки формы дорог — подпись и привязка к полю `RoadShape`. Одна таблица
/// на панель игры и панель витрины `roads`: подбирать значения удобнее там,
/// рядом с эталоном, и шкалы у двух панелей не должны разойтись.
pub fn shape_knobs() -> [(&'static str, SliderBinding<RoadShape>); 5] {
    [
        (
            "Lane width",
            SliderBinding {
                get: |shape| shape.lane_width,
                set: |shape, value| shape.lane_width = value,
                range: (LANE_WIDTH_MIN, LANE_WIDTH_MAX, LANE_WIDTH_STEP),
                text: |value| format!("{value:.2} m"),
            },
        ),
        (
            "Taper",
            SliderBinding {
                get: |shape| shape.taper,
                set: |shape, value| shape.taper = value,
                range: (TAPER_MIN, TAPER_MAX, TAPER_STEP),
                // метров клина на метр разницы ширин
                text: |value| format!("{value:.0}x"),
            },
        ),
        (
            "Curve tolerance",
            SliderBinding {
                get: |shape| shape.curve_tolerance,
                set: |shape, value| shape.curve_tolerance = value,
                range: (
                    CURVE_TOLERANCE_MIN,
                    CURVE_TOLERANCE_MAX,
                    CURVE_TOLERANCE_STEP,
                ),
                // ноль — ось по точкам OSM, и это стоит сказать словом
                text: |value| {
                    if value > 0.0 {
                        format!("{value:.1} m")
                    } else {
                        "OSM".to_string()
                    }
                },
            },
        ),
        (
            "Median gap",
            SliderBinding {
                get: |shape| shape.median_gap,
                set: |shape, value| shape.median_gap = value,
                range: (MEDIAN_GAP_MIN, MEDIAN_GAP_MAX, MEDIAN_GAP_STEP),
                text: |value| format!("{value:.1} m"),
            },
        ),
        (
            "Corner radius",
            SliderBinding {
                get: |shape| shape.corner_radius,
                set: |shape, value| shape.corner_radius = value,
                range: (CORNER_RADIUS_MIN, CORNER_RADIUS_MAX, CORNER_RADIUS_STEP),
                text: |value| format!("{value:.1}x"),
            },
        ),
    ]
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
