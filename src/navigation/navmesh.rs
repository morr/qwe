use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bevy::prelude::*;

use crate::grid::world_to_tile;
use crate::map::osm::model::{MapData, PolyArea, distance_to_segment, ring_bounds};
use crate::settings::{GRID_SIZE, NAVTILE_SIZE, PASSAGE_MAX_WIDTH};

/// Стоимость шага между тайлами (для A*): прямой и диагональный.
pub const COST_STRAIGHT: i32 = 100;
pub const COST_DIAGONAL: i32 = 141;
/// Множитель эвристики — та же шкала, что и стоимость шага.
pub const COST_MULTIPLIER: f32 = 100.0;

/// Тайловая сетка проходимости. Индексация — `x * GRID_SIZE.y + y`,
/// тайлы за границей карты непроходимы.
///
/// `Clone` — для снапшота под постройку иерархии northstar: копия сетки
/// стоит один memcpy, а чтение оригинала под локом заняло бы все ~10 с
/// постройки (см. `northstar::start_northstar_build`).
#[derive(Clone)]
pub struct Navmesh {
    passable: Vec<bool>,
}

impl Default for Navmesh {
    fn default() -> Self {
        Self {
            passable: vec![true; (GRID_SIZE.x * GRID_SIZE.y) as usize],
        }
    }
}

impl Navmesh {
    fn index(x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < GRID_SIZE.x && y < GRID_SIZE.y)
            .then_some((x * GRID_SIZE.y + y) as usize)
    }

    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        Self::index(x, y).is_some_and(|index| self.passable[index])
    }

    pub fn set_passable(&mut self, x: i32, y: i32, value: bool) {
        if let Some(index) = Self::index(x, y) {
            self.passable[index] = value;
        }
    }

    /// Соседи тайла для A*: 8 направлений, диагональ только когда оба смежных
    /// прямых тайла проходимы (чтобы путь не резал углы зданий).
    pub fn successors(&self, x: i32, y: i32) -> Vec<(IVec2, i32)> {
        let mut result = Vec::with_capacity(8);
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if !self.is_passable(nx, ny) {
                continue;
            }
            let is_diagonal = dx != 0 && dy != 0;
            if is_diagonal && !(self.is_passable(x, ny) && self.is_passable(nx, y)) {
                continue;
            }
            result.push((
                IVec2::new(nx, ny),
                if is_diagonal {
                    COST_DIAGONAL
                } else {
                    COST_STRAIGHT
                },
            ));
        }
        result
    }

    /// Заполнение из OSM-карты. Порядок важен: мосты прорезают проходимые
    /// коридоры поверх воды (иначе Упа разрезает карту надвое), здания и стены
    /// блокируют уже после, а арки прорезаются последними — их смысл именно в
    /// том, чтобы пробить только что заблокированный дом.
    ///
    /// Линейные водотоки блокируют вместе с площадной водой и по той же
    /// причине — русло переходят по мосту, а не вброд. Опасность у них своя:
    /// ручей идёт через весь город непрерывной ниткой, и без переходов
    /// `prune_unreachable` ампутировал бы отрезанный берег (ровно поэтому
    /// рельсы в заливку не попадают вовсе). Держат карту связной две вещи:
    /// прорезка мостов **после** этой заливки и трубы, которые не блокируют
    /// (`WaterLine::tunnel`) — под дорогой ручей чаще убран в культверт, чем
    /// перекрыт мостом.
    pub fn fill_from_mapdata(&mut self, map: &MapData) {
        // сетка переживает смену города: без сброса на новой карте остались
        // бы дома и прунинг старой
        self.passable.fill(true);
        for area in &map.water {
            self.set_area(area, false);
        }
        for line in map.water_lines.iter().filter(|line| !line.tunnel) {
            self.set_polyline(&line.points, line.width, false);
        }
        for road in map.roads.iter().filter(|road| road.bridge) {
            self.set_polyline(&road.points, road.width, true);
        }
        for area in &map.buildings {
            self.set_area(area, false);
        }
        for wall in &map.walls {
            self.set_polyline(&wall.points, wall.width, false);
        }
        for road in map.roads.iter().filter(|road| road.passage) {
            self.set_polyline(&road.points, road.width.min(PASSAGE_MAX_WIDTH), true);
        }
    }

    /// Тайлы, чей центр внутри полигона (с учётом дырок) — построчной
    /// заливкой (см. `row_spans`).
    fn set_area(&mut self, area: &PolyArea, value: bool) {
        let (min, max) = ring_bounds(&area.outer);
        let min_tile = world_to_tile(min);
        let max_tile = world_to_tile(max);
        let mut scratch = RowScratch::default();
        for y in min_tile.y.max(0)..=max_tile.y.min(GRID_SIZE.y - 1) {
            row_spans(&area.outer, &area.holes, y, &mut scratch);
            for &(from, to) in &scratch.spans {
                for x in from.max(0)..=to.min(GRID_SIZE.x - 1) {
                    self.set_passable(x, y, value);
                }
            }
        }
    }

    /// Тайлы, недостижимые из `start`, становятся непроходимыми: замкнутые
    /// дворы и острова иначе порождают заведомо безуспешные A*-поиски,
    /// обходящие всю карту (десятки мс каждый). 4-связность совпадает с
    /// достижимостью A*: диагональ требует обоих смежных прямых тайлов.
    pub fn prune_unreachable(&mut self, start: IVec2) -> usize {
        let Some(start_index) = Self::index(start.x, start.y) else {
            return 0;
        };
        if !self.passable[start_index] {
            return 0;
        }

        let mut reachable = vec![false; self.passable.len()];
        let mut queue = std::collections::VecDeque::new();
        reachable[start_index] = true;
        queue.push_back(start);
        while let Some(tile) = queue.pop_front() {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (tile.x + dx, tile.y + dy);
                if let Some(index) = Self::index(nx, ny)
                    && self.passable[index]
                    && !reachable[index]
                {
                    reachable[index] = true;
                    queue.push_back(IVec2::new(nx, ny));
                }
            }
        }

        let mut pruned = 0;
        for (index, is_reachable) in reachable.iter().enumerate() {
            if self.passable[index] && !is_reachable {
                self.passable[index] = false;
                pruned += 1;
            }
        }
        pruned
    }

    /// Тайлы в пределах полуширины от осевой полилинии.
    fn set_polyline(&mut self, points: &[Vec2], width: f32, value: bool) {
        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let min_tile = world_to_tile(from.min(to) - width);
            let max_tile = world_to_tile(from.max(to) + width);
            for x in min_tile.x.max(0)..=max_tile.x.min(GRID_SIZE.x - 1) {
                for y in min_tile.y.max(0)..=max_tile.y.min(GRID_SIZE.y - 1) {
                    let center = (Vec2::new(x as f32, y as f32) + 0.5) * NAVTILE_SIZE;
                    if distance_to_segment(center, from, to) <= width / 2.0 {
                        self.set_passable(x, y, value);
                    }
                }
            }
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
fn ring_spans(ring: &[Vec2], scan_y: f32, crossings: &mut Vec<f32>, out: &mut Vec<(i32, i32)>) {
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
        // центр тайла x — это (x + 0.5) * NAVTILE_SIZE; ищем x с
        // pair[0] <= центр < pair[1]
        let from = (pair[0] / NAVTILE_SIZE - 0.5).ceil() as i32;
        let to = (pair[1] / NAVTILE_SIZE - 0.5).ceil() as i32 - 1;
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
fn row_spans(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, scratch: &mut RowScratch) {
    let scan_y = (y as f32 + 0.5) * NAVTILE_SIZE;
    let RowScratch {
        crossings,
        spans,
        holes: hole_spans,
    } = scratch;
    ring_spans(outer, scan_y, crossings, spans);

    for hole in holes {
        if spans.is_empty() {
            return;
        }
        ring_spans(hole, scan_y, crossings, hole_spans);
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

/// Navmesh под `Arc<RwLock>` — его читают async-задачи поиска пути.
#[derive(Resource)]
pub struct ArcNavmesh(pub Arc<RwLock<Navmesh>>);

impl Default for ArcNavmesh {
    fn default() -> Self {
        // пустой (всё проходимо); заполняется системой `fill_navmesh`,
        // когда `MapData` загружена
        Self(Arc::new(RwLock::new(Navmesh::default())))
    }
}

impl ArcNavmesh {
    pub fn read(&self) -> RwLockReadGuard<'_, Navmesh> {
        self.0.read().unwrap()
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Navmesh> {
        self.0.write().unwrap()
    }
}

#[cfg(test)]
mod tests;
