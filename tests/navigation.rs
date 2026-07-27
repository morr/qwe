//! Юнит-тесты навигации: заполнение navmesh из данных карты и A*.

use bevy::math::IVec2;

use qwe::grid::{tile_center, world_to_tile};
use qwe::map::data;
use qwe::navigation::{Navmesh, astar_pathfinding};
use qwe::settings::{GRID_SIZE, PORTAL_POS};

fn filled_navmesh() -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_map();
    navmesh
}

#[test]
fn buildings_are_impassable() {
    let navmesh = filled_navmesh();
    for building in data::buildings() {
        let center_tile = world_to_tile(building.center());
        assert!(
            !navmesh.is_passable(center_tile.x, center_tile.y),
            "center of building at {:?} should be impassable",
            building.min
        );
    }
}

#[test]
fn pond_is_impassable_and_park_is_passable() {
    let navmesh = filled_navmesh();
    let pond_tile = world_to_tile(data::POND_CENTER);
    assert!(!navmesh.is_passable(pond_tile.x, pond_tile.y));

    let plaza_tile = world_to_tile(data::PARK_PLAZA);
    assert!(navmesh.is_passable(plaza_tile.x, plaza_tile.y));

    let portal_tile = world_to_tile(PORTAL_POS);
    assert!(navmesh.is_passable(portal_tile.x, portal_tile.y));
}

#[test]
fn out_of_bounds_is_impassable() {
    let navmesh = filled_navmesh();
    assert!(!navmesh.is_passable(-1, 0));
    assert!(!navmesh.is_passable(0, -1));
    assert!(!navmesh.is_passable(GRID_SIZE.x, 0));
    assert!(!navmesh.is_passable(0, GRID_SIZE.y));
}

#[test]
fn astar_finds_path_around_building() {
    let navmesh = filled_navmesh();
    // Первая многоэтажка: путь с юга на север здания должен его обойти
    let building = data::SLABS[0];
    let start = world_to_tile(building.center() - bevy::math::Vec2::new(0.0, building.size.y));
    let end = world_to_tile(building.center() + bevy::math::Vec2::new(0.0, building.size.y));

    let path = astar_pathfinding(&navmesh, start, end).expect("path should exist");
    assert!(path.len() > 2);
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
    let navmesh = filled_navmesh();
    let target = world_to_tile(data::SLABS[0].center());
    let start = world_to_tile(PORTAL_POS);
    assert!(astar_pathfinding(&navmesh, start, target).is_none());
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

#[test]
fn grid_roundtrip() {
    let tile = IVec2::new(123, 45);
    assert_eq!(world_to_tile(tile_center(tile)), tile);
}
