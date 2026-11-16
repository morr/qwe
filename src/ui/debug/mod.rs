//! Дебаг-тумблеры (порт `zxc/src/ui/debug/toggles.rs` на ресурсах вместо
//! стейтов): строки grid / doors / movepath в секции Overlays вкладки Debug.
//!
//! - grid — сетка navtiles гизмо-линиями;
//! - doors — входы в здания, свои и досочинённые (`map/osm/entrances/`);
//! - movepath — существующий `DrawMovePaths` (он же на клавише M);
//! - noise — поле хвои (`map/trees/conifer.rs`) текстурой на всю карту:
//!   серым — значение поля, зелёным — будущие хвойные массивы;
//! - road network — улицы сети и их швы (`map::mesh_network_overlay`);
//! - rut lines — вдоль чего легла колея: оси полос с линиями колёс и
//!   траектории узлов (`map::mesh_rut_overlay`);
//! - OSM contours — оси путей и контуры полигонов, какими их прочёл разбор до
//!   доводочных проходов (`map::mesh_osm_contours`): «данные или наш разбор».
//!
//! Хоткеи: N — слой навигации (`toggle_navmesh`: показ той подсистемы, по
//! которой сейчас ходят), M — movepath (в `movement`), G — «гизмо» одной
//! клавишей, то есть doors и movepath вместе. У grid хоткея нет: сетка нужна
//! редко и только вблизи, кнопки в панели достаточно.
//!
//! Добавление тумблера — тип-ресурс с `impl DebugToggle` (меткой строки),
//! `add_debug_toggle` в плагине и его реальное поведение (`overlays.rs`):
//! строка витрины и регистрация сохранения общие, их берут на себя
//! [`spawn_toggle_row`] и [`AddDebugToggleExt`].
//!
//! Кроме тумблеров вкладка держит листалки — строки, где клик перебирает
//! значения, а текущее значение написано словом справа:
//!
//! - `Camera start` — откуда стартует камера (`save` ⇄ `reset`,
//!   `camera::CameraPositionMode`);
//! - `Antialias` — сглаживание кромок карты (`4x` ⇄ `Off`, `post::Antialias`);
//! - `Navtile` — сторона ячейки навигации (`grid::NavtileBase`, смена
//!   перезагружает мир);
//! - `Raw OSM` — сырой OSM (`map::osm::parse::RawOsm`): `Off` → `parse`
//!   (доводочные проходы разбора выключены) → `parse+draw` (и достройки
//!   отрисовки); смена перезагружает мир.
//!
//! Замыкает вкладку `reset` — кнопка-действие, возвращающая ВСЕ настройки к
//! умолчаниям (`prefs::ResetSettings`), включая мировые: если уведены город,
//! seed, детерминизм или навтайл, клик перезагружает мир. Вкладка Debug для неё
//! — верное место: тут же стоят обе листалки, то есть половина ответа на вопрос
//! «а я далеко ушёл от базовых?».
//!
//! Настройки бэкендов поиска пути — сеточный слой, алгоритм, радиус агента —
//! стоят в панели Navigation (`ui/navigation/`) рядом друг с другом: они
//! взаимоисключающие, и видеть надо только настройки выбранного. Навтайл к ним
//! не относится, хотя и жил там: в тайлах этого размера мир строится всегда —
//! заливка проходимости, отсечение недостижимого, снап портала, генерация
//! входов в здания, — по какому бы бэкенду ни ходили пешки, и прятать его
//! вместе с настройками сетки значило бы называть глобальное частным.

use std::ops::{Deref, DerefMut};

use bevy::input::common_conditions::input_just_pressed;
use bevy::reflect::GetTypeRegistration;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::ui_widgets::Activate;
use derive_more::{Deref, DerefMut};

use bevy::prelude::*;

use crate::camera::CameraPositionMode;
use crate::grid::NavtileBase;
use crate::loading::{AppState, WorldInitSet};
use crate::map::RutLines;
use crate::map::osm::parse::RawOsm;
use crate::map::trees::{ConiferNoiseStyle, TreeRowStyle, TreeStyle};
use crate::movement::DrawMovePaths;
use crate::navigation::PolymeshDebug;
use crate::post::Antialias;
use crate::prefs::{PrefsExt, ResetSettings};
use crate::ui::knob::{AddKnobsExt, CycleBinding, Knobbed, spawn_cycle_row};
use crate::ui::rows::{ROW_LEFT_PX, on_off};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_block, spawn_section};
use crate::ui::{UiBuildSet, panel_title, spawn_panel_button};

// тумблеры — группы настроек (`prefs`), поэтому Reflect + SettingsGroup;
// Deref/DerefMut до bool даёт дженерику строки ([`spawn_toggle_row`]) доступ
// к значению — кортеж-поле `.0` сквозь трейт не выражается
#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "grid")]
pub struct DebugGrid(pub bool);

impl DebugToggle for DebugGrid {
    fn label() -> &'static str {
        "Grid"
    }
}

/// Показывать ли заливку непроходимых тайлов — строка `Show` под `Navmesh` в
/// панели Navigation (`ui/navigation/`). Слой рисуется, только пока сетка и
/// есть бэкенд навигации: поверх меша, по которому ходят, он показывал бы не
/// ту проходимость.
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "navmesh")]
pub struct DebugNavmesh(pub bool);

#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "doors")]
pub struct DebugDoors(pub bool);

impl DebugToggle for DebugDoors {
    fn label() -> &'static str {
        "Doors"
    }
}

#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "conifer_noise")]
pub struct DebugConiferNoise(pub bool);

impl DebugToggle for DebugConiferNoise {
    fn label() -> &'static str {
        "Noise field"
    }
}

/// Показывать ли оверлей сети улиц — строка `Road network` вкладки Debug:
/// склейка ways в улицы, сечения и швы (`map::mesh_network_overlay`).
#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "road_network")]
pub struct DebugRoadNetwork(pub bool);

impl DebugToggle for DebugRoadNetwork {
    fn label() -> &'static str {
        "Road network"
    }
}

/// Показывать ли оверлей линий колеи — строка `Rut lines` вкладки Debug: оси
/// полос с двумя линиями колёс (колея шейдера асфальта) и траектории узлов
/// (колея краски износа), разными цветами (`map::mesh_rut_overlay`).
#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "rut_lines")]
pub struct DebugRutLines(pub bool);

impl DebugToggle for DebugRutLines {
    fn label() -> &'static str {
        "Rut lines"
    }
}

/// Показывать ли оверлей контуров OSM — строка `OSM contours` вкладки Debug:
/// оси путей и контуры полигонов, какими их прочёл разбор до доводочных
/// проходов (`map::mesh_osm_contours`), поверх обработанной карты.
#[derive(Resource, Reflect, SettingsGroup, Default, Deref, DerefMut)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "osm_contours")]
pub struct DebugOsmContours(pub bool);

impl DebugToggle for DebugOsmContours {
    fn label() -> &'static str {
        "OSM contours"
    }
}

/// `DrawMovePaths` живёт у владельца (`movement`), но его строка — витрина
/// этой вкладки, поэтому impl стоит рядом с соседними тумблерами.
impl DebugToggle for DrawMovePaths {
    fn label() -> &'static str {
        "Move paths"
    }
}

/// Тумблер вкладки Debug: bool-ресурс, чья строка (`On`/`Off`) ничем, кроме
/// метки, не отличается от соседних. От конкретного типа требуется только
/// метка — переворот и текст состояния берёт дженерик
/// ([`spawn_toggle_row`]), доступ к значению дают derive-и `Deref`/`DerefMut`.
pub trait DebugToggle: Knobbed + Deref<Target = bool> + DerefMut + Default {
    /// Метка строки вкладки Debug.
    fn label() -> &'static str;
}

/// Регистрация тумблера одним вызовом: синхронизация строки витрины
/// ([`AddKnobsExt::add_knobs`]) поверх полного вызова сохраняемого ресурса
/// ([`PrefsExt::register_pref`]). Забытая половина здесь — ошибка сборки,
/// а не тихий отказ, тем же способом, каким [`PrefsExt::register_pref`]
/// закрыл свой старый квартет.
pub trait AddDebugToggleExt {
    fn add_debug_toggle<T: DebugToggle + SettingsGroup + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self;
}

impl AddDebugToggleExt for App {
    fn add_debug_toggle<T: DebugToggle + SettingsGroup + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self {
        self.add_knobs::<T>().register_pref::<T>()
    }
}

/// Строка-тумблер: метка из [`DebugToggle::label`], значение справа —
/// `On`/`Off`, клик переворачивает ресурс. Одна витрина на все тумблерные
/// строки вкладки.
fn spawn_toggle_row<T: DebugToggle>(
    commands: &mut Commands,
    panel: Entity,
    left_px: f32,
    resource: &T,
) -> Entity {
    spawn_cycle_row(
        commands,
        panel,
        T::label(),
        left_px,
        resource,
        CycleBinding {
            cycle: |toggle: &mut T| **toggle = !**toggle,
            text: |toggle| on_off(**toggle).to_string(),
        },
    )
}

mod overlays;

use self::overlays::{
    render_doors, render_grid, sync_conifer_noise_overlay, sync_navmesh_overlay,
    sync_osm_contours_overlay, sync_road_network_overlay, sync_rut_overlay,
};

pub struct UiDebugTogglesPlugin;

impl Plugin for UiDebugTogglesPlugin {
    fn build(&self, app: &mut App) {
        app.add_debug_toggle::<DebugGrid>()
            .add_debug_toggle::<DebugDoors>()
            .add_debug_toggle::<DebugConiferNoise>()
            .add_debug_toggle::<DebugRoadNetwork>()
            .add_debug_toggle::<DebugOsmContours>()
            .add_debug_toggle::<DebugRutLines>()
            // у заливки navmesh строки в этой вкладке нет — её «Show» стоит
            // в панели Navigation (`ui/navigation/`), так что синхронизации
            // строк у неё не заводится; сохранение и сброс — остаются
            .register_pref::<DebugNavmesh>()
            // DrawMovePaths регистрирует владелец (`movement`), здесь — только
            // синхронизация его строки; листалки регистрируются у владельцев
            .add_knobs::<DrawMovePaths>()
            .add_knobs::<CameraPositionMode>()
            .add_knobs::<Antialias>()
            .add_knobs::<NavtileBase>()
            .add_knobs::<RawOsm>()
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
                    sync_road_network_overlay,
                    sync_osm_contours_overlay,
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
                    // показывать не ту проходимость.
                    //
                    // Гейт на `Playing` обязателен: смена города проходит через
                    // `Loading`, где навмеш уже перезалит под новый город, а
                    // сущность слоя ещё жива — без гейта слой строился бы по
                    // неверному масштабу и рядом со старым остался бы второй.
                    sync_navmesh_overlay
                        .run_if(in_state(AppState::Playing))
                        .run_if(
                            resource_changed::<DebugNavmesh>
                                .or_else(resource_changed::<PolymeshDebug>),
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
                    sync_road_network_overlay
                        .run_if(in_state(AppState::Playing))
                        .run_if(resource_changed::<DebugRoadNetwork>),
                    // контуры — снимок разбора, ручки их не двигают: слой
                    // строится по тумблеру и на входе в мир, как сеть
                    sync_osm_contours_overlay
                        .run_if(in_state(AppState::Playing))
                        .run_if(resource_changed::<DebugOsmContours>),
                    // линии колеи кладёт сборка дорог ресурсом — и на входе в
                    // мир, и на всякой пересборке по ручкам формы: слой идёт
                    // за ресурсом, отдельной строки в `OnEnter` ему не нужно
                    sync_rut_overlay.run_if(in_state(AppState::Playing)).run_if(
                        resource_changed::<DebugRutLines>.or_else(resource_changed::<RutLines>),
                    ),
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

/// Всё, что читают строки вкладки при спавне, система берёт напрямую
/// ресурсами: значение каждого читает только его собственная строка, снимок
/// для нескольких читателей не нужен. Аргумент — на строку, число растёт
/// механически, а не потому что система обросла настройками, — поэтому
/// лимит глушится, а не сворачивается обратно в снимок-`SystemParam`.
#[allow(clippy::too_many_arguments)]
fn build_debug_tab(
    mut commands: Commands,
    panes: Res<SettingsPanes>,
    grid: Res<DebugGrid>,
    doors: Res<DebugDoors>,
    movepaths: Res<DrawMovePaths>,
    conifer_noise: Res<DebugConiferNoise>,
    road_network: Res<DebugRoadNetwork>,
    rut_lines: Res<DebugRutLines>,
    osm_contours: Res<DebugOsmContours>,
    position_mode: Res<CameraPositionMode>,
    antialias: Res<Antialias>,
    navtile: Res<NavtileBase>,
    raw_osm: Res<RawOsm>,
) {
    let pane = panes.pane(SettingsTab::Debug);

    let overlays = spawn_section(
        &mut commands,
        pane,
        SectionSlot::Overlays,
        panel_title("Overlays"),
        "debug_overlays",
    );
    spawn_toggle_row::<DebugGrid>(&mut commands, overlays, ROW_LEFT_PX, &*grid);
    spawn_toggle_row::<DebugDoors>(&mut commands, overlays, ROW_LEFT_PX, &*doors);
    spawn_toggle_row::<DrawMovePaths>(&mut commands, overlays, ROW_LEFT_PX, &*movepaths);
    spawn_toggle_row::<DebugConiferNoise>(&mut commands, overlays, ROW_LEFT_PX, &*conifer_noise);
    spawn_toggle_row::<DebugRoadNetwork>(&mut commands, overlays, ROW_LEFT_PX, &*road_network);
    spawn_toggle_row::<DebugRutLines>(&mut commands, overlays, ROW_LEFT_PX, &*rut_lines);
    spawn_toggle_row::<DebugOsmContours>(&mut commands, overlays, ROW_LEFT_PX, &*osm_contours);

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
        &*position_mode,
        CycleBinding {
            cycle: |mode: &mut CameraPositionMode| *mode = mode.next(),
            text: |mode| mode.label().to_string(),
        },
    );
    // сглаживание кромок карты: MSAA 4x ⇄ Off (`post::Antialias`)
    spawn_cycle_row(
        &mut commands,
        world,
        "Antialias",
        ROW_LEFT_PX,
        &*antialias,
        CycleBinding {
            cycle: |antialias: &mut Antialias| antialias.0 = !antialias.0,
            text: |antialias| if antialias.0 { "4x" } else { "Off" }.to_string(),
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
        &*navtile,
        CycleBinding {
            cycle: |navtile: &mut NavtileBase| *navtile = navtile.next(),
            text: |navtile| navtile.label().to_string(),
        },
    );
    // сырой OSM: Off → parse → parse+draw — наши достройки выключаются, мир
    // перезагружается (`city::parse_knobs_diverged`): это вход разбора, как навтайл
    spawn_cycle_row(
        &mut commands,
        world,
        "Raw OSM",
        ROW_LEFT_PX,
        &*raw_osm,
        CycleBinding {
            cycle: |raw: &mut RawOsm| *raw = raw.next(),
            text: |raw| raw.label().to_string(),
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
