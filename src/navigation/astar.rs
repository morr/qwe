use bevy::math::{IVec2, Vec2};
use pathfinding::directed::astar::astar;

use crate::navigation::navmesh::{COST_MULTIPLIER, Navmesh};

/// A* по тайлам navmesh. Возвращает путь, включая стартовый тайл, либо `None`,
/// если цель непроходима или недостижима.
pub fn astar_pathfinding(navmesh: &Navmesh, start: IVec2, end: IVec2) -> Option<Vec<IVec2>> {
    if !navmesh.is_passable(end.x, end.y) {
        return None;
    }

    astar(
        &start,
        |&IVec2 { x, y }| navmesh.successors(x, y),
        |&pos| {
            let length = (Vec2::new(pos.x as f32, pos.y as f32)
                - Vec2::new(end.x as f32, end.y as f32))
            .length();
            (length * COST_MULTIPLIER) as i32
        },
        |pos| *pos == end,
    )
    .map(|(path, _cost)| path)
}
