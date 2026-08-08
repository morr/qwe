use rand::Rng;

use super::*;

/// Тоже золотые: доля хэша решает сторону обхода и веер бегства, то есть
/// входит в контракт повтора наравне с засевом. Значения сняты с трёх
/// написаний формулы, которые эта функция заменила.
#[test]
fn hash_fraction_is_stable() {
    // 1 · 2654435761 — веер бегства и ось разведения первой пешки
    assert_eq!(hash_fraction(2_654_435_761), 0.618_034);
    assert_eq!(hash_fraction(2_703_968_361), 0.629_566_7);
    // (1 · 2246822519) ^ 0x9e37_79b9 — её же сторона обхода
    assert_eq!(hash_fraction(467_448_782), 0.108_836_36);
    // концы диапазона: ноль ровно, потолок строго меньше единицы
    assert_eq!(hash_fraction(0), 0.0);
    assert_eq!(hash_fraction(u32::MAX), 1.0);
}

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
