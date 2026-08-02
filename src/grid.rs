//! Преобразования между мировыми координатами (метры) и навигационной сеткой
//! (navtiles [`navtile_size`] × [`navtile_size`] м, 2 м по умолчанию). Начало
//! сетки — юго-западный угол карты (0, 0).

use bevy::prelude::*;

use crate::settings::navtile_size;

pub fn world_to_tile(pos: Vec2) -> IVec2 {
    (pos / navtile_size()).floor().as_ivec2()
}

pub fn tile_center(tile: IVec2) -> Vec2 {
    (tile.as_vec2() + 0.5) * navtile_size()
}
