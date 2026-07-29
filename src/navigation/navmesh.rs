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
    pub fn fill_from_mapdata(&mut self, map: &MapData) {
        // сетка переживает смену города: без сброса на новой карте остались
        // бы дома и прунинг старой
        self.passable.fill(true);
        for area in &map.water {
            self.set_area(area, false);
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
mod tests {
    use crate::map::osm::model::{AreaKind, RoadClass, RoadLine, point_in_polygon};

    use super::*;

    /// Тайлы строки `y`, залитые построчной заливкой. Отрезки обрезаются по
    /// ширине проверяемой полосы — как `set_area` обрезает их по сетке.
    fn scanline_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
        let mut scratch = RowScratch::default();
        row_spans(outer, holes, y, &mut scratch);
        scratch
            .spans
            .iter()
            .flat_map(|&(from, to)| from.max(0)..=to.min(width - 1))
            .collect::<Vec<_>>()
    }

    /// Те же тайлы точечной проверкой — эталон, который заменила заливка.
    fn point_test_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
        (0..width)
            .filter(|&x| {
                let center = (Vec2::new(x as f32, y as f32) + 0.5) * NAVTILE_SIZE;
                point_in_polygon(center, outer)
                    && !holes.iter().any(|hole| point_in_polygon(center, hole))
            })
            .collect()
    }

    fn assert_same_fill(outer: &[Vec2], holes: &[Vec<Vec2>], rows: i32, width: i32) {
        for y in 0..rows {
            assert_eq!(
                scanline_row(outer, holes, y, width),
                point_test_row(outer, holes, y, width),
                "row {y}"
            );
        }
    }

    fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
        vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
    }

    #[test]
    fn scanline_matches_point_test_for_a_rect_with_a_hole() {
        let outer = rect(Vec2::new(3.0, 5.0), Vec2::new(41.0, 33.0));
        let holes = vec![rect(Vec2::new(11.0, 13.0), Vec2::new(25.0, 27.0))];
        assert_same_fill(&outer, &holes, 20, 25);
    }

    /// Вогнутый контур: строка пересекает его дважды, и заливка обязана дать
    /// два отрезка, а не один сплошной.
    #[test]
    fn scanline_matches_point_test_for_a_concave_ring() {
        let outer = vec![
            Vec2::new(2.0, 2.0),
            Vec2::new(30.0, 2.0),
            Vec2::new(30.0, 30.0),
            Vec2::new(24.0, 30.0),
            Vec2::new(24.0, 9.0),
            Vec2::new(8.0, 9.0),
            Vec2::new(8.0, 30.0),
            Vec2::new(2.0, 30.0),
        ];
        assert_same_fill(&outer, &[], 18, 18);
    }

    /// Дырка, наполовину вылезшая за внешнее кольцо. Если сваливать её рёбра
    /// в общий even-odd список, торчащий кусок не вычтется, а зальётся —
    /// именно поэтому дырки вычитаются отрезками.
    #[test]
    fn scanline_matches_point_test_for_a_hole_sticking_out() {
        let outer = rect(Vec2::new(6.0, 6.0), Vec2::new(30.0, 30.0));
        let holes = vec![rect(Vec2::new(20.0, 12.0), Vec2::new(44.0, 22.0))];
        assert_same_fill(&outer, &holes, 18, 25);
    }

    /// Косые рёбра — единственное место, где заливка могла бы разъехаться с
    /// точечной проверкой на полтайла.
    #[test]
    fn scanline_matches_point_test_for_a_diagonal_ring() {
        let outer = vec![
            Vec2::new(1.7, 0.3),
            Vec2::new(37.4, 11.9),
            Vec2::new(21.1, 34.6),
            Vec2::new(5.2, 20.8),
        ];
        assert_same_fill(&outer, &[], 20, 22);
    }

    /// Арка режется последней: дом уже залит непроходимым, и проём должен
    /// пробить его насквозь, не открыв при этом остального дома.
    #[test]
    fn a_building_passage_carves_a_corridor_through_the_building() {
        let mut map = MapData::default();
        map.buildings.push(PolyArea {
            outer: rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
            holes: Vec::new(),
            kind: AreaKind::Building,
            height: None,
            entrances: Vec::new(),
        });
        map.roads.push(RoadLine {
            points: vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
            width: 5.0,
            class: RoadClass::Street,
            bridge: false,
            passage: true,
        });

        let mut navmesh = Navmesh::default();
        navmesh.fill_from_mapdata(&map);

        let passable_at = |point: Vec2| {
            let tile = world_to_tile(point);
            navmesh.is_passable(tile.x, tile.y)
        };
        for y in [101.0, 115.0, 129.0] {
            assert!(passable_at(Vec2::new(131.0, y)), "проём на y={y}");
        }
        assert!(!passable_at(Vec2::new(110.0, 115.0)), "стена западнее арки");
        assert!(
            !passable_at(Vec2::new(150.0, 115.0)),
            "стена восточнее арки"
        );
    }

    /// Ширина арки ограничена: `service` шириной 5 м не должен вырезать по
    /// тайлу фасада с каждой стороны проёма.
    #[test]
    fn a_passage_is_no_wider_than_the_cap() {
        let mut map = MapData::default();
        map.buildings.push(PolyArea {
            outer: rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
            holes: Vec::new(),
            kind: AreaKind::Building,
            height: None,
            entrances: Vec::new(),
        });
        map.roads.push(RoadLine {
            points: vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
            width: 12.0,
            class: RoadClass::Street,
            bridge: false,
            passage: true,
        });

        let mut navmesh = Navmesh::default();
        navmesh.fill_from_mapdata(&map);

        let row = world_to_tile(Vec2::new(131.0, 115.0)).y;
        let open = (0..GRID_SIZE.x)
            .filter(|&x| {
                let center = (x as f32 + 0.5) * NAVTILE_SIZE;
                (100.0..160.0).contains(&center) && navmesh.is_passable(x, row)
            })
            .count();
        assert!(
            open as f32 <= PASSAGE_MAX_WIDTH / NAVTILE_SIZE + 1.0,
            "проём в {open} тайлов шире потолка"
        );
    }
}
