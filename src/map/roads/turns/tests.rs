use super::*;
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, MapData};
use crate::map::roads::is_carriageway;
use crate::map::roads::junctions::marking_breaks;
use crate::map::roads::network::RoadNetwork;
use crate::map::roads::node_paint::{CrossingMode, NodePaint, NodePaintStyle};

const NODE: Vec2 = Vec2::new(100.0, 0.0);

fn road(points: Vec<Vec2>, highway: Highway, lanes: u8) -> RoadLine {
    RoadLine {
        highway,
        lanes: Some(lanes),
        ..street(points, f32::from(lanes) * STREET_LANE_WIDTH + 1.0)
    }
}

/// Улица по оси x через [`NODE`].
fn through(highway: Highway) -> RoadLine {
    road(vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)], highway, 2)
}

/// Примыкание к [`NODE`] снизу, с юга.
fn side(highway: Highway) -> RoadLine {
    road(vec![Vec2::new(100.0, -80.0), NODE], highway, 2)
}

fn turns_of(roads: Vec<RoadLine>, side: TrafficSide) -> (Turns, NodePaint) {
    let mut map = MapData {
        roads,
        traffic_side: side,
        ..default()
    };
    map.network = RoadNetwork::new(&map.roads);
    let drawn: Vec<&RoadLine> = map.roads.iter().collect();
    let paths: Vec<Vec<Vec2>> = map.roads.iter().map(|road| road.points.clone()).collect();
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let paint = NodePaint::new(
        &drawn,
        &paths,
        (&base, &[]),
        &map,
        NodePaintStyle {
            crossings: CrossingMode::Off,
            stop_lines: false,
        },
        |_| true,
        |_| Vec::new(),
    );
    let turns = Turns::new(&drawn, &paths, &paint.junctions, side);
    (turns, paint)
}

/// Концы кривой — середины полос на кромках узла.
fn body(path: &[Vec2]) -> (Vec2, Vec2) {
    (path[0], path[path.len() - 1])
}

/// Кривые всех узлов подряд.
fn curves(turns: &Turns) -> Vec<&[Vec2]> {
    turns
        .wear
        .iter()
        .flat_map(|junction| junction.curves.iter().map(Vec::as_slice))
        .collect()
}

#[test]
fn the_main_road_keeps_its_ruts_and_turns_get_curves() {
    let (turns, paint) = turns_of(
        vec![through(Highway::Tertiary), side(Highway::Residential)],
        TrafficSide::Right,
    );
    assert_eq!(paint.junctions.len(), 1);
    assert_eq!(paint.junctions[0].leading, vec![0], "главная ведёт узел");
    assert!(
        paint.asphalt[0].iter().all(|found| found.at != NODE),
        "колея главной идёт сквозь: {:?}",
        paint.asphalt[0]
    );
    assert!(
        paint.asphalt[1]
            .iter()
            .any(|found| found.at == NODE && found.reach > 0.0)
    );
    // прямо по главной — асфальтом; четыре поворота: по два с главной и на неё
    assert_eq!(turns.maneuvers, 4);
    // хвост — один на полосу: две полосы главной с каждой стороны и две
    // примыкания, у каждой свой, хоть манёвров из неё и в неё несколько
    let wear = &turns.wear[0];
    assert_eq!(wear.tails.len(), 6, "{:?}", wear.tails);
    for [edge, deep] in &wear.tails {
        assert!((edge.distance(*deep) - TURN_TAIL).abs() < 1e-3);
    }
}

#[test]
fn equal_streets_cross_each_other_by_curves() {
    let cross = road(
        vec![Vec2::new(100.0, -80.0), NODE, Vec2::new(100.0, 80.0)],
        Highway::Residential,
        2,
    );
    let (turns, paint) = turns_of(
        vec![through(Highway::Residential), cross],
        TrafficSide::Right,
    );
    assert!(paint.junctions[0].leading.is_empty(), "ведущей нет");
    // четыре прямо, четыре направо и четыре налево
    assert_eq!(turns.maneuvers, 12);
}

#[test]
fn the_near_turn_leaves_the_kerb_lane() {
    let (turns, _) = turns_of(
        vec![through(Highway::Tertiary), side(Highway::Residential)],
        TrafficSide::Right,
    );
    // с запада на юг — направо: от правой полосы (y < 0) в правую полосу юга
    // (x < 100, если смотреть по ходу вниз — справа запад)
    let right = curves(&turns)
        .into_iter()
        .find(|path| {
            let (start, end) = body(path);
            start.x < NODE.x && end.y < NODE.y - 1.0
        })
        .expect("поворот с запада на юг");
    let (start, end) = body(right);
    assert!(start.y < 0.0, "из полосы у правого бордюра: {start:?}");
    assert!(end.x < NODE.x, "в полосу у правого бордюра: {end:?}");
}

#[test]
fn left_hand_traffic_turns_near_to_the_left() {
    let (turns, _) = turns_of(
        vec![through(Highway::Tertiary), side(Highway::Residential)],
        TrafficSide::Left,
    );
    // с запада при левостороннем едут по верхней полосе (y > 0), куда бы ни
    // поворачивали
    for path in curves(&turns) {
        let (start, _) = body(path);
        // кромка западного плеча — в полуширине примыкания и метре от узла
        if start.x < NODE.x - 4.0 {
            assert!(
                start.y > 0.0,
                "левостороннее: с запада — по верхней: {start:?}"
            );
        }
    }
}

#[test]
fn tagged_lanes_decide_which_lanes_turn() {
    let mut main = road(
        vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
        Highway::Tertiary,
        2,
    );
    main.oneway = true;
    let right = LaneTurn {
        right: true,
        ..default()
    };
    let both = LaneTurn {
        through: true,
        right: true,
        ..default()
    };
    // по правилу направо — только из правой полосы
    let (rule, _) = turns_of(
        vec![main.clone(), side(Highway::Residential)],
        TrafficSide::Right,
    );
    main.turns = [vec![both, right], Vec::new()];
    let (tagged, _) = turns_of(vec![main, side(Highway::Residential)], TrafficSide::Right);
    let rights = |turns: &Turns| {
        curves(turns)
            .into_iter()
            .filter(|path| body(path).0.x < NODE.x - 1.0 && body(path).1.y < -1.0)
            .count()
    };
    assert_eq!(rights(&rule), 1);
    assert_eq!(
        rights(&tagged),
        2,
        "`through;right|right` — направо из обеих"
    );
}

#[test]
fn a_straight_curve_is_one_link_and_a_turn_is_many() {
    let lane = |point: Vec2, travel: Vec2| LaneEnd { point, travel };
    let straight = curve(
        lane(Vec2::ZERO, Vec2::X),
        lane(Vec2::new(50.0, 0.0), Vec2::X),
    );
    assert_eq!(straight.len(), 2, "прямо без сдвига — одно звено");
    let turn = curve(
        lane(Vec2::ZERO, Vec2::X),
        lane(Vec2::new(10.0, -10.0), -Vec2::Y),
    );
    // четверть круга радиуса 10 м: хорда звена отходит от дуги не больше
    // чем на 3 см — не меньше девяти звеньев
    assert!(turn.len() >= 10, "{}", turn.len());
    let worst = turn
        .windows(3)
        .map(|link| {
            // стрелка средней точки над хордой двух звеньев — вчетверо
            // стрелки одного
            let chord = link[2] - link[0];
            (link[1] - link[0]).perp_dot(chord).abs() / chord.length()
        })
        .fold(0.0_f32, f32::max);
    assert!(worst < 4.0 * SAGITTA + 0.01, "грани: {worst}");
}
