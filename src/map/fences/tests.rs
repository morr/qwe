//! Слой оград, собранный без мира.
//!
//! Эти тесты появились вместе со швом `mesh_fences`: до него слой собирался
//! внутри системы Bevy, и достать его из теста было нечем — модуль был самым
//! крупным в `map/*` без единого теста, хотя в нём лежит и вырезание проёмов,
//! и копия правила полутени.

use super::*;
use crate::map::osm::fixture;

/// Ширина линии ближней ступени, м — чтобы тест не зависел от таблицы LOD.
const NEAR_WIDTH: f32 = FENCE_LODS[0].width;

fn fence_across(from: Vec2, to: Vec2) -> FenceLine {
    fixture::fence(vec![from, to])
}

#[test]
fn a_fence_builds_one_blended_layer() {
    let fences = [fence_across(
        Vec2::new(100.0, 100.0),
        Vec2::new(200.0, 100.0),
    )];
    let (layers, report) = mesh_fences(&fences, &[], NEAR_WIDTH);

    assert_eq!(layers.len(), 1, "ограда — один слитый меш");
    let layer = &layers[0];
    assert_eq!(layer.name, "fences");
    assert_eq!(layer.z, Z_FENCE);
    // тень полупрозрачна, а непрозрачный материал съел бы вершинную альфу
    assert_eq!(layer.material, MaterialSpec::Blend);
    assert!(report.vertices > 0, "ограда не нарисована вовсе");
    assert_eq!(report.lines, 1);
}

#[test]
fn the_far_bucket_draws_nothing() {
    let fences = [fence_across(
        Vec2::new(100.0, 100.0),
        Vec2::new(200.0, 100.0),
    )];
    let width = FENCE_LODS[FENCE_LODS.len() - 1].width;
    assert_eq!(width, 0.0, "дальняя ступень обязана быть нулевой ширины");

    let (layers, report) = mesh_fences(&fences, &[], width);

    // не особый случай у вызывающего, а пустой список слоёв
    assert!(layers.is_empty(), "на общем плане ограда не рисуется");
    assert_eq!(report.vertices, 0);
    assert_eq!(report.pieces, 0);
    // ограда на входе всё равно посчитана: ноль кусков — это решение ступени,
    // а не пустая карта
    assert_eq!(report.lines, 1);
}

#[test]
fn a_road_through_a_fence_breaks_it_into_pieces() {
    let fence = fence_across(Vec2::new(100.0, 100.0), Vec2::new(200.0, 100.0));
    let across = fixture::street(vec![Vec2::new(150.0, 40.0), Vec2::new(150.0, 160.0)], 8.0);

    let (_, whole) = mesh_fences(std::slice::from_ref(&fence), &[], NEAR_WIDTH);
    let (_, cut) = mesh_fences(std::slice::from_ref(&fence), &[across], NEAR_WIDTH);

    assert_eq!(whole.pieces, 1, "ограду без дорог резать нечем");
    assert!(
        cut.pieces > whole.pieces,
        "дорога сквозь ограду обязана открыть проём: {} кусков против {}",
        cut.pieces,
        whole.pieces
    );
}

#[test]
fn a_road_along_a_fence_leaves_it_whole() {
    let fence = fence_across(Vec2::new(100.0, 100.0), Vec2::new(200.0, 100.0));
    // улица параллельно ограде и в стороне от неё: сквозь ограду не идёт
    let along = fixture::street(vec![Vec2::new(100.0, 140.0), Vec2::new(200.0, 140.0)], 8.0);

    let (_, report) = mesh_fences(std::slice::from_ref(&fence), &[along], NEAR_WIDTH);

    assert_eq!(
        report.pieces, 1,
        "дорога вдоль ограды не проходит сквозь неё и резать её не должна"
    );
}

#[test]
fn an_empty_map_builds_no_geometry() {
    let (layers, report) = mesh_fences(&[], &[], NEAR_WIDTH);

    assert_eq!(report.lines, 0);
    assert_eq!(report.vertices, 0);
    // слой описан, но пуст: положить его в мир — дело адаптера, и пустой меш
    // он пропустит сам
    assert_eq!(layers.len(), 1);
    assert!(layers[0].builder.is_empty());
}
