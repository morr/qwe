use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bevy::prelude::*;

use crate::grid::world_to_tile;
use crate::map::osm::model::{MapData, PolyArea, distance_to_segment, point_in_area, ring_bounds};
use crate::settings::{GRID_SIZE, NAVTILE_SIZE};

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
    /// коридоры поверх воды (иначе Упа разрезает карту надвое), а здания и
    /// стены блокируют уже после.
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
    }

    /// Тайлы, чей центр внутри полигона (с учётом дырок).
    fn set_area(&mut self, area: &PolyArea, value: bool) {
        let (min, max) = ring_bounds(&area.outer);
        let min_tile = world_to_tile(min);
        let max_tile = world_to_tile(max);
        for x in min_tile.x.max(0)..=max_tile.x.min(GRID_SIZE.x - 1) {
            for y in min_tile.y.max(0)..=max_tile.y.min(GRID_SIZE.y - 1) {
                let center = (Vec2::new(x as f32, y as f32) + 0.5) * NAVTILE_SIZE;
                if point_in_area(center, area) {
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
