//! Дебаг-тумблеры (порт `zxc/src/ui/debug/toggles.rs` на ресурсах вместо
//! стейтов): кнопки grid / doors / movepath в левом нижнем углу.
//!
//! - grid — сетка navtiles гизмо-линиями;
//! - doors — входы в здания, свои и досочинённые (`map/osm/entrances/`);
//! - movepath — существующий `DrawMovePaths` (он же на клавише M);
//! - noise — поле хвои (`map/trees/conifer.rs`) текстурой на всю карту:
//!   серым — значение поля, зелёным — будущие хвойные массивы.
//!
//! Хоткеи: N — слой навигации (`toggle_navmesh`: показ той подсистемы, по
//! которой сейчас ходят), M — movepath (в `movement`), G — «гизмо» одной
//! клавишей, то есть doors и movepath вместе. У grid хоткея нет: сетка нужна
//! редко и только вблизи, кнопки в панели достаточно.
//!
//! Кроме тумблеров ряд держит листалки — кнопки, где клик перебирает значения,
//! а зелёный держится, пока стоит умолчание:
//!
//! - `camera:` — откуда стартует камера (`save` ⇄ `reset`,
//!   `camera::CameraPositionMode`);
//! - `navtile:` — сторона ячейки навигации (`settings::NavtileBase`, смена
//!   перезагружает мир).
//!
//! Замыкает ряд `reset` — кнопка-действие, возвращающая ВСЕ настройки к
//! умолчаниям (`prefs::ResetSettings`), включая мировые: если уведены город,
//! seed, детерминизм или навтайл, клик перезагружает мир. Ряд для неё — верное
//! место: тут же стоят две листалки, зелёные ровно тогда, когда их настройка на
//! умолчании, то есть половина ответа на вопрос «а я далеко ушёл от базовых?».
//!
//! Настройки бэкендов поиска пути — сеточный слой, алгоритм, радиус агента —
//! стоят в панели Navigation (`ui/navigation/`) рядом друг с другом: они
//! взаимоисключающие, и видеть надо только настройки выбранного. Навтайл к ним
//! не относится, хотя и жил там: в тайлах этого размера мир строится всегда —
//! заливка проходимости, отсечение недостижимого, снап портала, генерация
//! входов в здания, — по какому бы бэкенду ни ходили пешки, и прятать его
//! вместе с настройками сетки значило бы называть глобальное частным.

use bevy::ecs::system::SystemParam;
use bevy::input::common_conditions::input_just_pressed;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::ui_widgets::Activate;

use bevy::prelude::*;

use crate::camera::CameraPositionMode;
use crate::loading::{AppState, WorldInitSet};
use crate::map::trees::{ConiferNoiseStyle, TreeRowStyle, TreeStyle};
use crate::movement::DrawMovePaths;
use crate::navigation::PolymeshDebug;
use crate::prefs::{ResetSettings, TrackPrefExt};
use crate::settings::NavtileBase;
use crate::ui::knob::{AddKnobsExt, CycleBinding, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_block, spawn_section};
use crate::ui::{UiBuildSet, panel_title, spawn_panel_button};

// оба тумблера — группы настроек (`prefs`), поэтому Reflect + SettingsGroup
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "grid")]
pub struct DebugGrid(pub bool);

/// Показывать ли заливку непроходимых тайлов — строка `Show` под `Navmesh` в
/// панели Navigation (`ui/navigation/`). Слой рисуется, только пока сетка и
/// есть бэкенд навигации: поверх меша, по которому ходят, он показывал бы не
/// ту проходимость.
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "navmesh")]
pub struct DebugNavmesh(pub bool);

#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "doors")]
pub struct DebugDoors(pub bool);

#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "conifer_noise")]
pub struct DebugConiferNoise(pub bool);

mod overlays;

use self::overlays::{render_doors, render_grid, sync_conifer_noise_overlay, sync_navmesh_overlay};

pub struct UiDebugTogglesPlugin;

impl Plugin for UiDebugTogglesPlugin {
    fn build(&self, app: &mut App) {
        app.add_knobs::<DebugGrid>()
            .add_knobs::<DebugDoors>()
            .add_knobs::<DrawMovePaths>()
            .add_knobs::<DebugConiferNoise>()
            .add_knobs::<CameraPositionMode>()
            .add_knobs::<NavtileBase>()
            .init_resource::<DebugGrid>()
            .init_resource::<DebugNavmesh>()
            .init_resource::<DebugDoors>()
            .init_resource::<DebugConiferNoise>()
            .register_type::<DebugGrid>()
            .register_type::<DebugNavmesh>()
            .register_type::<DebugDoors>()
            .register_type::<DebugConiferNoise>()
            .track_pref::<DebugGrid>()
            .track_pref::<DebugNavmesh>()
            .track_pref::<DebugDoors>()
            .track_pref::<DebugConiferNoise>()
            .add_systems(Startup, build_debug_tab.in_set(UiBuildSet::Sections))
            // тумблер, восстановленный из настроек, менялся до того, как
            // navmesh был заполнен и поле хвои посчитано, — красим слои ещё
            // раз по спавну мира
            .add_systems(
                OnEnter(AppState::Playing),
                (
                    sync_navmesh_overlay,
                    // порог красит хвойную область, а считает его посадка
                    sync_conifer_noise_overlay.after(crate::map::trees::build_conifer_field),
                )
                    .in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    render_grid.run_if(|grid: Res<DebugGrid>| grid.0),
                    // MapData появляется только под Playing
                    render_doors
                        .run_if(|doors: Res<DebugDoors>| doors.0)
                        .run_if(in_state(AppState::Playing)),
                    // слой сетки гаснет и при выборе полигонального бэкенда:
                    // рисовать его поверх меша, по которому ходят, — значит
                    // показывать не ту проходимость
                    sync_navmesh_overlay.run_if(
                        resource_changed::<DebugNavmesh>.or_else(resource_changed::<PolymeshDebug>),
                    ),
                    // подсвеченная область следует за ползунком доли хвои; смена
                    // состава деревьев (тумблеры Trees / Tree rows) меняет сам
                    // набор, по которому посчитано поле, панель Noise — его
                    // рельеф. `after`: пересемплирование и порог считаются в
                    // этом же кадре цепочкой деревьев — слой обязан читать поле
                    // уже после неё, а не прошлокадровое
                    sync_conifer_noise_overlay
                        .run_if(in_state(AppState::Playing))
                        .run_if(
                            resource_changed::<DebugConiferNoise>
                                .or_else(resource_changed::<TreeStyle>)
                                .or_else(resource_changed::<TreeRowStyle>)
                                .or_else(resource_changed::<ConiferNoiseStyle>),
                        )
                        .after(crate::map::trees::rebuild_trees),
                    toggle_navmesh
                        .run_if(input_just_pressed(KeyCode::KeyN))
                        .run_if(not(super::typing_in_text_input)),
                    toggle_gizmos
                        .run_if(input_just_pressed(KeyCode::KeyG))
                        .run_if(not(super::typing_in_text_input)),
                ),
            );
    }
}

/// Всё, что читают строки вкладки при спавне. Одним `SystemParam`, а не шестью
/// аргументами: строк у вкладки шесть, и каждая новая удлиняла бы подпись
/// системы (идиома `ui/navigation/mod.rs::NavPanelValues`).
#[derive(SystemParam)]
struct DebugValues<'w> {
    position_mode: Res<'w, CameraPositionMode>,
    navtile: Res<'w, NavtileBase>,
    grid: Res<'w, DebugGrid>,
    doors: Res<'w, DebugDoors>,
    movepaths: Res<'w, DrawMovePaths>,
    conifer_noise: Res<'w, DebugConiferNoise>,
}

fn build_debug_tab(mut commands: Commands, panes: Res<SettingsPanes>, values: DebugValues) {
    let pane = panes.pane(SettingsTab::Debug);

    let overlays = spawn_section(
        &mut commands,
        pane,
        SectionSlot::Overlays,
        panel_title("Overlays"),
        "debug_overlays",
    );
    spawn_cycle_row(
        &mut commands,
        overlays,
        "Grid",
        ROW_LEFT_PX,
        &*values.grid,
        CycleBinding {
            cycle: |grid: &mut DebugGrid| grid.0 = !grid.0,
            text: |grid| on_off(grid.0).to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        overlays,
        "Doors",
        ROW_LEFT_PX,
        &*values.doors,
        CycleBinding {
            cycle: |doors: &mut DebugDoors| doors.0 = !doors.0,
            text: |doors| on_off(doors.0).to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        overlays,
        "Move paths",
        ROW_LEFT_PX,
        &*values.movepaths,
        CycleBinding {
            cycle: |paths: &mut DrawMovePaths| paths.0 = !paths.0,
            text: |paths| on_off(paths.0).to_string(),
        },
    );
    spawn_cycle_row(
        &mut commands,
        overlays,
        "Noise field",
        ROW_LEFT_PX,
        &*values.conifer_noise,
        CycleBinding {
            cycle: |noise: &mut DebugConiferNoise| noise.0 = !noise.0,
            text: |noise| on_off(noise.0).to_string(),
        },
    );

    let world = spawn_section(
        &mut commands,
        pane,
        SectionSlot::WorldBuild,
        panel_title("World build"),
        "debug_world",
    );
    // откуда стартует камера — клик листает reset ⇄ save
    spawn_cycle_row(
        &mut commands,
        world,
        "Camera start",
        ROW_LEFT_PX,
        &*values.position_mode,
        CycleBinding {
            cycle: |mode: &mut CameraPositionMode| *mode = mode.next(),
            text: |mode| mode.label().to_string(),
        },
    );
    // сторона навтайла: клик листает 2m ⇄ 1m и перезагружает мир
    // (`city::reload_world`) — проходимость существует только в тайлах
    // текущего размера
    spawn_cycle_row(
        &mut commands,
        world,
        "Navtile",
        ROW_LEFT_PX,
        &*values.navtile,
        CycleBinding {
            cycle: |navtile: &mut NavtileBase| *navtile = navtile.next(),
            text: |navtile| navtile.label().to_string(),
        },
    );

    // сброс всех настроек на умолчания. Кнопка-действие, а не строка-значение:
    // показывать ей нечего, она не про своё состояние, а про чужие
    let actions = spawn_block(&mut commands, pane, SectionSlot::Actions, "debug_actions");
    spawn_panel_button(
        &mut commands,
        actions,
        (),
        "reset",
        false,
        |_activate: On<Activate>, mut commands: Commands| {
            commands.queue(ResetSettings);
        },
    );
}

/// N — «показать слой навигации»: у сетки и у меша свои тумблеры показа, а
/// клавиша одна, и жать её осмысленно только для той подсистемы, по которой
/// сейчас ходят (панель Navigation, `ui/navigation/`).
fn toggle_navmesh(mut navmesh: ResMut<DebugNavmesh>, mut polymesh: ResMut<PolymeshDebug>) {
    if polymesh.enabled {
        polymesh.show = !polymesh.show;
    } else {
        navmesh.0 = !navmesh.0;
    }
}

/// G — общий тумблер «гизмо»: doors и movepath разом. Гасит всё, если горит
/// хоть один слой, иначе зажигает оба, — чтобы одно нажатие всегда очищало
/// экран, в каком бы состоянии слои ни разошлись поодиночке.
fn toggle_gizmos(mut doors: ResMut<DebugDoors>, mut movepaths: ResMut<DrawMovePaths>) {
    let on = !(doors.0 || movepaths.0);
    doors.0 = on;
    movepaths.0 = on;
}
