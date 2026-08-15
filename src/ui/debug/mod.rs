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

use bevy::ecs::system::IntoObserverSystem;
use bevy::feathers::controls::ButtonVariant;
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
use crate::ui::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiLeftColumn, button_variant, panel_background,
    panel_button_label, row_label, spawn_panel_button,
};

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

/// Какой слой переключает кнопка; определяет подсветку «активна».
#[derive(Component, Clone, Copy)]
enum DebugToggleButton {
    Grid,
    Doors,
    Movepath,
    ConiferNoise,
}

/// Кнопка-листалка в этом же ряду. Зелёная, пока выбрано значение по
/// умолчанию, — так видно, что настройки не уведены от базовых, тем же цветом,
/// каким тумблеры показывают «включено».
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CyclerButton {
    Camera,
    Navtile,
}

/// Текст значения справа на листалке; та же метка, что на самой кнопке, —
/// один компонент на все листалки, а не маркер на каждую (идиома
/// `ui/navigation/mod.rs::NavValueLabel`).
#[derive(Component, Clone, Copy)]
struct CyclerValueLabel(CyclerButton);

/// Что показывает листалка и стоит ли она на умолчании. Одна функция на спавн
/// и на актуализацию: разойтись подписи и подсветке негде.
fn cycler_state(
    kind: CyclerButton,
    position_mode: &CameraPositionMode,
    navtile: &NavtileBase,
) -> (String, bool) {
    match kind {
        CyclerButton::Camera => (
            position_mode.label().to_string(),
            *position_mode == CameraPositionMode::default(),
        ),
        CyclerButton::Navtile => (
            navtile.label().to_string(),
            *navtile == NavtileBase::default(),
        ),
    }
}

mod overlays;

use self::overlays::{render_doors, render_grid, sync_conifer_noise_overlay, sync_navmesh_overlay};

pub struct UiDebugTogglesPlugin;

impl Plugin for UiDebugTogglesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugGrid>()
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
            .add_systems(Startup, render_debug_toggles)
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
                    sync_toggle_buttons,
                    sync_cycler_buttons,
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
                    sync_cycler_labels.run_if(
                        resource_changed::<CameraPositionMode>
                            .or_else(resource_changed::<NavtileBase>),
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

fn render_debug_toggles(
    mut commands: Commands,
    position_mode: Res<CameraPositionMode>,
    navtile: Res<NavtileBase>,
    grid: Res<DebugGrid>,
    doors: Res<DebugDoors>,
    movepaths: Res<DrawMovePaths>,
    conifer_noise: Res<DebugConiferNoise>,
) {
    let row = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                column_gap: px(6.),
                padding: UiRect::all(px(10.)),
                ..default()
            },
            panel_background(),
            // низ левой колонки: панель Noise стыкуется прямо над этим рядом
            UiLeftColumn::DebugToggles,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("debug_toggles"),
        ))
        .id();

    spawn_toggle(
        &mut commands,
        row,
        "grid",
        DebugToggleButton::Grid,
        grid.0,
        |_activate: On<Activate>, mut grid: ResMut<DebugGrid>| {
            grid.0 = !grid.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "doors",
        DebugToggleButton::Doors,
        doors.0,
        |_activate: On<Activate>, mut doors: ResMut<DebugDoors>| {
            doors.0 = !doors.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "movepath",
        DebugToggleButton::Movepath,
        movepaths.0,
        |_activate: On<Activate>, mut movepaths: ResMut<DrawMovePaths>| {
            movepaths.0 = !movepaths.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "noise",
        DebugToggleButton::ConiferNoise,
        conifer_noise.0,
        |_activate: On<Activate>, mut noise: ResMut<DebugConiferNoise>| {
            noise.0 = !noise.0;
        },
    );

    // откуда стартует камера — клик листает reset ⇄ save
    spawn_cycler(
        &mut commands,
        row,
        CyclerButton::Camera,
        "camera:",
        &position_mode,
        &navtile,
        |_activate: On<Activate>, mut mode: ResMut<CameraPositionMode>| {
            *mode = mode.next();
        },
    );
    // сторона навтайла: клик листает 2m ⇄ 1m и перезагружает мир
    // (`city::reload_world`) — проходимость существует только в тайлах
    // текущего размера
    spawn_cycler(
        &mut commands,
        row,
        CyclerButton::Navtile,
        "navtile:",
        &position_mode,
        &navtile,
        |_activate: On<Activate>, mut navtile: ResMut<NavtileBase>| {
            *navtile = navtile.next();
        },
    );

    // сброс всех настроек на умолчания. Кнопка-действие, а не тумблер и не
    // листалка: зелёный в этом ряду значит «этот ресурс стоит на умолчании», и
    // тем же цветом на кнопке, которая говорит о ЧУЖИХ ресурсах, читалось бы
    // другое утверждение — поэтому она всегда обычная (`is_active: false`), а
    // подсветку под курсором ей даёт feathers, как всякой кнопке
    spawn_panel_button(
        &mut commands,
        row,
        (),
        "reset",
        false,
        |_activate: On<Activate>, mut commands: Commands| {
            commands.queue(ResetSettings);
        },
    );
}

/// Кнопка-листалка: подпись слева, текущее значение справа.
fn spawn_cycler<M>(
    commands: &mut Commands,
    row: Entity,
    kind: CyclerButton,
    label: &str,
    position_mode: &CameraPositionMode,
    navtile: &NavtileBase,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let (value, is_default) = cycler_state(kind, position_mode, navtile);
    // подпись и значение — в своей строке-обёртке: узел самой кнопки приходит
    // из сцены feathers, и переписывать его целиком ради одного `column_gap`
    // значило бы потерять всё остальное, что она в нём задала
    let caption = (
        Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6.),
            ..default()
        },
        crate::ui::text_container(),
        children![
            row_label(label),
            (CyclerValueLabel(kind), panel_button_label(&value)),
        ],
    );
    super::spawn_panel_button_with(commands, row, kind, caption, is_default, on_activate);
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

/// Зелёный на листалках держится, пока выбрано значение по умолчанию.
fn sync_cycler_buttons(
    position_mode: Res<CameraPositionMode>,
    navtile: Res<NavtileBase>,
    mut buttons: Query<(&CyclerButton, &mut ButtonVariant)>,
) {
    for (cycler, mut variant) in &mut buttons {
        let (_, is_default) = cycler_state(*cycler, &position_mode, &navtile);
        variant.set_if_neq(button_variant(is_default));
    }
}

/// Актуализация подписей листалок после правки ресурса извне (кнопкой,
/// восстановленными настройками, BRP).
fn sync_cycler_labels(
    position_mode: Res<CameraPositionMode>,
    navtile: Res<NavtileBase>,
    mut labels: Query<(&mut Text, &CyclerValueLabel)>,
) {
    for (mut text, label) in &mut labels {
        let (value, _) = cycler_state(label.0, &position_mode, &navtile);
        text.0 = value;
    }
}

fn spawn_toggle<M>(
    commands: &mut Commands,
    row: Entity,
    label: &str,
    kind: DebugToggleButton,
    is_active: bool,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    spawn_panel_button(commands, row, kind, label, is_active, on_activate);
}

/// Зелёный на тумблерах держится, пока слой включён. Наведение и нажатие ведёт
/// feathers сама — здесь остаётся только «активность».
fn sync_toggle_buttons(
    grid: Res<DebugGrid>,
    doors: Res<DebugDoors>,
    movepaths: Res<DrawMovePaths>,
    conifer_noise: Res<DebugConiferNoise>,
    mut buttons: Query<(&DebugToggleButton, &mut ButtonVariant)>,
) {
    for (toggle, mut variant) in &mut buttons {
        let is_active = match toggle {
            DebugToggleButton::Grid => grid.0,
            DebugToggleButton::Doors => doors.0,
            DebugToggleButton::Movepath => movepaths.0,
            DebugToggleButton::ConiferNoise => conifer_noise.0,
        };
        variant.set_if_neq(button_variant(is_active));
    }
}
