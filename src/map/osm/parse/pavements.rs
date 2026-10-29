//! Покрытие дорожек без тега `surface` — по тому, где они идут.
//!
//! Теги решают больше половины дорожек ([`super::tags::tagged_pavement`]);
//! остаётся голый `footway`, и его решает **середина пути по длине**: больше
//! половины проб по парку, лесу или газону — тропинка, иначе — асфальт двора
//! и улицы. Кроме звена между мощёными: дорожка по зелени, оба конца которой
//! стоят на мощёных дорожках, — тоже мощёная. Как рисуется то и другое —
//! `roads.md`, **Paved paths**; здесь решается.

use std::collections::HashSet;

use bevy::math::Vec2;

use super::vertex_key;
use crate::map::along::densify;
use crate::map::grid::Grid;
use crate::map::osm::model::{
    MapData, Pavement, PolyArea, RoadClass, RoadLine, point_in_area, ring_bounds,
};

/// Шаг, с которым дорожка без тега меряет, по зелени ли она идёт, м.
const PAVEMENT_PROBE_STEP: f32 = 10.0;
/// Клетка индекса зелени, м: парк — сотни метров, сквер — десятки.
const GREEN_CELL: f32 = 100.0;

/// Что решил [`infer_pavements`]: сколько дорожек без покрытия было спрошено,
/// сколько из них ушли по зелени в тропинки и сколько остались мощёными,
/// потому что оба их конца на мощёных дорожках.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct InferredPavements {
    asked: usize,
    unpaved: usize,
    between_paved: usize,
}

impl std::fmt::Display for InferredPavements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            asked,
            unpaved,
            between_paved,
        } = self;
        write!(
            f,
            "osm parse: {unpaved} of {asked} untagged footways run through greenery and stay \
             unpaved, {between_paved} more are paved between paved paths"
        )
    }
}

/// Контекст прохода покрытий: всё, что проход читает, — ссылками, и
/// единственное, что он пишет, — дорожки (их поле `pavement`). Узкий проход
/// не получает `&mut MapData` целиком: список полей и есть договор с картой —
/// чего здесь нет, того проход не касается, — и тест строит его руками, без
/// карты. `MapData` нужен только мосту [`InferPavementsCtx::of`].
pub(super) struct InferPavementsCtx<'a> {
    roads: &'a mut Vec<RoadLine>,
    parks: &'a [PolyArea],
    woods: &'a [PolyArea],
    grass: &'a [PolyArea],
}

impl<'a> InferPavementsCtx<'a> {
    /// Собрать контекст с карты — мост между швом и его вызывающим.
    pub(super) fn of(map: &'a mut MapData) -> Self {
        let MapData {
            roads,
            parks,
            woods,
            grass,
            ..
        } = map;
        Self {
            roads,
            parks,
            woods,
            grass,
        }
    }
}

/// Покрытие дорожки без тега `surface` — по тому, где она идёт, по контексту
/// [`InferPavementsCtx`], а не по карте целиком.
///
/// Мощёную дорожку долго рисовали песчаной тропинкой, как и всякую: в центре
/// города тротуар из плитки (`footway=sidewalk, surface=paving_stones`)
/// ложился песчаной лентой рядом с бетонной полосой тротуара улицы, а сетка
/// аллей сквера из плитки — пляжем (разведка C1). По тегам в Туле и в парке
/// мощёных больше, но всё мощёное там и размечено; молчащий `footway` в
/// сквере — чаще протоптанная дорожка. **Кроме звена между мощёными**:
/// дорожка по зелени, оба конца которой — точки мощёных дорожек (тег, род —
/// лестница, площадь — или сама решённая мощёной), тоже мощёная. Один проход,
/// без цепочек: звено, стоящее на другом таком звене, остаётся тропинкой.
///
/// Детерминировано: пробы — функция точек пути, индекс зелени — её контуров.
pub(super) fn infer_pavements(ctx: InferPavementsCtx<'_>) -> InferredPavements {
    let InferPavementsCtx {
        roads,
        parks,
        woods,
        grass,
    } = ctx;
    let asked: Vec<usize> = roads
        .iter()
        .enumerate()
        .filter(|(_, road)| road.class == RoadClass::Alley && road.pavement.is_none())
        .map(|(index, _)| index)
        .collect();
    if asked.is_empty() {
        return InferredPavements::default();
    }
    let green: Vec<&PolyArea> = parks
        .iter()
        .chain(woods.iter())
        .chain(grass.iter())
        .collect();
    let mut index: Grid<usize> = Grid::new(GREEN_CELL);
    for (at, area) in green.iter().enumerate() {
        let (min, max) = ring_bounds(&area.outer);
        index.insert(min, max, at);
    }
    let in_green = |point: Vec2| {
        index
            .at(point)
            .iter()
            .any(|&at| point_in_area(point, green[at]))
    };
    let mut unpaved = 0;
    for &at in &asked {
        let road = &mut roads[at];
        let probes = densify(&road.points, PAVEMENT_PROBE_STEP);
        let inside = probes.iter().filter(|point| in_green(**point)).count();
        road.pavement = Some(if 2 * inside > probes.len() {
            unpaved += 1;
            Pavement::Unpaved
        } else {
            Pavement::Paved
        });
    }
    // Дорожка по газону, оба конца которой стоят на мощёных, — звено мощёной
    // сети, а не тропинка: лучи сквера от кольцевой аллеи к лестницам
    // центральной площадки ложились песком, и лестницы торчали на нём
    // мощёными обрубками с круглыми торцами (Тула, остров кольца, витрина 04)
    let paved_joints: HashSet<(i32, i32)> = roads
        .iter()
        .filter(|road| road.is_paved_path())
        .flat_map(|road| road.points.iter().map(|&point| vertex_key(point)))
        .collect();
    let mut between_paved = 0;
    for &at in &asked {
        let road = &mut roads[at];
        let (Some(&first), Some(&last)) = (road.points.first(), road.points.last()) else {
            continue;
        };
        if road.pavement == Some(Pavement::Unpaved)
            && vertex_key(first) != vertex_key(last)
            && paved_joints.contains(&vertex_key(first))
            && paved_joints.contains(&vertex_key(last))
        {
            road.pavement = Some(Pavement::Paved);
            unpaved -= 1;
            between_paved += 1;
        }
    }
    InferredPavements {
        asked: asked.len(),
        unpaved,
        between_paved,
    }
}

#[cfg(test)]
mod tests;
