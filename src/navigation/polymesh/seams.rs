//! Геометрия общей кромки чанков: подразбиение стороны, пересечения контуров
//! препятствий с линиями сетки и готовый контур чанка для CDT.
//!
//! Фаза **до** триангуляции: `chunk_outline` отдаёт
//! `Triangulation::from_outer_edges` внешний контур, в котором обе стороны шва
//! получают одинаковый набор вершин — только на таком контуре соседние слои
//! потом сходятся по общим рёбрам, а не по точкам. Сама сшивка — компоненты
//! связности слоя и сборка слоёв в один меш с графом переходов — живёт в
//! [`super::stitch`] и работает, когда все слои уже построены; округление на
//! общую сетку (`SEAM_QUANTUM`) она берёт отсюда.

use bevy::prelude::*;

use super::to_poly;

/// Шаг подразбиения кромки чанка, метры.
///
/// Без подразбиения на шве совпадают только углы чанка: CDT ставит вершины на
/// внешнем контуре лишь там, где его касается препятствие. Сшитые по двум
/// точкам полигоны становятся «соседями» через **точку**, а не через общее
/// ребро, и поиск идёт по мусорной смежности — замерено: 140 ГБ за считанные
/// секунды. Подразбиение даёт обеим сторонам совпадающие вершины (чанки
/// одинакового размера, точки кратны шагу от общего угла), то есть цепочку
/// настоящих общих рёбер. Интервал видимости переносится через ребро
/// целиком, поэтому шаг задаёт не точность пути, а лишь дробность цепочки;
/// геометрию проходов вдоль шва по-прежнему задают вершины препятствий.
/// `Triangulation::simplify` внешний контур не трогает (только `interiors`),
/// так что подразбиение переживает упрощение.
const SEAM_STEP_METERS: f32 = 20.0;

/// Шаг сетки, на которую сажаются координаты обрезанных контуров, метры.
///
/// Общую кромку два соседа режут порознь, и i_overlay выдаёт им точки,
/// различающиеся в младших разрядах f32. Такая пара не находит друг друга при
/// сшивке, и вершина остаётся на шве только с одной стороны. Переход через шов
/// тогда односторонний: из соседа внутрь общий полигон находится (он содержит
/// оба конца ребра), а обратно — нет, потому что лишняя вершина разбила ребро
/// надвое. На этой асимметрии воронка перестаёт сходиться, очередь растёт без
/// предела, процесс умирает от OOM. Округление на общую сетку убирает
/// расхождение в источнике; сантиметр — на два порядка меньше упрощения
/// (`SIMPLIFY_EPSILON`) и на порядок больше разрешения f32 на краю карты.
pub const SEAM_QUANTUM: f32 = 0.01;

pub(super) fn quantized(value: f32) -> f32 {
    (value / SEAM_QUANTUM).round() * SEAM_QUANTUM
}

/// Мировая координата узла сетки чанков вдоль одной оси — **функция от номера
/// узла**, а не от координат чанка.
///
/// Единственное место, где это произведение считается. Побитовое совпадение
/// всех сторон шва — несущий инвариант (почему именно — у [`seam_point`]), а
/// пока выражение одно, он держится построением, а не дисциплиной: у левого
/// соседа кромка не может выйти как `origin + size`, а у правого как
/// `(node + 1) * size`.
pub(super) fn node_coord(node: u32, size: f32) -> f32 {
    quantized(node as f32 * size)
}

/// Мировая точка узла сетки чанков: [`node_coord`] по обеим осям.
pub(super) fn node_world(node: UVec2, chunk_size: Vec2) -> Vec2 {
    Vec2::new(
        node_coord(node.x, chunk_size.x),
        node_coord(node.y, chunk_size.y),
    )
}

/// Точка подразбиения кромки чанка — **функция от номера узла сетки и номера
/// шага**, а не от координат конкретного чанка.
///
/// Это принципиально. Общую кромку два соседа считают каждый со своей стороны,
/// и если бы точка бралась как `origin + lerp(...)`, у левого соседа вышло бы
/// `x*s + s`, а у правого `(x+1)*s` — в f32 это разные числа. Поиск же хранит
/// интервал воронки в мировых координатах и сверяет его концы с координатами
/// вершин, так что расхождение в младшем разряде на каждом шве не даёт воронке
/// сходиться: она обходит одну и ту же область по кругу, очередь растёт без
/// предела, и процесс умирает от OOM (замерено: минута на карте из двух стен).
fn seam_point(node: u32, step: u32, steps: u32, chunk_size: f32) -> f32 {
    node as f32 * chunk_size + step as f32 / steps as f32 * chunk_size
}

/// Сколько отрезков подразбиения приходится на одну сторону чанка.
fn seam_steps(side: f32) -> u32 {
    (side / SEAM_STEP_METERS).ceil().max(1.0) as u32
}

/// Точки, где контуры препятствий пересекают линии сетки чанков.
///
/// Считаются один раз на всю карту и раздаются обоим соседям, поэтому кромку
/// оба получают с одинаковым набором вершин. Без этого препятствие, упёршееся
/// в шов только с одной стороны, оставляет вершину лишь в своём чанке: у
/// соседа ребро шва цельное, у него — разбитое надвое. Переход тогда
/// односторонний (из соседа внутрь общий полигон находится, обратно нет), и на
/// этой асимметрии воронка перестаёт сходиться.
#[derive(Default)]
pub(super) struct SeamPoints {
    /// координаты вдоль линии `x = node * chunk_size.x`, по индексу узла
    vertical: Vec<Vec<f32>>,
    /// координаты вдоль линии `y = node * chunk_size.y`, по индексу узла
    horizontal: Vec<Vec<f32>>,
}

pub(super) fn seam_points(
    contours: &[(Vec2, Vec2, Vec<[f32; 2]>)],
    grid: UVec2,
    chunk_size: Vec2,
) -> SeamPoints {
    let mut seams = SeamPoints {
        vertical: vec![Vec::new(); grid.x as usize + 1],
        horizontal: vec![Vec::new(); grid.y as usize + 1],
    };
    let cross = |from: f32, to: f32, other_from: f32, other_to: f32, line: f32| -> Option<f32> {
        if (from < line) == (to < line) {
            return None;
        }
        let ratio = (line - from) / (to - from);
        Some(quantized(other_from + (other_to - other_from) * ratio))
    };
    for (min, max, contour) in contours {
        for index in 0..contour.len() {
            let a = Vec2::from(contour[index]);
            let b = Vec2::from(contour[(index + 1) % contour.len()]);
            for node in 0..=grid.x {
                let line = node_coord(node, chunk_size.x);
                if min.x > line || max.x < line {
                    continue;
                }
                if let Some(value) = cross(a.x, b.x, a.y, b.y, line) {
                    seams.vertical[node as usize].push(value);
                }
            }
            for node in 0..=grid.y {
                let line = node_coord(node, chunk_size.y);
                if min.y > line || max.y < line {
                    continue;
                }
                if let Some(value) = cross(a.y, b.y, a.x, b.x, line) {
                    seams.horizontal[node as usize].push(value);
                }
            }
        }
    }
    for line in seams.vertical.iter_mut().chain(seams.horizontal.iter_mut()) {
        line.sort_by(f32::total_cmp);
        line.dedup();
    }
    seams
}

/// Внутренние точки одной стороны чанка: подразбиение плюс пересечения
/// препятствий с этой линией сетки. Оба конца исключены — их ставит кольцо.
fn side_points(node: u32, span: f32, steps: u32, crossings: &[f32]) -> Vec<f32> {
    let (low, high) = (node_coord(node, span), node_coord(node + 1, span));
    let mut points: Vec<f32> = (1..steps)
        .map(|step| quantized(seam_point(node, step, steps, span)))
        .collect();
    points.extend(
        crossings
            .iter()
            .copied()
            .filter(|value| *value > low && *value < high),
    );
    points.sort_by(f32::total_cmp);
    points.dedup();
    points
}

/// Контур чанка в мировых координатах, обход против часовой. Каждая сторона
/// считается в каноническом направлении (по возрастанию координаты) и при
/// необходимости разворачивается — иначе у двух соседей одна и та же кромка
/// вышла бы разными числами.
pub(super) fn chunk_outline(
    cell: UVec2,
    chunk_size: Vec2,
    seams: &SeamPoints,
) -> Vec<polyanya_glam::Vec2> {
    let (steps_x, steps_y) = (seam_steps(chunk_size.x), seam_steps(chunk_size.y));
    let xs = |node_y: u32| {
        side_points(
            cell.x,
            chunk_size.x,
            steps_x,
            &seams.horizontal[node_y as usize],
        )
    };
    let ys = |node_x: u32| {
        side_points(
            cell.y,
            chunk_size.y,
            steps_y,
            &seams.vertical[node_x as usize],
        )
    };
    let corner = |node_x: u32, node_y: u32| node_world(UVec2::new(node_x, node_y), chunk_size);

    let (low_x, low_y) = (cell.x, cell.y);
    let (high_x, high_y) = (cell.x + 1, cell.y + 1);
    let mut outline: Vec<Vec2> = vec![corner(low_x, low_y)];
    outline.extend(
        xs(low_y)
            .into_iter()
            .map(|x| Vec2::new(x, corner(0, low_y).y)),
    );
    outline.push(corner(high_x, low_y));
    outline.extend(
        ys(high_x)
            .into_iter()
            .map(|y| Vec2::new(corner(high_x, 0).x, y)),
    );
    outline.push(corner(high_x, high_y));
    outline.extend(
        xs(high_y)
            .into_iter()
            .rev()
            .map(|x| Vec2::new(x, corner(0, high_y).y)),
    );
    outline.push(corner(low_x, high_y));
    outline.extend(
        ys(low_x)
            .into_iter()
            .rev()
            .map(|y| Vec2::new(corner(low_x, 0).x, y)),
    );
    outline.into_iter().map(to_poly).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Общая кромка двух соседей должна быть **побитово** одинаковой: поиск
    /// сверяет концы интервала воронки с координатами вершин точным
    /// равенством, и расхождение в младшем разряде на каждом шве не даёт ей
    /// сходиться (см. `seam_point`). Тест держит инвариант со стороны
    /// результата: слева кромка считается как дальняя сторона чанка 0, справа
    /// — как ближняя сторона чанка 1, и наборы обязаны совпасть до бита.
    #[test]
    fn neighbouring_chunks_share_the_seam_bit_for_bit() {
        let chunk_size = Vec2::new(413.7, 271.3);
        let seams = seam_points(&[], UVec2::new(2, 1), chunk_size);
        let line = node_coord(1, chunk_size.x);
        let on_seam = |cell: UVec2| -> Vec<u32> {
            let mut ys: Vec<u32> = chunk_outline(cell, chunk_size, &seams)
                .into_iter()
                .filter(|point| point.x.to_bits() == line.to_bits())
                .map(|point| point.y.to_bits())
                .collect();
            ys.sort_unstable();
            ys
        };
        let left = on_seam(UVec2::new(0, 0));
        assert!(
            left.len() > 2,
            "кромка должна быть подразбита, а не только по углам: {}",
            left.len()
        );
        assert_eq!(
            left,
            on_seam(UVec2::new(1, 0)),
            "у соседей разные числа на общей кромке — воронка не сойдётся"
        );
    }
}
