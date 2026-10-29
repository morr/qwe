//! Разрезы линии краски: крючки у разрывов, сплошная подхода, куски.

use bevy::prelude::*;

use super::APPROACH;
use crate::map::along::place_on_path;
use crate::map::meshing::{Break, PaintStation};

/// На каких последних метрах перед краем разрыва, м, изгиб пути — крючок
/// конца линии ([`hook_breaks`]).
const HOOK_REACH: f32 = 3.0;
/// Поворот пути от этого, рад, за [`HOOK_REACH`] до края разрыва — крючок:
/// линия кончается там, где начала гнуться. У Орджоникидзе (Орёл, у Ляшко)
/// сглаженная ось на последних двух метрах перед узлом гнётся на 17° к нему,
/// и двойная сплошная с линией полос кончались загибом вбок (S6). Дуга
/// улицы поворачивает на столько за три метра только при радиусе меньше
/// семнадцати.
const HOOK_BEND: f32 = 10.0 * std::f32::consts::PI / 180.0;
/// От этого поворота, рад, звено крючка уже отошло от прямой.
const HOOK_START: f32 = 2.0 * std::f32::consts::PI / 180.0;

/// Крючки на концах линий: разрыв, продлённый назад до начала изгиба пути.
/// Край разрыва — ноль «до разрыва» `to_break` на звене пути `path` (длины
/// дуги `along`). Звено края повёрнуто от звена в [`HOOK_REACH`] перед ним со
/// стороны видимой линии больше чем на [`HOOK_BEND`] — конец загнут вбок, и
/// линия кончается в самой дальней от края вершине этих метров, где путь
/// отошёл от прямой больше [`HOOK_START`]: разрыв от неё до края, внахлёст с
/// найденным (`meshing::break_profile` сливает перекрытые). Сглаженная ось
/// перед узлом, где улица поворачивает к продолжению, и давала такие загибы.
/// Замкнутый путь сюда не приходит: у кольца линии по кругу и должны.
pub(super) fn hook_breaks(path: &[Vec2], along: &[f32], to_break: &[f32]) -> Vec<Break> {
    let links = path.len().saturating_sub(1);
    let direction = |link: usize| (path[link + 1] - path[link]).normalize_or_zero();
    // звено, на котором лежит длина дуги `at`, считая концы звеньев — не как
    // [`link_at`], которая ищет строго внутри
    let link_containing =
        |at: f32| (0..links).find(|&link| along[link] <= at && at <= along[link + 1]);
    let turn = |from: Vec2, to: Vec2| from.angle_to(to).abs();
    let mut hooks = Vec::new();
    for link in 0..links {
        let (near, far) = (to_break[link], to_break[link + 1]);
        // край разрыва на звене: видимая линия с одной стороны нуля
        let visible_before = near > 0.0 && far <= 0.0;
        let visible_after = near <= 0.0 && far > 0.0;
        if !visible_before && !visible_after {
            continue;
        }
        let edge = zero_at(along, link, near, far);
        // линия до изгиба — звено в [`HOOK_REACH`] от края; конец — звено края
        let back = if visible_before {
            edge - HOOK_REACH
        } else {
            edge + HOOK_REACH
        };
        let Some(body) = link_containing(back) else {
            continue;
        };
        let (straight, end) = (direction(body), direction(link));
        if straight == Vec2::ZERO || end == Vec2::ZERO || turn(straight, end) <= HOOK_BEND {
            continue;
        }
        // крючок: линия кончается там, где начала отходить от прямой
        let bent = (0..path.len()).filter(|&vertex| {
            let (away, after) = if visible_before {
                (edge - along[vertex], vertex < links)
            } else {
                (along[vertex] - edge, vertex > 0)
            };
            let leaving = if visible_before {
                after.then(|| direction(vertex))
            } else {
                after.then(|| direction(vertex - 1))
            };
            away > 0.0
                && away < HOOK_REACH
                && leaving.is_some_and(|leaving| turn(straight, leaving) > HOOK_START)
        });
        let farthest = if visible_before {
            bent.min_by(|&a, &b| along[a].total_cmp(&along[b]))
        } else {
            bent.max_by(|&a, &b| along[a].total_cmp(&along[b]))
        };
        let Some(vertex) = farthest else {
            continue;
        };
        let (from, to) = if visible_before {
            (along[vertex], edge)
        } else {
            (edge, along[vertex])
        };
        let Some((at, _)) = place_on_path(path, along, (from + to) / 2.0) else {
            continue;
        };
        hooks.push(Break {
            at,
            reach: (to - from) / 2.0 + HOOK_OVERLAP,
        });
    }
    hooks
}

/// Нахлёст разрыва крючка ([`hook_breaks`]) с найденным разрывом, м.
const HOOK_OVERLAP: f32 = 0.05;

/// Звено, внутри которого (строго) лежит длина дуги `at`, и доля `at` на нём.
fn link_at(along: &[f32], at: f32) -> Option<(usize, f32)> {
    let index = along
        .windows(2)
        .position(|pair| pair[0] < at && at < pair[1])?;
    Some((
        index,
        (at - along[index]) / (along[index + 1] - along[index]),
    ))
}

/// Где на звене `index` линейная между вершинами величина — `a` в первой, `b`
/// во второй — проходит через ноль: длина дуги нулевой точки. Вызывающий
/// гарантирует, что `a` и `b` разных знаков.
fn zero_at(along: &[f32], index: usize, a: f32, b: f32) -> f32 {
    along[index] + (along[index + 1] - along[index]) * a / (a - b)
}

/// Вершина на длине дуги `at` — вместе с интерполированным «до разрыва».
pub(super) fn insert_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    to_break: &mut Vec<f32>,
    at: f32,
) {
    let Some((index, t)) = link_at(along, at) else {
        return;
    };
    path.insert(index + 1, path[index].lerp(path[index + 1], t));
    to_break.insert(
        index + 1,
        to_break[index] + (to_break[index + 1] - to_break[index]) * t,
    );
    along.insert(index + 1, at);
}

/// Отрезки длин пути, где линия полос сплошная: [`APPROACH`] до входа в
/// каждый разрыв по ходу движения — перед узлом перестраиваться нельзя, а
/// на выезде из него можно сразу. `to_break` — «до разрыва» в вершинах пути
/// (в разрыве меньше нуля); между вершинами он линеен, так что край разрыва —
/// ноль на звене.
pub(super) fn approach_spans(along: &[f32], to_break: &[f32], forward: bool) -> Vec<(f32, f32)> {
    let mut spans = Vec::new();
    for index in 0..along.len().saturating_sub(1) {
        let (a, b) = (to_break[index], to_break[index + 1]);
        let (entering, leaving) = if forward {
            (a >= 0.0 && b < 0.0, false)
        } else {
            (false, a < 0.0 && b >= 0.0)
        };
        if !entering && !leaving {
            continue;
        }
        let edge = zero_at(along, index, a, b);
        spans.push(if forward {
            (edge - APPROACH, edge)
        } else {
            (edge, edge + APPROACH)
        });
    }
    spans
}

/// Отрезки длин пути, где `distance` (в вершинах, линейно между ними) меньше
/// `within`: у осевой — [`APPROACH`] по обе стороны разрыва или узла насквозь.
pub(super) fn near_spans(along: &[f32], distance: &[f32], within: f32) -> Vec<(f32, f32)> {
    let mut spans = Vec::new();
    let mut from = (distance.first() < Some(&within)).then(|| along[0]);
    for index in 0..along.len().saturating_sub(1) {
        let (a, b) = (distance[index] - within, distance[index + 1] - within);
        if (a < 0.0) == (b < 0.0) {
            continue;
        }
        let edge = zero_at(along, index, a, b);
        match from.take() {
            Some(start) => spans.push((start, edge)),
            None => from = Some(edge),
        }
    }
    if let (Some(start), Some(&end)) = (from, along.last()) {
        spans.push((start, end));
    }
    spans
}

/// Линия с вершинами на концах отрезков `spans` и сплошная ли она на каждом
/// звене: по середине звена — внутри ли отрезка.
pub(super) fn split_at_spans(
    mut line: Vec<Vec2>,
    mut stations: Vec<PaintStation>,
    along: &[f32],
    spans: &[(f32, f32)],
) -> (Vec<Vec2>, Vec<PaintStation>, Vec<bool>) {
    let mut along = along.to_vec();
    for &at in spans.iter().flat_map(|(from, to)| [from, to]) {
        let Some((index, t)) = link_at(&along, at) else {
            continue;
        };
        let (a, b) = (stations[index], stations[index + 1]);
        line.insert(index + 1, line[index].lerp(line[index + 1], t));
        stations.insert(
            index + 1,
            PaintStation {
                along: a.along + (b.along - a.along) * t,
                to_break: a.to_break + (b.to_break - a.to_break) * t,
                alpha: a.alpha + (b.alpha - a.alpha) * t,
            },
        );
        along.insert(index + 1, at);
    }
    let solid = along
        .windows(2)
        .map(|pair| {
            let middle = (pair[0] + pair[1]) / 2.0;
            spans
                .iter()
                .any(|&(from, to)| from <= middle && middle <= to)
        })
        .collect();
    (line, stations, solid)
}
