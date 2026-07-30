//! Юнит-тесты пространственной сетки.

use bevy::prelude::*;

use qwe::demon::Demon;
use qwe::spatial::{DemonDangerMap, SpatialGrid};

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

#[test]
fn nearest_within_radius() {
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(
        [
            (entity(1), Vec2::new(100.0, 100.0)),
            (entity(2), Vec2::new(130.0, 100.0)),
            (entity(3), Vec2::new(500.0, 500.0)),
        ]
        .into_iter(),
    );

    let (found, pos) = grid
        .nearest_in_range(Vec2::new(110.0, 100.0), 60.0)
        .expect("entity 1 is in range");
    assert_eq!(found, entity(1));
    assert_eq!(pos, Vec2::new(100.0, 100.0));
}

#[test]
fn nearest_across_cell_boundary() {
    let mut grid = SpatialGrid::<Demon>::default();
    // 59.0 и 61.0 — по разные стороны границы ячейки (60 м)
    grid.rebuild([(entity(7), Vec2::new(61.0, 10.0))].into_iter());

    let (found, _) = grid
        .nearest_in_range(Vec2::new(59.0, 10.0), 60.0)
        .expect("entity across the boundary is in range");
    assert_eq!(found, entity(7));
}

#[test]
fn nothing_outside_radius() {
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild([(entity(1), Vec2::new(100.0, 100.0))].into_iter());
    assert!(
        grid.nearest_in_range(Vec2::new(300.0, 100.0), 60.0)
            .is_none()
    );
}

#[test]
fn positions_outside_map_are_clamped() {
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild([(entity(1), Vec2::new(-5.0, 950.0))].into_iter());
    assert!(grid.nearest_in_range(Vec2::new(0.0, 899.0), 60.0).is_some());
}

#[test]
fn danger_map_covers_neighbor_cell_across_boundary() {
    let mut danger = DemonDangerMap::default();
    // демон у левого края своей ячейки [60..120): человек в соседней ячейке
    // в 56 м от него обязан попасть под пометку
    danger.rebuild([Vec2::new(61.0, 10.0)].into_iter());
    assert!(!danger.is_safe(Vec2::new(5.0, 10.0)));
    // а через две ячейки (заведомо дальше радиуса паники) — безопасно
    assert!(danger.is_safe(Vec2::new(190.0, 10.0)));
}

#[test]
fn danger_map_rebuild_clears_previous_marks() {
    let mut danger = DemonDangerMap::default();
    danger.rebuild([Vec2::new(100.0, 100.0)].into_iter());
    assert!(!danger.is_safe(Vec2::new(100.0, 100.0)));
    // демон ушёл — пересборка снимает старую пометку
    danger.rebuild(std::iter::empty());
    assert!(danger.is_safe(Vec2::new(100.0, 100.0)));
}
