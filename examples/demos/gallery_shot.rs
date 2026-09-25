//! Автоснимок витрины — общий на все витрины `examples/demos/`.
//!
//! Вынесен из витрин, а не скопирован в каждую, потому что три числа кадров и
//! подъём окна — знание, добытое отладкой (коммит 21853a3), и поправленное в
//! одной копии в другой осталось бы прежним.
//!
//! Снимается **не окно, а текстура**: на кадре [`SHOT_RETARGET_FRAME`] камера
//! витрины переводится рисовать в картинку размером с окно в логических
//! пикселях. Снимок поверхности окна macOS отдаёт чёрным, когда окно перекрыто
//! или экран заблокирован — окно поднимали, но от блокировки это не спасало,
//! и ночной прогон снимал одну черноту (`qwe::dev::on_offscreen_shot` у игры —
//! тот же приём). Панели в кадр не попадают: UI рисуется в окно.
//!
//! Числа: первый кадр уходит на сборку сетки (она идёт в `Update`, а не в
//! `Startup`), дальше нужно дать шейдеру и шрифту доехать до цели; после
//! снимка — столько же, потому что на диск его пишет наблюдатель, а не эта
//! система.

use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;

const SHOT_RETARGET_FRAME: u32 = 5;
const SHOT_FRAME: u32 = 30;
const SHOT_EXIT_FRAME: u32 = SHOT_FRAME + 30;

/// Куда класть автоснимок. Ресурс, а не чтение окружения из системы: гейт
/// расписания и сама система читали `std::env::var` по два раза на кадр всю
/// жизнь процесса.
#[derive(Resource)]
pub(crate) struct ShotRequest(String);

/// Картинка, в которую камера рисует для снимка.
#[derive(Resource)]
pub(crate) struct ShotTarget(Handle<Image>);

/// Заказан ли снимок переменной окружения витрины. Ставится в `Startup`.
pub(crate) fn request_shot(var: &'static str) -> impl Fn(Commands) {
    move |mut commands: Commands| {
        if let Ok(path) = std::env::var(var) {
            commands.insert_resource(ShotRequest(path));
        }
    }
}

/// Снимок витрины и выход — единственный способ посмотреть на неё из сессии:
/// BRP у примера нет.
#[allow(clippy::too_many_arguments)]
pub(crate) fn auto_shot(
    mut commands: Commands,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    cameras: Query<Entity, With<Camera2d>>,
    target: Option<Res<ShotTarget>>,
    request: Res<ShotRequest>,
) {
    *frame += 1;
    if *frame == SHOT_RETARGET_FRAME {
        let mut image = Image::new_fill(
            Extent3d {
                width: (window.width() as u32).max(1),
                height: (window.height() as u32).max(1),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        // цель рендера и источник копирования для `Screenshot::image`
        image.texture_descriptor.usage |=
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
        let handle = images.add(image);
        // масштаб 1 при логическом размере окна — тот же кадр, что в окне.
        // Физический размер с масштабом окна (2 на Retina) дал бы вдвое
        // больше пикселей, но при заблокированном экране он снимается
        // чёрным — проверено; с масштабом 1 и физическим размером кадр
        // вдвое мельче окна
        let target = ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: 1.0,
        };
        for camera in &cameras {
            commands
                .entity(camera)
                .insert(RenderTarget::Image(target.clone()));
        }
        commands.insert_resource(ShotTarget(handle));
    }
    if *frame == SHOT_FRAME
        && let Some(target) = target
    {
        commands
            .spawn(Screenshot::image(target.0.clone()))
            .observe(save_to_disk(request.0.clone()));
    }
    if *frame == SHOT_EXIT_FRAME {
        exit.write(AppExit::Success);
    }
}
