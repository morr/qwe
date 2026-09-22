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
        ..street(points, f32::from(lanes) * lane_width() + 1.0)
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
    let turns = Turns::new(&drawn, &paths, &paint.junctions, side, |_| false);
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

/// Стрелки: по тегу — на каждой полосе с тегом; без тега — только у
/// двухполосного в своём направлении подхода, по правилу: крайняя правая —
/// прямо и направо, крайняя левая — прямо и налево. Однополосная боковая
/// улица стрелок не получает.
#[test]
fn arrows_follow_the_tag_or_the_rule_on_a_wide_approach() {
    let mut main = road(
        vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
        Highway::Tertiary,
        2,
    );
    main.oneway = true;
    let (rule, _) = turns_of(
        vec![main.clone(), side(Highway::Residential)],
        TrafficSide::Right,
    );
    // подход главной — две полосы на восток; боковая двусторонняя — по одной
    let arrows: Vec<&LaneArrow> = rule.arrows.iter().collect();
    assert_eq!(arrows.len(), 2, "{arrows:?}");
    assert!(arrows.iter().all(|arrow| arrow.travel.x > 0.9));
    let kerb = arrows
        .iter()
        .min_by(|a, b| a.at.y.total_cmp(&b.at.y))
        .unwrap();
    assert!(kerb.turn.right && kerb.turn.through && !kerb.turn.left);

    let left = LaneTurn {
        left: true,
        ..default()
    };
    main.turns = [vec![left, LaneTurn::default()], Vec::new()];
    let (tagged, _) = turns_of(vec![main, side(Highway::Residential)], TrafficSide::Right);
    assert_eq!(tagged.arrows.len(), 1, "полоса без манёвров — без стрелки");
    assert!(tagged.arrows[0].turn.left);
}

/// Развилка разделённой улицы (кольцо «Макси», пример 17): съезд в две полосы
/// кончается там, где половины сходятся в двустороннюю, а вторая половина
/// уходит назад разворотом. Выбора нет — одно «прямо», и стрелок по правилу
/// нет; две стрелки «прямо» читались как лишняя разметка.
#[test]
fn no_rule_arrows_where_the_approach_has_no_choice() {
    let mut exit = road(vec![Vec2::ZERO, NODE], Highway::Unclassified, 2);
    exit.oneway = true;
    let mut entry = road(vec![NODE, Vec2::new(0.0, -20.0)], Highway::Unclassified, 2);
    entry.oneway = true;
    let onward = road(vec![NODE, Vec2::new(200.0, 0.0)], Highway::Unclassified, 4);
    let (turns, _) = turns_of(vec![exit, entry, onward], TrafficSide::Right);
    // встречный подход с востока тоже без выбора: только прямо во вторую
    // половину
    assert!(turns.arrows.is_empty(), "{:?}", turns.arrows);
}

/// На изогнутом подходе стрелка идёт по своей полосе, а не по прямой от
/// кромки: ось стрелки держит сдвиг полосы от оси дороги на всей длине, в том
/// числе за двадцать метров от узла, где прямая уже ушла бы с полосы.
#[test]
fn an_arrow_follows_its_lane_on_a_curved_approach() {
    // подход дугой радиусом 40 м, входит в узел с запада
    let arc: Vec<Vec2> = (0..=12)
        .map(|step| {
            let angle = std::f32::consts::FRAC_PI_2 * (1.0 - step as f32 / 12.0);
            NODE + Vec2::new(-40.0 * angle.sin(), 40.0 - 40.0 * angle.cos())
        })
        .collect();
    let mut main = road(
        arc.into_iter().chain([Vec2::new(200.0, 0.0)]).collect(),
        Highway::Tertiary,
        2,
    );
    main.oneway = true;
    let path = main.points.clone();
    let (turns, _) = turns_of(vec![main, side(Highway::Residential)], TrafficSide::Right);
    assert_eq!(turns.arrows.len(), 2);
    for arrow in &turns.arrows {
        let offset = crate::map::meshing::distance_to_path(arrow.at, &path);
        let (along, total) = crate::map::along::arclengths(&arrow.back);
        assert!(
            total >= 25.0,
            "ось полосы короче двадцати пяти метров: {total}"
        );
        let (far, _) = crate::map::along::place_on_path(&arrow.back, &along, 25.0).unwrap();
        let drift = crate::map::meshing::distance_to_path(far, &path) - offset;
        assert!(drift.abs() < 0.2, "стрелка ушла с полосы на {drift} м");
        // прямая от кромки в тех же 25 м ушла бы на метры
        let straight = arrow.at - arrow.travel.normalize() * 25.0;
        let off = crate::map::meshing::distance_to_path(straight, &path) - offset;
        assert!(off.abs() > 2.0, "{off}");
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
fn at_the_stem_of_a_t_the_middle_lanes_turn_both_ways() {
    let lane = LaneEnd {
        point: Vec2::ZERO,
        travel: Vec2::X,
        offset: 0.0,
    };
    let arm = ArmLanes {
        lanes: vec![lane; 3],
        turns: None,
    };
    let from = |maneuver, dead_end| {
        let mut lanes: Vec<usize> = lane_pairs(&arm, 3, maneuver, TrafficSide::Right, dead_end)
            .into_iter()
            .map(|(from, _)| from)
            .collect();
        lanes.sort_unstable();
        lanes
    };
    // есть «прямо»: поворачивает только крайняя к повороту полоса
    assert_eq!(from(Maneuver::Near, false), vec![0]);
    assert_eq!(from(Maneuver::Far, false), vec![2]);
    assert_eq!(from(Maneuver::Straight, false), vec![0, 1, 2]);
    // торец Т: средняя — в обе стороны, крайние — каждая только в свою
    assert_eq!(from(Maneuver::Near, true), vec![0, 1]);
    assert_eq!(from(Maneuver::Far, true), vec![1, 2]);
}

#[test]
fn a_straight_curve_is_one_link_and_a_turn_is_many() {
    let lane = |point: Vec2, travel: Vec2| LaneEnd {
        point,
        travel,
        offset: 0.0,
    };
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
