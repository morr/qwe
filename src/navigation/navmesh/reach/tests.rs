use super::*;
use crate::grid::{navtile_size, world_to_tile};
use crate::map::osm::fixture::{building, rect};
use crate::settings::MAP_SIZE;

#[test]
fn passable_from_reports_a_fully_blocked_grid() {
    let mut map = MapData::default();
    map.buildings
        .push(building(rect(Vec2::ZERO, MAP_SIZE), vec![]));
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    assert!(navmesh.passable_from(IVec2::ZERO).is_none());
}

#[test]
fn passable_from_wraps_past_the_start() {
    let mut map = MapData::default();
    let hole_center = MAP_SIZE * 0.25;
    let hole_tile_size = navtile_size();
    let hole_rect = rect(hole_center, hole_center + hole_tile_size);
    map.buildings
        .push(building(rect(Vec2::ZERO, MAP_SIZE), vec![hole_rect]));
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let hole_tile = world_to_tile(hole_center);
    let from_after_hole = IVec2::new(hole_tile.x + 1, hole_tile.y);
    assert_eq!(
        navmesh.passable_from(from_after_hole),
        Some(hole_tile),
        "скан после дырки обязан вернуть саму дырку"
    );
    assert_eq!(
        navmesh.passable_from(IVec2::ZERO),
        Some(hole_tile),
        "скан от нуля вернёт тот же тайл"
    );
}

/// Дом-кольцо: двор внутри проходим по заливке, но снаружи в него не войти.
fn courtyard_map() -> MapData {
    let mut map = MapData::default();
    map.buildings.push(building(
        rect(Vec2::new(180.0, 180.0), Vec2::new(220.0, 220.0)),
        vec![rect(Vec2::new(195.0, 195.0), Vec2::new(205.0, 205.0))],
    ));
    map
}

/// Замкнутый двор недостижим, и прунинг его закрывает — ради этого он и есть:
/// A* к недостижимой цели обходит всю связную область.
#[test]
fn pruning_closes_a_courtyard_no_one_can_walk_into() {
    let map = courtyard_map();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let yard = world_to_tile(Vec2::new(200.0, 200.0));
    let street = world_to_tile(Vec2::new(100.0, 100.0));
    assert!(navmesh.is_passable(yard.x, yard.y), "двор залит проходимым");

    let pruned = navmesh.prune_unreachable(Vec2::new(100.0, 100.0));

    assert!(pruned > 0, "двор должен быть срезан");
    assert!(!navmesh.is_passable(yard.x, yard.y), "двор закрыт");
    assert!(navmesh.is_passable(street.x, street.y), "улица цела");
}

/// Старт на непроходимом тайле не режет ничего. Это случай
/// `no clear spot for portal`: обход от такого старта не достигает ни одного
/// тайла, и «залить и вырезать недостигнутое» вырезало бы всю карту.
#[test]
fn pruning_from_a_blocked_start_cuts_nothing() {
    let map = courtyard_map();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let street = world_to_tile(Vec2::new(100.0, 100.0));
    let yard = world_to_tile(Vec2::new(200.0, 200.0));

    // точка в стене кольца
    let pruned = navmesh.prune_unreachable(Vec2::new(185.0, 200.0));

    assert_eq!(pruned, 0, "срезать нечего");
    assert!(navmesh.is_passable(street.x, street.y), "улица цела");
    assert!(navmesh.is_passable(yard.x, yard.y), "двор цел");
}

/// Старт за краем сетки — тот же ранний выход.
#[test]
fn pruning_from_outside_the_grid_cuts_nothing() {
    let map = courtyard_map();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let street = world_to_tile(Vec2::new(100.0, 100.0));

    let pruned = navmesh.prune_unreachable(Vec2::new(-50.0, -50.0));

    assert_eq!(pruned, 0, "срезать нечего");
    assert!(navmesh.is_passable(street.x, street.y), "улица цела");
}
