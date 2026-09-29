//! **Обочина — не двор**: квартал, заведённый на обочину улицы, из неё
//! вырезается — площадью, а не вершинами.
//!
//! Обочина ([`RoadLine::verge_at`], кромка — ось дорожки вдоль улицы) —
//! плитка, и рисуется она **под** кварталами (`Z_ROAD_VERGE` < `Z_LANDUSE`),
//! чтобы замапленный газон оставался газоном. Двор, залезший на неё, торчит
//! из плитки тёмным пятном. Правила подтяжки вершин
//! ([`super::pull_areas_to_roads`]) это знают — край между дорожкой и улицей
//! уходит под дорожку, вершина на голой стороне — к ближнему нарисованному
//! краю, — но решают по **вершинам**, а край между двумя решёнными вершинами
//! прямой: одна вершина у развилки дорожек в равной дали от кромки и от
//! дорожки (0.47 и 0.48 м) ушла под асфальт, соседи — под дорожку, и ребро
//! между ними оставило щепку двора 2 × 0.3 м на плитке (Тула, витрина 15).
//!
//! Здесь тот же вопрос задан **площади**: квартал после подтяжки
//! пересекается с полосами плитки обочин — от оси улицы до кромки плитки без
//! [`VERGE_CUT_INSET`] ([`cut_reach`]), — и из него вычитаются те куски
//! пересечения, что **тонки** ([`is_sliver`]). Кромка узкой обочины — ось
//! дорожки, так что новый край квартала ложится под её ленту. Широкий кусок
//! остаётся: квартал, нарисованный до бордюра полосой в три метра, — газон
//! между кромкой и тротуаром, так он и читается (Советская, та же витрина).

use bevy::math::Vec2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use super::LANDUSE_OVERLAP;
use super::pockets::in_parallel;
use crate::map::along::{arclengths, densify};
use crate::map::grid::Grid;
use crate::map::meshing::miter_offsets;
use crate::map::osm::model::{MapData, PolyArea, RoadLine, ring_area};
use crate::map::roads::paved_verge;
use crate::map::shapes::{Contour, area_contours, contour_bounds, oriented, ring_of};

/// Шаг точек обочины, м: вдвое реже рисунка (`roads.rs`, 2.5 м) — профиль
/// обочины ([`RoadLine::verge_profile`]) и так снят пробами через пять
/// метров, а точки — цена булевой операции.
const VERGE_STEP: f32 = 5.0;
/// Точек обочины в одном куске: кусок берётся по габариту, и длинная улица не
/// тащит к каждому кварталу свои сотни метров.
const VERGE_RUN: usize = 16;
/// Насколько полоса выреза уже обочины, м. Кромка обочины — ось дорожки, а
/// лента рисуется по сглаженной оси: вырез точно по кромке мог бы открыть
/// нить голой земли между двором и плиткой там, где сглаживание увело
/// рисунок внутрь. Полметра двора под дорожкой не видны.
const VERGE_CUT_INSET: f32 = LANDUSE_OVERLAP;
/// Меньше этого, м², кусок квартала после выреза — не двор, а щепка на
/// плитке или под асфальтом: отбрасывается.
const MIN_BLOCK_PART: f32 = 1.0;
/// Меньше этого, м², квартал потерял — не вырез, а шум обводки: кольца
/// остаются как были, без переобводки.
const CUT_EPSILON: f32 = 0.01;
/// Уже этого в среднем, м, кусок двора на плитке обочины — щепка
/// ([`is_sliver`]) и вырезается. Щепка Тулы 15 — около 0.3 м; полоса газона
/// между кромкой и дорожкой на Советской рядом — три метра, она остаётся.
const SLIVER_WIDTH_MAX: f32 = 1.0;
/// Ячейка сетки кусков обочин, м.
const CELL: f32 = 64.0;

/// Полосы обочин дороги — от оси до кромки плюс `reach(обочина по месту)`,
/// кусками по [`VERGE_RUN`] точек. Та же постройка, что у рисунка
/// (`roads.rs::push_verges`), только по сырым точкам. Её берут покрытие
/// карманов (`pockets.rs`, обочина во всю ширину) и вырез из кварталов
/// (плитка обочины, [`cut_reach`]).
pub(super) fn verge_rings(road: &RoadLine, reach: impl Fn(f32) -> f32) -> Vec<Vec<Vec2>> {
    let half = road.width / 2.0;
    let raw = arclengths(&road.points).1;
    let mut rings = Vec::new();
    for side in 0..2 {
        if road.verges[side] <= 0.0 {
            continue;
        }
        let dense = densify(&road.points, VERGE_STEP);
        if dense.len() < 2 {
            continue;
        }
        let (along, total) = arclengths(&dense);
        let scale = raw / total.max(f32::EPSILON);
        // `miter_offsets` плюсом сдвигает влево — сторона 0
        let sign = if side == 0 { 1.0 } else { -1.0 };
        let normals = miter_offsets(&dense, false, sign);
        let outer: Vec<Vec2> = dense
            .iter()
            .zip(&normals)
            .zip(&along)
            .map(|((&point, &normal), &at)| {
                point + normal * (half + reach(road.verge_at(side, at * scale)))
            })
            .collect();
        let last = dense.len() - 1;
        for start in (0..last).step_by(VERGE_RUN - 1) {
            let end = (start + VERGE_RUN - 1).min(last);
            let ring: Vec<Vec2> = outer[start..=end]
                .iter()
                .chain(dense[start..=end].iter().rev())
                .copied()
                .collect();
            if ring_area(&ring) > 0.0 {
                rings.push(ring);
            }
        }
    }
    rings
}

/// Сколько обочины шириной `verge` вырезается из кварталов, м: её плитка
/// ([`paved_verge`]) без [`VERGE_CUT_INSET`] у дорожки. Газон широкой
/// обочины — трава двора (`roads.rs::VERGE_YARD_COLOR`), и двор над ним
/// ничего не портит; вырезать его — значит перекрасить заводской квартал.
fn cut_reach(verge: f32) -> f32 {
    paved_verge(verge).min(verge - VERGE_CUT_INSET).max(0.0)
}

/// Вырезать щепки двора с плитки обочин из кварталов (`MapData::landuse`).
/// Квартал без щепок остаётся теми же кольцами; задетый — переобводится,
/// и если вырез разрезал его на части, каждая часть — своя площадь того же
/// вида (кусок меньше [`MIN_BLOCK_PART`] отбрасывается). Возвращает, сколько
/// кварталов вырез задел.
///
/// Кварталы друг от друга не зависят и режутся по потокам, как карманы;
/// порядок результата — порядок кварталов.
pub(super) fn cut_verges_from_blocks(map: &mut MapData) -> usize {
    let pieces: Vec<(Contour, Vec2, Vec2)> = map
        .roads
        .iter()
        .filter(|road| !road.bridge && !road.passage && road.points.len() >= 2)
        .flat_map(|road| verge_rings(road, cut_reach))
        .map(|ring| {
            let contour = oriented(&ring, true);
            let (low, high) = contour_bounds(&contour);
            (contour, low, high)
        })
        .collect();
    if pieces.is_empty() {
        return 0;
    }
    let mut grid = Grid::new(CELL);
    for (index, (_, low, high)) in pieces.iter().enumerate() {
        grid.insert(*low, *high, index);
    }
    let cut: Vec<Option<Vec<PolyArea>>> =
        in_parallel(&map.landuse, |block| cut_block(block, &pieces, &grid));
    let mut touched = 0;
    let blocks = std::mem::take(&mut map.landuse);
    for (block, cut) in blocks.into_iter().zip(cut) {
        match cut {
            Some(parts) => {
                touched += 1;
                map.landuse.extend(parts);
            }
            None => map.landuse.push(block),
        }
    }
    touched
}

/// Квартал без обочин под ним — частями; `None` — обочины его не задели.
fn cut_block(
    block: &PolyArea,
    pieces: &[(Contour, Vec2, Vec2)],
    grid: &Grid<usize>,
) -> Option<Vec<PolyArea>> {
    if block.outer.len() < 3 {
        return None;
    }
    let subject = area_contours(block);
    let (low, high) = subject.iter().map(contour_bounds).fold(
        (Vec2::INFINITY, Vec2::NEG_INFINITY),
        |(low, high), (a, b)| (low.min(a), high.max(b)),
    );
    let clip: Vec<Contour> = grid
        .near(low, high)
        .into_iter()
        .map(|index| &pieces[index])
        .filter(|(_, from, to)| from.cmple(high).all() && to.cmpge(low).all())
        .map(|(contour, _, _)| contour.clone())
        .collect();
    if clip.is_empty() {
        return None;
    }
    // двор на плитке — кусками; вырезаются только щепки
    let slivers: Vec<Contour> = subject
        .overlay(&clip, OverlayRule::Intersect, FillRule::NonZero)
        .into_iter()
        .filter(|shape| is_sliver(shape))
        .flatten()
        .collect();
    if slivers.is_empty() {
        return None;
    }
    let shapes = subject.overlay(&slivers, OverlayRule::Difference, FillRule::NonZero);
    let before = net_area(&subject);
    let after: f32 = shapes.iter().map(|shape| net_area(shape)).sum();
    if before - after < CUT_EPSILON {
        return None;
    }
    Some(
        shapes
            .iter()
            .filter(|shape| net_area(shape) >= MIN_BLOCK_PART)
            .map(|shape| PolyArea {
                outer: ring_of(&shape[0]),
                holes: shape[1..].iter().map(ring_of).collect(),
                ..block.clone()
            })
            .collect(),
    )
}

/// Площадь фигуры `i_overlay` за вычетом её дыр, м².
fn net_area(shape: &[Contour]) -> f32 {
    shape
        .iter()
        .enumerate()
        .map(|(index, contour)| {
            let area = ring_area(&ring_of(contour));
            if index == 0 { area } else { -area }
        })
        .sum()
}

/// Щепка ли кусок двора на плитке обочины: средняя ширина (две площади на
/// периметр) уже [`SLIVER_WIDTH_MAX`]. Широкая полоса двора между кромкой и
/// дорожкой — это газон по данным, квартал нарисован до бордюра нарочно, и
/// читается он газоном, а не пятном; щепка и нить вдоль дорожки — нет.
fn is_sliver(shape: &[Contour]) -> bool {
    let Some(outer) = shape.first() else {
        return false;
    };
    let ring = ring_of(outer);
    let perimeter: f32 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(from, to)| from.distance(*to))
        .sum();
    perimeter > 0.0 && 2.0 * net_area(shape) / perimeter < SLIVER_WIDTH_MAX
}
