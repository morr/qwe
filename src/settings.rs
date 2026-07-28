//! Все размеры и скорости — метры и м/с. Пиксели существуют только в
//! константах рендера (`PIXELS_PER_METER`) и настройках зума камеры.

use bevy::prelude::*;

/// Пиксельная плотность ассетов: 16 px = 1 м.
pub const PIXELS_PER_METER: f32 = 16.0;

/// Карта: центр Тулы, реальные данные OpenStreetMap вокруг гео-центра ниже.
/// Начало координат — юго-западный угол bbox.
pub const MAP_SIZE: Vec2 = Vec2::new(3000.0, 2250.0);

/// Гео-центр карты (Тула, Кремль у центра кадра). Проекция —
/// локальная равнопромежуточная: метры от юго-западного угла bbox.
pub const GEO_CENTER_LAT: f64 = 54.19021;
pub const GEO_CENTER_LON: f64 = 37.61485;
/// Метров в градусе широты (и долготы на экваторе).
pub const METERS_PER_DEG_LAT: f64 = 111_320.0;

/// Ячейка навигации, м.
pub const NAVTILE_SIZE: f32 = 2.0;

/// Размер навигационной сетки в тайлах: `MAP_SIZE / NAVTILE_SIZE`.
pub const GRID_SIZE: IVec2 = IVec2::new(1500, 1125);

/// Центр портала — хинт; при старте снапится к ближайшему проходимому тайлу.
pub const PORTAL_POS: Vec2 = Vec2::new(620.0, 380.0);
pub const PORTAL_DIAMETER: f32 = 9.0;

// --- Люди ---
/// 20000 на карте центра Тулы (изначальная цель MVP была 5000 [Q5]).
pub const HUMAN_COUNT: usize = 20000;
/// Визуальный габарит ×2 против реального (0.5 м) — иначе слишком мелко.
pub const HUMAN_SIZE: f32 = 1.0;
/// Скорости ×2 против реализма — так живее.
pub const HUMAN_WALK_SPEED: f32 = 2.8;
pub const HUMAN_FLEE_SPEED: f32 = 8.0;
/// Радиус, в котором человек замечает демона и паникует.
pub const HUMAN_PANIC_RADIUS: f32 = 60.0;
/// Блуждание: случайная точка в 20–40 м, затем пауза 2–10 сек.
pub const HUMAN_WANDER_RANGE: (f32, f32) = (20.0, 40.0);
pub const HUMAN_WANDER_PAUSE: (f32, f32) = (2.0, 10.0);

// --- Демоны ---
/// Визуальный габарит ×2 против фигуры-заглушки 1 × 1 м.
pub const DEMON_SIZE: f32 = 2.0;
/// Единая скорость демона: всегда +35% к скорости убегающего человека,
/// и в блуждании, и в погоне.
pub const DEMON_SPEED: f32 = HUMAN_FLEE_SPEED * 1.35;
/// Радиус агро демона.
pub const DEMON_AGGRO_RADIUS: f32 = 45.0;
/// Пауза «пожирания» после убийства, сек.
pub const DEMON_DEVOUR_PAUSE: (f32, f32) = (1.5, 2.0);
/// Дистанция убийства.
pub const KILL_DISTANCE: f32 = 1.0;
/// Спавн: стартовый залп, затем интервал, кап.
pub const DEMON_INITIAL_BURST: usize = 8;
pub const DEMON_SPAWN_INTERVAL: f32 = 5.0;
pub const DEMON_CAP: usize = 100;

/// Гистерезис выхода из погони/паники: множитель радиуса.
pub const RADIUS_HYSTERESIS: f32 = 1.5;

// --- Z-слои (см. y-сортировку юнитов) ---
pub const Z_GROUND: f32 = 0.0;
pub const Z_PARK: f32 = 0.5;
pub const Z_POND: f32 = 1.0;
pub const Z_ALLEY: f32 = 1.5;
pub const Z_ROAD: f32 = 2.0;
pub const Z_CORPSE: f32 = 3.0;
pub const Z_PORTAL: f32 = 4.0;
pub const Z_BUILDING: f32 = 5.0;
/// Юниты: z = `Z_UNIT_BASE - y * Y_SORT_FACTOR` (кто ниже — тот ближе).
pub const Z_UNIT_BASE: f32 = 10.0;
pub const Y_SORT_FACTOR: f32 = 0.005;
pub const Z_TREE: f32 = 20.0;

/// Z юнита по его мировой y-координате.
pub fn unit_z(y: f32) -> f32 {
    Z_UNIT_BASE - y * Y_SORT_FACTOR
}
