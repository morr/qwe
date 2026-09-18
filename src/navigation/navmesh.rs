//! Тайловая сетка проходимости и её конверсии мир↔тайл. Работа с ней разложена
//! по ролям, и каждая роль — свой файл:
//!
//! - [`raster`] — растеризация: площади построчной заливкой, ленты полилиний;
//! - [`fill`] — заливка по `MapData`: что блокирует, что прорезает и в каком
//!   порядке;
//! - [`gates`] — калитки по умолчанию в глухих оградах;
//! - [`reach`] — достижимость: обход, соседи для A*, прунинг.
//!
//! Здесь остаётся сам тип: хранение, индексация и переводы координат, на
//! которые опираются все четверо.

mod fill;
mod gates;
mod raster;
mod reach;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bevy::prelude::*;

pub use self::reach::GatesAndPrune;
use crate::grid::{grid_size, navtile_size};

/// Множитель эвристики — та же шкала, что и стоимость шага
/// ([`Navmesh::successors`]).
pub const COST_MULTIPLIER: f32 = 100.0;

/// Тайловая сетка проходимости. Индексация — `x * grid_size.y + y`,
/// тайлы за границей карты непроходимы.
///
/// Размеры — поля, а не глобалы: заливка снимает их с текущего
/// [`navtile_size`], и всё, что работает по снапшоту (постройка northstar,
/// отменённая при смене размера), ходит по размерам **своего** снапшота,
/// а не по уже переключённому атомику.
///
/// Отсюда же и конверсии: растеризация и запросы к готовой сетке переводят
/// мир↔тайл через [`Self::to_tile`] / [`Self::tile_center`], а не через
/// `grid::world_to_tile` — тот считает по атомику.
///
/// `Clone` — для снапшота под постройку иерархии northstar: копия сетки
/// стоит один memcpy, а чтение оригинала под локом заняло бы все ~10 с
/// постройки (см. `northstar::start_northstar_build`).
#[derive(Clone)]
pub struct Navmesh {
    passable: Vec<bool>,
    /// Размер сетки в тайлах на момент заливки.
    pub grid_size: IVec2,
    /// Размер тайла в метрах на момент заливки.
    pub tile_size: f32,
}

impl Default for Navmesh {
    fn default() -> Self {
        let grid_size = grid_size();
        Self {
            passable: vec![true; (grid_size.x * grid_size.y) as usize],
            grid_size,
            tile_size: navtile_size(),
        }
    }
}

impl Navmesh {
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.grid_size.x && y < self.grid_size.y)
            .then_some((x * self.grid_size.y + y) as usize)
    }

    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.passable[index])
    }

    pub fn set_passable(&mut self, x: i32, y: i32, value: bool) {
        if let Some(index) = self.index(x, y) {
            self.passable[index] = value;
        }
    }

    /// Мир → тайл по размерам **своего** снапшота — пара к
    /// [`Self::tile_center`] и та же арифметика, что в `grid::world_to_tile`,
    /// но по `self.tile_size`. Всё, что растеризует в эту сетку или спрашивает
    /// её о точке, ходит через них: глобальные конверсии считают по уже
    /// переключённому атомику, и сетка, залитая при другом размере навтайла,
    /// отвечала бы про чужой тайл (см. док типа).
    pub fn to_tile(&self, position: Vec2) -> IVec2 {
        (position / self.tile_size).floor().as_ivec2()
    }

    /// Центр тайла в мировых метрах — по размерам своего снапшота.
    pub fn tile_center(&self, tile: IVec2) -> Vec2 {
        (tile.as_vec2() + 0.5) * self.tile_size
    }

    fn index_of(&self, tile: IVec2) -> Option<usize> {
        self.index(tile.x, tile.y)
    }

    fn index_center(&self, index: usize) -> Vec2 {
        let (x, y) = (
            index as i32 / self.grid_size.y,
            index as i32 % self.grid_size.y,
        );
        (Vec2::new(x as f32, y as f32) + 0.5) * self.tile_size
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
