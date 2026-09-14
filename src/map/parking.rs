//! Стоянки: асфальт с расчерченными местами и машинами на них.
//!
//! На снимке города двор со стоянкой ни с чем не спутать — прямоугольник
//! асфальта, расчерченный белыми полосками, и на половине мест что-то стоит.
//! До сих пор `amenity=parking` не запрашивался вовсе, и все эти площадки
//! были просто землёй.
//!
//! Места раскладываются **рядами вдоль длинной оси** площадки: ряд мест,
//! проезд, ряд мест — так их и размечают. Каждое место проверяется на
//! попадание в контур, поэтому Г-образная стоянка не получает мест поверх
//! газона, а площадка, в которую не встаёт ни одно место, — вовсе никаких.
//! Маленький двор места получает, но не разметку: см. [`MIN_AREA`].
//!
//! Разметка и машины делят **один** список мест: раскладка считается один раз
//! на загрузку мира в [`ParkingLayout`] (`map::spawn::spawn_map`), а краска
//! здесь и машины (`map::cars::fill_lots`) её только читают. Две независимые
//! раскладки поставили бы машину мимо её полосы.

use bevy::prelude::*;

use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::model::{point_in_area, ring_bounds, signed_ring_area};
use crate::map::osm::{PolyArea, RoadLine};

/// Место, м: легковая машина плюс просвет по обе стороны.
const STALL_WIDTH: f32 = 2.6;
const STALL_DEPTH: f32 = 5.2;
/// Проезд между спинами двух рядов, м.
const AISLE: f32 = 6.0;
/// Отступ разметки от края площадки, м.
const EDGE_MARGIN: f32 = 1.2;
/// Ширина полосы разметки, м, и её цвет — та же белая краска, что на улице.
const LINE_WIDTH: f32 = 0.12;
const LINE_COLOR: Color = Color::srgb(0.82, 0.82, 0.80);
/// Стоянка мельче этого пятна не получает **разметки**: две машины во дворе
/// никто не расчерчивает. Места на ней остаются — машины на них стоят
/// (`map::cars::fill_lots`), просто по неразмеченному асфальту.
const MIN_AREA: f32 = 120.0;

/// Одно место: центр и направление, в котором машина стоит.
pub struct Stall {
    pub at: Vec2,
    pub along: Vec2,
}

/// Раскладка всех стоянок карты — по списку мест на контур `MapData::parking`,
/// в том же порядке.
///
/// Кеш здесь окупается там, где у разрывов разметки (`roads::junctions`) не
/// окупился: вход раскладки — только контуры стоянок, а они меняются лишь со
/// сменой мира, тогда как слой машин пересобирается на каждое деление ползунка
/// солнца, зума и стиля дорог. Считать одно и то же по кадру — ровно та работа,
/// которой быть не должно; плата — резидентные 16 байт на место (Тула ~0.17 МБ).
#[derive(Resource, Default)]
pub struct ParkingLayout(pub Vec<Vec<Stall>>);

impl ParkingLayout {
    /// Места всех стоянок, **кроме тех, что легли под дорогу**. Контур
    /// стоянки в OSM бывает нарисован криво и накрывает настоящую улицу — в Туле
    /// так через большую стоянку у развязки идёт односторонняя дорога, — а
    /// ряды мест о дорогах не знают, и машины вставали поперёк неё. Проезд
    /// самой стоянки (`parking_aisle`) не в счёт: он внутри стоянки не рисуется,
    /// и ряды стоят по своей сетке. Мост тоже: он над стоянкой.
    pub fn new(lots: &[PolyArea], roads: &[RoadLine]) -> Self {
        let crossings: Vec<Crossing> = roads
            .iter()
            .filter(|road| !road.bridge && !road.parking_aisle)
            .filter_map(Crossing::of)
            .collect();
        Self(
            lots.iter()
                .map(|lot| {
                    let (min, max) = ring_bounds(&lot.outer);
                    let near: Vec<&Crossing> = crossings
                        .iter()
                        .filter(|road| road.min.cmple(max).all() && road.max.cmpge(min).all())
                        .collect();
                    let mut found = stalls(lot);
                    found.retain(|stall| !near.iter().any(|road| road.covers(stall)));
                    found
                })
                .collect(),
        )
    }
}

/// Дорога, под которой мест не бывает: осевая, полуширина и AABB, раздутый на
/// полуширину и полдлины места — чтобы стоянку спрашивали только о дорогах
/// рядом.
struct Crossing<'a> {
    points: &'a [Vec2],
    half_width: f32,
    min: Vec2,
    max: Vec2,
}

impl<'a> Crossing<'a> {
    fn of(road: &'a RoadLine) -> Option<Self> {
        if road.points.len() < 2 {
            return None;
        }
        let half_width = road.width / 2.0;
        let (min, max) = ring_bounds(&road.points);
        let grow = Vec2::splat(half_width + STALL_DEPTH);
        Some(Self {
            points: &road.points,
            half_width,
            min: min - grow,
            max: max + grow,
        })
    }

    /// Лента дороги задевает место. Место — прямоугольник в своей рамке
    /// (поперёк — ширина места, вдоль машины — глубина), раздутый на
    /// полуширину дороги; задевает, если осевая пересекает раздутый
    /// прямоугольник. На углах это чуть строже честного расстояния (квадратный
    /// угол вместо скругления) — лишнее место у кромки дороги не жалко.
    fn covers(&self, stall: &Stall) -> bool {
        let length_axis = stall.along;
        let width_axis = Vec2::new(-length_axis.y, length_axis.x);
        let half = Vec2::new(
            STALL_WIDTH / 2.0 + self.half_width,
            STALL_DEPTH / 2.0 + self.half_width,
        );
        let local = |point: Vec2| {
            let offset = point - stall.at;
            Vec2::new(offset.dot(width_axis), offset.dot(length_axis))
        };
        self.points
            .windows(2)
            .any(|pair| segment_hits_box(local(pair[0]), local(pair[1]), half))
    }
}

/// Отрезок пересекает прямоугольник `[-half, half]` (Лианг–Барски).
fn segment_hits_box(from: Vec2, to: Vec2, half: Vec2) -> bool {
    let span = to - from;
    let (mut enter, mut exit) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        let (start, delta, limit) = (from[axis], span[axis], half[axis]);
        if delta == 0.0 {
            if start.abs() > limit {
                return false;
            }
            continue;
        }
        let (mut near, mut far) = ((-limit - start) / delta, (limit - start) / delta);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        enter = enter.max(near);
        exit = exit.min(far);
        if enter > exit {
            return false;
        }
    }
    true
}

/// Места стоянки — рядами вдоль её длинной оси. Пусто, если площадка мелкая
/// или вырожденная.
pub fn stalls(area: &PolyArea) -> Vec<Stall> {
    let Some(rect) = min_area_rect(&area.outer) else {
        return Vec::new();
    };
    let along = (rect[1] - rect[0]).try_normalize().unwrap_or(Vec2::X);
    let across = Vec2::new(-along.y, along.x);
    let length = (rect[1] - rect[0]).length() - 2.0 * EDGE_MARGIN;
    let width = (rect[2] - rect[1]).length() - 2.0 * EDGE_MARGIN;
    if length < STALL_WIDTH || width < STALL_DEPTH {
        return Vec::new();
    }
    let origin = rect[0] + (along + across) * EDGE_MARGIN;

    // ряд мест, проезд, ряд мест: шаг поперёк — две глубины плюс проезд, и
    // в каждом таком шаге два ряда, спинами друг к другу
    let mut stalls = Vec::new();
    let pitch = 2.0 * STALL_DEPTH + AISLE;
    let mut row = 0.0;
    while row + STALL_DEPTH <= width {
        for side in [0.0, STALL_DEPTH] {
            if row + side + STALL_DEPTH > width {
                continue;
            }
            let depth = row + side + STALL_DEPTH / 2.0;
            let mut place = STALL_WIDTH / 2.0;
            while place + STALL_WIDTH / 2.0 <= length {
                let at = origin + along * place + across * depth;
                // машина стоит поперёк ряда, носом в проезд
                if fits(area, at, along, across) {
                    stalls.push(Stall { at, along: across });
                }
                place += STALL_WIDTH;
            }
        }
        row += pitch;
    }
    stalls
}

/// Место целиком внутри контура — по четырём углам, как и коробки на кровле.
fn fits(area: &PolyArea, at: Vec2, along: Vec2, across: Vec2) -> bool {
    let half_width = along * (STALL_WIDTH / 2.0);
    let half_depth = across * (STALL_DEPTH / 2.0);
    [
        at - half_width - half_depth,
        at + half_width - half_depth,
        at + half_width + half_depth,
        at - half_width + half_depth,
    ]
    .iter()
    .all(|corner| point_in_area(*corner, area))
}

/// Разметка мест в меш: по полоске между соседними местами. Полоса, а не
/// прямоугольник места: расчерчивают именно границы.
pub fn push_markings(builder: &mut MeshBuilder, area: &PolyArea, stalls: &[Stall]) {
    if signed_ring_area(&area.outer).abs() < MIN_AREA {
        return;
    }
    let color = LINE_COLOR.to_linear();
    for stall in stalls {
        let along = stall.along;
        let across = Vec2::new(-along.y, along.x);
        let half_depth = along * (STALL_DEPTH / 2.0);
        let half_line = across * (LINE_WIDTH / 2.0);
        // граница слева от места: две соседние границы совпадут, и это
        // дешевле, чем искать соседа
        let edge = stall.at - across * (STALL_WIDTH / 2.0);
        builder.push_quad(
            [
                edge - half_depth - half_line,
                edge + half_depth - half_line,
                edge + half_depth + half_line,
                edge - half_depth + half_line,
            ],
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::{AreaKind, BuildingUse};

    fn lot(outer: Vec<Vec2>) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind: AreaKind::Parking,
            building_use: BuildingUse::Other,
            height: None,
            entrances: Vec::new(),
        }
    }

    fn rect(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::ZERO,
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    #[test]
    fn a_lot_is_filled_with_rows_of_stalls() {
        let lot = lot(rect(20.0, 40.0));
        let stalls = stalls(&lot);
        // 40 × 20 держит ровно пару рядов спинами: на проезд и следующую пару
        // нужно 2 · 1.2 + 2 · 5.2 + 6.0 + 5.2 = 24 м поперёк
        assert!(stalls.len() > 20, "{}", stalls.len());
        for stall in &stalls {
            assert!(point_in_area(stall.at, &lot), "{:?}", stall.at);
        }
    }

    /// Шаг поперёк — пара рядов, проезд, пара рядов; на 40 × 20 из теста выше
    /// ветка с проездом не исполняется ни разу, поэтому площадка здесь шире.
    #[test]
    fn rows_pair_up_with_an_aisle_between_the_pairs() {
        let lot = lot(rect(30.0, 40.0));
        let stalls = stalls(&lot);
        // полосы по глубине (проекция центра на `Stall::along`): их четыре,
        // и шаги между ними — 5.2, 5.2 + 6.0, 5.2
        let mut bands: Vec<f32> = Vec::new();
        for stall in &stalls {
            let depth = stall.at.dot(stall.along);
            if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
                bands.push(depth);
            }
        }
        bands.sort_by(f32::total_cmp);
        assert_eq!(bands.len(), 4, "{bands:?}");
        let gaps: Vec<f32> = bands.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!((gaps[0] - STALL_DEPTH).abs() < 0.01, "{gaps:?}");
        assert!((gaps[1] - (STALL_DEPTH + AISLE)).abs() < 0.01, "{gaps:?}");
        assert!((gaps[2] - STALL_DEPTH).abs() < 0.01, "{gaps:?}");
    }

    #[test]
    fn a_yard_corner_gets_nothing() {
        // 4 × 6 — двор на пару машин, размечать нечего
        assert!(stalls(&lot(rect(4.0, 6.0))).is_empty());
    }

    #[test]
    fn stalls_stay_out_of_a_notch() {
        // Г-образная площадка: места не должны лечь в вырез
        let ell = lot(vec![
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 12.0),
            Vec2::new(18.0, 12.0),
            Vec2::new(18.0, 26.0),
            Vec2::new(0.0, 26.0),
        ]);
        for stall in stalls(&ell) {
            assert!(point_in_area(stall.at, &ell), "{:?}", stall.at);
        }
    }

    /// Дорога через стоянку (кривой контур в OSM) выбивает места под своей
    /// лентой и не трогает остальные; проезд самой стоянки и мост — не выбивают.
    #[test]
    fn a_road_across_the_lot_takes_the_stalls_under_it() {
        use crate::map::osm::fixture::{bridge, street};

        let lot = lot(rect(30.0, 60.0));
        let all = stalls(&lot).len();
        let road = street(vec![Vec2::new(30.0, -10.0), Vec2::new(30.0, 40.0)], 5.0);
        let crossed =
            &ParkingLayout::new(std::slice::from_ref(&lot), std::slice::from_ref(&road)).0[0];
        assert!(crossed.len() < all, "{} of {all}", crossed.len());
        assert!(crossed.len() > all / 2, "{} of {all}", crossed.len());
        for stall in crossed {
            // места у дороги — не ближе полуширины места и дороги к оси
            assert!(
                (stall.at.x - 30.0).abs() >= STALL_WIDTH / 2.0 + 2.5 - 0.01,
                "{:?}",
                stall.at
            );
        }

        let aisle = RoadLine {
            parking_aisle: true,
            ..road.clone()
        };
        let over = bridge(road.points.clone(), 5.0);
        for spared in [aisle, over] {
            assert_eq!(
                ParkingLayout::new(std::slice::from_ref(&lot), &[spared]).0[0].len(),
                all
            );
        }
    }
}
