//! Витрина крови: все лужи и все веера брызг, которые умеет рисовать игра,
//! разом на одном экране — и поле тел под ними, собранное ровно так, как его
//! собирает убийство.
//!
//! Кровь под трупом — две дочерние сущности тела: **лужа** (`BloodPool`,
//! растекается на глазах) и **брызги** (`BloodSpatter`, ложатся разом). Форм у
//! каждой по десятку ([`blood::POOLS`], [`blood::SPATTERS`]), они разыграны из
//! номера варианта и лежат в атласе силуэтов; какая достанется телу, а также
//! поворот, размер и тон — решают биты его `Entity`
//! (`human::look::blood_look`). Отсюда три полосы витрины:
//!
//! - **Лужи** — по глифу в клетке, без разброса ([`BloodLook::plain`]): видно
//!   саму форму, а не то, как её повернуло;
//! - **Брызги** — то же для вееров;
//! - **Тела** — [`FIELD_ROWS`] × [`COLS`] трупов. Клетка здесь
//!   собирается **настоящим** [`to_corpse`]: витрина спавнит человека
//!   (диск, одежда) и убивает его, а не рисует свою копию трупа. Поэтому
//!   всё, что видно в поле, — это то, что увидит игрок, вплоть до роста лужи
//!   на первых секундах после `R`.
//!
//! Пример не трогает конфиг игры и не поднимает мир: ни `HumanPlugin` (он
//! расселил бы 20 000 пешек), ни `PrefsPlugin`, ни `MapPlugin`. Из игры взяты
//! атлас силуэтов (`SilhouettePlugin`), переход в труп и система растекания.
//!
//! ```text
//! cargo run --example blood_gallery
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `R` | перебросить поле тел (и заново пустить растекание) |
//! | `G` | подложка: мостовая → трава → лес |
//! | `P` | лужи вкл/выкл |
//! | `S` | брызги вкл/выкл |
//! | `L` | подписи вкл/выкл |
//! | `F12` | снимок в `blood_gallery.png` |
//!
//! `BLOOD_GALLERY_SHOT=<путь> cargo run --example blood_gallery` — снять
//! витрину самой и закрыться.

use bevy::app::AppExit;
use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::sprite::Anchor;
use bevy::window::PrimaryWindow;
use qwe::camera::{hovering_ui, zoom_to_cursor};
use qwe::human::{
    Attire, BloodLook, BloodPool, BloodSpatter, CORPSE_SPAN, CorpsePose, blood_pool, blood_spatter,
    spread_blood, to_corpse,
};
use qwe::map::{GROUND_COLOR, PARK_COLOR, WOOD_COLOR};
use qwe::settings::{HUMAN_MIN_PX, HUMAN_SIZE, Z_CORPSE};
use qwe::silhouette::{Glyph, Silhouette, SilhouettePlugin, Silhouettes, blood};

const WINDOW_WIDTH: f32 = 1500.0;
const WINDOW_HEIGHT: f32 = 900.0;

/// Шаг сетки, м. Ячейка веера брызг — [`CORPSE_SPAN`] × 1.35; шаг заметно
/// шире её, чтобы соседние веера не сливались в один.
const CELL: f32 = CORPSE_SPAN * 1.57;
/// Ширина витрины в клетках — по самому длинному семейству крови: полосы
/// атласа и поле тел тогда одной ширины.
const COLS: usize = if blood::POOLS > blood::SPATTERS {
    blood::POOLS
} else {
    blood::SPATTERS
};
/// Сколько рядов тел в поле.
const FIELD_ROWS: usize = 4;
/// Зазор между полосами атласа и полем тел, в клетках.
const BAND_GAP: f32 = 0.55;
/// Подпись под клеткой, м от её центра.
const CAPTION_Y: f32 = -CELL * 0.42;
/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в метрах. Подобран под стартовый зум.
const TEXT_SCALE: f32 = 0.014;
const CAPTION_FONT: f32 = 22.0;
const ROW_LABEL_FONT: f32 = 34.0;
/// Подписи — тёмно-серые: на всех трёх подложках читаются и с кровью не
/// спорят.
const LABEL_COLOR: Color = Color::srgb(0.16, 0.17, 0.20);
/// Поля вокруг сетки при стартовом зуме, доля её ширины.
const VIEW_MARGIN: f32 = 1.18;
/// Куда `F12` кладёт снимок витрины.
const SHOT_PATH: &str = "blood_gallery.png";
/// Переменная окружения автоснимка: витрина снимает себя сама и закрывается —
/// так её видно не только человеку за экраном.
const SHOT_ENV: &str = "BLOOD_GALLERY_SHOT";
/// Когда снимать и когда закрываться, секунд от старта. Первое — с запасом на
/// растекание луж (в игре оно длится 1,6 с), второе — на запись файла.
const SHOT_AT: f32 = 2.5;
const SHOT_QUIT_AT: f32 = 4.0;
/// Поза, под которую положены клетки полос атласа: одна на все, чтобы
/// смещение лужи к груди сдвинуло полосу целиком, а не разъехалось по клеткам.
const BAND_POSE: CorpsePose = CorpsePose {
    glyph: Glyph::Corpse(0),
    heading: 0.0,
    flip: false,
};

/// Подложка витрины — те же цвета земли, что и на карте.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Ground {
    #[default]
    Pavement,
    Park,
    Wood,
}

impl Ground {
    fn color(self) -> Color {
        match self {
            Self::Pavement => GROUND_COLOR,
            Self::Park => PARK_COLOR,
            Self::Wood => WOOD_COLOR,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Pavement => Self::Park,
            Self::Park => Self::Wood,
            Self::Wood => Self::Pavement,
        }
    }
}

/// Что показано. Ресурсом, а не состоянием сущностей: поле пересобирается по
/// `R`, и тумблеры сбрасывались бы на каждой пересборке.
#[derive(Resource)]
struct Show {
    pools: bool,
    spatters: bool,
    captions: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            pools: true,
            spatters: true,
            captions: true,
        }
    }
}

/// Номер броска: `R` увеличивает его, и поле пересобирается. Тела получают
/// новые `Entity`, а с ними — новую кровь.
#[derive(Resource, Default)]
struct Roll(u32);

/// Всё, что пересобирается броском.
#[derive(Component)]
struct GalleryTag;

/// Подписи. От броска не зависят: спавнятся один раз.
#[derive(Component)]
struct Caption;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe blood gallery — лужи, брызги и тела".to_string(),
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
        // атлас силуэтов — из игры: витрина показывает те глифы, которыми
        // рисуется город, а не свою копию
        .add_plugins(SilhouettePlugin)
        // киты панели игры нужны здесь ради одного: во встроенном `default_font`
        // нет кириллицы, а feathers несёт Fira Sans, которым написаны панели
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<Ground>()
        .init_resource::<Show>()
        .init_resource::<Roll>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(Startup, (spawn_camera, spawn_labels))
        .add_systems(
            Update,
            (
                cycle_ground.run_if(input_just_pressed(KeyCode::KeyG)),
                reroll.run_if(input_just_pressed(KeyCode::KeyR)),
                toggle_pools.run_if(input_just_pressed(KeyCode::KeyP)),
                toggle_spatters.run_if(input_just_pressed(KeyCode::KeyS)),
                toggle_captions.run_if(input_just_pressed(KeyCode::KeyL)),
                shoot.run_if(input_just_pressed(KeyCode::F12)),
                auto_shot,
                zoom_to_cursor.run_if(not(hovering_ui)),
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает
                // ту же сборку, что потом идёт на каждый бросок
                rebuild.run_if(resource_changed::<Roll>),
                apply_show.run_if(resource_changed::<Show>),
                // растекание — система игры, без неё лужи остались бы
                // родившимися, то есть втрое меньше себя
                spread_blood,
            ),
        )
        .run();
}

/// Ширина и высота сетки в метрах — по ним ставится камера.
fn grid_size() -> Vec2 {
    Vec2::new(
        COLS as f32 * CELL,
        (2.0 + BAND_GAP + FIELD_ROWS as f32) * CELL,
    )
}

/// Клетка полосы атласа: полосы стоят сверху, лужи над брызгами.
fn band_cell(row: usize, column: usize) -> Vec2 {
    Vec2::new(column as f32 * CELL, -(row as f32) * CELL)
}

/// Клетка поля тел — под полосами, через [`BAND_GAP`].
fn field_cell(row: usize, column: usize) -> Vec2 {
    Vec2::new(
        column as f32 * CELL,
        -(1.0 + BAND_GAP + row as f32) * CELL - CELL,
    )
}

fn spawn_camera(mut commands: Commands, window: Single<&Window, With<PrimaryWindow>>) {
    let size = grid_size();
    // масштаб — метров на логический пиксель; кадрирование стартовое и
    // единственное, дальше камера принадлежит пользователю
    let zoom = size.x * VIEW_MARGIN / window.width();
    let centre = Vec2::new(size.x / 2.0 - CELL / 2.0, -size.y / 2.0 + CELL / 2.0);
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
            min_zoom: zoom / 30.0,
            max_zoom: zoom * 4.0,
            // колесо ведёт `camera::zoom_to_cursor`: линейный зум самого
            // PanCamera на крупном плане неуправляем
            zoom_speed: 0.0,
            key_zoom_in: None,
            key_zoom_out: None,
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            pan_speed: 12.0,
            ..default()
        },
    ));
}

/// Названия полос и номера клеток. Броска не переживать им незачем — они от
/// него не зависят.
fn spawn_labels(mut commands: Commands, assets: Res<AssetServer>) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    let mut row_label = |text: &str, at: Vec2| {
        commands.spawn((
            Caption,
            Text2d::new(text.to_string()),
            label_font(&font, ROW_LABEL_FONT),
            TextColor(LABEL_COLOR),
            // прижата правым краем к колонке #0: по центру клетки название
            // полосы наползало бы на первое пятно
            Anchor::CENTER_RIGHT,
            Transform::from_xyz(at.x - CELL * 0.38, at.y, 20.0).with_scale(Vec3::splat(TEXT_SCALE)),
        ));
    };
    row_label("Лужи", band_cell(0, 0));
    row_label("Брызги", band_cell(1, 0));
    row_label("Тела", field_cell(0, 0));

    let mut caption = |text: String, at: Vec2| {
        commands.spawn((
            Caption,
            Text2d::new(text),
            label_font(&font, CAPTION_FONT),
            TextColor(LABEL_COLOR),
            Transform::from_xyz(at.x, at.y + CAPTION_Y, 20.0).with_scale(Vec3::splat(TEXT_SCALE)),
        ));
    };
    for variant in 0..blood::POOLS {
        caption(format!("Лужа #{variant}"), band_cell(0, variant));
    }
    for variant in 0..blood::SPATTERS {
        caption(format!("Брызги #{variant}"), band_cell(1, variant));
    }
}

/// Пересборка витрины: полосы атласа и поле тел.
fn rebuild(
    mut commands: Commands,
    silhouettes: Res<Silhouettes>,
    existing: Query<Entity, With<GalleryTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    // полосы атласа — без разброса: сравнивать надо формы, а не повороты
    for variant in 0..blood::POOLS {
        let look = BloodLook::plain(Glyph::pool(variant), Glyph::spatter(0));
        spawn_stain(
            &mut commands,
            &silhouettes,
            band_cell(0, variant),
            look,
            true,
        );
    }
    for variant in 0..blood::SPATTERS {
        let look = BloodLook::plain(Glyph::pool(0), Glyph::spatter(variant));
        spawn_stain(
            &mut commands,
            &silhouettes,
            band_cell(1, variant),
            look,
            false,
        );
    }

    for row in 0..FIELD_ROWS {
        for column in 0..COLS {
            spawn_corpse(
                &mut commands,
                &silhouettes,
                field_cell(row, column),
                row * COLS + column,
            );
        }
    }
}

/// Клетка полосы: держатель на месте клетки и под ним одно пятно — ровно та
/// же сущность, что висит на трупе.
fn spawn_stain(
    commands: &mut Commands,
    silhouettes: &Silhouettes,
    at: Vec2,
    look: BloodLook,
    pool: bool,
) {
    let mut cell = commands.spawn((
        GalleryTag,
        Transform::from_translation(at.extend(Z_CORPSE)),
        Visibility::default(),
        Name::new("stain cell"),
    ));
    if pool {
        cell.with_child(blood_pool(silhouettes, BAND_POSE, look));
    } else {
        cell.with_child(blood_spatter(silhouettes, BAND_POSE, look));
    }
}

/// Клетка поля: человек, которого тут же убивают. Труп собирает игра
/// ([`to_corpse`]) — витрине незачем знать, из чего он состоит.
fn spawn_corpse(commands: &mut Commands, silhouettes: &Silhouettes, at: Vec2, index: usize) {
    let attire = attire_of(index);
    let human = commands
        .spawn((
            GalleryTag,
            Attire(attire),
            silhouettes.sprite(Glyph::Disc, attire, Vec2::splat(HUMAN_SIZE)),
            Silhouette::new(Vec2::splat(HUMAN_SIZE), HUMAN_MIN_PX),
            Transform::from_translation(at.extend(0.0)),
            Name::new("body"),
        ))
        .id();
    to_corpse(commands, silhouettes, human);
}

/// Одежда клетки: та же холодная половина круга, что у города
/// (`human::look::roll_attire`), только разложенная ровно, а не жребием, —
/// поле обязано показывать разброс крови, а не разброс одежды.
fn attire_of(index: usize) -> Color {
    let step = 120.0 / (COLS * FIELD_ROWS) as f32;
    Color::hsl(170.0 + step * index as f32, 0.32, 0.45)
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

fn cycle_ground(mut ground: ResMut<Ground>, mut clear: ResMut<ClearColor>) {
    *ground = ground.next();
    clear.0 = ground.color();
}

fn reroll(mut roll: ResMut<Roll>) {
    roll.0 += 1;
}

fn toggle_pools(mut show: ResMut<Show>) {
    show.pools = !show.pools;
}

fn toggle_spatters(mut show: ResMut<Show>) {
    show.spatters = !show.spatters;
}

fn toggle_captions(mut show: ResMut<Show>) {
    show.captions = !show.captions;
}

fn shoot(commands: Commands) {
    save_shot(commands, SHOT_PATH.to_string());
}

/// Автоснимок: снимает по часам и закрывает окно. Без `BLOOD_GALLERY_SHOT`
/// ничего не делает — витрина остаётся у человека в руках.
fn auto_shot(
    time: Res<Time>,
    commands: Commands,
    mut elapsed: Local<f32>,
    mut shot: Local<bool>,
    mut quit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var(SHOT_ENV) else {
        return;
    };
    *elapsed += time.delta_secs();
    if !*shot && *elapsed >= SHOT_AT {
        *shot = true;
        save_shot(commands, path);
    } else if *elapsed >= SHOT_QUIT_AT {
        quit.write(AppExit::Success);
    }
}

fn save_shot(mut commands: Commands, path: String) {
    info!("saving the gallery to {path}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

/// Тумблеры применяются одной системой: сущности пересобираются броском, и
/// держать состояние показа в них самих нельзя.
fn apply_show(
    show: Res<Show>,
    mut pools: Query<&mut Visibility, With<BloodPool>>,
    mut spatters: Query<&mut Visibility, (With<BloodSpatter>, Without<BloodPool>)>,
    mut captions: Query<
        &mut Visibility,
        (With<Caption>, Without<BloodPool>, Without<BloodSpatter>),
    >,
) {
    for mut value in &mut pools {
        *value = visibility(show.pools);
    }
    for mut value in &mut spatters {
        *value = visibility(show.spatters);
    }
    for mut value in &mut captions {
        *value = visibility(show.captions);
    }
}
