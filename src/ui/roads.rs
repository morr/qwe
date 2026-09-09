//! Панель стиля дорожных лент: стык на изломе, сглаживание осевой, кант,
//! тротуары, разметка. Полей ввода в `bevy_ui` нет, поэтому каждая строка —
//! кнопка, листающая значение по кругу (как у панели деревьев); правка
//! `RoadStyle` пересобирает дорожные слои (`map::roads::rebuild_roads`).
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

use crate::map::{CarStyle, RoadJoin, RoadSmoothing, RoadStyle, TramStyle};
use crate::settings::{CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_STEP};
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
