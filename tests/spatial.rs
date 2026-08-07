//! Юнит-тесты пространственной сетки.

use std::collections::HashMap;

use bevy::prelude::*;

use qwe::demon::Demon;
use qwe::spatial::SpatialGrid;

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

/// Позиции живут у вызывающего (в игре — `SimPosition`), сетка хранит только
/// сущности; тестовый аналог `pos_of` — обычный HashMap.
fn positions(entries: &[(Entity, Vec2)]) -> HashMap<Entity, Vec2> {
    entries.iter().copied().collect()
}

#[test]
fn nearest_within_radius() {
    let entries = [
        (entity(1), Vec2::new(100.0, 100.0)),
        (entity(2), Vec2::new(130.0, 100.0)),
        (entity(3), Vec2::new(500.0, 500.0)),
    ];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    let (found, found_pos) = grid
        .nearest_in_range(Vec2::new(110.0, 100.0), 60.0, |e| pos.get(&e).copied())
        .expect("entity 1 is in range");
    assert_eq!(found, entity(1));
    assert_eq!(found_pos, Vec2::new(100.0, 100.0));
}

#[test]
fn nearest_across_cell_boundary() {
    // 59.0 и 61.0 — по разные стороны границы ячейки (60 м)
    let entries = [(entity(7), Vec2::new(61.0, 10.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    let (found, _) = grid
        .nearest_in_range(Vec2::new(59.0, 10.0), 60.0, |e| pos.get(&e).copied())
        .expect("entity across the boundary is in range");
    assert_eq!(found, entity(7));
}

#[test]
fn nothing_outside_radius() {
    let entries = [(entity(1), Vec2::new(100.0, 100.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    assert!(
        grid.nearest_in_range(Vec2::new(300.0, 100.0), 60.0, |e| pos.get(&e).copied())
            .is_none()
    );
}

#[test]
fn positions_outside_map_are_clamped() {
    let entries = [(entity(1), Vec2::new(-5.0, 950.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    assert!(
        grid.nearest_in_range(Vec2::new(0.0, 899.0), 60.0, |e| pos.get(&e).copied())
            .is_some()
    );
}

#[test]
fn incremental_insert_move_remove() {
    let mut grid = SpatialGrid::<Demon>::default();
    let walker = entity(9);
    let mut pos: HashMap<Entity, Vec2> = HashMap::new();

    // спавн
    pos.insert(walker, Vec2::new(10.0, 10.0));
    grid.insert(walker, Vec2::new(10.0, 10.0));
    assert!(
        grid.nearest_in_range(Vec2::new(12.0, 10.0), 60.0, |e| pos.get(&e).copied())
            .is_some()
    );

    // переезд через границу ячейки: из старой пропал, в новой нашёлся
    pos.insert(walker, Vec2::new(400.0, 400.0));
    grid.insert(walker, Vec2::new(400.0, 400.0));
    assert!(
        grid.nearest_in_range(Vec2::new(12.0, 10.0), 60.0, |e| pos.get(&e).copied())
            .is_none()
    );
    assert!(
        grid.nearest_in_range(Vec2::new(402.0, 400.0), 60.0, |e| pos.get(&e).copied())
            .is_some()
    );

    // смерть/despawn
    grid.remove(walker);
    assert!(
        grid.nearest_in_range(Vec2::new(402.0, 400.0), 60.0, |e| pos.get(&e).copied())
            .is_none()
    );
    // повторное удаление — no-op, не паника
    grid.remove(walker);
}

/// Обход прямоугольника отдаёт кандидатов из всех накрытых ячеек — и только
/// из них; прямоугольник за краем карты прижимается, а не паникует.
#[test]
fn rect_walk_covers_exactly_the_overlapping_cells() {
    let entries = [
        // внутри прямоугольника
        (entity(1), Vec2::new(100.0, 100.0)),
        // вне прямоугольника, но в накрытой им ячейке — грубый охват отдаёт
        (entity(2), Vec2::new(179.0, 100.0)),
        // ячейка не накрыта
        (entity(3), Vec2::new(500.0, 100.0)),
    ];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());

    let mut seen = Vec::new();
    grid.for_each_in_rect(Vec2::new(90.0, 90.0), Vec2::new(150.0, 110.0), |e| {
        seen.push(e)
    });
    // `Ord` у `Entity` идёт не по индексу — сортируем по нему явно
    seen.sort_by_key(|e: &Entity| e.index());
    assert_eq!(seen, vec![entity(1), entity(2)]);

    // прямоугольник, вылезающий за юго-западный угол карты: край прижат,
    // накрыты ячейки 0..1 — только entity(1)
    let mut seen = 0;
    grid.for_each_in_rect(Vec2::new(-100.0, -100.0), Vec2::new(110.0, 110.0), |_| {
        seen += 1
    });
    assert_eq!(seen, 1);
}

#[test]
fn insert_into_same_cell_keeps_single_entry() {
    let mut grid = SpatialGrid::<Demon>::default();
    let walker = entity(3);
    grid.insert(walker, Vec2::new(10.0, 10.0));
    // сдвиг внутри той же ячейки — записи не дублируются
    grid.insert(walker, Vec2::new(20.0, 20.0));

    let mut seen = 0;
    grid.for_each_in_cells_around(Vec2::new(15.0, 15.0), 60.0, |_| seen += 1);
    assert_eq!(seen, 1);
}
