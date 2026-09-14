//! Крепость: кремлёвская стена и её башни (`AreaKind::Kremlin`).
//!
//! Крепость рисуется не как дом, и отличий три. **Окон нет** ни на стене, ни
//! на башне — кладка, и только она (`layers::wall_frame` метит такую стену
//! `WallMark::Solid`). **Стена и башня кроются по-разному**: верх стены —
//! плоский боевой ход с зубцами ([`super::clutter::merlons`]), башня — шатёр.
//! И **цвет у крепости свой**, красный кирпич, а не палитра материала.
//!
//! Стену от башни отличает форма пятна, а не тег: у Тульского кремля стена —
//! `building=wall`, башни — `building=yes` + `man_made=tower`, но в других
//! городах бывает и `historic=citywalls` на обоих. Узкая длинная лента — стена,
//! компактное пятно — башня ([`is_tower`]).

use bevy::prelude::*;

use super::material::RoofKind;
use super::roofs::LandmarkRoof;
use crate::map::osm::PolyArea;
use crate::map::osm::model::signed_ring_area;

/// Компактность пятна (`площадь / периметр²`), выше которой это башня, а не
/// прясло стены. У квадрата 1/16 ≈ 0.063, у прямоугольника 5 : 1 — 0.035, у
/// ленты стены 3 × 40 м — 0.017.
const TOWER_COMPACTNESS_MIN: f32 = 0.03;

/// Кирпич крепостной стены: тульский кремль красно-оранжевый, и светлее
/// прежней тёмной константы — та на тени стены уходила почти в чёрный.
pub(super) const WALL_COLORS: [Color; 2] =
    [Color::srgb(0.66, 0.36, 0.28), Color::srgb(0.62, 0.34, 0.27)];
/// Боевой ход по верху стены — тот же кирпич, чуть темнее стены на свету.
const WALKWAY_COLORS: [Color; 1] = [Color::srgb(0.56, 0.32, 0.26)];
/// Шатры башен — крашеное железо, серо-зелёное.
const TOWER_ROOF_COLORS: [Color; 2] =
    [Color::srgb(0.40, 0.46, 0.42), Color::srgb(0.46, 0.48, 0.47)];

/// Подъём шатра башни в сторонах плана: крепостной шатёр ниже колокольного.
const TOWER_TENT_RISE: f32 = 0.9;

/// Башня ли это, а не прясло стены.
pub(super) fn is_tower(building: &PolyArea) -> bool {
    let perimeter: f32 = std::iter::once(&building.outer)
        .chain(&building.holes)
        .map(|ring| ring_perimeter(ring))
        .sum();
    if perimeter <= 0.0 || !building.holes.is_empty() {
        return false;
    }
    signed_ring_area(&building.outer).abs() / (perimeter * perimeter) >= TOWER_COMPACTNESS_MIN
}

fn ring_perimeter(ring: &[Vec2]) -> f32 {
    (0..ring.len())
        .map(|index| ring[index].distance(ring[(index + 1) % ring.len()]))
        .sum()
}

/// Форма крыши: шатёр на башне, плоский ход на стене.
pub(super) fn roof_form(building: &PolyArea) -> LandmarkRoof {
    match is_tower(building) {
        true => LandmarkRoof::Tent {
            rise: TOWER_TENT_RISE,
        },
        false => LandmarkRoof::Flat,
    }
}

/// Чем крыто: шатёр — фальцевым железом, ход — кирпичом, который фактура
/// гравия передаёт ровным зерном без швов.
pub(super) fn roof_kind(building: &PolyArea) -> RoofKind {
    match is_tower(building) {
        true => RoofKind::Seam,
        false => RoofKind::Gravel,
    }
}

pub(super) fn roof_palette(building: &PolyArea) -> &'static [Color] {
    match is_tower(building) {
        true => &TOWER_ROOF_COLORS,
        false => &WALKWAY_COLORS,
    }
}
