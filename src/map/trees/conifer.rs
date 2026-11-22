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
//! примесь `mix · jitter` ([`TreeStyle::noise_mix`]) — детерминированный
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
//! Здесь же — связка набора деревьев со стилями: состав набора
//! ([`tree_compose`]) и пересемплирование поля под ручки ([`resample_field`],
//! [`recompose_row_trees`], [`build_conifer_field`], [`retune_conifer_field`]).
//!
//! [`TreeStyle::noise_mix`]: super::TreeStyle::noise_mix

use bevy::math::Vec2;
use bevy::prelude::{Reflect, ReflectDefault, ReflectResource, Res, ResMut, Resource, debug};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use noise::{NoiseFn, Simplex};

use super::TREE_NOISE_MIX_DEFAULT;
use super::{TreeRowStyle, TreeStyle};
use crate::map::osm::model::TreeSet;
use crate::map::osm::{MapData, TreeCompose, TreeRowLayout};

/// Длина волны поля, м: единица шума на столько метров. Массив — это не волна
/// шума, а её **вершина**: отсекая верхние 10 %, получаешь острова втрое-вчетверо
/// мельче волны, — поэтому 400 м на входе дают массивы поперёк 100–200 м. С
/// длиной 120 м (первый заход) массивы выходили по 50 м, то есть вкраплениями в
/// десяток крон, а не участками леса.
pub const CONIFER_NOISE_WAVELENGTH: f32 = 400.0;
/// Границы длины волны: 50 м — уже рябь в несколько крон, 1600 м — четверть
/// карты одной волной.
pub const CONIFER_NOISE_WAVELENGTH_MIN: f32 = 50.0;
pub const CONIFER_NOISE_WAVELENGTH_MAX: f32 = 1600.0;
pub const CONIFER_NOISE_WAVELENGTH_STEP: f32 = 50.0;
/// Октавы fbm: контур массива нужен рваный, а не гладкий овал. Дефолт — три:
/// при [`CONIFER_NOISE_LACUNARITY`] = 2 четвёртая октава — это волна в 50 м, то
/// есть рябь мельче самого массива, и породу у его кромки она разыгрывает по
/// монетке. Три октавы — 400/200/100 м; больше — осознанная рваность из панели
/// Noise, а не дефолт.
pub(crate) const CONIFER_NOISE_OCTAVES: u32 = 3;
/// Диапазоны октав, lacunarity и persistence — как у слайдеров zxc
/// (`zxc/src/map/generator/perlin_noise.rs`).
pub const CONIFER_NOISE_OCTAVES_MIN: f32 = 1.0;
pub const CONIFER_NOISE_OCTAVES_MAX: f32 = 8.0;
/// Шаг целочисленный: октава либо есть целиком, либо её нет.
pub const CONIFER_NOISE_OCTAVES_STEP: f32 = 1.0;
pub(crate) const CONIFER_NOISE_LACUNARITY: f32 = 2.0;
pub const CONIFER_NOISE_LACUNARITY_MIN: f32 = 1.0;
pub const CONIFER_NOISE_LACUNARITY_MAX: f32 = 4.0;
pub const CONIFER_NOISE_LACUNARITY_STEP: f32 = 0.1;
pub(crate) const CONIFER_NOISE_PERSISTENCE: f32 = 0.5;
pub const CONIFER_NOISE_PERSISTENCE_MIN: f32 = 0.0;
pub const CONIFER_NOISE_PERSISTENCE_MAX: f32 = 1.0;
pub const CONIFER_NOISE_PERSISTENCE_STEP: f32 = 0.05;
/// Сид поля — фиксированный: карта города обязана быть одинаковой от запуска
/// к запуску, как и посадка деревьев.
pub(crate) const CONIFER_NOISE_SEED: u32 = 0x00C0_FFEE;

// Умолчание каждого ползунка — внутри его же диапазона.
const _: () = {
    assert!(
        CONIFER_NOISE_WAVELENGTH >= CONIFER_NOISE_WAVELENGTH_MIN
            && CONIFER_NOISE_WAVELENGTH <= CONIFER_NOISE_WAVELENGTH_MAX
    );
    assert!(
        CONIFER_NOISE_OCTAVES as f32 >= CONIFER_NOISE_OCTAVES_MIN
            && CONIFER_NOISE_OCTAVES as f32 <= CONIFER_NOISE_OCTAVES_MAX
    );
    assert!(
        CONIFER_NOISE_LACUNARITY >= CONIFER_NOISE_LACUNARITY_MIN
            && CONIFER_NOISE_LACUNARITY <= CONIFER_NOISE_LACUNARITY_MAX
    );
    assert!(
        CONIFER_NOISE_PERSISTENCE >= CONIFER_NOISE_PERSISTENCE_MIN
            && CONIFER_NOISE_PERSISTENCE <= CONIFER_NOISE_PERSISTENCE_MAX
    );
};

/// Параметры fbm поля хвои — панель Noise (видна при включённом дебаг-слое
/// `noise`). Дефолты и границы ползунков лежат выше, рядом с ресурсом; правка
/// любого поля пересемплирует поле и пересобирает кроны
/// (`retune_conifer_field`).
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

impl ConiferNoiseStyle {
    /// Значения, зажатые шкалой: старый файл настроек или запись по BRP могут
    /// принести что угодно. Нулевая длина волны даёт `1/0 = inf` и NaN в
    /// позициях шума; неограниченные октавы — петлю на столько итераций;
    /// нечисловые f32 размножаются через `amplitude`/`frequency` в NaN.
    pub fn wavelength(&self) -> f32 {
        if self.wavelength.is_finite() {
            self.wavelength
                .clamp(CONIFER_NOISE_WAVELENGTH_MIN, CONIFER_NOISE_WAVELENGTH_MAX)
        } else {
            CONIFER_NOISE_WAVELENGTH
        }
    }

    pub fn octaves(&self) -> u32 {
        self.octaves.clamp(
            CONIFER_NOISE_OCTAVES_MIN as u32,
            CONIFER_NOISE_OCTAVES_MAX as u32,
        )
    }

    pub fn lacunarity(&self) -> f32 {
        if self.lacunarity.is_finite() {
            self.lacunarity
                .clamp(CONIFER_NOISE_LACUNARITY_MIN, CONIFER_NOISE_LACUNARITY_MAX)
        } else {
            CONIFER_NOISE_LACUNARITY
        }
    }

    pub fn persistence(&self) -> f32 {
        if self.persistence.is_finite() {
            self.persistence
                .clamp(CONIFER_NOISE_PERSISTENCE_MIN, CONIFER_NOISE_PERSISTENCE_MAX)
        } else {
            CONIFER_NOISE_PERSISTENCE
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
            mix: TREE_NOISE_MIX_DEFAULT,
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
        let base = f64::from(1.0 / self.style.wavelength());
        let x = f64::from(point.x) * base;
        let y = f64::from(point.y) * base;
        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut total = 0.0;
        for _ in 0..self.style.octaves() {
            sum += self.noise.get([x * frequency, y * frequency]) * amplitude;
            total += amplitude;
            amplitude *= f64::from(self.style.persistence());
            frequency *= f64::from(self.style.lacunarity());
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

/// Сборка `MapData::trees` из включённых источников: одиночные деревья, лес и
/// аллеи выбранной политики размещения.
///
/// Меняется сам набор деревьев, а не только их вид, поэтому вслед за сборкой
/// пересчитывается поле хвои: оно индексировано по `MapData::trees`, и без
/// пересемплирования порода поехала бы на все деревья после первой же аллеи.
///
/// Состав набора деревьев под ручки стилей — без мира: его собирает и
/// [`recompose_row_trees`] на входе в мир, и поток загрузки — под снимок
/// ручек (`map/prepare.rs`).
pub(crate) fn tree_compose(style: &TreeStyle, rows: &TreeRowStyle) -> TreeCompose {
    TreeCompose {
        layout: TreeRowLayout {
            placement: rows.placement,
            osm_spacing: rows.osm_spacing,
        },
        woods: style.woods,
        rows: rows.enabled,
        standalone: style.standalone,
    }
}

/// Значения поля хвои — под набор деревьев и параметры шума, порог — под долю
/// хвои. Одна пара на все пересемплирования: смена состава набора
/// ([`recompose_row_trees`]), вход в мир ([`build_conifer_field`]), правка
/// шума ([`retune_conifer_field`]) и сборка потока загрузки
/// (`map/prepare.rs::small_lane` — локальное поле для
/// [`super::mesh_game_trees`]).
pub fn resample_field(
    field: &mut ConiferField,
    trees: &TreeSet,
    style: &TreeStyle,
    noise: &ConiferNoiseStyle,
) {
    field.resample(trees.positions(), noise, style.noise_mix);
    field.set_share(style.conifer_share);
}

/// Признак «уже собрано» лежит в `MapData::composed_for`, а не в `Local`: при
/// смене города ресурс заменяется целиком, а `Local` пережил бы замену и решил,
/// что для нового города работа сделана.
pub fn recompose_row_trees(
    mut map: ResMut<MapData>,
    style: Res<TreeStyle>,
    rows: Res<TreeRowStyle>,
    noise: Res<ConiferNoiseStyle>,
    mut field: ResMut<ConiferField>,
) {
    let compose = tree_compose(&style, &rows);
    if map.composed_for == Some(compose) {
        return;
    }
    map.compose_trees(compose);
    resample_field(&mut field, &map.trees, &style, &noise);
}

/// Значение поля хвои в каждом посаженном дереве — до спавна крон, потому что
/// именно по нему `resolve` выбирает форму. Считается один раз на город: у
/// нового города свой набор деревьев, а старые значения к нему не относятся.
pub fn build_conifer_field(
    mut field: ResMut<ConiferField>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
    noise: Res<ConiferNoiseStyle>,
) {
    let started = std::time::Instant::now();
    resample_field(&mut field, &map.trees, &style, &noise);
    debug!(
        "conifer field: {} trees sampled in {:.1?}",
        map.trees.len(),
        started.elapsed()
    );
}

/// Пересемплирование поля после правки параметров шума (панель Noise) или
/// примеси (`TreeStyle::noise_mix`). Идёт в цепочке между
/// [`recompose_row_trees`] и [`super::rebuild_trees`], и выходит сразу, если
/// поле уже посчитано под текущие параметры, — так смена состава не платит за
/// второй resample, а правка цвета листвы не платит вовсе.
pub fn retune_conifer_field(
    mut field: ResMut<ConiferField>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
    noise: Res<ConiferNoiseStyle>,
) {
    if field.sampled_for(&noise, style.noise_mix) {
        return;
    }
    let started = std::time::Instant::now();
    resample_field(&mut field, &map.trees, &style, &noise);
    debug!(
        "conifer field retuned: {} trees resampled in {:.1?}",
        map.trees.len(),
        started.elapsed()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Мусорные значения (BRP, старый settings.toml) не добивают до шума:
    /// нулевая длина волны давала `1/0 = inf` и NaN в позициях, октавы без
    /// потолка — петлю на столько итераций, нечисловое — NaN в амплитуде.
    #[test]
    fn noise_style_clamps_garbage_to_the_slider_scale() {
        assert_eq!(
            ConiferNoiseStyle::default().wavelength(),
            CONIFER_NOISE_WAVELENGTH
        );
        assert_eq!(
            ConiferNoiseStyle {
                wavelength: 0.0,
                ..Default::default()
            }
            .wavelength(),
            CONIFER_NOISE_WAVELENGTH_MIN
        );
        assert_eq!(
            ConiferNoiseStyle {
                wavelength: f32::NEG_INFINITY,
                ..Default::default()
            }
            .wavelength(),
            CONIFER_NOISE_WAVELENGTH
        );
        assert_eq!(
            ConiferNoiseStyle {
                octaves: u32::MAX,
                ..Default::default()
            }
            .octaves(),
            CONIFER_NOISE_OCTAVES_MAX as u32
        );
        assert_eq!(
            ConiferNoiseStyle {
                persistence: f32::NAN,
                ..Default::default()
            }
            .persistence(),
            CONIFER_NOISE_PERSISTENCE
        );
        assert_eq!(
            ConiferNoiseStyle {
                lacunarity: -1.0,
                ..Default::default()
            }
            .lacunarity(),
            CONIFER_NOISE_LACUNARITY_MIN
        );
    }
}
