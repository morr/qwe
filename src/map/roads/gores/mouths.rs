//! **Устья** (R20) — пустоты, зажатые проезжими частями со всех сторон:
//! островок без кольца. Правило и границы — в документации [`mouths`];
//! подробности — `references/parking-big-lot.md` (**Gore**) и
//! `references/roads-merges.md` (**split_mouths**).

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};

use super::GORE_MIN_AREA;
use crate::map::osm::model::{distance_to_segment, point_in_polygon, ring_bounds};
use crate::map::shapes::{
    ARC, Contour, Shape, contour_area, contour_bounds, is_ring, oriented, ring_of,
};

/// Устье больше этого, м², — уже квартал между улицами, а не пустота в узле.
pub(super) const MOUTH_MAX_AREA: f32 = 250.0;
/// Насколько глубоко в пустоте, м, вершина дорожки или проба середины пары
/// делают её не устьем: переход, что срезает остриё островка (Тула, R20 —
/// зебра по кончику треугольника), проходит у самой кромки.
const MOUTH_POINT_DEPTH: f32 = 1.0;
/// Пустота, которую асфальт узла уже замостил на эту долю, — асфальт
/// перекрёстка, а не устье: островок посреди него не нужен.
const MOUTH_PAVED_SHARE: f32 = 0.5;
/// Шаг проб этой доли, м.
const MOUTH_PAVED_PROBE: f32 = 1.0;
/// Допуск, с которым оси спрямляются перед объединением лент устьев, м.
const MOUTH_SIMPLIFY: f32 = 0.5;
/// Размыкание дыры, м: перемычка у́же двух таких делит её на куски.
const MOUTH_OPENING: f32 = 0.75;
/// Шаг сетки, которой вершины и контуры занятого ищут устья рядом, м.
const MOUTH_CELL: f32 = 32.0;

/// Устья (R20): пустоты, которые **со всех сторон зажаты проезжими частями**
/// — дыры объединения лент `carriageways` (ось и ширина) площадью от
/// [`GORE_MIN_AREA`] до [`MOUTH_MAX_AREA`]. В устье разделённой улицы, где
/// половины расходятся, а зазор замыкает связка, такая пустота — на земле
/// направляющий островок краской; у нас в ней оставалась голая земля цвета
/// тротуара (Тула, Красноармейский у площади Московского вокзала).
///
/// Не устье — пустота, где что-то **замаплено**: вершина контура `taken`
/// (дом, газон, стоянка, остров кольца, островок узла) внутри неё или её
/// вершина внутри контура, точка `points` (вершина дорожки — у островка
/// с тротуаром; проба середины пары — там газон разделительной) внутри неё.
pub(in crate::map::roads) fn mouths(
    carriageways: &[(&[Vec2], f32)],
    taken: &[&[Vec2]],
    points: &[Vec2],
    paved: &[&[Vec2]],
) -> Vec<Shape> {
    // стыки фаской: цена объединения — число точек, а скругление — десяток
    // точек на стык (как у карманов разбора). Торцы круглые, как у лент:
    // квадратный торец полотна на шве way выступал ступенькой в устье
    let style = StrokeStyle::new(1.0)
        .line_join(LineJoin::Bevel)
        .start_cap(LineCap::Round(ARC))
        .end_cap(LineCap::Round(ARC));
    let mut ribbons: Vec<Contour> = Vec::new();
    for (path, width) in carriageways {
        if path.len() >= 2 {
            let ring = is_ring(path);
            let path = &path[usize::from(ring)..];
            // и нарисованная ось — гладкая, с вершиной на каждые полметра
            // дуги: спрямлённая на [`MOUTH_SIMPLIFY`], она вдесятеро короче
            let points: Contour =
                crate::map::along::simplify(path, ring, MOUTH_SIMPLIFY, |_| false)
                    .into_iter()
                    .map(|index| path[index].to_array())
                    .collect();
            let style = StrokeStyle {
                width: *width,
                ..style.clone()
            };
            ribbons.extend(points.stroke(style, ring).into_iter().flatten());
        }
    }
    let mut holes: Vec<Vec<Vec2>> = ribbons
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .flat_map(|shape| shape.into_iter().skip(1))
        .filter(|hole| (GORE_MIN_AREA..=2.0 * MOUTH_MAX_AREA).contains(&contour_area(hole)))
        .map(|hole| vec![oriented(&ring_of(&hole), true)])
        .collect::<Vec<Shape>>()
        // щель между половинами, что идут вплотную, — та же дыра, что и
        // устье, из которого они расходятся: размыкание отделяет одно от
        // другого, а щель остаётся разделительной пары
        .outline(&OutlineStyle::new(-MOUTH_OPENING).line_join(LineJoin::Round(ARC)))
        .outline(&OutlineStyle::new(MOUTH_OPENING).line_join(LineJoin::Round(ARC)))
        .into_iter()
        .filter_map(|shape| shape.into_iter().next())
        .filter(|hole| (GORE_MIN_AREA..=MOUTH_MAX_AREA).contains(&contour_area(hole)))
        .map(|hole| ring_of(&hole))
        .collect();
    if holes.is_empty() {
        return Vec::new();
    }
    let mut grid = crate::map::grid::Grid::new(MOUTH_CELL);
    for (index, hole) in holes.iter().enumerate() {
        let (low, high) = ring_bounds(hole);
        grid.insert(low, high, index);
    }
    let mut kept = vec![true; holes.len()];
    for point in points {
        for &index in grid.at(*point) {
            let hole = &holes[index];
            if kept[index]
                && point_in_polygon(*point, hole)
                && hole
                    .iter()
                    .zip(hole.iter().cycle().skip(1))
                    .all(|(a, b)| distance_to_segment(*point, *a, *b) >= MOUTH_POINT_DEPTH)
            {
                kept[index] = false;
            }
        }
    }
    for ring in taken {
        let (low, high) = ring_bounds(ring);
        for index in grid.near(low, high) {
            let hole = &holes[index];
            if kept[index]
                && (ring.iter().any(|point| point_in_polygon(*point, hole))
                    || hole.iter().any(|point| point_in_polygon(*point, ring)))
            {
                kept[index] = false;
            }
        }
    }
    let mut paved_grid = crate::map::grid::Grid::new(MOUTH_CELL);
    for (index, outline) in paved.iter().enumerate() {
        let (low, high) = ring_bounds(outline);
        paved_grid.insert(low, high, index);
    }
    for (index, hole) in holes.iter().enumerate() {
        if kept[index] && paved_share(hole, paved, &paved_grid) >= MOUTH_PAVED_SHARE {
            kept[index] = false;
        }
    }
    let mut index = 0;
    holes.retain(|_| {
        index += 1;
        kept[index - 1]
    });
    holes
        .iter()
        .map(|hole| vec![oriented(hole, true)])
        .collect()
}

/// Устья без полос переходов `crossings` (ось зебры поперёк дороги), каждая
/// раздута на `half` по дороге: остриё устья отступает от зебры, а не
/// штрихуется поверх её полос.
pub(in crate::map::roads) fn clear_crossings(
    mouths: Vec<Shape>,
    crossings: &[(Vec2, Vec2)],
    half: f32,
) -> Vec<Shape> {
    if mouths.is_empty() || crossings.is_empty() {
        return mouths;
    }
    let mut kept = Vec::new();
    for mouth in mouths {
        let Some((low, high)) = mouth.first().map(contour_bounds) else {
            continue;
        };
        let bands: Vec<Contour> = crossings
            .iter()
            .filter(|(from, to)| {
                (from.min(*to) - half).cmple(high).all() && (from.max(*to) + half).cmpge(low).all()
            })
            .filter_map(|(from, to)| {
                let across = (*to - *from).try_normalize()?.perp() * half;
                Some(oriented(
                    &[*from - across, *to - across, *to + across, *from + across],
                    true,
                ))
            })
            .collect();
        if bands.is_empty() {
            kept.push(mouth);
            continue;
        }
        kept.extend(vec![mouth].overlay(&bands, OverlayRule::Difference, FillRule::NonZero));
    }
    kept
}

/// Доля пустоты `hole`, которую уже мостит асфальт узла `paved` (скругления
/// и носы — они ложатся до лент): по пробам через [`MOUTH_PAVED_PROBE`].
fn paved_share(hole: &[Vec2], paved: &[&[Vec2]], grid: &crate::map::grid::Grid<usize>) -> f32 {
    let (low, high) = ring_bounds(hole);
    let near = grid.near(low, high);
    let (mut inside, mut covered) = (0usize, 0usize);
    let steps = ((high - low) / MOUTH_PAVED_PROBE).ceil().as_uvec2();
    for i in 0..=steps.x {
        for j in 0..=steps.y {
            let probe = low + Vec2::new(i as f32, j as f32) * MOUTH_PAVED_PROBE;
            if !point_in_polygon(probe, hole) {
                continue;
            }
            inside += 1;
            if near
                .iter()
                .any(|&index| point_in_polygon(probe, paved[index]))
            {
                covered += 1;
            }
        }
    }
    if inside == 0 {
        return 0.0;
    }
    covered as f32 / inside as f32
}
