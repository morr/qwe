//! Контракт обхода по дуговой координате.
//!
//! Тут прибиты ровно те его свойства, на которые вызывающие опираются молча:
//! ряд машин и состав на пути идут по возрастающему `at` и ждут, что точка
//! идёт вперёд вместе с ним, а на краях и на дублирующейся вершине место не
//! пропадает.

use super::*;

const EPSILON: f32 = 1e-4;

fn straight() -> Vec<Vec2> {
    vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]
}

fn assert_close(left: Vec2, right: Vec2, what: &str) {
    assert!(left.distance(right) < EPSILON, "{what}: {left} != {right}");
}

/// Накопленные длины не убывают, начинаются с нуля и кончаются полной длиной.
#[test]
fn the_arclengths_grow_with_the_points() {
    let points = straight();
    let (along, total) = arclengths(&points);
    assert_eq!(along.len(), points.len());
    assert_eq!(along[0], 0.0);
    assert!(along.windows(2).all(|pair| pair[1] >= pair[0]), "{along:?}");
    assert_eq!(*along.last().unwrap(), total);
    assert!((total - 20.0).abs() < EPSILON, "{total}");
}

/// Дублирующаяся вершина добавляет к длине ноль, а не разрывает счёт.
#[test]
fn a_repeated_vertex_adds_no_length() {
    let point = Vec2::new(3.0, 4.0);
    let (along, total) = arclengths(&[Vec2::ZERO, point, point]);
    assert_eq!(along, [0.0, 5.0, 5.0]);
    assert_eq!(total, 5.0);
}

/// На прямой место — это ровно `lerp` между концами.
#[test]
fn a_place_on_a_straight_line_is_a_lerp() {
    let points = [Vec2::ZERO, Vec2::new(10.0, 20.0)];
    let (along, total) = arclengths(&points);
    for step in 0..=10 {
        let share = step as f32 / 10.0;
        let (point, direction) = place_on_path(&points, &along, total * share).unwrap();
        assert_close(point, points[0].lerp(points[1], share), "place");
        assert_close(direction, (points[1] - points[0]).normalize(), "direction");
    }
}

/// Точка идёт вперёд вместе с `at` — на изломе тоже. Сумма пройденного равна
/// полной длине ровно тогда, когда обход нигде не пятится назад: любой откат
/// и возврат дали бы сумму больше.
#[test]
fn a_place_moves_forward_with_the_coordinate() {
    let points = straight();
    let (along, total) = arclengths(&points);
    let mut travelled = 0.0;
    let mut last = points[0];
    for step in 0..=40 {
        let at = total * step as f32 / 40.0;
        let (point, _) = place_on_path(&points, &along, at).unwrap();
        travelled += last.distance(point);
        last = point;
    }
    assert!(
        (travelled - total).abs() < EPSILON,
        "{travelled} != {total}"
    );
    assert_close(last, *points.last().unwrap(), "tail");
}

/// `at = 0` — первая точка ломаной и направление первого звена.
#[test]
fn the_start_of_the_path_is_a_place() {
    let points = straight();
    let (along, _) = arclengths(&points);
    let (point, direction) = place_on_path(&points, &along, 0.0).unwrap();
    assert_close(point, points[0], "place");
    assert_close(direction, Vec2::X, "direction");
}

/// `at = total` — последняя точка и направление последнего звена: шаг вдоль
/// пути упирается в конец ровно, и отказ тут выбрасывал бы крайний объект.
#[test]
fn the_end_of_the_path_is_a_place() {
    let points = straight();
    let (along, total) = arclengths(&points);
    let (point, direction) = place_on_path(&points, &along, total).unwrap();
    assert_close(point, *points.last().unwrap(), "place");
    assert_close(direction, Vec2::Y, "direction");
}

/// Дублирующаяся вершина не съедает место: направление берётся у ближайшего
/// звена с длиной — в середине ломаной у следующего, в хвосте у предыдущего.
#[test]
fn a_repeated_vertex_still_gives_a_place() {
    let middle = Vec2::new(10.0, 0.0);
    let points = [Vec2::ZERO, middle, middle, Vec2::new(10.0, 10.0)];
    let (along, total) = arclengths(&points);
    let (point, direction) = place_on_path(&points, &along, 10.0).unwrap();
    assert_close(point, middle, "place");
    assert_close(direction, Vec2::Y, "direction");

    let tail = Vec2::new(10.0, 10.0);
    let points = [Vec2::ZERO, middle, tail, tail];
    let (along, total_with_tail) = arclengths(&points);
    let (point, direction) = place_on_path(&points, &along, total_with_tail).unwrap();
    assert_close(point, tail, "tail place");
    assert_close(direction, Vec2::Y, "tail direction");
    assert_eq!(total, total_with_tail);
}

/// Направления вовсе нет только у вырожденной ломаной.
#[test]
fn a_path_without_length_has_no_place() {
    let point = Vec2::new(1.0, 2.0);
    let (along, _) = arclengths(&[point, point, point]);
    assert!(place_on_path(&[point, point, point], &along, 0.0).is_none());
    assert!(place_on_path(&[point], &[0.0], 0.0).is_none());
}
