//! Преобразования между мировыми координатами (метры) и навигационной сеткой
//! (navtiles 2 × 2 м). Начало сетки — юго-западный угол карты (0, 0).

use bevy::prelude::*;

use crate::settings::{GRID_SIZE, NAVTILE_SIZE};

pub fn world_to_tile(pos: Vec2) -> IVec2 {
    (pos / NAVTILE_SIZE).floor().as_ivec2()
}

pub fn tile_center(tile: IVec2) -> Vec2 {
    (tile.as_vec2() + 0.5) * NAVTILE_SIZE
}

pub fn tile_in_bounds(tile: IVec2) -> bool {
    tile.x >= 0 && tile.y >= 0 && tile.x < GRID_SIZE.x && tile.y < GRID_SIZE.y
}
