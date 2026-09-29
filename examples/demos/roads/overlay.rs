//! Тумблеры оверлеев. Сами слои — игровые, их же рисуют строки вкладки Debug
//! игры:
//!
//! - сеть (`qwe::map::mesh_network_overlay`, строка `Road network`): улицы сети
//!   каждая своим цветом и швы между ways — строка `Network` панели, либо
//!   `ROADS_NETWORK=1` при запуске;
//! - контуры OSM (`qwe::map::mesh_osm_contours`, строка `OSM contours`): оси
//!   путей и контуры полигонов до доводочных проходов разбора — «данные или наш
//!   разбор» на одном кадре; строка `Contours`, либо `ROADS_CONTOURS=1`;
//! - линии колеи (`qwe::map::mesh_rut_overlay`, строка `Rut lines`): оси полос
//!   с линиями колёс и траектории узлов — строка `Ruts`, либо `ROADS_RUTS=1`.
//!
//! Переменные окружения — для автоснимка.

use bevy::prelude::*;

/// Какие оверлеи показывать.
#[derive(Resource, Clone, Copy, PartialEq)]
pub(crate) struct Overlays {
    pub network: bool,
    pub contours: bool,
    pub ruts: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            network: std::env::var_os("ROADS_NETWORK").is_some(),
            contours: std::env::var_os("ROADS_CONTOURS").is_some(),
            ruts: std::env::var_os("ROADS_RUTS").is_some(),
        }
    }
}
