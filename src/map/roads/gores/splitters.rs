//! **Островки по правилу** — сплиттеры на двусторонних подходах к кольцам:
//! там, где веера из въезда и съезда в OSM нет, краска и колея подхода
//! рвутся на длину островка. Подробности — `references/roads-rings.md`.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::ASPHALT_PAD;
use super::sweep;
use crate::ease::smoothstep;
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::Break;
use crate::map::osm::RoadLine;
use crate::map::roads::junctions::node_key;
use crate::map::roads::rings::{Ring, Rings};
use crate::map::roads::{is_carriageway, lane_count};
use crate::map::shapes::{Shape, shape_area};

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
/// Косинус угла, дальше которого подход у основания островка и на всей его
/// длине не отходит от луча из центра кольца, — 35°.
const SPLITTER_MIN_OUTWARD: f32 = 0.82;
/// Звенья контура островка вдоль подхода.
const SPLITTER_STEPS: usize = 8;

/// Островок, поставленный по правилу на двусторонний подход к кольцу: в
/// OSM такой подход — один way, веера из въезда и съезда, между которыми
/// [`super::Gores::of`] нашёл бы клин, нет. Подход у кольца расширяется на
/// островок: каждая полоса сохраняет ширину, между ними — штриховка.
pub(in crate::map::roads) struct Splitter {
    /// Подход.
    pub road: usize,
    /// Штрихуемый контур — капля от основания у кольца к острию.
    pub island: Shape,
    /// Асфальт расширения подхода.
    pub flare: Vec<Shape>,
    /// Разрыв краски и колеи подхода на длину островка.
    pub gap: Break,
}

/// Островки на двусторонних подходах к кольцам `rings`: подход — проезжая
/// часть в две полосы и больше, приходящая концом в узел кольца.
pub(in crate::map::roads) fn splitters(
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
            // нога Y-подхода — въезд или съезд, островок ей — веер
            || rings.leg_flow(road).is_some()
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
            if let Some(splitter) = splitter(road, line.width, &from_ring, ring, ring.width) {
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
    // островок стоит на подходе, который уходит от кольца: ось, у основания
    // идущая вдоль кольца, положила бы каплю на его полотно и загнула бы её
    // крюком (Рязань, витрина 05 — подходы, заведённые в узел по касательной)
    let (at_base, heading) = place_on_path(path, &along, base)?;
    let at_tip = place_on_path(path, &along, base + length)?.0;
    let outward = (at_base - ring.center).try_normalize()?;
    let chord = (at_tip - at_base).try_normalize()?;
    if heading.dot(outward) < SPLITTER_MIN_OUTWARD || chord.dot(outward) < SPLITTER_MIN_OUTWARD {
        return None;
    }
    let half = (SPLITTER_WIDTH_SHARE * radius)
        .clamp(*SPLITTER_HALF_WIDTH.start(), *SPLITTER_HALF_WIDTH.end());
    // полуширина островка: капля — полная у основания, в ноль к острию
    let spread = |at: f32| {
        let share = ((at - base) / length).clamp(0.0, 1.0);
        half * (1.0 - share).powf(0.8)
    };
    let island = sweep(path, &along, base..base + length, SPLITTER_STEPS, spread)
        .into_iter()
        .max_by(|a, b| shape_area(a).total_cmp(&shape_area(b)))?;
    // расширение — от кромки кольца до острия, полотно раздвинуто на островок.
    // От кромки до основания оно **растёт** из ширины полотна, а не стоит
    // полной ширины сразу: торец во всю ширину — прямоугольный, и там, где
    // ось подхода у кольца идёт вдоль него (Рязань, витрина 05), его углы
    // вылезали за кромку кольца ступенями
    let edge = (ring_width / 2.0 - ASPHALT_PAD).max(0.0);
    let flare = sweep(
        path,
        &along,
        edge..base + length,
        SPLITTER_STEPS * 2,
        |at| {
            width / 2.0
                + if at < base {
                    half * smoothstep((at - edge) / (base - edge).max(f32::EPSILON))
                } else {
                    spread(at)
                }
        },
    );
    let middle = place_on_path(path, &along, base + length / 2.0)?.0;
    Some(Splitter {
        road,
        island,
        flare,
        gap: Break {
            at: middle,
            reach: length / 2.0 + SPLITTER_GAP,
        },
    })
}
