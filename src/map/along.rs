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

use bevy::prelude::*;

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
pub(super) fn place_on_path(points: &[Vec2], along: &[f32], at: f32) -> Option<(Vec2, Vec2)> {
    let last = points.len().checked_sub(2)?;
    let index = match along.binary_search_by(|value| value.total_cmp(&at)) {
        Ok(index) => index.min(last),
        Err(index) => index.saturating_sub(1).min(last),
    };
    let direction = (points[index + 1] - points[index]).try_normalize()?;
    Some((points[index] + direction * (at - along[index]), direction))
}
