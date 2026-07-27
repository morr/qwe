mod astar;
mod navmesh;

use bevy::prelude::*;

pub use self::astar::astar_pathfinding;
pub use self::navmesh::{ArcNavmesh, COST_DIAGONAL, COST_MULTIPLIER, COST_STRAIGHT, Navmesh};

/// Ответ асинхронного поиска пути (снимается в
/// `movement::listen_for_pathfinding_tasks`).
#[derive(Debug)]
pub struct PathfindingResult {
    pub path: Option<Vec<IVec2>>,
    pub start_tile: IVec2,
    pub end_tile: IVec2,
}

/// Проходимый тайл в точке или среди 8 соседей — иначе `None`.
pub fn find_passable_tile_near(navmesh: &Navmesh, tile: IVec2) -> Option<IVec2> {
    if navmesh.is_passable(tile.x, tile.y) {
        return Some(tile);
    }
    [
        IVec2::new(-1, 0),
        IVec2::new(1, 0),
        IVec2::new(0, -1),
        IVec2::new(0, 1),
        IVec2::new(-1, -1),
        IVec2::new(-1, 1),
        IVec2::new(1, -1),
        IVec2::new(1, 1),
    ]
    .iter()
    .map(|&offset| tile + offset)
    .find(|candidate| navmesh.is_passable(candidate.x, candidate.y))
}

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArcNavmesh>();
    }
}
