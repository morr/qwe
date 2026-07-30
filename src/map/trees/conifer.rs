//! Поле хвои: где в лесу растут ели, а где лиственные.
//!
//! Хвоя растёт **массивами** — участок леса хвойный почти целиком, а между
//! массивами хвои почти нет. Поэтому выбор породы идёт не по индексу дерева в
//! списке (индекс к географии не привязан и рассыпал бы хвою по одному дереву
//! среди лиственных), а по fbm-симплексу от **мировых координат** ствола:
//! соседние деревья читают почти одно и то же значение и становятся хвойными
//! вместе.
//!
//! Порог берётся **квантилем** значений в самих деревьях, а не фиксированным
//! уровнем шума: fbm распределён колоколом, и «взять всё выше 0.9» дало бы долю
//! хвои, к запрошенной не относящуюся никак. Квантиль делает ползунок точным
//! при любых параметрах шума, а кластеризацию не портит — отбираются те же
//! деревья на вершинах пятен.

use bevy::math::Vec2;
use bevy::prelude::Resource;
use noise::{NoiseFn, Simplex};

use crate::settings::{
    CONIFER_NOISE_FREQUENCY, CONIFER_NOISE_LACUNARITY, CONIFER_NOISE_OCTAVES,
    CONIFER_NOISE_PERSISTENCE, CONIFER_NOISE_SEED,
};

/// Поле хвои: сам шум, его значение в каждом посаженном дереве и порог,
/// отсекающий хвойную долю.
#[derive(Resource)]
pub struct ConiferField {
    noise: Simplex,
    /// Значение поля в каждом дереве, в порядке `MapData::trees`.
    values: Vec<f32>,
    /// Доля, под которую посчитан [`Self::threshold`]; `NaN` — не посчитан.
    share: f32,
    /// Дерево хвойное, если его значение не ниже порога.
    threshold: f32,
}

impl Default for ConiferField {
    fn default() -> Self {
        Self {
            noise: Simplex::new(CONIFER_NOISE_SEED),
            values: Vec::new(),
            share: f32::NAN,
            threshold: f32::INFINITY,
        }
    }
}

impl ConiferField {
    /// Значение поля в мировой точке, 0..1.
    pub fn sample(&self, point: Vec2) -> f32 {
        let x = point.x as f64 * CONIFER_NOISE_FREQUENCY;
        let y = point.y as f64 * CONIFER_NOISE_FREQUENCY;
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut total = 0.0;
        for _ in 0..CONIFER_NOISE_OCTAVES {
            sum += self.noise.get([x * frequency, y * frequency]) * amplitude;
            total += amplitude;
            amplitude *= CONIFER_NOISE_PERSISTENCE;
            frequency *= CONIFER_NOISE_LACUNARITY;
        }
        // симплекс отдаёт −1..1, нормируем в 0..1: порог квантильный, так что
        // для отбора это безразлично, но дебаг-слой красит по значению
        ((sum / total + 1.0) / 2.0) as f32
    }

    /// Пересчёт значений под новый набор деревьев (город сменился). Порог при
    /// этом сбрасывается: он считается по этим самым значениям.
    pub fn resample(&mut self, trees: &[(Vec2, f32)]) {
        self.values = trees
            .iter()
            .map(|&(position, _)| self.sample(position))
            .collect();
        self.share = f32::NAN;
    }

    /// Порог под запрошенную долю хвои — квантиль значений. Повторный вызов с
    /// той же долей ничего не делает: сортировка не должна повторяться на
    /// каждой правке цвета листвы.
    pub fn set_share(&mut self, share: f32) {
        if self.share == share {
            return;
        }
        self.share = share;
        // край диапазона — не квантиль, а «все» / «никто»: при share = 0
        // квантиль всё равно отдал бы максимум списка, то есть одну ель
        self.threshold = if share <= 0.0 || self.values.is_empty() {
            f32::INFINITY
        } else if share >= 1.0 {
            f32::NEG_INFINITY
        } else {
            let mut sorted = self.values.clone();
            sorted.sort_unstable_by(f32::total_cmp);
            sorted[((1.0 - share) * sorted.len() as f32) as usize]
        };
    }

    /// Порог, по которому дебаг-слой красит хвойную область.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Хвойное ли дерево с этим индексом в `MapData::trees`.
    pub fn is_conifer(&self, index: usize) -> bool {
        self.values
            .get(index)
            .is_some_and(|&value| value >= self.threshold)
    }
}
