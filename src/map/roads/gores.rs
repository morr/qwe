//! Направляющие островки у колец — асфальт со штриховкой.
//!
//! Подход к кольцу в OSM — два односторонних way, въезд и съезд, расходящиеся
//! от общей дороги к двум узлам кольца. Клин между ними и кольцом на земле —
//! ровный асфальт с косой разметкой, по нему не ездят и не ходят; так его
//! рисует и Яндекс. У нас там выходил треугольник **тротуара**: полосы тротуара
//! обоих полотен накрывали клин с двух сторон. На большой стоянке тем же
//! треугольником ложился бордюр (`roads/lots.rs`) — сперва обводкой, потом
//! заливкой с серпом асфальта внутри и рваным стыком у кольца (отчёты автора).
//!
//! Островок — свойство **сети у кольца**, а не стоянки, поэтому считается здесь
//! для любого кольца города, а стоянка только вычитает его из своего бордюра.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle};

use super::lots::{ARC, Contour, Shape, contour_area, is_ring, oriented, push_shape, stroke};
use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::model::{RoadLine, distance_to_segment, point_in_polygon, ring_bounds};

/// Радиус замыкания, м: клин между двумя полотнами ближе двух радиусов друг к
/// другу — островок.
const GORE_CLOSING: f32 = 6.0;
/// Островок мельче этого, м², не рисуется.
const GORE_MIN_AREA: f32 = 12.0;
/// Размыкание клина, м: щель у́же двух таких между почти сомкнутыми полотнами —
/// не островок.
const GORE_OPENING: f32 = 0.3;
/// На сколько асфальт островка заходит под кромки полотен, м, — чтобы между
/// ним и лентой дороги не оставалось волоска земли.
const ASPHALT_PAD: f32 = 0.5;
/// Насколько близко конец полотна к вершине кольца, чтобы считаться его
/// подходом, м.
const ARM_SNAP: f32 = 1.0;
/// Сколько подхода от кольца идёт в замыкание, м. Полотно проспекта тянется
/// на сотни метров, и вместе со встречным затянуло бы штриховкой всю
/// разделительную; островок же кончается в десятках метров от кольца.
const ARM_REACH: f32 = 40.0;
/// Насколько вершина клина близко к кромке полотна, чтобы клин его касался, м.
const ARM_TOUCH: f32 = 0.5;
/// Разметка островка: обводка, ширина косой полосы и шаг между полосами, м.
const LINE_WIDTH: f32 = 0.2;
const HATCH_WIDTH: f32 = 0.35;
const HATCH_STEP: f32 = 1.6;
const LINE_COLOR: Color = Color::srgb(0.88, 0.88, 0.86);

/// Улица так, как она нарисована, — что нужно островкам.
pub(super) struct GoreRoad {
    pub path: Vec<Vec2>,
    pub width: f32,
    pub oneway: bool,
    pub roundabout: bool,
    /// Замкнут ли way **в OSM**. По нарисованному пути этого не узнать:
    /// сглаживание обращается с замкнутым way как с открытым и срезает угол на
    /// его шве, так что концы нарисованного кольца расходятся на метры.
    pub closed: bool,
}

impl GoreRoad {
    /// Собрать из дороги и её нарисованной оси; у замкнутого way ось
    /// замыкается обратно — иначе в полосе кольца на шве дыра.
    pub fn new(road: &RoadLine, drawn: &[Vec2]) -> Self {
        let closed = is_ring(&road.points);
        let mut path = drawn.to_vec();
        if let (true, Some(first)) = (closed, path.first().copied())
            && path.last().is_some_and(|last| last.distance(first) > 0.01)
        {
            path.push(first);
        }
        Self {
            path,
            width: road.width,
            oneway: road.oneway,
            roundabout: road.roundabout,
            closed,
        }
    }

    /// Кольцо — по тегу **или по форме**: замкнутое одностороннее полотно.
    /// Большое кольцо у ТРЦ «Макси» (way 397005605) в OSM — просто
    /// `oneway=yes`, замкнутый сам на себя, без `junction=roundabout`, и по
    /// одному тегу ни один его островок не находился.
    fn is_roundabout(&self) -> bool {
        self.roundabout || (self.oneway && self.closed)
    }
}

/// Островки города: асфальт клина целиком и та его часть, что штрихуется.
pub(super) struct Gores {
    asphalt: Vec<Shape>,
    hatched: Vec<Shape>,
}

impl Gores {
    /// Клин — то, что затянуло **замыкание асфальта кольца и его подходов**
    /// ([`GORE_CLOSING`]), минус асфальт всех улиц рядом и остров самого кольца.
    ///
    /// Тем же замыканием затягивается и скругление с **внешней** стороны
    /// подхода, а подходы к кольцу пологие, и оно выходит немаленьким. Отличает
    /// его не площадь, а соседи: островок лежит **между двумя подходами**,
    /// скругление касается одного.
    pub fn of(roads: &[GoreRoad]) -> Self {
        let knots: Vec<Vec2> = roads
            .iter()
            .filter(|road| road.is_roundabout())
            .flat_map(|road| road.path.iter().copied())
            .collect();
        let at_ring = |point: Vec2| knots.iter().any(|knot| knot.distance(point) <= ARM_SNAP);
        // подход — кусок одностороннего полотна от кольца на [`ARM_REACH`]
        let arms: Vec<(Vec<Vec2>, f32)> = roads
            .iter()
            .filter(|road| road.oneway && !road.is_roundabout())
            .flat_map(|road| {
                let mut arms = Vec::new();
                if road.path.first().is_some_and(|point| at_ring(*point)) {
                    arms.push((head(road.path.iter().copied()), road.width));
                }
                if road.path.last().is_some_and(|point| at_ring(*point)) {
                    arms.push((head(road.path.iter().rev().copied()), road.width));
                }
                arms
            })
            .collect();
        if arms.len() < 2 {
            return Self {
                asphalt: Vec::new(),
                hatched: Vec::new(),
            };
        }

        let mut network: Vec<Contour> = Vec::new();
        // Куда дотягивается замыкание: габарит ленты, выпущенный на её
        // полуширину и на само замыкание. Дальше этого прямоугольника клина
        // нет — ни искать его там, ни вычитать из него нечего.
        let mut reach: Vec<(Vec2, Vec2)> = Vec::new();
        for (path, width) in &arms {
            network.extend(stroke(path, *width, LineCap::Round(ARC)));
            reach.push(closing_span(path, *width));
        }
        let mut solid: Vec<Contour> = Vec::new();
        for road in roads.iter().filter(|road| road.is_roundabout()) {
            network.extend(stroke(&road.path, road.width, LineCap::Round(ARC)));
            reach.push(closing_span(&road.path, road.width));
            // остров маленького кольца замыкание затянуло бы тоже
            if is_ring(&road.path) {
                solid.push(oriented(&road.path[1..], true));
            }
        }
        // Доходит до клина улица **кромкой**, а не осью: у проспекта это
        // восемь метров, и по оси он в замыкание не попадал.
        for road in roads {
            let pad = road.width / 2.0;
            let (low, high) = ring_bounds(&road.path);
            let near = reach
                .iter()
                .any(|(from, to)| (low - pad).cmple(*to).all() && (high + pad).cmpge(*from).all());
            if near {
                solid.extend(stroke(&road.path, road.width, LineCap::Round(ARC)));
            }
        }

        let round = LineJoin::Round(ARC);
        // клин целиком — всё, что замыкание затянуло между двумя подходами
        let closed = network
            .simplify_shape(FillRule::NonZero)
            .outline(&OutlineStyle::new(GORE_CLOSING).line_join(round.clone()))
            .outline(&OutlineStyle::new(-GORE_CLOSING).line_join(round.clone()));
        // Разность — дело **одного** клина: замыкание уже разложило город на
        // отдельные фигуры, и асфальт из дальнего конца города ни одной из них
        // не касается. Одной булевой на все стоило вдвое-втрое дороже
        // (замеры — в `osm-map`): половина работы уходила на пересечения лент
        // между собой там, где клина и нет.
        let mut wedges: Vec<Shape> = Vec::new();
        for shape in closed {
            let Some((low, high)) = shape.first().map(contour_bounds) else {
                continue;
            };
            // Островок лежит между двумя подходами, скругление с внешней
            // стороны подхода — у одного; то же спрашивается ниже у самой
            // фигуры, но там ответ точный, а платить за него разностью незачем
            let between = arms
                .iter()
                .filter(|(path, width)| {
                    let (from, to) = ring_bounds(path);
                    let reach = width / 2.0 + ARM_TOUCH;
                    low.cmple(to + reach).all() && high.cmpge(from - reach).all()
                })
                .count();
            if between < 2 {
                continue;
            }
            let clip: Vec<Contour> = solid
                .iter()
                .filter(|contour| {
                    let (from, to) = contour_bounds(contour);
                    from.cmple(high).all() && to.cmpge(low).all()
                })
                .cloned()
                .collect();
            wedges.extend(vec![shape].overlay(&clip, OverlayRule::Difference, FillRule::NonZero));
        }
        let bodies: Vec<Shape> = wedges
            .into_iter()
            .filter(|shape| {
                let Some(outer) = shape.first() else {
                    return false;
                };
                contour_area(outer) >= GORE_MIN_AREA
                    && arms
                        .iter()
                        .filter(|(path, width)| {
                            let reach = width / 2.0 + ARM_TOUCH;
                            outer.iter().any(|point| {
                                let point = Vec2::from_array(*point);
                                path.windows(2).any(|link| {
                                    distance_to_segment(point, link[0], link[1]) <= reach
                                })
                            })
                        })
                        .count()
                        >= 2
            })
            .collect();
        // штрихуется клин **разомкнутый** — без остриёв и перемычек тоньше
        // двух [`GORE_OPENING`], где полосе встать негде; асфальтом же
        // заливается весь, с заходом под кромки полотен. Одной фигурой на оба
        // дела он был сперва, и в срезанных размыканием местах между
        // штриховкой и дорогой проглядывала земля (отчёт автора)
        let hatched = bodies
            .outline(&OutlineStyle::new(-GORE_OPENING).line_join(round.clone()))
            .outline(&OutlineStyle::new(GORE_OPENING).line_join(round.clone()))
            .into_iter()
            .filter(|shape| {
                shape
                    .first()
                    .is_some_and(|outer| contour_area(outer) >= GORE_MIN_AREA)
            })
            .collect();
        let asphalt = bodies.outline(&OutlineStyle::new(ASPHALT_PAD).line_join(round));
        Self { asphalt, hatched }
    }

    pub fn count(&self) -> usize {
        self.hatched.len()
    }

    /// Контуры островков — тому, кто вычитает их из своего (бордюр стоянки).
    pub fn contours(&self) -> impl Iterator<Item = &Contour> {
        self.asphalt.iter().flatten()
    }

    /// Лежит ли точка на штриховке островка.
    pub fn contains(&self, point: Vec2) -> bool {
        self.hatched.iter().any(|shape| {
            let mut rings = shape.iter().map(|contour| {
                contour
                    .iter()
                    .copied()
                    .map(Vec2::from_array)
                    .collect::<Vec<Vec2>>()
            });
            rings
                .next()
                .is_some_and(|outer| point_in_polygon(point, &outer))
                && !rings.any(|hole| point_in_polygon(point, &hole))
        })
    }

    /// Асфальт островков — в слой улиц: он выше тротуаров и кроет их треугольник.
    pub fn push_asphalt(&self, builder: &mut MeshBuilder, color: LinearRgba) {
        for shape in &self.asphalt {
            push_shape(builder, shape.clone(), color);
        }
    }

    /// Обводка и косая штриховка.
    ///
    /// Полосы идут под 45° к длинной оси островка шагом [`HATCH_STEP`];
    /// обрезка по контурам — одна булева операция на все островки города.
    pub fn push_markings(&self, builder: &mut MeshBuilder) {
        let color = LINE_COLOR.to_linear();
        let mut stripes: Vec<Contour> = Vec::new();
        for gore in &self.hatched {
            for ring in gore {
                let ring: Vec<Vec2> = ring.iter().copied().map(Vec2::from_array).collect();
                builder.push_stroke(&ring, true, LINE_WIDTH, color);
            }
            let Some(outer) = gore.first() else {
                continue;
            };
            let ring: Vec<Vec2> = outer.iter().copied().map(Vec2::from_array).collect();
            let Some(corners) = min_area_rect(&ring) else {
                continue;
            };
            let (side, next) = (corners[1] - corners[0], corners[2] - corners[1]);
            let long = if side.length() >= next.length() {
                side
            } else {
                next
            };
            let Some(axis) = long.try_normalize() else {
                continue;
            };
            let along = Vec2::from_angle(std::f32::consts::FRAC_PI_4).rotate(axis);
            let across = along.perp();
            let centre = corners.iter().sum::<Vec2>() / 4.0;
            let radius = corners[0].distance(centre) + HATCH_STEP;
            let count = (radius / HATCH_STEP).ceil() as i32;
            for index in -count..=count {
                let middle = centre + across * (index as f32 * HATCH_STEP);
                let (half, length) = (across * (HATCH_WIDTH / 2.0), along * radius);
                stripes.push(vec![
                    (middle - length - half).to_array(),
                    (middle + length - half).to_array(),
                    (middle + length + half).to_array(),
                    (middle - length + half).to_array(),
                ]);
            }
        }
        if stripes.is_empty() {
            return;
        }
        // обрезка — по штрихуемой части, а не по асфальту: тот шире на заход
        // под полотна, и полосы вылезли бы на дорогу
        let gores: Vec<Contour> = self.hatched.iter().flatten().cloned().collect();
        for shape in stripes.overlay(&gores, OverlayRule::Intersect, FillRule::NonZero) {
            push_shape(builder, shape, color);
        }
    }
}

/// Куда дотягивается замыкание ленты: её габарит, выпущенный на полуширину
/// полотна и на [`GORE_CLOSING`].
fn closing_span(path: &[Vec2], width: f32) -> (Vec2, Vec2) {
    let (low, high) = ring_bounds(path);
    let pad = width / 2.0 + GORE_CLOSING;
    (low - pad, high + pad)
}

/// Габарит контура — то же, что `ring_bounds`, но по точкам `i_overlay`.
fn contour_bounds(contour: &Contour) -> (Vec2, Vec2) {
    contour.iter().fold(
        (Vec2::INFINITY, Vec2::NEG_INFINITY),
        |(low, high), point| {
            let point = Vec2::from_array(*point);
            (low.min(point), high.max(point))
        },
    )
}

/// Начало ломаной — первые [`ARM_REACH`] метров.
fn head(points: impl Iterator<Item = Vec2>) -> Vec<Vec2> {
    let mut path: Vec<Vec2> = Vec::new();
    let mut left = ARM_REACH;
    for point in points {
        let Some(last) = path.last().copied() else {
            path.push(point);
            continue;
        };
        let step = last.distance(point);
        if step >= left {
            path.push(last.lerp(point, left / step.max(f32::EPSILON)));
            break;
        }
        left -= step;
        path.push(point);
    }
    path
}
