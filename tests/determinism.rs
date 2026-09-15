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
//! заморозку). Первый класс закрыт с корнем: последовательность запуска мира
//! теперь одна на игру и на повтор (`loading::SimBootPlugin`), и объявить старт
//! забыть нельзя. Второй остаётся намеренным расхождением — повтор пинит
//! плоский A*, см. `determinism::replay`.
//!
//! Сравнение только по `SimTick`, не по кадрам и не по настенным часам.

use bevy::prelude::*;
use qwe::determinism::replay::{
    Fingerprint, Progress, SUMMON_TICK, brutes_alive, replay_app, run_to_tick, summon_brute,
};
use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::fixture::{Yard, crowded_yard};
use qwe::navigation::{Backend, Navmesh};
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

/// Навмеш двора — тот же рецепт, что у потока загрузки в игре: заливка по
/// `MapData`, затем прунинг недостижимого от портала.
fn yard_navmesh(yard: &Yard) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&yard.map);
    navmesh.prune_unreachable(world_to_tile(yard.portal));
    navmesh
}

fn app(seed: u64) -> App {
    let yard = crowded_yard();
    let navmesh = yard_navmesh(&yard);
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

/// Прогон с призывом: до [`SUMMON_TICK`], души и `SummonRequested` между
/// кадрами — так призыв доходит до фиксированного шага на одном и том же тике
/// при любом числе тиков на кадр (контракт повтора, скилл `determinism`), —
/// затем до [`TICKS`]. Громила обязан быть жив: иначе прогон сравнивал бы
/// отказ в призыве, а не призыв.
fn run_with_summon(app: &mut App, pattern: &[u32]) -> Fingerprint {
    run_to_tick(app, SUMMON_TICK, pattern, Progress::Silent);
    summon_brute(app);
    let print = run_to_tick(app, TICKS, pattern, Progress::Silent);

    assert_eq!(
        brutes_alive(app.world_mut()),
        1,
        "призыв Громилы не прошёл: {print:?}"
    );
    assert!(
        print.moving > 0,
        "мир стоит на месте — сравнивать нечего: {print:?}"
    );
    print
}

/// Прогон идёт по НАСТОЯЩЕЙ геометрии, а не по пустой всюду проходимой сетке.
///
/// Тот самый класс дефекта, на котором `a_restart_replays_the_run` сработал
/// впервые: мир не объявляли начавшимся, бэкенд оставался заглушкой, и весь
/// прогон честно и воспроизводимо шёл сквозь дома. Отпечатки при этом
/// совпадали — сравнивать два одинаково неверных мира тесту нечем.
///
/// `Default` у `Backend` больше нет, так что заглушку не собрать; осталось
/// проверить, что ресурс вообще доехал до прогона (без него системы поиска
/// пути молча не запускаются) и что он видит непроходимое непроходимым.
#[test]
fn the_run_gets_a_real_backend_not_an_empty_grid() {
    let navmesh = yard_navmesh(&crowded_yard());

    // во дворе есть здания и стены, так что непроходимое обязано найтись —
    // иначе сама фикстура перестала быть сценой с препятствиями
    let blocked = (0..navmesh.grid_size.x)
        .flat_map(|x| (0..navmesh.grid_size.y).map(move |y| IVec2::new(x, y)))
        .find(|tile| !navmesh.is_passable(tile.x, tile.y))
        .expect("во дворе нет ни одного непроходимого тайла");

    let app = app(1);
    let backend = app
        .world()
        .get_resource::<Backend>()
        .expect("прогон остался без ресурса Backend — системы поиска пути молча не работали бы");
    assert!(
        !backend.walkable().allows(tile_center(blocked)),
        "бэкенд считает проходимым тайл {blocked}, непроходимый в навмеше двора — \
         прогон идёт не по той геометрии"
    );
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

/// Призыв — ввод симуляции, как ползунок: поданный между кадрами после
/// [`SUMMON_TICK`], он приходится на один тик и при ровных кадрах, и при
/// рваных. Ловит призыв, съеденный в `Update`, и Громилу, чей номер или цена
/// зависят от того, сколько тиков уместилось в кадр.
#[test]
fn a_summon_replays_at_any_frame_rate() {
    let steady = run_with_summon(&mut app(1), &[1]);
    let ragged = run_with_summon(&mut app(1), &RAGGED);
    assert_eq!(steady, ragged);
}

/// Рестарт после призыва возвращает прогон без призыва: Громила уходит вместе с
/// демонами, `Souls` обнуляются на `WorldStarted`. Сравнение со свежим `App` —
/// то, что даёт этому тесту [`a_restart_replays_the_run`]: состояние призыва,
/// пережившее сброс, разошлось бы здесь.
#[test]
fn a_restart_forgets_the_summon() {
    let mut app = app(1);
    run_with_summon(&mut app, &[1]);

    app.world_mut().trigger(RestartEvent::default());
    app.update();
    let after_restart = run_to_tick(&mut app, TICKS, &[1], Progress::Silent);

    assert_eq!(after_restart, run(1, &[1]));
}
