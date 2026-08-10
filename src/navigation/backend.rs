//! Активный бэкенд навигации одним значением — снимок, который можно унести
//! в async-таск поиска или заморозить на весь прогон.

use std::sync::{Arc, RwLock};

use bevy::prelude::*;

use super::{
    Navmesh, PathfindingAlgorithm, PathfindingResult, PolymeshBuild, find_path,
    find_path_northstar, find_path_polymesh,
};
use crate::grid::tile_center;

/// Снимок активного бэкенда навигации: сеточный navmesh с выбранным
/// алгоритмом (и иерархией northstar, когда она построена) — либо
/// полигональный меш, который перекрывает всё сеточное, когда включён и готов.
///
/// Одно значение вместо россыпи ресурсов: потребители симуляции не ветвятся
/// «какой бэкенд включён» — они спрашивают путь, а выбор сделан здесь один
/// раз, при снятии снимка ([`Pathfinder::backend`](super::Pathfinder::backend)).
/// Клон дешёвый (одни `Arc`), значение `Send`: его уносят async-таски поиска,
/// а детерминированный режим замораживает на прогон
/// (`determinism::DeterministicRun`).
#[derive(Clone)]
pub struct Backend {
    /// Тот же `Arc`, что в `ArcNavmesh`: заливка нового города сбрасывает
    /// сетку на месте, поэтому снимок не отстаёт от мира.
    navmesh: Arc<RwLock<Navmesh>>,
    algorithm: PathfindingAlgorithm,
    northstar: Option<Arc<bevy_northstar::prelude::OrdinalGrid>>,
    mesh: Option<Arc<PolymeshBuild>>,
}

/// Пустой мир: сетка по умолчанию, плоский алгоритм, ни иерархии, ни меша.
/// Нужен только как заглушка `DeterministicRun` до первой заморозки — сам
/// детерминированный конвейер раньше `OnEnter(Live)` не работает.
impl Default for Backend {
    fn default() -> Self {
        Self {
            navmesh: Arc::default(),
            algorithm: PathfindingAlgorithm::default(),
            northstar: None,
            mesh: None,
        }
    }
}

impl Backend {
    pub(super) fn new(
        navmesh: Arc<RwLock<Navmesh>>,
        algorithm: PathfindingAlgorithm,
        northstar: Option<Arc<bevy_northstar::prelude::OrdinalGrid>>,
        mesh: Option<Arc<PolymeshBuild>>,
    ) -> Self {
        Self {
            navmesh,
            algorithm,
            northstar,
            mesh,
        }
    }

    /// Полигональный меш снимка, если бэкенд — он. Переходный доступ для
    /// проверки проходимости под спасением застрявших (`movement::Walkable`).
    pub fn mesh(&self) -> Option<&Arc<PolymeshBuild>> {
        self.mesh.as_ref()
    }

    /// Один поиск пути активным бэкендом — общее тело обоих диспетчеров,
    /// чтобы режимы не разъехались в том, ЧТО именно считается.
    pub fn search(
        &self,
        start_world: Vec2,
        start_tile: IVec2,
        end_tile: IVec2,
    ) -> PathfindingResult {
        let (path, started_at) = match &self.mesh {
            Some(mesh) => {
                let started_at = std::time::Instant::now();
                // цель осталась тайловой (её выбрало поведение по
                // проходимости сетки) — на меше это её центр
                let path = find_path_polymesh(mesh, start_world, tile_center(end_tile));
                (path, started_at)
            }
            None => {
                let (tiles, started_at) = self.grid_path(start_tile, end_tile);
                let path =
                    tiles.map(|tiles| tiles.into_iter().map(tile_center).collect::<Vec<Vec2>>());
                (path, started_at)
            }
        };
        PathfindingResult {
            end_tile,
            path,
            duration: started_at.elapsed(),
        }
    }

    /// Сеточный поиск: иерархия northstar, если она построена, иначе плоский
    /// алгоритм. Возвращает путь в тайлах и момент старта самого поиска —
    /// метрика не должна включать ожидание `RwLock`.
    fn grid_path(
        &self,
        start_tile: IVec2,
        end_tile: IVec2,
    ) -> (Option<Vec<IVec2>>, std::time::Instant) {
        let hierarchical = self.algorithm.needs_northstar();
        if let Some(grid) = self.northstar.as_deref().filter(|_| hierarchical) {
            let started_at = std::time::Instant::now();
            let path = find_path_northstar(
                grid,
                start_tile,
                end_tile,
                self.algorithm == PathfindingAlgorithm::ThetaStar,
            );
            return (path, started_at);
        }
        // сетка northstar ещё строится — до её готовности иерархические
        // алгоритмы обслуживает A*
        let algorithm = if hierarchical {
            PathfindingAlgorithm::Astar
        } else {
            self.algorithm
        };
        let navmesh = self.navmesh.read().unwrap();
        // после захвата лока: метрика — сам поиск, без RwLock
        let started_at = std::time::Instant::now();
        (
            find_path(&navmesh, start_tile, end_tile, algorithm),
            started_at,
        )
    }
}
