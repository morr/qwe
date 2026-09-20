//! Дороги на большой стоянке — бордюром и разметкой **поверх** её асфальта.
//!
//! Стоянка лежит выше дорожных лент (`Z_PARKING` над `Z_ROAD`) и кроет всё,
//! что на неё заходит. Для двора это верно: асфальт площадки и есть проезд. У
//! большой стоянки ([`parking::is_ground`]) сквозь площадку идёт настоящая
//! дорога ([`parking::is_through`]) — у ТРЦ «Макси» бульвар с односторонним
//! движением и тремя кольцами, — и спрятанная, она оставляла восемь гектаров
//! штриховки без единого ориентира (отчёт автора).
//!
//! Асфальт у такой дороги тот же, что у площадки, и второй раз не кладётся:
//! дорогу на площадке показывает её **бордюр** ([`parking::kerb_width`], слой
//! `lot_sidewalks`). Он считается **полигоном, а не лентами**: полосы сквозных
//! дорог с бордюром, минус асфальт всех улиц площадки (проезд ряда прорезает в
//! бордюре устье, и вдоль бульвара тот выходит островками у торцов рядов),
//! в пределах её контура. Лентами он был сперва — асфальт каждой дороги лежал
//! вторым слоем поверх бордюров всех остальных, — и у колец, где съезды
//! расходятся веером, ленты рубили друг друга в обрывки с рваными торцами, а
//! островок кольца оставался кольцом (отчёт автора). У булевой разности рваных
//! краёв нет, остров кольца залит, а обрезки у́же двух [`KERB_OPENING`] снимает
//! размыкание.
//!
//! Между двумя встречными полотнами, идущими бок о бок, бордюра нет: там
//! **двойная сплошная** (слой `lot_lines`), как рисуют Яндекс и 2ГИС, — см.
//! [`medians`].

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};

use super::gores::Gores;
use super::{RoadStyle, SIDEWALK_COLOR};
use crate::map::meshing::{MeshBuilder, RibbonJoin};
use crate::map::osm::model::{
    MapData, PolyArea, RoadLine, distance_to_segment, point_in_area, ring_bounds, signed_ring_area,
};
use crate::map::parking::{is_ground, is_through, kerb_width};

/// Шаг, которым ось дороги ощупывается на «внутри ли площадки» и на соседа
/// через разделительную, м.
const PROBE_STEP: f32 = 2.0;
/// Скругление офсетов: длина хорды в долях радиуса (`LineJoin::Round`).
pub(super) const ARC: f32 = 0.3;
/// Размыкание бордюра, м: обрезок у́же двух таких снимается, углы скругляются.
/// Бордюр сам 1.2 м и больше — ему это ничего не стоит.
const KERB_OPENING: f32 = 0.3;
/// Обрезок бордюра мельче этого, м², не рисуется: островок у торца ряда — от
/// четырнадцати, а мельче выходят огрызки там, где дорогу режет кромка площадки.
const MIN_KERB_AREA: f32 = 6.0;
/// На сколько бордюр выпущен за контур площадки, м, — до тротуара улицы, с
/// которой дорога на неё заходит.
const KERB_OVERHANG: f32 = 2.0;
/// На сколько осевая дотягивается до острия островка у кольца, м, и шаг, которым
/// оно ищется ([`reach_gore`]).
const MEDIAN_REACH: f32 = 8.0;
const MEDIAN_REACH_STEP: f32 = 0.25;
/// Зазор между торцом двойной линии и остриём штриховки, м.
const MEDIAN_GORE_GAP: f32 = 0.6;
/// Осевую пары полотен считает то из них, от которого сосед лежит в эту
/// сторону, — ровно одно из двух ([`medians`]).
const MEDIAN_SIDE: Vec2 = Vec2::new(1.0, 0.618);
/// Сколько асфальта между кромками двух полотен ещё считается разделительной,
/// м: шире — уже остров с бордюром.
const MEDIAN_GAP: f32 = 3.0;
/// На сколько полотна могут заходить друг на друга, оставаясь двумя дорогами с
/// разделительной, м: дальше это слияние полос, и линии посреди него нет.
const MEDIAN_OVERLAP: f32 = 1.0;
/// Кусок разделительной короче этого, м, — не она: так сходятся два съезда.
const MEDIAN_MIN: f32 = 8.0;
/// Насколько соседнее полотно идёт «рядом», а не «навстречу торцом»: косинус
/// угла между осями и доля расстояния, на которую сосед смещён вдоль оси.
const MEDIAN_PARALLEL: f32 = 0.9;
const MEDIAN_SKEW: f32 = 0.35;
/// Двойная сплошная: ширина линии и расстояние между осями линий, м. Шире
/// настоящей (0.15 и 0.3) — иначе на отдалении обе сливаются в волосок.
const DOUBLE_LINE_WIDTH: f32 = 0.2;
const DOUBLE_LINE_GAUGE: f32 = 0.5;
const DOUBLE_LINE_COLOR: Color = Color::srgb(0.88, 0.88, 0.86);

pub(super) type Contour = Vec<[f32; 2]>;
pub(super) type Shape = Vec<Contour>;

/// Улица у большой стоянки — так, как она нарисована (сглаженная ось).
struct Street {
    path: Vec<Vec2>,
    width: f32,
    /// Бордюр с одной стороны — у сквозной дороги, чья ось зашла на площадку.
    kerb: Option<f32>,
    /// Одностороннее полотно, не кольцо: у такого бывает разделительная.
    carriageway: bool,
}

struct Ground<'a> {
    lot: &'a PolyArea,
    low: Vec2,
    high: Vec2,
    streets: Vec<Street>,
}

/// Большие стоянки карты и улицы, что на них заходят.
pub(super) struct Grounds<'a>(Vec<Ground<'a>>);

/// Два слоя поверх асфальта стоянки: бордюры сквозных дорог и двойная сплошная
/// между встречными полотнами.
pub(super) struct LotLayers {
    pub sidewalks: MeshBuilder,
    pub lines: MeshBuilder,
}

impl<'a> Grounds<'a> {
    pub fn of(map: &'a MapData) -> Self {
        Self(
            map.parking
                .iter()
                .filter(|lot| is_ground(lot))
                .map(|lot| {
                    let (low, high) = ring_bounds(&lot.outer);
                    Ground {
                        lot,
                        low,
                        high,
                        streets: Vec::new(),
                    }
                })
                .collect(),
        )
    }

    /// Улица `road`, нарисованная по `path`, — каждой большой стоянке, до
    /// которой она достаёт.
    pub fn push(&mut self, road: &RoadLine, path: &[Vec2]) {
        if self.0.is_empty() || path.len() < 2 {
            return;
        }
        let kerb = is_through(road).then(|| kerb_width(road));
        let reach = road.width / 2.0 + kerb.unwrap_or_default();
        // замкнутость — по сырым точкам OSM: сглаживание срезает угол на шве
        // замкнутого way, и концы нарисованного кольца расходятся
        let mut closed = path.to_vec();
        if let (true, Some(first)) = (is_ring(&road.points), closed.first().copied())
            && closed
                .last()
                .is_some_and(|last| last.distance(first) > 0.01)
        {
            closed.push(first);
        }
        let path = closed.as_slice();
        let (low, high) = ring_bounds(path);
        for ground in &mut self.0 {
            if low.cmpgt(ground.high + reach).any() || high.cmplt(ground.low - reach).any() {
                continue;
            }
            // бордюр — только дороге, чья ось идёт по площадке: у улицы вдоль
            // кромки он свой, тротуаром, и на площадку не заходит
            let inside = kerb.is_some() && enters(path, ground.lot);
            ground.streets.push(Street {
                path: path.to_vec(),
                width: road.width,
                kerb: kerb.filter(|_| inside),
                carriageway: inside && road.oneway && !road.roundabout && !is_ring(path),
            });
        }
    }

    pub fn layers(&self, style: &RoadStyle, gores: &Gores) -> LotLayers {
        let mut layers = LotLayers {
            sidewalks: MeshBuilder::with_surface_coords(),
            lines: MeshBuilder::default(),
        };
        for ground in &self.0 {
            if !ground.streets.iter().any(|street| street.kerb.is_some()) {
                continue;
            }
            let mut medians = medians(&ground.streets, ground.lot);
            for (midline, _) in &mut medians {
                reach_gore(midline, gores);
            }
            medians.retain(|(midline, _)| midline.len() >= 2);
            if style.markings {
                for (midline, _) in &medians {
                    layers.lines.push_rails(
                        midline,
                        DOUBLE_LINE_GAUGE,
                        DOUBLE_LINE_WIDTH,
                        DOUBLE_LINE_COLOR.to_linear(),
                        RibbonJoin::Round,
                    );
                }
            }
            if style.sidewalks {
                for shape in kerbs(ground, &medians, gores) {
                    push_shape(&mut layers.sidewalks, shape, SIDEWALK_COLOR.to_linear());
                }
            }
        }
        layers
    }
}

pub(super) fn push_shape(builder: &mut MeshBuilder, shape: Shape, color: LinearRgba) {
    let mut rings = shape.into_iter().map(|contour| {
        contour
            .into_iter()
            .map(Vec2::from_array)
            .collect::<Vec<Vec2>>()
    });
    let Some(outer) = rings.next() else {
        return;
    };
    let holes: Vec<Vec<Vec2>> = rings.collect();
    builder.push_polygon(&outer, &holes, color);
}

/// Бордюры площадки — полосы сквозных дорог и острова колец, минус асфальт
/// всех улиц, разделительные и **направляющие островки** у колец
/// (`roads/gores.rs`: там асфальт со штриховкой, бордюра между полотнами нет),
/// разомкнутые на [`KERB_OPENING`].
fn kerbs(ground: &Ground, medians: &[(Vec<Vec2>, f32)], gores: &Gores) -> Vec<Shape> {
    let mut bands: Vec<Contour> = Vec::new();
    let mut asphalt: Vec<Contour> = Vec::new();
    let mut islands: Vec<Contour> = Vec::new();
    for street in &ground.streets {
        let band = stroke(&street.path, street.width, LineCap::Round(ARC));
        if let Some(kerb) = street.kerb {
            bands.extend(stroke(
                &street.path,
                street.width + 2.0 * kerb,
                LineCap::Round(ARC),
            ));
            // остров кольца — весь, а не ободком вдоль полотна
            if is_ring(&street.path) {
                islands.push(oriented(&street.path[1..], true));
            }
        }
        asphalt.extend(band);
    }
    for (midline, width) in medians {
        asphalt.extend(stroke(midline, *width, LineCap::Butt));
    }
    let lot: Vec<Contour> = std::iter::once(oriented(&ground.lot.outer, true))
        .chain(ground.lot.holes.iter().map(|hole| oriented(hole, false)))
        .collect();
    let round = LineJoin::Round(ARC);
    // бордюр выпущен за контур на [`KERB_OVERHANG`]: обрезанный ровно по
    // нему, он вставал к тротуару улицы, с которой дорога заходит на
    // площадку, ступенькой — контур там скруглён замыканием, а тротуар нет
    let reach = vec![lot].outline(&OutlineStyle::new(KERB_OVERHANG).line_join(round.clone()));
    let mut cut = asphalt;
    cut.extend(gores.contours().cloned());
    bands.extend(islands);
    bands
        .overlay(&cut, OverlayRule::Difference, FillRule::NonZero)
        .overlay(&reach, OverlayRule::Intersect, FillRule::NonZero)
        .outline(&OutlineStyle::new(-KERB_OPENING).line_join(round.clone()))
        .outline(&OutlineStyle::new(KERB_OPENING).line_join(round))
        .into_iter()
        .filter(|shape| {
            shape
                .first()
                .is_some_and(|outer| contour_area(outer) >= MIN_KERB_AREA)
        })
        .collect()
}

/// Дотянуть осевую до островка у кольца, если он в пределах [`MEDIAN_REACH`] по
/// её ходу.
///
/// Осевая кончается там, где полотна перестают идти бок о бок, а клин
/// штриховки — там, где зазор между ними сходит на нет, и между остриём клина
/// и двойной линией оставалось метра три голого асфальта (отчёт автора). На
/// земле края островка **сходятся в** двойную сплошную.
fn reach_gore(midline: &mut Vec<Vec2>, gores: &Gores) {
    // осевая считается, пока между полотнами до [`MEDIAN_GAP`] асфальта, клин —
    // пока их от 0.6 м: в промежутке обе есть разом, и двойная линия уезжала
    // внутрь штриховки (отчёт автора). Концы, попавшие в клин, срезаются, и
    // дотягивается осевая уже от чистого места
    while midline.last().is_some_and(|point| gores.contains(*point)) {
        midline.pop();
    }
    let inside = midline
        .iter()
        .take_while(|point| gores.contains(**point))
        .count();
    midline.drain(..inside);
    for end in [false, true] {
        let count = midline.len();
        if count < 2 {
            return;
        }
        let (tip, before) = if end {
            (midline[count - 1], midline[count - 2])
        } else {
            (midline[0], midline[1])
        };
        let Some(heading) = (tip - before).try_normalize() else {
            continue;
        };
        let steps = (MEDIAN_REACH / MEDIAN_REACH_STEP) as usize;
        // сколько по ходу до штриховки; вплотную линия не подводится — между её
        // торцом и остриём клина остаётся [`MEDIAN_GORE_GAP`] (просьба автора:
        // встык торец двойной линии сливался с обводкой островка)
        let Some(to_gore) = (1..=steps)
            .map(|step| step as f32 * MEDIAN_REACH_STEP)
            .find(|reach| gores.contains(tip + heading * *reach))
        else {
            continue;
        };
        let point = tip + heading * (to_gore - MEDIAN_GORE_GAP);
        match (to_gore > MEDIAN_GORE_GAP, end) {
            // до клина дальше зазора — линия дотягивается, не доходя на зазор
            (true, true) => midline.push(point),
            (true, false) => midline.insert(0, point),
            // клин ближе зазора — торец отодвигается назад
            (false, true) => midline[count - 1] = point,
            (false, false) => midline[0] = point,
        }
    }
}

pub(super) fn contour_area(contour: &Contour) -> f32 {
    let ring: Vec<Vec2> = contour.iter().copied().map(Vec2::from_array).collect();
    signed_ring_area(&ring).abs()
}

/// Разделительные площадки: осевая между двумя встречными полотнами и ширина
/// полосы между их осями.
///
/// Бульвар в OSM — два односторонних way бок о бок, и между их полотнами
/// остаётся полметра-метр. Бордюры обоих сливались там в светлую нитку во всю
/// длину бульвара; на снимке же это один асфальт с двойной сплошной. Сосед
/// ищется от каждой точки оси: другое одностороннее полотно, идущее **рядом**
/// ([`MEDIAN_PARALLEL`], [`MEDIAN_SKEW`] — продолжение той же дороги торец в
/// торец соседом не считается) не дальше [`MEDIAN_GAP`] асфальта. Считает осевую
/// **одно** полотно пары — то, от которого сосед лежит в сторону
/// [`MEDIAN_SIDE`]: с обеих сторон сразу её считали сперва, и две двойные линии
/// ложились одна на другую со сдвигом в сантиметры — толстая двойная, которая
/// там, где одно из полотен кончалось раньше, переходила в тонкую (отчёт
/// автора). Сторона задана направлением в мире, а не номером way, поэтому на
/// стыке двух ways одного полотна осевая не рвётся.
fn medians(streets: &[Street], lot: &PolyArea) -> Vec<(Vec<Vec2>, f32)> {
    let mut found: Vec<(Vec<Vec2>, f32)> = Vec::new();
    // габариты полотен: сосед дальше досягаемости по звеньям не перебирается
    let bounds: Vec<(Vec2, Vec2)> = streets
        .iter()
        .map(|street| ring_bounds(&street.path))
        .collect();
    for (index, street) in streets.iter().enumerate() {
        if !street.carriageway {
            continue;
        }
        let mut run: Vec<Vec2> = Vec::new();
        let mut widest: f32 = 0.0;
        let mut flush = |run: &mut Vec<Vec2>, widest: &mut f32| {
            let length: f32 = run.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
            if length >= MEDIAN_MIN {
                found.push((std::mem::take(run), *widest));
            }
            run.clear();
            *widest = 0.0;
        };
        for (at, heading) in samples(&street.path) {
            let beside = streets
                .iter()
                .enumerate()
                .filter(|(other, street)| *other != index && street.carriageway)
                .filter_map(|(other_index, other)| {
                    let asphalt = (street.width + other.width) / 2.0;
                    let (low, high) = bounds[other_index];
                    let reach = asphalt + MEDIAN_GAP;
                    if at.cmplt(low - reach).any() || at.cmpgt(high + reach).any() {
                        return None;
                    }
                    let (near, other_heading) = nearest(&other.path, at)?;
                    let apart = near - at;
                    let distance = apart.length();
                    (distance <= asphalt + MEDIAN_GAP
                        && distance >= asphalt - MEDIAN_OVERLAP
                        && heading.dot(other_heading).abs() >= MEDIAN_PARALLEL
                        && apart.dot(heading).abs() <= MEDIAN_SKEW * distance
                        && apart.dot(MEDIAN_SIDE) > 0.0)
                        .then_some((near, distance))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1));
            match beside {
                Some((near, distance)) if point_in_area(at.midpoint(near), lot) => {
                    run.push(at.midpoint(near));
                    widest = widest.max(distance);
                }
                _ => flush(&mut run, &mut widest),
            }
        }
        flush(&mut run, &mut widest);
    }
    found
}

/// Точки оси с шагом [`PROBE_STEP`] и направление оси в каждой.
fn samples(path: &[Vec2]) -> Vec<(Vec2, Vec2)> {
    let mut points = Vec::new();
    for pair in path.windows(2) {
        let Some(heading) = (pair[1] - pair[0]).try_normalize() else {
            continue;
        };
        let steps = (pair[0].distance(pair[1]) / PROBE_STEP).ceil().max(1.0) as usize;
        let from = usize::from(!points.is_empty());
        for step in from..=steps {
            points.push((pair[0].lerp(pair[1], step as f32 / steps as f32), heading));
        }
    }
    points
}

/// Ближайшая к `at` точка ломаной и направление звена, на котором она лежит.
fn nearest(path: &[Vec2], at: Vec2) -> Option<(Vec2, Vec2)> {
    path.windows(2)
        .filter_map(|pair| {
            let step = pair[1] - pair[0];
            let heading = step.try_normalize()?;
            let along = ((at - pair[0]).dot(step) / step.length_squared()).clamp(0.0, 1.0);
            Some((pair[0] + step * along, heading))
        })
        .min_by(|left, right| left.0.distance(at).total_cmp(&right.0.distance(at)))
}

/// Заходит ли ось на площадку — пробами через [`PROBE_STEP`].
fn enters(path: &[Vec2], lot: &PolyArea) -> bool {
    samples(path).iter().any(|(at, _)| point_in_area(*at, lot))
}

/// Замкнута ли ломаная — кольцо развязки.
pub(super) fn is_ring(path: &[Vec2]) -> bool {
    path.len() >= 4
        && path
            .first()
            .zip(path.last())
            .is_some_and(|(first, last)| distance_to_segment(*first, *last, *last) < 0.01)
}

/// Полоса вокруг ломаной; кольцо обводится замкнутым, без торцов.
pub(super) fn stroke(path: &[Vec2], width: f32, cap: LineCap<[f32; 2]>) -> Vec<Contour> {
    let ring = is_ring(path);
    let points = if ring { &path[1..] } else { path };
    let contour: Contour = points.iter().map(Vec2::to_array).collect();
    let style = StrokeStyle::new(width)
        .line_join(LineJoin::Round(ARC))
        .start_cap(cap.clone())
        .end_cap(cap);
    contour.stroke(style, ring).into_iter().flatten().collect()
}

/// Кольцо в закрутке, которой ждёт `i_overlay`: внешнее против часовой, дырка
/// по ней.
pub(super) fn oriented(ring: &[Vec2], counterclockwise: bool) -> Contour {
    let mut points: Contour = ring.iter().map(Vec2::to_array).collect();
    if (signed_ring_area(ring) > 0.0) != counterclockwise {
        points.reverse();
    }
    points
}
