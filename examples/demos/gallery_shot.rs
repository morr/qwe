//! Автоснимок витрины — общий на все витрины `examples/demos/`.
//!
//! Вынесен из витрин, а не скопирован в каждую, потому что три числа кадров и
//! съёмка в текстуру — знание, добытое отладкой (коммиты 21853a3 и 4cbff7ff), и
//! поправленное в одной копии в другой осталось бы прежним.
//!
//! Снимается **не окно, а текстура**: на кадре [`SHOT_RETARGET_FRAME`] камера
//! витрины переводится рисовать в картинку размером с окно в логических
//! пикселях, умноженных на масштаб снимка. Снимок поверхности окна macOS отдаёт
//! чёрным, когда окно перекрыто или экран заблокирован — окно поднимали, но от
//! блокировки это не спасало, и ночной прогон снимал одну черноту
//! (`qwe::dev::on_offscreen_shot` у игры — тот же приём). Панели в кадр не
//! попадают: UI рисуется в окно.
//!
//! **Масштаб** — переменная `<VAR>_SCALE` рядом с путём (`ROADS_SHOT_SCALE=2`):
//! картинка во столько раз больше по каждой стороне, а `scale_factor` цели тот
//! же, так что логический размер вида — окно, и в кадре то же, что при 1,
//! только чётче. Чёрным такой кадр выходил не из-за блокировки экрана, а из-за
//! снимка: `Screenshot::image(handle)` строит цель с `scale_factor: 1.0`, а
//! `prepare_screenshots` подменяет выходное вложение по цели **целиком**
//! (`ImageRenderTarget` сравнивается вместе с масштабом). При масштабе 2 камера
//! и снимок называли разные цели, камера рисовала мимо подмены, и в файл уходила
//! пустая текстура снимка. Поэтому снимок заказывается той же
//! `ImageRenderTarget`, что стоит на камере.
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

/// Больше этого масштаба картинка у 1600×1000-окна выходит за 16 тыс. px по
/// стороне — предел текстуры wgpu на большинстве GPU.
const SHOT_SCALE_MAX: f32 = 8.0;

/// Куда класть автоснимок и во сколько раз он крупнее логического окна.
/// Ресурс, а не чтение окружения из системы: гейт расписания и сама система
/// читали `std::env::var` по два раза на кадр всю жизнь процесса.
#[derive(Resource)]
pub(crate) struct ShotRequest {
    path: String,
    scale: f32,
}

/// Цель, в которую камера рисует для снимка, — вместе с масштабом: снимок
/// обязан назвать ту же самую цель, см. шапку модуля.
#[derive(Resource)]
pub(crate) struct ShotTarget(ImageRenderTarget);

/// Заказан ли снимок переменной окружения витрины (`var` — путь, `var_SCALE` —
/// масштаб, по умолчанию 1). Ставится в `Startup`.
pub(crate) fn request_shot(var: &'static str) -> impl Fn(Commands) {
    move |mut commands: Commands| {
        if let Ok(path) = std::env::var(var) {
            let scale = shot_scale(std::env::var(format!("{var}_SCALE")).ok().as_deref());
            commands.insert_resource(ShotRequest { path, scale });
        }
    }
}

/// Масштаб снимка из значения переменной: нет её, не число или не больше
/// нуля — 1, сверху — [`SHOT_SCALE_MAX`].
fn shot_scale(value: Option<&str>) -> f32 {
    value
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .map_or(1.0, |s| s.min(SHOT_SCALE_MAX))
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
        // логический размер окна × масштаб снимка. Не физический размер окна:
        // масштаб Retina здесь ни при чём, и снимок не должен от него зависеть
        let side = |logical: f32| ((logical * request.scale).round() as u32).max(1);
        let mut image = Image::new_fill(
            Extent3d {
                width: side(window.width()),
                height: side(window.height()),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        // цель рендера и источник копирования для `Screenshot`
        image.texture_descriptor.usage |=
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
        // `scale_factor` цели делит её физический размер обратно до окна:
        // проекция камеры считается по логическому размеру цели, так что в
        // кадр попадает ровно то, что в окне, а пикселей в `scale²` раз больше
        let target = ImageRenderTarget {
            handle: images.add(image),
            scale_factor: request.scale,
        };
        for camera in &cameras {
            commands
                .entity(camera)
                .insert(RenderTarget::Image(target.clone()));
        }
        commands.insert_resource(ShotTarget(target));
    }
    if *frame == SHOT_FRAME
        && let Some(target) = target
    {
        // не `Screenshot::image(handle)`: та строит цель с масштабом 1 и при
        // масштабе снимка ≠ 1 не совпадает с целью камеры — кадр чёрный
        commands
            .spawn(Screenshot(RenderTarget::Image(target.0.clone())))
            .observe(save_to_disk(request.path.clone()));
    }
    if *frame == SHOT_EXIT_FRAME {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_scale_reads_the_variable_and_falls_back_to_one() {
        assert_eq!(shot_scale(None), 1.0);
        assert_eq!(shot_scale(Some("2")), 2.0);
        assert_eq!(shot_scale(Some(" 1.5 ")), 1.5);
        assert_eq!(shot_scale(Some("x")), 1.0);
        assert_eq!(shot_scale(Some("0")), 1.0);
        assert_eq!(shot_scale(Some("-2")), 1.0);
        assert_eq!(shot_scale(Some("100")), SHOT_SCALE_MAX);
    }
}
