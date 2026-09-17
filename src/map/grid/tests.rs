//! Равномерная сетка-индекс.
//!
//! До этого модуля правило вставки и правило детерминизма жили в свободных
//! функциях `osm/model.rs`, а шесть из семи самодельных копий цикла не
//! проверял никто: ошибка на границе ячеек читалась как пропавший объект на
//! скриншоте.

use super::*;

/// Сетка с шагом 10 м и значением, занимающим ровно одну ячейку.
fn one_cell() -> Grid<usize> {
    let mut grid = Grid::new(10.0);
    grid.insert_at(Vec2::new(5.0, 5.0), 7);
    grid
}

#[test]
fn a_value_is_found_from_anywhere_in_its_cell() {
    let grid = one_cell();
    for point in [
        Vec2::new(0.0, 0.0),
        Vec2::new(9.9, 9.9),
        Vec2::new(5.0, 5.0),
    ] {
        assert_eq!(grid.at(point), [7], "{point}");
    }
    assert!(grid.at(Vec2::new(10.0, 5.0)).is_empty());
    assert!(grid.at(Vec2::new(-0.1, 5.0)).is_empty());
}

/// Отрицательные координаты — не край карты, а обычное место: ячейка берётся
/// через `floor`, а не через усечение к нулю, иначе −0.5 и +0.5 попали бы в
/// одну ячейку и всё левее и ниже нуля слиплось бы.
#[test]
fn cells_do_not_fold_around_zero() {
    let grid = Grid::<usize>::new(10.0);
    assert_eq!(grid.cell_of(Vec2::new(-0.5, -0.5)), IVec2::new(-1, -1));
    assert_eq!(grid.cell_of(Vec2::new(0.5, 0.5)), IVec2::ZERO);
    assert_eq!(grid.cell_of(Vec2::new(-10.0, -10.0)), IVec2::new(-1, -1));
    assert_eq!(grid.cell_of(Vec2::new(-10.1, -10.1)), IVec2::new(-2, -2));
}

/// Главный инвариант: значение лежит во **всех** задетых ячейках, поэтому
/// спрашивающему хватает одной — своей.
#[test]
fn a_value_lands_in_every_cell_its_box_touches() {
    let mut grid = Grid::new(10.0);
    grid.insert(Vec2::new(5.0, 5.0), Vec2::new(25.0, 15.0), 1);

    // рамка задевает 3 × 2 ячейки, и в каждой значение видно
    for x in 0..3 {
        for y in 0..2 {
            let point = Vec2::new(x as f32 * 10.0 + 1.0, y as f32 * 10.0 + 1.0);
            assert_eq!(grid.at(point), [1], "{point}");
        }
    }
    assert!(grid.at(Vec2::new(31.0, 1.0)).is_empty());
    assert!(grid.at(Vec2::new(1.0, 21.0)).is_empty());
}

/// Ответ по рамке — отсортирован и без повторов: значение лежит в нескольких
/// ячейках сразу, а порядок обхода `HashMap` наружу протечь не должен.
#[test]
fn a_box_query_answers_sorted_and_once() {
    let mut grid = Grid::new(10.0);
    // каждое значение занимает несколько ячеек и кладётся в порядке, обратном
    // тому, в котором ожидается ответ
    for value in [30, 20, 10] {
        grid.insert(Vec2::new(5.0, 5.0), Vec2::new(25.0, 25.0), value);
    }
    assert_eq!(grid.near(Vec2::ZERO, Vec2::splat(30.0)), [10, 20, 30]);
    assert!(grid.near(Vec2::splat(100.0), Vec2::splat(110.0)).is_empty());
}

/// А `near_each` отдаёт то же самое **как есть**: значение из трёх задетых
/// ячеек придёт трижды. Порядок при этом определён — по возрастанию ячеек, —
/// так что и он не зависит от обхода `HashMap`.
#[test]
fn the_raw_query_repeats_a_value_once_per_cell() {
    let mut grid = Grid::new(10.0);
    grid.insert(Vec2::new(5.0, 5.0), Vec2::new(25.0, 5.0), 1);

    let raw: Vec<usize> = grid
        .near_each(Vec2::ZERO, Vec2::new(30.0, 5.0))
        .copied()
        .collect();
    assert_eq!(raw, [1, 1, 1], "три задетые ячейки — три ответа");
    assert_eq!(grid.near(Vec2::ZERO, Vec2::new(30.0, 5.0)), [1]);
}

/// Пары — перебор кандидатов внутри ячейки, и он тоже отсортирован: по ним
/// сшиваются ленты гаражей и сравниваются дома, и порядок решать не должен.
///
/// Значения кладутся **не** по возрастанию нарочно: пара обязана прийти
/// упорядоченной внутри себя, иначе те же двое из двух разных ячеек дали бы
/// `(a, b)` и `(b, a)`, и `dedup` их не склеил бы.
#[test]
fn pairs_are_taken_inside_a_cell_sorted_and_once() {
    let mut grid = Grid::new(10.0);
    for value in [2, 1, 3] {
        grid.insert_at(Vec2::new(5.0, 5.0), value);
    }
    // сосед в другой ячейке в пары не попадает
    grid.insert_at(Vec2::new(15.0, 5.0), 9);

    assert_eq!(grid.pairs(), [(1, 2), (1, 3), (2, 3)]);
}

/// Значение, лежащее в двух общих ячейках, даёт одну пару, а не две.
#[test]
fn a_pair_sharing_two_cells_is_reported_once() {
    let mut grid = Grid::new(10.0);
    for value in [1, 2] {
        grid.insert(Vec2::new(5.0, 5.0), Vec2::new(15.0, 5.0), value);
    }
    assert_eq!(grid.pairs(), [(1, 2)]);
}

#[test]
fn an_empty_grid_answers_nothing() {
    let grid = Grid::<usize>::new(10.0);
    assert!(grid.is_empty());
    assert!(grid.at(Vec2::ZERO).is_empty());
    assert!(grid.near(Vec2::ZERO, Vec2::splat(100.0)).is_empty());
    assert!(grid.pairs().is_empty());
    assert!(!one_cell().is_empty());
}
