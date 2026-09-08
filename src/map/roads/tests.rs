use super::*;
use crate::map::meshing::distance_to_path;

fn road(points: Vec<Vec2>, width: f32, passage: bool) -> RoadLine {
    RoadLine {
        points,
        width,
        class: RoadClass::Alley,
        bridge: false,
        passage,
    }
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
fn smooth_path_applies_where_nothing_is_pinned() {
    // `smooth_path` — общий вход для рельсов, трамвая и зелёной полосы: в
    // отличие от `centerline` ему нечего закреплять, и сглаживание он
    // применяет всегда
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    assert!(smooth_path(&points, 5.0, RoadSmoothing::Strong).len() > 3);
    assert!(matches!(
        smooth_path(&points, 5.0, RoadSmoothing::Off),
        Cow::Borrowed(_)
    ));
}

#[test]
fn casing_is_wider_than_the_fill() {
    // диапазоны самих ширин — тесты `map::footprint`; здесь — что кант
    // геометрически торчит из-под заливки
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
