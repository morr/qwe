//! Растр районов как текстура — общий для двух слоёв, что красят районы на
//! карте: отладочного (`ui/debug/overlays.rs::sync_district_overlay`) и слоя
//! территории осады (`ui/siege.rs::sync_territory_layer`). Оба — один спрайт
//! на `MAP_SIZE` с текселем на клетку растра меток (`DISTRICT_LABEL_METERS`);
//! различаются только цветом района, ключом пересборки и сэмплером.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::district::{DistrictId, Districts};
use crate::settings::{DISTRICT_LABEL_METERS, MAP_SIZE};

/// Ступеней прогресса скверны, различимых на слое: текстура пересобирается,
/// только когда какой-то район перешёл ступень.
pub(crate) const PROGRESS_SHADES: f32 = 16.0;

/// Размер текстуры — клетка растра меток на тексель (700 × 463).
pub(crate) fn texture_size() -> UVec2 {
    (MAP_SIZE / DISTRICT_LABEL_METERS).ceil().as_uvec2()
}

/// Район под каждым текселем, строка 0 — верх спрайта (максимальный мировой y).
pub(crate) fn texel_districts(districts: &Districts, size: UVec2) -> Vec<Option<DistrictId>> {
    let mut labels = Vec::with_capacity((size.x * size.y) as usize);
    for row in 0..size.y {
        for column in 0..size.x {
            let position = Vec2::new(column as f32 + 0.5, (size.y - 1 - row) as f32 + 0.5)
                * DISTRICT_LABEL_METERS;
            labels.push(districts.district_at(position));
        }
    }
    labels
}

/// Канал цвета в байт текстуры.
pub(crate) fn byte(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0) as u8
}

/// Текстура слоя: тексель — цвет своего района из `colors` (по `DistrictId`),
/// вне района — `empty`. Сэмплер ставит вызывающий.
pub(crate) fn district_texture(
    labels: &[Option<DistrictId>],
    size: UVec2,
    colors: &[[u8; 4]],
    empty: [u8; 4],
) -> Image {
    let mut data = Vec::with_capacity(labels.len() * 4);
    for label in labels {
        let texel = label.map_or(empty, |id| colors[id as usize]);
        data.extend_from_slice(&texel);
    }
    Image::new(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Ступень прогресса района, `0..=PROGRESS_SHADES`.
pub(crate) fn progress_shade(progress: f32) -> u64 {
    (progress.clamp(0.0, 1.0) * PROGRESS_SHADES) as u64
}

/// Ключ пересборки слоя, FNV-1a по значению на район.
pub(crate) fn fnv_key(values: impl IntoIterator<Item = u64>) -> u64 {
    values
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, value| {
            (hash ^ value).wrapping_mul(0x1000_0000_01b3)
        })
}
