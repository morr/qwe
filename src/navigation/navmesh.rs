use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bevy::prelude::*;

use crate::grid::world_to_tile;
use crate::map::data;
use crate::settings::{GRID_SIZE, NAVTILE_SIZE};

/// Стоимость шага между тайлами (для A*): прямой и диагональный.
pub const COST_STRAIGHT: i32 = 100;
pub const COST_DIAGONAL: i32 = 141;
/// Множитель эвристики — та же шкала, что и стоимость шага.
pub const COST_MULTIPLIER: f32 = 100.0;

/// Тайловая сетка проходимости. Индексация — `x * GRID_SIZE.y + y`,
/// тайлы за границей карты непроходимы.
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

    /// Заполнение из данных карты: здания и пруд → непроходимые тайлы.
    pub fn fill_from_map(&mut self) {
        for building in data::buildings() {
            let min_tile = world_to_tile(building.min);
            let max_tile = world_to_tile(building.max());
            for x in min_tile.x..=max_tile.x {
                for y in min_tile.y..=max_tile.y {
                    self.set_passable(x, y, false);
                }
            }
        }

        let pond_min = world_to_tile(data::POND_CENTER - data::POND_RADII);
        let pond_max = world_to_tile(data::POND_CENTER + data::POND_RADII);
        for x in pond_min.x..=pond_max.x {
            for y in pond_min.y..=pond_max.y {
                let center = (Vec2::new(x as f32, y as f32) + 0.5) * NAVTILE_SIZE;
                if data::is_in_pond(center) {
                    self.set_passable(x, y, false);
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
        let mut navmesh = Navmesh::default();
        navmesh.fill_from_map();
        Self(Arc::new(RwLock::new(navmesh)))
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
