//! Приёмка детерминированного режима: один seed — один прогон, тик в тик.
//!
//! Проверяются три утверждения, и третье — главное:
//!
//! 1. **Повтор.** Прогон до тика N, `RestartEvent`, снова до N — отпечатки
//!    состояния совпадают. Это и есть «нажал R, и всё повторилось».
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
//! Сравнение только по `SimTick`, не по настенным часам и не по числу кадров:
//! `Time<Virtual>::max_delta` отбрасывает виртуальное время на долгих кадрах,
//! поэтому на одной и той же реальной секунде прогоны стоят на разных тиках —
//! это разная скорость проигрывания, а не расхождение.
//!
//! Bevy-приложение поднимается без окна, рендера и UI: карта читается из кеша
//! Overpass, navmesh заливается и прунится ровно как в `OnEnter(Playing)`.
//! Часть косметических систем (`draw_move_paths`, гизмо) не находит своих
//! параметров и молча пропускается — это ожидаемо и на симуляцию не влияет.
//!
//! ```text
//! cargo run --example determinism_replay -- [ticks]
//! ```

use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;

use qwe::city::City;
use qwe::demon::Demon;
use qwe::determinism::{Determinism, SimTick};
use qwe::grid::world_to_tile;
use qwe::human::Human;
use qwe::loading::{AppState, PlayPhase, WorldInitSet};
use qwe::map::osm::{MapData, overpass, parse};
use qwe::movement::{MovableState, SimPosition};
use qwe::navigation::{ArcNavmesh, Navmesh, PolymeshDebug, snap_portal_position};
use qwe::portal::PortalPos;
use qwe::restart::RestartEvent;
use qwe::rng::{PawnId, WorldSeed};
use qwe::telemetry::Telemetry;

const CITY: City = City::Tula;
/// 64 тика в секунду — шаг `Time<Fixed>` по умолчанию.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / 64);
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

    println!("\nprogon 1: seed 1, ровный кадр");
    let (first, second) = {
        let mut app = build_app(&json, &navmesh, portal, 1);
        let first = run_to_tick(&mut app, ticks, &[1]);

        println!("restart, progon 2: тот же seed, ровный кадр");
        // камеры в примере нет вовсе, поле на прогон не влияет
        app.world_mut().trigger(RestartEvent::default());
        // рестарт живёт в обсервере — даём кадр на применение команд
        app.update();
        let second = run_to_tick(&mut app, ticks, &[1]);
        (first, second)
    };

    println!("progon 3: тот же seed, РВАНЫЙ кадр (1..30 тиков за update)");
    let ragged = {
        let mut app = build_app(&json, &navmesh, portal, 1);
        run_to_tick(&mut app, ticks, &RAGGED_TICKS_PER_FRAME)
    };

    println!("progon 4: ДРУГОЙ seed");
    let other_seed = {
        let mut app = build_app(&json, &navmesh, portal, 2);
        run_to_tick(&mut app, ticks, &[1])
    };

    println!(
        "\n{:<28} {:>20} {:>8} {:>8}",
        "прогон", "отпечаток", "убито", "спаслось"
    );
    for (name, print) in [
        ("1. ровный", &first),
        ("2. после рестарта", &second),
        ("3. рваный кадр", &ragged),
        ("4. другой seed", &other_seed),
    ] {
        println!(
            "{name:<28} {:>20} {:>8} {:>8}",
            print.hash, print.killed, print.escaped
        );
    }
    println!();

    let mut failures = 0;
    failures += check("рестарт повторяет прогон", first == second);
    failures += check("рваный кадр не меняет прогон", first == ragged);
    failures += check("другой seed даёт другой прогон", first != other_seed);

    if failures > 0 {
        std::process::exit(1);
    }
    println!("\nOK: симуляция детерминирована по (seed, тик).");
}

fn check(what: &str, passed: bool) -> u32 {
    println!("{} {what}", if passed { "  ok  " } else { "FAILED" });
    u32::from(!passed)
}

/// Отпечаток состояния мира на конкретном тике.
struct Fingerprint {
    hash: u64,
    killed: usize,
    escaped: usize,
}

impl PartialEq for Fingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.killed == other.killed && self.escaped == other.escaped
    }
}

fn build_app(json: &str, navmesh: &Navmesh, portal: Vec2, seed: u64) -> App {
    let mut app = App::new();
    // Косметические системы (`draw_lunge_paths`, `draw_move_paths`) просят
    // `Gizmos`, а он живёт в рендере, которого здесь нет. По умолчанию Bevy
    // роняет кадр на такой невалидности параметров — здесь это ожидаемое
    // состояние, и им место в предупреждении, а не в панике. Симуляции эти
    // системы не касаются: они только рисуют.
    app.set_error_handler(bevy::ecs::error::warn);
    app.add_plugins((MinimalPlugins, StatesPlugin))
        // ровно один шаг `Time<Real>` на `app.update()`: иначе число тиков в
        // кадре задавали бы настенные часы, и «рваный» прогон было бы не
        // отличить от случайного
        .insert_resource(TimeUpdateStrategy::ManualDuration(TICK))
        .init_state::<AppState>()
        .add_sub_state::<PlayPhase>()
        .configure_sets(
            OnEnter(AppState::Playing),
            (WorldInitSet::Navmesh, WorldInitSet::Spawn).chain(),
        )
        .add_plugins((
            qwe::rng::RngPlugin,
            qwe::determinism::DeterminismPlugin,
            qwe::navigation::NavigationPlugin,
            qwe::movement::MovementPlugin,
            qwe::spatial::SpatialPlugin,
            qwe::telemetry::TelemetryPlugin,
            qwe::demon::DemonPlugin,
            qwe::human::HumanPlugin,
            qwe::restart::RestartPlugin,
        ))
        // хоткеи (R, M, …) висят на `input_just_pressed`, а `InputPlugin`
        // здесь нет: без ресурса условие валит кадр целиком
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(parse_map(json))
        .insert_resource(PortalPos(portal))
        .insert_resource(WorldSeed(seed))
        .insert_resource(Determinism(true))
        // бэкенд — сеточный, хотя в игре по умолчанию полигональный: в этом
        // примере нет прогрева, который дожидается постройки меша
        // (`NavigationBuildPending`), и меш, доехавший посреди прогона,
        // менял бы пути на полпути — то есть ровно то, что проверка обязана
        // исключить. Плоский A* готов с первого кадра
        .insert_resource(PolymeshDebug {
            enabled: false,
            ..default()
        });

    // navmesh — готовый: в игре его заливает поток загрузки, здесь это уже
    // сделано один раз на все четыре прогона
    *app.world_mut()
        .resource_mut::<ArcNavmesh>()
        .0
        .write()
        .unwrap() = navmesh.clone();

    // потолок виртуальной дельты за кадр снят: «рваный» прогон намеренно
    // подаёт до 30 тиков за один `update`, а штатные 250 мс срезали бы их до
    // шестнадцати — и проверка независимости от fps проверяла бы не то
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(10));

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Playing);
    app.update();
    // прогрев в этом примере ждать нечего: бэкенд — плоский A*, он готов
    // сразу, а `poll_warmup` живёт в `LoadingPlugin`, которого здесь нет
    app.world_mut()
        .resource_mut::<NextState<PlayPhase>>()
        .set(PlayPhase::Live);
    app.update();
    app
}

/// Крутит приложение, пока `SimTick` не дойдёт до `target`, подавая за кадр
/// столько тиков, сколько говорит очередной элемент `pattern` (циклически).
fn run_to_tick(app: &mut App, target: u64, pattern: &[u32]) -> Fingerprint {
    let started = std::time::Instant::now();
    let mut frame = 0usize;
    let mut reported = 0u64;
    while app.world().resource::<SimTick>().0 < target {
        // последний кадр урезается до остатка: иначе кадр на 30 тиков
        // перескакивает цель, и отпечатки снимались бы на РАЗНЫХ тиках — а
        // тогда «рваный кадр» проваливался бы всегда, и не потому, что
        // симуляция зависит от fps
        let remaining = (target - app.world().resource::<SimTick>().0) as u32;
        let ticks_this_frame = pattern[frame % pattern.len()].min(remaining);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(TICK * ticks_this_frame));
        app.update();
        frame += 1;

        // прогресс каждые 10%: прогон 20 000 пешек с ожиданием поиска пути на
        // каждом тике идёт минутами, и молчащий процесс неотличим от висящего
        let tick = app.world().resource::<SimTick>().0;
        if tick * 10 / target.max(1) > reported {
            reported = tick * 10 / target.max(1);
            println!(
                "  тик {tick}/{target}, {:.0} с",
                started.elapsed().as_secs_f32()
            );
        }
    }
    let tick = app.world().resource::<SimTick>().0;
    println!(
        "  готово: тик {tick} за {frame} кадров, {:.1} с",
        started.elapsed().as_secs_f32()
    );
    fingerprint(app.world_mut())
}

/// Хэш отсортированного состояния всех пешек. FNV-1a руками: заводить крейт
/// ради одного хэша не стоит, а `DefaultHasher` не обещает стабильности даже
/// внутри одной сборки.
fn fingerprint(world: &mut World) -> Fingerprint {
    let mut rows: Vec<(u8, u32, u32, u32, u8)> = Vec::new();

    let mut humans =
        world.query_filtered::<(&PawnId, &SimPosition, &qwe::movement::Movable), With<Human>>();
    for (pawn_id, position, movable) in humans.iter(world) {
        rows.push(row(0, pawn_id, position, &movable.state));
    }
    let mut demons =
        world.query_filtered::<(&PawnId, &SimPosition, &qwe::movement::Movable), With<Demon>>();
    for (pawn_id, position, movable) in demons.iter(world) {
        rows.push(row(1, pawn_id, position, &movable.state));
    }
    rows.sort_unstable();

    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    };
    for (species, pawn_id, x, y, state) in rows {
        eat(species);
        for byte in pawn_id.to_le_bytes() {
            eat(byte);
        }
        for byte in x.to_le_bytes() {
            eat(byte);
        }
        for byte in y.to_le_bytes() {
            eat(byte);
        }
        eat(state);
    }

    let telemetry = world.resource::<Telemetry>();
    Fingerprint {
        hash,
        killed: telemetry.killed,
        escaped: telemetry.escaped,
    }
}

/// Позиция — в БИТАХ: сравнивать float'ы на равенство нельзя, а нужна ровно
/// побайтовая одинаковость.
fn row(
    species: u8,
    pawn_id: &PawnId,
    position: &SimPosition,
    state: &MovableState,
) -> (u8, u32, u32, u32, u8) {
    let state = match state {
        MovableState::Idle => 0,
        MovableState::Pathfinding(_) => 1,
        MovableState::Moving(_) => 2,
        MovableState::PathfindingError(_) => 3,
    };
    (
        species,
        pawn_id.0,
        position.0.x.to_bits(),
        position.0.y.to_bits(),
        state,
    )
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
