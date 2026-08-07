//! Детерминированный жребий: один seed мира — и все потоки случайных чисел
//! выводятся из него.
//!
//! **Состояние ГПСЧ нигде не хранится глобально.** Ни ресурса-генератора, ни
//! счётчика, которые рестарт обязан был бы сбрасывать: каждый поток
//! пересевается из `seed_for(WorldSeed, домен, ключ)`, а значит одинаковый
//! seed даёт одинаковую симуляцию просто потому, что сбрасывать нечего.
//!
//! Потоков два рода:
//!
//! - **потоки сущностей** ([`EntityRng`]) — по одному на человека и на демона,
//!   засеяны его [`PawnId`]. Все поведенческие жребии пешки идут из её
//!   собственного потока, поэтому не зависят ни от порядка обхода запроса, ни
//!   от того, сколько соседей тянуло числа раньше в этом тике. Именно это
//!   делает детерминизм устойчивым: общий генератор рассыпался бы от любой
//!   перестановки в обходе (`panic` тянет жребий, обходя `HashSet<Entity>`, —
//!   его порядок между рестартами не совпадает);
//! - **потоки размещения** — локальные, живут внутри одной функции
//!   (`spawn_population`), где обход заведомо последовательный.
//!
//! Карту seed не трогает: OSM разбирается из кэш-файла, а деревья и входы
//! засеяны собственными координатами (`map/trees.rs`,
//! `map/osm/entrances`) — они детерминированы и без него. Seed управляет
//! **симуляцией**.

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use rand::SeedableRng;

/// ГПСЧ симуляции. Единственное место, где он выбирается: xoshiro256++ —
/// 32 байта состояния (важно при 20 000 людей) и заметно быстрее
/// криптографического thread-local генератора `rand::rng()`.
///
/// Воспроизводимость гарантируется **в пределах одной сборки**: `SmallRng` не
/// фиксирует алгоритм между версиями `rand`. Этого достаточно для «нажал R —
/// повторилось»; понадобится переносимость между машинами — здесь меняется
/// один тип на `rand_chacha::ChaCha8Rng`, остальной код не знает разницы.
pub type SimRng = rand::rngs::SmallRng;

/// Потолок seed: `toml` укладывает целые в `i64`, и значение выше него не
/// сохранилось бы в `settings.toml`.
pub const MAX_SEED: u64 = i64::MAX as u64;

/// Верхняя граница жребия кнопки перегенерации — девять знаков, чтобы seed
/// можно было прочитать с экрана и набрать руками.
pub const SEED_ROLL_RANGE: u64 = 1_000_000_000;

/// Seed мира: симуляция — чистая функция от него и от настроек. Переживает
/// перезапуск приложения и смену города (город — отдельная координата, а не
/// часть жребия).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "world", key = "seed")]
pub struct WorldSeed(pub u64);

impl Default for WorldSeed {
    fn default() -> Self {
        Self(1)
    }
}

/// Независимая ветка жребия. Домены разведены солью, поэтому человек и демон
/// с одним и тем же [`PawnId`] тянут разные числа.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RngDomain {
    /// Размещение населения: отбор проходимых тайлов в `spawn_population`.
    Population,
    /// Поток одного человека.
    Human,
    /// Поток одного демона.
    Demon,
}

impl RngDomain {
    /// Произвольные нечётные константы — важно только то, что они далеки
    /// друг от друга и фиксированы навсегда: меняя их, мы меняем все миры.
    const fn salt(self) -> u64 {
        match self {
            Self::Population => 0x5EED_0000_0001,
            Self::Human => 0x5EED_0000_0002,
            Self::Demon => 0x5EED_0000_0003,
        }
    }
}

/// Финализатор splitmix64: рассеивает соседние значения по всему диапазону.
/// Без него `pawn_id` 0, 1, 2 дали бы почти одинаковые засевы.
const fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Засев потока `(домен, ключ)` в мире `world_seed`.
///
/// Два раунда, а не один: ключ подмешивается уже к рассеянному засеву домена,
/// иначе близкие `world_seed` при одинаковом ключе давали бы близкие потоки.
pub const fn seed_for(world_seed: u64, domain: RngDomain, key: u64) -> u64 {
    splitmix64(splitmix64(world_seed ^ domain.salt()).wrapping_add(key))
}

/// Порядковый номер пешки внутри её вида и прогона: люди — `0..HUMAN_COUNT` в
/// порядке спавна, демоны — `DemonSpawner::spawned`.
///
/// Нужен вместо `Entity` везде, где требуется стабильный «личный номер»:
/// индексы сущностей после рестарта переиспользуются в другом порядке (он
/// зависит от того, кого съели в прошлом прогоне), поэтому хэш по
/// `entity.index()` менял бы веер бегства между прогоном и его повтором.
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Component)]
pub struct PawnId(pub u32);

/// Личный поток жребия пешки. Без `Reflect`: состояние ГПСЧ по BRP смотреть
/// незачем, а 20 000 таких компонентов только замусорили бы выдачу.
#[derive(Component)]
pub struct EntityRng(pub SimRng);

impl EntityRng {
    pub fn seeded(world_seed: u64, domain: RngDomain, pawn_id: u32) -> Self {
        Self(SimRng::seed_from_u64(seed_for(
            world_seed,
            domain,
            u64::from(pawn_id),
        )))
    }
}

/// Отдельный поток, не привязанный к сущности (размещение населения).
pub fn stream(world_seed: u64, domain: RngDomain, key: u64) -> SimRng {
    SimRng::seed_from_u64(seed_for(world_seed, domain, key))
}

pub struct RngPlugin;

impl Plugin for RngPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<WorldSeed>()
            .register_type::<PawnId>()
            .init_resource::<WorldSeed>();
    }
}

#[cfg(test)]
mod tests {
    use rand::Rng;

    use super::*;

    /// Золотые значения: засев — часть контракта мира, и молчаливая правка
    /// `splitmix64` или соли обесценила бы все записанные seed'ы.
    #[test]
    fn seed_for_is_stable() {
        assert_eq!(seed_for(1, RngDomain::Human, 0), 1_463_652_970_567_674_826);
        assert_eq!(seed_for(1, RngDomain::Human, 1), 13_079_080_308_853_110_745);
        assert_eq!(seed_for(1, RngDomain::Demon, 0), 7_917_996_653_345_551_703);
        assert_eq!(
            seed_for(1, RngDomain::Population, 0),
            14_296_305_627_125_674_832
        );
    }

    #[test]
    fn domains_and_keys_are_separated() {
        let seed = 42;
        let human = seed_for(seed, RngDomain::Human, 7);
        assert_ne!(human, seed_for(seed, RngDomain::Demon, 7));
        assert_ne!(human, seed_for(seed, RngDomain::Human, 8));
        assert_ne!(human, seed_for(seed + 1, RngDomain::Human, 7));
    }

    /// Потоки соседних `pawn_id` должны быть независимы, а не просто различны:
    /// `pawn_id` — порядковый номер спавна, и коррелированные засевы дали бы
    /// соседям по номеру похожие внешность и повадки.
    ///
    /// Проверяется средним `|a − b|` первых чисел у 256 соседних пар: у двух
    /// независимых равномерных величин оно равно 1/3, у слипшихся потоков — 0.
    /// Полоса 0.28…0.39 — это ±4σ от 1/3 при 256 парах, то есть тест не может
    /// мигать, но ловит и полное совпадение, и заметную корреляцию.
    ///
    /// Одну пару сравнивать бессмысленно: два независимых числа сходятся
    /// ближе 0.05 примерно в каждом десятом случае.
    #[test]
    fn neighbouring_pawn_ids_are_independent() {
        const PAIRS: u32 = 256;
        let mut total = 0.0f64;
        for pawn_id in 0..PAIRS {
            let first: f64 = EntityRng::seeded(1, RngDomain::Human, pawn_id).0.random();
            let second: f64 = EntityRng::seeded(1, RngDomain::Human, pawn_id + 1)
                .0
                .random();
            total += (first - second).abs();
        }
        let mean = total / f64::from(PAIRS);
        assert!(
            (0.28..0.39).contains(&mean),
            "потоки соседних pawn_id не независимы: среднее |a − b| = {mean}, ждали ≈1/3"
        );
    }

    #[test]
    fn same_seed_same_stream() {
        let mut left = EntityRng::seeded(7, RngDomain::Demon, 3).0;
        let mut right = EntityRng::seeded(7, RngDomain::Demon, 3).0;
        for _ in 0..16 {
            assert_eq!(left.random::<u64>(), right.random::<u64>());
        }
    }
}
