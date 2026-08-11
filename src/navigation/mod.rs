mod astar;
mod backend;
mod navmesh;
mod northstar;
#[cfg(test)]
mod parity_tests;
mod polymesh;

use bevy::prelude::*;

pub use self::astar::{PathfindingAlgorithm, find_path};
pub use self::backend::{Backend, Walkable};
pub use self::navmesh::{ArcNavmesh, COST_DIAGONAL, COST_MULTIPLIER, COST_STRAIGHT, Navmesh};
pub use self::northstar::{
    NorthstarGrid, build_from_navmesh, find_path_northstar, northstar_wanted, poll_northstar_build,
    start_northstar_build,
};
pub use self::polymesh::{
    PolyNavmesh, PolymeshBuild, PolymeshDebug, build_polymesh_from_map, find_path_polymesh,
    poll_polymesh_build, sync_polymesh_build,
};
use crate::grid::{tile_center, world_to_tile};
use crate::loading::{AppState, PlayPhase, WorldInitSet};
use crate::map::osm::model::MapData;
use crate::settings::NavtileBase;

/// Ответ асинхронного поиска пути (снимается в
/// `movement::listen_for_pathfinding_tasks`).
#[derive(Debug)]
pub struct PathfindingResult {
    /// Waypoint'ы в мировых метрах, включая стартовую точку. Сеточные
    /// алгоритмы отдают тайлы, и бэкенд переводит их `tile_center`;
    /// полигональный меш отдаёт мировые точки сразу.
    pub path: Option<Vec<Vec2>>,
    pub end_tile: IVec2,
    /// Длительность самого поиска (без ожидания RwLock) — для диагностики.
    pub duration: std::time::Duration,
}

/// Проходимый тайл в точке или среди 8 соседей — иначе `None`. Просеивание
/// цели: точка, выбранная поведением (вершина контура дома, вход, случайная
/// точка блуждания), лежит на препятствии чаще, чем нет, а нужен от неё лишь
/// соседний свободный тайл.
pub fn find_passable_tile_near(navmesh: &Navmesh, tile: IVec2) -> Option<IVec2> {
    nearest_passable_tile(navmesh, tile, 1)
}

/// Ближайший (по евклидову расстоянию) проходимый тайл в пределах
/// `max_radius` тайлов — кольцевым поиском от `tile` наружу.
///
/// Кольца чебышёвские, а ответ евклидов, поэтому кольцо, в котором нашёлся
/// первый кандидат, не последнее: у угла кольца `r` расстояние `r·√2`, а в
/// кольце `r + 1` может стоять сосед по прямой на `r + 1`, что ближе уже при
/// `r ≥ 3`. Поиск идёт, пока лучшее найденное дальше внутренней границы
/// следующего кольца.
pub fn nearest_passable_tile(navmesh: &Navmesh, tile: IVec2, max_radius: i32) -> Option<IVec2> {
    nearest_tile_where(tile, max_radius, |candidate| {
        navmesh.is_passable(candidate.x, candidate.y)
    })
}

/// То же кольцевым поиском, но «свободен» решает вызывающий: спасение
/// застрявших меряет тайл активным бэкендом, и на полигональном меше
/// проходимости сетки мало (контуры раздуты на радиус агента).
pub fn nearest_tile_where(
    tile: IVec2,
    max_radius: i32,
    is_free: impl Fn(IVec2) -> bool,
) -> Option<IVec2> {
    if is_free(tile) {
        return Some(tile);
    }
    let mut best: Option<(i32, IVec2)> = None;
    for radius in 1..=max_radius {
        // ближе, чем `radius`, в этом и во всех следующих кольцах уже не будет
        if let Some((distance, found)) = best
            && distance <= radius * radius
        {
            return Some(found);
        }
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // только само кольцо: его внутренность просмотрена раньше
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate = tile + IVec2::new(dx, dy);
                if !is_free(candidate) {
                    continue;
                }
                let distance = dx * dx + dy * dy;
                if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                    best = Some((distance, candidate));
                }
            }
        }
    }
    best.map(|(_, found)| found)
}

/// Есть ли прямая проходимая линия между двумя мировыми точками. Нужна там,
/// где сущность идёт напрямую, минуя тайловый путь (бросок демона), — иначе
/// «напрямик» означало бы сквозь здание.
///
/// Шаг сэмплирования — четверть тайла. Полностью «супернакрытие» не считаем:
/// пропустить можно только срез угла короче четверти тайла, а стоит это
/// вчетверо дешевле.
pub fn line_of_sight(navmesh: &Navmesh, from: Vec2, to: Vec2) -> bool {
    let delta = to - from;
    let steps = (delta.length() / (navmesh.tile_size / 4.0)).ceil() as i32;
    (0..=steps).all(|step| {
        let point = from + delta * (step as f32 / steps.max(1) as f32);
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    })
}

/// Всё нужное для запуска поиска пути одним system-параметром: сеточный
/// navmesh, иерархическая сетка, выбранный алгоритм — и полигональный меш с
/// его тумблером, который перекрывает всё перечисленное, когда готов.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Pathfinder<'w> {
    pub navmesh: Res<'w, ArcNavmesh>,
    pub northstar: Res<'w, NorthstarGrid>,
    pub algorithm: Res<'w, PathfindingAlgorithm>,
    pub poly: Res<'w, PolyNavmesh>,
    pub polymesh: Res<'w, PolymeshDebug>,
}

impl Pathfinder<'_> {
    /// Полигональный меш, если панель включена и он уже построен. `None`
    /// означает «ищем по сетке» — и пока панель выключена, и пока постройка
    /// идёт (5–20 с): тот же приём, которым HPA* до готовности
    /// `NorthstarGrid` обслуживается A*.
    pub fn polymesh_build(&self) -> Option<std::sync::Arc<PolymeshBuild>> {
        self.polymesh.enabled.then(|| self.poly.build()).flatten()
    }

    /// Живой снимок активного бэкенда — то, чем симуляция ищет пути и меряет
    /// проходимость, не зная имён ресурсов за ним. Детерминированный режим
    /// снимает его один раз на прогон (`determinism::DeterministicRun`).
    pub fn backend(&self) -> Backend {
        Backend::new(
            self.navmesh.0.clone(),
            *self.algorithm,
            self.northstar.get(),
            self.polymesh_build(),
        )
    }
}

/// «Выбранный бэкенд ещё строится» — один system-параметр вместо четырёх
/// ресурсов в сигнатуре прогрева (`loading.rs::poll_warmup`).
#[derive(bevy::ecs::system::SystemParam)]
pub struct NavigationBuildPending<'w> {
    pub northstar: Res<'w, NorthstarGrid>,
    pub algorithm: Res<'w, PathfindingAlgorithm>,
    pub poly: Res<'w, PolyNavmesh>,
    pub polymesh: Res<'w, PolymeshDebug>,
}

impl NavigationBuildPending<'_> {
    /// Ждать ли ещё. Спрашивается только про **выбранный** бэкенд: сетка
    /// northstar не нужна ни при включённой панели Polymesh, ни плоскому A*,
    /// и ждать её в этих случаях значило бы держать экран загрузки зря.
    pub fn is_building(&self) -> bool {
        if self.polymesh.enabled {
            return self.poly.build().is_none();
        }
        self.algorithm.needs_northstar() && self.northstar.get().is_none()
    }
}

/// «Пути активной навигации — метрические полилинии, а не центры тайлов» —
/// свойство, которым гейтится расталкивание (`movement::separation_runs`):
/// боковой толчок имеет смысл, только когда его не откатит следующий тайловый
/// waypoint.
///
/// Отвечает **тумблер** полигонального меша, а не готовность постройки: пока
/// меш строится (0.3–20 с), запросы обслуживает сетка, но мигать
/// расталкиванием на этом переходе хуже, чем доработать полсекунды по-старому.
///
/// `Option` — потребители живут в `MovementPlugin`, который используется в
/// тестах и демо-сценах без плагина навигации: нет ресурса — пространство
/// тайловое.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ContinuousSpace<'w> {
    polymesh: Option<Res<'w, PolymeshDebug>>,
}

impl ContinuousSpace<'_> {
    pub fn is_continuous(&self) -> bool {
        self.polymesh
            .as_ref()
            .is_some_and(|polymesh| polymesh.enabled)
    }
}

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArcNavmesh>()
            .register_type::<PathfindingAlgorithm>()
            .init_resource::<PathfindingAlgorithm>()
            .register_type::<NavtileBase>()
            .init_resource::<NavtileBase>()
            .init_resource::<NorthstarGrid>()
            // navmesh заполняется и прореживается фоновым потоком загрузки
            // (`map/osm/download.rs`) — здесь остаётся только иерархия,
            // которая строится по уже финальной проходимости
            // не раньше конца прогрева: постройка занимает rayon'ом все ядра,
            // и A*, который в это время развозит пешек в кадре, замедляется
            // вдвое (85 мс на поиск против 36 мс в бенче).
            //
            // И только если иерархия выбрана: строится ровно тот бэкенд, по
            // которому ходят (`northstar_wanted`). Переключение алгоритма или
            // возврат с polymesh на сетку запускает постройку тогда же — тем
            // же условием в `Update`, как ленивая постройка меша
            .add_systems(
                OnEnter(PlayPhase::Live),
                start_northstar_build
                    .run_if(northstar_wanted)
                    .run_if(not(crate::determinism::deterministic)),
            )
            // в детерминированном режиме — наоборот, на входе в ПРОГРЕВ:
            // прогон обязан целиком пройти на одном бэкенде, а достроившаяся
            // посреди него иерархия поменяла бы пути на полпути. Довод про
            // отбираемые ядра здесь не работает: в этом режиме прогрев ничего
            // не считает (`FixedUpdate` стоит на паузе), и ждать сборку —
            // ровно его работа (см. `loading.rs::poll_warmup`)
            .add_systems(
                OnEnter(PlayPhase::Warmup),
                start_northstar_build
                    .run_if(northstar_wanted)
                    .run_if(crate::determinism::deterministic),
            )
            .add_systems(
                Update,
                (
                    start_northstar_build
                        .run_if(in_state(PlayPhase::Live))
                        .run_if(northstar_wanted)
                        .run_if(
                            resource_changed::<PathfindingAlgorithm>
                                .or_else(resource_changed::<PolymeshDebug>),
                        ),
                    poll_northstar_build,
                ),
            )
            // полигональный меш — основной бэкенд (`PolymeshDebug::enabled`
            // включён по умолчанию), но постройка всё равно асинхронна и
            // условна: выключенная панель не строит ничего, и до готовности
            // меша запросы обслуживает сетка
            .register_type::<PolymeshDebug>()
            .init_resource::<PolymeshDebug>()
            .init_resource::<PolyNavmesh>()
            // восстановленный из настроек enabled: перестройка на входе в
            // мир, когда MapData нового города уже вставлена
            .add_systems(
                OnEnter(AppState::Playing),
                sync_polymesh_build
                    .run_if(|debug: Res<PolymeshDebug>| debug.enabled)
                    .in_set(WorldInitSet::Spawn),
            )
            // смена города: оба бэкенда описывают геометрию старого мира
            // (иерархия повела бы прогрев новой карты путями сквозь дома,
            // летящая постройка меша — тем более), чистка здесь, у владельца,
            // а не в `city.rs`; `ArcNavmesh` не в счёт — его перезаливает сам
            // поток загрузки
            .add_systems(OnExit(AppState::Playing), clear_map_backends)
            .add_systems(
                Update,
                (
                    // resource_changed без resource_exists паникует до
                    // загрузки карты (BRP может дёрнуть тумблер в Loading)
                    sync_polymesh_build.run_if(
                        resource_exists::<MapData>
                            .and_then(resource_changed::<PolymeshDebug>)
                            .and_then(in_state(AppState::Playing)),
                    ),
                    poll_polymesh_build.run_if(|poly: Res<PolyNavmesh>| poly.is_building()),
                ),
            );
    }
}

fn clear_map_backends(mut northstar: ResMut<NorthstarGrid>, mut polymesh: ResMut<PolyNavmesh>) {
    northstar.clear();
    polymesh.clear();
}

/// Предел спирального поиска места для портала, метры мира.
const PORTAL_SEARCH_METERS: f32 = 400.0;

/// Ближайший к `position` центр тайла, вокруг которого хватает свободного
/// места для портала. Хинт `PORTAL_POS` мог попасть в здание OSM-карты;
/// снап делает поток загрузки (`map/osm/download.rs`) сразу после заливки
/// navmesh, той же функцией пользуется офлайн-бенч
/// (`examples/bench/pathfinding_bench.rs`), чтобы navmesh совпал с игровым.
pub fn snap_portal_position(navmesh: &Navmesh, position: Vec2) -> Option<Vec2> {
    let start = world_to_tile(position);

    // радиус, в котором вокруг кандидата всё должно быть проходимо
    // (диаметр портала + спавн демонов по кромке), тайлы
    let clearance = (crate::settings::PORTAL_DIAMETER / 2.0 / navmesh.tile_size) as i32 + 1;
    let is_clear = |tile: IVec2| {
        (-clearance..=clearance).all(|dx| {
            (-clearance..=clearance).all(|dy| navmesh.is_passable(tile.x + dx, tile.y + dy))
        })
    };

    let search_tiles = (PORTAL_SEARCH_METERS / navmesh.tile_size) as i32;
    for radius in 0..=search_tiles {
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let tile = start + IVec2::new(dx, dy);
                if is_clear(tile) {
                    return Some(tile_center(tile));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Пустая сетка с одним свободным тайлом — так проверяется, какой именно
    /// тайл выберет кольцевой поиск.
    fn navmesh_with_only(free: &[IVec2]) -> Navmesh {
        let mut navmesh = Navmesh::default();
        for x in 0..navmesh.grid_size.x {
            for y in 0..navmesh.grid_size.y {
                navmesh.set_passable(x, y, false);
            }
        }
        for tile in free {
            navmesh.set_passable(tile.x, tile.y, true);
        }
        navmesh
    }

    #[test]
    fn the_tile_itself_wins_when_it_is_passable() {
        let navmesh = navmesh_with_only(&[IVec2::new(10, 10), IVec2::new(11, 10)]);
        assert_eq!(
            nearest_passable_tile(&navmesh, IVec2::new(10, 10), 4),
            Some(IVec2::new(10, 10))
        );
    }

    /// Кольца чебышёвские, ответ евклидов: угол кольца 3 (расстояние 4.24)
    /// обязан проиграть прямому соседу из кольца 4 (расстояние 4.0).
    #[test]
    fn a_straight_neighbour_of_the_next_ring_beats_a_corner_of_this_one() {
        let corner = IVec2::new(13, 13);
        let straight = IVec2::new(14, 10);
        let navmesh = navmesh_with_only(&[corner, straight]);
        assert_eq!(
            nearest_passable_tile(&navmesh, IVec2::new(10, 10), 8),
            Some(straight)
        );
    }

    #[test]
    fn nothing_within_the_radius_means_none() {
        let navmesh = navmesh_with_only(&[IVec2::new(30, 10)]);
        assert_eq!(nearest_passable_tile(&navmesh, IVec2::new(10, 10), 8), None);
    }
}
