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

/// Прямой штрих — лента из двух точек, четыре вершины на квад.
const VERTS_PER_STRAIGHT_DASH: usize = 4;

#[test]
fn dashes_cover_the_path_at_the_given_period() {
    let mut builder = MeshBuilder::default();
    builder.push_dashes(
        &[Vec2::ZERO, Vec2::new(100.0, 0.0)],
        2.0,
        6.0,
        6.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
    );
    // период 12 м на 100 м: штрихи с 0, 12, … 96 — восемь целых и хвост
    assert_eq!(builder.positions.len(), 9 * VERTS_PER_STRAIGHT_DASH);
}

#[test]
fn a_path_shorter_than_one_dash_still_gets_a_dash() {
    // в ж/д развязке коротких ways большинство, и голая лента без штриховки
    // читалась бы как дорога
    let mut builder = MeshBuilder::default();
    builder.push_dashes(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        2.0,
        30.0,
        30.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
    );
    assert_eq!(builder.positions.len(), VERTS_PER_STRAIGHT_DASH);
}

#[test]
fn dashes_follow_the_bends_of_the_path() {
    let path = [Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(30.0, 30.0)];
    let width = 2.0;
    let mut builder = MeshBuilder::default();
    builder.push_dashes(&path, width, 6.0, 6.0, LinearRgba::WHITE, RibbonJoin::Miter);
    assert!(!builder.is_empty());
    // штрих, накрывший излом, обязан повернуть вместе с путём, а не срезать угол
    for position in &builder.positions {
        let point = Vec2::new(position[0], position[1]);
        let distance = distance_to_path(point, &path);
        assert!(distance <= width / 2.0 + 1e-3, "dash drifted {distance} m");
    }
}

#[test]
fn ticks_sit_across_the_path_at_the_given_step() {
    let mut builder = MeshBuilder::default();
    // шаг 6 м на 60 м, первая шпала на 3 м: 3, 9, … 57 — десять штук
    builder.push_ticks(
        &[Vec2::ZERO, Vec2::new(60.0, 0.0)],
        4.0,
        0.7,
        6.0,
        LinearRgba::WHITE,
    );
    assert_eq!(builder.positions.len(), 10 * 4);

    // путь идёт по x, значит шпала обязана стоять поперёк — по y, на ±половину
    // длины, и нигде не выйти за неё
    let spread = |axis: usize| {
        builder
            .positions
            .iter()
            .map(|position| position[axis].abs())
            .fold(0.0_f32, f32::max)
    };
    assert!((spread(1) - 2.0).abs() < 1e-4, "tick is not 4 m across");
}

#[test]
fn ticks_turn_with_the_path() {
    // после излома на 90° шпалы обязаны развернуться вместе с путём
    let mut builder = MeshBuilder::default();
    builder.push_ticks(
        &[Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(30.0, 30.0)],
        4.0,
        0.7,
        6.0,
        LinearRgba::WHITE,
    );
    // на втором колене шпала лежит поперёк y, то есть тянется по x за x = 30
    let beyond = builder
        .positions
        .iter()
        .any(|position| position[0] > 31.0 && position[1] > 1.0);
    assert!(beyond, "ticks did not rotate on the bend");
}

#[test]
fn a_degenerate_path_makes_no_ticks() {
    let mut builder = MeshBuilder::default();
    builder.push_ticks(&[Vec2::ZERO], 4.0, 0.7, 6.0, LinearRgba::WHITE);
    builder.push_ticks(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        4.0,
        0.7,
        0.0,
        LinearRgba::WHITE,
    );
    assert!(builder.is_empty());
}

#[test]
fn a_degenerate_path_makes_no_dashes() {
    let mut builder = MeshBuilder::default();
    builder.push_dashes(
        &[Vec2::ZERO],
        2.0,
        6.0,
        6.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
    );
    builder.push_dashes(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        2.0,
        0.0,
        6.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
    );
    assert!(builder.is_empty());
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

/// Торцы задаются по отдельности: русло, упирающееся одним концом во вход в
/// трубу, обязано быть срезано ровно там и остаться скруглённым с другого.
#[test]
fn capped_ribbon_rounds_only_the_asked_end() {
    let mut builder = MeshBuilder::default();
    builder.push_ribbon_capped(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        [RibbonCap::Round, RibbonCap::Butt],
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
    assert!(
        max_x <= 10.0 + 1e-4,
        "butt end bulges past the node: {max_x}"
    );
    assert!(
        min_x <= -1.0 + ARC_TOLERANCE,
        "round start is missing: {min_x}"
    );
}

#[test]
fn arc_steps_scale_with_radius() {
    // допуск на стрелку хорды один, поэтому широкой дороге нужно больше хорд
    assert!(arc_steps(8.0, PI) > arc_steps(1.75, PI));
    assert_eq!(arc_steps(0.0, PI), 1);
    assert!(arc_steps(1000.0, PI) <= MAX_ARC_STEPS);
}

/// Квадрат 20 × 20 с обходом против часовой стрелки.
fn ccw_square() -> [Vec2; 4] {
    [
        Vec2::ZERO,
        Vec2::new(20.0, 0.0),
        Vec2::new(20.0, 20.0),
        Vec2::new(0.0, 20.0),
    ]
}

/// Кайма квадрата лежит внутри него при любом направлении обхода: дальний
/// край — ровно на ширину каймы от контура.
#[test]
fn inset_band_stays_inside_the_ring_whichever_way_it_winds() {
    for ring in [
        ccw_square().to_vec(),
        ccw_square().into_iter().rev().collect(),
    ] {
        let mut builder = MeshBuilder::default();
        let width = builder.push_inset_band(&ring, 2.0, false, LinearRgba::RED, LinearRgba::WHITE);
        assert_eq!(width, Some(2.0));
        // четыре квада по четыре вершины
        assert_eq!(builder.vertex_count(), 16);
        for position in &builder.positions {
            assert!(
                (-1e-4..=20.0 + 1e-4).contains(&position[0])
                    && (-1e-4..=20.0 + 1e-4).contains(&position[1]),
                "band vertex outside the ring: {position:?}"
            );
        }
        let inner: Vec<_> = builder
            .positions
            .iter()
            .filter(|position| {
                position[0] > 1e-4
                    && position[0] < 20.0 - 1e-4
                    && position[1] > 1e-4
                    && position[1] < 20.0 - 1e-4
            })
            .collect();
        assert_eq!(inner.len(), 8, "far edge vertices: {inner:?}");
        for position in inner {
            let near_far_edge =
                |value: f32| (value - 2.0).abs() < 1e-4 || (value - 18.0).abs() < 1e-4;
            assert!(
                near_far_edge(position[0]) && near_far_edge(position[1]),
                "{position:?}"
            );
        }
    }
}

/// Кайма дырки лежит снаружи её контура — в заливке, а не в самой дырке.
#[test]
fn inset_band_of_a_hole_lies_outside_it() {
    let hole = ccw_square();
    let mut builder = MeshBuilder::default();
    builder.push_inset_band(&hole, 2.0, true, LinearRgba::RED, LinearRgba::WHITE);
    let outside = builder.positions.iter().filter(|position| {
        position[0] < -1e-4
            || position[0] > 20.0 + 1e-4
            || position[1] < -1e-4
            || position[1] > 20.0 + 1e-4
    });
    assert_eq!(outside.count(), 8, "far edge should sit outside the hole");
}

/// Узкая полоса каймы не получает: два метра с каждой стороны на трёхметровом
/// газоне-разделителе вылезли бы за его дальний край на дорогу.
#[test]
fn inset_band_is_clamped_by_the_ring_thickness() {
    let strip = [
        Vec2::ZERO,
        Vec2::new(100.0, 0.0),
        Vec2::new(100.0, 3.0),
        Vec2::new(0.0, 3.0),
    ];
    let mut builder = MeshBuilder::default();
    let width = builder
        .push_inset_band(&strip, 2.0, false, LinearRgba::RED, LinearRgba::WHITE)
        .unwrap();
    // толщина полосы (площадь / периметр) — 300 / 206 ≈ 1.46 м, кайма — 0.6 её
    assert!(width < 1.0, "{width}");
    let hair = [
        Vec2::ZERO,
        Vec2::new(100.0, 0.0),
        Vec2::new(100.0, 0.2),
        Vec2::new(0.0, 0.2),
    ];
    let mut builder = MeshBuilder::default();
    assert_eq!(
        builder.push_inset_band(&hair, 2.0, false, LinearRgba::RED, LinearRgba::WHITE),
        None
    );
    assert!(builder.is_empty());
}

#[test]
fn a_plain_builder_carries_no_ribbon_coords() {
    let mut builder = MeshBuilder::default();
    builder.push_ribbon(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Round,
    );
    assert!(builder.ribbon_coords_for_test().is_none());
    assert!(!builder.build().contains_attribute(ATTRIBUTE_RIBBON));
}

/// Меш поверхности несёт координаты на каждой вершине — и у ленты, и у
/// полигона, у которого они нулевые; иначе раскладка вершин не сойдётся.
#[test]
fn surface_coords_cover_every_vertex() {
    let mut builder = MeshBuilder::with_surface_coords();
    builder.push_polygon(
        &[Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)],
        &[],
        LinearRgba::WHITE,
    );
    builder.push_ribbon(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Round,
    );
    let coords = builder.ribbon_coords_for_test().unwrap();
    assert_eq!(coords.len(), builder.vertex_count());
    assert!(coords[..3].iter().all(|&ribbon| ribbon == [0.0; 4]));
    assert!(builder.build().contains_attribute(ATTRIBUTE_RIBBON));
}

/// Поперёк — ±полуширина на краях ленты, «до разрыва» растёт от обоих торцов
/// к середине, полуширина — та, что просили; код разметки — тот, что выставлен.
#[test]
fn ribbon_coords_follow_the_ribbon_frame() {
    let mut builder = MeshBuilder::with_surface_coords();
    builder.set_markings(Some(Markings {
        lanes: 2,
        oneway: false,
    }));
    builder.push_ribbon(
        &[Vec2::ZERO, Vec2::new(30.0, 0.0)],
        false,
        4.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Butt,
    );
    let coords = builder.ribbon_coords_for_test().unwrap();
    // середина вставлена: два квада по четыре вершины
    assert_eq!(builder.vertex_count(), 8);
    for (position, ribbon) in builder.positions.iter().zip(coords) {
        let [across, to_break, half_width, mode] = *ribbon;
        assert_eq!(
            across, position[1],
            "across follows the offset from the axis"
        );
        let expected = position[0].min(30.0 - position[0]);
        assert!(
            (to_break - expected).abs() < 1e-4,
            "to_break at x={}: {to_break} vs {expected}",
            position[0]
        );
        assert_eq!(half_width, 2.0);
        assert_eq!(mode, 4.0, "two lanes, two-way");
    }
    let middle = builder
        .positions
        .iter()
        .filter(|position| (position[0] - 15.0).abs() < 1e-4)
        .count();
    assert_eq!(middle, 4, "the midpoint vertex pair is missing");
}

/// За торцом «до торца» отрицательно — по нему шейдер гасит разметку на
/// полудиске, торчащем на перекрёсток.
#[test]
fn cap_coords_go_negative_past_the_end() {
    let mut builder = MeshBuilder::with_surface_coords();
    builder.push_ribbon(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        RibbonCap::Round,
    );
    let coords = builder.ribbon_coords_for_test().unwrap();
    let beyond: Vec<f32> = builder
        .positions
        .iter()
        .zip(coords)
        .filter(|(position, _)| position[0] > 10.0 + 1e-4 || position[0] < -1e-4)
        .map(|(_, ribbon)| ribbon[1])
        .collect();
    assert!(!beyond.is_empty(), "no cap vertices past the ends");
    assert!(beyond.iter().all(|&to_break| to_break < 0.0), "{beyond:?}");
    assert!(
        coords.iter().all(|ribbon| ribbon[3] == 0.0),
        "markings were never asked for"
    );
}

/// Разрыв посреди ленты: «до разрыва» — V с дном `-reach` в его центре, на
/// центре стоит вершина, а торцы, которых в списке нет, продолжают линию —
/// координата на полудисках растёт дальше по той же прямой.
#[test]
fn breaks_carve_a_gap_into_the_ribbon_coords() {
    let mut builder = MeshBuilder::with_surface_coords();
    let breaks = [Break {
        at: Vec2::new(30.0, 0.0),
        reach: 5.0,
    }];
    builder.push_ribbon_broken(
        &[Vec2::ZERO, Vec2::new(60.0, 0.0)],
        4.0,
        LinearRgba::WHITE,
        RibbonJoin::Round,
        [RibbonCap::Round; 2],
        RibbonBreaks::At(&breaks),
    );
    let coords = builder.ribbon_coords_for_test().unwrap();
    for (position, ribbon) in builder.positions.iter().zip(coords) {
        let expected = (position[0] - 30.0).abs() - 5.0;
        assert!(
            (ribbon[1] - expected).abs() < 1e-3,
            "to_break at x={}: {} vs {expected}",
            position[0],
            ribbon[1]
        );
    }
    let at_center = builder
        .positions
        .iter()
        .filter(|position| (position[0] - 30.0).abs() < 1e-4)
        .count();
    assert_eq!(
        at_center, 4,
        "the kink at the break centre needs a vertex pair"
    );
}

/// Торец в списке разрывов — тупик или перекрёсток: за ним «до разрыва»
/// уходит в минус, как у ленты без списка. Торца в списке нет — way
/// продолжается следующим: координата за торцом растёт, и разметка идёт
/// сквозь стык.
#[test]
fn a_listed_end_stops_the_marking_and_an_unlisted_one_carries_it_on() {
    let path = [Vec2::ZERO, Vec2::new(10.0, 0.0)];
    let beyond_end = |breaks: &[Break]| -> Vec<f32> {
        let mut builder = MeshBuilder::with_surface_coords();
        builder.push_ribbon_broken(
            &path,
            2.0,
            LinearRgba::WHITE,
            RibbonJoin::Round,
            [RibbonCap::Butt, RibbonCap::Round],
            RibbonBreaks::At(breaks),
        );
        let coords = builder.ribbon_coords_for_test().unwrap();
        builder
            .positions
            .iter()
            .zip(coords)
            .filter(|(position, _)| position[0] > 10.0 + 1e-4)
            .map(|(_, ribbon)| ribbon[1])
            .collect()
    };
    let dead_end = beyond_end(&[Break {
        at: Vec2::new(10.0, 0.0),
        reach: 0.0,
    }]);
    assert!(!dead_end.is_empty(), "no cap vertices past the end");
    assert!(
        dead_end.iter().all(|&to_break| to_break < 0.0),
        "{dead_end:?}"
    );

    let junction = beyond_end(&[Break {
        at: Vec2::new(10.0, 0.0),
        reach: 3.0,
    }]);
    assert!(
        junction.iter().all(|&to_break| to_break < -3.0),
        "{junction:?}"
    );

    let continuation = beyond_end(&[Break {
        at: Vec2::ZERO,
        reach: 0.0,
    }]);
    assert!(
        continuation.iter().all(|&to_break| to_break > 10.0),
        "{continuation:?}"
    );
}

/// Два разрыва внахлёст — один: дно V ниже любого из двух.
#[test]
fn overlapping_gaps_merge_into_one() {
    let breaks = [
        Break {
            at: Vec2::new(20.0, 0.0),
            reach: 5.0,
        },
        Break {
            at: Vec2::new(26.0, 0.0),
            reach: 5.0,
        },
    ];
    let mut builder = MeshBuilder::with_surface_coords();
    builder.push_ribbon_broken(
        &[Vec2::ZERO, Vec2::new(60.0, 0.0)],
        4.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
        [RibbonCap::Butt; 2],
        RibbonBreaks::At(&breaks),
    );
    let deepest = builder
        .ribbon_coords_for_test()
        .unwrap()
        .iter()
        .map(|ribbon| ribbon[1])
        .fold(f32::INFINITY, f32::min);
    assert!(
        (deepest + 8.0).abs() < 1e-3,
        "the union [15, 31] is 16 m long, so the bottom is -8: {deepest}"
    );
}

/// Без единого разрыва координата всё равно растёт вдоль ленты — по ней идут
/// штрихи — и нигде не гаснет.
#[test]
fn a_ribbon_without_breaks_keeps_the_marking_coordinate_growing() {
    let mut builder = MeshBuilder::with_surface_coords();
    builder.push_ribbon_broken(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
        [RibbonCap::Butt; 2],
        RibbonBreaks::At(&[]),
    );
    let coords = builder.ribbon_coords_for_test().unwrap();
    for (position, ribbon) in builder.positions.iter().zip(coords) {
        assert!((ribbon[1] - (position[0] + FAR_FROM_BREAKS)).abs() < 1e-3);
    }
}

#[test]
fn a_template_keeps_its_ribbon_coords_scaled() {
    let mut template = MeshBuilder::with_surface_coords();
    template.push_ribbon(
        &[Vec2::ZERO, Vec2::new(10.0, 0.0)],
        false,
        2.0,
        LinearRgba::WHITE,
        RibbonJoin::Miter,
        RibbonCap::Butt,
    );
    let mut builder = MeshBuilder::with_surface_coords();
    builder.push_template(&template, Vec2::new(100.0, 100.0), 3.0);
    let source = template.ribbon_coords_for_test().unwrap();
    let copied = builder.ribbon_coords_for_test().unwrap();
    assert_eq!(copied.len(), source.len());
    for (from, to) in source.iter().zip(copied) {
        assert_eq!(to[0], from[0] * 3.0);
        assert_eq!(to[1], from[1] * 3.0);
        assert_eq!(to[2], from[2] * 3.0);
    }
    // шаблон без координат под сборщик с ними — нули, а не паника
    let mut plain = MeshBuilder::default();
    plain.push_rect(Vec2::ZERO, Vec2::ONE, LinearRgba::WHITE);
    builder.push_template(&plain, Vec2::ZERO, 1.0);
    assert_eq!(
        builder.ribbon_coords_for_test().unwrap().len(),
        builder.vertex_count()
    );
}
