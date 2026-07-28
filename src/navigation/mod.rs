mod astar;
mod navmesh;

use bevy::prelude::*;

pub use self::astar::astar_pathfinding;
pub use self::navmesh::{ArcNavmesh, COST_DIAGONAL, COST_MULTIPLIER, COST_STRAIGHT, Navmesh};
use crate::grid::{tile_center, world_to_tile};
use crate::loading::{AppState, WorldInitSet};
use crate::map::osm::MapData;
use crate::portal::PortalPos;

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

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArcNavmesh>().add_systems(
            OnEnter(AppState::Playing),
            (fill_navmesh, snap_portal, prune_unreachable)
                .chain()
                .in_set(WorldInitSet::Navmesh),
        );
    }
}

/// Растеризация загруженной OSM-карты в navmesh.
fn fill_navmesh(map: Res<MapData>, arc_navmesh: Res<ArcNavmesh>) {
    let started = std::time::Instant::now();
    let mut navmesh = arc_navmesh.write();
    navmesh.fill_from_mapdata(&map);
    info!("navmesh filled in {:?}", started.elapsed());
}

/// Карманы, недостижимые от портала, выключаются из navmesh — там никто
/// не спавнится и туда не прокладываются пути.
fn prune_unreachable(arc_navmesh: Res<ArcNavmesh>, portal: Res<PortalPos>) {
    let started = std::time::Instant::now();
    let mut navmesh = arc_navmesh.write();
    let pruned = navmesh.prune_unreachable(world_to_tile(portal.0));
    info!(
        "navmesh: pruned {pruned} unreachable tiles in {:?}",
        started.elapsed()
    );
}

/// Радиус, в котором вокруг кандидата на портал всё должно быть проходимо
/// (портал 9 м + спавн демонов по кромке), тайлы.
const PORTAL_CLEARANCE_TILES: i32 = 4;
/// Предел спирального поиска места для портала, тайлы.
const PORTAL_SEARCH_TILES: i32 = 200;

/// Хинт `PORTAL_POS` мог попасть в здание OSM-карты: снап к ближайшему
/// тайлу, вокруг которого достаточно свободного места.
fn snap_portal(arc_navmesh: Res<ArcNavmesh>, mut portal: ResMut<PortalPos>) {
    let navmesh = arc_navmesh.read();
    let start = world_to_tile(portal.0);

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
                    let position = tile_center(tile);
                    if position != portal.0 {
                        info!("portal snapped {:?} => {position:?}", portal.0);
                        portal.0 = position;
                    }
                    return;
                }
            }
        }
    }
    warn!("no clear spot for portal near {:?}", portal.0);
}
