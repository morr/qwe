//! Как рисуется демон: уголёк с ярким ядром, горячая рампа оттенков и ореол.
//!
//! Демон обязан читаться с зума толпы силуэтом и цветом (`VISION.md`, 12.6):
//! зубчатый уголёк — «не человек» уже в 8 px, красно-оранжевая рампа — тёплое
//! среди холодной толпы, а ореол вдвое-втрое шире тела виден и там, где само
//! тело уже точка. Ореол — дочерняя сущность: он наследует пульс пожирания и
//! уходит вместе с демоном, а y-сортировка кладёт его под соседних людей.

use bevy::prelude::*;

use crate::settings::{DEMON_MIN_PX, DEMON_SIZE};
use crate::silhouette::{Glyph, Silhouette, Silhouettes};

/// Сколько оттенков в кольце: демон номер `index` берёт `index % DEMON_TINT_SHADES`-й.
const DEMON_TINT_SHADES: usize = 5;
/// Самый тёмный оттенок кольца — демон номер 0: багровый.
const DEMON_TINT_BASE: Vec3 = Vec3::new(0.78, 0.08, 0.10);
/// Шаг кольца: с каждым оттенком краснота уходит в оранжевое.
const DEMON_TINT_STEP: Vec3 = Vec3::new(0.05, 0.07, -0.01);
/// Диаметр ореола в телах демона.
const HALO_RATIO: f32 = 3.0;
/// Цвет ореола: тёплый, полупрозрачный и ярче белого — HDR под bloom камеры
/// (`post.rs`): порог `BLOOM_THRESHOLD` 1.1 пропускает в свечение только такие
/// цвета. После альфа-смешивания центр даёт ~1.4, кромка гаснет в LDR.
const HALO_COLOR: Color = Color::linear_rgba(2.4, 0.6, 0.2, 0.35);

/// Ореол демона — дочерняя сущность с глифом [`Glyph::Halo`].
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonHalo;

/// Оттенок демона номер `index` — кольцо из [`DEMON_TINT_SHADES`] тонов, чтобы
/// вышедшие подряд демоны не сливались друг с другом.
pub(super) fn demon_tint(index: usize) -> Color {
    let tint = DEMON_TINT_BASE + DEMON_TINT_STEP * (index % DEMON_TINT_SHADES) as f32;
    Color::srgb(tint.x, tint.y, tint.z)
}

/// Спрайт и силуэт демона номер `index`.
pub(super) fn demon_body(silhouettes: &Silhouettes, index: usize) -> (Sprite, Silhouette) {
    (
        silhouettes.sprite(Glyph::Ember, demon_tint(index), Vec2::splat(DEMON_SIZE)),
        Silhouette::new(Vec2::splat(DEMON_SIZE), DEMON_MIN_PX),
    )
}

/// Ореол — под телом (z чуть ниже) и в [`HALO_RATIO`] раз шире.
pub(super) fn halo(silhouettes: &Silhouettes) -> impl Bundle {
    let size = Vec2::splat(DEMON_SIZE * HALO_RATIO);
    (
        DemonHalo,
        silhouettes.sprite(Glyph::Halo, HALO_COLOR, size),
        Silhouette::new(size, DEMON_MIN_PX * HALO_RATIO),
        Transform::from_xyz(0.0, 0.0, -0.01),
        Name::new("demon_halo"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_ring_wraps_after_five_shades() {
        assert_eq!(demon_tint(0), Color::srgb(0.78, 0.08, 0.10));
        assert_eq!(demon_tint(5), demon_tint(0));
        assert_eq!(demon_tint(9), demon_tint(4));
    }

    /// Ради чего кольцо и заведено: подряд вышедшие из портала демоны не
    /// сливаются друг с другом. Сравнение попарное, а не «все разные по
    /// красному каналу», — проверяем ровно то, что видит глаз. Без него
    /// нулевой [`DEMON_TINT_STEP`] прошёл бы мимо остальных тестов.
    #[test]
    fn five_demons_in_a_row_get_five_different_tints() {
        let tints: Vec<Color> = (0..DEMON_TINT_SHADES).map(demon_tint).collect();
        for (index, left) in tints.iter().enumerate() {
            for right in &tints[index + 1..] {
                assert_ne!(left, right, "shades {tints:?} do not differ");
            }
        }
    }

    #[test]
    fn every_shade_stays_hot() {
        for index in 0..DEMON_TINT_SHADES {
            let Srgba {
                red, green, blue, ..
            } = demon_tint(index).to_srgba();
            assert!(
                red > 0.7 && red > green * 2.0 && red > blue * 2.0,
                "shade {index} is not red-orange"
            );
        }
    }
}
