//! Активный бэкенд навигации одним значением — снимок, который можно унести
//! в async-таск поиска или заморозить на весь прогон.

use std::sync::{Arc, RwLock, RwLockReadGuard};

use bevy::prelude::*;

use super::{
    Navmesh, PathfindingAlgorithm, PathfindingResult, PolymeshBuild, find_passable_tile_near,
    find_path, find_path_northstar, find_path_polymesh, line_of_sight, nearest_tile_where,
};
use crate::grid::{tile_center, world_to_tile};
use crate::settings::RESCUE_SEARCH_TILES;

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

    /// Взгляд на проходимость снимка. Read-лок сетки берётся здесь один раз
    /// — вызывающая система держит значение весь свой прогон (или создаёт
    /// лениво, если нужен не всегда), а не платит за лок на каждый вызов.
    pub fn walkable(&self) -> Walkable<'_> {
        Walkable {
            navmesh: self.navmesh.read().unwrap(),
            mesh: self.mesh.as_deref(),
        }
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

/// Чем меряется «свободно» — тем же бэкендом, по которому пешки ходят.
/// Полигональный меш строже сетки: его контуры раздуты на радиус агента, и
/// свободный тайл вплотную к стене на меше уже внутри препятствия. Пока меш
/// строится или выключен, остаётся одна сетка.
pub struct Walkable<'a> {
    navmesh: RwLockReadGuard<'a, Navmesh>,
    mesh: Option<&'a PolymeshBuild>,
}

impl Walkable<'_> {
    pub fn allows(&self, point: Vec2) -> bool {
        let tile = world_to_tile(point);
        // сетка первой: индекс в `Vec` против запроса в BVH
        self.navmesh.is_passable(tile.x, tile.y)
            && self.mesh.is_none_or(|mesh| mesh.contains(point))
    }

    /// Есть ли прямая проходимая линия между двумя мировыми точками — для
    /// сущности, идущей напрямую, минуя путь (бросок демона, фильтр смены
    /// цели в погоне).
    ///
    /// Осознанно по сетке даже при меш-бэкенде: проверка стоит в горячих
    /// циклах (решающие тики демонов перебирают кандидатов), и индекс в `Vec`
    /// там уместнее запроса в BVH. Перевод на меш-строгий тест — одна правка
    /// здесь, а не четырёх систем поведения.
    pub fn line_of_sight(&self, from: Vec2, to: Vec2) -> bool {
        line_of_sight(&self.navmesh, from, to)
    }

    /// Просеивание цели: проходимый тайл в точке или среди её 8 соседей —
    /// иначе `None`. Точка, выбранная поведением (вершина контура дома, вход,
    /// случайная точка блуждания), лежит на препятствии чаще, чем нет, а
    /// нужен от неё лишь соседний свободный тайл.
    ///
    /// Тоже осознанно по сетке: цель в обоих бэкендах остаётся тайлом
    /// (identity фильтра устаревших ответов и прихода), посадку старта в меш
    /// даёт метровый допуск полимеш-поиска, а редкий промах цели мимо меша
    /// выражается провалом поиска и ловится штатно.
    pub fn sift_target(&self, tile: IVec2) -> Option<IVec2> {
        find_passable_tile_near(&self.navmesh, tile)
    }

    /// Точка доката: продолжать ли катиться за концом пути.
    ///
    /// Осознанно по сетке даже при меш-бэкенде: проверка стоит в цикле ходока
    /// по всем движущимся каждый тик, а докат в раздутое радиусом агента
    /// пространство — штатный вход спасения (`rescue_from_impassable`), как и
    /// прочие прямые сдвиги `SimPosition`.
    pub fn coast_allows(&self, point: Vec2) -> bool {
        let tile = world_to_tile(point);
        self.navmesh.is_passable(tile.x, tile.y)
    }

    /// Куда переставить застрявшего. Сперва — снап меша (метровый допуск): в
    /// инфляцию контура пешка попадает сантиметрами, и перебирать за неё тайлы
    /// с запросом в BVH на каждый незачем. Не помог — значит она не у стены, а
    /// внутри дома, и тогда работает кольцевой поиск по тайлам, тот же, что и
    /// на голой сетке.
    pub fn nearest_free_point(&self, point: Vec2) -> Option<Vec2> {
        self.mesh
            .and_then(|mesh| mesh.nearest_free_point(point))
            .filter(|&snapped| self.allows(snapped))
            .or_else(|| {
                nearest_tile_where(world_to_tile(point), RESCUE_SEARCH_TILES, |candidate| {
                    self.allows(tile_center(candidate))
                })
                .map(tile_center)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сеточный снимок с перечисленными заблокированными тайлами — интерфейс
    /// тестируется тем же путём, каким его берут потребители: значение, а не
    /// ресурсы.
    fn grid_backend(blocked: &[IVec2]) -> Backend {
        let mut navmesh = Navmesh::default();
        for tile in blocked {
            navmesh.set_passable(tile.x, tile.y, false);
        }
        Backend::new(
            Arc::new(RwLock::new(navmesh)),
            PathfindingAlgorithm::Astar,
            None,
            None,
        )
    }

    /// Контракт пути един для обоих бэкендов: мировые точки, стартовая
    /// включена (приёмка срезает её сама).
    #[test]
    fn a_grid_search_returns_world_waypoints_including_the_start() {
        let backend = grid_backend(&[]);
        let start = IVec2::new(10, 10);
        let goal = IVec2::new(13, 10);
        let result = backend.search(tile_center(start), start, goal);
        let path = result.path.expect("прямой путь по пустой сетке");
        assert_eq!(path.first().copied(), Some(tile_center(start)));
        assert_eq!(path.last().copied(), Some(tile_center(goal)));
        assert_eq!(result.end_tile, goal);
    }

    #[test]
    fn a_search_to_an_impassable_goal_fails() {
        let goal = IVec2::new(13, 10);
        let backend = grid_backend(&[goal]);
        let start = IVec2::new(10, 10);
        let result = backend.search(tile_center(start), start, goal);
        assert!(result.path.is_none());
    }

    /// Без меша строгая проверка совпадает с сеточной — и обе видят
    /// заблокированный тайл.
    #[test]
    fn the_walkable_view_answers_by_the_grid_when_there_is_no_mesh() {
        let blocked = IVec2::new(11, 10);
        let backend = grid_backend(&[blocked]);
        let walkable = backend.walkable();
        assert!(!walkable.allows(tile_center(blocked)));
        assert!(!walkable.coast_allows(tile_center(blocked)));
        assert!(walkable.allows(tile_center(IVec2::new(10, 10))));
    }

    /// Просеивание отдаёт сам тайл, когда он проходим, и соседа — когда нет.
    #[test]
    fn sift_target_shifts_a_blocked_tile_to_a_free_neighbour() {
        let blocked = IVec2::new(11, 10);
        let backend = grid_backend(&[blocked]);
        let walkable = backend.walkable();
        assert_eq!(
            walkable.sift_target(IVec2::new(10, 10)),
            Some(IVec2::new(10, 10))
        );
        let shifted = walkable.sift_target(blocked).expect("сосед свободен");
        assert_ne!(shifted, blocked);
        assert!((shifted - blocked).abs().max_element() <= 1);
    }

    #[test]
    fn line_of_sight_is_blocked_by_a_tile_between_the_points() {
        let backend = grid_backend(&[IVec2::new(11, 10)]);
        let walkable = backend.walkable();
        let from = tile_center(IVec2::new(10, 10));
        let to = tile_center(IVec2::new(12, 10));
        assert!(!walkable.line_of_sight(from, to));
        assert!(walkable.line_of_sight(from, from));
    }
}
