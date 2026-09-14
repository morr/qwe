//! Резка осевой по контурам площадей: куски ломаной **вне** набора полигонов,
//! с точными точками пересечения контура.
//!
//! Два потребителя, и оба рисуют ленту там, где площадь её уже накрывает:
//! русло внутри площадной воды (`map::water`, конец заходит за берег на ширину
//! отмели) и проезд стоянки внутри самой стоянки (`map::roads`, конец режется
//! ровно по краю). Навмеш не трогает ни тот, ни другой — это только рисунок.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::osm::PolyArea;
use crate::map::osm::model::{grid_cell, point_in_area, put_in_cells, ring_bounds};

/// Шаг сетки рёбер контуров, м. Контур реки — тысячи рёбер, и ось спрашивает
/// только те, что лежат в клетках её звена.
const CELL: f32 = 32.0;

/// Пересечения ближе этого вдоль звена — одно: общая вершина двух рёбер
/// контура даёт два попадания в одну точку.
const SAME_CROSSING: f32 = 1e-4;

/// Кусок оси вне площадей, уже с заходом внутрь, и какие из двух его концов
/// отрезаны контуром (`[начало, конец]`), а не пришли из OSM.
#[derive(Debug)]
pub struct OutsideRun {
    pub points: Vec<Vec2>,
    pub clipped: [bool; 2],
}

/// Площади для резки осей: сами полигоны с их AABB и сетка рёбер всех их колец.
pub struct AreaIndex<'a> {
    areas: &'a [PolyArea],
    bounds: Vec<(Vec2, Vec2)>,
    edges: HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
}

impl<'a> AreaIndex<'a> {
    pub fn new(areas: &'a [PolyArea]) -> Self {
        let mut edges = HashMap::new();
        for area in areas {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                for (index, &from) in ring.iter().enumerate() {
                    let to = ring[(index + 1) % ring.len()];
                    put_in_cells(&mut edges, from.min(to), from.max(to), CELL, (from, to));
                }
            }
        }
        Self {
            areas,
            bounds: areas.iter().map(|area| ring_bounds(&area.outer)).collect(),
            edges,
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        self.areas
            .iter()
            .zip(&self.bounds)
            .any(|(area, &(min, max))| {
                point.cmpge(min).all() && point.cmple(max).all() && point_in_area(point, area)
            })
    }

    /// Доли звена `from → to`, где оно пересекает ребро контура, по
    /// возрастанию и без повторов.
    fn crossings(&self, from: Vec2, to: Vec2) -> Vec<f32> {
        let (min, max) = (from.min(to), from.max(to));
        let span = to - from;
        let mut hits = Vec::new();
        for x in grid_cell(min.x, CELL)..=grid_cell(max.x, CELL) {
            for y in grid_cell(min.y, CELL)..=grid_cell(max.y, CELL) {
                let Some(cell) = self.edges.get(&(x, y)) else {
                    continue;
                };
                for &(a, b) in cell {
                    let edge = b - a;
                    let denominator = span.perp_dot(edge);
                    if denominator == 0.0 {
                        continue;
                    }
                    let t = (a - from).perp_dot(edge) / denominator;
                    let u = (a - from).perp_dot(span) / denominator;
                    // начало звена считается, конец — нет: контур, пришедший ровно
                    // в вершину оси, обязан сбросить «внутри/снаружи» один раз, а не
                    // ноль (он же конец предыдущего звена)
                    if (0.0..1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                        hits.push(t);
                    }
                }
            }
        }
        hits.sort_by(f32::total_cmp);
        hits.dedup_by(|next, kept| *next - *kept < SAME_CROSSING);
        hits
    }

    /// Куски оси вне площадей. Каждый отрезанный конец продолжен внутрь на
    /// `reach` вдоль той же оси, а за её концом — по прямой последнего звена;
    /// при `reach` 0 конец лежит ровно на контуре.
    ///
    /// Площадь между двумя кусками, что уже `2 · reach`, не режет ось вовсе:
    /// заходы с обеих сторон всё равно легли бы друг на друга, а две ленты
    /// внахлёст с кромкой, гаснущей навстречу, дают шов посреди протоки.
    pub fn outside_runs(&self, path: &[Vec2], reach: f32) -> Vec<OutsideRun> {
        if path.len() < 2 {
            return Vec::new();
        }

        // ось с вершинами в каждой точке пересечения; у каждого звена между
        // соседними вершинами — внутри оно площади или снаружи. Внутри/снаружи
        // меняется только на пересечении, так что точку в полигоне спрашиваем
        // одну на промежуток между ними, а не на каждое звено
        let mut points = vec![path[0]];
        let mut inside = Vec::with_capacity(path.len());
        let mut state = None;
        for segment in path.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let hits = self.crossings(from, to);
            let stops = hits
                .iter()
                .map(|&t| (from.lerp(to, t), true))
                .chain(std::iter::once((to, false)));
            for (point, crossing) in stops {
                let last = points[points.len() - 1];
                if point != last {
                    let covered = *state.get_or_insert_with(|| self.contains(last.midpoint(point)));
                    inside.push(covered);
                    points.push(point);
                }
                if crossing {
                    state = None;
                }
            }
        }
        if inside.is_empty() {
            return Vec::new();
        }

        // узкая площадь посреди оси — не разрез
        let lengths: Vec<f32> = points
            .windows(2)
            .map(|piece| piece[0].distance(piece[1]))
            .collect();
        let runs: Vec<(usize, usize)> = runs_of(&inside).collect();
        for (start, end) in runs {
            let bounded = start > 0 && end + 1 < inside.len();
            if inside[start] && bounded && lengths[start..=end].iter().sum::<f32>() < 2.0 * reach {
                inside[start..=end].fill(false);
            }
        }

        runs_of(&inside)
            .filter(|&(start, _)| !inside[start])
            .map(|(start, end)| {
                let last = points.len() - 1;
                let clipped = [start > 0, end + 1 < last];
                let mut run = Vec::new();
                if clipped[0] && reach > 0.0 {
                    let mut back = walk(points[..=start].iter().rev().copied(), reach);
                    back.reverse();
                    run.extend(back);
                }
                run.extend_from_slice(&points[start..=end + 1]);
                if clipped[1] && reach > 0.0 {
                    run.extend(walk(points[end + 1..].iter().copied(), reach));
                }
                OutsideRun {
                    points: run,
                    clipped,
                }
            })
            .collect()
    }
}

/// Промежутки `[start, end]` подряд одинаковых значений.
fn runs_of(flags: &[bool]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = 0;
    (0..flags.len()).filter_map(move |index| {
        let closes = index + 1 == flags.len() || flags[index + 1] != flags[index];
        closes.then(|| {
            let run = (start, index);
            start = index + 1;
            run
        })
    })
}

/// Точки ломаной, пройденной на `reach` от её первой точки (сама первая не
/// входит), с последней ровно на `reach`. Ломаная кончилась раньше — дальше по
/// прямой её последнего звена.
fn walk(chain: impl Iterator<Item = Vec2>, reach: f32) -> Vec<Vec2> {
    let mut chain = chain;
    let mut out = Vec::new();
    let Some(mut previous) = chain.next() else {
        return out;
    };
    let mut left = reach;
    let mut direction = Vec2::ZERO;
    for next in chain {
        let length = previous.distance(next);
        if length <= 0.0 {
            continue;
        }
        direction = (next - previous) / length;
        if length >= left {
            out.push(previous + direction * left);
            return out;
        }
        out.push(next);
        left -= length;
        previous = next;
    }
    if direction != Vec2::ZERO {
        out.push(previous + direction * left);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{square, water_area};

    #[test]
    fn a_zero_reach_cuts_exactly_on_the_outline() {
        // проезд входит в стоянку 100 × 100 с запада и кончается в её середине
        let lot = water_area(square(Vec2::ZERO, 50.0), Vec::new());
        let path = [Vec2::new(-80.0, 0.0), Vec2::ZERO];
        let found = AreaIndex::new(std::slice::from_ref(&lot)).outside_runs(&path, 0.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].clipped, [false, true]);
        assert_eq!(found[0].points.len(), 2);
        assert!(found[0].points[1].distance(Vec2::new(-50.0, 0.0)) < 1e-3);
    }
}
