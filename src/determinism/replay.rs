//! Повтор прогона: приложение без окна и рендера, прокрутка по тикам и
//! отпечаток состояния.
//!
//! Одна машинерия на две проверки — они отличаются только масштабом:
//!
//! - `tests/determinism.rs` — синтетический двор (`fixture::crowded_yard`),
//!   десятки пешек, полторы виртуальные секунды. Идёт в `cargo test` и ловит
//!   самую вероятную поломку: систему, которая осталась в `Update` и трогает
//!   состояние симуляции.
//! - `examples/acceptance/determinism_replay.rs` — настоящий город, 20 000
//!   пешек, сотни тиков. Идёт руками, минутами, и проверяет то же самое на
//!   нагрузке, до которой тест не доходит.
//!
//! Обе проверки настолько же сильны, насколько **живая** у них сцена, и это не
//! фигура речи: пока `apply_pathfinding_results` молча не выполнялась (см.
//! `SimLoad` ниже), сравнивались два одинаково неподвижных мира, и повтор
//! совпадал при любой поломке. Отсюда [`Fingerprint::moving`] и двор вместо
//! разбросанной по километрам толпы.
//!
//! Второе, что здесь легко получить нечаянно, — **мир, отличный от игрового**.
//! `LoadingPlugin` сюда не входит, поэтому всё, что он делает по дороге в
//! `Live`, приходится делать руками, и молчаливого «не сделали» не бывает
//! видно: мир едет, просто не тот. Оба случая уже случились и оба ловятся
//! теперь `a_restart_replays_the_run` — необъявленный старт мира (бэкендом
//! прогона оставалась пустая всюду проходимая сетка) и алгоритм по умолчанию
//! (иерархия достраивалась посреди прогона). Добавляя сюда ресурс или фазу,
//! сверяйтесь с `loading.rs`, а не с тем, что «и так работает».
//!
//! Сравнивать состояния можно только по [`SimTick`](super::SimTick):
//! `Time<Virtual>::max_delta` отбрасывает виртуальное время на долгих кадрах,
//! поэтому на одной и той же реальной секунде прогоны стоят на разных тиках —
//! это разная скорость проигрывания, а не расхождение.

use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;

use super::{Determinism, SimTick};
use crate::demon::Demon;
use crate::human::{Human, PopulationSize};
use crate::loading::{AppState, PlayPhase};
use crate::map::osm::MapData;
use crate::movement::{MovableState, SimPosition};
use crate::navigation::{ArcNavmesh, Navmesh, PathfindingAlgorithm, PolymeshDebug};
use crate::portal::PortalPos;
use crate::rng::{PawnId, WorldSeed};
use crate::telemetry::Telemetry;

/// 64 тика в секунду — шаг `Time<Fixed>` по умолчанию.
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / 64);

/// Отпечаток состояния мира на конкретном тике.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fingerprint {
    pub hash: u64,
    pub killed: usize,
    pub escaped: usize,
    /// Сколько пешек не стоят на месте. В хэш это и так входит; наружу — чтобы
    /// проверка могла убедиться, что сравнивает живой прогон, а не два
    /// одинаково неподвижных мира.
    pub moving: usize,
}

/// Печатать ли ход прокрутки. Приёмке она нужна — прогон идёт минутами, и
/// молчащий процесс неотличим от висящего; тесту незачем.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    Silent,
    Print,
}

/// Приложение для повтора: симуляция целиком, без окна, рендера и UI.
///
/// Бэкенд — сеточный плоский A*, и он **задан явно**
/// ([`PathfindingAlgorithm::Astar`]), а не выбран умолчанием. В игре по
/// умолчанию HPA*, чью иерархию (и полигональный меш при своей настройке)
/// дожидается прогрев; здесь прогрева нет, и постройка, доехавшая посреди
/// прогона, меняла бы пути на полпути — то есть ровно то, что проверка обязана
/// исключить. Раньше это держалось на том, что за короткий прогон иерархия
/// «не успеет», и разваливалось на рестарте: тот берёт снимок бэкенда заново,
/// и к этому моменту иерархия уже стояла — повтор шёл другим алгоритмом.
/// Плоский A* не просит постройки вовсе и готов с первого кадра.
pub fn replay_app(
    map: MapData,
    navmesh: Navmesh,
    portal: Vec2,
    seed: u64,
    population: usize,
) -> App {
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
        .add_plugins((
            // как заводится мир — той же одной реализацией, что и в игре:
            // состояния, порядок инициализации, пауза прогрева и объявление
            // старта. Всё остальное здесь своё
            crate::loading::SimBootPlugin,
            crate::rng::RngPlugin,
            super::DeterminismPlugin,
            crate::navigation::NavigationPlugin,
            crate::movement::MovementPlugin,
            crate::spatial::SpatialPlugin,
            crate::telemetry::TelemetryPlugin,
            crate::demon::DemonPlugin,
            crate::human::HumanPlugin,
            crate::restart::RestartPlugin,
        ))
        // хоткеи (R, M, …) висят на `input_just_pressed`, а `InputPlugin`
        // здесь нет: без ресурса условие валит кадр целиком
        .init_resource::<ButtonInput<KeyCode>>()
        // Счётчик нагрузки просит `apply_pathfinding_results` — без него она
        // не проходит валидацию параметров и **молча не выполняется**, то есть
        // ни один найденный путь не применяется, пешки навсегда остаются в
        // `Pathfinding`, и повтор сравнивает два одинаково неподвижных мира.
        // Целиком `SimTimePlugin` брать нельзя: его регулятор крутит
        // `Time<Virtual>` по замеренной нагрузке, а это настенные часы в
        // содержимом тика
        .init_resource::<crate::sim_time::SimLoad>()
        .insert_resource(PathfindingAlgorithm::Astar)
        .insert_resource(map)
        .insert_resource(PortalPos(portal))
        .insert_resource(WorldSeed(seed))
        .insert_resource(PopulationSize(population))
        .insert_resource(Determinism(true))
        .insert_resource(PolymeshDebug {
            enabled: false,
            ..default()
        });

    // navmesh — готовый: в игре его заливает поток загрузки
    *app.world_mut()
        .resource_mut::<ArcNavmesh>()
        .0
        .write()
        .unwrap() = navmesh;

    // потолок виртуальной дельты за кадр снят: «рваный» прогон намеренно
    // подаёт до 30 тиков за один `update`, а штатные 250 мс срезали бы их до
    // шестнадцати — и проверка независимости от fps проверяла бы не то
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(10));

    // Дальше — те же две фазы, что проходит игра. Мир объявляет свой старт
    // сам, на входе в `Live` (`SimBootPlugin`): этим событием прогон забирает
    // себе бэкенд, обнуляет тики, телеметрию, часы и спавнер демонов.
    //
    // Прогрев проходится насквозь: ждать здесь нечего (бэкенд — плоский A*,
    // готов сразу, а `poll_warmup` живёт в `LoadingPlugin`, которого здесь
    // нет), но пройти его надо — на нём мир стои́т на паузе, и первый тик
    // случается уже после объявления старта. Раньше эта сцена прогрева не
    // знала, тикала в нём и потому объявляла старт руками, до первого кадра.
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Playing);
    app.update();
    app.world_mut()
        .resource_mut::<NextState<PlayPhase>>()
        .set(PlayPhase::Live);
    app.update();
    app
}

/// Крутит приложение, пока [`SimTick`](super::SimTick) не дойдёт до `target`,
/// подавая за кадр столько тиков, сколько говорит очередной элемент `pattern`
/// (циклически). Разный `pattern` при одном отпечатке — и есть проверка «fps
/// ни при чём».
pub fn run_to_tick(app: &mut App, target: u64, pattern: &[u32], progress: Progress) -> Fingerprint {
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

        if progress == Progress::Print {
            let tick = app.world().resource::<SimTick>().0;
            if tick * 10 / target.max(1) > reported {
                reported = tick * 10 / target.max(1);
                println!(
                    "  тик {tick}/{target}, {:.0} с",
                    started.elapsed().as_secs_f32()
                );
            }
        }
    }
    if progress == Progress::Print {
        println!(
            "  готово: тик {} за {frame} кадров, {:.1} с",
            app.world().resource::<SimTick>().0,
            started.elapsed().as_secs_f32()
        );
    }
    fingerprint(app.world_mut())
}

/// Хэш отсортированного состояния всех пешек. FNV-1a руками: заводить крейт
/// ради одного хэша не стоит, а `DefaultHasher` не обещает стабильности даже
/// внутри одной сборки.
pub fn fingerprint(world: &mut World) -> Fingerprint {
    let mut rows: Vec<(u8, u32, u32, u32, u8)> = Vec::new();

    let mut humans =
        world.query_filtered::<(&PawnId, &SimPosition, &crate::movement::Movable), With<Human>>();
    for (pawn_id, position, movable) in humans.iter(world) {
        rows.push(row(0, pawn_id, position, &movable.state));
    }
    let mut demons =
        world.query_filtered::<(&PawnId, &SimPosition, &crate::movement::Movable), With<Demon>>();
    for (pawn_id, position, movable) in demons.iter(world) {
        rows.push(row(1, pawn_id, position, &movable.state));
    }
    rows.sort_unstable();
    let moving = rows.iter().filter(|&&(.., state)| state != IDLE).count();

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
        moving,
    }
}

/// Код `MovableState::Idle` в строке отпечатка.
const IDLE: u8 = 0;

/// Позиция — в БИТАХ: сравнивать float'ы на равенство нельзя, а нужна ровно
/// побайтовая одинаковость.
fn row(
    species: u8,
    pawn_id: &PawnId,
    position: &SimPosition,
    state: &MovableState,
) -> (u8, u32, u32, u32, u8) {
    let state = match state {
        MovableState::Idle => IDLE,
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
