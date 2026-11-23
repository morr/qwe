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

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use qwe::determinism::replay::{Fingerprint, Progress, replay_app, run_to_tick};
use qwe::grid::tile_center;
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
    navmesh.prune_unreachable(yard.portal);
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

/// Сводка мира для [`a_restart_rebuilds_the_same_world`]: сколько сущностей
/// носит каждый набор компонентов проекта, и значение каждого ресурса.
///
/// Отпечаток прогона видит ровно то, что в него входит (`PawnId`, позиции,
/// состояние движения, счётчики исходов). Трупы, кровь, чанки и всё состояние,
/// живущее в ресурсах (расписания, очереди), ему невидимы — здесь они строкой.
/// Не отражённые ресурсы (`Backend`, `FarCrowdField`, `DestinationClaims`…)
/// входят именем с пометкой: пин видит, что ресурс пережил рестарт, но не его
/// содержание.
///
/// Два исключения — то, что пережить рестарт вправе: заводные часы (накопленное
/// время прогона, не состояние мира) и души — `SoulMote` гаснет по своему
/// таймеру, `restart.rs` их не трогает.
fn world_rows(world: &World) -> Vec<String> {
    let components = world.components();
    let mut archetypes: std::collections::BTreeMap<String, usize> = Default::default();
    for archetype in world.archetypes().iter() {
        let count = archetype.entities().len();
        if count == 0 {
            continue;
        }
        let mut names: Vec<String> = archetype
            .components()
            .iter()
            .filter_map(|id| components.get_info(*id))
            .map(|info| info.name().to_string())
            .filter(|name| name.starts_with("qwe::"))
            .collect();
        if names.is_empty() || names.contains(&"qwe::human::soul::SoulMote".to_string()) {
            continue;
        }
        names.sort();
        *archetypes.entry(names.join(", ")).or_default() += count;
    }

    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut resources: Vec<(String, String)> = Vec::new();
    for (id, entity) in world.resource_entities().iter() {
        let Some(info) = world.components().get_info(id) else {
            continue;
        };
        let name = info.name().to_string();
        if name.starts_with("bevy_time::time::Time") {
            continue;
        }
        let value = info
            .type_id()
            .and_then(|type_id| registry.get(type_id))
            .and_then(|registration| registration.data::<ReflectComponent>())
            .and_then(|reflect| reflect.reflect(world.get_entity(entity).expect("resource entity")))
            .map(|value| format!("{value:?}"))
            .unwrap_or_else(|| "<not reflected>".to_string());
        resources.push((name, value));
    }
    resources.sort();

    let mut rows: Vec<String> = archetypes
        .iter()
        .map(|(names, count)| format!("{count:6}  {names}"))
        .collect();
    rows.extend(
        resources
            .iter()
            .map(|(name, value)| format!("{name} = {value}")),
    );
    rows
}

/// «Нажал R — и мир собрался заново»: прогон до тика N, `RestartEvent`, снова
/// до N — и весь мир, каким его видит ECS, а не только отпечаток прогона.
///
/// [`a_restart_replays_the_run`] держит членство сбросов `WorldStarted` снаружи
/// ровно настолько, насколько расходится то, что входит в отпечаток: сброс,
/// затрагивающий только трупы, кровь или ресурс вне счётчиков исходов, ему
/// не виден (проверено мутацией: убрать `CorpseTag` из деспавна `on_restart` —
/// трупы прошлого прогона переживают рестарт, отпечаток в отпечаток, а здесь
/// лишняя строка с `qwe::human::CorpseTag`).
///
/// Второй прогон идёт в ТОМ ЖЕ `App` — как и у отпечатка, поэтому ловит
/// состояние, пережившее сброс, а не отличия двух сборок.
#[test]
fn a_restart_rebuilds_the_same_world() {
    let mut app = app(1);
    let first = run_to_tick(&mut app, TICKS, &[1], Progress::Silent);
    assert!(
        first.killed > 0,
        "в прогоне не было смертей — после рестарта не с чем сравнивать трупы и кровь: {first:?}"
    );
    let fresh = world_rows(app.world());

    app.world_mut().trigger(RestartEvent::default());
    // рестарт живёт в обсервере — даём кадр на применение команд
    app.update();
    run_to_tick(&mut app, TICKS, &[1], Progress::Silent);
    let restarted = world_rows(app.world());

    let mut drift: Vec<String> = Vec::new();
    for row in &fresh {
        if !restarted.contains(row) {
            drift.push(format!("- {row}"));
        }
    }
    for row in &restarted {
        if !fresh.contains(row) {
            drift.push(format!("+ {row}"));
        }
    }
    assert!(
        drift.is_empty(),
        "после рестарта мир не такой, как свежий прогон на том же тике:\n{}",
        drift.join("\n")
    );
}
