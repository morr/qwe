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
//! Кроме тумблеров ряд держит листающую кнопку `camera` (откуда стартует
//! камера — `save` ⇄ `reset`, `camera::CameraPositionMode`): другого ряда
//! кнопок в UI нет, а
//! заводить панель на одну строку незачем. Всё, что про навигацию — сеточный
//! слой, размер навтайла, алгоритм поиска пути, — жило здесь же, а теперь
//! стоит в панели Navigation (`ui/navigation.rs`) рядом с настройками второго
//! бэкенда: они взаимоисключающие, и видеть надо только настройки выбранного.

use bevy::asset::RenderAssetUsages;
use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::image::{Image, ImageSampler};
use bevy::input::common_conditions::input_just_pressed;
use bevy::picking::hover::Hovered;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;

use bevy::prelude::*;

use crate::camera::CameraPositionMode;
use crate::grid::tile_center;
use crate::loading::{AppState, WorldInitSet};
use crate::map::ConiferField;
use crate::map::osm::MapData;
use crate::map::trees::{ConiferNoiseStyle, TreeRowStyle, TreeStyle};
use crate::movement::DrawMovePaths;
use crate::navigation::{ArcNavmesh, PolymeshDebug};
use crate::settings::{MAP_SIZE, Z_CONIFER_NOISE_OVERLAY, grid_size, navtile_size};
use crate::ui::{
    GameUiRoot, TOGGLE_ACTIVE_COLOR, TOGGLE_HOVER_LIGHTEN, TOGGLE_PRESSED_LIGHTEN,
    UI_SCREEN_EDGE_PX_OFFSET, UiLeftColumnSlot, UiOpacity, spawn_panel_button, ui_color,
};

// оба тумблера — группы настроек (`prefs`), поэтому Reflect + SettingsGroup
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "grid")]
pub struct DebugGrid(pub bool);

/// Показывать ли заливку непроходимых тайлов — строка `Show` под `Navmesh` в
/// панели Navigation (`ui/navigation.rs`). Слой рисуется, только пока сетка и
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

#[derive(Component)]
struct NavmeshOverlayMarker;

/// Слой поля хвои и то, под что он нарисован: порог и поколение поля.
/// Пересобирать текстуру, пока оба те же, незачем — правка любого другого поля
/// `TreeStyle` (тот же ползунок плотности) иначе перерисовывала бы её на
/// каждом шаге. Одного порога мало: правка параметров шума (панель Noise)
/// меняет рельеф поля, а порог-квантиль при этом может совпасть числом.
#[derive(Component)]
struct ConiferNoiseOverlayMarker {
    threshold: f32,
    generation: u32,
}

/// Подпись на кнопке-переключателе стартовой позиции камеры.
#[derive(Component)]
struct CameraPositionLabel;

/// Кнопка-листалка в этом же ряду. Зелёная, пока выбрано значение по
/// умолчанию, — так видно, что настройки не уведены от базовых, тем же цветом,
/// каким тумблеры показывают «включено».
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum CyclerButton {
    Camera,
}

/// Z заливки navmesh: над зданиями (5.0), под юнитами (5.5+).
const NAVMESH_OVERLAY_Z: f32 = 5.2;

/// Сторона текстуры поля хвои, тексели: 512 на 5.6 км карты — тексель ~11 м,
/// вдвое мельче кроны, то есть контур массива читается точно, а пересборка
/// слоя остаётся четвертью миллиона выборок, а не миллионами.
const CONIFER_NOISE_OVERLAY_PX: u32 = 512;
/// Прозрачность поля: под порогом слой только подсвечивает рельеф шума и не
/// должен скрывать лес, над порогом — показывает будущий массив, и его видно.
const CONIFER_NOISE_ALPHA: f32 = 0.30;
const CONIFER_STAND_ALPHA: f32 = 0.55;
/// Цвет хвойной области — холодная зелень ели, а не листвы.
const CONIFER_STAND_COLOR: Vec3 = Vec3::new(0.15, 0.90, 0.35);

/// Метка двери, м: с высоты, на которой видно квартал, кружок меньше метра
/// уже не читается.
const DOOR_MARKER_RADIUS: f32 = 1.5;
const DOOR_COLOR: Color = Color::srgba(0.15, 0.85, 1.0, 0.9);
/// Сколько экранов вокруг камеры рисовать двери. Как и у movepath: на всю
/// карту это десять тысяч гизмо за кадр, а за соседним экраном их не видно.
const DOORS_VIEW_SCREENS: f32 = 1.5;

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
                    update_toggle_buttons,
                    update_cycler_buttons,
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
                    sync_camera_position_label.run_if(resource_changed::<CameraPositionMode>),
                    toggle_navmesh.run_if(input_just_pressed(KeyCode::KeyN)),
                    toggle_gizmos.run_if(input_just_pressed(KeyCode::KeyG)),
                ),
            );
    }
}

fn render_debug_toggles(mut commands: Commands, position_mode: Res<CameraPositionMode>) {
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
            BackgroundColor(ui_color(UiOpacity::Medium)),
            // низ левой колонки: панель Noise стыкуется прямо над этим рядом
            UiLeftColumnSlot(0),
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
        |_activate: On<Activate>, mut grid: ResMut<DebugGrid>| {
            grid.0 = !grid.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "doors",
        DebugToggleButton::Doors,
        |_activate: On<Activate>, mut doors: ResMut<DebugDoors>| {
            doors.0 = !doors.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "movepath",
        DebugToggleButton::Movepath,
        |_activate: On<Activate>, mut movepaths: ResMut<DrawMovePaths>| {
            movepaths.0 = !movepaths.0;
        },
    );
    spawn_toggle(
        &mut commands,
        row,
        "noise",
        DebugToggleButton::ConiferNoise,
        |_activate: On<Activate>, mut noise: ResMut<DebugConiferNoise>| {
            noise.0 = !noise.0;
        },
    );

    // откуда стартует камера — клик листает reset ⇄ save
    let camera_button = commands
        .spawn((
            Button,
            Pickable::default(),
            Hovered::default(),
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6.),
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            CyclerButton::Camera,
            BackgroundColor(cycler_background(
                *position_mode == CameraPositionMode::default(),
                false,
                false,
            )),
            children![
                (
                    Text::new("camera:"),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.78, 0.75)),
                ),
                (
                    CameraPositionLabel,
                    Text::new(position_mode.label()),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(
            |_activate: On<Activate>, mut mode: ResMut<CameraPositionMode>| {
                *mode = mode.next();
            },
        )
        .id();
    commands.entity(row).add_child(camera_button);
}

/// N — «показать слой навигации»: у сетки и у меша свои тумблеры показа, а
/// клавиша одна, и жать её осмысленно только для той подсистемы, по которой
/// сейчас ходят (панель Navigation, `ui/navigation.rs`).
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

/// Фон кнопки ряда: зелёный, когда значение «активно» (тумблер включён,
/// листалка стоит на умолчании), плюс осветление под курсором и под нажатием.
fn cycler_background(is_active: bool, is_pressed: bool, is_hovered: bool) -> Color {
    let base = if is_active {
        TOGGLE_ACTIVE_COLOR
    } else {
        ui_color(UiOpacity::Heavy)
    };
    let lighten = if is_pressed {
        TOGGLE_PRESSED_LIGHTEN
    } else if is_hovered {
        TOGGLE_HOVER_LIGHTEN
    } else {
        0.0
    };
    base.mix(&Color::WHITE, lighten)
}

/// Зелёный на листалках держится, пока выбрано значение по умолчанию.
fn update_cycler_buttons(
    position_mode: Res<CameraPositionMode>,
    mut buttons: Query<(&CyclerButton, &Hovered, Has<Pressed>, &mut BackgroundColor)>,
) {
    for (cycler, hovered, is_pressed, mut background) in &mut buttons {
        let is_default = match cycler {
            CyclerButton::Camera => *position_mode == CameraPositionMode::default(),
        };
        background.set_if_neq(BackgroundColor(cycler_background(
            is_default,
            is_pressed,
            hovered.get(),
        )));
    }
}

/// Актуализация подписи при смене режима стартовой позиции (кнопкой или по BRP).
fn sync_camera_position_label(
    mode: Res<CameraPositionMode>,
    mut labels: Query<&mut Text, With<CameraPositionLabel>>,
) {
    for mut text in &mut labels {
        text.0 = mode.label().to_string();
    }
}

fn spawn_toggle<M>(
    commands: &mut Commands,
    row: Entity,
    label: &str,
    kind: DebugToggleButton,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    spawn_panel_button(commands, row, kind, label, on_activate);
}

fn update_toggle_buttons(
    grid: Res<DebugGrid>,
    doors: Res<DebugDoors>,
    movepaths: Res<DrawMovePaths>,
    conifer_noise: Res<DebugConiferNoise>,
    mut buttons: Query<(
        &DebugToggleButton,
        &Hovered,
        Has<Pressed>,
        &mut BackgroundColor,
    )>,
) {
    for (toggle, hovered, is_pressed, mut background) in &mut buttons {
        let is_active = match toggle {
            DebugToggleButton::Grid => grid.0,
            DebugToggleButton::Doors => doors.0,
            DebugToggleButton::Movepath => movepaths.0,
            DebugToggleButton::ConiferNoise => conifer_noise.0,
        };
        background.set_if_neq(BackgroundColor(cycler_background(
            is_active,
            is_pressed,
            hovered.get(),
        )));
    }
}

/// Сетка navtiles гизмо-линиями по краям тайлов.
fn render_grid(mut gizmos: Gizmos) {
    let color = Color::srgba(0.2, 0.2, 0.2, 0.3);
    let (grid_size, tile_size) = (grid_size(), navtile_size());
    for x in 0..=grid_size.x {
        let world_x = x as f32 * tile_size;
        gizmos.line_2d(
            Vec2::new(world_x, 0.0),
            Vec2::new(world_x, MAP_SIZE.y),
            color,
        );
    }
    for y in 0..=grid_size.y {
        let world_y = y as f32 * tile_size;
        gizmos.line_2d(
            Vec2::new(0.0, world_y),
            Vec2::new(MAP_SIZE.x, world_y),
            color,
        );
    }
}

/// Входы в здания — кружок на каждую дверь. Как и у movepath, рисуется
/// гизмо каждый кадр и отсекается по вьюпорту: дверей на карте под десять
/// тысяч, и гизмо на всю карту разом кладёт кадр.
fn render_doors(
    map: Res<MapData>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut gizmos: Gizmos,
) {
    let camera_position = camera.translation.truncate();
    let half_view =
        Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * DOORS_VIEW_SCREENS;

    for building in &map.buildings {
        for &door in &building.entrances {
            let offset = (door - camera_position).abs();
            if offset.x > half_view.x || offset.y > half_view.y {
                continue;
            }
            gizmos.circle_2d(door, DOOR_MARKER_RADIUS, DOOR_COLOR);
        }
    }
}

/// Спавн/despawn заливки непроходимых тайлов при переключении тумблера.
/// Один слитый меш на все тайлы: на OSM-карте их сотни тысяч, отдельные
/// entity на каждый укладывали кадр.
fn sync_navmesh_overlay(
    mut commands: Commands,
    navmesh_show: Res<DebugNavmesh>,
    polymesh: Res<PolymeshDebug>,
    arc_navmesh: Res<ArcNavmesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    overlay: Query<Entity, With<NavmeshOverlayMarker>>,
) {
    if polymesh.enabled || !navmesh_show.0 {
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        return;
    }

    let color = Color::srgba(0.9, 0.15, 0.15, 0.35).to_linear();
    let mut builder = crate::map::MeshBuilder::default();
    let navmesh = arc_navmesh.read();
    for x in 0..navmesh.grid_size.x {
        for y in 0..navmesh.grid_size.y {
            if navmesh.is_passable(x, y) {
                continue;
            }
            let center = tile_center(IVec2::new(x, y));
            builder.push_rect(
                center - navmesh.tile_size / 2.0,
                center + navmesh.tile_size / 2.0,
                color,
            );
        }
    }
    if builder.is_empty() {
        return;
    }

    commands.spawn((
        NavmeshOverlayMarker,
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(materials.add(ColorMaterial {
            alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, NAVMESH_OVERLAY_Z),
        DespawnOnExit(AppState::Playing),
        Name::new("navmesh_overlay"),
    ));
}

/// Спавн/despawn слоя поля хвои. Слой — один спрайт на всю карту с текстурой,
/// посчитанной на CPU: заливать поле мешем в четверть миллиона квадов незачем,
/// оно и так гладкое.
///
/// Серая рампа — само значение поля, зелёным залито всё, что не ниже текущего
/// порога, то есть **будущий хвойный массив**: так видно и рельеф шума, и что
/// из него отберёт ползунок доли. Зелень покрывает и застройку — поле
/// определено на всей карте, а деревья стоят только в лесах.
///
/// Слой рисует поле **без примеси** (`TreeStyle::noise_mix` живёт только в
/// деревьях): при mix > 0 одиночные кроны намеренно стоят «не с той стороны»
/// зелёной границы — это вкрапления, а не рассинхрон слоя.
fn sync_conifer_noise_overlay(
    mut commands: Commands,
    enabled: Res<DebugConiferNoise>,
    field: Res<ConiferField>,
    mut images: ResMut<Assets<Image>>,
    overlay: Query<(Entity, &ConiferNoiseOverlayMarker)>,
) {
    let threshold = field.threshold();
    if enabled.0
        && overlay.iter().any(|(_, drawn)| {
            drawn.threshold.to_bits() == threshold.to_bits()
                && drawn.generation == field.generation()
        })
    {
        return;
    }
    for (entity, _) in &overlay {
        commands.entity(entity).despawn();
    }
    if !enabled.0 {
        return;
    }

    let side = CONIFER_NOISE_OVERLAY_PX;
    let mut data = Vec::with_capacity((side * side * 4) as usize);
    for row in 0..side {
        for column in 0..side {
            // строка 0 текстуры — верх спрайта, то есть максимальный мировой y
            let position = Vec2::new(
                (column as f32 + 0.5) / side as f32 * MAP_SIZE.x,
                (side - 1 - row) as f32 / side as f32 * MAP_SIZE.y,
            );
            let value = field.sample(position);
            let (color, alpha) = if value >= threshold {
                (CONIFER_STAND_COLOR * value, CONIFER_STAND_ALPHA)
            } else {
                (Vec3::splat(value), CONIFER_NOISE_ALPHA)
            };
            let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0) as u8;
            data.extend_from_slice(&[byte(color.x), byte(color.y), byte(color.z), byte(alpha)]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // приложение целиком на `ImagePlugin::default_nearest()` (пиксель-арт
    // спрайтов), но поле гладкое: с ближайшим соседом тексель в 11 м вылезал
    // на карту квадратом в пол-кроны, и слой читался как сетка, а не как шум
    image.sampler = ImageSampler::linear();

    commands.spawn((
        ConiferNoiseOverlayMarker {
            threshold,
            generation: field.generation(),
        },
        Sprite {
            image: images.add(image),
            custom_size: Some(MAP_SIZE),
            ..default()
        },
        Transform::from_translation((MAP_SIZE / 2.0).extend(Z_CONIFER_NOISE_OVERLAY)),
        DespawnOnExit(AppState::Playing),
        Name::new("conifer_noise_overlay"),
    ));
}
