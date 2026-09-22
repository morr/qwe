//! Нарисованная ось улицы: одна кривая на всю улицу, а не по куску на way.
//!
//! Chaikin по каждому way в отдельности оставлял угол на каждом шве OSM:
//! концы way закреплены, и две соседние ленты сходились в узле изломом. А
//! общий узел с другой дорогой не срезался вовсе — сквозная улица ломалась на
//! каждом перекрёстке. Здесь ось строится по улице целиком
//! ([`RoadNetwork`]):
//!
//! 1. ways улицы сшиваются в одну ломаную, и она **упрощается** до допуска
//!    [`Curve::simplify`] — дрожание вершин OSM уходит;
//! 2. каждая оставшаяся вершина заменяется **дугой**, касательной к обоим
//!    звеньям. Радиус — [`Curve::radius`], но не больше, чем уводит ось от
//!    OSM на [`Curve::deviation`], и не меньше полуширины: у
//!    дуги меньшего радиуса внутренний край ленты складывается;
//! 3. **узел, где сходятся три дороги и больше, остаётся на месте**: по нему
//!    находят друг друга скругления бордюров, разрывы разметки, стежки и
//!    клинья. Сквозная пара проходит его прямым отрезком по биссектрисе
//!    ([`through_pad`]), а излом уходит на две дуги по концам отрезка: край
//!    ленты у самого узла остаётся прямым, и скругление бордюра к боковому
//!    плечу строится, как раньше. Излом круче [`THROUGH_MAX_BEND`] в таком узле
//!    остаётся углом: это уже поворот, а не сквозная улица;
//! 4. кривая режется обратно на ways **в ближайшей к узлу шва точке дуги**,
//!    и каждый кусок получает порядок точек своего way.
//!
//! Мосты и арки в улицу не сглаживаются: их точки читает навмеш, и ось
//! моста строится по-прежнему ([`centerline`]). Они делят улицу на пробеги,
//! концы пробега закреплены. Дорожки ни в какой улице не лежат и тоже идут
//! по [`centerline`].

use std::borrow::Cow;
use std::f32::consts::PI;

use bevy::prelude::*;

use super::centerline;
use super::network::pairs::Pairs;
use super::network::{self, RoadNetwork, RoadNodes, StreetWay};
use super::rings::{self, Rings};
use super::shape::RoadShape;
use crate::map::along::simplify;
use crate::map::meshing::arc_steps;
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::smooth::Smoothing;

/// Доля допуска оси, которую берёт упрощение, — остальное берёт дуга: из
/// трёх метров по умолчанию метр уходит на дрожание вершин OSM, два — на
/// скругление.
const SIMPLIFY_SHARE: f32 = 1.0 / 3.0;
/// Радиус дуги на изломе на метр допуска, м: 30 м при трёх метрах.
const RADIUS_PER_METER: f32 = 10.0;
/// Излом в общем узле, до которого сквозная пара проходит его плавно, рад.
/// Тот же предел, что у склейки улиц ([`network::MAX_BEND`]).
const THROUGH_MAX_BEND: f32 = network::MAX_BEND;
/// Полудлина прямого отрезка, которым улица проходит закреплённый узел, м:
/// дуга на его конце оставляет узлу [`KERB_STRAIGHT`] прямого края.
const THROUGH_RUN: f32 = 24.0;
/// Излом в закреплённом узле, который остаётся углом, рад: его кроет асфальт
/// перекрёстка, а прямой отрезок через узел укоротил бы прямой край, на
/// котором лежит скругление бордюра.
const THROUGH_MIN_BEND: f32 = 4.0 * PI / 180.0;
/// Сколько прямого края дуги оставляют закреплённому узлу, м: столько берёт
/// скругление бордюра к поперечной улице (`roads/corners.rs`).
const KERB_STRAIGHT: f32 = 12.0;
/// Излом мельче этого не скругляется, рад: дуга была бы в сантиметр.
const MIN_BEND: f32 = 0.5 * PI / 180.0;

/// Сглаживание оси при допуске `tolerance` — насколько ось может уйти от
/// точек OSM, м (ручка `Curve tolerance`, [`RoadShape::curve_tolerance`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    /// Допуск упрощения, м: вершина ближе к хорде соседей — дрожание OSM.
    pub simplify: f32,
    /// Насколько дуга может увести ось от вершины OSM, м. Вместе с допуском
    /// упрощения — весь допуск.
    pub deviation: f32,
    /// Радиус дуги на изломе, м, — пока его не ограничит `deviation`.
    pub radius: f32,
}

impl Curve {
    /// `None` — нулевой допуск: ось идёт по точкам OSM.
    pub fn of(tolerance: f32) -> Option<Self> {
        (tolerance > 0.0).then_some(Self {
            simplify: tolerance * SIMPLIFY_SHARE,
            deviation: tolerance * (1.0 - SIMPLIFY_SHARE),
            radius: tolerance * RADIUS_PER_METER,
        })
    }

    /// Ступень сглаживания Chaikin'а для того, что по улице не идёт
    /// (дорожки, мосты): есть допуск — лёгкое, нет — никакого.
    pub fn smoothing(curve: Option<Self>) -> Smoothing {
        if curve.is_some() {
            Smoothing::Light
        } else {
            Smoothing::Off
        }
    }
}

/// Осевые всех дорог, по индексу дороги.
pub struct Axes<'a> {
    pub paths: Vec<Cow<'a, [Vec2]>>,
    /// Швы между ways, пройденные одной кривой.
    pub seams: usize,
    /// Изломы, на которые не хватило звеньев для радиуса в полуширину.
    pub tight: usize,
    /// Парные половины разделённых улиц — уже разведённые по этим осям
    /// (`roads/network/pairs.rs`).
    pub pairs: Pairs,
    /// Кольца, нарисованные гладкой фигурой (`roads/rings.rs`); без
    /// сглаживания — ни одного.
    pub rings: Rings,
}

/// Осевые, по которым строятся ленты, ряды машин и полоса тротуара.
///
/// `network` — улицы этих же `roads`; если она собрана не по ним (карта из
/// теста, без разбора), улицы собираются здесь.
pub fn street_axes<'a>(
    roads: &'a [RoadLine],
    network: &RoadNetwork,
    nodes: &RoadNodes,
    shape: &RoadShape,
) -> Axes<'a> {
    let curve = Curve::of(shape.curve_tolerance());
    let mut smoothed: Vec<Option<Vec<Vec2>>> = vec![None; roads.len()];
    let (mut seams, mut tight) = (0, 0);
    let local;
    let network = if network.covers(roads.len()) {
        network
    } else {
        local = RoadNetwork::new(roads);
        &local
    };
    if let Some(curve) = curve {
        for street in &network.streets {
            let excluded = |way: &StreetWay| roads[way.road].carves_navmesh();
            let whole = street.closed && !street.ways.iter().any(excluded);
            for run in street.ways.split(excluded) {
                if run.is_empty() {
                    continue;
                }
                let found = smooth_run(roads, run, whole, nodes, curve, &mut smoothed);
                seams += found.0;
                tight += found.1;
            }
        }
    }
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .zip(smoothed)
        .map(|(road, path)| match path {
            Some(path) => Cow::Owned(path),
            None => centerline(road, Curve::smoothing(curve), nodes),
        })
        .collect();
    // половины разделённых улиц — на постоянный зазор, по уже гладким осям
    let mut pairs = Pairs::new(roads, &paths, shape.median_gap());
    pairs.align(&mut paths, roads, network, nodes);
    // кольца — эллипсом, подходы к ним — по касательной; после разводки пар:
    // половины подхода гнутся у самого кольца, где пара уже разошлась
    let rings = if curve.is_some() {
        rings::reshape(roads, nodes, &mut paths)
    } else {
        Rings::default()
    };
    Axes {
        paths,
        seams,
        tight,
        pairs,
        rings,
    }
}

/// Сшитая ломаная пробега: точки, полуширина и закреплённость в каждой
/// вершине, вершины швов.
struct Run {
    points: Vec<Vec2>,
    halves: Vec<f32>,
    pinned: Vec<bool>,
    /// Вершина, с которой начинается way `k`; у кольца первая — 0.
    starts: Vec<usize>,
}

impl Run {
    fn stitch(roads: &[RoadLine], run: &[StreetWay], closed: bool, nodes: &RoadNodes) -> Self {
        let mut this = Run {
            points: Vec::new(),
            halves: Vec::new(),
            pinned: Vec::new(),
            starts: Vec::new(),
        };
        for (k, way) in run.iter().enumerate() {
            let road = &roads[way.road];
            let half = road.width / 2.0;
            let mut own = road.points.clone();
            if way.reversed {
                own.reverse();
            }
            if k == 0 {
                this.starts.push(0);
                this.push(own[0], half, false);
            } else {
                // шов: закреплён, если в нём сходится кто-то кроме соседей
                let joint = this.points.len() - 1;
                let previous = run[k - 1].road;
                this.pinned[joint] = pins(roads, nodes, this.points[joint], &[way.road, previous]);
                this.halves[joint] = this.halves[joint].max(half);
                this.starts.push(joint);
            }
            for &point in &own[1..own.len() - 1] {
                this.push(point, half, pins(roads, nodes, point, &[way.road]));
            }
            this.push(own[own.len() - 1], half, false);
        }
        let last = this.points.len() - 1;
        if closed {
            // последняя вершина повторяет первую: кольцо идёт по циклу без неё
            this.points.pop();
            this.halves.pop();
            this.pinned.pop();
            let (first, previous) = (run[0].road, run[run.len() - 1].road);
            this.pinned[0] = pins(roads, nodes, this.points[0], &[first, previous]);
        } else {
            this.pinned[0] = true;
            this.pinned[last] = true;
        }
        this
    }

    fn push(&mut self, point: Vec2, half: f32, pinned: bool) {
        self.points.push(point);
        self.halves.push(half);
        self.pinned.push(pinned);
    }
}

/// Закреплён ли узел `point` улицы: в нём сходится проезжая дорога (улица
/// или проезд) не из `own`. Узел с пешеходной дорожкой — нет: дорожка
/// кончается под асфальтом улицы, скругления бордюра и разрывы разметки её не
/// касаются, а закреплённый переход в 15 м от перекрёстка гнул ось так, что
/// скруглению там не хватало прямого края.
fn pins(roads: &[RoadLine], nodes: &RoadNodes, point: Vec2, own: &[usize]) -> bool {
    nodes
        .roads_at(point)
        .iter()
        .any(|&other| !own.contains(&other) && roads[other].class == RoadClass::Street)
}

/// Сглаживает пробег улицы и раскладывает его по ways. Возвращает число
/// свободных швов и тесных изломов.
fn smooth_run(
    roads: &[RoadLine],
    run: &[StreetWay],
    closed: bool,
    nodes: &RoadNodes,
    curve: Curve,
    smoothed: &mut [Option<Vec<Vec2>>],
) -> (usize, usize) {
    if run.iter().any(|way| roads[way.road].points.len() < 2) {
        return (0, 0);
    }
    let stitched = Run::stitch(roads, run, closed, nodes);
    let count = stitched.points.len();
    if count < if closed { 3 } else { 2 } {
        return (0, 0);
    }
    let mut keep = stitched.pinned.clone();
    for &start in &stitched.starts {
        keep[start] = true;
    }
    keep[0] = true;
    let kept = simplify(&stitched.points, closed, curve.simplify, |index| {
        keep[index]
    });
    let kept: Vec<Vertex> = kept
        .iter()
        .map(|&index| Vertex {
            at: stitched.points[index],
            half: stitched.halves[index],
            pinned: stitched.pinned[index],
            source: Some(index),
        })
        .collect();

    // закреплённый узел, который улица проходит насквозь, — прямой отрезок
    // по биссектрисе, а излом уходит на две дуги по его концам
    let mut vertices: Vec<Vertex> = Vec::with_capacity(kept.len());
    for (q, vertex) in kept.iter().enumerate() {
        let pad = neighbours(q, kept.len(), closed)
            .filter(|_| vertex.pinned)
            .and_then(|(before, after)| {
                through_pad(kept[before].at, vertex.at, kept[after].at, curve.deviation)
            });
        match pad {
            Some([start, end]) => {
                let free = |at| Vertex {
                    at,
                    pinned: false,
                    source: None,
                    ..*vertex
                };
                vertices.extend([free(start), *vertex, free(end)]);
            }
            None => vertices.push(*vertex),
        }
    }

    let mut out: Vec<Vec2> = Vec::new();
    // выход вершины `vertices[q]` — `spans[q]` в `out`
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(vertices.len());
    let mut tight = 0;
    for (q, vertex) in vertices.iter().enumerate() {
        let start = out.len();
        match neighbours(q, vertices.len(), closed).filter(|_| !vertex.pinned) {
            Some((before, after)) => {
                let corner = Corner {
                    before: vertices[before].at,
                    at: vertex.at,
                    after: vertices[after].at,
                    pinned: [vertices[before].pinned, vertices[after].pinned],
                    half: vertex.half,
                    curve,
                };
                if corner.fillet(&mut out) {
                    tight += 1;
                }
            }
            None => out.push(vertex.at),
        }
        spans.push((start, out.len() - 1));
    }
    if closed {
        out.push(out[0]);
    }

    // место разреза у каждого шва — точка его дуги, ближайшая к узлу
    let cut_of = |vertex: usize| {
        let q = vertices
            .iter()
            .position(|kept| kept.source == Some(vertex))
            .expect("a seam is always kept");
        let (from, to) = spans[q];
        let node = stitched.points[vertex];
        (from..=to)
            .min_by(|&a, &b| out[a].distance(node).total_cmp(&out[b].distance(node)))
            .unwrap_or(from)
    };
    let cuts: Vec<usize> = stitched.starts.iter().map(|&start| cut_of(start)).collect();
    for (k, way) in run.iter().enumerate() {
        let mut piece: Vec<Vec2> = if k + 1 < run.len() {
            out[cuts[k]..=cuts[k + 1]].to_vec()
        } else if closed {
            // последний way кольца уходит через шов к началу первого; у
            // кольца из одного way это и есть всё кольцо от его разреза
            let mut piece = out[cuts[k]..].to_vec();
            piece.extend_from_slice(&out[1..=cuts[0]]);
            piece
        } else {
            out[cuts[k]..].to_vec()
        };
        piece.dedup();
        if piece.len() < 2 {
            return (0, 0);
        }
        if way.reversed {
            piece.reverse();
        }
        smoothed[way.road] = Some(piece);
    }
    let seams = stitched
        .starts
        .iter()
        .filter(|&&start| (start > 0 || closed && run.len() > 1) && !stitched.pinned[start])
        .count();
    (seams, tight)
}

/// Сколько звена длиной `length` может взять дуга с одного его конца. К
/// свободному соседу — половина. К закреплённому узлу — сколько оставит ему
/// [`KERB_STRAIGHT`] прямого края, но не больше половины и не меньше
/// четверти звена: на узле кладётся скругление бордюра, а оно ложится только
/// на прямой край ленты.
fn room(length: f32, pinned: bool) -> f32 {
    if pinned {
        (length - KERB_STRAIGHT).max(length / 4.0).min(length / 2.0)
    } else {
        length / 2.0
    }
}

/// Излом оси: вершина `at` между соседями по упрощённой ломаной.
struct Corner {
    before: Vec2,
    at: Vec2,
    after: Vec2,
    /// Закреплён ли сосед до и после.
    pinned: [bool; 2],
    half: f32,
    curve: Curve,
}

impl Corner {
    /// Направления звеньев и излом между ними; `None` — излома нет.
    fn bend(&self) -> Option<(Vec2, Vec2, f32)> {
        let incoming = (self.at - self.before).try_normalize()?;
        let outgoing = (self.after - self.at).try_normalize()?;
        let bend = incoming.angle_to(outgoing).abs();
        (bend >= MIN_BEND).then_some((incoming, outgoing, bend))
    }

    /// Сколько звена от вершины берёт скругление радиуса `radius`: не больше
    /// половины каждого звена — вторая половина за соседом, — а у звена к
    /// закреплённому узлу ещё и оставляя ему прямой край ([`room`]).
    fn reach(&self, radius: f32, bend: f32) -> f32 {
        (radius * (bend / 2.0).tan())
            .min(room(self.at.distance(self.before), self.pinned[0]))
            .min(room(self.at.distance(self.after), self.pinned[1]))
    }

    /// Радиус дуги: ступень сглаживания, зажатая отклонением от вершины и
    /// снизу полушириной.
    fn radius(&self, bend: f32) -> f32 {
        let deviation = self.curve.deviation / (1.0 / (bend / 2.0).cos() - 1.0).max(f32::EPSILON);
        self.curve.radius.min(deviation).max(self.half)
    }

    /// Дуга, касательная к обоим звеньям. Да — звеньев не хватило, и радиус
    /// вышел меньше полуширины.
    fn fillet(&self, out: &mut Vec<Vec2>) -> bool {
        let Some((incoming, outgoing, bend)) = self.bend() else {
            out.push(self.at);
            return false;
        };
        let reach = self.reach(self.radius(bend), bend);
        let radius = reach / (bend / 2.0).tan();
        let turn = incoming.perp_dot(outgoing).signum();
        let start = self.at - incoming * reach;
        let centre = start + incoming.perp() * turn * radius;
        let steps = arc_steps(radius, bend);
        let arm = start - centre;
        for step in 0..=steps {
            let angle = turn * bend * step as f32 / steps as f32;
            out.push(centre + Vec2::from_angle(angle).rotate(arm));
        }
        radius < self.half * 0.99
    }
}

/// Вершина упрощённой ломаной. `source` — её номер в сшитой ломаной; у концов
/// прямого отрезка через узел ([`through_pad`]) его нет.
#[derive(Clone, Copy)]
struct Vertex {
    at: Vec2,
    half: f32,
    pinned: bool,
    source: Option<usize>,
}

/// Соседи вершины `q` из `count`; у конца открытого пробега их нет.
fn neighbours(q: usize, count: usize, closed: bool) -> Option<(usize, usize)> {
    if closed {
        Some(((q + count - 1) % count, (q + 1) % count))
    } else if q == 0 || q + 1 == count {
        None
    } else {
        Some((q - 1, q + 1))
    }
}

/// Концы прямого отрезка по биссектрисе, которым улица проходит закреплённый
/// узел `at`: бордюр перекрёстка ([`corners`](super::corners)) скругляется
/// только по прямому краю, и изогнутая у самого узла ось оставила бы его без
/// скругления. Отрезок уводит ось от звеньев OSM не дальше
/// `deviation` и берёт не больше половины каждого звена. `None` — излом
/// мельче [`THROUGH_MIN_BEND`] (его кроет перекрёсток) или круче
/// [`THROUGH_MAX_BEND`] (это поворот): тогда узел остаётся углом.
fn through_pad(before: Vec2, at: Vec2, after: Vec2, deviation: f32) -> Option<[Vec2; 2]> {
    let incoming = (at - before).try_normalize()?;
    let outgoing = (after - at).try_normalize()?;
    let bend = incoming.angle_to(outgoing).abs();
    if !(THROUGH_MIN_BEND..=THROUGH_MAX_BEND).contains(&bend) {
        return None;
    }
    let middle = (incoming + outgoing).normalize();
    let reach = (deviation / (bend / 2.0).sin())
        .min(THROUGH_RUN)
        .min(at.distance(before) / 2.0)
        .min(at.distance(after) / 2.0);
    Some([at - middle * reach, at + middle * reach])
}

#[cfg(test)]
mod tests;
