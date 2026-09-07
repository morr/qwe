//! Зерно земли: один полупрозрачный спрайт на всю карту, замощённый бесшовной
//! плиткой шума, поверх всех заливок земли (земля, кварталы, парки, луга,
//! песок) и под водой и дорогами. Ровная заливка одного цвета на тысячи
//! метров читается как лист бумаги; лёгкая пятнистость в полтона — как
//! почва. Рисуется одним квадом с текстурой, так что кадру безразлична.
//!
//! Плитка бесшовная по построению: точка `(u, v)` плитки берётся на торе в
//! 4D-шуме — `(cos u, sin u, cos v, sin v)`, — и противоположные края плитки
//! совпадают точно, а не «почти». Две октавы: крупные пятна и мелкая рябь.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use noise::{NoiseFn, Simplex};

use crate::loading::AppState;
use crate::settings::{MAP_SIZE, Z_GROUND_GRAIN};

/// Сторона плитки в текселях и в метрах: тексель — метр. На обычном зуме
/// (метры на пиксель) плитка семплируется реже своего разрешения, и шум
/// достаточно гладкий, чтобы линейная фильтрация не мерцала при панораме.
const TILE_PX: u32 = 256;
const TILE_METRES: f32 = TILE_PX as f32;
/// Октавы шума: (длина волны, м; вес). Крупные пятна в полквартала и рябь.
const OCTAVES: [(f32, f32); 2] = [(48.0, 1.0), (14.0, 0.45)];
/// Сила зерна: альфа белого на светлых пятнах и чёрного на тёмных. Земля
/// светлая (~0.87), и белое на ней почти не видно — отсюда разница.
const LIGHT_ALPHA: f32 = 0.24;
const DARK_ALPHA: f32 = 0.09;
const SEED: u32 = 7;

/// Спавн спрайта зерна при входе в мир.
pub fn spawn_ground_grain(commands: &mut Commands, images: &mut Assets<Image>) {
    commands.spawn((
        Sprite {
            image: images.add(grain_tile()),
            custom_size: Some(MAP_SIZE),
            image_mode: SpriteImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: TILE_METRES / TILE_PX as f32,
            },
            ..default()
        },
        Transform::from_translation((MAP_SIZE / 2.0).extend(Z_GROUND_GRAIN)),
        DespawnOnExit(AppState::Playing),
        Name::new("ground_grain"),
    ));
}

/// Бесшовная плитка зерна, RGBA: белое с малой альфой на светлых пятнах,
/// чёрное — на тёмных.
fn grain_tile() -> Image {
    let noise = Simplex::new(SEED);
    let mut data = Vec::with_capacity((TILE_PX * TILE_PX * 4) as usize);
    for y in 0..TILE_PX {
        for x in 0..TILE_PX {
            let value = grain_at(&noise, x as f32 / TILE_PX as f32, y as f32 / TILE_PX as f32);
            let (grey, alpha) = if value >= 0.0 {
                (255, value * LIGHT_ALPHA)
            } else {
                (0, -value * DARK_ALPHA)
            };
            data.extend_from_slice(&[grey, grey, grey, (alpha.min(1.0) * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: TILE_PX,
            height: TILE_PX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Шум в точке плитки `(u, v) ∈ [0, 1)²`, в −1..1. Точка берётся на торе:
/// каждая координата плитки — угол окружности радиуса, подобранного так,
/// чтобы длина окружности равнялась стороне плитки в единицах длины волны.
fn grain_at(noise: &Simplex, u: f32, v: f32) -> f32 {
    use std::f64::consts::TAU;
    let (a, b) = (u as f64 * TAU, v as f64 * TAU);
    let mut total = 0.0;
    let mut weight = 0.0;
    for (wavelength, amplitude) in OCTAVES {
        let radius = (TILE_METRES / wavelength) as f64 / TAU;
        let sample = noise.get([
            radius * a.cos(),
            radius * a.sin(),
            radius * b.cos(),
            radius * b.sin(),
        ]);
        total += sample * amplitude as f64;
        weight += amplitude as f64;
    }
    (total / weight).clamp(-1.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_is_seamless() {
        let noise = Simplex::new(SEED);
        for i in 0..TILE_PX {
            let t = i as f32 / TILE_PX as f32;
            assert!((grain_at(&noise, 0.0, t) - grain_at(&noise, 1.0, t)).abs() < 1e-4);
            assert!((grain_at(&noise, t, 0.0) - grain_at(&noise, t, 1.0)).abs() < 1e-4);
        }
    }

    #[test]
    fn the_grain_is_faint_and_two_sided() {
        let image = grain_tile();
        let data = image.data.as_ref().unwrap();
        let alphas = data.chunks_exact(4).map(|texel| texel[3]);
        let max = alphas.clone().max().unwrap();
        assert!(max <= (LIGHT_ALPHA * 255.0) as u8 + 1, "alpha {max}");
        let light = data.chunks_exact(4).filter(|t| t[0] == 255).count();
        let dark = data.chunks_exact(4).filter(|t| t[0] == 0).count();
        assert!(light > 0 && dark > 0);
    }
}
