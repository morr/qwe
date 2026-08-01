//! Поле хвои: где в лесу растут ели, а где лиственные.
//!
//! Хвоя растёт **массивами** — участок леса хвойный почти целиком, а между
//! массивами хвои почти нет. Поэтому выбор породы идёт не по индексу дерева в
//! списке (индекс к географии не привязан и рассыпал бы хвою по одному дереву
//! среди лиственных), а по fbm-симплексу от **мировых координат** ствола:
//! соседние деревья читают почти одно и то же значение и становятся хвойными
//! вместе.
//!
//! Сплошным массив быть не обязан: к значению поля в дереве добавляется
//! примесь `mix · jitter` ([`TreeStyle::conifer_mix`]) — детерминированный
//! джиттер по позиции ствола. Он двигает деревья через порог в обе стороны:
//! лиственные вкрапления в хвойном массиве и одиночные ели среди лиственных,
//! тем глубже от кромки, чем больше `mix`.
//!
//! Порог берётся **квантилем** значений в самих деревьях, а не фиксированным
//! уровнем шума: fbm распределён колоколом, и «взять всё выше 0.9» дало бы долю
//! хвои, к запрошенной не относящуюся никак. Квантиль считается по значениям
//! **с примесью**, так что ползунок доли точен при любых параметрах шума и
//! любой силе примеси, а кластеризацию не портит — без примеси отбираются те
//! же деревья на вершинах пятен.
//!
//! [`TreeStyle::conifer_mix`]: super::TreeStyle::conifer_mix

use bevy::math::Vec2;
use bevy::prelude::{Reflect, ReflectDefault, ReflectResource, Resource};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use noise::{NoiseFn, Simplex};

use crate::settings::{
    CONIFER_MIX_DEFAULT, CONIFER_NOISE_LACUNARITY, CONIFER_NOISE_OCTAVES,
    CONIFER_NOISE_PERSISTENCE, CONIFER_NOISE_SEED, CONIFER_NOISE_WAVELENGTH,
};

/// Параметры fbm поля хвои — панель Noise (видна при включённом дебаг-слое
/// `noise`). Дефолты и границы ползунков — в `settings.rs`; правка любого поля
/// пересемплирует поле и пересобирает кроны (`retune_conifer_field`).
#[derive(Resource, Reflect, SettingsGroup, Clone, Debug, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "conifer_noise")]
pub struct ConiferNoiseStyle {
    /// Длина волны, м: единица шума на столько метров мира.
    pub wavelength: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub persistence: f32,
}

impl Default for ConiferNoiseStyle {
    fn default() -> Self {
        Self {
            wavelength: CONIFER_NOISE_WAVELENGTH,
            octaves: CONIFER_NOISE_OCTAVES,
            lacunarity: CONIFER_NOISE_LACUNARITY,
            persistence: CONIFER_NOISE_PERSISTENCE,
        }
    }
}

/// Поле хвои: сам шум, его значение в каждом посаженном дереве и порог,
/// отсекающий хвойную долю.
#[derive(Resource)]
pub struct ConiferField {
    noise: Simplex,
    /// Параметры, под которые посчитаны [`Self::values`], — чтобы
    /// `retune_conifer_field` не пересемплировал без нужды.
    style: ConiferNoiseStyle,
    /// Сила примеси, с которой посчитаны [`Self::values`].
    mix: f32,
    /// Значение поля (с примесью) в каждом дереве, в порядке `MapData::trees`.
    values: Vec<f32>,
    /// Доля, под которую посчитан [`Self::threshold`]; `NaN` — не посчитан.
    share: f32,
    /// Дерево хвойное, если его значение не ниже порога.
    threshold: f32,
    /// Растёт на каждом пересемплировании — ключ кеша дебаг-слоя: порог мог
    /// совпасть числом и на новых параметрах, а рельеф поля уже другой.
    generation: u32,
}

impl Default for ConiferField {
    fn default() -> Self {
        Self {
            noise: Simplex::new(CONIFER_NOISE_SEED),
            style: ConiferNoiseStyle::default(),
            mix: CONIFER_MIX_DEFAULT,
            values: Vec::new(),
            share: f32::NAN,
            threshold: f32::INFINITY,
            generation: 0,
        }
    }
}

impl ConiferField {
    /// Значение поля в мировой точке, 0..1 — **без примеси**: дебаг-слой рисует
    /// непрерывный рельеф шума, а примесь живёт только в деревьях.
    pub fn sample(&self, point: Vec2) -> f32 {
        let base = f64::from(1.0 / self.style.wavelength);
        let x = f64::from(point.x) * base;
        let y = f64::from(point.y) * base;
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut total = 0.0;
        for _ in 0..self.style.octaves {
            sum += self.noise.get([x * frequency, y * frequency]) * amplitude;
            total += amplitude;
            amplitude *= f64::from(self.style.persistence);
            frequency *= f64::from(self.style.lacunarity);
        }
        // симплекс отдаёт −1..1, нормируем в 0..1: порог квантильный, так что
        // для отбора это безразлично, но дебаг-слой красит по значению
        ((sum / total + 1.0) / 2.0) as f32
    }

    /// Примесь дерева, ±0.5: хеш (финализатор murmur3) от битов мировых
    /// координат ствола. Именно от **позиции**, не от индекса: тумблеры состава
    /// пересобирают `MapData::trees` и сдвигают индексы, а прореживание
    /// плотностью берёт префикс — позиция же не двигается никогда, и порода
    /// дерева не мигает от чужих ручек.
    fn jitter(point: Vec2) -> f32 {
        let mut hash = (u64::from(point.x.to_bits()) << 32 | u64::from(point.y.to_bits()))
            ^ u64::from(CONIFER_NOISE_SEED);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        hash ^= hash >> 33;
        (hash >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }

    /// Пересчёт значений — под новый набор деревьев или новые параметры. Порог
    /// при этом сбрасывается: он считается по этим самым значениям.
    pub fn resample(&mut self, trees: &[(Vec2, f32)], style: &ConiferNoiseStyle, mix: f32) {
        self.style = style.clone();
        self.mix = mix;
        self.values = trees
            .iter()
            .map(|&(position, _)| self.sample(position) + mix * Self::jitter(position))
            .collect();
        self.share = f32::NAN;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Посчитаны ли значения ровно под эти параметры и примесь.
    pub fn sampled_for(&self, style: &ConiferNoiseStyle, mix: f32) -> bool {
        self.style == *style && self.mix == mix
    }

    /// Номер пересемплирования — ключ кеша дебаг-слоя.
    pub fn generation(&self) -> u32 {
        self.generation
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

    /// Сырые значения — тестам: проверить, что примесь привязана к позиции.
    #[cfg(test)]
    pub fn values_for_test(&self) -> &[f32] {
        &self.values
    }
}
