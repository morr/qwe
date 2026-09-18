//! Навигационная сетка как единица измерения: размер навтайла, размер сетки в
//! тайлах и преобразования мир↔тайл. Начало сетки — юго-западный угол карты
//! (0, 0).
//!
//! Живёт здесь, а не в `settings.rs`, потому что размер навтайла — не ручка
//! среди прочих ручек мира, а масштаб, в котором построено всё навигационное:
//! проходимость, иерархия northstar, генерация входов, слоты назначения. У
//! него один владелец, и владелец — эта сетка.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::MAP_SIZE;

/// Ячейка навигации по умолчанию, м. Живое значение переключается кнопкой
/// `navtile:` ([`NavtileBase`]) и читается через [`navtile_size`].
pub const DEFAULT_NAVTILE_SIZE: f32 = 2.0;

/// Текущий размер навтайла — process-global атомик, а не ресурс: его читают
/// потоки без доступа к ECS (заливка navmesh в потоке загрузки, генерация
/// входов там же). Пишется он только на главном потоке и только когда ни один
/// из этих потоков не жив (`loading::sync_navtile_size` в начале `Loading`).
static NAVTILE_SIZE_BITS: AtomicU32 = AtomicU32::new(DEFAULT_NAVTILE_SIZE.to_bits());

/// Текущий размер ячейки навигации, м.
pub fn navtile_size() -> f32 {
    f32::from_bits(NAVTILE_SIZE_BITS.load(Ordering::Relaxed))
}

/// Пишет живой размер навтайла. `pub(crate)`, и это не формальность:
/// единственное безопасное место записи — `loading::sync_navtile_size` на
/// входе в `Loading`, когда ни один фоновый поток не жив.
pub(crate) fn set_navtile_size(size: f32) {
    NAVTILE_SIZE_BITS.store(size.to_bits(), Ordering::Relaxed);
}

/// Размер навигационной сетки в тайлах: `MAP_SIZE / navtile_size()`.
pub fn grid_size() -> IVec2 {
    (MAP_SIZE / navtile_size()).as_ivec2()
}

/// Базовый размер навтайла — кнопка `navtile:` в debug-панели. Смена значения
/// перезагружает мир (`city::reload_world`): проходимость и иерархия northstar
/// существуют только в тайлах текущего размера.
///
/// Цена 1 м против 2 м (Тула, замер `pathfinding_bench`): постройка northstar
/// 14.4 с против 11 с, HPA* ×1.7 по CPU, +1.6 ГБ RSS — зато косые ленты
/// держатся от 1.41 м вместо 2.83 м, бордюры и арки заметно точнее.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "navigation", key = "navtile")]
pub enum NavtileBase {
    #[default]
    M2,
    M1,
}

impl NavtileBase {
    pub fn size(self) -> f32 {
        match self {
            Self::M2 => 2.0,
            Self::M1 => 1.0,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::M2 => Self::M1,
            Self::M1 => Self::M2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::M2 => "2m",
            Self::M1 => "1m",
        }
    }
}

pub fn world_to_tile(pos: Vec2) -> IVec2 {
    (pos / navtile_size()).floor().as_ivec2()
}

pub fn tile_center(tile: IVec2) -> Vec2 {
    (tile.as_vec2() + 0.5) * navtile_size()
}
