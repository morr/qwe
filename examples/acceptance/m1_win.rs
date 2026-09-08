//! Приёмка вехи M1 «Срез и скверна»: цепочка районов от портала к сердцу,
//! один бастион на пути и один Громила — прогон без окна, до `Outcome::Won`.
//!
//! Сцена — `district_city` (`map/osm/fixture.rs`): шесть дворов цепочкой,
//! единственный мост через воду, сердце в последнем дворе; портал в первом,
//! семь переходов до сердца. Бастион ставится в район южного берега — в тот,
//! который держит мост. Пока он стоит, скверна не идёт на север; ломает его
//! Громила, призванный на тике [`SUMMON_TICK`] (сообщение `SummonRequested`
//! шага 9, души выданы заранее — иначе призыв не пройдёт).
//!
//! Три утверждения, как в `VISION.md` для M1:
//!
//! 1. **Сердце падает.** `Outcome::Won` наступает до [`MAX_TICKS`].
//! 2. **Тик победы повторяется.** Второй прогон на том же seed выигрывает на
//!    том же тике — вся осада (перепись, скверна, осада, удар, исход) внутри
//!    контракта повтора.
//! 3. **Мост — единственный путь.** Ни один район северного берега не
//!    осквернён раньше района бастиона: скверна прошла через мост, а не мимо.
//!
//! Не на Туле: на настоящем городе прогон стоит порядка секунды реального
//! времени на тик (`determinism_replay.rs`), а цепочка из семи районов при 30 с
//! на район — это 13 000+ тиков и дорога Громилы. Здесь население — сотни, и
//! прогон идёт минутами. Это зародыш стенда баланса из M7: время до сердца
//! как число, а не как впечатление от живой Тулы.
//!
//! ```text
//! cargo run --example m1_win
//! ```

use bevy::prelude::*;

use qwe::bastion::{BastionDestroyed, BastionSite, BastionSites, fill::closeness};
use qwe::corruption::{Corruption, DistrictCorrupted};
use qwe::demon::{BruteTag, Demon, DemonKind};
use qwe::determinism::SimTick;
use qwe::determinism::replay::{Progress, replay_app_with, run_to_tick};
use qwe::district::{DistrictId, Districts};
use qwe::map::osm::fixture::{DistrictCity, district_city};
use qwe::map::osm::model::BastionKind;
use qwe::navigation::Navmesh;
use qwe::outcome::Outcome;
use qwe::souls::{Souls, SummonRequested};

/// Тик, на котором пишется призыв Громилы. Не нулевой: первые тики — залп Бесов
/// и раздача `PawnId`, призыв должен встать в очередь за ними, как в игре.
const SUMMON_TICK: u64 = 10;

/// Душ, выданных перед призывом: цена Громилы (25) с запасом, чтобы стенд не
/// зависел от того, сколько людей успели съесть Бесы у портала.
const SOULS_GRANT: u32 = 100;

/// Потолок прогона — четверть часа симуляции (64 тика в секунду). Семь
/// переходов по 30 с плюс дорога Громилы к мосту укладываются с запасом; если
/// нет — сердце не падает, и это провал утверждения 1, а не долгий прогон.
const MAX_TICKS: u64 = 60_000;

/// Шаг прокрутки между строками прогресса — 20 с симуляции.
const CHUNK_TICKS: u64 = 1_280;

/// Людей во дворах. Сотни, не тысячи: каждая заявка на путь считается плоским
/// A* на полноразмерной сетке, и это — цена тика.
const POPULATION: usize = 200;

/// Северный берег — всё, что за водой (`district_city`: вода 800–900 м).
const NORTH_BANK_Y: f32 = 900.0;

const SEED: u64 = 1;

/// Журнал прогона: на каком тике какой район пал и когда сломали бастион.
#[derive(Resource, Default)]
struct SiegeLog {
    corrupted: Vec<(u64, DistrictId)>,
    bastion_fell: Option<u64>,
}

/// Итог одного прогона — то, что сравнивается между прогонами и печатается.
#[derive(Debug, Clone, PartialEq)]
struct RunReport {
    outcome: Outcome,
    /// Тик, на котором сломан бастион моста.
    bastion_fell: Option<u64>,
    /// Тик первого осквернённого района северного берега.
    first_north: Option<u64>,
    /// Тик осквернения района бастиона.
    bastion_district_fell: Option<u64>,
    killed: usize,
    brutes_alive: usize,
    wall_secs: f32,
}

fn main() {
    println!("progon 1: seed {SEED}");
    let first = run(SEED);
    println!("\nprogon 2: тот же seed");
    let second = run(SEED);

    println!("\n{:<26} {:>14} {:>14}", "", "прогон 1", "прогон 2");
    row(
        "исход",
        &describe(&first.outcome),
        &describe(&second.outcome),
    );
    row(
        "бастион сломан, тик",
        &opt(first.bastion_fell),
        &opt(second.bastion_fell),
    );
    row(
        "район бастиона пал, тик",
        &opt(first.bastion_district_fell),
        &opt(second.bastion_district_fell),
    );
    row(
        "первый район за мостом",
        &opt(first.first_north),
        &opt(second.first_north),
    );
    row(
        "съедено",
        &first.killed.to_string(),
        &second.killed.to_string(),
    );
    row(
        "Громил живо",
        &first.brutes_alive.to_string(),
        &second.brutes_alive.to_string(),
    );
    row(
        "реальное время, с",
        &format!("{:.0}", first.wall_secs),
        &format!("{:.0}", second.wall_secs),
    );
    println!();

    let mut failures = 0;
    failures += check(
        "сердце падает до потолка прогона",
        matches!(first.outcome, Outcome::Won { .. }),
    );
    failures += check(
        "тик победы повторяется на том же seed",
        first.outcome == second.outcome,
    );
    failures += check(
        "скверна прошла через мост: северный берег пал после района бастиона",
        bridge_first(&first) && bridge_first(&second),
    );

    if failures > 0 {
        std::process::exit(1);
    }
    println!("\nOK: M1 держится — сердце падает, тик победы воспроизводим, путь один — мост.");
}

fn run(seed: u64) -> RunReport {
    let started = std::time::Instant::now();
    let city = district_city();
    let navmesh = build_navmesh(&city);
    let districts = Districts::build(&navmesh, city.portal, city.heart);
    let bastion_district = districts
        .district_at(city.south_bank)
        .expect("южный берег не попал ни в один район");
    let sites = bridge_bastion(&districts, &navmesh, city.south_bank, bastion_district);
    println!(
        "  районов {}, портал {} переходов от сердца, бастион в районе {bastion_district}",
        districts.len(),
        districts.districts[districts.portal.expect("район портала") as usize]
            .dist_to_heart
            .expect("портал не связан с сердцем"),
    );
    // центроиды нужны отчёту после прогона — ресурс уедет в приложение
    let north_bank: Vec<DistrictId> = districts
        .districts
        .iter()
        .enumerate()
        .filter(|(_, district)| district.centroid.y > NORTH_BANK_Y)
        .map(|(id, _)| id as DistrictId)
        .collect();

    let mut app = replay_app_with(city.map, navmesh, city.portal, seed, POPULATION, |app| {
        app.insert_resource(districts)
            .insert_resource(sites)
            .init_resource::<SiegeLog>()
            .add_observer(
                |event: On<DistrictCorrupted>, tick: Res<SimTick>, mut log: ResMut<SiegeLog>| {
                    log.corrupted.push((tick.0, event.district));
                },
            )
            .add_observer(
                |_event: On<BastionDestroyed>, tick: Res<SimTick>, mut log: ResMut<SiegeLog>| {
                    log.bastion_fell.get_or_insert(tick.0);
                },
            );
    });

    run_to_tick(&mut app, SUMMON_TICK, &[1], Progress::Silent);
    app.world_mut().resource_mut::<Souls>().earned += SOULS_GRANT;
    app.world_mut().write_message(SummonRequested {
        kind: DemonKind::Brute,
    });
    println!("  тик {SUMMON_TICK}: +{SOULS_GRANT} душ, призыв Громилы");

    let mut next_report = 0;
    while app.world().resource::<Outcome>().is_running() {
        let tick = app.world().resource::<SimTick>().0;
        if tick >= MAX_TICKS {
            break;
        }
        run_to_tick(
            &mut app,
            (tick + CHUNK_TICKS).min(MAX_TICKS),
            &[1],
            Progress::Silent,
        );
        let tick = app.world().resource::<SimTick>().0;
        if tick / (CHUNK_TICKS * 8) > next_report || !app.world().resource::<Outcome>().is_running()
        {
            next_report = tick / (CHUNK_TICKS * 8);
            let world = app.world_mut();
            let corruption = world.resource::<Corruption>();
            let corrupted = corruption.progress.iter().filter(|&&p| p >= 1.0).count();
            let to_heart = corruption.to_heart;
            let brutes = world
                .query_filtered::<(), (With<Demon>, With<BruteTag>)>()
                .iter(world)
                .len();
            println!(
                "  тик {tick:>6} ({:>4.0} с): осквернено {corrupted}, до сердца {}, Громил {brutes}, {:.0} с реального",
                tick as f32 / 64.0,
                to_heart.map_or("-".to_string(), |hops| hops.to_string()),
                started.elapsed().as_secs_f32(),
            );
        }
    }

    let world = app.world_mut();
    let log = world.resource::<SiegeLog>();
    let tick_of = |district: DistrictId| {
        log.corrupted
            .iter()
            .find(|&&(_, id)| id == district)
            .map(|&(tick, _)| tick)
    };
    let report = RunReport {
        outcome: *world.resource::<Outcome>(),
        bastion_fell: log.bastion_fell,
        first_north: north_bank.iter().filter_map(|&id| tick_of(id)).min(),
        bastion_district_fell: tick_of(bastion_district),
        killed: world.resource::<qwe::telemetry::Telemetry>().killed,
        brutes_alive: world
            .query_filtered::<(), (With<Demon>, With<BruteTag>)>()
            .iter(world)
            .len(),
        wall_secs: started.elapsed().as_secs_f32(),
    };
    println!("  {}", describe(&report.outcome));
    report
}

/// Один бастион-твердыня на южном берегу у моста; здоровье — по близости к
/// сердцу, как у настоящих мест (`bastion::plan_sites`).
fn bridge_bastion(
    districts: &Districts,
    navmesh: &Navmesh,
    at: Vec2,
    district: DistrictId,
) -> BastionSites {
    let max_dist = districts
        .districts
        .iter()
        .filter_map(|district| district.dist_to_heart)
        .max()
        .unwrap_or(0);
    let pos = navmesh.tile_center(navmesh.to_tile(at));
    assert!(
        navmesh.is_passable(navmesh.to_tile(at).x, navmesh.to_tile(at).y),
        "точка бастиона {at:?} непроходима"
    );
    BastionSites {
        sites: vec![BastionSite {
            pos,
            kind: BastionKind::Stronghold,
            district,
            closeness: closeness(
                districts.districts[district as usize].dist_to_heart,
                max_dist,
            ),
        }],
        tagged: 0,
        dropped: 0,
    }
}

/// Та же последовательность, что в потоке загрузки: заливка и прунинг от
/// портала. Сердце снапать не нужно — центр двора проходим по построению.
fn build_navmesh(city: &DistrictCity) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&city.map);
    navmesh.prune_unreachable(navmesh.to_tile(city.portal));
    navmesh
}

/// Северный берег пал после района бастиона — или не пал вовсе (тогда
/// утверждение о мосте не нарушено, а провал — у утверждения о сердце).
fn bridge_first(report: &RunReport) -> bool {
    match (report.first_north, report.bastion_district_fell) {
        (Some(north), Some(bridge)) => north > bridge,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Running => "Running".to_string(),
        Outcome::Won { tick } => format!("Won @ {tick} ({:.0} с)", *tick as f32 / 64.0),
        Outcome::Lost { tick, reason } => format!("Lost {reason:?} @ {tick}"),
    }
}

fn opt(tick: Option<u64>) -> String {
    tick.map_or("-".to_string(), |tick| tick.to_string())
}

fn row(name: &str, first: &str, second: &str) {
    println!("{name:<26} {first:>14} {second:>14}");
}

fn check(what: &str, passed: bool) -> u32 {
    println!("{} {what}", if passed { "  ok  " } else { "FAILED" });
    u32::from(!passed)
}
