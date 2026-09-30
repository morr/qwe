//! **Линии колеи** — вдоль чего кладётся колея асфальта, одним оверлеем
//! поверх карты. У колеи два источника, и оверлей рисует оба разными цветами:
//!
//! 1. **Колея полос** — шейдер поверхности улиц (`surface.wgsl`) по раскладке
//!    полос ленты (`MeshBuilder::set_lanes`, [`LaneFrame`]): в каждой полосе
//!    две колеи в [`RUT_OFFSET`] от её середины. Оверлей — ось каждой полосы и
//!    две линии колёс по сторонам.
//! 2. **Колея траекторий узла** — `turns::JunctionWear`, кривые манёвров от
//!    кромки до кромки и хвосты вглубь полос; её кладёт
//!    `Painter::paint_turn_wear`. Оверлей — сами кривые и хвосты.
//!
//! Линии собирает **тот же проход**, что кладёт асфальт и краску
//! (`roads::build_roads`), — отсюда они и берутся, а не
//! пересчитываются: оверлей показывает ровно то, по чему колея легла.
//!
//! Упрощение, о котором надо знать: на клине между сечениями и на рампе
//! слияния раскладка асфальта плывёт (`set_lane_taper`, `set_lane_profile`), а
//! оверлей рисует раскладку тела ленты на всей её длине — на клине его линии
//! колёс расходятся с шейдерной колеёй на долю полосы.

use bevy::prelude::*;

use super::paint::RUT_OFFSET;
use crate::map::MeshBuilder;
use crate::map::meshing::{LaneFrame, RibbonCap, RibbonJoin, miter_offsets};
use crate::map::surface::{LayerMesh, MaterialSpec};
use crate::settings::Z_RUT_OVERLAY;

/// Толщина оси полосы, м.
const LANE_AXIS_WIDTH: f32 = 0.2;
/// Толщина линии колеса, м.
const WHEEL_WIDTH: f32 = 0.12;
/// Толщина кривой и хвоста траектории узла, м.
const TURN_WIDTH: f32 = 0.2;

/// Колея полос — оранжевым: ось полосы ярче, колёса бледнее.
const LANE_AXIS_COLOR: Color = Color::srgb(1.0, 0.55, 0.0);
const LANE_WHEEL_COLOR: Color = Color::srgba(1.0, 0.75, 0.3, 0.9);
/// Колея траекторий узла — зелёным: кривые и хвосты.
const TURN_CURVE_COLOR: Color = Color::srgb(0.1, 0.9, 0.25);
const TURN_TAIL_COLOR: Color = Color::srgb(0.55, 1.0, 0.2);

/// Лента, получившая колею полос: её ось (та, по которой лёг асфальт) и
/// раскладка тела.
#[derive(Clone, Debug)]
pub struct LaneRuts {
    pub axis: Vec<Vec2>,
    pub frame: LaneFrame,
}

/// Линии колеи карты — из прохода сборки дорог.
#[derive(Resource, Clone, Debug, Default)]
pub struct RutLines {
    /// Ленты с колеёй полос.
    pub lanes: Vec<LaneRuts>,
    /// Кривые манёвров всех узлов.
    pub curves: Vec<Vec<Vec2>>,
    /// Хвосты вглубь полос: `[у кромки, в глубине полосы]`.
    pub tails: Vec<[Vec2; 2]>,
}

impl RutLines {
    /// Середины полос раскладки поперёк ленты, м от её оси: границы полос —
    /// `origin + k · шаг`, проезжая часть — `low..high`.
    pub fn lane_centres(frame: LaneFrame, lane_width: f32) -> Vec<f32> {
        if lane_width <= 0.0 {
            return Vec::new();
        }
        let first = ((frame.low - frame.origin) / lane_width).ceil();
        let mut centres = Vec::new();
        let mut k = first;
        loop {
            let centre = frame.origin + (k + 0.5) * lane_width;
            if centre + lane_width / 2.0 > frame.high + 1e-3 {
                break;
            }
            centres.push(centre);
            k += 1.0;
        }
        centres
    }
}

/// Слой оверлея линий колеи при шаге полосы `lane_width`.
pub fn mesh_rut_overlay(ruts: &RutLines, lane_width: f32) -> LayerMesh {
    let mut builder = MeshBuilder::default();
    let mut line = |points: &[Vec2], width: f32, color: Color| {
        if points.len() < 2 {
            return;
        }
        builder.push_ribbon(
            points,
            false,
            width,
            color.to_linear(),
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    };
    for lane in &ruts.lanes {
        // вектор от оси до кромки единичной полуширины — линия в `across`
        // от оси лежит в `axis + offset · across`
        let unit = miter_offsets(&lane.axis, false, 1.0);
        let at = |across: f32| -> Vec<Vec2> {
            lane.axis
                .iter()
                .zip(&unit)
                .map(|(point, offset)| *point + *offset * across)
                .collect()
        };
        for centre in RutLines::lane_centres(lane.frame, lane_width) {
            line(&at(centre), LANE_AXIS_WIDTH, LANE_AXIS_COLOR);
            line(&at(centre - RUT_OFFSET), WHEEL_WIDTH, LANE_WHEEL_COLOR);
            line(&at(centre + RUT_OFFSET), WHEEL_WIDTH, LANE_WHEEL_COLOR);
        }
    }
    for curve in &ruts.curves {
        line(curve, TURN_WIDTH, TURN_CURVE_COLOR);
    }
    for tail in &ruts.tails {
        line(tail, TURN_WIDTH, TURN_TAIL_COLOR);
    }
    LayerMesh::new(builder, Z_RUT_OVERLAY, "rut_lines", MaterialSpec::Blend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_lane_frame_has_two_centres_off_the_axis() {
        let frame = LaneFrame {
            origin: 0.0,
            low: -3.3,
            high: 3.3,
        };
        let centres = RutLines::lane_centres(frame, 3.3);
        assert_eq!(centres.len(), 2);
        assert!((centres[0] + 1.65).abs() < 1e-4);
        assert!((centres[1] - 1.65).abs() < 1e-4);
    }

    #[test]
    fn an_odd_frame_keeps_its_middle_lane_on_the_axis() {
        let frame = LaneFrame {
            origin: 1.65,
            low: -4.95,
            high: 4.95,
        };
        let centres = RutLines::lane_centres(frame, 3.3);
        assert_eq!(centres.len(), 3);
        assert!(centres[1].abs() < 1e-4);
    }

    #[test]
    fn lanes_and_turns_draw_a_non_empty_overlay() {
        let ruts = RutLines {
            lanes: vec![LaneRuts {
                axis: vec![Vec2::ZERO, Vec2::new(50.0, 0.0)],
                frame: LaneFrame {
                    origin: 0.0,
                    low: -3.3,
                    high: 3.3,
                },
            }],
            curves: vec![vec![Vec2::ZERO, Vec2::new(5.0, 5.0), Vec2::new(10.0, 5.0)]],
            tails: vec![[Vec2::ZERO, Vec2::new(-10.0, 0.0)]],
        };
        let layer = mesh_rut_overlay(&ruts, 3.3);
        assert_eq!(layer.name, "rut_lines");
        // две полосы × три линии + кривая + хвост, по четыре вершины на отрезок
        assert!(layer.builder.vertex_count() >= 8 * 4);
    }
}
