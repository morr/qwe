//! Тумблер оверлея сети. Сам слой — игровой (`qwe::map::mesh_network_overlay`,
//! его же рисует строка `Road network` вкладки Debug игры): улицы сети каждая
//! своим цветом и швы между ways.
//!
//! Строка `Network` панели, либо `ROADS_NETWORK=1` при запуске — для
//! автоснимка.

use bevy::prelude::*;

/// Показывать ли оверлей сети.
#[derive(Resource, Clone, Copy, PartialEq)]
pub(crate) struct NetworkOverlay {
    pub visible: bool,
}

impl Default for NetworkOverlay {
    fn default() -> Self {
        Self {
            visible: std::env::var_os("ROADS_NETWORK").is_some(),
        }
    }
}
