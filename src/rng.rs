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
//! - **потоки решений** ([`WanderIndex::next`]) — поток заводится на каждое
//!   решение пешки и живёт ровно до конца этого решения. Засев —
//!   `(PawnId, номер решения)`, то есть **наблюдаемая личность пешки и
//!   порядковый номер её выбора**, а не история потока. Поэтому жребий не
//!   зависит ни от порядка обхода запроса, ни от того, сколько соседей тянуло
//!   числа раньше в этом тике, ни от того, сколько выборок съело предыдущее
//!   решение этой же пешки: общий генератор рассыпался бы от любой
//!   перестановки в обходе (`panic` тянет жребий, обходя `HashSet<Entity>`, —
//!   его порядок между рестартами не совпадает), а живой поток на пешке —
//!   от любой добавленной строчки `rng.random()` внутри решения;
//! - **потоки размещения** — локальные, живут внутри одной функции
//!   (`spawn_population`), где обход заведомо последовательный.
//!
//! **Почему номер решения, а не позиция.** Соблазнительно засевать поток
//! координатами пешки — тогда и в недетерминированном режиме она ходила бы
//! из одной точки в одно и то же место. Но `move_moving_entities` при
//! достижении путевой точки делает `sim_position.0 = target`, а точки пути —
//! это `tile_center(...)`: пешка, дошедшая до тайла `T`, стоит **побитово**
//! там же, где стояла в прошлый раз. Отображение `(pawn_id, тайл) → цель`
//! стало бы детерминированной функцией, а всякая траектория такой функции на
//! конечном множестве рано или поздно замыкается в цикл — через несколько
//! минут каждый человек ходил бы по своему кольцу и никогда с него не сошёл.
//! Номер решения только растёт, поэтому цикла не бывает по построению.
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

/// Сколько решений пешка уже приняла — второй (после [`PawnId`]) вход её
/// жребия.
///
/// «Решение» — это один поход к [`WanderIndex::next`]: выбор цели прогулки,
/// период перепрокладки при панике, направление бегства. Счётчик общий на все
/// три: ключ `(pawn_id, номер)` уникален независимо от того, какое решение
/// принималось, потому что `next` его сдвигает.
///
/// Хранится на пешке, но состоянием ГПСЧ **не** является: по нему поток
/// выводится, а не продолжается. Рестарт сбрасывать его не обязан — пешки
/// пересоздаются.
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Component)]
pub struct WanderIndex(pub u32);

impl WanderIndex {
    /// Номер, отданный жребиям спавна (цвет, темп, начальный курс). Пешка
    /// заводится со счётчиком `1`, чтобы её первое решение не столкнулось с
    /// ними.
    pub const SPAWN: u32 = 0;

    /// Пешка, готовая принимать решения.
    pub const fn ready() -> Self {
        Self(Self::SPAWN + 1)
    }

    /// Поток на очередное решение — и сдвиг счётчика.
    ///
    /// Сдвиг обязателен и потому безусловен: сайт, который взял бы поток, не
    /// сдвинув счётчик, получал бы одно и то же число при каждом вызове —
    /// убегающий вечно сворачивал бы в одну сторону.
    pub fn next(&mut self, world_seed: u64, domain: RngDomain, pawn_id: u32) -> SimRng {
        let stream = decision_stream(world_seed, domain, pawn_id, self.0);
        self.0 = self.0.wrapping_add(1);
        stream
    }
}

/// Поток одного решения пешки. Ключ склеен из номера пешки и номера решения —
/// целиком, без перемешивания: `seed_for` всё равно прогоняет его через
/// splitmix64.
pub fn decision_stream(world_seed: u64, domain: RngDomain, pawn_id: u32, decision: u32) -> SimRng {
    let key = (u64::from(pawn_id) << 32) | u64::from(decision);
    SimRng::seed_from_u64(seed_for(world_seed, domain, key))
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
            .register_type::<WanderIndex>()
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
        assert_independent(|pawn_id| decision_stream(1, RngDomain::Human, pawn_id, 0).random());
    }

    /// То же требование к соседним **решениям** одной пешки, и оно строже: у
    /// `(pawn_id, номер)` меняется младшая половина ключа, тогда как у
    /// соседних `pawn_id` — старшая. Слипнись потоки здесь, человек ходил бы
    /// раз за разом в одну и ту же сторону.
    #[test]
    fn neighbouring_decisions_are_independent() {
        assert_independent(|decision| decision_stream(1, RngDomain::Human, 42, decision).random());
    }

    /// Среднее `|a − b|` первых чисел у 256 соседних пар: у двух независимых
    /// равномерных величин оно равно 1/3, у слипшихся потоков — 0. Полоса
    /// 0.28…0.39 — это ±4σ от 1/3 при 256 парах, то есть тест не может мигать,
    /// но ловит и полное совпадение, и заметную корреляцию.
    ///
    /// Одну пару сравнивать бессмысленно: два независимых числа сходятся
    /// ближе 0.05 примерно в каждом десятом случае.
    fn assert_independent(draw: impl Fn(u32) -> f64) {
        const PAIRS: u32 = 256;
        let mut total = 0.0f64;
        for index in 0..PAIRS {
            total += (draw(index) - draw(index + 1)).abs();
        }
        let mean = total / f64::from(PAIRS);
        assert!(
            (0.28..0.39).contains(&mean),
            "потоки не независимы: среднее |a − b| = {mean}, ждали ≈1/3"
        );
    }

    #[test]
    fn same_seed_same_stream() {
        let mut left = decision_stream(7, RngDomain::Demon, 3, 0);
        let mut right = decision_stream(7, RngDomain::Demon, 3, 0);
        for _ in 0..16 {
            assert_eq!(left.random::<u64>(), right.random::<u64>());
        }
    }

    /// Главное свойство схемы: k-е решение пешки одинаково, **когда бы оно ни
    /// случилось**. Именно оно делает жребий независимым от расписания — и
    /// поэтому одинаковым в обоих режимах.
    #[test]
    fn the_same_decision_number_gives_the_same_dice() {
        let mut early = WanderIndex::ready();
        let mut late = WanderIndex::ready();
        // одна пешка приняла три решения подряд, другая — те же три, но между
        // ними прошло сколько угодно тиков: номера совпадают, значит и числа
        let mut from_early = Vec::new();
        for _ in 0..3 {
            from_early.push(early.next(9, RngDomain::Human, 5).random::<u64>());
        }
        let mut from_late = Vec::new();
        for _ in 0..3 {
            from_late.push(late.next(9, RngDomain::Human, 5).random::<u64>());
        }
        assert_eq!(from_early, from_late);
        assert_eq!(early, late);
    }

    /// Счётчик обязан сдвигаться на каждом обращении: сайт, берущий поток без
    /// сдвига, получал бы одно и то же число вечно — убегающий сворачивал бы
    /// в одну сторону, а гуляющий ходил бы по кольцу.
    #[test]
    fn every_decision_advances_the_counter() {
        let mut index = WanderIndex::ready();
        let first = index.next(9, RngDomain::Human, 5).random::<u64>();
        let second = index.next(9, RngDomain::Human, 5).random::<u64>();
        assert_ne!(first, second);
        assert_eq!(index, WanderIndex(WanderIndex::SPAWN + 3));
    }

    /// Жребии спавна (цвет, темп, курс) не имеют права столкнуться с первым
    /// решением пешки.
    #[test]
    fn the_spawn_draw_is_not_the_first_decision() {
        let spawn = decision_stream(9, RngDomain::Human, 5, WanderIndex::SPAWN).random::<u64>();
        let first = WanderIndex::ready()
            .next(9, RngDomain::Human, 5)
            .random::<u64>();
        assert_ne!(spawn, first);
    }

    /// Ключ склеен сдвигом, поэтому пешка №1 с решением №0 и пешка №0 с
    /// решением №1 обязаны разойтись — иначе номера перетекали бы друг в друга.
    #[test]
    fn the_pawn_and_the_decision_do_not_bleed_into_each_other() {
        assert_ne!(
            decision_stream(9, RngDomain::Human, 1, 0).random::<u64>(),
            decision_stream(9, RngDomain::Human, 0, 1).random::<u64>()
        );
    }
}
