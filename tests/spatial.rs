//! Юнит-тесты пространственной сетки.

use std::collections::HashMap;

use bevy::prelude::*;

use qwe::demon::Demon;
use qwe::spatial::CELL_SIZE;
use qwe::spatial::SpatialGrid;

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

/// Позиции живут у вызывающего (в игре — `SimPosition`), сетка хранит только
/// сущности; тестовый аналог выборки — обычный HashMap.
fn positions(entries: &[(Entity, Vec2)]) -> HashMap<Entity, Vec2> {
    entries.iter().copied().collect()
}

/// Тестовый порядковый номер: там, где он сам по себе безразличен, за него
/// сходит индекс сущности — важно лишь, что он у каждого кандидата свой.
fn order(entity: Entity) -> u32 {
    entity.index().index()
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
        .nearest_in_range(
            Vec2::new(110.0, 100.0),
            60.0,
            |e| pos.get(&e).copied(),
            order,
        )
        .expect("entity 1 is in range");
    assert_eq!(found, entity(1));
    assert_eq!(found_pos, Vec2::new(100.0, 100.0));
}

#[test]
fn nearest_across_cell_boundary() {
    // по метр с каждой стороны границы ячейки
    let entries = [(entity(7), Vec2::new(CELL_SIZE + 1.0, 10.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    let (found, _) = grid
        .nearest_in_range(
            Vec2::new(CELL_SIZE - 1.0, 10.0),
            CELL_SIZE,
            |e| pos.get(&e).copied(),
            order,
        )
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
        grid.nearest_in_range(
            Vec2::new(300.0, 100.0),
            60.0,
            |e| pos.get(&e).copied(),
            order
        )
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
        grid.nearest_in_range(Vec2::new(0.0, 899.0), 60.0, |e| pos.get(&e).copied(), order)
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
        grid.nearest_in_range(Vec2::new(12.0, 10.0), 60.0, |e| pos.get(&e).copied(), order)
            .is_some()
    );

    // переезд через границу ячейки: из старой пропал, в новой нашёлся
    pos.insert(walker, Vec2::new(400.0, 400.0));
    grid.insert(walker, Vec2::new(400.0, 400.0));
    assert!(
        grid.nearest_in_range(Vec2::new(12.0, 10.0), 60.0, |e| pos.get(&e).copied(), order)
            .is_none()
    );
    assert!(
        grid.nearest_in_range(
            Vec2::new(402.0, 400.0),
            60.0,
            |e| pos.get(&e).copied(),
            order
        )
        .is_some()
    );

    // смерть/despawn
    grid.remove(walker);
    assert!(
        grid.nearest_in_range(
            Vec2::new(402.0, 400.0),
            60.0,
            |e| pos.get(&e).copied(),
            order
        )
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

/// Из равноудалённых побеждает меньший порядковый номер — не тот, до кого
/// обход ячейки дошёл последним.
///
/// Точная ничья в живой игре редка (на Туле 5 на 3.84 млн поисков), и тест
/// именно поэтому её подстраивает: сама по себе она не случится ни в одном
/// прогоне, а зависимость исхода от порядка внутри ячейки — то есть от истории
/// спавнов и смертей (`swap_remove` перекладывает хвост на место удалённого) —
/// осталась бы непроверяемой. Оба взаимных порядка вставки проверяются потому,
/// что порядок и есть та величина, от которой исход зависеть не имеет права.
#[test]
fn equidistant_candidates_are_decided_by_the_number_not_by_traversal() {
    let junior = (entity(2), Vec2::new(90.0, 100.0));
    let senior = (entity(9), Vec2::new(110.0, 100.0));
    let pos = positions(&[junior, senior]);

    // одна ячейка: победителя выбирает только порядок вставки
    for insertion in [[junior, senior], [senior, junior]] {
        let mut grid = SpatialGrid::<Demon>::default();
        for (entry, position) in insertion {
            grid.insert(entry, position);
        }
        let (found, _) = grid
            .nearest_in_range(
                Vec2::new(100.0, 100.0),
                60.0,
                |e| pos.get(&e).copied(),
                order,
            )
            .expect("оба кандидата в радиусе");
        assert_eq!(found, junior.0, "ничью разрешил порядок вставки в ячейку");
    }

    // разные ячейки: победителя выбирает порядок обхода ячеек, и меньший
    // номер стоит в ТОЙ, которую обходят первой
    let senior_next_cell = (entity(9), Vec2::new(130.0, 100.0));
    let pos = positions(&[junior, senior_next_cell]);
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild([junior, senior_next_cell].into_iter());
    let (found, _) = grid
        .nearest_in_range(
            Vec2::new(110.0, 100.0),
            60.0,
            |e| pos.get(&e).copied(),
            order,
        )
        .expect("оба кандидата в радиусе");
    assert_eq!(found, junior.0, "ничью разрешил порядок обхода ячеек");
}

/// «Не дальше `radius`» — включительно. Граница радиуса и сравнение «ближе
/// найденного» разведены, и это проверка, что при разведении граница не
/// съехала на строгую.
#[test]
fn a_candidate_exactly_at_the_radius_is_in_range() {
    let entries = [(entity(1), Vec2::new(160.0, 100.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);

    assert!(
        grid.nearest_in_range(
            Vec2::new(100.0, 100.0),
            60.0,
            |e| pos.get(&e).copied(),
            order
        )
        .is_some(),
        "кандидат ровно на радиусе выпал из поиска"
    );
}

/// Радиус шире ячейки — уже не три на три: успокоение беглеца ищет демонов
/// вплоть до ×1,5 радиуса паники (90 м), и кольцо ячеек обязано вырасти само.
/// Спрашивающий у правого края своей ячейки, кандидат за два края — 66 м,
/// то есть дальше ячейки, но в пределах радиуса. Сузь обход до трёх на три —
/// беглец успокаивается при демоне в 61–90 м и паникует снова.
#[test]
fn a_radius_wider_than_a_cell_walks_the_second_ring() {
    let entries = [(entity(7), Vec2::new(125.0, 5.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let pos = positions(&entries);
    let asking = Vec2::new(59.0, 5.0);

    let found = grid.nearest_in_range(asking, CELL_SIZE * 1.5, |e| pos.get(&e).copied(), order);
    assert_eq!(
        found.map(|(found, _)| found),
        Some(entity(7)),
        "кандидат во втором кольце выпал из поиска"
    );
    // контроль расстояния: радиусом в одну ячейку тот же кандидат не достаётся
    assert!(
        grid.nearest_in_range(asking, CELL_SIZE, |e| pos.get(&e).copied(), order)
            .is_none()
    );
}

/// Грубая проверка занятости ходит тем же кольцом, что и точный поиск: на ней
/// стоит успокоение между тиками решения, и «пусто» обязано означать, что во
/// втором кольце тоже никого нет.
#[test]
fn the_coarse_probe_sees_the_second_ring_too() {
    let entries = [(entity(7), Vec2::new(125.0, 5.0))];
    let mut grid = SpatialGrid::<Demon>::default();
    grid.rebuild(entries.into_iter());
    let asking = Vec2::new(59.0, 5.0);

    assert!(
        grid.any_in_cells_around(asking, CELL_SIZE * 1.5),
        "занятая ячейка во втором кольце не замечена"
    );
    assert!(
        !grid.any_in_cells_around(asking, CELL_SIZE),
        "радиус в одну ячейку второго кольца не достаёт"
    );
}

/// `moved` — это переезд, а не upsert: внутри ячейки он не трогает таблицу
/// вовсе, через границу — переставляет.
#[test]
fn moved_touches_the_grid_only_across_a_cell_boundary() {
    let mut grid = SpatialGrid::<Demon>::default();
    let walker = entity(5);
    let pos = positions(&[(walker, Vec2::new(10.0, 10.0))]);

    // не вставленного сдвиг внутри ячейки в сетку не заводит
    grid.moved(walker, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
    assert!(
        grid.nearest_in_range(Vec2::new(15.0, 15.0), 60.0, |e| pos.get(&e).copied(), order)
            .is_none()
    );

    grid.insert(walker, Vec2::new(10.0, 10.0));
    // переезд через границу: в старой ячейке не осталось, в новой нашёлся
    grid.moved(walker, Vec2::new(10.0, 10.0), Vec2::new(400.0, 400.0));
    let pos = positions(&[(walker, Vec2::new(400.0, 400.0))]);
    let mut seen = 0;
    grid.for_each_in_cells_around(Vec2::new(10.0, 10.0), 0.0, |_| seen += 1);
    assert_eq!(seen, 0, "запись осталась в покинутой ячейке");
    assert!(
        grid.nearest_in_range(
            Vec2::new(402.0, 400.0),
            60.0,
            |e| pos.get(&e).copied(),
            order
        )
        .is_some()
    );
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
