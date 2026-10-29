//! Лента крепостной стены (`WallLine`) — и что её режет: крепостные
//! сооружения карты ([`Fortresses`]) и замапленные газоны ([`Meadows`]),
//! по которым широкая обочина выбирает свой луг.

use bevy::prelude::*;

use super::verges::{MEADOW_PROBE_BEYOND, MEADOW_PROBES};
use crate::map::osm::model::{point_in_area, polyline_length, ring_bounds};
use crate::map::osm::{AreaKind, PolyArea};

/// Шаг, с которым осевая крепостной стены проверяется на «стоит ли тут
/// здание стены», м.
const WALL_PROBE_STEP: f32 = 2.0;

/// Крепостные сооружения карты (`AreaKind::Kremlin`) с их AABB — чтобы лента
/// `barrier=city_wall` не рисовалась поверх стены, которая уже нарисована
/// зданием.
///
/// Лента — единственный рисунок стены там, где мапер провёл только линию. Но
/// у Тульского кремля есть и `building=wall`, и башни, и красная лента
/// ложилась по ним сверху: в 2.5D — тёмно-оранжевой обводкой рядом с поднятой
/// стеной, у башен — кругами поверх шатров. Поэтому лента режется на куски, и
/// рисуются только те, что идут **мимо** крепостных зданий. Навмеша это не
/// касается: он по-прежнему блокирует всю ленту.
pub(super) struct Fortresses<'a> {
    areas: Vec<(&'a PolyArea, (Vec2, Vec2))>,
}

impl<'a> Fortresses<'a> {
    pub(super) fn of(buildings: &'a [PolyArea]) -> Self {
        Self {
            areas: buildings
                .iter()
                .filter(|building| building.kind == AreaKind::Kremlin)
                .map(|building| (building, ring_bounds(&building.outer)))
                .collect(),
        }
    }

    fn covers(&self, point: Vec2) -> bool {
        self.areas.iter().any(|(area, (min, max))| {
            point.cmpge(*min).all() && point.cmple(*max).all() && point_in_area(point, area)
        })
    }

    /// Куски осевой, не накрытые крепостными зданиями. Каждый отрезок
    /// проверяется точками через [`WALL_PROBE_STEP`]; кусок начинается и
    /// кончается на такой точке.
    pub(super) fn bare_runs(&self, points: &[Vec2]) -> Vec<Vec<Vec2>> {
        if self.areas.is_empty() {
            return vec![points.to_vec()];
        }
        let mut runs = Vec::new();
        let mut current: Vec<Vec2> = Vec::new();
        let mut cut = false;
        let mut visit = |point: Vec2, runs: &mut Vec<Vec<Vec2>>| {
            if self.covers(point) {
                cut = true;
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                }
                current.clear();
            } else {
                current.push(point);
            }
        };
        for (index, pair) in points.windows(2).enumerate() {
            let steps = (pair[0].distance(pair[1]) / WALL_PROBE_STEP)
                .ceil()
                .max(1.0) as usize;
            // первая точка отрезка — только у первого, дальше она уже была
            // концом предыдущего
            let from = usize::from(index > 0);
            for step in from..=steps {
                visit(pair[0].lerp(pair[1], step as f32 / steps as f32), &mut runs);
            }
        }
        if current.len() >= 2 {
            runs.push(current);
        }
        // Огрызок между двумя зданиями — не стена, а зазор разметки: осевая
        // `city_wall` и контур башни в OSM расходятся на метр-другой, и на
        // Тульском кремле у угловой башни от ленты оставался красный язычок.
        // Режется только там, где лента вообще резалась: неразрезанная линия
        // любой длины — единственный рисунок своей стены.
        if cut {
            runs.retain(|run| polyline_length(run) >= WALL_STUB_MAX);
        }
        runs
    }
}

/// Замапленные газоны и скверы (`MapData::parks`, `MapData::grass`) — рядом с
/// ними газон широкой обочины ([`super::verges::push_verges`]) засеян их
/// лугом, а везде ещё — приглушённой травой двора.
pub(super) struct Meadows<'a> {
    areas: Vec<(&'a PolyArea, (Vec2, Vec2))>,
}

impl<'a> Meadows<'a> {
    pub(super) fn of(parks: &'a [PolyArea], grass: &'a [PolyArea]) -> Self {
        Self {
            areas: parks
                .iter()
                .chain(grass)
                .map(|area| (area, ring_bounds(&area.outer)))
                .collect(),
        }
    }

    fn covers(&self, point: Vec2) -> bool {
        self.areas.iter().any(|(area, (min, max))| {
            point.cmpge(*min).all() && point.cmple(*max).all() && point_in_area(point, area)
        })
    }

    /// Лежит ли газон у обочины шириной `verge` по стороне `side` дороги
    /// шириной `width` по `points`: пробы [`MEADOW_PROBES`] долей длины — на
    /// середине обочины и на [`MEADOW_PROBE_BEYOND`] за ней. Газон в OSM то
    /// доходит до бордюра, то кончается у дорожки, отсюда две глубины.
    pub(super) fn beside(&self, points: &[Vec2], width: f32, side: usize, verge: f32) -> bool {
        if self.areas.is_empty() {
            return false;
        }
        let (along, total) = crate::map::along::arclengths(points);
        // `perp` смотрит влево — сторона 0
        let sign = if side == 0 { 1.0 } else { -1.0 };
        MEADOW_PROBES.iter().any(|share| {
            let Some((at, direction)) =
                crate::map::along::place_on_path(points, &along, total * share)
            else {
                return false;
            };
            let normal = direction.perp() * sign;
            [
                width / 2.0 + verge / 2.0,
                width / 2.0 + verge + MEADOW_PROBE_BEYOND,
            ]
            .into_iter()
            .any(|offset| self.covers(at + normal * offset))
        })
    }

    /// Лежит ли газон под каким-нибудь углом контура `outline` или его
    /// серединой — для газона угла
    /// ([`super::corners::KerbReturns::verge_lawns`]).
    pub(super) fn under(&self, outline: &[Vec2]) -> bool {
        let centre = outline.iter().copied().sum::<Vec2>() / outline.len().max(1) as f32;
        !self.areas.is_empty()
            && outline
                .iter()
                .chain([&centre])
                .any(|&point| self.covers(point))
    }
}

/// Кусок крепостной ленты короче этого между крепостными зданиями не рисуется, м.
/// Двенадцати не хватило: у северо-восточных башен Тульского кремля осевая
/// расходится с контурами на куски в пятнадцать–тридцать метров, и от ленты
/// оставались красные крюки у каждого угла. Прясло стены между башнями длиннее
/// сорока метров, так что настоящий неразмеченный кусок стены под порог не
/// попадает.
const WALL_STUB_MAX: f32 = 40.0;
