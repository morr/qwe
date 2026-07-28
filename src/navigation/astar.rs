//! Поиск пути по тайлам navmesh: несколько алгоритмов из крейта
//! `pathfinding`, переключаемых на лету (кнопка в дебаг-панели).

use bevy::math::{IVec2, Vec2};
use bevy::prelude::*;
use pathfinding::directed::{astar::astar, bfs::bfs, dijkstra::dijkstra, fringe::fringe};

use crate::navigation::navmesh::{COST_MULTIPLIER, Navmesh};

/// Активный алгоритм поиска пути. IDA*/IDDFS из крейта не включены:
/// на открытых сетках такого размера они практически не завершаются.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub enum PathfindingAlgorithm {
    #[default]
    Astar,
    Dijkstra,
    Fringe,
    Bfs,
}

impl PathfindingAlgorithm {
    /// Следующий по циклу — для кнопки-переключателя.
    pub fn next(self) -> Self {
        match self {
            Self::Astar => Self::Dijkstra,
            Self::Dijkstra => Self::Fringe,
            Self::Fringe => Self::Bfs,
            Self::Bfs => Self::Astar,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Astar => "A*",
            Self::Dijkstra => "Dijkstra",
            Self::Fringe => "Fringe",
            Self::Bfs => "BFS",
        }
    }
}

/// Поиск пути выбранным алгоритмом. Возвращает путь, включая стартовый тайл,
/// либо `None`, если цель непроходима или недостижима.
pub fn find_path(
    navmesh: &Navmesh,
    start: IVec2,
    end: IVec2,
    algorithm: PathfindingAlgorithm,
) -> Option<Vec<IVec2>> {
    if !navmesh.is_passable(end.x, end.y) {
        return None;
    }

    let successors = |&IVec2 { x, y }: &IVec2| navmesh.successors(x, y);
    let heuristic = |&pos: &IVec2| {
        let length = (Vec2::new(pos.x as f32, pos.y as f32)
            - Vec2::new(end.x as f32, end.y as f32))
        .length();
        (length * COST_MULTIPLIER) as i32
    };
    let success = |pos: &IVec2| *pos == end;

    match algorithm {
        PathfindingAlgorithm::Astar => {
            astar(&start, successors, heuristic, success).map(|(path, _cost)| path)
        }
        PathfindingAlgorithm::Dijkstra => {
            dijkstra(&start, successors, success).map(|(path, _cost)| path)
        }
        PathfindingAlgorithm::Fringe => {
            fringe(&start, successors, heuristic, success).map(|(path, _cost)| path)
        }
        // BFS не взвешен: диагональ «стоит» как прямой шаг, путь чуть более
        // ступенчатый — зато нагляден как крайний случай.
        PathfindingAlgorithm::Bfs => bfs(
            &start,
            |&pos| {
                navmesh
                    .successors(pos.x, pos.y)
                    .into_iter()
                    .map(|(tile, _cost)| tile)
            },
            success,
        ),
    }
}
