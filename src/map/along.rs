//! Ходьба по дуговой координате ломаной — один обход на весь `map/*`.
//!
//! Всё, что карта расставляет **вдоль** линейной геометрии — ряды машин,
//! составы на станционных путях, — обязано идти по длине всей ломаной, а не по
//! каждому её звену порознь: звено в городе сплошь и рядом короче двух
//! отступов, и обход `points.windows(2)` выбрасывал такие звенья целиком, на
//! каждой вершине сбрасывал шаг и отступал от изгиба так, будто там торец.
//! Слой машин через это прошёл ([`super::cars`]), слой вагонов —
//! ([`super::wagons`]) — следом, отсюда общая пара примитивов, а не копия.
//!
//! Кривизну обход не лечит: у кузова жёсткая база, и на изломе соседние места
//! наезжают друг на друга. Проверять это положено вызывающему — по **мировому**
//! расстоянию до предыдущего поставленного объекта, а не по дуговой координате.
//!
//! Сюда же — то, что смотрит на ломаную целиком, а не на звено: ближайшая
//! точка с её дуговой координатой ([`nearest_on_path`]) и упрощение Дугласа —
//! Пекера ([`simplify`]). У каждого было по две-три копии в `roads/*`.

use bevy::prelude::*;

use crate::map::osm::model::{closest_on_segment, distance_to_segment};

/// Накопленные длины по точкам ломаной и её полная длина.
pub(super) fn arclengths(points: &[Vec2]) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (index, &point) in points.iter().enumerate() {
        if index > 0 {
            total += point.distance(points[index - 1]);
        }
        along.push(total);
    }
    (along, total)
}

/// Точка ломаной на дуговой координате `at` и направление звена, на которое
/// она попала: звено ищется бинарным поиском по `along`, позиция внутри него —
/// интерполяцией.
///
/// Края отрезка `[0, total]` — **внутри**, а не снаружи: `at = 0` даёт первую
/// точку ломаной, `at = total` — последнюю, каждую с направлением своего
/// крайнего звена. Полуоткрытость тут была бы вредна — шаг вдоль пути
/// упирается в конец ровно, и отказ на границе выбрасывал бы последний
/// объект ряда.
///
/// **Дублирующаяся вершина не съедает место.** Нулевое звено не имеет
/// направления, а `binary_search` при равных ключах вправе вернуть любой из
/// них — то есть ответ зависел бы от неоговорённой детали `std`. Позиция от
/// выбора не зависит (совпавшие вершины — одна и та же точка), а направление
/// берётся у ближайшего звена, у которого длина есть: сперва вперёд, потом
/// назад. `None` остаётся только там, где направления нет вовсе — у ломаной
/// короче двух точек или нулевой длины целиком.
pub(super) fn place_on_path(points: &[Vec2], along: &[f32], at: f32) -> Option<(Vec2, Vec2)> {
    let last = points.len().checked_sub(2)?;
    let index = match along.binary_search_by(|value| value.total_cmp(&at)) {
        Ok(index) => index.min(last),
        Err(index) => index.saturating_sub(1).min(last),
    };
    let direction = direction_at(points, index)?;
    Some((points[index] + direction * (at - along[index]), direction))
}

/// Направление ближайшего к `index` звена, у которого есть длина.
fn direction_at(points: &[Vec2], index: usize) -> Option<Vec2> {
    (index..points.len() - 1)
        .chain((0..index).rev())
        .find_map(|link| (points[link + 1] - points[link]).try_normalize())
}

/// Ближайшая к `point` точка ломаной и её дуговая координата; при равных
/// расстояниях — на первом из звеньев. `None` — у ломаной нет ни одного звена.
pub(super) fn nearest_on_path(points: &[Vec2], point: Vec2) -> Option<(Vec2, f32)> {
    let mut best: Option<(f32, Vec2, f32)> = None;
    let mut run = 0.0;
    for link in points.windows(2) {
        let onto = closest_on_segment(point, link[0], link[1]);
        let distance = onto.distance(point);
        if best.is_none_or(|(closest, ..)| distance < closest) {
            best = Some((distance, onto, run + link[0].distance(onto)));
        }
        run += link[0].distance(link[1]);
    }
    best.map(|(_, onto, along)| (onto, along))
}

/// Упрощение Дугласа — Пекера с допуском `tolerance`: индексы оставшихся
/// вершин по возрастанию. Концы разомкнутой ломаной и вершины `keep`
/// остаются всегда; у кольца (`closed`) всегда остаётся первая, и последний
/// пролёт идёт от последней оставленной вершины к ней.
pub(super) fn simplify(
    points: &[Vec2],
    closed: bool,
    tolerance: f32,
    keep: impl Fn(usize) -> bool,
) -> Vec<usize> {
    let count = points.len();
    if count <= 2 {
        return (0..count).collect();
    }
    let mut kept: Vec<bool> = (0..count).map(keep).collect();
    kept[0] = true;
    if !closed {
        kept[count - 1] = true;
    }
    let anchors: Vec<usize> = (0..count).filter(|&index| kept[index]).collect();
    let mut spans: Vec<(usize, usize)> =
        anchors.windows(2).map(|pair| (pair[0], pair[1])).collect();
    if closed {
        spans.push((anchors[anchors.len() - 1], count));
    }
    while let Some((from, to)) = spans.pop() {
        let (a, b) = (points[from % count], points[to % count]);
        let farthest = (from + 1..to)
            .map(|index| (index, distance_to_segment(points[index % count], a, b)))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((far, distance)) = farthest
            && distance > tolerance
        {
            kept[far % count] = true;
            spans.push((from, far));
            spans.push((far, to));
        }
    }
    (0..count).filter(|&index| kept[index]).collect()
}

#[cfg(test)]
mod tests;
