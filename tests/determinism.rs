//! Реплей детерминированного режима: один seed — один прогон, тик в тик.
//!
//! Быстрая половина приёмки (`examples/acceptance/determinism_replay.rs`
//! гоняет то же самое на настоящем городе и 20 000 пешках, но минутами):
//! синтетический двор, несколько десятков пешек, полторы виртуальные секунды.
//! Ловит она то, ради чего всё и заведено, — систему, которая осталась в
//! `Update` и трогает состояние симуляции: такая ломает
//! [`frame_rate_does_not_change_the_run`], где число тиков на кадр гуляет от 1
//! до 30 (проверено переносом `move_moving_entities` в `Update` — тест падает).
//!
//! Двор, а не настоящая карта, — потому что проверка сильна ровно настолько,
//! насколько сцена **нелинейна**: разбредясь по километрам, пешки только идут
//! по путям, а ход по пути линеен по времени, и подать его одним кадром или
//! тридцатью безразлично. Расхождение родится на порогах — радиусе паники,
//! броске демона, — а для них толпа и портал должны стоять друг на друге
//! (`fixture::crowded_yard`).
//!
//! Второй по важности — [`a_restart_replays_the_run`]: он гоняет второй прогон
//! в ТОМ ЖЕ `App` и потому ловит состояние, пережившее сброс. Оба дефекта, на
//! которых он впервые сработал, были в самой сборке приложения повтора: мир не
//! объявляли начавшимся (и бэкенд оставался пустой всюду проходимой сеткой), а
//! алгоритм не задавали явно (и достроившаяся иерархия попадала в рестартовую
//! заморозку) — см. `determinism::replay`.
//!
//! Сравнение только по `SimTick`, не по кадрам и не по настенным часам.

use bevy::prelude::*;
use qwe::determinism::replay::{Fingerprint, Progress, replay_app, run_to_tick};
use qwe::grid::world_to_tile;
use qwe::map::osm::fixture::crowded_yard;
use qwe::navigation::Navmesh;
use qwe::restart::RestartEvent;

/// Полторы виртуальные секунды: демон из портала (интервал спавна — секунда)
/// успевает появиться и погнаться, а блуждающие — выбрать цель и пойти.
const TICKS: u64 = 96;

/// Пешек — столько, чтобы прогон был живым, но каждая заявка считалась плоским
/// A* на полноразмерной сетке.
const POPULATION: usize = 64;

/// Сколько тиков подаётся за один кадр в «рваном» прогоне. Числа произвольные,
/// важно только их непостоянство: 1 тик — это ~64 fps, 30 — кадр почти в
/// полсекунды.
const RAGGED: [u32; 12] = [1, 7, 3, 12, 1, 30, 2, 5, 19, 1, 9, 4];

fn app(seed: u64) -> App {
    let yard = crowded_yard();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&yard.map);
    navmesh.prune_unreachable(world_to_tile(yard.portal));
    replay_app(yard.map, navmesh, yard.portal, seed, POPULATION)
}

fn run(seed: u64, pattern: &[u32]) -> Fingerprint {
    let mut app = app(seed);
    let print = run_to_tick(&mut app, TICKS, pattern, Progress::Silent);
    assert!(
        print.moving > 0,
        "мир стоит на месте — сравнивать нечего: {print:?}"
    );
    print
}

#[test]
fn the_same_seed_replays_tick_for_tick() {
    assert_eq!(run(1, &[1]), run(1, &[1]));
}

/// Главный из трёх: подаёт РАЗНОЕ число тиков за кадр и требует того же
/// отпечатка на том же тике. Первый тест проходил бы и на симуляции, которая
/// целиком висит в `Update`.
#[test]
fn frame_rate_does_not_change_the_run() {
    assert_eq!(run(1, &[1]), run(1, &RAGGED));
}

/// Иначе первые два проходили бы и на симуляции, которая просто стоит.
#[test]
fn a_different_seed_gives_a_different_run() {
    assert_ne!(run(1, &[1]), run(2, &[1]));
}

/// «Нажал R — и всё повторилось»: прогон до тика N, `RestartEvent`, снова до
/// N. Отличается от [`the_same_seed_replays_tick_for_tick`] тем, что второй
/// прогон идёт в ТОМ ЖЕ `App`, и потому ловит состояние, пережившее сброс.
#[test]
fn a_restart_replays_the_run() {
    let mut app = app(1);
    let first = run_to_tick(&mut app, TICKS, &[1], Progress::Silent);

    app.world_mut().trigger(RestartEvent::default());
    // рестарт живёт в обсервере — даём кадр на применение команд
    app.update();
    let second = run_to_tick(&mut app, TICKS, &[1], Progress::Silent);

    assert_eq!(first, second);
}
