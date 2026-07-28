//! Пространственные сетки для поиска ближайших демонов/людей без O(люди ×
//! демоны). Ячейка 60 м ≥ максимального радиуса (паника 60 м), поэтому поиск
//! в радиусе сводится к обходу 3 × 3 соседних ячеек.

use std::marker::PhantomData;

use bevy::prelude::*;

use crate::demon::Demon;
use crate::human::Human;
use crate::movement::SimPosition;
use crate::settings::MAP_SIZE;

pub const CELL_SIZE: f32 = 60.0;

/// Порядок симуляции в `FixedUpdate`: сетки → демоны → люди. Демоны раньше
/// людей, чтобы убийства применились до `escape` и человек не был засчитан
/// и убитым, и спасшимся в один тик.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    SpatialRebuild,
    DemonBehavior,
    HumanBehavior,
}

/// Равномерная Vec-сетка позиций сущностей типа-маркера `T`.
/// Пересобирается целиком каждый тик `FixedUpdate` — дёшево и для 5000.
#[derive(Resource)]
pub struct SpatialGrid<T: Send + Sync + 'static> {
    cells: Vec<Vec<(Entity, Vec2)>>,
    width: i32,
    height: i32,
    _marker: PhantomData<T>,
}

impl<T: Send + Sync + 'static> Default for SpatialGrid<T> {
    fn default() -> Self {
        let width = (MAP_SIZE.x / CELL_SIZE).ceil() as i32;
        let height = (MAP_SIZE.y / CELL_SIZE).ceil() as i32;
        Self {
            cells: (0..width * height).map(|_| Vec::new()).collect(),
            width,
            height,
            _marker: PhantomData,
        }
    }
}

impl<T: Send + Sync + 'static> SpatialGrid<T> {
    fn cell_coords(&self, pos: Vec2) -> (i32, i32) {
        (
            ((pos.x / CELL_SIZE) as i32).clamp(0, self.width - 1),
            ((pos.y / CELL_SIZE) as i32).clamp(0, self.height - 1),
        )
    }

    pub fn rebuild(&mut self, entries: impl Iterator<Item = (Entity, Vec2)>) {
        for cell in &mut self.cells {
            cell.clear();
        }
        for (entity, pos) in entries {
            let (x, y) = self.cell_coords(pos);
            self.cells[(x * self.height + y) as usize].push((entity, pos));
        }
    }

    /// Ближайшая сущность не дальше `radius` от `pos`.
    pub fn nearest_in_range(&self, pos: Vec2, radius: f32) -> Option<(Entity, Vec2)> {
        let (cx, cy) = self.cell_coords(pos);
        let cell_span = (radius / CELL_SIZE).ceil() as i32;
        let mut best: Option<(Entity, Vec2)> = None;
        let mut best_distance_squared = radius * radius;

        for x in (cx - cell_span).max(0)..=(cx + cell_span).min(self.width - 1) {
            for y in (cy - cell_span).max(0)..=(cy + cell_span).min(self.height - 1) {
                for &(entity, entry_pos) in &self.cells[(x * self.height + y) as usize] {
                    let distance_squared = pos.distance_squared(entry_pos);
                    if distance_squared <= best_distance_squared {
                        best_distance_squared = distance_squared;
                        best = Some((entity, entry_pos));
                    }
                }
            }
        }
        best
    }
}

pub struct SpatialPlugin;

impl Plugin for SpatialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpatialGrid<Demon>>()
            .init_resource::<SpatialGrid<Human>>()
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::SpatialRebuild,
                    SimSet::DemonBehavior,
                    SimSet::HumanBehavior,
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                (rebuild_demon_grid, rebuild_human_grid).in_set(SimSet::SpatialRebuild),
            );
    }
}

fn rebuild_demon_grid(
    mut grid: ResMut<SpatialGrid<Demon>>,
    query: Query<(Entity, &SimPosition), With<Demon>>,
) {
    grid.rebuild(query.iter().map(|(entity, pos)| (entity, pos.0)));
}

/// Сетка людей: только живые (у трупов поведенческие компоненты сняты,
/// `SimPosition` остаётся — фильтруем по `Human`, который снимается со смертью).
fn rebuild_human_grid(
    mut grid: ResMut<SpatialGrid<Human>>,
    query: Query<(Entity, &SimPosition), With<Human>>,
) {
    grid.rebuild(query.iter().map(|(entity, pos)| (entity, pos.0)));
}
