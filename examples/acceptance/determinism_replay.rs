//! Приёмка детерминированного режима на настоящем городе: один seed — один
//! прогон, тик в тик.
//!
//! Быстрая половина той же проверки живёт в `tests/determinism.rs` и идёт в
//! `cargo test` на синтетической карте. Здесь — Тула целиком, 20 000 пешек и
//! сотни тиков: то же самое под нагрузкой, до которой тест не доходит.
//!
//! Проверяются три утверждения, и третье — главное:
//!
//! 1. **Повтор.** Прогон до тика N, `RestartEvent`, снова до N — отпечатки
//!    состояния совпадают. Это и есть «нажал R, и всё повторилось».
//!    **Сейчас падает** — известный дефект, не регрессия запустившего:
//!    расхождение вскрылось, когда в приложение повтора добавили `SimLoad` и
//!    мир наконец поехал (см. `determinism::replay`). Прогон сам по себе
//!    воспроизводим — не воспроизводим сброс.
//! 2. **Seed что-то значит.** С другим seed отпечаток обязан отличаться —
//!    иначе первый пункт проходил бы и на симуляции, которая просто стоит.
//! 3. **Частота кадров ни при чём.** Третий прогон подаёт РАЗНОЕ число тиков
//!    на `app.update()` — от одного до тридцати, по сеяной таблице, то есть
//!    эмулирует и просадки до нескольких fps, и рывки. Отпечаток снимается по
//!    достижении того же **тика**, и обязан совпасть с первыми двумя.
//!
//! Без третьего пункта тест проверял бы только засев ГПСЧ: первые два прогона
//! идут с одинаковым числом тиков на кадр и не поймали бы ни одну систему,
//! которая осталась в `Update` и трогает состояние симуляции. Именно она —
//! самый вероятный способ сломать детерминизм будущей правкой.
//!
//! Карта читается из кеша Overpass, navmesh заливается и прунится ровно как в
//! `OnEnter(Playing)`; сборка приложения, прокрутка и отпечаток — общие с
//! тестом (`determinism::replay`).
//!
//! ```text
//! cargo run --example determinism_replay -- [ticks]
//! ```

use bevy::prelude::*;

use qwe::city::City;
use qwe::determinism::replay::{Fingerprint, Progress, replay_app, run_to_tick};
use qwe::grid::world_to_tile;
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::{Navmesh, snap_portal_position};
use qwe::restart::RestartEvent;
use qwe::settings::HUMAN_COUNT;

const CITY: City = City::Tula;

/// Сколько тиков прогонять по умолчанию — пять виртуальных секунд.
///
/// Мало по меркам симуляции и много по меркам времени: прогон стоит примерно
/// секунду реального времени на тик. Пример гонит мир так быстро, как
/// позволяет процессор, поэтому между подачей заявки и её сроком не проходит
/// НИСКОЛЬКО реального времени — и `apply_pathfinding_results` каждый тик
/// ждёт целую пачку поисков (до 1024 A* по Туле). В самой игре этого нет: там
/// тик длится 15.6 мс реального времени, у пачки есть восемь таких, а
/// установившийся спрос — порядка десятка заявок в тик, а не тысячи.
///
/// Больше тиков — аргументом: `cargo run --example determinism_replay -- 1280`
/// ловит и погоню с убийствами, но идёт около часа.
const DEFAULT_TICKS: u64 = 320;

/// Сколько тиков подаётся за один `app.update()` в «рваном» прогоне.
/// Числа произвольные, важно только их непостоянство: 1 тик — это ~64 fps,
/// 30 тиков — кадр длиной почти в полсекунды.
const RAGGED_TICKS_PER_FRAME: [u32; 12] = [1, 7, 3, 12, 1, 30, 2, 5, 19, 1, 9, 4];

fn main() {
    let ticks: u64 = std::env::args()
        .nth(1)
        .map(|value| value.parse().expect("ticks must be a number"))
        .unwrap_or(DEFAULT_TICKS);

    // JSON читается один раз, а `MapData` разбирается заново для каждого
    // прогона: она не `Clone`, а ресурс нужен каждому приложению свой
    let json = load_json();
    let navmesh = build_navmesh(&parse_map(&json));
    let portal = snap_portal_position(&navmesh, CITY.portal_hint()).expect("no spot for portal");
    let app = |seed| replay_app(parse_map(&json), navmesh.clone(), portal, seed, HUMAN_COUNT);

    println!("\nprogon 1: seed 1, ровный кадр");
    let (first, second) = {
        let mut app = app(1);
        let first = run_to_tick(&mut app, ticks, &[1], Progress::Print);

        println!("restart, progon 2: тот же seed, ровный кадр");
        // камеры в примере нет вовсе, поле на прогон не влияет
        app.world_mut().trigger(RestartEvent::default());
        // рестарт живёт в обсервере — даём кадр на применение команд
        app.update();
        let second = run_to_tick(&mut app, ticks, &[1], Progress::Print);
        (first, second)
    };

    println!("progon 3: тот же seed, РВАНЫЙ кадр (1..30 тиков за update)");
    let ragged = run_to_tick(&mut app(1), ticks, &RAGGED_TICKS_PER_FRAME, Progress::Print);

    println!("progon 4: ДРУГОЙ seed");
    let other_seed = run_to_tick(&mut app(2), ticks, &[1], Progress::Print);

    report(&[
        ("1. ровный", first),
        ("2. после рестарта", second),
        ("3. рваный кадр", ragged),
        ("4. другой seed", other_seed),
    ]);

    let mut failures = 0;
    failures += check("рестарт повторяет прогон", first == second);
    failures += check("рваный кадр не меняет прогон", first == ragged);
    failures += check("другой seed даёт другой прогон", first != other_seed);

    if failures > 0 {
        std::process::exit(1);
    }
    println!("\nOK: симуляция детерминирована по (seed, тик).");
}

fn report(runs: &[(&str, Fingerprint)]) {
    println!(
        "\n{:<28} {:>20} {:>8} {:>8} {:>8}",
        "прогон", "отпечаток", "в пути", "убито", "спаслось"
    );
    for (name, print) in runs {
        println!(
            "{name:<28} {:>20} {:>8} {:>8} {:>8}",
            print.hash, print.moving, print.killed, print.escaped
        );
    }
    println!();
}

fn check(what: &str, passed: bool) -> u32 {
    println!("{} {what}", if passed { "  ok  " } else { "FAILED" });
    u32::from(!passed)
}

fn load_json() -> String {
    let path = overpass::cache_path(CITY);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no OSM cache at {}: {error}. run the app once to download it",
            path.display()
        )
    })
}

fn parse_map(json: &str) -> MapData {
    parse::parse(json, CITY).expect("failed to parse cached OSM json")
}

/// Та же последовательность, что и в загрузке игры: заливка, снап портала,
/// прунинг недостижимого.
fn build_navmesh(map: &MapData) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    let portal = snap_portal_position(&navmesh, CITY.portal_hint()).expect("no spot for portal");
    let pruned = navmesh.prune_unreachable(world_to_tile(portal));
    println!("navmesh: pruned {pruned} unreachable tiles, portal at {portal:?}");
    navmesh
}
