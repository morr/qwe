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
use crate::map::along::{arclengths, densify, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::{miter_offsets, ring_perimeter};
use crate::map::osm::model::{
    MapData, PolyArea, RoadLine, SidewalkSide, closest_on_segment, ring_area, sidewalk_band,
};
use crate::map::parallel::in_parallel;
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

/// Кусок покрытия: контуры `i_overlay` и их общий габарит — то, что кладётся
/// в сетку по габариту и достаётся из неё окном. Так ищут куски и покрытие
/// карманов (`pockets.rs`), и вырез обочин из кварталов.
pub(super) struct Cover {
    pub(super) contours: Vec<Contour>,
    pub(super) low: Vec2,
    pub(super) high: Vec2,
}

impl Cover {
    pub(super) fn of(contours: Vec<Contour>) -> Self {
        let (low, high) = contours.iter().map(contour_bounds).fold(
            (Vec2::INFINITY, Vec2::NEG_INFINITY),
            |(low, high), (from, to)| (low.min(from), high.max(to)),
        );
        Self {
            contours,
            low,
            high,
        }
    }

    /// Задевает ли габарит куска окно `low..high` (края включительно).
    pub(super) fn overlaps(&self, low: Vec2, high: Vec2) -> bool {
        self.low.cmple(high).all() && self.high.cmpge(low).all()
    }
}

/// Шаг, с которым сторона улицы ищет вдоль себя отдельную дорожку, м.
const SEPARATE_PROBE_STEP: f32 = 5.0;
/// Как далеко за внешним краем полосы тротуара ещё лежит «её» дорожка, м: за
/// газоном в пару метров, но не через дом.
pub(super) const SEPARATE_REACH: f32 = 4.0;
/// Насколько дорожка может заходить на проезжую часть от кромки, м: `footway`
/// бывает замаплен прямо по бордюру, и его ось тогда чуть внутри ленты.
const SEPARATE_INSIDE: f32 = 1.0;
/// Какой газон, м, должен лежать между кромкой проезжей части и ближним краем
/// дорожки, чтобы полоса ушла. Ближе — полоса остаётся и лежит под дорожкой:
/// снятая, она оставляла между бордюром и дорожкой щель земли в метр, а у
/// угла — дыру до земли в рамке скруглений (Тула, витрины 15 и 21). На земле
/// от бордюра до плитки — тоже тротуар.
const SEPARATE_LAWN: f32 = 1.5;
/// Как далеко от кромки ещё лежит дорожка, до которой мостится обочина
/// ([`RoadLine::verges`]), м: дальше «её» полосы ([`SEPARATE_REACH`]) — у
/// проспектов центра Тулы тротуар идёт за газоном в шесть-восемь метров, и
/// голая земля между ними была дырой на весь квартал (витрина 02).
const VERGE_REACH: f32 = 10.0;
/// То же у двусторонней улицы, м: у Фрунзе в Туле (витрина 02) дорожка идёт
/// в 8–15 м от кромки, и в 10 м её находила только половина проб — сторона
/// оставалась без обочины, и между полосой тротуара и дорожкой лежала голая
/// земля. У половины разделённой улицы дальше [`VERGE_REACH`] по внутренней
/// стороне — уже дорожки за встречной половиной, и обочина мостила бы
/// разделительную с чужим полотном.
const VERGE_REACH_TWO_WAY: f32 = 16.0;
/// Косинус угла, от которого дорожка считается идущей вдоль улицы.
const SEPARATE_PARALLEL: f32 = 0.85;
/// Косинус угла, от которого косое звено дорожки ещё ведёт профиль обочины
/// ([`RoadLine::verge_profile`]), но не считается дорожкой вдоль: у угла
/// перекрёстка тротуар заворачивает под 30–50° к улице, и обочина по
/// параллельным звеньям обрывалась там, где он начинал уходить.
const VERGE_SLANT: f32 = 0.5;
/// Доля проб стороны, у которых нашлась дорожка, чтобы полоса ушла.
const SEPARATE_SHARE: f32 = 0.6;
/// Клетка индекса дорожек, м.
const SEPARATE_CELL: f32 = 40.0;

/// Что решил [`measure_footways_beside_streets`]: сколько сторон без тега
/// было спрошено и сколько из них отдано отдельной дорожке.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct SeparateSidewalks {
    pub(super) asked: usize,
    pub(super) dropped: usize,
    pub(super) took: std::time::Duration,
}

impl std::fmt::Display for SeparateSidewalks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            asked,
            dropped,
            took,
        } = self;
        write!(
            f,
            "osm parse: {dropped} of {asked} untagged sidewalk sides left to a separately mapped footway in {took:?}"
        )
    }
}

/// Дорожка вдоль улицы: тротуар без тега ей отдаётся, обочина до неё меряется.
///
/// OSM рисует тротуар двумя способами: тегом `sidewalk*` на улице или
/// отдельным `footway` (обычно `footway=sidewalk`) вдоль кромки — и тогда на
/// улице ставят `sidewalk=separate`. Ставят не всегда: у юго-восточной
/// половины Ленина в Туле тега нет вовсе, и полоса по правилу ложилась рядом с
/// мощёной дорожкой в метре от неё — два параллельных тротуара со щелью травы
/// на всю длину проспекта (разведка A1, примеры 01 и 02).
///
/// Сторона без тега ([`SidewalkSide::Inferred`]) у всякой проезжей части
/// отдаётся дорожке, если на [`SEPARATE_SHARE`] проб вдоль неё (шаг
/// [`SEPARATE_PROBE_STEP`]) с **этой** стороны идёт мощёная дорожка
/// ([`RoadLine::is_paved_path`]) — параллельно ([`SEPARATE_PARALLEL`]), осью от
/// [`SEPARATE_INSIDE`] внутри кромки до [`SEPARATE_REACH`] за внешним краем
/// полосы, — и между кромкой и ближним краем дорожки (медиана по пробам) лежит
/// газон не уже [`SEPARATE_LAWN`]. Грунтовая тропинка тротуаром не считается:
/// песчаная лента вместо полосы была бы хуже дубля. Тег (`Tagged`) не
/// трогается — его ставил человек.
///
/// Всякая сторона мощёной улицы с такой дорожкой вдоль (до [`VERGE_REACH`]
/// от кромки, у двусторонней — до [`VERGE_REACH_TWO_WAY`]) получает **обочину** ([`RoadLine::verges`]) — от кромки до оси
/// дорожки, с полосой тротуара или без: газон между ними рисуется газоном, а
/// голая земля — плиткой (`roads.rs`, слой под зеленью).
pub(super) fn measure_footways_beside_streets(roads: &mut [RoadLine]) -> SeparateSidewalks {
    let started = std::time::Instant::now();
    // звено дорожки и её полуширина
    let mut links: Vec<(Vec2, Vec2, f32)> = Vec::new();
    let mut index: Grid<u32> = Grid::new(SEPARATE_CELL);
    for road in roads.iter().filter(|road| road.is_paved_path()) {
        for pair in road.points.windows(2) {
            index.insert_segment(pair[0], pair[1], 0.0, links.len() as u32);
            links.push((pair[0], pair[1], road.width / 2.0));
        }
    }
    let mut report = SeparateSidewalks::default();
    if links.is_empty() {
        return report;
    }
    for road in roads.iter_mut() {
        let paved = !road.is_unpaved_street();
        let asks = paved || road.sidewalks.contains(&SidewalkSide::Inferred);
        if !road.is_carriageway() || !asks {
            continue;
        }
        let half = road.width / 2.0;
        let near = (half - SEPARATE_INSIDE).max(0.0);
        let far = half + sidewalk_band(road.width) + SEPARATE_REACH;
        // обочина тянется и к дорожке дальше «её» полосы
        let verge_reach = if road.oneway {
            VERGE_REACH
        } else {
            VERGE_REACH_TWO_WAY
        };
        let reach = far.max(half + verge_reach);
        let (along, total) = arclengths(&road.points);
        let mut probes = 0;
        // по сторонам — ближайшая дорожка каждой пробы: ось и полуширина
        let mut hits: [Vec<(f32, f32)>; 2] = [Vec::new(), Vec::new()];
        // профиль обочины: где по точкам стояла проба и как далеко от неё
        // дорожка — и косая, заворачивающая у угла
        let mut places: [Vec<(f32, f32)>; 2] = [Vec::new(), Vec::new()];
        let mut at = SEPARATE_PROBE_STEP / 2.0;
        while at < total {
            if let Some((point, direction)) = place_on_path(&road.points, &along, at) {
                probes += 1;
                let mut found: [Option<(f32, f32)>; 2] = [None; 2];
                // косое звено — только профилю: у угла дорожка заворачивает
                let mut slanted: [Option<f32>; 2] = [None; 2];
                // повтор звена из соседней клетки безвреден — берётся ближайшее
                for &link in index.near_each(point - reach, point + reach) {
                    let (from, to, path_half) = links[link as usize];
                    let heading = (to - from).normalize_or_zero();
                    let parallel = heading.dot(direction).abs();
                    if parallel < VERGE_SLANT {
                        continue;
                    }
                    let offset = closest_on_segment(point, from, to) - point;
                    let distance = offset.length();
                    if (near..=reach).contains(&distance) {
                        // `perp` смотрит влево по ходу точек — сторона 0
                        let at_side = usize::from(offset.dot(direction.perp()) < 0.0);
                        if parallel < SEPARATE_PARALLEL {
                            let side = &mut slanted[at_side];
                            if side.is_none_or(|nearest| distance < nearest) {
                                *side = Some(distance);
                            }
                            continue;
                        }
                        let side = &mut found[at_side];
                        if side.is_none_or(|(nearest, _)| distance < nearest) {
                            *side = Some((distance, path_half));
                        }
                    }
                }
                for (((hits, places), found), slanted) in
                    hits.iter_mut().zip(&mut places).zip(found).zip(slanted)
                {
                    if let Some(found) = found {
                        hits.push(found);
                    }
                    if let Some(distance) = found.map(|(distance, _)| distance).or(slanted) {
                        places.push((at, distance));
                    }
                }
            }
            at += SEPARATE_PROBE_STEP;
        }
        for (((side, verge), profile), (mut hits, places)) in road
            .sidewalks
            .iter_mut()
            .zip(road.verges.iter_mut())
            .zip(road.verge_profile.iter_mut())
            .zip(hits.into_iter().zip(places))
        {
            let enough =
                |count: usize| probes > 0 && count as f32 >= SEPARATE_SHARE * probes as f32;
            if *side == SidewalkSide::Inferred {
                report.asked += 1;
                // газон до ближнего края дорожки — медиана по пробам «её» полосы
                let mut lawns: Vec<f32> = hits
                    .iter()
                    .filter(|(at, _)| *at <= far)
                    .map(|(at, path)| at - path - half)
                    .collect();
                lawns.sort_by(f32::total_cmp);
                if enough(lawns.len()) && lawns[lawns.len() / 2] >= SEPARATE_LAWN {
                    *side = SidewalkSide::None;
                    report.dropped += 1;
                }
            }
            if paved && enough(hits.len()) {
                // до оси дорожки у каждой пробы — профиль, медиана — постоянная
                *profile = places
                    .iter()
                    .map(|&(at, distance)| (at, (distance - half).max(0.0)))
                    .collect();
                hits.sort_by(|a, b| a.0.total_cmp(&b.0));
                *verge = (hits[hits.len() / 2].0 - half).max(0.0);
            }
        }
    }
    report.took = started.elapsed();
    report
}

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
    let pieces: Vec<Cover> = map
        .roads
        .iter()
        .filter(|road| !road.bridge && !road.passage && road.points.len() >= 2)
        .flat_map(|road| verge_rings(road, cut_reach))
        .map(|ring| Cover::of(vec![oriented(&ring, true)]))
        .collect();
    if pieces.is_empty() {
        return 0;
    }
    let mut grid = Grid::new(CELL);
    for (index, piece) in pieces.iter().enumerate() {
        grid.insert(piece.low, piece.high, index);
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
fn cut_block(block: &PolyArea, pieces: &[Cover], grid: &Grid<usize>) -> Option<Vec<PolyArea>> {
    if block.outer.len() < 3 {
        return None;
    }
    let Cover {
        contours: subject,
        low,
        high,
    } = Cover::of(area_contours(block));
    let clip: Vec<Contour> = grid
        .near(low, high)
        .into_iter()
        .map(|index| &pieces[index])
        .filter(|piece| piece.overlaps(low, high))
        .flat_map(|piece| piece.contours.iter().cloned())
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
    let perimeter = ring_perimeter(&ring_of(outer));
    perimeter > 0.0 && 2.0 * net_area(shape) / perimeter < SLIVER_WIDTH_MAX
}
