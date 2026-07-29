use super::*;

#[test]
fn polygon_with_hole_triangulates() {
    let mut builder = MeshBuilder::default();
    builder.push_polygon(
        &[
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ],
        &[vec![
            Vec2::new(4.0, 4.0),
            Vec2::new(6.0, 4.0),
            Vec2::new(6.0, 6.0),
            Vec2::new(4.0, 6.0),
        ]],
        LinearRgba::WHITE,
    );
    assert!(!builder.is_empty());
    assert_eq!(builder.skipped_polygons(), 0);
    assert_eq!(builder.positions.len(), 8);
    // квадрат с дыркой — 8 треугольников
    assert_eq!(builder.indices.len(), 24);
}

#[test]
fn degenerate_polygon_is_skipped() {
    let mut builder = MeshBuilder::default();
    builder.push_polygon(&[Vec2::ZERO, Vec2::new(1.0, 1.0)], &[], LinearRgba::WHITE);
    assert!(builder.is_empty());
    assert_eq!(builder.skipped_polygons(), 1);
}

#[test]
fn closed_stroke_wraps_around_and_keeps_width() {
    let square = [
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ];
    let mut builder = MeshBuilder::default();
    builder.push_stroke(&square, true, 2.0, LinearRgba::WHITE);
    // замкнутая лента: по кваду на ребро, включая ребро назад в начало
    assert_eq!(builder.positions.len(), 16);
    assert_eq!(builder.indices.len(), 24);
    // прямой угол: miter ставит вершины ровно на ±полширины от контура,
    // ничего не торчит дальше (у push_polyline торцы уходили за угол)
    for position in &builder.positions {
        let corner = Vec2::new(position[0], position[1]);
        let offset_by_half = |value: f32| {
            [-1.0, 1.0, 9.0, 11.0]
                .iter()
                .any(|expected: &f32| (value - expected).abs() < 1e-4)
        };
        assert!(
            offset_by_half(corner.x) && offset_by_half(corner.y),
            "stroke vertex off the band: {corner:?}"
        );
    }
}

#[test]
fn open_stroke_does_not_extend_past_its_ends() {
    let mut builder = MeshBuilder::default();
    builder.push_stroke(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
    );
    assert_eq!(builder.indices.len(), 6);
    let max_x = builder
        .positions
        .iter()
        .map(|position| position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    // push_polyline продлил бы торец до 11.0 — здесь ровно конец пути
    assert_eq!(max_x, 10.0);
}

#[test]
fn stroke_merges_points_closer_than_quarter_width() {
    let mut builder = MeshBuilder::default();
    let dense: Vec<Vec2> = (0..5)
        .map(|step| Vec2::new(step as f32 * 0.01, 0.0))
        .collect();
    builder.push_stroke(&dense, false, 2.0, LinearRgba::WHITE);
    // все точки в пределах 0.5 — путь схлопывается и рисовать нечего
    assert!(builder.is_empty());
}

#[test]
fn polyline_makes_quad_per_segment() {
    let mut builder = MeshBuilder::default();
    builder.push_polyline(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
        2.0,
        LinearRgba::WHITE,
    );
    assert_eq!(builder.positions.len(), 8);
    assert_eq!(builder.indices.len(), 12);
}
