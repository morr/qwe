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
use crate::map::osm::PolyArea;
use crate::map::osm::model::{point_in_area, signed_ring_area};

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
    pub fn new(lots: &[PolyArea]) -> Self {
        Self(lots.iter().map(stalls).collect())
    }
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
}
