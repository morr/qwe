mod astar;
mod navmesh;
mod northstar;
mod polymesh;

use bevy::prelude::*;

pub use self::astar::{PathfindingAlgorithm, find_path};
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
    /// алгоритмы отдают тайлы, и диспетчер переводит их `tile_center`;
    /// полигональный меш отдаёт мировые точки сразу.
    pub path: Option<Vec<Vec2>>,
    pub start_tile: IVec2,
    pub end_tile: IVec2,
    /// Длительность самого поиска (без ожидания RwLock) — для диагностики.
    pub duration: std::time::Duration,
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
                start_northstar_build.run_if(northstar_wanted),
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
            // полигональный меш-прототип: ленив (ничего не строит, пока
            // панель Polymesh не включат) и постройка асинхронна
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

/// Предел спирального поиска места для портала, метры мира.
const PORTAL_SEARCH_METERS: f32 = 400.0;

/// Ближайший к `position` центр тайла, вокруг которого хватает свободного
/// места для портала. Хинт `PORTAL_POS` мог попасть в здание OSM-карты;
/// снап делает поток загрузки (`map/osm/download.rs`) сразу после заливки
/// navmesh, той же функцией пользуется офлайн-бенч
/// (`examples/pathfinding_bench.rs`), чтобы navmesh совпал с игровым.
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
