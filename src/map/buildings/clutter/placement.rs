//! Движок размещения коробок на кровле: рама дома, поиск места с проверкой на
//! контур и на занятость, и докуда у коробки дотянется тень, не съехав с
//! нарисованной кровли. Что именно ставить, решает родитель.

use bevy::prelude::*;

use super::PLACE_TRIES;
use super::RoofItem;
use crate::map::osm::PolyArea;
use crate::map::osm::model::{point_in_area, signed_ring_area};
use crate::map::seed::Lcg;
use crate::map::shadow_dir;

/// Зазор между соседними коробками, м. Ноль означал бы «можно вплотную», а
/// два блока кондиционера стенка в стенку читаются как один длинный ящик.
const CLUTTER_GAP: f32 = 0.5;

/// Отступ оборудования от края кровли, м, и доля меньшей стороны дома —
/// на узком корпусе метровый отступ съедает всю крышу.
const EDGE_MARGIN: f32 = 1.6;
const EDGE_MARGIN_SHARE: f32 = 0.18;

/// Докуда тень коробки дотянется, не съехав с нарисованной кровли: ближайшее
/// пересечение луча тени с контуром здания — из каждого угла основания, по
/// внешнему кольцу и по каждому двору.
///
/// Меряется по **ненадвинутому** контуру, как и попадание коробки в него
/// (`fit`): сдвиг 2.5D переносит кровлю целиком. У скатной крыши контур чуть
/// уже нарисованного ската (настоящая крыша свисает), так что труба на коньке
/// обрезается с запасом в свою пользу — лучше, чем наоборот.
pub(super) fn shadow_reach(building: &PolyArea, lift: Vec2, base: &[Vec2; 4]) -> f32 {
    let dir = shadow_dir();
    // лучи идут из всех четырёх углов base, сдвинутых на -lift — ребро можно
    // пропустить, только если оно позади каждого из этих истинных начал луча
    let min_origin = base
        .iter()
        .map(|c| (*c - lift).dot(dir))
        .fold(f32::MAX, f32::min);
    let mut reach = f32::MAX;
    for ring in std::iter::once(&building.outer).chain(building.holes.iter()) {
        for (index, &a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            // ребро целиком позади всех лучей — ни один их не догонит
            if a.dot(dir) < min_origin && b.dot(dir) < min_origin {
                continue;
            }
            for corner in base {
                if let Some(hit) = ray_hits_segment(*corner - lift, dir, a, b) {
                    reach = reach.min(hit);
                }
            }
        }
    }
    reach
}

/// Длина по лучу до пересечения с отрезком, если луч его пересекает.
fn ray_hits_segment(from: Vec2, dir: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let edge = b - a;
    let denom = dir.perp_dot(edge);
    if denom.abs() < 1e-6 {
        return None;
    }
    let to_a = a - from;
    let along_ray = to_a.perp_dot(edge) / denom;
    let along_edge = to_a.perp_dot(dir) / denom;
    (along_ray >= 0.0 && (0.0..=1.0).contains(&along_edge)).then_some(along_ray)
}

/// Рама дома: длинная ось, её длина и ширина, начало — угол описанного по
/// этой оси прямоугольника, уже с отступом от края.
pub(super) struct Frame {
    pub(super) origin: Vec2,
    pub(super) axis: Vec2,
    pub(super) perp: Vec2,
    pub(super) length: f32,
    pub(super) width: f32,
    pub(super) footprint: f32,
}

impl Frame {
    /// Рама с отступом от края; `None` — после отступа не осталось места.
    pub(super) fn of(building: &PolyArea, axis: Vec2, perp: Vec2) -> Option<Self> {
        let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
        let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
        for point in &building.outer {
            min_u = min_u.min(point.dot(axis));
            max_u = max_u.max(point.dot(axis));
            min_v = min_v.min(point.dot(perp));
            max_v = max_v.max(point.dot(perp));
        }
        let (length, width) = (max_u - min_u, max_v - min_v);
        let margin = EDGE_MARGIN.min(width.min(length) * EDGE_MARGIN_SHARE);
        let (length, width) = (length - 2.0 * margin, width - 2.0 * margin);
        if length <= 0.0 || width <= 0.0 {
            return None;
        }
        Some(Self {
            origin: axis * (min_u + margin) + perp * (min_v + margin),
            axis,
            perp,
            length,
            width,
            footprint: signed_ring_area(&building.outer).abs(),
        })
    }

    pub(super) fn center(&self) -> Vec2 {
        self.origin + self.axis * (self.length / 2.0) + self.perp * (self.width / 2.0)
    }
}

/// Одна попытка поставить коробку: точка в раме по ГПСЧ, проверка на попадание
/// в контур и на свободное место. Промах просто теряется — дырявый ряд вентшахт
/// на Г-образном доме выглядит естественнее, чем шахта, висящая над двором.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
    items: &mut Vec<RoofItem>,
    frame: &Frame,
    building: &PolyArea,
    rng: &mut Lcg,
    lift: Vec2,
    size: Vec2,
    height: f32,
    top: Color,
    wall: Color,
) {
    for _ in 0..PLACE_TRIES {
        let center = frame.origin
            + frame.axis * rng.range(0.0, frame.length)
            + frame.perp * rng.range(0.0, frame.width);
        let Some(base) = fit(building, center, size, frame.axis, frame.perp, lift) else {
            continue;
        };
        if !clear(items, &base, frame.axis, frame.perp) {
            continue;
        }
        items.push(RoofItem {
            base,
            height,
            top,
            wall,
            reach: shadow_reach(building, lift, &base),
        });
        return;
    }
}

/// Свободно ли место под коробку. Все предметы плоской кровли разложены в
/// одной раме (`axis`/`perp`) — и вентшахты, и блоки кондиционеров, и лента
/// зенитного фонаря, — так что пересечение двух оснований это обычная проверка
/// двух отрезков по каждой из двух осей, с зазором [`CLUTTER_GAP`] между ними.
///
/// Без неё вентшахта садилась на машинное помещение, а блок кондиционера — на
/// вентшахту: [`fit`] спрашивает только про контур дома и ничего не знает о
/// том, что на кровле уже стоит.
pub(super) fn clear(items: &[RoofItem], base: &[Vec2; 4], axis: Vec2, perp: Vec2) -> bool {
    let span = |corners: &[Vec2; 4], dir: Vec2| {
        corners
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), corner| {
                let at = corner.dot(dir);
                (lo.min(at), hi.max(at))
            })
    };
    let apart = |a: (f32, f32), b: (f32, f32)| a.1 + CLUTTER_GAP <= b.0 || b.1 + CLUTTER_GAP <= a.0;
    let (along, across) = (span(base, axis), span(base, perp));
    !items
        .iter()
        .any(|item| !apart(along, span(&item.base, axis)) && !apart(across, span(&item.base, perp)))
}

/// Основание коробки, если все четыре угла легли внутрь контура. Проверка по
/// **ненадвинутому** пятну: сдвиг 2.5D переносит кровлю целиком, и то, что
/// внутри контура, останется внутри нарисованной крыши.
pub(super) fn fit(
    building: &PolyArea,
    center: Vec2,
    size: Vec2,
    axis: Vec2,
    perp: Vec2,
    lift: Vec2,
) -> Option<[Vec2; 4]> {
    let base = rect(center, size, axis, perp);
    base.iter()
        .all(|corner| point_in_area(*corner, building))
        .then(|| base.map(|corner| corner + lift))
}

/// Прямоугольник `size` вокруг точки, повёрнутый в раму (CCW при
/// правой паре `axis`/`perp`).
pub(super) fn rect(center: Vec2, size: Vec2, axis: Vec2, perp: Vec2) -> [Vec2; 4] {
    let half_axis = axis * (size.x / 2.0);
    let half_perp = perp * (size.y / 2.0);
    [
        center - half_axis - half_perp,
        center + half_axis - half_perp,
        center + half_axis + half_perp,
        center - half_axis + half_perp,
    ]
}
