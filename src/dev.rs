//! Инструменты для отладочных сессий (skill `live-app`): скриншот в файл по
//! клавише F12 или BRP-событию `TakeScreenshotEvent`, и **закадровый** снимок
//! ([`OffscreenShotEvent`]) — тот же кадр, но мимо окна.
//!
//! Закадровый нужен потому, что обычный снимает **поверхность окна**: на
//! заблокированном экране, под другим окном или на спящем дисплее macOS отдаёт
//! чёрный кадр, и проверить картинку нечем. Здесь же кадр рисуется во
//! внеэкранную текстуру своей камерой, и оконный сервер к этому не причастен.
//! Заодно снимок получает **свою рамку**: точку, зум и размер, не трогая
//! камеру пользователя.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera_controller::pan_camera::PanCamera;
use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::grid::world_to_tile;
use crate::loading::AppState;
use crate::movement::Movable;
use crate::settings::unit_z;

const SCREENSHOT_PATH: &str = "screenshot.png";
const OFFSCREEN_PATH: &str = "offscreen.png";
/// Размер закадрового кадра по умолчанию, px. Длинная сторона ровно 1568 —
/// граница, за которой картинку всё равно ужмут при чтении, так что кадр
/// доезжает до глаз без пережатия.
const OFFSCREEN_SIZE: UVec2 = UVec2::new(1568, 980);
/// Сколько кадров дать камере отрисоваться, прежде чем снимать. Один кадр на
/// то, чтобы цель вообще появилась в графе рендера, второй — на сам кадр;
/// снимать в тот же кадр, в который камера создана, значит снять пустоту.
const WARMUP_FRAMES: u32 = 2;
/// Нижняя граница стороны кадра, px: `size` приходит из BRP, а текстура с
/// нулевой стороной — ошибка валидации wgpu, то есть падение по чужому вводу.
/// 16 — заведомо безопасный минимум, замером он не выбирался.
const OFFSCREEN_MIN_SIDE: u32 = 16;

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct TakeScreenshotEvent;

/// Закадровый снимок: кадр рисуется во внеэкранную текстуру и кладётся в файл,
/// минуя окно. Все поля необязательны — пустое событие снимает `offscreen.png`
/// [`OFFSCREEN_SIZE`] (1568 × 980) с текущей позиции камеры и текущим зумом:
///
/// ```text
/// brp event OffscreenShotEvent '{"at":[2300,1900],"zoom":0.4,"path":"gsk.png"}'
/// ```
// `reflect(Default)` обязателен: BRP собирает событие из **частичного**
// JSON (`{"at": [...]}`), и недостающие поля берутся из `Default`
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event, Default)]
pub struct OffscreenShotEvent {
    /// Куда смотреть, м. `None` — туда же, куда смотрит камера пользователя.
    pub at: Option<Vec2>,
    /// Метров на условный пиксель кадра. `None` — как у камеры.
    pub zoom: Option<f32>,
    /// Размер кадра, px.
    pub size: Option<UVec2>,
    /// Куда положить файл; путь относительно рабочего каталога.
    pub path: Option<String>,
}

/// Камера закадрового снимка: живёт ровно столько кадров, сколько нужно, чтобы
/// её цель успела отрисоваться.
#[derive(Component)]
struct OffscreenCamera {
    target: Handle<Image>,
    path: String,
    frames: u32,
}

/// Тестовый агент навигации: спавнится в `from` и идёт в `to` (метры).
/// Триггерится по BRP: `brp event SpawnTestWalkerEvent '{"from":[..],"to":[..]}'`.
#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct SpawnTestWalkerEvent {
    pub from: Vec2,
    pub to: Vec2,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct TestWalker;

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin::default(),
        ))
        .register_type::<TakeScreenshotEvent>()
        .register_type::<OffscreenShotEvent>()
        .register_type::<SpawnTestWalkerEvent>()
        .register_type::<TestWalker>()
        .add_observer(on_take_screenshot)
        .add_observer(on_offscreen_shot)
        .add_observer(on_spawn_test_walker)
        .add_systems(Update, capture_offscreen)
        .add_systems(
            Update,
            trigger_screenshot.run_if(bevy::input::common_conditions::input_just_pressed(
                KeyCode::F12,
            )),
        );
    }
}

fn on_spawn_test_walker(event: On<SpawnTestWalkerEvent>, mut commands: Commands) {
    let mut movable = Movable::new(4.0);
    let entity = commands
        .spawn((
            Sprite {
                color: Color::srgb(0.1, 0.1, 0.9),
                custom_size: Some(Vec2::splat(2.0)),
                ..default()
            },
            Transform::from_translation(event.from.extend(unit_z(event.from.y))),
            TestWalker,
            // ходок срочен: он и раньше им был — правило `!is_human` считало
            // срочным всё, что не человек, а вида у ходока нет вовсе. Без
            // маркера его заявка попала бы в очередь гуляющих и ждала камеру
            crate::movement::UrgentPath,
            DespawnOnExit(AppState::Playing),
            Name::new("test_walker"),
        ))
        .id();
    movable.to_pathfinding(
        entity,
        world_to_tile(event.from),
        world_to_tile(event.to),
        &mut commands,
    );
    commands.entity(entity).insert(movable);
}

fn trigger_screenshot(mut commands: Commands) {
    commands.trigger(TakeScreenshotEvent);
}

fn on_take_screenshot(_event: On<TakeScreenshotEvent>, mut commands: Commands) {
    info!("saving screenshot to {SCREENSHOT_PATH}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(SCREENSHOT_PATH));
}

/// Своя камера во внеэкранную текстуру. Пост-обработка у неё та же, что у
/// пользовательской (`post::camera_post_process`), иначе снимок показывал бы
/// не то, что видно на экране: без bloom портал, ореолы демонов и искры душ
/// теряют свечение. Виньетка (`post.rs`) в кадр не попадает — она UI-нода и
/// живёт на камере панелей.
fn on_offscreen_shot(
    event: On<OffscreenShotEvent>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    // не просто `With<Camera2d>`: закадровая камера, которую этот же обсервер
    // и спавнит, тоже `Camera2d`, и пока она жива (три кадра) `Single` матчил
    // бы две сущности. Обсервер при этом **молча** пропускается — второе
    // событие подряд не оставляло ни файла, ни строки в логе
    camera: Single<(&Transform, &Projection), (With<Camera2d>, With<PanCamera>)>,
) {
    let (transform, projection) = *camera;
    let size = event
        .size
        .unwrap_or(OFFSCREEN_SIZE)
        .max(UVec2::splat(OFFSCREEN_MIN_SIDE));
    let at = event.at.unwrap_or(transform.translation.truncate());
    let zoom = event.zoom.unwrap_or(transform.scale.x).max(f32::EPSILON);
    let path = event.path.clone().unwrap_or(OFFSCREEN_PATH.to_string());

    let mut image = Image::new_fill(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // цель рендера и источник копирования: первое — чтобы в неё рисовать,
    // второе — чтобы `Screenshot::image` смог её прочитать
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    let target = images.add(image);

    info!("offscreen shot: {size} at {at} zoom {zoom} -> {path}");
    commands.spawn((
        Camera2d,
        Camera {
            // раньше пользовательской в общем порядке отрисовки. Это не
            // защита от предупреждения о неоднозначности: `sort_cameras`
            // (bevy_render) ключует его парой (order, target), а цели здесь
            // разные — окно и текстура, так что одинаковый порядок движок бы
            // и не заметил
            order: -1,
            ..default()
        },
        // в 0.19 цель — самостоятельный компонент, а не поле `Camera`
        RenderTarget::Image(target.clone().into()),
        projection.clone(),
        Transform::from_translation(at.extend(0.0)).with_scale(Vec3::splat(zoom)),
        Msaa::Off,
        crate::post::camera_post_process(),
        OffscreenCamera {
            target,
            path,
            frames: 0,
        },
        Name::new("offscreen_camera"),
    ));
}

/// Дать камере отрисоваться [`WARMUP_FRAMES`] кадров, снять её цель и **на
/// следующем кадре** убрать камеру.
///
/// Снять и убрать в один кадр нельзя, и это не осторожность, а устройство
/// движка: `prepare_screenshots` подменяет выходное вложение цели своей
/// текстурой на тот кадр, в котором снимок запрошен, — то есть рисует в неё
/// **сама камера**, и если её в этом кадре уже нет, в файл уходит чистый ноль.
fn capture_offscreen(mut commands: Commands, mut cameras: Query<(Entity, &mut OffscreenCamera)>) {
    for (entity, mut shot) in &mut cameras {
        shot.frames += 1;
        match shot.frames.cmp(&WARMUP_FRAMES) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                commands
                    .spawn(Screenshot::image(shot.target.clone()))
                    .observe(save_to_disk(shot.path.clone()));
            }
            std::cmp::Ordering::Greater => commands.entity(entity).despawn(),
        }
    }
}
