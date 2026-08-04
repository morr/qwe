//! Поиск пути по тайлам navmesh: несколько алгоритмов из крейта
//! `pathfinding`, переключаемых на лету (кнопка в дебаг-панели).

use bevy::math::{IVec2, Vec2};
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use pathfinding::directed::{astar::astar, bfs::bfs, dijkstra::dijkstra, fringe::fringe};

use crate::navigation::navmesh::{COST_MULTIPLIER, Navmesh};

/// Активный алгоритм поиска пути: четыре из крейта `pathfinding` плюс
/// иерархические HPA*/Theta* из `bevy_northstar` (см. `northstar.rs`).
/// IDA*/IDDFS не включены: на открытых сетках такого размера они
/// практически не завершаются.
///
/// Замеры у вариантов — из `examples/pathfinding_bench.rs`: 1000 одинаковых
/// задач (сид `0xC0FFEE`, 80% маршрутов через город к случайному зданию),
/// 10 потоков, dev-профиль; путь — средняя длина найденного маршрута.
/// Пересняты 2026-07-28, воспроизводятся командой
/// `cargo run --example pathfinding_bench -- 1000`.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "navigation", key = "algorithm")]
pub enum PathfindingAlgorithm {
    /// avg 36.4 мс, p50 20.9, p95 128.6, max 450, путь 1260 м.
    /// Базовая линия: лучший из плоских, эвристика режет фронт вчетверо
    /// против Dijkstra при том же оптимальном пути.
    Astar,
    /// avg 220.0 мс, p50 207.6, p95 538.2, max 836, путь 1260 м.
    /// В 6 раз дороже A*: без эвристики выгребает весь достижимый регион.
    Dijkstra,
    /// avg 327.8 мс, p50 142.2, p95 1282, max 4635, путь 1260 м.
    /// Худший здесь: пороговые итерации переобходят фронт на открытых
    /// пространствах карты. Медиана вдвое лучше среднего — всё в хвосте.
    Fringe,
    /// avg 152.9 мс, p50 141.9, p95 378.7, max 547, путь 1280 м.
    /// Дешевле Dijkstra (нет очереди с приоритетом), но не взвешен:
    /// диагональ «стоит» как прямой шаг, путь на 1.5% длиннее и ступенчатее.
    Bfs,
    /// avg 1.30 мс, p50 0.89, p95 4.02, max 14.9, путь 1393 м. **Дефолт**:
    /// в 28 раз дешевле A* по CPU, худший случай 15 мс против 450 — ценой
    /// пути длиннее на 10%. Разовая цена — постройка сетки ~12 с; она идёт
    /// фоновым таском, и пока не готова, запросы обслуживает A*.
    #[default]
    Hpa,
    /// avg 122.3 мс, p50 1.63, p95 246.8, max 10485, путь 1311 м.
    /// Лучшая медиана в таблице и катастрофический хвост: когда трассировка
    /// прямой видимости не проходит, any-angle сваливается в полный перебор.
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

    /// Нужна ли алгоритму иерархия northstar. Плоские четыре обходятся сеткой
    /// проходимости, и строить под них `OrdinalGrid` — двенадцать секунд всех
    /// ядер впустую (`northstar::northstar_wanted`).
    pub fn needs_northstar(self) -> bool {
        matches!(self, Self::Hpa | Self::ThetaStar)
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
