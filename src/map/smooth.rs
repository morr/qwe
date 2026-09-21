//! Сглаживание осевой по Chaikin — одно на все ленты карты.
//!
//! Осевую сглаживают дороги, рельсы, трамвай, водотоки, ряд припаркованных
//! машин и зелёная полоса под аллеей. Само правило при этом одно: срезать
//! изломы круче [`MIN_SMOOTH_ANGLE`], зажав длину среза шириной ленты, — и
//! жило оно посреди `map/roads.rs`, между палитрой дорожных слоёв и сборкой
//! мостовых теней, откуда его и тянули к себе пять чужих модулей.
//!
//! **Сглаживание работает на копии.** `RoadLine::points` и `width` несут на
//! себе навмеш, арки, посадку деревьев и генератор входов; ни одному из них
//! нельзя поехать оттого, что поменялся рисунок. Поэтому [`smooth_path`]
//! возвращает `Cow`: без сглаживания — заимствованные точки OSM, без
//! копирования вовсе.

use std::borrow::Cow;
use std::f32::consts::PI;

use bevy::prelude::*;

/// Изломы мельче Chaikin не срезает: прямые участки обязаны остаться точками
/// OSM, иначе сглаживание съедает и без того редкую геометрию длинных улиц.
const MIN_SMOOTH_ANGLE: f32 = 10.0 * PI / 180.0;
/// Доля сегмента, отрезаемая с каждой стороны излома (классический Chaikin).
const CHAIKIN_CUT: f32 = 0.25;

/// Сколько раз осевая прогоняется через Chaikin перед построением ленты.
///
/// Имена вариантов — значения в `settings.toml` (`RoadStyle::smoothing`,
/// `TreeRowStyle::smoothing`), переименовывать их нельзя.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Smoothing {
    /// Осевая ровно по данным OSM — как в самом OSM, где углы остаются острыми.
    Off,
    /// Один проход: улица на повороте перестаёт ломаться под углом, а рисунок
    /// сети ещё держится там, где OSM ставил узлы.
    #[default]
    Light,
    Strong,
}

impl Smoothing {
    pub const ALL: [Self; 3] = [Self::Off, Self::Light, Self::Strong];

    fn iterations(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Light => 1,
            Self::Strong => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Light => "Light",
            Self::Strong => "Strong",
        }
    }
}

/// Сглаживание осевой на копии — общее для дорог, рельсов и зелёной полосы под
/// аллеей (`map::spawn`). Длина среза зажата шириной ленты, поэтому ширина
/// здесь параметр, а не константа.
pub fn smooth_path(points: &[Vec2], width: f32, smoothing: Smoothing) -> Cow<'_, [Vec2]> {
    smooth_pinned(points, width, smoothing, false, |_| false)
}

/// [`smooth_path`], не трогающее вершины, для которых `pinned` — да, и знающее
/// про кольцо. Зовёт его `roads::centerline`: у дороги закреплены концы арки и
/// узлы, общие с другими дорогами.
///
/// `closed` — замкнутый way (последняя точка повторяет первую). Такой путь
/// сглаживается **по циклу**: шов для Chaikin — обычный излом, а не пара
/// закреплённых концов. Иначе на шве оставался единственный несрезанный угол
/// кольца, а лента получала там два торцевых полудиска поверх собственного
/// асфальта.
pub(super) fn smooth_pinned(
    points: &[Vec2],
    width: f32,
    smoothing: Smoothing,
    closed: bool,
    pinned: impl Fn(Vec2) -> bool + Copy,
) -> Cow<'_, [Vec2]> {
    let iterations = smoothing.iterations();
    let least = if closed { 4 } else { 3 };
    if iterations == 0 || points.len() < least {
        return Cow::Borrowed(points);
    }
    let mut path = points.to_vec();
    for _ in 0..iterations {
        path = chaikin(&path, width, closed, pinned);
    }
    Cow::Owned(path)
}

/// Срезание углов по Chaikin: излом заменяется парой точек на прилежащих
/// сегментах. Срезаются только изломы круче [`MIN_SMOOTH_ANGLE`], а длина
/// среза зажата шириной дороги — иначе на длинных сегментах осевая уезжает от
/// данных OSM на десятки метров и дорога перестаёт совпадать с домами.
/// Концы пути и вершины, для которых `pinned` — да, закреплены; у кольца
/// (`closed`) концов нет — срезается каждый излом, шов в том числе.
fn chaikin(points: &[Vec2], width: f32, closed: bool, pinned: impl Fn(Vec2) -> bool) -> Vec<Vec2> {
    // у кольца последняя точка повторяет первую: идём по циклу без неё, а в
    // конце замыкаем обратно
    let ring = if closed {
        &points[..points.len() - 1]
    } else {
        points
    };
    let count = ring.len();
    let mut path = Vec::with_capacity(count * 2 + 1);
    if !closed {
        path.push(ring[0]);
    }
    let corners = if closed { 0..count } else { 1..count - 1 };
    for index in corners {
        let previous = ring[(index + count - 1) % count];
        let (corner, next) = (ring[index], ring[(index + 1) % count]);
        if pinned(corner) {
            path.push(corner);
            continue;
        }
        let (Some(incoming), Some(outgoing)) = (
            (corner - previous).try_normalize(),
            (next - corner).try_normalize(),
        ) else {
            path.push(corner);
            continue;
        };
        if incoming.angle_to(outgoing).abs() < MIN_SMOOTH_ANGLE {
            path.push(corner);
            continue;
        }
        let back = (corner.distance(previous) * CHAIKIN_CUT).min(width);
        let forward = (next.distance(corner) * CHAIKIN_CUT).min(width);
        path.push(corner - incoming * back);
        path.push(corner + outgoing * forward);
    }
    match (closed, path.first().copied()) {
        (true, Some(first)) => path.push(first),
        _ => path.push(ring[count - 1]),
    }
    path
}

#[cfg(test)]
mod tests;
