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

use bevy::platform::collections::HashMap;

use super::junctions::node_key;
use super::rings::{Ring, Rings};
use super::{is_carriageway, lane_count};
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::{Break, MeshBuilder, min_area_rect};
use crate::map::osm::model::{RoadLine, distance_to_segment, ring_bounds};
use crate::map::shapes::{
    ARC, Contour, RING_EPSILON, Shape, contour_area, contour_bounds, is_ring, oriented,
    point_in_shape, push_shape, ring_of, stroke,
};

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
/// На сколько осевая разделительной дотягивается до острия островка, м, и
/// шаг, которым оно ищется ([`Gores::reach`]).
const MEDIAN_REACH: f32 = 8.0;
const MEDIAN_REACH_STEP: f32 = 0.25;
/// Зазор между торцом двойной линии и остриём штриховки, м.
const MEDIAN_GORE_GAP: f32 = 0.6;

/// Улица так, как она нарисована, — что нужно островкам.
pub(super) struct GoreRoad {
    pub path: Vec<Vec2>,
    pub width: f32,
    pub oneway: bool,
    /// [`RoadLine::is_roundabout`] — тег **или форма**: большое кольцо у ТРЦ
    /// «Макси» (way 397005605) в OSM просто `oneway=yes`, замкнутый сам на
    /// себя, без `junction=roundabout`, и по одному тегу ни один его островок
    /// не находился.
    pub roundabout: bool,
}

impl GoreRoad {
    /// Собрать из дороги и её нарисованной оси. Замкнутость — по **сырым**
    /// точкам OSM: это свойство way, а не стиля рисования. Сглаживание кольцо
    /// замыкает (`smooth_pinned` идёт по циклу), так что дозамыкание —
    /// страховка на случай оси, пришедшей другим путём.
    pub fn new(road: &RoadLine, drawn: &[Vec2]) -> Self {
        let mut path = drawn.to_vec();
        if let (true, Some(first)) = (is_ring(&road.points), path.first().copied())
            && path
                .last()
                .is_some_and(|last| last.distance(first) > RING_EPSILON)
        {
            path.push(first);
        }
        Self {
            path,
            width: road.width,
            oneway: road.oneway,
            roundabout: road.is_roundabout(),
        }
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
            .filter(|road| road.roundabout)
            .flat_map(|road| road.path.iter().copied())
            .collect();
        let at_ring = |point: Vec2| knots.iter().any(|knot| knot.distance(point) <= ARM_SNAP);
        // подход — кусок одностороннего полотна от кольца на [`ARM_REACH`]
        let arms: Vec<(Vec<Vec2>, f32)> = roads
            .iter()
            .filter(|road| road.oneway && !road.roundabout)
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
            network.extend(stroke(path, *width, LineCap::Round(ARC), is_ring(path)));
            reach.push(closing_span(path, *width));
        }
        let mut solid: Vec<Contour> = Vec::new();
        for road in roads.iter().filter(|road| road.roundabout) {
            let ring = is_ring(&road.path);
            network.extend(stroke(&road.path, road.width, LineCap::Round(ARC), ring));
            reach.push(closing_span(&road.path, road.width));
            // остров маленького кольца замыкание затянуло бы тоже
            if ring {
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
                solid.extend(stroke(
                    &road.path,
                    road.width,
                    LineCap::Round(ARC),
                    is_ring(&road.path),
                ));
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
                    let touch = width / 2.0 + ARM_TOUCH;
                    low.cmple(to + touch).all() && high.cmpge(from - touch).all()
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
                            let touch = width / 2.0 + ARM_TOUCH;
                            outer.iter().any(|point| {
                                let point = Vec2::from_array(*point);
                                path.windows(2).any(|link| {
                                    distance_to_segment(point, link[0], link[1]) <= touch
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

    /// Добавить островки, поставленные по правилу ([`splitters`]): контур
    /// штрихуется, расширение подхода — асфальт.
    pub fn add_splitters(&mut self, splitters: &[Splitter]) {
        for splitter in splitters {
            self.hatched.push(splitter.island.clone());
            self.asphalt.push(splitter.flare.clone());
        }
    }

    /// Контуры островков — тому, кто вычитает их из своего (бордюр стоянки).
    pub fn contours(&self) -> impl Iterator<Item = &Contour> {
        self.asphalt.iter().flatten()
    }

    /// Лежит ли точка на штриховке островка.
    pub fn contains(&self, point: Vec2) -> bool {
        self.hatched
            .iter()
            .any(|shape| point_in_shape(point, shape))
    }

    /// Дотянуть осевую разделительной до островка, если он в пределах
    /// [`MEDIAN_REACH`] по её ходу.
    ///
    /// Осевая кончается там, где половины перестают идти бок о бок, а клин
    /// штриховки — там, где зазор между ними сходит на нет, и между остриём
    /// клина и двойной линией оставалось метра три голого асфальта (отчёт
    /// автора). На земле края островка **сходятся в** двойную сплошную.
    pub fn reach(&self, midline: &mut Vec<Vec2>) {
        if self.hatched.is_empty() {
            return;
        }
        // осевая есть, пока между половинами до трёх метров асфальта, клин —
        // пока их от 0.6 м: в промежутке обе есть разом, и двойная линия
        // уезжала внутрь штриховки (отчёт автора). Концы, попавшие в клин,
        // срезаются, и дотягивается осевая уже от чистого места
        while midline.last().is_some_and(|point| self.contains(*point)) {
            midline.pop();
        }
        let inside = midline
            .iter()
            .take_while(|point| self.contains(**point))
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
            // сколько по ходу до штриховки; вплотную линия не подводится —
            // между её торцом и остриём клина остаётся [`MEDIAN_GORE_GAP`]
            // (просьба автора: встык торец двойной линии сливался с обводкой
            // островка)
            let Some(to_gore) = (1..=steps)
                .map(|step| step as f32 * MEDIAN_REACH_STEP)
                .find(|reach| self.contains(tip + heading * *reach))
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

    /// Асфальт островков — в слой улиц: он выше тротуаров и кроет их треугольник.
    pub fn push_asphalt(&self, builder: &mut MeshBuilder, color: LinearRgba) {
        for shape in &self.asphalt {
            push_shape(builder, shape.clone(), color);
        }
    }

    /// Штрихуемые островки и направление поперёк их косых полос — слою
    /// краски (`roads/paint.rs`): обводку и полосы рисует его шейдер, и с
    /// зумом они гаснут вместе с зебрами. Штрихуется только разомкнутая
    /// часть клина, не асфальт: тот шире на заход под полотна, и полосы
    /// вылезли бы на дорогу.
    ///
    /// Полосы идут под 45° к длинной оси островка.
    pub fn islands(&self) -> impl Iterator<Item = (&Shape, Vec2)> {
        self.hatched.iter().filter_map(|gore| {
            let ring = ring_of(gore.first()?);
            let corners = min_area_rect(&ring)?;
            let (side, next) = (corners[1] - corners[0], corners[2] - corners[1]);
            let long = if side.length() >= next.length() {
                side
            } else {
                next
            };
            let along = Vec2::from_angle(std::f32::consts::FRAC_PI_4).rotate(long.try_normalize()?);
            Some((gore, along.perp()))
        })
    }
}

/// Кольцо меньше этого радиуса, м, островков на подходах не получает: на
/// дворовом кольце им негде встать.
const SPLITTER_MIN_RADIUS: f32 = 10.0;
/// Между кромкой кольца и основанием островка, м.
const SPLITTER_GAP: f32 = 1.0;
/// Длина островка — доля радиуса кольца, в пределах, м.
const SPLITTER_SHARE: f32 = 0.6;
const SPLITTER_LENGTH: std::ops::RangeInclusive<f32> = 6.0..=20.0;
/// Полуширина основания — доля радиуса, в пределах, м.
const SPLITTER_WIDTH_SHARE: f32 = 0.06;
const SPLITTER_HALF_WIDTH: std::ops::RangeInclusive<f32> = 0.6..=1.5;
/// Звенья контура островка вдоль подхода.
const SPLITTER_STEPS: usize = 8;

/// Островок, поставленный по правилу на двусторонний подход к кольцу: в
/// OSM такой подход — один way, веера из въезда и съезда, между которыми
/// [`Gores::of`] нашёл бы клин, нет. Подход у кольца расширяется на
/// островок: каждая полоса сохраняет ширину, между ними — штриховка.
pub(super) struct Splitter {
    /// Подход.
    pub road: usize,
    /// Штрихуемый контур — капля от основания у кольца к острию.
    pub island: Shape,
    /// Асфальт расширения подхода.
    pub flare: Shape,
    /// Разрыв краски и колеи подхода на длину островка.
    pub gap: Break,
}

/// Островки на двусторонних подходах к кольцам `rings`: подход — проезжая
/// часть в две полосы и больше, приходящая концом в узел кольца.
pub(super) fn splitters(
    drawn: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    rings: &Rings,
) -> Vec<Splitter> {
    let mut nodes: HashMap<(i32, i32), usize> = HashMap::new();
    for (index, ring) in rings.list.iter().enumerate() {
        if ring.mean_radius() < SPLITTER_MIN_RADIUS {
            continue;
        }
        for &road in &ring.roads {
            for &point in &drawn[road].points {
                nodes.insert(node_key(point), index);
            }
        }
    }
    let mut found = Vec::new();
    for (road, line) in drawn.iter().enumerate() {
        if line.oneway
            || line.bridge
            || !is_carriageway(line)
            || lane_count(line) < 2
            || rings.of(road).is_some()
        {
            continue;
        }
        let path = paths[road].as_ref();
        for end in [false, true] {
            let Some(&tip) = (if end { path.last() } else { path.first() }) else {
                continue;
            };
            let Some(&ring) = nodes.get(&node_key(tip)) else {
                continue;
            };
            let ring = &rings.list[ring];
            let mut from_ring = path.to_vec();
            if end {
                from_ring.reverse();
            }
            let ring_width = drawn[ring.roads[0]].width;
            if let Some(splitter) = splitter(road, line.width, &from_ring, ring, ring_width) {
                found.push(splitter);
            }
        }
    }
    found
}

/// Островок на подходе `path`, идущем **от** узла кольца.
fn splitter(
    road: usize,
    width: f32,
    path: &[Vec2],
    ring: &Ring,
    ring_width: f32,
) -> Option<Splitter> {
    let radius = ring.mean_radius();
    let base = ring_width / 2.0 + SPLITTER_GAP;
    let (along, total) = arclengths(path);
    let length = (SPLITTER_SHARE * radius)
        .clamp(*SPLITTER_LENGTH.start(), *SPLITTER_LENGTH.end())
        .min(total - base - width);
    if length < *SPLITTER_LENGTH.start() {
        return None;
    }
    let half = (SPLITTER_WIDTH_SHARE * radius)
        .clamp(*SPLITTER_HALF_WIDTH.start(), *SPLITTER_HALF_WIDTH.end());
    // полуширина островка: капля — полная у основания, в ноль к острию
    let spread = |at: f32| {
        let share = ((at - base) / length).clamp(0.0, 1.0);
        half * (1.0 - share).powf(0.8)
    };
    let sample = |at: f32| {
        let (point, direction) = place_on_path(path, &along, at)?;
        Some((point, direction.perp()))
    };
    let mut left = Vec::new();
    let mut right = Vec::new();
    for step in 0..=SPLITTER_STEPS {
        let at = base + length * step as f32 / SPLITTER_STEPS as f32;
        let (point, normal) = sample(at)?;
        left.push(point + normal * spread(at));
        right.push(point - normal * spread(at));
    }
    right.pop();
    right.reverse();
    let island: Vec<Vec2> = left.into_iter().chain(right).collect();
    // расширение — от кромки кольца до острия, полотно раздвинуто на островок
    let edge = (ring_width / 2.0 - ASPHALT_PAD).max(0.0);
    let mut sides = [Vec::new(), Vec::new()];
    let steps = SPLITTER_STEPS * 2;
    for step in 0..=steps {
        let at = edge + (base + length - edge) * step as f32 / steps as f32;
        let (point, normal) = sample(at)?;
        let reach = width / 2.0 + if at < base { half } else { spread(at) };
        sides[0].push(point + normal * reach);
        sides[1].push(point - normal * reach);
    }
    sides[1].reverse();
    let flare: Vec<Vec2> = sides.concat();
    let middle = sample(base + length / 2.0)?.0;
    Some(Splitter {
        road,
        island: vec![oriented(&island, true)],
        flare: vec![oriented(&flare, true)],
        gap: Break {
            at: middle,
            reach: length / 2.0 + SPLITTER_GAP,
        },
    })
}

/// Куда дотягивается замыкание ленты: её габарит, выпущенный на полуширину
/// полотна и на [`GORE_CLOSING`].
fn closing_span(path: &[Vec2], width: f32) -> (Vec2, Vec2) {
    let (low, high) = ring_bounds(path);
    let pad = width / 2.0 + GORE_CLOSING;
    (low - pad, high + pad)
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
