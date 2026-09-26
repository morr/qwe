use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::fixture::street;
use crate::map::shapes::is_ring;

/// Оси при допуске `tolerance`, м: 3 — дефолт ручки, 0 — ось по OSM.
fn axes(roads: &[RoadLine], tolerance: f32) -> Vec<Vec<Vec2>> {
    let network = RoadNetwork::new(roads);
    let nodes = RoadNodes::new(roads);
    let shape = RoadShape {
        curve_tolerance: tolerance,
        ..default()
    };
    street_axes(roads, &[], &network, &nodes, &shape)
        .paths
        .into_iter()
        .map(Cow::into_owned)
        .collect()
}

/// Излом между последним звеном `a` и первым звеном `b`, рад.
fn kink(a: &[Vec2], b: &[Vec2]) -> f32 {
    let incoming = (a[a.len() - 1] - a[a.len() - 2]).normalize();
    let outgoing = (b[1] - b[0]).normalize();
    incoming.angle_to(outgoing).abs()
}

/// Излом между соседними хордами выборки дуги: ось без угла — это ось, чьи
/// изломы не круче шага выборки, в том числе на разрезе между ways.
const SAMPLE_TURN: f32 = 7.0 * PI / 180.0;

/// Наибольший излом между соседними звеньями, рад.
fn sharpest(path: &[Vec2]) -> f32 {
    path.windows(3)
        .filter_map(|w| {
            let a = (w[1] - w[0]).try_normalize()?;
            let b = (w[2] - w[1]).try_normalize()?;
            Some(a.angle_to(b).abs())
        })
        .fold(0.0, f32::max)
}

#[test]
fn a_seam_between_ways_is_one_curve() {
    // два way одной улицы сходятся под 30°: раньше узел шва был закреплён и
    // лента ломалась в нём углом
    let seam = Vec2::new(100.0, 0.0);
    let turn = Vec2::from_angle(30f32.to_radians()) * 100.0;
    let roads = vec![
        street(vec![Vec2::ZERO, seam], 8.0),
        street(vec![seam, seam + turn], 8.0),
    ];
    let paths = axes(&roads, 3.0);
    let (a, b) = (&paths[0], &paths[1]);
    assert_eq!(a[a.len() - 1], b[0], "the pieces meet");
    assert!(kink(a, b) < SAMPLE_TURN, "{}", kink(a, b).to_degrees());
    assert!(sharpest(a).max(sharpest(b)) < SAMPLE_TURN);
    // и не уходит от OSM дальше допуска дуги: два метра из трёх
    for &point in a.iter().chain(b) {
        let off = distance_to_path(point, &roads[0].points)
            .min(distance_to_path(point, &roads[1].points));
        assert!(off <= 2.0 + 0.01, "{point} is {off} m off");
    }
}

#[test]
fn a_reversed_way_keeps_its_own_order() {
    let seam = Vec2::new(100.0, 0.0);
    let far = Vec2::new(180.0, 40.0);
    let roads = vec![
        street(vec![Vec2::ZERO, seam], 8.0),
        street(vec![far, seam], 8.0),
    ];
    let paths = axes(&roads, 3.0);
    assert_eq!(paths[1][0], far);
    assert_eq!(paths[0][0], Vec2::ZERO);
    assert_eq!(paths[0][paths[0].len() - 1], paths[1][paths[1].len() - 1]);
}

#[test]
fn a_junction_node_stays_and_the_through_pair_passes_it_smoothly() {
    // на изломе сквозной улицы кончается поперечная: узел остаётся на
    // месте, но улица проходит его без угла
    let node = Vec2::new(100.0, 0.0);
    let turn = Vec2::from_angle(20f32.to_radians()) * 100.0;
    let roads = vec![
        street(vec![Vec2::ZERO, node], 8.0),
        street(vec![node, node + turn], 8.0),
        street(vec![node, Vec2::new(100.0, -80.0)], 8.0),
    ];
    let paths = axes(&roads, 3.0);
    let through = if paths[0][paths[0].len() - 1] == node {
        (&paths[0], &paths[1])
    } else {
        panic!("the node moved: {:?}", paths[0]);
    };
    assert_eq!(through.1[0], node);
    // у самого узла ось прямая: скругление бордюра к боковому плечу
    // строится только по прямому краю
    let before = through.0[through.0.len() - 2];
    assert!(before.distance(node) > 5.0, "{before} is the last vertex");
    let turn = kink(through.0, through.1);
    assert!(turn < SAMPLE_TURN, "{}", turn.to_degrees());
    assert!(sharpest(through.0).max(sharpest(through.1)) < SAMPLE_TURN);
    assert_eq!(paths[2][0], node, "the side arm still starts at the node");
}

#[test]
fn a_footway_crossing_pins_nothing() {
    // тот же излом, но в узле только пешеходная дорожка: она кончается под
    // асфальтом, и улица срезает излом дугой, как любой свободный
    let node = Vec2::new(100.0, 0.0);
    let turn = Vec2::from_angle(20f32.to_radians()) * 100.0;
    let footway = RoadLine {
        class: RoadClass::Alley,
        highway: crate::map::osm::Highway::Path,
        ..street(vec![node, Vec2::new(100.0, -80.0)], 3.5)
    };
    let roads = vec![street(vec![Vec2::ZERO, node, node + turn], 8.0), footway];
    let path = &axes(&roads, 3.0)[0];
    assert!(
        !path.contains(&node),
        "the crossing stayed pinned: {path:?}"
    );
    assert!(sharpest(path) < SAMPLE_TURN);
}

#[test]
fn a_corner_is_no_tighter_than_the_half_width() {
    // поворот на 90° внутри way на длинных звеньях: радиус не меньше
    // полуширины, иначе внутренний край ленты складывается
    let width = 14.2;
    let roads = vec![street(
        vec![Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)],
        width,
    )];
    let path = &axes(&roads, 3.0)[0];
    for w in path.windows(3) {
        let (a, b) = (w[1] - w[0], w[2] - w[1]);
        let turn = a.angle_to(b).abs();
        if turn > 1e-4 {
            let radius = (a.length() + b.length()) / 2.0 / turn;
            assert!(radius >= width / 2.0 * 0.95, "radius {radius}");
        }
    }
}

#[test]
fn jitter_is_simplified_away() {
    let points: Vec<Vec2> = (0..=20)
        .map(|i| Vec2::new(i as f32 * 10.0, if i % 2 == 0 { 0.0 } else { 0.4 }))
        .collect();
    let roads = vec![street(points, 8.0)];
    let path = &axes(&roads, 3.0)[0];
    assert_eq!(path.as_slice(), &[Vec2::ZERO, Vec2::new(200.0, 0.0)]);
}

#[test]
fn a_closed_way_stays_a_ring() {
    let points = vec![
        Vec2::ZERO,
        Vec2::new(60.0, 0.0),
        Vec2::new(60.0, 60.0),
        Vec2::new(0.0, 60.0),
        Vec2::ZERO,
    ];
    let roads = vec![street(points, 8.0)];
    let path = &axes(&roads, 3.0)[0];
    assert!(is_ring(path), "{path:?}");
    assert!(path.len() > 8);
}

#[test]
fn bridges_and_arches_keep_the_old_centerline() {
    let seam = Vec2::new(100.0, 0.0);
    let mut roads = vec![
        street(vec![Vec2::ZERO, seam], 8.0),
        street(
            vec![seam, Vec2::new(150.0, 20.0), Vec2::new(200.0, 20.0)],
            8.0,
        ),
    ];
    roads[1].bridge = true;
    let nodes = RoadNodes::new(&roads);
    let paths = axes(&roads, 3.0);
    assert_eq!(
        paths[1],
        centerline(&roads[1], Smoothing::Light, &nodes).into_owned()
    );
    assert_eq!(
        paths[0][paths[0].len() - 1],
        seam,
        "the run ends at the bridge"
    );
}

#[test]
fn smoothing_off_is_the_osm_centerline() {
    let roads = vec![street(
        vec![Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)],
        8.0,
    )];
    assert_eq!(axes(&roads, 0.0)[0], roads[0].points);
}
