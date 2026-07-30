//! Юнит-тесты навигации: A* по синтетическому navmesh и заполнение
//! navmesh из рукотворной `MapData` (без сети).

use bevy::math::{IVec2, Vec2};

use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
use qwe::navigation::{Navmesh, PathfindingAlgorithm, find_path, line_of_sight};

fn astar_pathfinding(navmesh: &Navmesh, start: IVec2, end: IVec2) -> Option<Vec<IVec2>> {
    find_path(navmesh, start, end, PathfindingAlgorithm::Astar)
}
use qwe::settings::GRID_SIZE;

/// Navmesh с одним прямоугольным препятствием (в тайлах, включительно).
fn navmesh_with_block(min: IVec2, max: IVec2) -> Navmesh {
    let mut navmesh = Navmesh::default();
    for x in min.x..=max.x {
        for y in min.y..=max.y {
            navmesh.set_passable(x, y, false);
        }
    }
    navmesh
}

fn rect_ring(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

#[test]
fn out_of_bounds_is_impassable() {
    let navmesh = Navmesh::default();
    assert!(!navmesh.is_passable(-1, 0));
    assert!(!navmesh.is_passable(0, -1));
    assert!(!navmesh.is_passable(GRID_SIZE.x, 0));
    assert!(!navmesh.is_passable(0, GRID_SIZE.y));
}

#[test]
fn line_of_sight_is_blocked_by_a_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let west = tile_center(IVec2::new(95, 105));
    let east = tile_center(IVec2::new(115, 105));
    let north = tile_center(IVec2::new(105, 115));

    // сквозь здание — нет; вдоль его южной стороны и по вертикали — да
    assert!(!line_of_sight(&navmesh, west, east));
    assert!(!line_of_sight(&navmesh, west, north));
    assert!(line_of_sight(
        &navmesh,
        tile_center(IVec2::new(95, 99)),
        tile_center(IVec2::new(115, 99))
    ));
    assert!(line_of_sight(&navmesh, west, west));
}

#[test]
fn astar_finds_path_around_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let start = IVec2::new(105, 95);
    let end = IVec2::new(105, 115);

    let path = astar_pathfinding(&navmesh, start, end).expect("path should exist");
    assert!(
        path.len() > 20,
        "path must detour, got {} tiles",
        path.len()
    );
    assert_eq!(*path.first().unwrap(), start);
    assert_eq!(*path.last().unwrap(), end);
    for tile in &path {
        assert!(
            navmesh.is_passable(tile.x, tile.y),
            "path goes through impassable tile {tile:?}"
        );
    }
}

#[test]
fn astar_to_impassable_target_returns_none() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    assert!(astar_pathfinding(&navmesh, IVec2::new(90, 90), IVec2::new(105, 105)).is_none());
}

#[test]
fn astar_does_not_cut_corners() {
    let mut navmesh = Navmesh::default();
    // одиночный блок: диагональ вокруг угла запрещена
    navmesh.set_passable(10, 10, false);
    let path = astar_pathfinding(&navmesh, IVec2::new(9, 10), IVec2::new(11, 10))
        .expect("path should exist");
    for pair in path.windows(2) {
        let step = (pair[1] - pair[0]).abs();
        if step == IVec2::ONE {
            assert!(
                navmesh.is_passable(pair[0].x, pair[1].y)
                    && navmesh.is_passable(pair[1].x, pair[0].y),
                "diagonal step cuts corner at {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}

/// Каждый алгоритм обходит препятствие валидным путём с теми же концами.
#[test]
fn every_algorithm_finds_valid_path_around_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let start = IVec2::new(105, 95);
    let end = IVec2::new(105, 115);

    for algorithm in [
        PathfindingAlgorithm::Astar,
        PathfindingAlgorithm::Dijkstra,
        PathfindingAlgorithm::Fringe,
        PathfindingAlgorithm::Bfs,
    ] {
        let path = find_path(&navmesh, start, end, algorithm)
            .unwrap_or_else(|| panic!("{algorithm:?} should find a path"));
        assert_eq!(*path.first().unwrap(), start, "{algorithm:?}");
        assert_eq!(*path.last().unwrap(), end, "{algorithm:?}");
        for pair in path.windows(2) {
            let step = (pair[1] - pair[0]).abs();
            assert!(
                step.max_element() <= 1,
                "{algorithm:?} makes a non-adjacent step {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
        for tile in &path {
            assert!(
                navmesh.is_passable(tile.x, tile.y),
                "{algorithm:?} path goes through impassable tile {tile:?}"
            );
        }
    }
}

#[test]
fn grid_roundtrip() {
    let tile = IVec2::new(123, 45);
    assert_eq!(world_to_tile(tile_center(tile)), tile);
}

/// Рукотворная карта: здание с двором-дыркой, вода с мостом, стена.
#[test]
fn fill_from_mapdata_blocks_and_carves() {
    let map = MapData {
        buildings: vec![PolyArea {
            outer: rect_ring(Vec2::new(100.0, 100.0), Vec2::new(160.0, 160.0)),
            holes: vec![rect_ring(Vec2::new(120.0, 120.0), Vec2::new(140.0, 140.0))],
            kind: AreaKind::Building,
            height: Some(15.0),
            entrances: Vec::new(),
        }],
        water: vec![PolyArea {
            outer: rect_ring(Vec2::new(300.0, 0.0), Vec2::new(340.0, 400.0)),
            holes: Vec::new(),
            kind: AreaKind::Water,
            height: None,
            entrances: Vec::new(),
        }],
        parks: Vec::new(),
        woods: Vec::new(),
        grass: Vec::new(),
        sand: Vec::new(),
        roads: vec![RoadLine {
            points: vec![Vec2::new(280.0, 200.0), Vec2::new(360.0, 200.0)],
            width: 8.0,
            class: RoadClass::Street,
            bridge: true,
            passage: false,
        }],
        walls: vec![WallLine {
            points: vec![Vec2::new(500.0, 100.0), Vec2::new(500.0, 200.0)],
            width: 3.0,
        }],
        trees: Vec::new(),
        tree_appears_at: Vec::new(),
    };

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    // здание непроходимо, двор-дырка — проходим
    let building_tile = world_to_tile(Vec2::new(110.0, 110.0));
    assert!(!navmesh.is_passable(building_tile.x, building_tile.y));
    let courtyard_tile = world_to_tile(Vec2::new(130.0, 130.0));
    assert!(navmesh.is_passable(courtyard_tile.x, courtyard_tile.y));

    // вода непроходима, мост через неё — проходим
    let water_tile = world_to_tile(Vec2::new(320.0, 100.0));
    assert!(!navmesh.is_passable(water_tile.x, water_tile.y));
    let bridge_tile = world_to_tile(Vec2::new(320.0, 200.0));
    assert!(navmesh.is_passable(bridge_tile.x, bridge_tile.y));

    // стена непроходима
    let wall_tile = world_to_tile(Vec2::new(500.0, 150.0));
    assert!(!navmesh.is_passable(wall_tile.x, wall_tile.y));

    // и путь через реку существует и идёт по мосту
    let path = astar_pathfinding(
        &navmesh,
        world_to_tile(Vec2::new(280.0, 200.0)),
        world_to_tile(Vec2::new(360.0, 200.0)),
    )
    .expect("path across the bridge should exist");
    for tile in &path {
        assert!(navmesh.is_passable(tile.x, tile.y));
    }
}
