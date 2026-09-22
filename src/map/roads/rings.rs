//! Кольцо заново: гладкая фигура вместо гранёной ломаной OSM.
//!
//! Кольцо в OSM — замкнутый way или цепочка дуг (`junction=roundabout`),
//! нарисованная десятком вершин: лента по ним шла гранями, а Chaikin по
//! каждой дуге отдельно гнул её к хордам. Здесь дуги собираются в одну петлю
//! ([`Ring`]), в её вершины вписывается **эллипс** — или окружность, если оси
//! почти равны, — и каждая дуга рисуется по нему.
//!
//! **Общие узлы кольца остаются на месте**: по ним находят друг друга
//! подходы, разрывы разметки, скругления бордюров и краска узла. Эллипс
//! через них не проходит, поэтому он масштабируется по лучу из центра на
//! гладко меняющийся множитель ([`Ring::scale`]) — ровно единица в каждом
//! узле. Отступ узла от эллипса — метр-два на дугу в десятки метров, и на
//! глаз фигура остаётся эллипсом.
//!
//! Односторонний подход входит в кольцо и выходит из него **по касательной
//! дугой** ([`bend_approach`]): последние метры подхода заменяет кривая,
//! касательная к нему и приходящая в узел под [`ENTRY_ANGLE`] к ходу
//! кольца, — как на земле, где въезд отклоняют, чтобы на кольцо не влетали
//! по прямой.

use std::borrow::Cow;
use std::f32::consts::TAU;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::node_key;
use super::network::RoadNodes;
use super::turns::{LaneEnd, curve};
use crate::map::along::nearest_on_path;
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::shapes::is_ring;

/// Шаг, с которым петля кольца выбирается для подгонки, м: вершины OSM
/// стоят неравномерно, и густо нарисованная дуга перетянула бы эллипс.
const SAMPLE_STEP: f32 = 1.0;
/// Разница осей, до которой кольцо — окружность, доля большей оси.
const CIRCLE_SHARE: f32 = 0.08;
/// Насколько вершина OSM может отстоять от вписанной фигуры, м, — и доля
/// меньшей полуоси, если она больше. Дальше — петля не кольцо (вытянутая
/// дорога вокруг сквера), и она рисуется по-старому. Кольцо secondary в
/// Туле — «яйцо» 150 × 115 м — отходит от эллипса на 14 м, и оно кольцо:
/// узлы держат форму сами, через них проходит множитель луча.
const FIT_SLACK: f32 = 4.0;
const FIT_SHARE: f32 = 0.35;
/// Кольцо меньше этого радиуса, м, не перерисовывается.
const MIN_RADIUS: f32 = 4.0;
/// Стрелка прогиба звена дуги, м, — допуск [`meshing`](crate::map::meshing).
const CHORD_TOLERANCE: f32 = 0.05;
/// Звено ближе к узлу, м, не ставится: узел — сам вершина.
const PIN_CLEARANCE: f32 = 0.3;
/// Дуг в кольце не больше — страховка от петли по сети.
const MAX_ARCS: usize = 32;
/// Угол, под которым подход приходит в узел кольца, к ходу кольца, рад.
const ENTRY_ANGLE: f32 = 25.0 * std::f32::consts::PI / 180.0;
/// Длина дуги подхода — доля среднего радиуса кольца, в пределах, м.
const APPROACH_SHARE: f32 = 0.5;
const APPROACH_MIN: f32 = 6.0;
const APPROACH_MAX: f32 = 20.0;
/// Какую долю подхода дуга может занять, если он короткий.
const APPROACH_LENGTH_SHARE: f32 = 0.6;
/// Дуга короче этого, м, не строится.
const BEND_MIN_LENGTH: f32 = 4.0;
/// Подход, приходящий в узел почти под нужным углом, не трогается, рад.
const BEND_MIN_ANGLE: f32 = 5.0 * std::f32::consts::PI / 180.0;
/// Сколько подхода, м, остаётся прямым перед его общим узлом с кем-то ещё.
const PIN_MARGIN: f32 = 1.0;
/// Насколько кромки подхода и кольца могут разойтись, м, чтобы щель между
/// ними залилась асфальтом ([`web`]).
const WEB_GAP: f32 = 2.5;
/// База, по которой берётся направление подхода у его торца, м.
const HEADING_BASE: f32 = 3.0;

/// Кольцо: дуги по ходу движения и вписанная в них фигура.
pub struct Ring {
    /// Дороги кольца по ходу движения; у замкнутого way — одна.
    pub roads: Vec<usize>,
    pub center: Vec2,
    /// Направление большой полуоси.
    axis: Vec2,
    /// Полуоси: `x` — вдоль [`Ring::axis`], `y` — поперёк.
    pub radii: Vec2,
    /// Идёт ли движение против часовой стрелки.
    pub ccw: bool,
    /// Узлы кольца: параметр эллипса и множитель луча, отсортированы.
    pins: Vec<(f32, f32)>,
    /// Нарисованная ось кольца целиком, замкнутая: последняя точка равна
    /// первой.
    pub path: Vec<Vec2>,
}

impl Ring {
    pub fn mean_radius(&self) -> f32 {
        (self.radii.x + self.radii.y) / 2.0
    }

    fn local(&self, point: Vec2) -> Vec2 {
        let offset = point - self.center;
        Vec2::new(offset.dot(self.axis), offset.dot(self.axis.perp()))
    }

    /// Параметр эллипса точки и её множитель по лучу из центра.
    fn param(&self, point: Vec2) -> (f32, f32) {
        let unit = self.local(point) / self.radii;
        (unit.y.atan2(unit.x).rem_euclid(TAU), unit.length())
    }

    /// Множитель луча в параметре `t`: эрмитов сплайн по узлам, замкнутый по
    /// кругу, — единица в каждом узле ровно, между ними гладко.
    fn scale(&self, t: f32) -> f32 {
        let count = self.pins.len() as isize;
        if count == 0 {
            return 1.0;
        }
        let at = |k: isize| {
            let (pin, scale) = self.pins[k.rem_euclid(count) as usize];
            (pin + TAU * k.div_euclid(count) as f32, scale)
        };
        let slope = |k: isize| {
            let ((before, low), (after, high)) = (at(k - 1), at(k + 1));
            (high - low) / (after - before).max(1e-4)
        };
        let t = t.rem_euclid(TAU);
        let index = self
            .pins
            .iter()
            .rposition(|&(pin, _)| pin <= t)
            .map_or(-1, |index| index as isize);
        let ((from, low), (to, high)) = (at(index), at(index + 1));
        let span = (to - from).max(1e-5);
        let x = (t - from) / span;
        let (x2, x3) = (x * x, x * x * x);
        low * (2.0 * x3 - 3.0 * x2 + 1.0)
            + slope(index) * span * (x3 - 2.0 * x2 + x)
            + high * (-2.0 * x3 + 3.0 * x2)
            + slope(index + 1) * span * (x3 - x2)
    }

    /// Точка нарисованной оси в параметре `t`.
    pub fn point(&self, t: f32) -> Vec2 {
        let (sin, cos) = t.sin_cos();
        let local = self.axis * (self.radii.x * cos) + self.axis.perp() * (self.radii.y * sin);
        self.center + local * self.scale(t)
    }

    /// Касательная по возрастанию `t` — против часовой стрелки.
    fn tangent(&self, t: f32) -> Vec2 {
        let step = 1e-3;
        (self.point(t + step) - self.point(t - step)).normalize_or_zero()
    }

    /// Ход движения по кольцу в параметре `t`.
    pub fn travel(&self, t: f32) -> Vec2 {
        let tangent = self.tangent(t);
        if self.ccw { tangent } else { -tangent }
    }

    /// Нормаль наружу, от центра.
    pub fn outward(&self, t: f32) -> Vec2 {
        -self.tangent(t).perp()
    }

    /// Сколько звеньев нужно дуге в `sweep` радиан, чтобы хорда отходила от
    /// неё не больше чем на [`CHORD_TOLERANCE`].
    fn steps(&self, sweep: f32) -> usize {
        let radius = self.radii.max_element();
        let step = 2.0 * (1.0 - CHORD_TOLERANCE / radius).clamp(-1.0, 1.0).acos();
        ((sweep / step.max(1e-3)).ceil() as usize).max(1)
    }

    /// Ось дуги `points` по фигуре — от её первой точки до последней по ходу,
    /// через её общие узлы.
    fn arc(&self, points: &[Vec2], nodes: &RoadNodes) -> Vec<Vec2> {
        let (mut first, mut last) = (points[0], points[points.len() - 1]);
        let direction = if self.ccw { 1.0 } else { -1.0 };
        let start = self.param(first).0;
        let closed = is_ring(points);
        // шов замкнутого way, где никто не примыкает, — просто вершина:
        // ось идёт по фигуре и там
        if closed && !nodes.is_shared(first) {
            first = self.point(start);
            last = first;
        }
        let sweep = if closed {
            TAU
        } else {
            ((self.param(last).0 - start) * direction).rem_euclid(TAU)
        };
        // узлы внутри дуги — её вершины как есть
        let mut marks: Vec<(f32, Option<Vec2>)> = points[1..points.len() - 1]
            .iter()
            .filter(|point| nodes.is_shared(**point))
            .map(|&point| {
                let offset = ((self.param(point).0 - start) * direction).rem_euclid(TAU);
                (offset, Some(point))
            })
            .filter(|(offset, _)| *offset > 0.0 && *offset < sweep)
            .collect();
        let clearance = PIN_CLEARANCE / self.mean_radius();
        let steps = self.steps(sweep);
        let samples: Vec<f32> = (1..steps)
            .map(|step| sweep * step as f32 / steps as f32)
            .filter(|offset| {
                marks
                    .iter()
                    .all(|(pin, _)| (pin - offset).abs() > clearance)
                    && *offset > clearance
                    && sweep - offset > clearance
            })
            .collect();
        marks.extend(samples.into_iter().map(|offset| (offset, None)));
        marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut path = vec![first];
        path.extend(marks.into_iter().map(|(offset, exact)| {
            exact.unwrap_or_else(|| self.point(start + direction * offset))
        }));
        path.push(last);
        path
    }
}

/// Кольца города и какое из них у каждой дороги.
#[derive(Default)]
pub struct Rings {
    pub list: Vec<Ring>,
    of_road: Vec<Option<usize>>,
    /// Асфальт между кольцом и улицей вдоль него, где их кромки только
    /// разошлись ([`webs_along`]): без него в щели серпом проступал тротуар.
    pub webs: Vec<Vec<Vec2>>,
}

impl Rings {
    /// Кольцо, которому принадлежит дорога.
    pub fn of(&self, road: usize) -> Option<&Ring> {
        self.of_road
            .get(road)
            .copied()
            .flatten()
            .map(|ring| &self.list[ring])
    }
}

/// Может ли дорога быть дугой перерисовываемого кольца.
fn is_arc(road: &RoadLine) -> bool {
    road.is_roundabout()
        && road.class == RoadClass::Street
        && !road.carves_navmesh()
        && road.points.len() >= 2
}

/// Кольца `roads`: дуги собираются в петли, в петлю вписывается фигура, и
/// оси дуг в `paths` заменяются её дугами; подходы входят в кольцо по
/// касательной. Кольцо, в которое фигура не вписалась, остаётся как было.
pub fn reshape<'a>(
    roads: &'a [RoadLine],
    nodes: &RoadNodes,
    paths: &mut [Cow<'a, [Vec2]>],
) -> Rings {
    let mut rings = Rings {
        list: Vec::new(),
        of_road: vec![None; roads.len()],
        webs: Vec::new(),
    };
    for chain in chains(roads) {
        let Some(ring) = fit(roads, &chain, nodes) else {
            continue;
        };
        let index = rings.list.len();
        let mut path: Vec<Vec2> = Vec::new();
        for &road in &chain {
            let arc = ring.arc(&roads[road].points, nodes);
            let skip = usize::from(!path.is_empty());
            path.extend(arc.iter().skip(skip));
            paths[road] = Cow::Owned(arc);
            rings.of_road[road] = Some(index);
        }
        rings.list.push(Ring { path, ..ring });
    }
    // узлы колец — к ним приходят подходы
    let mut pins: HashMap<(i32, i32), (usize, f32)> = HashMap::new();
    for (index, ring) in rings.list.iter().enumerate() {
        for &road in &ring.roads {
            for &point in &roads[road].points {
                pins.insert(node_key(point), (index, ring.param(point).0));
            }
        }
    }
    // подходы
    for (index, road) in roads.iter().enumerate() {
        if rings.of_road[index].is_some()
            || !road.oneway
            || road.class != RoadClass::Street
            || road.carves_navmesh()
            || paths[index].len() < 2
        {
            continue;
        }
        let mut path = paths[index].to_vec();
        let mut bent = false;
        for entry in [true, false] {
            let end = if entry { path[path.len() - 1] } else { path[0] };
            let Some(&(ring, t)) = pins.get(&node_key(end)) else {
                continue;
            };
            let ring = &rings.list[ring];
            let (travel, outward) = (ring.travel(t), ring.outward(t));
            let (sin, cos) = ENTRY_ANGLE.sin_cos();
            // въезд приходит в узел снаружи, съезд уходит из него наружу;
            // съезд строится как въезд по развёрнутому подходу
            let arrival = if entry {
                travel * cos - outward * sin
            } else {
                -(travel * cos + outward * sin)
            };
            if !entry {
                path.reverse();
            }
            let reach = (APPROACH_SHARE * ring.mean_radius()).clamp(APPROACH_MIN, APPROACH_MAX);
            if let Some(new) = bend_approach(&path, arrival, reach, nodes) {
                path = new;
                bent = true;
            }
            if !entry {
                path.reverse();
            }
        }
        if bent {
            paths[index] = Cow::Owned(path);
        }
    }
    let widths: Vec<f32> = rings
        .list
        .iter()
        .map(|ring| {
            ring.roads
                .iter()
                .map(|&arc| roads[arc].width)
                .fold(0.0, f32::max)
        })
        .collect();
    for (index, road) in roads.iter().enumerate() {
        if rings.of_road[index].is_some()
            || road.class != RoadClass::Street
            || road.carves_navmesh()
        {
            continue;
        }
        for (ring, &width) in rings.list.iter().zip(&widths) {
            rings
                .webs
                .extend(webs_along(&paths[index], road.width / 2.0, ring, width));
        }
    }
    rings
}

/// Насколько улица должна идти вдоль кольца, чтобы щель между ними была
/// перепонкой: косинус угла между ними. Улица, упёршаяся в кольцо поперёк,
/// щели вдоль него не оставляет — там угол бордюра.
const WEB_ALONG: f32 = 0.7;

/// Асфальт между улицей `path` (полуширина `half`) и осью кольца — на каждом
/// отрезке, где улица идёт вдоль кольца снаружи, а кромки их разошлись не
/// больше чем на [`WEB_GAP`]. Дальше между ними настоящий островок. Отрезок,
/// где кромки не расходятся вовсе, перепонки не даёт: щели нет. Так
/// закрывается и щель у подхода, и щель у обходного съезда, который идёт
/// вдоль кольца, не заходя в него (Тула, витрина 04, юго-восток), — без
/// перепонки в щели серпом проступал тротуар.
fn webs_along(path: &[Vec2], half: f32, ring: &Ring, ring_width: f32) -> Vec<Vec<Vec2>> {
    let touch = half + ring_width / 2.0;
    let reach = touch + WEB_GAP;
    let (low, high) = ring.path.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(low, high), &point| (low.min(point), high.max(point)),
    );
    let (low, high) = (low - reach, high + reach);
    let mut webs = Vec::new();
    let mut near: Vec<Vec2> = Vec::new();
    let mut far: Vec<Vec2> = Vec::new();
    let mut open = false;
    let mut flush = |near: &mut Vec<Vec2>, far: &mut Vec<Vec2>, open: &mut bool| {
        if near.len() >= 3 && *open {
            let mut web = std::mem::take(near);
            web.extend(far.drain(..).rev());
            webs.push(web);
        }
        near.clear();
        far.clear();
        *open = false;
    };
    for link in path.windows(2) {
        let (from, to) = (link[0], link[1]);
        let span = to - from;
        // звено целиком мимо рамки кольца
        if span.length_squared() < 1e-6
            || from.max(to).cmplt(low).any()
            || from.min(to).cmpgt(high).any()
        {
            flush(&mut near, &mut far, &mut open);
            continue;
        }
        let direction = span.normalize();
        let steps = (span.length() / SAMPLE_STEP).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let point = from + span * (step as f32 / steps as f32);
            let (onto, distance) = nearest_on_path(&ring.path, point)
                .map_or((point, f32::INFINITY), |(onto, _)| {
                    (onto, onto.distance(point))
                });
            let along = direction.dot(ring.tangent(ring.param(onto).0)).abs() >= WEB_ALONG;
            let outside = point.distance_squared(ring.center) >= onto.distance_squared(ring.center);
            if !along || distance > reach || (distance > touch && !outside) {
                flush(&mut near, &mut far, &mut open);
                continue;
            }
            if near.last() == Some(&point) {
                continue;
            }
            open |= distance > touch;
            near.push(point);
            far.push(onto);
        }
    }
    flush(&mut near, &mut far, &mut open);
    webs
}

/// Петли из дуг колец: замкнутый way — петля сам по себе, открытые дуги
/// сцепляются конец к началу, пока не вернутся к первой.
fn chains(roads: &[RoadLine]) -> Vec<Vec<usize>> {
    let arcs: Vec<usize> = (0..roads.len())
        .filter(|&road| is_arc(&roads[road]))
        .collect();
    let mut by_start: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for &road in &arcs {
        by_start
            .entry(node_key(roads[road].points[0]))
            .or_default()
            .push(road);
    }
    let mut used = vec![false; roads.len()];
    let mut chains = Vec::new();
    for &first in &arcs {
        if used[first] {
            continue;
        }
        if is_ring(&roads[first].points) {
            used[first] = true;
            chains.push(vec![first]);
            continue;
        }
        let start = node_key(roads[first].points[0]);
        let mut chain = vec![first];
        let mut closed = false;
        for _ in 0..MAX_ARCS {
            let last = chain[chain.len() - 1];
            let end = node_key(roads[last].points[roads[last].points.len() - 1]);
            if end == start {
                closed = true;
                break;
            }
            let next = by_start.get(&end).and_then(|next| {
                next.iter()
                    .copied()
                    .find(|road| !used[*road] && !chain.contains(road))
            });
            match next {
                Some(next) => chain.push(next),
                None => break,
            }
        }
        if closed {
            for &road in &chain {
                used[road] = true;
            }
            chains.push(chain);
        }
    }
    chains
}

/// Вписать фигуру в петлю `chain`: центр и оси — по моментам петли,
/// выбранной равномерно по длине, полуоси — наименьшими квадратами в осях.
/// `None` — петля не кольцо.
fn fit(roads: &[RoadLine], chain: &[usize], nodes: &RoadNodes) -> Option<Ring> {
    let mut outline: Vec<Vec2> = Vec::new();
    for &road in chain {
        let skip = usize::from(!outline.is_empty());
        outline.extend(roads[road].points.iter().skip(skip));
    }
    if outline.len() < 4 {
        return None;
    }
    let mut samples: Vec<Vec2> = Vec::new();
    for link in outline.windows(2) {
        let steps = (link[0].distance(link[1]) / SAMPLE_STEP).ceil().max(1.0) as usize;
        samples.extend((0..steps).map(|step| link[0].lerp(link[1], step as f32 / steps as f32)));
    }
    let count = samples.len() as f32;
    let center = samples.iter().sum::<Vec2>() / count;
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for point in &samples {
        let offset = *point - center;
        xx += offset.x * offset.x;
        yy += offset.y * offset.y;
        xy += offset.x * offset.y;
    }
    let axis = Vec2::from_angle(0.5 * (2.0 * xy).atan2(xx - yy));
    // A·u² + B·v² = 1 наименьшими квадратами
    let (mut uuuu, mut uuvv, mut vvvv, mut uu, mut vv) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for point in &samples {
        let offset = *point - center;
        let (u, v) = (offset.dot(axis), offset.dot(axis.perp()));
        let (u2, v2) = (u * u, v * v);
        uuuu += u2 * u2;
        uuvv += u2 * v2;
        vvvv += v2 * v2;
        uu += u2;
        vv += v2;
    }
    let det = uuuu * vvvv - uuvv * uuvv;
    if det.abs() < 1e-6 {
        return None;
    }
    let a = (uu * vvvv - vv * uuvv) / det;
    let b = (vv * uuuu - uu * uuvv) / det;
    if a <= 0.0 || b <= 0.0 {
        return None;
    }
    let mut radii = Vec2::new(1.0 / a.sqrt(), 1.0 / b.sqrt());
    if (radii.x - radii.y).abs() < CIRCLE_SHARE * radii.max_element() {
        let radius = samples
            .iter()
            .map(|point| point.distance(center))
            .sum::<f32>()
            / count;
        radii = Vec2::splat(radius);
    }
    if radii.min_element() < MIN_RADIUS {
        return None;
    }
    // ход: знак площади петли
    let area: f32 = outline
        .windows(2)
        .map(|link| link[0].perp_dot(link[1]))
        .sum();
    let mut ring = Ring {
        roads: chain.to_vec(),
        center,
        axis,
        radii,
        ccw: area > 0.0,
        pins: Vec::new(),
        path: Vec::new(),
    };
    let slack = FIT_SLACK.max(FIT_SHARE * radii.min_element());
    let off = outline.iter().any(|point| {
        let (_, scale) = ring.param(*point);
        let reach = point.distance(center);
        (reach - reach / scale.max(1e-4)).abs() > slack
    });
    if off {
        return None;
    }
    // узлы: общие вершины и концы открытых дуг (они общие с соседней дугой)
    let mut pins: Vec<(f32, f32)> = Vec::new();
    for &road in chain {
        let points = &roads[road].points;
        let open = !is_ring(points);
        for (index, &point) in points.iter().enumerate() {
            let end = index == 0 || index == points.len() - 1;
            if (open && end) || nodes.is_shared(point) {
                pins.push(ring.param(point));
            }
        }
    }
    pins.sort_by(|a, b| a.0.total_cmp(&b.0));
    pins.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
    ring.pins = pins;
    Some(ring)
}

/// Дуга подхода `path`, идущего **к** узлу кольца в его последней точке:
/// последние `reach` метров заменяет кривая, касательная к подходу и
/// приходящая в узел по `arrival`. `None` — подход и так приходит под нужным
/// углом, или места под дугу нет: дальше по нему узел с кем-то ещё.
fn bend_approach(path: &[Vec2], arrival: Vec2, reach: f32, nodes: &RoadNodes) -> Option<Vec<Vec2>> {
    let end = *path.last()?;
    // пройденное от торца до каждой вершины
    let mut back = vec![0.0; path.len()];
    for index in (0..path.len() - 1).rev() {
        back[index] = back[index + 1] + path[index].distance(path[index + 1]);
    }
    let length = back[0];
    let point_at = |distance: f32| point_back(path, &back, distance);
    let heading = (end - point_at(HEADING_BASE.min(length))).try_normalize()?;
    if heading.angle_to(arrival).abs() < BEND_MIN_ANGLE {
        return None;
    }
    let pinned = (1..path.len() - 1)
        .rev()
        .find(|&index| nodes.is_shared(path[index]))
        .map_or(f32::INFINITY, |index| back[index] - PIN_MARGIN);
    let reach = reach.min(APPROACH_LENGTH_SHARE * length).min(pinned);
    if reach < BEND_MIN_LENGTH {
        return None;
    }
    let from = point_at(reach);
    let travel = (point_at((reach - 0.5).max(0.0)) - from).try_normalize()?;
    let mut bent: Vec<Vec2> = path
        .iter()
        .zip(&back)
        .filter(|(_, distance)| **distance > reach + 0.05)
        .map(|(point, _)| *point)
        .collect();
    bent.extend(curve(
        LaneEnd {
            point: from,
            travel,
            offset: 0.0,
        },
        LaneEnd {
            point: end,
            travel: arrival,
            offset: 0.0,
        },
    ));
    Some(bent)
}

/// Точка ломаной в `distance` метрах от её конца (`back` — пройденное от
/// конца до каждой вершины).
fn point_back(path: &[Vec2], back: &[f32], distance: f32) -> Vec2 {
    for index in (0..path.len() - 1).rev() {
        if back[index] >= distance {
            let span = (back[index] - back[index + 1]).max(1e-6);
            return path[index + 1].lerp(path[index], (distance - back[index + 1]) / span);
        }
    }
    path[0]
}

#[cfg(test)]
mod tests;
