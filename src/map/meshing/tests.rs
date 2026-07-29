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

/// Расстояние от точки до ломаной — ни одна вершина скруглённой ленты не
/// имеет права уйти дальше полуширины, торцы включительно.
fn distance_to_path(point: Vec2, path: &[Vec2]) -> f32 {
    path.windows(2)
        .map(|segment| {
            let span = segment[1] - segment[0];
            let t = (point - segment[0]).dot(span) / span.length_squared();
            point.distance(segment[0] + span * t.clamp(0.0, 1.0))
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn round_ribbon_stays_within_half_width() {
    let path = [
        Vec2::ZERO,
        Vec2::new(10.0, 0.0),
        Vec2::new(14.0, 8.0),
        Vec2::new(4.0, 12.0),
    ];
    let mut builder = MeshBuilder::default();
    builder.push_ribbon(
        &path,
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Round,
    );
    for position in &builder.positions {
        let vertex = Vec2::new(position[0], position[1]);
        assert!(
            distance_to_path(vertex, &path) <= 1.0 + 1e-4,
            "round ribbon vertex off the band: {vertex:?}"
        );
    }
}

#[test]
fn round_join_fills_the_outer_gap() {
    // прямой угол влево: щель butt-квадов справа по ходу, веер обязан лечь
    // на дугу радиуса в полуширину вокруг излома
    let corner = Vec2::new(10.0, 0.0);
    let path = [Vec2::ZERO, corner, Vec2::new(10.0, 10.0)];
    let mut builder = MeshBuilder::default();
    builder.push_ribbon(
        &path,
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Butt,
    );
    // два сегмента по кваду + веер, у веера центр в изломе
    assert!(builder.positions.len() > 8, "join fan is missing");
    let fan = &builder.positions[8..];
    assert_eq!(Vec2::new(fan[0][0], fan[0][1]), corner);
    for position in &fan[1..] {
        let vertex = Vec2::new(position[0], position[1]);
        assert!(
            (vertex.distance(corner) - 1.0).abs() < 1e-4,
            "fan vertex off the arc: {vertex:?}"
        );
        // внешняя сторона левого поворота — правая, то есть y < 0 или x > 10
        assert!(vertex.x >= corner.x - 1e-4 && vertex.y <= corner.y + 1e-4);
    }
}

/// Лента из трёх точек с изломом в `turn` градусов на средней.
fn bent_round_ribbon(turn: f32, width: f32) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let elbow = Vec2::new(10.0, 0.0);
    builder.push_ribbon(
        &[
            Vec2::ZERO,
            elbow,
            elbow + Vec2::from_angle(turn.to_radians()) * 10.0,
        ],
        false,
        width,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Butt,
    );
    builder
}

#[test]
fn invisible_turns_skip_the_join_fan() {
    // щель шириной 1.75 · 0.5° ≈ 1.5 см — мельче ARC_TOLERANCE, веера нет
    let builder = bent_round_ribbon(0.5, 3.5);
    assert_eq!(builder.positions.len(), 8);
    assert_eq!(builder.indices.len(), 12);
}

#[test]
fn visible_turns_get_a_join_fan() {
    // а тот же излом в 5° оставляет 15 см — на приближении это видимая
    // прорезь поперёк дороги, веер обязан быть
    let builder = bent_round_ribbon(5.0, 3.5);
    assert!(builder.positions.len() > 8, "join fan is missing");
}

#[test]
fn round_cap_bulges_past_the_end_by_half_width() {
    let mut builder = MeshBuilder::default();
    builder.push_ribbon(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Round,
    );
    let max_x = builder
        .positions
        .iter()
        .map(|position| position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let min_x = builder
        .positions
        .iter()
        .map(|position| position[0])
        .fold(f32::INFINITY, f32::min);
    // полуширины за торец — как у квадратного продления push_polyline, так что
    // габарит дороги не меняется, меняется форма. Дуга ломаная и вершину
    // полудиска сэмплирует не всегда, поэтому недобор в пределах допуска на
    // стрелку хорды; перебора не бывает никогда
    assert!(max_x <= 11.0 + 1e-4, "end cap overshoots: {max_x}");
    assert!(min_x >= -1.0 - 1e-4, "start cap overshoots: {min_x}");
    assert!(max_x >= 11.0 - ARC_TOLERANCE, "end cap short: {max_x}");
    assert!(min_x <= -1.0 + ARC_TOLERANCE, "start cap short: {min_x}");
}

#[test]
fn arc_steps_scale_with_radius() {
    // допуск на стрелку хорды один, поэтому широкой дороге нужно больше хорд
    assert!(arc_steps(8.0, PI) > arc_steps(1.75, PI));
    assert_eq!(arc_steps(0.0, PI), 1);
    assert!(arc_steps(1000.0, PI) <= MAX_ARC_STEPS);
}
