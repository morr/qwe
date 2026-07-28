//! Иерархический поиск пути из `bevy_northstar` (HPA* / Theta*). Плагин
//! крейта не используется — его `Grid` строится из нашего navmesh один раз
//! после заливки и дёргается напрямую из async-тасков (`Grid: Send + Sync`).

use std::sync::Arc;

use bevy::prelude::*;
use bevy_northstar::prelude::{GridSettingsBuilder, Nav, OrdinalGrid, PathfindArgs};

use crate::navigation::Navmesh;
use crate::settings::GRID_SIZE;

/// Размер чанка иерархии; делит GRID_SIZE нацело (1500 и 1125 кратны 25),
/// иначе northstar округляет с warning'ом.
const CHUNK_SIZE: u32 = 25;

/// Иерархическая сетка northstar; `None`, пока карта не загружена.
#[derive(Resource, Default)]
pub struct NorthstarGrid(pub Option<Arc<OrdinalGrid>>);

/// Постройка сетки northstar из заполненного navmesh (входы чанков,
/// кеши внутренних путей — считается параллельно внутри крейта).
pub fn build_from_navmesh(navmesh: &Navmesh) -> OrdinalGrid {
    let settings = GridSettingsBuilder::new_2d(GRID_SIZE.x as u32, GRID_SIZE.y as u32)
        .chunk_size(CHUNK_SIZE)
        .build();
    let mut grid = OrdinalGrid::new(&settings);
    for x in 0..GRID_SIZE.x {
        for y in 0..GRID_SIZE.y {
            let nav = if navmesh.is_passable(x, y) {
                Nav::Passable(1)
            } else {
                Nav::Impassable
            };
            grid.set_nav(UVec3::new(x as u32, y as u32, 0), nav);
        }
    }
    grid.build();
    grid
}

/// Путь через иерархию: `refined` (HPA* с трассировкой) либо Theta*
/// (any-angle, точки пути не обязаны быть соседними тайлами — движение
/// идёт к центрам точек по очереди, смежность ему не нужна).
pub fn find_path_northstar(
    grid: &OrdinalGrid,
    start: IVec2,
    end: IVec2,
    any_angle: bool,
) -> Option<Vec<IVec2>> {
    if start.min_element() < 0 || end.min_element() < 0 {
        return None;
    }
    let mut args = PathfindArgs::new(
        UVec3::new(start.x as u32, start.y as u32, 0),
        UVec3::new(end.x as u32, end.y as u32, 0),
    );
    args = if any_angle {
        args.thetastar()
    } else {
        args.refined()
    };
    let path = grid.pathfind(&mut args)?;
    Some(
        path.path()
            .iter()
            .map(|point| IVec2::new(point.x as i32, point.y as i32))
            .collect(),
    )
}
