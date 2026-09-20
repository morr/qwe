//! Словарь `i_overlay` на всю карту: контур, фигура, обводка, закрутка кольца.
//!
//! Булевы операции над полигонами считают три модуля — разбор стоянок
//! (`osm/parse/lots.rs`), бордюр большой стоянки (`roads/lots.rs`) и
//! направляющие островки (`roads/gores.rs`), — и каждый принёс свой набор
//! обёрток: свои `Contour` и `Shape`, своё `ARC`, своё `oriented` слово в
//! слово, свой габарит контура. Копия примитива — тот самый дефект, против
//! которого заведены `map/seed.rs` (один LCG и один посев на все слои),
//! `map/grid.rs` (одна сетка на все индексы) и `map/shadow.rs::push_union`
//! (восемнадцать строк объединения свипов), и здесь то же правило для фигур.
//!
//! Заодно это разводит `roads/lots.rs` и `roads/gores.rs`: восемь из этих имён
//! жили в `lots.rs` как `pub(super)` ради одного `gores.rs`, и два модуля
//! ссылались друг на друга по кругу — хотя ни одно из имён не про стоянку.

use bevy::prelude::*;
use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, StrokeStyle};

use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{PolyArea, point_in_polygon, ring_area, signed_ring_area};

/// Скругление офсетов: длина хорды в долях радиуса (`LineJoin::Round`).
pub(crate) const ARC: f32 = 0.3;
/// Насколько концы ломаной должны сойтись, чтобы считаться одной точкой, м:
/// так узнаётся замкнутый way ([`is_ring`]) и так же решается, нужно ли
/// дотягивать нарисованную ось до её начала.
pub(crate) const RING_EPSILON: f32 = 0.01;

/// Кольцо так, как его берёт `i_overlay`.
pub(crate) type Contour = Vec<[f32; 2]>;
/// Фигура: внешнее кольцо, за ним дырки.
pub(crate) type Shape = Vec<Contour>;

/// Контур обратно в точки карты.
pub(crate) fn ring_of(contour: &Contour) -> Vec<Vec2> {
    contour.iter().copied().map(Vec2::from_array).collect()
}

/// Кольцо в закрутке, которой ждёт `i_overlay`: внешнее против часовой, дырка
/// по ней.
pub(crate) fn oriented(ring: &[Vec2], counterclockwise: bool) -> Contour {
    let mut points: Contour = ring.iter().map(Vec2::to_array).collect();
    if (signed_ring_area(ring) > 0.0) != counterclockwise {
        points.reverse();
    }
    points
}

/// Кольца площади в той же закрутке — в OSM порядок точек какой придётся.
pub(crate) fn area_contours(area: &PolyArea) -> Vec<Contour> {
    std::iter::once(oriented(&area.outer, true))
        .chain(area.holes.iter().map(|hole| oriented(hole, false)))
        .collect()
}

/// Площадь кольца контура, абсолютная.
pub(crate) fn contour_area(contour: &Contour) -> f32 {
    ring_area(&ring_of(contour))
}

/// Площадь фигуры по её внешнему кольцу; у пустой — ноль.
pub(crate) fn shape_area(shape: &Shape) -> f32 {
    shape.first().map_or(0.0, contour_area)
}

/// Габарит контура — то же, что `ring_bounds`, но по точкам `i_overlay`.
pub(crate) fn contour_bounds(contour: &Contour) -> (Vec2, Vec2) {
    contour.iter().fold(
        (Vec2::INFINITY, Vec2::NEG_INFINITY),
        |(low, high), point| {
            let point = Vec2::from_array(*point);
            (low.min(point), high.max(point))
        },
    )
}

/// Замкнута ли ломаная — кольцо развязки.
pub(crate) fn is_ring(path: &[Vec2]) -> bool {
    path.len() >= 4
        && path
            .first()
            .zip(path.last())
            .is_some_and(|(first, last)| first.distance(*last) < RING_EPSILON)
}

/// Полоса вокруг ломаной.
///
/// `ring` — обводить замкнутой, без торцов: у кольца развязки торцов нет, и
/// первая точка в нём повторяет последнюю, поэтому она и снимается. Признак
/// **аргумент, а не [`is_ring`] внутри**: у дорог кольцо обводится замкнутым, а
/// разбор стоянок гонит сюда куски полотна, склеенные через стык звеньев, — и
/// кусок, случайно сомкнувшийся в кольцо, там обводится открытым, как и всегда
/// обводился.
pub(crate) fn stroke(
    path: &[Vec2],
    width: f32,
    cap: LineCap<[f32; 2]>,
    ring: bool,
) -> Vec<Contour> {
    let points = if ring { &path[1..] } else { path };
    let contour: Contour = points.iter().map(Vec2::to_array).collect();
    let style = StrokeStyle::new(width)
        .line_join(LineJoin::Round(ARC))
        .start_cap(cap.clone())
        .end_cap(cap);
    contour.stroke(style, ring).into_iter().flatten().collect()
}

/// Лежит ли точка на фигуре: внутри внешнего кольца и ни в одной дырке.
pub(crate) fn point_in_shape(point: Vec2, shape: &Shape) -> bool {
    let mut rings = shape.iter().map(ring_of);
    rings
        .next()
        .is_some_and(|outer| point_in_polygon(point, &outer))
        && !rings.any(|hole| point_in_polygon(point, &hole))
}

/// Фигура — в меш: внешнее кольцо с дырками.
pub(crate) fn push_shape(builder: &mut MeshBuilder, shape: Shape, color: LinearRgba) {
    let mut rings = shape.iter().map(ring_of);
    let Some(outer) = rings.next() else {
        return;
    };
    let holes: Vec<Vec<Vec2>> = rings.collect();
    builder.push_polygon(&outer, &holes, color);
}
