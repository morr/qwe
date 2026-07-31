use super::*;

fn road(points: Vec<Vec2>, width: f32, passage: bool) -> RoadLine {
    RoadLine {
        points,
        width,
        class: RoadClass::Alley,
        bridge: false,
        passage,
    }
}

/// Расстояние от точки до ломаной — тем же способом, что и в тестах мешинга.
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
fn chaikin_keeps_endpoints() {
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, 3.5);
    assert_eq!(smoothed[0], original[0]);
    assert_eq!(smoothed[smoothed.len() - 1], original[original.len() - 1]);
}

#[test]
fn chaikin_deviation_is_bounded_by_width() {
    // длинные сегменты: без ограничения шириной срез ушёл бы на 5 м от угла
    let width = 3.5;
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, width);
    // сами точки среза лежат на исходных сегментах
    for point in &smoothed {
        assert!(distance_to_path(*point, &original) < 1e-4);
    }
    // а хорда, заменившая угол, отходит от него не дальше ширины дороги
    let deviation = smoothed
        .windows(2)
        .map(|segment| distance_to_path(segment[0].midpoint(segment[1]), &original))
        .fold(0.0_f32, f32::max);
    assert!(deviation <= width, "chaikin drifted {deviation} m");
}

#[test]
fn chaikin_leaves_straight_runs_alone() {
    // изломы по 2° мельче MIN_SMOOTH_ANGLE — ломаная возвращается как есть
    let step = 10.0 * 2.0_f32.to_radians().tan();
    let original = vec![
        Vec2::ZERO,
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, step),
        Vec2::new(30.0, step * 2.0),
    ];
    assert_eq!(chaikin(&original, 3.5), original);
}

#[test]
fn passage_roads_are_not_smoothed() {
    // концы арки приколоты к вершинам контура здания — сглаживать её нельзя
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let arch = road(points.clone(), 5.0, true);
    assert_eq!(
        centerline(&arch, RoadSmoothing::Strong).as_ref(),
        points.as_slice()
    );
    let ordinary = road(points, 5.0, false);
    assert!(centerline(&ordinary, RoadSmoothing::Strong).len() > 3);
}

#[test]
fn smoothing_off_borrows_the_osm_centerline() {
    let ordinary = road(vec![Vec2::ZERO, Vec2::new(20.0, 0.0)], 5.0, false);
    assert!(matches!(
        centerline(&ordinary, RoadSmoothing::Off),
        Cow::Borrowed(_)
    ));
}

#[test]
fn rails_are_smoothed_like_roads_but_never_pinned() {
    // у рельса нет `passage`, поэтому сглаживание к нему применяется всегда
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    assert!(smooth_path(&points, 5.0, RoadSmoothing::Strong).len() > 3);
    assert!(matches!(
        smooth_path(&points, 5.0, RoadSmoothing::Off),
        Cow::Borrowed(_)
    ));
}

#[test]
fn rail_dashes_are_narrower_than_the_bed_and_leave_gaps() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let width = 5.0;

    let mut bed = MeshBuilder::default();
    push_ribbon(&mut bed, &points, width, LinearRgba::WHITE, RoadJoin::Round);
    let mut dashes = MeshBuilder::default();
    dashes.push_dashes(
        &points,
        width * RAIL_DASH_SCALE,
        RAIL_DASH_LEN,
        RAIL_DASH_GAP,
        LinearRgba::WHITE,
        dash_join(RoadJoin::Round),
    );

    let extent = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    // штриховка обязана лежать внутри ленты, иначе она читается как вторая линия
    assert!(!dashes.is_empty());
    assert!(extent(&dashes) < extent(&bed));

    // и обязана быть прерывистой: сплошная лента на том же пути — один кусок
    let mut solid = MeshBuilder::default();
    solid.push_dashes(
        &points,
        width * RAIL_DASH_SCALE,
        1000.0,
        1000.0,
        LinearRgba::WHITE,
        dash_join(RoadJoin::Round),
    );
    assert!(dashes.vertex_count() > solid.vertex_count());
}

#[test]
fn casing_is_wider_than_the_fill() {
    // кант обязан торчать из-под заливки на всех классах дорог
    for width in [3.5_f32, 5.0, 8.0, 16.0] {
        assert!(casing_width(width) >= *CASING_RANGE.start());
        assert!(casing_width(width) <= *CASING_RANGE.end());
    }

    let points = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let extent = |width: f32| {
        let mut builder = MeshBuilder::default();
        push_ribbon(
            &mut builder,
            &points,
            width,
            LinearRgba::WHITE,
            RoadJoin::Round,
        );
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let fill = 3.5;
    assert!(extent(fill + 2.0 * casing_width(fill)) > extent(fill));
}

#[test]
fn bridge_curb_is_thicker_than_a_casing() {
    // бордюр обязан торчать из-под канта на любом классе — иначе при
    // включённом канте мост неотличим от окантованной дороги
    for width in [3.5_f32, 5.0, 8.0, 16.0] {
        assert!(bridge_curb_width(width) > casing_width(width));
        assert!(bridge_curb_width(width) >= *BRIDGE_CURB_RANGE.start());
        assert!(bridge_curb_width(width) <= *BRIDGE_CURB_RANGE.end());
    }
}

#[test]
fn bridge_curb_ends_are_square_under_every_join() {
    let points = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let max_x = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[0])
            .fold(f32::NEG_INFINITY, f32::max)
    };

    // ровный срез: бордюр кончается ровно на конце осевой при любом стиле стыка
    for join in RoadJoin::ALL {
        let mut curb = MeshBuilder::default();
        push_bridge_curb(&mut curb, &points, 5.0, join);
        assert!(!curb.is_empty());
        assert!(max_x(&curb) <= 20.0 + 1e-4, "curb pokes past the deck end");
    }

    // а заливка со стилем Round — полудиск за концом, для контраста
    let mut fill = MeshBuilder::default();
    push_ribbon(&mut fill, &points, 5.0, LinearRgba::WHITE, RoadJoin::Round);
    assert!(max_x(&fill) > 20.0);
}
