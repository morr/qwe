//! Арки: `tunnel=building_passage` — это дорога, проложенная сквозь дом, и в
//! navmesh она уже прорезана коридором. Здание же рисуется сплошным, и пешки
//! идут сквозь нарисованную стену — карта врёт.
//!
//! Проём лежит **в плоскости стены** и выровнен по грани контура. Ширина —
//! ширина самой дороги, спроецированная углом входа (|sin| между дорогой и
//! гранью: перпендикулярный вход — полная ширина, скользящий — почти ничего)
//! и подрезанная концами грани, чтобы у арки возле угла дома квад не повисал
//! в воздухе. Высота — [`ARCH_HEIGHT`] настоящих метров долей высоты
//! **этого** дома: `band × 6 / height`. Не через `EXTRUDE_SCALE` — подъём
//! обрезан `EXTRUDE_RANGE`, и у сарая или башни нарисованный метр стоит не
//! тех же 0.35 настоящих.
//!
//! Стена ищется не пересечением дороги с контуром, а от **концов** прохода: в
//! OSM арку сплошь и рядом размечают отрезком от вершины контура до вершины
//! контура (арка 485488257 в Туле — ровно такая), то есть дорога лежит внутри
//! дома и стен касается только концами. Пересечения там нет вовсе, зато
//! каждый конец — и есть выход арки наружу.

use std::collections::HashMap;

use bevy::color::Mix;
use bevy::prelude::*;

use super::height_or_default;
use super::layers::silhouette_edges;
use crate::map::SHADOW_COLOR;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{closest_on_segment, point_in_area, ring_bounds};
use crate::map::osm::{PolyArea, RoadLine};
use crate::settings::ARCH_HEIGHT;

/// Цвет проёма арки — та же подложка, что у земли в `spawn.rs`: сквозь арку
/// видно двор, а не стену.
const ARCH_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
/// Дальше скольких метров конец прохода не считается выходом на стену:
/// конец в OSM — общая вершина контура, так что реально там ноль; запас
/// покрывает шум проекции и слегка неровную разметку.
const ARCH_WALL_REACH: f32 = 6.0;
/// Насколько дальше ближайшей грани всё ещё «та же» стена, м: у общей вершины
/// двух граней обе на нулевом расстоянии, и проём обязан кроиться по обеим.
const ARCH_WALL_TIE: f32 = 0.5;

/// Проём, прорезанный в одной грани контура здания.
pub(super) struct ArchOpening {
    /// Грань, в которой прорезан проём.
    pub(super) a: Vec2,
    pub(super) b: Vec2,
    /// Интервал проёма вдоль грани, м от `a`.
    pub(super) low: f32,
    pub(super) high: f32,
    /// Вертикальный габарит проёма — доля полосы/подъёма режима.
    pub(super) sill: Vec2,
}

pub(super) fn arch_openings(
    building: &PolyArea,
    passages: &[&RoadLine],
    band: Vec2,
) -> Vec<ArchOpening> {
    if passages.is_empty() || band == Vec2::ZERO {
        return Vec::new();
    }
    // доля стены, которую занимает проём; у совсем низкого дома арка не
    // может быть выше него самого
    let sill = band * (ARCH_HEIGHT / height_or_default(building)).min(1.0);

    // видимые стены — те же грани, что рисует `extrusion_builder`
    let walls: Vec<(Vec2, Vec2)> = silhouette_edges(&building.outer, Vec2::NEG_Y)
        .into_iter()
        .chain(
            building
                .holes
                .iter()
                .flat_map(|hole| silhouette_edges(hole, Vec2::Y)),
        )
        .collect();

    let mut openings = Vec::new();
    for passage in passages {
        // конец прохода и направление, которым дорога входит в дом
        let ends = [
            passage.points.first().zip(passage.points.get(1)),
            passage
                .points
                .last()
                .zip(passage.points.iter().rev().nth(1)),
        ];
        for &(&point, &neighbour) in ends.iter().flatten() {
            let Some(direction) = (neighbour - point).try_normalize() else {
                continue;
            };
            // конец прохода в OSM — общая вершина контура, то есть точка
            // стыка ДВУХ граней: проём, зажатый в одну из них, обрезался бы
            // до половины ширины дороги. Кроим по всем граням в пределах
            // допуска от ближайшей — на стыке куски продолжают друг друга.
            let nearest = walls
                .iter()
                .map(|&(a, b)| point.distance(closest_on_segment(point, a, b)))
                .fold(f32::MAX, f32::min);
            if nearest > ARCH_WALL_REACH {
                continue;
            }
            for &(a, b) in &walls {
                let at = closest_on_segment(point, a, b);
                if point.distance(at) > nearest + ARCH_WALL_TIE {
                    continue;
                }
                let Some(along) = (b - a).try_normalize() else {
                    continue;
                };
                // дорога под углом к стене дырявит её уже собственной ширины
                let half = passage.width / 2.0 * direction.perp_dot(along).abs();

                let length = (b - a).length();
                let base = (at - a).dot(along);
                let (low, high) = ((base - half).max(0.0), (base + half).min(length));
                if high - low < 0.05 {
                    continue;
                }
                openings.push(ArchOpening {
                    a,
                    b,
                    low,
                    high,
                    sill,
                });
            }
        }
    }
    openings
}

/// Стена с проёмами: боковые куски во всю высоту и перемычка над каждой
/// аркой. Это **настоящий вырез** — в дыру просвечивают нижние слои (дорога,
/// проложенная сквозь дом, тень), а не закраска цветом земли.
pub(super) fn push_wall_with_openings(
    builder: &mut MeshBuilder,
    a: Vec2,
    b: Vec2,
    lift: Vec2,
    openings: &[ArchOpening],
    bottom: LinearRgba,
    top: LinearRgba,
) {
    let mut cuts: Vec<&ArchOpening> = openings
        .iter()
        .filter(|opening| opening.a == a && opening.b == b)
        .collect();
    if cuts.is_empty() {
        builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
        return;
    }
    cuts.sort_by(|first, second| first.low.total_cmp(&second.low));

    let Some(along) = (b - a).try_normalize() else {
        return;
    };
    let length = (b - a).length();
    let piece = |builder: &mut MeshBuilder, from: f32, to: f32| {
        if to - from < 0.01 {
            return;
        }
        let (p0, p1) = (a + along * from, a + along * to);
        builder.push_quad_gradient([p0, p1, p1 + lift, p0 + lift], [bottom, bottom, top, top]);
    };

    let mut cursor = 0.0;
    for cut in cuts {
        piece(builder, cursor, cut.low);
        // перемычка над проёмом; цвет её низа — градиент стены на этой высоте
        let fraction = if lift.length_squared() > 0.0 {
            (cut.sill.length() / lift.length()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sill_color = bottom.mix(&top, fraction);
        let (p0, p1) = (a + along * cut.low, a + along * cut.high);
        builder.push_quad_gradient(
            [p0 + cut.sill, p1 + cut.sill, p1 + lift, p0 + lift],
            [sill_color, sill_color, top, top],
        );
        cursor = cut.high.max(cursor);
    }
    piece(builder, cursor, length);
}

/// Фасадные режимы: полоса фасада — один earcut-полигон на всё здание, и
/// честный паз в нём потребовал бы булевой операции над полигонами. Здесь
/// проём **закрашивается** цветом подложки — компромисс; настоящий вырез
/// живёт в 2.5D (`push_wall_with_openings`).
pub(super) fn push_arches(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    passages: &[&RoadLine],
    band: Vec2,
) {
    // проём затенён перемычкой над ним — подложка мешается с тоном тени
    let color = ARCH_COLOR
        .mix(&Color::srgb(0.22, 0.24, 0.33), SHADOW_COLOR.alpha())
        .to_linear();
    for opening in arch_openings(building, passages, band) {
        let Some(along) = (opening.b - opening.a).try_normalize() else {
            continue;
        };
        push_swept_quad(
            builder,
            [
                opening.a + along * opening.low,
                opening.a + along * opening.high,
            ],
            opening.sill,
            color,
        );
    }
}

/// Проходы, разложенные по домам, которые они прорезают: `building_passage`
/// размечают ровно тем куском дороги, что лежит под домом, поэтому дом
/// ищется по середине прохода.
pub(super) fn arches_by_building<'a>(
    buildings: &[PolyArea],
    passages: &'a [RoadLine],
) -> HashMap<usize, Vec<&'a RoadLine>> {
    let mut by_building: HashMap<usize, Vec<&RoadLine>> = HashMap::new();
    for passage in passages.iter().filter(|road| road.passage) {
        let Some(middle) = passage_middle(passage) else {
            continue;
        };
        let pierced = buildings.iter().position(|building| {
            let (min, max) = ring_bounds(&building.outer);
            middle.x >= min.x
                && middle.x <= max.x
                && middle.y >= min.y
                && middle.y <= max.y
                && point_in_area(middle, building)
        });
        if let Some(index) = pierced {
            by_building.entry(index).or_default().push(passage);
        }
    }
    by_building
}

/// Середина ломаной по длине — устойчивее к неравномерным сегментам, чем
/// средняя точка списка.
pub(super) fn passage_middle(passage: &RoadLine) -> Option<Vec2> {
    let total: f32 = passage
        .points
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum();
    if total <= 0.0 {
        return passage.points.first().copied();
    }
    let mut walked = 0.0;
    for segment in passage.points.windows(2) {
        let length = segment[0].distance(segment[1]);
        if walked + length >= total / 2.0 {
            let t = (total / 2.0 - walked) / length;
            return Some(segment[0].lerp(segment[1], t));
        }
        walked += length;
    }
    passage.points.last().copied()
}

/// Отрезок, протянутый вектором `sweep`, — прямоугольник проёма в стене.
fn push_swept_quad(builder: &mut MeshBuilder, edge: [Vec2; 2], sweep: Vec2, color: LinearRgba) {
    builder.push_polygon(
        &[edge[0], edge[1], edge[1] + sweep, edge[0] + sweep],
        &[],
        color,
    );
}
