//! Один ГПСЧ и один посев от точки на весь `map/*`.
//!
//! Всё, что карта расставляет «случайно» — кроны деревьев, оборудование на
//! кровлях, припаркованные машины, — обязано быть воспроизводимым: пересборка
//! слоя на пороге зума, переключение режима высот и рестарт не имеют права
//! ничего сдвинуть. Отсюда одна и та же пара примитивов у всех: ГПСЧ Лемера,
//! засеянный [`seed_from_point`] от опорной точки самого объекта — первой
//! вершины контура дома, первой точки улицы, — а не от его номера в выгрузке.

use bevy::prelude::*;

/// ГПСЧ Лемера (Park–Miller), как в Village.js: `seed = 48271·seed mod 2³¹−1`.
pub(crate) struct Lcg(u32);

impl Lcg {
    pub(crate) fn new(seed: u32) -> Self {
        Self((seed % 0x7FFF_FFFF).max(1))
    }

    pub(crate) fn next_f32(&mut self) -> f32 {
        self.0 = ((u64::from(self.0) * 48271) % 0x7FFF_FFFF) as u32;
        self.0 as f32 / 2_147_483_647.0
    }

    /// Число в `[from, to)`.
    pub(crate) fn range(&mut self, from: f32, to: f32) -> f32 {
        from + self.next_f32() * (to - from)
    }

    /// Среднее трёх uniform — колокол на (0,1) со средним 0.5.
    pub(crate) fn gauss3(&mut self) -> f32 {
        (self.next_f32() + self.next_f32() + self.next_f32()) / 3.0
    }

    /// Сумма четырёх uniform / 2 − 1 — колокол на (−1,1) со средним 0.
    pub(crate) fn bell4(&mut self) -> f32 {
        (self.next_f32() + self.next_f32() + self.next_f32() + self.next_f32()) / 2.0 - 1.0
    }
}

/// Посев от опорной точки — три перемешивающих раунда, чтобы соседние по
/// координате объекты не попадали в один слот таблицы. Сантиметры, а не
/// метры: два дома на одной улице отличаются десятками сантиметров.
pub(crate) fn seed_from_point(point: Vec2) -> u32 {
    let x = (point.x * 100.0) as i32 as u32;
    let y = (point.y * 100.0) as i32 as u32;
    let mut hash = x ^ y.rotate_left(16);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^= hash >> 16;
    hash
}
