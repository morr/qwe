//! Витрина крон: все варианты дерева, которые умеет рисовать игра, разом на
//! одном экране — и панель, которой их видом можно управлять вживую.
//!
//! Крона в игре задана двумя числами: **формой** (`TreeShape`) и **номером
//! варианта** (`0..TREE_VARIANTS`). Форм с собственной геометрией три —
//! `Cotton`, `Conifer`, `Palm`; четвёртая, `Mixed`, своей геометрии не имеет и
//! разрешается в `Cotton`/`Conifer` по полю хвои ещё до сборки меша. Вариантов
//! `TREE_VARIANTS` на форму, и вариант задан целиком своим номером: крона и её
//! тень разыгрываются из одного потока `variant_rng(variant)` подряд. Отсюда
//! вся сетка — 3 × [`TREE_VARIANTS`] клеток, и это исчерпывающий список: в
//! игре не бывает кроны, которой здесь нет.
//!
//! **Геометрия здесь та же, что в игре, а не её копия.** Клетку собирает
//! [`crown_variant`] — ровно тот вызов, которым `spawn_trees` набивает свой пул
//! вариантов. Тени тоже кладутся как в игре: один слитый меш на всю витрину
//! через `MeshBuilder::push_template`, а не сущность на дерево.
//!
//! **Панель слева — ручки самой генерации** (`CrownParams`, разбор — в
//! `params.rs`): число вершин базы, джиттер радиуса, крупность выступов,
//! кольца штриховки, толщины линий, геометрия тени, сид. Дефолт каждой равен
//! константе, на которой нарисован город, поэтому «Сброс к игре» возвращает
//! витрину ровно к тому, что видно в игре, а отклонение ручки читается как
//! «на столько мы от игры отошли». В самой игре этих ручек нет: город
//! рисуется дефолтом.
//!
//! Что видно в каждой клетке:
//!
//! - контур кроны и кольца штриховки — своя процедура у каждой формы
//!   (облако штрихует отдельными рёбрами, хвоя — шевронами без ГПСЧ, пальма —
//!   целыми листьями);
//! - тень варианта: растянутый силуэт, тот же силуэт со смещением или ярусный
//!   веер хвои. Какая именно — решает разыгранная на вариант «высота» кроны,
//!   поэтому в одном ряду тени разные, и это то, из-за чего лес не выглядит
//!   штампованным.
//!
//! Пример не трогает конфиг игры: ни `PrefsPlugin`, ни `MapPlugin`, ни
//! `CameraPlugin` — читать и писать `settings.toml` тут нечему.
//!
//! ```text
//! cargo run --example tree_gallery
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `H` | тени вкл/выкл |
//! | `G` | подложка: лес → парк → мостовая |
//! | `L` | подписи вкл/выкл |

mod panel;
mod params;

use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::input::common_conditions::input_just_pressed;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::window::PrimaryWindow;
use qwe::camera::cursor_offset;
use qwe::map::trees::crown_variant;
use qwe::map::{
    GROUND_COLOR, MeshBuilder, PARK_COLOR, SHADOW_COLOR, TreeShape, TreeStyle, WOOD_COLOR,
};
use qwe::settings::TREE_VARIANTS;

use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::panel::{spawn_panel, sync_param_rows, sync_reset_button};
use crate::params::Tuning;

const WINDOW_WIDTH: f32 = 1500.0;
const WINDOW_HEIGHT: f32 = 860.0;

/// Шаг сетки по горизонтали, в радиусах кроны. Тень хвои уходит вправо-вниз на
/// три «высоты» (до ~3.1 радиуса по x), так что клетка заметно шире кроны —
/// иначе веер соседа лезет в соседнюю крону. При задранной ручке «Height base»
/// тени и вправду перехлёстываются: это не раскладка сломалась, а тень стала
/// длиннее, чем в игре.
const CELL_X: f32 = 5.0;
/// Шаг по вертикали: крона, её тень вниз и строка подписи под ними.
const CELL_Y: f32 = 5.5;
/// Подпись под кроной.
const CAPTION_Y: f32 = -2.3;
/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в радиусах кроны. Подобран под стартовый зум (`spawn_camera`): на нём
/// подпись выходит ~13 экранных пикселей, а клетка — около сотни, так что
/// длиннее «Conifer #11» строке в ней всё равно не поместиться.
const TEXT_SCALE: f32 = 0.04;
const CAPTION_FONT: f32 = 20.0;
const ROW_LABEL_FONT: f32 = 30.0;
/// Подписи темнее чернил кроны не нужны — это тот же карандаш.
const LABEL_COLOR: Color = Color::srgb(0.14, 0.16, 0.20);
/// Множитель зума на щелчок колеса — как в игре (`camera.rs::ZOOM_STEP`).
/// **Мультипликативный, а не линейный**: у `PanCamera` зум прибавляется
/// (`zoom_factor -= lines·zoom_speed`), и на крупном плане, где сам
/// `zoom_factor` мельче шага, один щелчок швыряет камеру от упора до упора.
const ZOOM_STEP: f32 = 1.12;
/// Поля вокруг сетки при стартовом зуме, доля её ширины. Слева в них встаёт
/// название ряда.
const VIEW_MARGIN: f32 = 1.25;

/// Подложка витрины — те же три цвета земли, что и на карте: лес (под ним
/// кроны и растут), открытый парк, мостовая. Крона читается на всех трёх
/// по-разному, и это стоит видеть.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Ground {
    #[default]
    Wood,
    Park,
    Pavement,
}

impl Ground {
    fn color(self) -> Color {
        match self {
            Self::Wood => WOOD_COLOR,
            Self::Park => PARK_COLOR,
            Self::Pavement => GROUND_COLOR,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Wood => Self::Park,
            Self::Park => Self::Pavement,
            Self::Pavement => Self::Wood,
        }
    }
}

/// Что показано. Отдельным ресурсом, а не состоянием сущностей: кроны и тени
/// пересобираются на каждую правку ручки, и без него тумблер сбрасывался бы на
/// каждой протяжке.
#[derive(Resource)]
struct Show {
    shadows: bool,
    captions: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            shadows: true,
            captions: true,
        }
    }
}

/// Крона или слой теней — всё, что пересобирается при правке ручек.
#[derive(Component)]
struct CrownTag;

/// Слой теней — один слитый меш на всю витрину, как `tree_shadows` в игре.
#[derive(Component)]
struct ShadowLayer;

/// Подписи клеток и рядов. Живут отдельно от крон: от ручек они не зависят.
#[derive(Component)]
struct Caption;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe tree gallery — все варианты крон".to_string(),
                        resolution: (WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    level: bevy::log::Level::WARN,
                    filter: "warn,qwe=warn".to_string(),
                    ..default()
                }),
        )
        .add_plugins(PanCameraPlugin)
        // киты панели игры — кнопки, ползунки, тема. Заодно и шрифт: во
        // встроенном `default_font` кириллицы нет, и подписи выходят
        // квадратиками, а feathers несёт в себе Fira Sans, на котором написаны
        // панели игры
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<Ground>()
        .init_resource::<Show>()
        .init_resource::<Tuning>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(Startup, (spawn_camera, spawn_labels, spawn_panel))
        .add_systems(
            Update,
            (
                cycle_ground.run_if(input_just_pressed(KeyCode::KeyG)),
                toggle_shadows.run_if(input_just_pressed(KeyCode::KeyH)),
                toggle_captions.run_if(input_just_pressed(KeyCode::KeyL)),
                zoom_to_cursor,
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает
                // ту же сборку, что потом идёт на каждую правку ручки
                rebuild_crowns.run_if(resource_changed::<Tuning>),
                (sync_param_rows, sync_reset_button).run_if(resource_changed::<Tuning>),
                apply_show.run_if(resource_changed::<Show>),
            ),
        )
        .run();
}

/// Ширина и высота сетки в мировых единицах — по ним ставится камера.
fn grid_size() -> Vec2 {
    Vec2::new(
        TREE_VARIANTS as f32 * CELL_X,
        TreeShape::CONCRETE.len() as f32 * CELL_Y,
    )
}

/// Полоса окна, занятая панелью: отступ от края экрана, сама панель и такой
/// же зазор справа от неё.
fn panel_span() -> f32 {
    PANEL_WIDTH_PX + 2.0 * UI_SCREEN_EDGE_PX_OFFSET
}

/// Экранная ширина, оставшаяся витрине от панели.
fn viewport_width() -> f32 {
    WINDOW_WIDTH - panel_span()
}

fn spawn_camera(mut commands: Commands) {
    let size = grid_size();
    // масштаб = мировых единиц на пиксель, считается по свободной от панели
    // части окна: иначе левые колонки витрины стоят под панелью
    let zoom = size.x * VIEW_MARGIN / viewport_width();
    let grid_centre = Vec2::new(size.x / 2.0 - CELL_X / 2.0, -size.y / 2.0 + CELL_Y / 2.0);
    // Свободная часть окна лежит правее центра экрана ровно на полширины
    // занятой панелью полосы — значит камера едет ВЛЕВО на столько же, и мир
    // на экране уходит вправо, из-под панели. Знак тут единственное, что
    // отличает «сетка по центру свободного места» от «первая колонка и все
    // названия рядов спрятаны за панелью».
    let centre = grid_centre - Vec2::new(panel_span() / 2.0 * zoom, 0.0);
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(centre.extend(0.0)).with_scale(Vec3::splat(zoom)),
        Msaa::Off,
        PanCamera {
            zoom_factor: zoom,
            min_zoom: zoom / 20.0,
            max_zoom: zoom * 4.0,
            // колесо ведёт `zoom_to_cursor`: линейный зум самого PanCamera на
            // крупном плане неуправляем
            zoom_speed: 0.0,
            key_zoom_in: None,
            key_zoom_out: None,
            // витрина плоская, поворот камеры тут только мешает
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            // шаг панорамы задан в мировых единицах, а сцена мелкая
            pan_speed: 8.0,
            ..default()
        },
    ));
}

fn cell(row: usize, variant: usize) -> Vec2 {
    Vec2::new(variant as f32 * CELL_X, -(row as f32) * CELL_Y)
}

/// Названия рядов и номера клеток. От ручек не зависят, поэтому спавнятся один
/// раз и переживают пересборку крон.
fn spawn_labels(mut commands: Commands, assets: Res<AssetServer>) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    for (row, shape) in TreeShape::CONCRETE.into_iter().enumerate() {
        let y = cell(row, 0).y;
        commands.spawn((
            Caption,
            Text2d::new(shape.label()),
            label_font(&font, ROW_LABEL_FONT),
            TextColor(LABEL_COLOR),
            // прижат правым краем к колонке #0: по центру клетки название ряда
            // наползало бы на первую крону
            Anchor::CENTER_RIGHT,
            Transform::from_xyz(-CELL_X * 0.36, y, 2.0).with_scale(Vec3::splat(TEXT_SCALE)),
        ));
        for variant in 0..TREE_VARIANTS {
            let position = cell(row, variant);
            commands.spawn((
                Caption,
                Text2d::new(format!("{} #{variant}", shape.label())),
                label_font(&font, CAPTION_FONT),
                TextColor(LABEL_COLOR),
                Transform::from_xyz(position.x, position.y + CAPTION_Y, 2.0)
                    .with_scale(Vec3::splat(TEXT_SCALE)),
            ));
        }
    }
}

/// Пересборка сетки под текущие ручки: деспавн прежних крон и слоя теней,
/// сборка новых тем же вызовом, которым их строит игра.
fn rebuild_crowns(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    tuning: Res<Tuning>,
    show: Res<Show>,
    existing: Query<Entity, With<CrownTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // цвета листвы и чернил — игровые: витрина показывает геометрию, а палитра
    // и так крутится в панели Trees самой игры
    let style = TreeStyle {
        variance: tuning.variance,
        ..default()
    };
    let tints: Vec<Handle<ColorMaterial>> = tint_factors(tuning.variance)
        .iter()
        .map(|&factor| materials.add(Color::srgb(factor, factor, factor)))
        .collect();
    let mut shadows = MeshBuilder::default();

    for (row, shape) in TreeShape::CONCRETE.into_iter().enumerate() {
        for variant in 0..TREE_VARIANTS {
            let position = cell(row, variant);
            let built = crown_variant(shape, variant, &style, &tuning.crown);
            shadows.push_template(&built.shadow, position, 1.0);
            commands.spawn((
                CrownTag,
                Mesh2d(meshes.add(built.crown)),
                // тот же выбор оттенка, что в игре: по индексу дерева
                MeshMaterial2d(tints[(variant * 7) % tints.len()].clone()),
                Transform::from_translation(position.extend(1.0)),
                Name::new(format!("{} #{variant}", shape.label())),
            ));
        }
    }

    commands.spawn((
        CrownTag,
        ShadowLayer,
        Mesh2d(meshes.add(shadows.build())),
        MeshMaterial2d(materials.add(SHADOW_COLOR)),
        Transform::from_xyz(0.0, 0.0, 0.5),
        visibility(show.shadows),
        Name::new("tree_shadows"),
    ));
}

/// Квантованные множители яркости — копия `TreeStyle::tint_factors`, которая в
/// игре приватна. При `variance == 0` все пять равны единице, и витрина стоит
/// одинаково зелёной.
fn tint_factors(variance: f32) -> [f32; 5] {
    [-1.0, -0.5, 0.0, 0.5, 1.0].map(|bell: f32| 2.0_f32.powf(variance * bell))
}

/// Подпись шрифтом панелей игры: во встроенном шрифте bevy кириллицы нет.
fn label_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn visibility(shown: bool) -> Visibility {
    if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

/// Зум колесом к точке под курсором — перенос `camera.rs::zoom_to_cursor`:
/// шаг **умножает** масштаб, поэтому на любом приближении щелчок колеса
/// меняет картинку одинаково, и мировая точка под курсором остаётся на месте.
fn zoom_to_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    camera: Single<(&mut Transform, &mut PanCamera), With<Camera2d>>,
) {
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
    };
    if lines == 0.0 {
        return;
    }
    let (mut transform, mut controller) = camera.into_inner();
    let old_zoom = controller.zoom_factor;
    let new_zoom =
        (old_zoom * ZOOM_STEP.powf(-lines)).clamp(controller.min_zoom, controller.max_zoom);
    if new_zoom == old_zoom {
        return;
    }
    if let Some(cursor) = window.cursor_position() {
        let offset = cursor_offset(&window, cursor);
        let world_under_cursor = transform.translation.truncate() + offset * old_zoom;
        transform.translation =
            (world_under_cursor - offset * new_zoom).extend(transform.translation.z);
    }
    controller.zoom_factor = new_zoom;
    transform.scale = Vec3::splat(new_zoom);
}

fn cycle_ground(mut ground: ResMut<Ground>, mut clear: ResMut<ClearColor>) {
    *ground = ground.next();
    clear.0 = ground.color();
}

fn toggle_shadows(mut show: ResMut<Show>) {
    show.shadows = !show.shadows;
}

fn toggle_captions(mut show: ResMut<Show>) {
    show.captions = !show.captions;
}

/// Тумблеры применяются одной системой: сущности пересобираются, и держать
/// состояние показа в них самих нельзя.
fn apply_show(
    show: Res<Show>,
    mut shadows: Query<&mut Visibility, (With<ShadowLayer>, Without<Caption>)>,
    mut captions: Query<&mut Visibility, With<Caption>>,
) {
    for mut value in &mut shadows {
        *value = visibility(show.shadows);
    }
    for mut value in &mut captions {
        *value = visibility(show.captions);
    }
}
