//! Выпрямленный Y-подход — правка разбора, а не рисования.
//!
//! У Y-подхода с хвостом ([`y_legs`], [`tail_walk`]) второй ногой служат
//! чужие улицы: первая идёт через развилку дальше, вторая подхватывает её на
//! стыке и доводит до кольца. Так замаплен север кольца Рязани (витрина 05):
//! развилка в десяти метрах от оси кольца, стык — в трёх с половиной, и
//! хвост от стыка идёт вдоль кольца вплотную. Рисованием этого не поправить:
//! узлы на месте, хвост двусторонний во всю ширину улиц, и между ногой,
//! хвостом и кольцом островку не остаётся места — асфальт ложился одним
//! наплывом от развилки до кольца.
//!
//! Поэтому подход **выпрямляется в данных**, до сечений: развилка и стык
//! сливаются в один узел, отнесённый от кольца на луче из середины дуги
//! между узлами ног ([`fork_point`]), обе ноги идут из него в свои узлы
//! прямо, а конец второй улицы от стыка отрезается в свой way. Дальше это
//! обычный «Y» из двух ног — въезд и съезд в полосу по касательной, клин
//! между ними — веер с островком, — и навмеш, двери и кварталы видят ту же
//! развилку, что лента. Правдоподобие, а не точность: развилки нет там, где
//! её поставил картограф, зато подход читается как подход.

use std::borrow::Cow;
use std::f32::consts::{PI, TAU};
use std::time::{Duration, Instant};

use bevy::platform::collections::HashSet;
use bevy::prelude::Vec2;

use super::{LEG_MAX, Ring, RoadNodes, Tail, TailWalk, fit_rings, node_key, y_legs};
use crate::map::osm::RoadLine;

/// Как далеко от оси кольца встаёт развилка: доля хорды между узлами ног, но
/// не ближе [`FORK_MIN`], м. У южного и восточного подходов того же кольца
/// Рязани, замапленных «Y» из двух way, развилка стоит в 11 м при хорде в
/// 20 м, и островок на них виден едва-едва; 0.7 хорды даёт ему места.
const FORK_SHARE: f32 = 0.7;
const FORK_MIN: f32 = 10.0;
/// Дальше этого, м, развилка и стык не переносятся: это уже не поправка
/// подхода, а другая дорога.
const SHIFT_MAX: f32 = 20.0;

/// Что сделал [`straighten_tails`]: сколько подходов выпрямлено и за сколько.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Straightened {
    pub tails: usize,
    pub elapsed: Duration,
}

/// Выпрямить Y-подходы с хвостом ([модуль](self)). Дорога, отрезанная от
/// второй улицы, добавляется в конец `roads`.
pub fn straighten_tails(roads: &mut Vec<RoadLine>) -> Straightened {
    let started = Instant::now();
    let plans: Vec<Plan> = {
        let nodes = RoadNodes::new(roads);
        let mut paths: Vec<Cow<[Vec2]>> = roads
            .iter()
            .map(|road| Cow::Borrowed(road.points.as_slice()))
            .collect();
        let (rings, pins) = fit_rings(roads, &nodes, &mut paths);
        let (_, tails) = y_legs(roads, &rings, &pins, &nodes, &paths);
        tails
            .iter()
            .filter_map(|tail| plan(roads, &rings.list[tail.ring], &nodes, tail))
            .collect()
    };
    let mut touched: HashSet<usize> = HashSet::new();
    let mut moved: HashSet<(i32, i32)> = HashSet::new();
    let mut done = 0;
    for plan in plans {
        let roads_of = [plan.leg, plan.first, plan.second];
        let keys = [node_key(plan.fork), node_key(plan.joint)];
        if roads_of.iter().any(|road| touched.contains(road))
            || keys.iter().any(|key| moved.contains(key))
        {
            continue;
        }
        apply(roads, &plan);
        touched.extend(roads_of);
        moved.insert(node_key(plan.to));
        done += 1;
    }
    if done > 0 {
        roads.retain(|road| road.points.len() >= 2);
    }
    Straightened {
        tails: done,
        elapsed: started.elapsed(),
    }
}

/// Что выпрямить: хвост ([`TailWalk`]), нога и куда встанет развилка.
struct Plan {
    leg: usize,
    first: usize,
    second: usize,
    fork: Vec2,
    joint: Vec2,
    foot: Vec2,
    /// Новая развилка — туда сходятся и развилка, и стык.
    to: Vec2,
}

/// Новая развилка Y-подхода с ногами в узлах `a` и `b` кольца: на луче из
/// середины дуги между ними, в [`FORK_SHARE`] хорды от оси наружу.
fn fork_point(ring: &Ring, a: Vec2, b: Vec2) -> Vec2 {
    let (t_a, t_b) = (ring.param(a).0, ring.param(b).0);
    let half = ((t_b - t_a + PI).rem_euclid(TAU) - PI) / 2.0;
    let middle = t_a + half;
    let reach = (FORK_SHARE * a.distance(b)).max(FORK_MIN);
    ring.point(middle) + ring.outward(middle) * reach
}

/// План выпрямления хвоста `tail` или `None`, если трогать нельзя: вершина,
/// которая уйдёт, общая с кем-то ещё; вторая улица кончается не в кольце;
/// развилка уехала бы дальше [`SHIFT_MAX`] или нога вышла бы длиннее
/// [`LEG_MAX`].
fn plan(roads: &[RoadLine], ring: &Ring, nodes: &RoadNodes, tail: &Tail) -> Option<Plan> {
    let TailWalk {
        fork,
        first,
        joint,
        second,
        foot,
        ..
    } = tail.walk;
    let at = |road: usize, point: Vec2| {
        roads[road]
            .points
            .iter()
            .position(|other| node_key(*other) == node_key(point))
    };
    // между двумя вершинами дороги никто не примыкает
    let free_between = |road: usize, a: usize, b: usize| {
        let (low, high) = (a.min(b), a.max(b));
        roads[road].points[low + 1..high]
            .iter()
            .all(|point| !nodes.is_shared(*point))
    };
    let (fork_at, joint_at) = (at(first, fork)?, at(first, joint)?);
    let (turn_at, foot_at) = (at(second, joint)?, at(second, foot)?);
    let last = roads[second].points.len() - 1;
    let leg_last = roads[tail.leg].points.len() - 1;
    if !(foot_at == 0 || foot_at == last)
        || !free_between(first, fork_at, joint_at)
        || !free_between(second, turn_at, foot_at)
        || !free_between(tail.leg, 0, leg_last)
    {
        return None;
    }
    let to = fork_point(ring, tail.leg_foot, foot);
    let fits = to.distance(fork) <= SHIFT_MAX
        && to.distance(joint) <= SHIFT_MAX
        && to.distance(foot) <= LEG_MAX
        && to.distance(tail.leg_foot) <= LEG_MAX;
    fits.then_some(Plan {
        leg: tail.leg,
        first,
        second,
        fork,
        joint,
        foot,
        to,
    })
}

fn apply(roads: &mut Vec<RoadLine>, plan: &Plan) {
    let at = |points: &[Vec2], point: Vec2| {
        points
            .iter()
            .position(|other| node_key(*other) == node_key(point))
            .expect("узел плана — на своей дороге")
    };
    // первая улица: кусок от развилки до стыка сходится в новую развилку
    let points = &mut roads[plan.first].points;
    let (fork_at, joint_at) = (at(points, plan.fork), at(points, plan.joint));
    points.drain(fork_at.min(joint_at) + 1..fork_at.max(joint_at));
    // вторая: от стыка до кольца — нога прямо, отдельным way, если улица
    // идёт за стык дальше
    let points = &roads[plan.second].points;
    let (turn_at, foot_at) = (at(points, plan.joint), at(points, plan.foot));
    let (leg, rest) = if turn_at < foot_at {
        (vec![plan.joint, plan.foot], points[..=turn_at].to_vec())
    } else {
        (vec![plan.foot, plan.joint], points[turn_at..].to_vec())
    };
    if rest.len() >= 2 {
        let piece = RoadLine {
            points: leg,
            ..roads[plan.second].clone()
        };
        roads[plan.second].points = rest;
        roads.push(piece);
    } else {
        roads[plan.second].points = leg;
    }
    // своя нога — прямо
    let points = &mut roads[plan.leg].points;
    let ends = [points[0], points[points.len() - 1]];
    *points = ends.to_vec();
    // развилка и стык — один узел, у всех дорог, что через них шли
    let (fork, joint) = (node_key(plan.fork), node_key(plan.joint));
    for road in roads.iter_mut() {
        let mut hit = false;
        for point in &mut road.points {
            let key = node_key(*point);
            if key == fork || key == joint {
                *point = plan.to;
                hit = true;
            }
        }
        if hit {
            road.points.dedup_by(|a, b| node_key(*a) == node_key(*b));
        }
    }
}

#[cfg(test)]
mod tests;
