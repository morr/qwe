mod astar;
mod navmesh;
mod northstar;

use bevy::prelude::*;

pub use self::astar::{PathfindingAlgorithm, find_path};
pub use self::navmesh::{ArcNavmesh, COST_DIAGONAL, COST_MULTIPLIER, COST_STRAIGHT, Navmesh};
pub use self::northstar::{
    NorthstarGrid, build_from_navmesh, find_path_northstar, poll_northstar_build,
    start_northstar_build,
};
use crate::grid::{tile_center, world_to_tile};
use crate::loading::{AppState, WorldInitSet};
use crate::settings::NAVTILE_SIZE;

/// Ответ асинхронного поиска пути (снимается в
/// `movement::listen_for_pathfinding_tasks`).
#[derive(Debug)]
pub struct PathfindingResult {
    pub path: Option<Vec<IVec2>>,
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

/// Шаг сэмплирования луча видимости — четверть тайла. Полностью «супернакрытие»
/// не считаем: пропустить можно только срез угла короче полуметра, а стоит это
/// вчетверо дешевле.
const LINE_OF_SIGHT_STEP: f32 = NAVTILE_SIZE / 4.0;

/// Есть ли прямая проходимая линия между двумя мировыми точками. Нужна там,
/// где сущность идёт напрямую, минуя тайловый путь (бросок демона), — иначе
/// «напрямик» означало бы сквозь здание.
pub fn line_of_sight(navmesh: &Navmesh, from: Vec2, to: Vec2) -> bool {
    let delta = to - from;
    let steps = (delta.length() / LINE_OF_SIGHT_STEP).ceil() as i32;
    (0..=steps).all(|step| {
        let point = from + delta * (step as f32 / steps.max(1) as f32);
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    })
}

/// Всё нужное для запуска поиска пути одним system-параметром:
/// navmesh, иерархическая сетка и выбранный алгоритм.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Pathfinder<'w> {
    pub navmesh: Res<'w, ArcNavmesh>,
    pub northstar: Res<'w, NorthstarGrid>,
    pub algorithm: Res<'w, PathfindingAlgorithm>,
}

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArcNavmesh>()
            .register_type::<PathfindingAlgorithm>()
            .init_resource::<PathfindingAlgorithm>()
            .init_resource::<NorthstarGrid>()
            // navmesh заполняется и прореживается фоновым потоком загрузки
            // (`map/osm/download.rs`) — здесь остаётся только иерархия,
            // которая строится по уже финальной проходимости
            .add_systems(
                OnEnter(AppState::Playing),
                start_northstar_build.in_set(WorldInitSet::Navmesh),
            )
            .add_systems(Update, poll_northstar_build);
    }
}

/// Радиус, в котором вокруг кандидата на портал всё должно быть проходимо
/// (диаметр портала + спавн демонов по кромке), тайлы.
const PORTAL_CLEARANCE_TILES: i32 =
    (crate::settings::PORTAL_DIAMETER / 2.0 / crate::settings::NAVTILE_SIZE) as i32 + 1;
/// Предел спирального поиска места для портала, тайлы.
const PORTAL_SEARCH_TILES: i32 = 200;

/// Ближайший к `position` центр тайла, вокруг которого хватает свободного
/// места для портала. Хинт `PORTAL_POS` мог попасть в здание OSM-карты;
/// снап делает поток загрузки (`map/osm/download.rs`) сразу после заливки
/// navmesh, той же функцией пользуется офлайн-бенч
/// (`examples/pathfinding_bench.rs`), чтобы navmesh совпал с игровым.
pub fn snap_portal_position(navmesh: &Navmesh, position: Vec2) -> Option<Vec2> {
    let start = world_to_tile(position);

    let is_clear = |tile: IVec2| {
        (-PORTAL_CLEARANCE_TILES..=PORTAL_CLEARANCE_TILES).all(|dx| {
            (-PORTAL_CLEARANCE_TILES..=PORTAL_CLEARANCE_TILES)
                .all(|dy| navmesh.is_passable(tile.x + dx, tile.y + dy))
        })
    };

    for radius in 0..=PORTAL_SEARCH_TILES {
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
