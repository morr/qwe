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
use crate::map::osm::model::is_fortress_tower;

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

/// Башня ли это, а не прясло стены ([`crate::map::osm::model::is_fortress_tower`]
/// — правило одно на разбор, который режет прясла по башням, и на отрисовку).
pub(super) fn is_tower(building: &PolyArea) -> bool {
    is_fortress_tower(building)
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
