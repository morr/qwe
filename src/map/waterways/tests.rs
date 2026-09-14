use super::*;
use crate::map::osm::fixture::{square, water_area};
use crate::map::osm::model::polyline_length;

/// Вершины меша воды с долей глубины: 0 — цвет берега, 1 — полная глубина.
fn depths(builder: &MeshBuilder) -> Vec<(Vec2, f32)> {
    let shore = WATER_SHORE_COLOR.to_linear().red;
    let deep = WATER_COLOR.to_linear().red;
    builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
        .map(|(position, color)| {
            (
                Vec2::new(position[0], position[1]),
                (color[0] - shore) / (deep - shore),
            )
        })
        .collect()
}

#[test]
fn the_shoal_runs_from_the_bank_to_full_depth() {
    let found = depths(&mesh_water_areas(&[pond(0.0)]));
    let on_bank =
        |point: Vec2| (point.x.abs() - 50.0).abs() < 1e-2 || (point.y.abs() - 50.0).abs() < 1e-2;
    assert!(found.iter().any(|&(point, _)| on_bank(point)));
    for &(point, depth) in &found {
        assert!((-1e-3..=1.0 + 1e-3).contains(&depth), "{point} at {depth}");
        if on_bank(point) {
            assert!(depth < 1e-3, "a bank vertex {point} is {depth} deep");
        }
    }
    assert!(found.iter().any(|&(_, depth)| depth > 1.0 - 1e-3));
}

#[test]
fn two_ponds_sharing_a_border_have_no_shoal_across_it() {
    // рукав упирается в реку общей границей x = 50: вдоль неё берега нет
    let arm = water_area(square(on_x(100.0), 50.0), Vec::new());
    let found = depths(&mesh_water_areas(&[pond(0.0), arm]));
    for &(point, depth) in &found {
        if (point.x - 50.0).abs() < 1e-2 && point.y.abs() < 40.0 {
            assert!(depth > 0.5, "the seam at {point} is only {depth} deep");
        }
    }
}

#[test]
fn a_narrow_arm_never_reaches_full_depth() {
    // 8 м поперёк: до берега не дальше 4 м, а полная глубина — на шести
    let strip = water_area(
        vec![
            Vec2::new(-4.0, -60.0),
            Vec2::new(4.0, -60.0),
            Vec2::new(4.0, 60.0),
            Vec2::new(-4.0, 60.0),
        ],
        Vec::new(),
    );
    let found = depths(&mesh_water_areas(&[strip]));
    assert!(!found.is_empty());
    let deepest = found.iter().map(|&(_, depth)| depth).fold(0.0, f32::max);
    assert!(deepest < 4.0 / WATER_SHORE_WIDTH + 0.1, "{deepest}");
    assert!(deepest > 0.5, "{deepest}");
}

const REACH: f32 = 6.0;

fn on_x(x: f32) -> Vec2 {
    Vec2::new(x, 0.0)
}

/// Пруд 100 × 100 м с центром в `x`, на оси.
fn pond(x: f32) -> PolyArea {
    water_area(square(on_x(x), 50.0), Vec::new())
}

fn runs(path: &[Vec2], water: &[PolyArea]) -> Vec<OpenRun> {
    WaterIndex::new(water).open_runs(path, REACH)
}

fn close(a: Vec2, b: Vec2) -> bool {
    a.distance(b) < 1e-3
}

#[test]
fn a_channel_on_dry_land_is_left_whole() {
    let path = [on_x(-300.0), on_x(-200.0), Vec2::new(-150.0, 40.0)];
    let found = runs(&path, &[pond(0.0)]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].points, path);
    assert_eq!(found[0].clipped, [false, false]);
}

#[test]
fn a_mouth_reaches_past_the_bank_by_the_shore_width() {
    // ручей с запада впадает в пруд и кончается в его середине
    let path = [on_x(-200.0), on_x(0.0)];
    let found = runs(&path, &[pond(0.0)]);
    assert_eq!(found.len(), 1);
    let run = &found[0];
    assert_eq!(run.clipped, [false, true]);
    assert!(close(run.points[0], on_x(-200.0)));
    // берег на x = -50, заход на ширину отмели глубже
    assert!(close(*run.points.last().unwrap(), on_x(-50.0 + REACH)));
}

#[test]
fn a_mouth_shallower_than_the_shore_is_carried_on_straight() {
    // ось кончается в метре от берега — заход дотягивается по прямой
    let path = [on_x(-200.0), on_x(-49.0)];
    let found = runs(&path, &[pond(0.0)]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].clipped, [false, true]);
    assert!(close(*found[0].points.last().unwrap(), on_x(-50.0 + REACH)));
}

#[test]
fn a_channel_through_a_pond_is_cut_on_both_banks() {
    let path = [on_x(-200.0), on_x(200.0)];
    let found = runs(&path, &[pond(0.0)]);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].clipped, [false, true]);
    assert_eq!(found[1].clipped, [true, false]);
    assert!(close(*found[0].points.last().unwrap(), on_x(-50.0 + REACH)));
    assert!(close(found[1].points[0], on_x(50.0 - REACH)));
    assert!(close(*found[1].points.last().unwrap(), on_x(200.0)));
}

#[test]
fn a_centerline_inside_its_own_river_is_not_drawn() {
    let path = [on_x(-40.0), Vec2::new(0.0, 20.0), on_x(40.0)];
    assert!(runs(&path, &[pond(0.0)]).is_empty());
}

#[test]
fn a_strip_of_water_narrower_than_two_shores_does_not_cut() {
    // протока 8 м поперёк русла: заходы по 6 м с двух сторон легли бы внахлёст
    let strip = water_area(
        vec![
            Vec2::new(-4.0, -50.0),
            Vec2::new(4.0, -50.0),
            Vec2::new(4.0, 50.0),
            Vec2::new(-4.0, 50.0),
        ],
        Vec::new(),
    );
    let path = [on_x(-100.0), on_x(100.0)];
    let found = runs(&path, &[strip]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].clipped, [false, false]);
    assert!((polyline_length(&found[0].points) - 200.0).abs() < 1e-3);
}

#[test]
fn an_island_keeps_its_stretch_of_channel() {
    // русло целиком в пруду, но пересекает остров 40 × 40 м посередине
    let lake = water_area(square(on_x(0.0), 100.0), vec![square(on_x(0.0), 20.0)]);
    let path = [on_x(-80.0), on_x(80.0)];
    let found = runs(&path, &[lake]);
    assert_eq!(found.len(), 1);
    let run = &found[0];
    assert_eq!(run.clipped, [true, true]);
    assert!(close(run.points[0], on_x(-20.0 - REACH)));
    assert!(close(*run.points.last().unwrap(), on_x(20.0 + REACH)));
}

#[test]
fn a_bank_through_a_vertex_of_the_axis_still_cuts() {
    // вершина оси ровно на берегу: пересечение приходится на конец одного звена
    // и начало другого, и обязано быть посчитано ровно один раз
    let path = [on_x(-200.0), on_x(-50.0), on_x(0.0)];
    let found = runs(&path, &[pond(0.0)]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].clipped, [false, true]);
    assert!(close(*found[0].points.last().unwrap(), on_x(-50.0 + REACH)));
}

#[test]
fn overlapping_water_polygons_cut_along_their_union() {
    // riverbank и natural=water поверх него: граница одного внутри другого —
    // не берег
    let path = [on_x(-200.0), on_x(0.0)];
    let found = runs(&path, &[pond(0.0), pond(30.0)]);
    assert_eq!(found.len(), 1);
    assert!(close(*found[0].points.last().unwrap(), on_x(-50.0 + REACH)));
}
