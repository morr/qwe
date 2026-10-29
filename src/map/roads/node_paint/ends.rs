//! Разрывы, выехавшие за конец дороги, и слишком короткие куски линий.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::{MIN_RUN, Walk};
use crate::map::meshing::Break;
use crate::map::osm::RoadLine;
use crate::map::roads::junctions::node_key;
use crate::map::roads::{is_carriageway, lane_count};
use crate::map::shapes::is_ring;

/// Разрыв краски, что выходит за конец дороги, продолжается на дороге за ним:
/// OSM режет улицу на ways у светофора, у перехода, и зебра у самого конца
/// короткого way отрезала линии только на нём — двойная сплошная соседнего
/// начиналась вплотную к зебре (Первомайская в Туле, 3279, 2799). Продолжение —
/// единственная другая проезжая часть, чей конец в той же точке, и точка не
/// узел: на узле у каждой дороги свой разрыв, а ведущая линий не рвёт.
/// Остатки разрывов (`spills`) — концы и длины, что обрезал [`Walk::gap`].
pub(super) fn spill_over_ends(
    drawn: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    junctions: &[(i32, i32)],
    spills: &[(Vec2, f32)],
    breaks: &mut [Vec<Break>],
) {
    if spills.is_empty() {
        return;
    }
    let ends = open_ends(drawn, paths);
    for &(at, reach) in spills {
        let key = node_key(at);
        if junctions.contains(&key) {
            continue;
        }
        let Some(roads) = ends.get(&key).filter(|roads| roads.len() == 2) else {
            continue;
        };
        // оба конца в точке — и та дорога, чей разрыв обрезан: продолжение —
        // вторая из двух
        for &road in roads {
            let path = paths[road].as_ref();
            let walk = Walk::new(path);
            let from = walk.project(at);
            let covered = breaks[road].iter().any(|found| {
                found.reach > 0.0 && (walk.project(found.at) - from).abs() <= found.reach + 1e-3
            });
            if !covered {
                breaks[road].push(Break { at, reach });
            }
        }
    }
}

/// Торцы дороги могут быть швом: проезжая часть, не кольцо и не точка.
fn has_open_ends(road: &RoadLine, path: &[Vec2]) -> bool {
    is_carriageway(road) && path.len() >= 2 && !is_ring(path)
}

/// Дороги с торцом в каждом узле — по [`has_open_ends`].
fn open_ends(drawn: &[&RoadLine], paths: &[impl AsRef<[Vec2]>]) -> HashMap<(i32, i32), Vec<usize>> {
    let mut ends: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (road, path) in paths.iter().enumerate() {
        let path = path.as_ref();
        if !has_open_ends(drawn[road], path) {
            continue;
        }
        for end in [path[0], path[path.len() - 1]] {
            ends.entry(node_key(end)).or_default().push(road);
        }
    }
    ends
}

/// Концы `[начало, конец]` каждой дороги, за которыми улица идёт дальше с
/// **меньшим** числом полос: чистый шов с одной проезжей частью, где линиям,
/// которых у соседа нет, продолжаться некуда — они гаснут в клине.
pub(super) fn narrowing_ends(drawn: &[&RoadLine], paths: &[impl AsRef<[Vec2]>]) -> Vec<[bool; 2]> {
    let ends = open_ends(drawn, paths);
    paths
        .iter()
        .enumerate()
        .map(|(road, path)| {
            let path = path.as_ref();
            if !has_open_ends(drawn[road], path) {
                return [false; 2];
            }
            [path[0], path[path.len() - 1]].map(|end| {
                ends.get(&node_key(end)).is_some_and(|roads| {
                    roads.len() == 2
                        && roads
                            .iter()
                            .filter(|&&other| other != road)
                            .all(|&other| lane_count(drawn[other]) < lane_count(drawn[road]))
                })
            })
        })
        .collect()
}

/// Кусок линий между двумя разрывами короче [`MIN_RUN`] — тоже разрыв. Конец
/// пути, за которым улица сужается (`narrowing`, [`narrowing_ends`]), считается
/// таким же разрывом: у короткой улицы между узлом и клином оставался штрих в
/// пару метров у кромки (витрина 08).
pub(super) fn bridge_short_runs(path: &[Vec2], breaks: &mut Vec<Break>, narrowing: [bool; 2]) {
    if path.len() < 2 {
        return;
    }
    let walk = Walk::new(path);
    let mut spans: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = walk.project(found.at);
            (at - found.reach, at + found.reach)
        })
        .collect();
    for (narrows, at) in narrowing.into_iter().zip([0.0, walk.total]) {
        if narrows {
            spans.push((at, at));
        }
    }
    for (low, high) in short_runs(spans, MIN_RUN) {
        breaks.extend(walk.gap(low, high));
    }
}

/// Куски пути между отрезками длин `spans` (разрывы, спроецированные на путь;
/// порядок любой) короче `min_run`: каждый — пара длин `(от, до)`, которую
/// вызывающий закрывает разрывом по-своему. Перекрытые и смежные отрезки куска
/// не оставляют. Общий обход у линий полос ([`bridge_short_runs`]) и у двойной
/// сплошной разделительной (`medians::bridge_short_pieces`).
pub(in crate::map::roads) fn short_runs(
    mut spans: Vec<(f32, f32)>,
    min_run: f32,
) -> Vec<(f32, f32)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut runs = Vec::new();
    let Some(&(_, mut reach)) = spans.first() else {
        return runs;
    };
    for span in &spans[1..] {
        if span.0 > reach && span.0 - reach < min_run {
            runs.push((reach, span.0));
        }
        reach = reach.max(span.1);
    }
    runs
}
