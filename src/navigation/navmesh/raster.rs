//! Растеризация в сетку: площади — построчной заливкой, ленты — капсульным
//! сметанием плюс цепочка тайлов по осевой. Отсюда рисуют все заливки
//! ([`super::fill`]), сама она про предметную область не знает.
//!
//! Мир↔тайл здесь переводится только через снимок сетки самого навмеша
//! ([`Navmesh::to_tile`]), а не через `crate::grid::world_to_tile`, читающий
//! процессный атомик размера навтайла.

use bevy::prelude::*;

use super::Navmesh;
use crate::map::osm::model::{PolyArea, distance_to_segment, ring_bounds};

impl Navmesh {
    /// Тайлы, чей центр внутри полигона (с учётом дырок) — построчной
    /// заливкой (см. `row_spans`).
    pub(super) fn set_area(&mut self, area: &PolyArea, value: bool) {
        let (min, max) = ring_bounds(&area.outer);
        let min_tile = self.to_tile(min);
        let max_tile = self.to_tile(max);
        let mut scratch = RowScratch::default();
        for y in min_tile.y.max(0)..=max_tile.y.min(self.grid_size.y - 1) {
            row_spans(&area.outer, &area.holes, y, self.tile_size, &mut scratch);
            for &(from, to) in &scratch.spans {
                for x in from.max(0)..=to.min(self.grid_size.x - 1) {
                    self.set_passable(x, y, value);
                }
            }
        }
    }

    /// Тайлы в пределах полуширины от осевой полилинии — **плюс** все тайлы,
    /// через которые осевая проходит ([`Self::visit_segment_tiles`]).
    ///
    /// Одной полуширины мало. Тайлы метятся по «центр ближе полуширины», и
    /// лента у́же `tile_size · √2` (2.83 м при тайле 2 м) на косой линии вырождается в
    /// цепочку тайлов, соприкасающихся **углами**: ручей в 2.5 м рисуется в
    /// навмеш-оверлее шахматкой. Своим A* её не перейти — он не срезает углы, —
    /// но `OrdinalGrid` из `bevy_northstar` (HPA*, Theta*) собирается без
    /// такого фильтра и шагает по диагонали прямо между двумя
    /// заблокированными тайлами, а `line_of_sight` сэмплирует точки и
    /// проскакивает через место касания. На Туле это и вышло: на HPA* люди
    /// ходили через ручей.
    ///
    /// Поднимать ширину до минимума — лечение симптома: порог зависит от угла и
    /// от сдвига линии относительно сетки, и даже 3 м оставляли щель. Проход по
    /// осевой даёт четырёхсвязную цепочку **по построению**, при любой ширине,
    /// угле и сдвиге, и при этом не раздувает канаву в 1.5 м до трёх метров.
    pub(super) fn set_polyline(&mut self, points: &[Vec2], width: f32, value: bool) {
        self.set_polyline_capped(points, width, value, [true; 2]);
    }

    /// То же, но с управляемыми торцами — `[начало, конец]`, `true` — капсульное
    /// продление за конец. Срезанный торец нужен руслу на входе в трубу: там
    /// вода уходит под землю, и полукруг непроходимых тайлов за узлом глушил бы
    /// вход в культверт (`water_line_caps`; отрисовка режет тот же торец).
    pub(super) fn set_polyline_capped(
        &mut self,
        points: &[Vec2],
        width: f32,
        value: bool,
        round_caps: [bool; 2],
    ) {
        self.visit_polyline_capped(points, width, round_caps, &mut |grid, x, y| {
            grid.set_passable(x, y, value)
        });
    }

    /// Обход тех же тайлов без записи в сетку: `visit` решает сам — так
    /// собирается маска бордюров и режутся проёмы «только там, где бордюр».
    pub(super) fn visit_polyline(
        &mut self,
        points: &[Vec2],
        width: f32,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        self.visit_polyline_capped(points, width, [true; 2], visit);
    }

    fn visit_polyline_capped(
        &mut self,
        points: &[Vec2],
        width: f32,
        round_caps: [bool; 2],
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        let (grid_size, tile_size) = (self.grid_size, self.tile_size);
        let last = points.len().saturating_sub(2);
        for (index, segment) in points.windows(2).enumerate() {
            let (from, to) = (segment[0], segment[1]);
            // срез торца: тайл за плоскостью конца не в ленте, даже если до
            // самого узла ему ближе полуширины
            let butt_start = index == 0 && !round_caps[0];
            let butt_end = index == last && !round_caps[1];
            let along = (to - from).normalize_or_zero();
            let min_tile = self.to_tile(from.min(to) - width);
            let max_tile = self.to_tile(from.max(to) + width);
            for x in min_tile.x.max(0)..=max_tile.x.min(grid_size.x - 1) {
                for y in min_tile.y.max(0)..=max_tile.y.min(grid_size.y - 1) {
                    let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
                    if distance_to_segment(center, from, to) > width / 2.0 {
                        continue;
                    }
                    if butt_start && (center - from).dot(along) < 0.0 {
                        continue;
                    }
                    if butt_end && (center - to).dot(along) > 0.0 {
                        continue;
                    }
                    visit(self, x, y);
                }
            }
            self.visit_segment_tiles(from, to, visit);
        }
    }

    /// Тайлы в прямоугольнике вокруг каждого сегмента: как
    /// [`Self::visit_polyline`], но без капсульных продлений за концы (та же
    /// разница, что `Butt` против `Round` у торцов ленты в отрисовке) и без
    /// цепочки по осевой — покрытию бордюров связность не нужна.
    pub(super) fn visit_polyline_rect(
        &mut self,
        points: &[Vec2],
        width: f32,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let delta = to - from;
            let length = delta.length();
            let Some(direction) = delta.try_normalize() else {
                continue;
            };
            let min_tile = self.to_tile(from.min(to) - width);
            let max_tile = self.to_tile(from.max(to) + width);
            for x in min_tile.x.max(0)..=max_tile.x.min(self.grid_size.x - 1) {
                for y in min_tile.y.max(0)..=max_tile.y.min(self.grid_size.y - 1) {
                    let center = (Vec2::new(x as f32, y as f32) + 0.5) * self.tile_size;
                    let along = (center - from).dot(direction);
                    let lateral = (center - from).perp_dot(direction).abs();
                    if (0.0..=length).contains(&along) && lateral <= width / 2.0 {
                        visit(self, x, y);
                    }
                }
            }
        }
    }

    /// Тайлы, через которые проходит отрезок, — обход сетки по Amanatides–Woo:
    /// на каждом шаге пересекается ближайшая граница, по x либо по y, поэтому
    /// соседние тайлы цепочки всегда смежны **по стороне**, а не по углу.
    /// Именно эта четырёхсвязность и делает преграду непроходимой для всех
    /// потребителей сетки (см. [`Self::set_polyline`]).
    fn visit_segment_tiles(
        &mut self,
        from: Vec2,
        to: Vec2,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        let mut tile = self.to_tile(from);
        let end = self.to_tile(to);
        let delta = to - from;

        // t — доля отрезка; t_max — до следующей границы по оси, t_delta — шаг
        // между границами. Нулевая проекция даёт бесконечность: по этой оси
        // граница не пересекается никогда.
        let tile_size = self.tile_size;
        let axis = move |d: f32, origin: f32, tile: i32| {
            if d == 0.0 {
                return (f32::INFINITY, f32::INFINITY, 0);
            }
            let step = if d > 0.0 { 1 } else { -1 };
            let boundary = (tile + step.max(0)) as f32 * tile_size;
            ((boundary - origin) / d, tile_size / d.abs(), step)
        };
        let (mut t_max_x, t_delta_x, step_x) = axis(delta.x, from.x, tile.x);
        let (mut t_max_y, t_delta_y, step_y) = axis(delta.y, from.y, tile.y);

        visit(self, tile.x, tile.y);
        // потолок шагов — страховка от вырожденного отрезка: ходов не больше,
        // чем тайлов по обеим осям вместе
        let limit = (end.x - tile.x).abs() + (end.y - tile.y).abs();
        for _ in 0..limit {
            if t_max_x < t_max_y {
                tile.x += step_x;
                t_max_x += t_delta_x;
            } else {
                tile.y += step_y;
                t_max_y += t_delta_y;
            }
            visit(self, tile.x, tile.y);
        }
    }
}

/// x-координаты пересечений кольца с горизонталью `scan_y`. Условие
/// пересечения — ровно то же, что в `point_in_polygon`, включая строгие
/// сравнения: иначе построчная заливка разошлась бы с точечной проверкой на
/// кромке полигона.
fn ring_crossings(ring: &[Vec2], scan_y: f32, out: &mut Vec<f32>) {
    if ring.len() < 2 {
        return;
    }
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > scan_y) != (b.y > scan_y) {
            out.push((b.x - a.x) * (scan_y - a.y) / (b.y - a.y) + a.x);
        }
        j = i;
    }
}

/// Отрезки тайлов `[from, to]` строки `y`, чьи центры лежат внутри кольца.
///
/// Это и есть замена перебору «каждый тайл AABB × всё кольцо»: кольцо
/// проходится один раз на строку, а не один раз на тайл. На доме разницы
/// нет, на Темзе — три порядка (её AABB тянется через полкарты).
fn ring_spans(
    ring: &[Vec2],
    scan_y: f32,
    tile_size: f32,
    crossings: &mut Vec<f32>,
    out: &mut Vec<(i32, i32)>,
) {
    out.clear();
    crossings.clear();
    ring_crossings(ring, scan_y, crossings);
    if crossings.is_empty() {
        return;
    }
    crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // `point_in_polygon` переключает флаг на каждом пересечении справа от
    // точки, значит внутренние отрезки — это пары [c0, c1), [c2, c3), …
    // Нечётный хвост (вырожденное кольцо) отбрасывается вместе с `chunks_exact`.
    for pair in crossings.chunks_exact(2) {
        // центр тайла x — это (x + 0.5) * tile_size; ищем x с
        // pair[0] <= центр < pair[1]
        let from = (pair[0] / tile_size - 0.5).ceil() as i32;
        let to = (pair[1] / tile_size - 0.5).ceil() as i32 - 1;
        if from <= to {
            out.push((from, to));
        }
    }
}

/// Переиспользуемые буферы построчной заливки: на реке строк тысячи, и
/// аллокация на каждую съела бы часть выигрыша.
#[derive(Default)]
struct RowScratch {
    crossings: Vec<f32>,
    /// Результат строки — отрезки внешнего кольца за вычетом дырок.
    spans: Vec<(i32, i32)>,
    holes: Vec<(i32, i32)>,
}

/// Отрезки строки `y` для полигона с дырками.
///
/// Дырки вычитаются отрезками, а не сваливаются в общий even-odd список:
/// even-odd совпал бы с прежней поточечной проверкой только для дырок строго
/// внутри внешнего кольца, а кусок дырки, вылезший наружу (кривая
/// OSM-мультиполигональная связка), он бы, наоборот, залил.
fn row_spans(
    outer: &[Vec2],
    holes: &[Vec<Vec2>],
    y: i32,
    tile_size: f32,
    scratch: &mut RowScratch,
) {
    let scan_y = (y as f32 + 0.5) * tile_size;
    let RowScratch {
        crossings,
        spans,
        holes: hole_spans,
    } = scratch;
    ring_spans(outer, scan_y, tile_size, crossings, spans);

    for hole in holes {
        if spans.is_empty() {
            return;
        }
        ring_spans(hole, scan_y, tile_size, crossings, hole_spans);
        for &(cut_from, cut_to) in hole_spans.iter() {
            let mut index = 0;
            while index < spans.len() {
                let (from, to) = spans[index];
                if cut_to < from || cut_from > to {
                    index += 1;
                    continue;
                }
                spans.remove(index);
                if cut_to < to {
                    spans.insert(index, (cut_to + 1, to));
                }
                if from < cut_from {
                    spans.insert(index, (from, cut_from - 1));
                    index += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
