use rand::RngExt;

use super::spawn::pump_batch;
use super::wander::choose_target;
use super::*;
use crate::human::components::{HumanWanderTag, WanderHeading};
use crate::human::{HUMAN_BODY_RADIUS_MAX, HUMAN_SPEED_SPREAD_MAX};
use crate::map::osm::fixture::{building, rect};
use crate::map::osm::{MapData, PolyArea};
use crate::movement::point_in_cone;
use crate::rng::{PawnId, RngDomain, decision_stream};
use crate::settings::{HUMAN_WANDER_RANGE, MAP_SIZE, WANDER_CONE};
use crate::silhouette::Silhouettes;

/// Мир одного гуляющего: гейту хватает ресурса, системе — `Movable` и `Pace`.
fn pace_app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<HumanStyle>()
        .add_systems(Update, sync_human_pace.run_if(spread_changed));

    let entity = app
        .world_mut()
        .spawn((Human, HumanWanderTag, Movable::new(0.0), Pace(1.0)))
        .id();

    // первый прогон: Local = None → система отработает раз
    app.update();

    (app, entity)
}

/// После прогрева ползунок **Body radius** не должен менять скорости людей.
/// Часовой на `Movable::speed` — проверка лучше, чем сравнение скоростей,
/// потому что при неизменном `spread` система переписала бы ровно то же число,
/// и «лишний проход» иначе неотличим от его отсутствия.
#[test]
fn the_body_radius_slider_leaves_the_crowd_alone() {
    let (mut app, entity) = pace_app();
    {
        // `entity_mut` держится в переменной: `Mut<Movable>` заимствует
        // именно его, и временное значение умерло бы раньше заимствования
        let mut entity_mut = app.world_mut().entity_mut(entity);
        let mut movable = entity_mut.get_mut::<Movable>().unwrap();
        movable.speed = -1.0; // часовой
    }

    app.world_mut().resource_mut::<HumanStyle>().body_radius = HUMAN_BODY_RADIUS_MAX;
    app.update();

    // если система запустилась, -1.0 был бы перезаписан
    assert_eq!(
        app.world().entity(entity).get::<Movable>().unwrap().speed,
        -1.0
    );
}

/// Смена `spread` должна гонять систему.
#[test]
fn moving_the_spread_slider_retunes_the_crowd() {
    let (mut app, entity) = pace_app();
    let new_spread = HUMAN_SPEED_SPREAD_MAX;
    app.world_mut().resource_mut::<HumanStyle>().spread = new_spread;
    app.update();

    let expected = Pace(1.0).speed(HUMAN_WALK_SPEED, new_spread);
    assert_eq!(
        app.world().entity(entity).get::<Movable>().unwrap().speed,
        expected
    );
}

/// Запрет — на направление к демону и всё, что ближе 45° к нему.
#[test]
fn recoil_cone_catches_the_way_back() {
    let ban = Vec2::X;
    assert!(in_recoil_cone(Vec2::X, ban));
    assert!(in_recoil_cone(Vec2::from_angle(0.7), ban));
    assert!(in_recoil_cone(Vec2::from_angle(-0.7), ban));
}

/// Всё, что дальше 45°, — разрешено, включая ровно противоположное.
#[test]
fn recoil_cone_lets_the_rest_through() {
    let ban = Vec2::X;
    assert!(!in_recoil_cone(Vec2::from_angle(0.8), ban));
    assert!(!in_recoil_cone(Vec2::Y, ban));
    assert!(!in_recoil_cone(-Vec2::X, ban));
}

/// Точно на границе конуса цель разрешена: сравнение строгое.
#[test]
fn recoil_cone_boundary_is_allowed() {
    assert!(!in_recoil_cone(Vec2::from_angle(RECOIL_CONE), Vec2::X));
}

/// Население в пинах спавна. Дефолт мира стал 200 000 (план ×10,
/// docs/population-x10-plan.md), а пин проверяет функцию от seed, не
/// масштаб, — 5 000 держат тест быстрым и во флеше, и в flush.
const PIN_POPULATION: usize = 5_000;

/// Строки отпечатка населения: всё, что разыгрывается при спавне, в битах —
/// на float'ах сравнивать нечего, нужна побайтовая одинаковость.
fn spawn_fingerprint(world: &mut World) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    let mut query = world.query::<(&PawnId, &Transform, &Pace, &WanderHeading)>();
    let mut rows: Vec<_> = query
        .iter(world)
        .map(|(pawn_id, transform, pace, heading)| {
            (
                pawn_id.0,
                transform.translation.x.to_bits(),
                transform.translation.y.to_bits(),
                pace.0.to_bits(),
                heading.0.x.to_bits(),
                heading.0.y.to_bits(),
            )
        })
        .collect();
    rows.sort_unstable();
    rows
}

/// Отпечаток населения, расселённого одним флешем (`spawn_population`).
fn population_fingerprint(seed: u64, count: usize) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    let mut world = World::new();
    let navmesh = crate::navigation::Navmesh::default();
    spawn_population(
        &mut world.commands(),
        &navmesh,
        &PopulationBirth {
            spread: 0.3,
            world_seed: seed,
            count,
            silhouettes: &Silhouettes::default(),
        },
    );
    world.flush();
    spawn_fingerprint(&mut world)
}

/// Отпечаток населения, расселённого насосом (`pump_batch`) партиями по
/// `batch` — до исчерпания очереди.
fn pumped_population_fingerprint(
    seed: u64,
    count: usize,
    batch: usize,
) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    let mut world = World::new();
    let navmesh = crate::navigation::Navmesh::default();
    let mut pending = PendingPopulation::new(count);
    while !pump_batch(
        &mut world.commands(),
        &navmesh,
        0.3,
        seed,
        &Silhouettes::default(),
        &mut pending,
        batch,
    ) {}
    world.flush();
    spawn_fingerprint(&mut world)
}

/// Навмеш, где проходимы ровно перечисленные дырки в сплошной застройке.
fn navmesh_blocked_except(holes: Vec<Vec<Vec2>>) -> crate::navigation::Navmesh {
    let mut map = MapData::default();
    map.buildings
        .push(building(rect(Vec2::ZERO, crate::settings::MAP_SIZE), holes));
    let mut navmesh = crate::navigation::Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    navmesh
}

/// Население — чистая функция от seed. Это фундамент всего остального:
/// если спавн разъезжается, повторять симуляцию уже нечему.
#[test]
fn population_is_a_function_of_the_seed() {
    assert_eq!(
        population_fingerprint(7, PIN_POPULATION),
        population_fingerprint(7, PIN_POPULATION)
    );
}

/// ...и при этом seed что-то значит: разные seed дают разное население.
#[test]
fn different_seeds_give_different_populations() {
    assert_ne!(
        population_fingerprint(7, PIN_POPULATION),
        population_fingerprint(8, PIN_POPULATION)
    );
}

/// Пин Фазы 1 плана ×10 («партиями == не партиями», бит-в-бит): насос
/// режет `0..count` на партии, но личный жребий выводится из номера пешки,
/// а позиции считаются разом до всякого разрезания — батчингу нечем
/// сместить хоть один бросок. Одна партия (batch = count), десять (50) и
/// немедленный спавн обязаны дать один и тот же отпечаток.
#[test]
fn batching_does_not_change_the_population() {
    let whole = population_fingerprint(7, 500);
    let one_batch = pumped_population_fingerprint(7, 500, 500);
    let ten_batches = pumped_population_fingerprint(7, 500, 50);
    assert_eq!(whole, one_batch, "партия «всё сразу» != немедленный спавн");
    assert_eq!(whole, ten_batches, "десять партий != немедленный спавн");
}

/// Пустой навмеш не спавнит никого: приложение остаётся живым с
/// единственной строкой в логе.
#[test]
fn population_refuses_a_navmesh_without_passable_tiles() {
    let mut world = World::new();
    let navmesh = navmesh_blocked_except(vec![]);
    spawn_population(
        &mut world.commands(),
        &navmesh,
        &PopulationBirth {
            spread: 0.3,
            world_seed: 7,
            count: 4,
            silhouettes: &Silhouettes::default(),
        },
    );
    world.flush();
    let count = world.query::<&Human>().iter(&world).count();
    assert_eq!(count, 0, "пустой навмеш не должен спавнить никого");
}

/// Сплошь заблокированный навмеш (одна дырка в тайл) — все люди в тайле
/// дырки, но не более чем за максимум попыток.
#[test]
fn population_falls_back_to_a_grid_scan() {
    let mut world = World::new();
    let hole_center = crate::settings::MAP_SIZE * 0.25;
    let hole_tile = crate::grid::world_to_tile(hole_center);
    let hole_tile_size = crate::grid::navtile_size();
    let hole_rect = rect(hole_center, hole_center + hole_tile_size);
    let navmesh = navmesh_blocked_except(vec![hole_rect]);
    spawn_population(
        &mut world.commands(),
        &navmesh,
        &PopulationBirth {
            spread: 0.3,
            world_seed: 7,
            count: 4,
            silhouettes: &Silhouettes::default(),
        },
    );
    world.flush();

    let count = world.query::<&Human>().iter(&world).count();
    assert_eq!(count, 4, "все люди должны быть спавнены даже со сканом");

    let hole_center_world = crate::grid::tile_center(hole_tile);
    let mut query = world.query::<&Transform>();
    for transform in query.iter(&world) {
        let position = transform.translation.truncate();
        assert_eq!(
            position, hole_center_world,
            "все люди должны быть в центре дырки"
        );
    }
}

/// Вершина контура — детерминированно, т.к. `building_target` индексирует
/// `outer` по случайному числу, а в тесте контур из одной вершины.
fn building_at(corner: Vec2) -> PolyArea {
    building(vec![corner], vec![])
}

/// На первую цель после спавна: жребий 80/20 не разыгрывается, идёт прогулка
/// поблизости, независимо от расстояния до зданий.
#[test]
fn the_first_target_after_spawn_is_a_nearby_stroll() {
    let mut map = MapData::default();
    let home_far = MAP_SIZE / 2.0 + Vec2::X * 300.0;
    map.buildings.push(building_at(home_far));

    let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
    let position = MAP_SIZE / 2.0;
    let heading = Vec2::X;

    for _ in 0..50 {
        let target = choose_target(&map, &mut rng, position, heading, None, true)
            .expect("прогулка поблизости всегда успешна");
        assert!(
            target.distance(position) <= HUMAN_WANDER_RANGE.1,
            "цель должна быть в пределах диапазона прогулки, а не у дома в 300 м"
        );
    }
}

/// Паника перебивает `is_first_wander`: если `ban` установлена, ищем дом,
/// независимо от `is_first_wander = true`.
#[test]
fn panic_forces_an_errand_over_the_first_wander_tag() {
    let mut map = MapData::default();
    let home_ahead = MAP_SIZE / 2.0 + Vec2::X * 300.0;
    map.buildings.push(building_at(home_ahead));

    let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
    let position = MAP_SIZE / 2.0;
    let heading = Vec2::X;
    let ban = Some(-Vec2::X); // демон за спиной

    let target = choose_target(&map, &mut rng, position, heading, ban, true)
        .expect("дом по курсу — всегда успешен");
    assert_eq!(target, home_ahead, "цель должна быть вершиной дома");
}

/// Если вся выборка зданий в запретном конусе или ближе `RECOIL_MIN_ERRAND`,
/// `None` — вызывающий обязан пропустить пешку, сохранив паническое состояние.
#[test]
fn a_ban_that_kills_the_whole_sample_gives_no_target() {
    let mut map = MapData::default();
    let home_back = MAP_SIZE / 2.0 - Vec2::X * 300.0; // только назад
    map.buildings.push(building_at(home_back));

    let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
    let position = MAP_SIZE / 2.0;
    let heading = Vec2::X;
    let ban = Some(-Vec2::X);

    let target = choose_target(&map, &mut rng, position, heading, ban, false);
    assert_eq!(
        target, None,
        "единственный дом назад и в конусе — должен быть отсеян"
    );
}

/// В городе без зданий прогулка проверяется на запрет: если конус запретит
/// её, `None` отправляет пешку в холостой цикл, иначе цель разрешена.
#[test]
fn a_city_without_buildings_strolls_outside_the_ban_cone() {
    let map = MapData::default(); // нет зданий
    let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
    let position = MAP_SIZE / 2.0;
    let heading = Vec2::X;
    let ban = Vec2::X;

    let mut had_none = false;
    for _ in 0..200 {
        if let Some(target) = choose_target(&map, &mut rng, position, heading, Some(ban), false) {
            let direction = (target - position).normalize();
            assert!(
                !in_recoil_cone(direction, ban),
                "цель должна быть вне запретного конуса"
            );
        } else {
            had_none = true;
        }
    }
    assert!(had_none, "запрет должен отсечь хотя бы один вызов из 200");
}

/// Политика не должна съедать или добавлять броски поверх `point_in_cone`:
/// жребий 80/20 и сама функция выбора точки в конусе занимают ровно по
/// одному `random_range`, и тест страхует сдвиг потока решений.
#[test]
fn the_stroll_branch_spends_exactly_two_draws() {
    let map = MapData::default(); // нет зданий
    let position = MAP_SIZE / 2.0;
    let heading = Vec2::X;

    // первый поток: вызовем choose_target, потом `random_range` на нём
    let mut rng1 = decision_stream(1, RngDomain::Human, 0, 1);
    let _ = choose_target(&map, &mut rng1, position, heading, None, true);
    let draw1: f32 = rng1.random_range(0.0..1.0);

    // второй поток: `point_in_cone` напрямую, потом тот же `random_range`
    let mut rng2 = decision_stream(1, RngDomain::Human, 0, 1);
    let _ = point_in_cone(
        &mut rng2,
        position,
        heading,
        WANDER_CONE,
        HUMAN_WANDER_RANGE,
    );
    let draw2: f32 = rng2.random_range(0.0..1.0);

    assert_eq!(draw1.to_bits(), draw2.to_bits(), "потоки должны совпадать");
}
