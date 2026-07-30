//! Пространственные сетки для поиска ближайших демонов/людей без O(люди ×
//! демоны). Ячейка 60 м ≥ максимального радиуса (паника 60 м), поэтому поиск
//! в радиусе сводится к обходу 3 × 3 соседних ячеек.

use std::marker::PhantomData;

use bevy::prelude::*;

use crate::demon::Demon;
use crate::human::Human;
use crate::movement::SimPosition;
use crate::settings::{HUMAN_PANIC_RADIUS, MAP_SIZE};

pub const CELL_SIZE: f32 = 60.0;

// Гарантия danger-карты «ноль в ячейке = демона ближе радиуса паники нет»
// держится только пока ячейка не меньше радиуса.
const _: () = assert!(HUMAN_PANIC_RADIUS <= CELL_SIZE);

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
        self.nearest_in_range_where(pos, radius, |_| true)
    }

    /// Ближайшая сущность не дальше `radius`, проходящая фильтр
    /// (например, «её ещё никто не преследует»).
    pub fn nearest_in_range_where(
        &self,
        pos: Vec2,
        radius: f32,
        mut filter: impl FnMut(Entity) -> bool,
    ) -> Option<(Entity, Vec2)> {
        let (cx, cy) = self.cell_coords(pos);
        let cell_span = (radius / CELL_SIZE).ceil() as i32;
        let mut best: Option<(Entity, Vec2)> = None;
        let mut best_distance_squared = radius * radius;

        for x in (cx - cell_span).max(0)..=(cx + cell_span).min(self.width - 1) {
            for y in (cy - cell_span).max(0)..=(cy + cell_span).min(self.height - 1) {
                for &(entity, entry_pos) in &self.cells[(x * self.height + y) as usize] {
                    let distance_squared = pos.distance_squared(entry_pos);
                    if distance_squared <= best_distance_squared && filter(entity) {
                        best_distance_squared = distance_squared;
                        best = Some((entity, entry_pos));
                    }
                }
            }
        }
        best
    }
}

/// Danger-карта: помечены ячейки, в 3×3-окрестности которых есть демон.
/// Ячейка ≥ радиуса паники, поэтому непомеченная ячейка гарантирует: демона
/// ближе `HUMAN_PANIC_RADIUS` нет — `panic` пропускает такого человека по
/// одному чтению вместо обхода 3×3 ячеек сетки демонов на каждого из ~20 000.
///
/// Пересобирается с нуля каждый тик вместе с сеткой демонов: 100 демонов ×
/// 9 ячеек — микросекунды, инкрементальное сопровождение не окупается и несёт
/// инварианты (спавн, рестарт, смена города, lunge мимо движения).
#[derive(Resource)]
pub struct DemonDangerMap {
    cells: Vec<bool>,
    width: i32,
    height: i32,
}

impl Default for DemonDangerMap {
    fn default() -> Self {
        let width = (MAP_SIZE.x / CELL_SIZE).ceil() as i32;
        let height = (MAP_SIZE.y / CELL_SIZE).ceil() as i32;
        Self {
            cells: vec![false; (width * height) as usize],
            width,
            height,
        }
    }
}

impl DemonDangerMap {
    fn cell_coords(&self, pos: Vec2) -> (i32, i32) {
        (
            ((pos.x / CELL_SIZE) as i32).clamp(0, self.width - 1),
            ((pos.y / CELL_SIZE) as i32).clamp(0, self.height - 1),
        )
    }

    pub fn rebuild(&mut self, demons: impl Iterator<Item = Vec2>) {
        self.cells.fill(false);
        for pos in demons {
            let (cx, cy) = self.cell_coords(pos);
            for x in (cx - 1).max(0)..=(cx + 1).min(self.width - 1) {
                for y in (cy - 1).max(0)..=(cy + 1).min(self.height - 1) {
                    self.cells[(x * self.height + y) as usize] = true;
                }
            }
        }
    }

    /// Гарантированно ли в радиусе паники от точки нет ни одного демона.
    /// Грубый префильтр: `false` означает лишь «демон может быть рядом»,
    /// точную дистанцию решает `nearest_in_range`.
    pub fn is_safe(&self, pos: Vec2) -> bool {
        let (cx, cy) = self.cell_coords(pos);
        !self.cells[(cx * self.height + cy) as usize]
    }
}

pub struct SpatialPlugin;

impl Plugin for SpatialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpatialGrid<Demon>>()
            .init_resource::<SpatialGrid<Human>>()
            .init_resource::<DemonDangerMap>()
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::SpatialRebuild,
                    SimSet::DemonBehavior,
                    SimSet::HumanBehavior,
                )
                    .chain()
                    .run_if(in_state(crate::loading::AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (rebuild_demon_grid, rebuild_human_grid).in_set(SimSet::SpatialRebuild),
            );
    }
}

fn rebuild_demon_grid(
    mut grid: ResMut<SpatialGrid<Demon>>,
    mut danger: ResMut<DemonDangerMap>,
    query: Query<(Entity, &SimPosition), With<Demon>>,
) {
    grid.rebuild(query.iter().map(|(entity, pos)| (entity, pos.0)));
    danger.rebuild(query.iter().map(|(_, pos)| pos.0));
}

/// Сетка людей: только живые (у трупов поведенческие компоненты сняты,
/// `SimPosition` остаётся — фильтруем по `Human`, который снимается со смертью).
fn rebuild_human_grid(
    mut diagnostics: bevy::diagnostic::Diagnostics,
    mut grid: ResMut<SpatialGrid<Human>>,
    query: Query<(Entity, &SimPosition), With<Human>>,
) {
    let started = std::time::Instant::now();
    grid.rebuild(query.iter().map(|(entity, pos)| (entity, pos.0)));
    crate::diagnostics::measure_ms(
        &mut diagnostics,
        &crate::diagnostics::SIM_SPATIAL_MS,
        started,
    );
}
