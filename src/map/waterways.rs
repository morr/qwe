//! Лента открытых русел (`waterways`) — отдельным слоем над дорогами, но под
//! мостами, и **с вырезом там, где русло лежит внутри площадной воды**.
//!
//! Русло в OSM — осевая линия, и она не знает о полигоне реки, в который
//! впадает: осевая Упы идёт внутри своего же `riverbank` на всём протяжении, а
//! ручей доходит до пруда и продолжается в нём до узла на осевой. Внутри
//! полигона лента невидима (цвет и фактура у слоёв общие), зато поперёк его
//! отмели она ложилась ровным прямоугольником глубокой воды. Поэтому ось
//! режется по контурам площадной воды, а у каждого отрезанного конца лента
//! заходит за берег ровно на ширину отмели ([`WATER_SHORE_WIDTH`]) — и её
//! собственная отмель (кромки ленты, шейдер `surface.wgsl`) гаснет на этом
//! заходе так же, как гаснет кайма полигона вглубь от берега. На стыке у
//! кромки ленты и у каймы берега на одной глубине один и тот же цвет, и отмель
//! заворачивает из реки в русло без шва.
//!
//! Сетку это не трогает: навмеш глушит и полигон, и полосу русла целиком
//! (`Navmesh::fill_from_mapdata`), и то, что лента внутри полигона больше не
//! рисуется, проходимости не меняет.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::meshing::{Break, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin};
use crate::map::osm::model::{grid_cell, point_in_area, put_in_cells, ring_bounds};
use crate::map::osm::{PolyArea, WaterLine, water_line_caps};
use crate::map::roads::{self, RoadSmoothing};
use crate::map::spawn::{WATER_COLOR, WATER_SHORE_WIDTH};

/// Шаг сетки рёбер водных контуров, м. Контур реки — тысячи рёбер, и русло
/// спрашивает только те, что лежат в клетках его звена.
const CELL: f32 = 32.0;

/// Пересечения ближе этого вдоль звена — одно: общая вершина двух рёбер
/// контура даёт два попадания в одну точку.
const SAME_CROSSING: f32 = 1e-4;

/// Лента открытых русел одним мешем. **Трубы не рисуются вовсе**: под землёй
/// воды не видно, а пунктир вдоль улицы читался как ручей поверх неё. Тем, что
/// человек проходит там, где на карте «ручей», управляет не эта отрисовка, а
/// её отсутствие: русло обрывается на портале культверта и продолжается за ним
/// (`water_line_caps`), и между порталами воды на карте просто нет.
pub fn mesh_water_lines(lines: &[WaterLine], water: &[PolyArea]) -> MeshBuilder {
    let color = WATER_COLOR.to_linear();
    let index = WaterIndex::new(water);
    let mut open = MeshBuilder::with_surface_coords();

    for line in lines.iter().filter(|line| !line.tunnel) {
        // сглаживание как у дорог: русло в OSM — ломаная по точкам съёмки, и на
        // её изломах лента без сглаживания заметно гранёная. Режется уже
        // сглаженная ось — та, по которой лента и ляжет
        let path = roads::smooth_path(&line.points, line.width, RoadSmoothing::Light);
        // круглые торцы там, где вода продолжается: два way одного русла
        // встречаются в общем узле, и полудиски сливаются в непрерывную реку.
        // Портал культверта — исключение: за ним воды нет, и полудиск торчал бы
        // на полуширину русла в сухую землю
        let caps = water_line_caps(line, lines).map(|round| {
            if round {
                RibbonCap::Round
            } else {
                RibbonCap::Butt
            }
        });
        for run in index.open_runs(&path, WATER_SHORE_WIDTH) {
            // отрезанный конец лежит в воде на ширину отмели глубже берега:
            // полудиск там ни к чему, а «до разрыва» от берега до торца идёт от
            // нуля к минус ширине отмели — по нему шейдер гасит кромки ленты
            let mut breaks = Vec::with_capacity(2);
            let mut run_caps = caps;
            for (end, point) in [(0, run.points[0]), (1, run.points[run.points.len() - 1])] {
                if run.clipped[end] {
                    run_caps[end] = RibbonCap::Butt;
                    breaks.push(Break {
                        at: point,
                        reach: WATER_SHORE_WIDTH,
                    });
                }
            }
            // `At` даже без разрывов: у `Ends` «до разрыва» считается до торца, и
            // отмель гасла бы у каждого конца русла, в том числе на суше
            open.push_ribbon_broken(
                &run.points,
                line.width,
                color,
                RibbonJoin::Round,
                run_caps,
                RibbonBreaks::At(&breaks),
            );
        }
    }

    open
}

/// Кусок оси русла вне площадной воды, уже с заходом за берег, и какие из двух
/// его концов отрезаны контуром (`[начало, конец]`), а не пришли из OSM.
#[derive(Debug)]
struct OpenRun {
    points: Vec<Vec2>,
    clipped: [bool; 2],
}

/// Площадная вода для резки русел: сами полигоны с их AABB и сетка рёбер всех
/// их колец.
struct WaterIndex<'a> {
    areas: &'a [PolyArea],
    bounds: Vec<(Vec2, Vec2)>,
    edges: HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
}

impl<'a> WaterIndex<'a> {
    fn new(areas: &'a [PolyArea]) -> Self {
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

    /// Доли звена `from → to`, где оно пересекает ребро водного контура, по
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
                    // начало звена считается, конец — нет: берег, пришедший ровно в
                    // вершину оси, обязан сбросить «внутри/снаружи» один раз, а не
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

    /// Куски оси вне воды. Каждый отрезанный конец продолжен в воду на `reach`
    /// вдоль той же оси, а за её концом — по прямой последнего звена.
    ///
    /// Вода между двумя кусками, что уже `2 · reach`, не режет русло вовсе:
    /// заходы с обеих сторон всё равно легли бы друг на друга, а две ленты
    /// внахлёст с отмелью, гаснущей навстречу, дают шов посреди протоки.
    fn open_runs(&self, path: &[Vec2], reach: f32) -> Vec<OpenRun> {
        if path.len() < 2 {
            return Vec::new();
        }

        // ось с вершинами в каждой точке пересечения; у каждого звена между
        // соседними вершинами — внутри оно воды или снаружи. Внутри/снаружи
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
                    let wet = *state.get_or_insert_with(|| self.contains(last.midpoint(point)));
                    inside.push(wet);
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

        // узкая вода посреди русла — не разрез
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
                if clipped[0] {
                    let mut back = walk(points[..=start].iter().rev().copied(), reach);
                    back.reverse();
                    run.extend(back);
                }
                run.extend_from_slice(&points[start..=end + 1]);
                if clipped[1] {
                    run.extend(walk(points[end + 1..].iter().copied(), reach));
                }
                OpenRun {
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
mod tests;
