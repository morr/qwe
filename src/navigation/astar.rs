//! Поиск пути по тайлам navmesh: несколько алгоритмов из крейта
//! `pathfinding`, переключаемых на лету (кнопка в дебаг-панели).

use bevy::math::{IVec2, Vec2};
use bevy::prelude::*;
use pathfinding::directed::{astar::astar, bfs::bfs, dijkstra::dijkstra, fringe::fringe};

use crate::navigation::navmesh::{COST_MULTIPLIER, Navmesh};

/// Активный алгоритм поиска пути: четыре из крейта `pathfinding` плюс
/// иерархические HPA*/Theta* из `bevy_northstar` (см. `northstar.rs`).
/// IDA*/IDDFS не включены: на открытых сетках такого размера они
/// практически не завершаются.
///
/// По умолчанию HPA*: на замере `examples/pathfinding_bench.rs` (1000
/// одинаковых задач) он в 28 раз дешевле плоского A* по CPU (1.3 мс против
/// 36.4) и держит худший случай в 15 мс против 450 — ценой пути длиннее на
/// ~10%.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub enum PathfindingAlgorithm {
    Astar,
    Dijkstra,
    Fringe,
    Bfs,
    #[default]
    Hpa,
    ThetaStar,
}

impl PathfindingAlgorithm {
    /// Следующий по циклу — для кнопки-переключателя.
    pub fn next(self) -> Self {
        match self {
            Self::Astar => Self::Dijkstra,
            Self::Dijkstra => Self::Fringe,
            Self::Fringe => Self::Bfs,
            Self::Bfs => Self::Hpa,
            Self::Hpa => Self::ThetaStar,
            Self::ThetaStar => Self::Astar,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Astar => "A*",
            Self::Dijkstra => "Dijkstra",
            Self::Fringe => "Fringe",
            Self::Bfs => "BFS",
            Self::Hpa => "HPA*",
            Self::ThetaStar => "Theta*",
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
        // иерархические алгоритмы идут не через navmesh, а через
        // `NorthstarGrid` — маршрутизация в `Movable::to_pathfinding`
        PathfindingAlgorithm::Hpa | PathfindingAlgorithm::ThetaStar => None,
    }
}
