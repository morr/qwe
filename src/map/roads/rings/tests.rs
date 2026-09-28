use std::f32::consts::TAU;

use super::*;
use crate::map::osm::fixture::street;

const CENTER: Vec2 = Vec2::new(200.0, 100.0);
const RADIUS: f32 = 30.0;

/// Точка на окружности кольца под углом `turn` оборотов.
fn on_circle(turn: f32, radius: f32) -> Vec2 {
    CENTER + Vec2::from_angle(turn * TAU) * radius
}

/// Кольцо против часовой стрелки — гранями по `sides` вершинам, одним
/// замкнутым way.
fn faceted_ring(sides: usize) -> RoadLine {
    let mut points: Vec<Vec2> = (0..sides)
        .map(|side| on_circle(side as f32 / sides as f32, RADIUS))
        .collect();
    points.push(points[0]);
    RoadLine {
        oneway: true,
        ..street(points, 8.0)
    }
}

fn reshaped(roads: &[RoadLine]) -> (Vec<Vec<Vec2>>, Rings) {
    let nodes = RoadNodes::new(roads);
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    let rings = reshape(roads, &nodes, &mut paths);
    (
        paths.into_iter().map(|path| path.into_owned()).collect(),
        rings,
    )
}

#[test]
fn a_faceted_ring_is_drawn_round() {
    let (paths, rings) = reshaped(&[faceted_ring(8)]);
    assert_eq!(rings.list.len(), 1);
    let ring = &rings.list[0];
    assert!(ring.ccw);
    // восьмиугольник вписан в окружность: радиус фигуры — между радиусом
    // вписанной и описанной
    let inscribed = RADIUS * (TAU / 16.0).cos();
    assert!(
        (inscribed..=RADIUS).contains(&ring.mean_radius()),
        "{}",
        ring.mean_radius()
    );
    let path = &paths[0];
    assert!(
        path.len() > 40,
        "гладко, а не восемь граней: {}",
        path.len()
    );
    assert_eq!(path[0], path[path.len() - 1], "замкнутым");
    for point in path {
        let off = (point.distance(ring.center) - ring.mean_radius()).abs();
        assert!(off < 0.6, "точка ушла с окружности на {off}: {point:?}");
    }
}

#[test]
fn shared_nodes_stay_on_the_ring() {
    let mut ring = faceted_ring(8);
    // подход в вершину кольца, отставленную на полтора метра наружу
    let node = on_circle(0.25, RADIUS + 1.5);
    ring.points[2] = node;
    let approach = RoadLine {
        oneway: true,
        ..street(vec![CENTER + Vec2::new(0.0, 90.0), node], 7.0)
    };
    let (paths, _) = reshaped(&[ring, approach]);
    assert!(paths[0].contains(&node), "узел — вершина оси кольца");
    assert_eq!(
        paths[1].last(),
        Some(&node),
        "подход приходит в тот же узел"
    );
}

#[test]
fn arcs_are_chained_into_one_ring() {
    let arc = |from: f32, to: f32| {
        let points = (0..=4)
            .map(|step| on_circle(from + (to - from) * step as f32 / 4.0, RADIUS))
            .collect();
        RoadLine {
            oneway: true,
            roundabout: true,
            ..street(points, 8.0)
        }
    };
    let roads = [arc(0.0, 0.4), arc(0.7, 1.0), arc(0.4, 0.7)];
    let (paths, rings) = reshaped(&roads);
    assert_eq!(rings.list.len(), 1);
    assert_eq!(rings.list[0].roads, vec![0, 2, 1], "по ходу движения");
    for (path, road) in paths.iter().zip(&roads) {
        assert_eq!(path[0], road.points[0]);
        assert_eq!(path.last(), road.points.last());
    }
}

#[test]
fn an_approach_enters_at_an_angle_to_the_ring() {
    let ring = faceted_ring(12);
    let node = ring.points[3];
    // въезд с севера прямо в центр — по радиусу
    let entry = RoadLine {
        oneway: true,
        ..street(vec![node + Vec2::new(0.0, 60.0), node], 7.0)
    };
    let (paths, rings) = reshaped(&[ring, entry]);
    let path = &paths[1];
    assert_eq!(path.last(), Some(&node));
    assert!(path.len() > 3, "по дуге: {path:?}");
    let ring = &rings.list[0];
    let t = ring.param(node).0;
    let arrival = (path[path.len() - 1] - path[path.len() - 2]).normalize();
    let angle = arrival.angle_to(ring.travel(t)).abs();
    // последнее звено — хорда кривой, она отстаёт от касательной на
    // полшага звена
    assert!(
        (angle - ENTRY_ANGLE).abs() < 0.25,
        "приходит под {angle} к ходу кольца"
    );
    assert!(
        arrival.dot(ring.outward(t)) < 0.0,
        "въезд идёт внутрь кольца"
    );
}

#[test]
fn a_two_way_approach_along_the_ring_is_bent_into_it_across() {
    let ring = faceted_ring(12);
    let node = ring.points[3];
    let t = TAU * 3.0 / 12.0;
    let (radial, along) = (Vec2::from_angle(t), Vec2::from_angle(t).perp());
    // двусторонний подход, заведённый в узел вдоль кольца: последние 15 м
    // идут по касательной снаружи, а по лучу — только те, что до них
    let tangential = RoadLine {
        oneway: false,
        ..street(
            vec![
                node + radial * 40.0 + along * 15.0,
                node + radial * 4.0 + along * 15.0,
                node,
            ],
            7.6,
        )
    };
    // и такой же, приходящий по лучу, — его не трогают
    let other = ring.points[9];
    let straight = RoadLine {
        oneway: false,
        ..street(vec![other * 2.0 - CENTER, other], 7.6)
    };
    let (paths, rings) = reshaped(&[ring, tangential.clone(), straight.clone()]);
    let ring = &rings.list[0];
    let path = &paths[1];
    assert_eq!(path.last(), Some(&node));
    let arrival = (path[path.len() - 1] - path[path.len() - 2]).normalize();
    let inward = -ring.outward(ring.param(node).0);
    assert!(
        arrival.dot(inward) > 0.9,
        "приходит в узел по лучу, а не вдоль кольца: {arrival}"
    );
    assert_eq!(paths[2], straight.points, "подход по лучу не гнётся");
}

/// Y-подход (Рязань, витрина 05): две двусторонние ноги из одного узла в два
/// узла кольца — въезд и съезд. Въезд — нога, чей узел ниже по ходу; обе
/// гнутся в узел по касательной, как односторонние, а не к лучу.
#[test]
fn the_legs_of_a_y_approach_enter_and_leave_along_the_ring() {
    let ring = faceted_ring(12);
    // кольцо против часовой: узел 4 (120°) ниже по ходу, чем узел 2 (60°)
    let (upstream, downstream) = (ring.points[2], ring.points[4]);
    let apex = CENTER + Vec2::new(0.0, 70.0);
    let leg = |points: Vec<Vec2>| RoadLine {
        oneway: false,
        ..street(points, 7.6)
    };
    // ноги нарочно в разном порядке точек
    let exit = leg(vec![upstream, apex]);
    let entry = leg(vec![downstream, apex]);
    let (paths, rings) = reshaped(&[ring, exit, entry]);
    assert_eq!(rings.leg_flow(0), None, "кольцо — не нога");
    assert_eq!(
        rings.leg_flow(1),
        Some(true),
        "съезд — от кольца, по точкам"
    );
    assert_eq!(
        rings.leg_flow(2),
        Some(false),
        "въезд — к кольцу, против точек"
    );
    let ring = &rings.list[0];
    for (path, node, into) in [(&paths[1], upstream, false), (&paths[2], downstream, true)] {
        assert_eq!(path[0], node);
        assert!(path.len() > 2, "нога гнётся: {path:?}");
        let t = ring.param(node).0;
        // по ходу потока: въезд приходит в узел, съезд уходит из него
        let away = (path[1] - path[0]).normalize();
        let flow = if into { -away } else { away };
        let angle = flow.angle_to(ring.travel(t)).abs();
        assert!(
            (angle - ENTRY_ANGLE).abs() < 0.25,
            "нога под {angle} к ходу кольца, а не по касательной"
        );
    }
}

/// Вторая нога Y-подхода — хвост чужих улиц (Рязань, витрина 05, север):
/// улица идёт через развилку дальше, а в другой узел кольца её доводит конец
/// поперечной. Короткая дорога из развилки — всё равно нога, въезд по
/// касательной; без хвоста — двусторонний подход, как был.
#[test]
fn a_y_approach_whose_other_leg_is_a_tail_of_two_streets() {
    let ring = faceted_ring(12);
    let (upstream, downstream) = (ring.points[2], ring.points[4]);
    let fork = CENTER + Vec2::new(0.0, 70.0);
    let joint = CENTER + Vec2::new(12.0, 45.0);
    let two_way = |points: Vec<Vec2>| RoadLine {
        oneway: false,
        ..street(points, 7.6)
    };
    let leg = two_way(vec![downstream, fork]);
    let through = two_way(vec![CENTER + Vec2::new(-80.0, 90.0), fork, joint]);
    let across = two_way(vec![CENTER + Vec2::new(12.0, 120.0), joint, upstream]);
    let (_, rings) = reshaped(&[ring.clone(), leg.clone(), through.clone(), across]);
    assert_eq!(
        rings.leg_flow(1),
        Some(false),
        "въезд — к кольцу, против точек"
    );
    assert_eq!(rings.leg_flow(2), None, "хвост рисуется как есть");
    assert_eq!(rings.leg_flow(3), None);
    let (_, lone) = reshaped(&[ring.clone(), leg.clone(), through]);
    assert_eq!(lone.leg_flow(1), None, "без хвоста — не нога");
    // односторонний соседний подход из той же развилки — не хвост: улица
    // хвоста идёт через развилку дальше (Рязань 04)
    let beside = RoadLine {
        oneway: true,
        ..street(vec![fork, upstream], 7.6)
    };
    let (_, apart) = reshaped(&[ring, leg, beside]);
    assert_eq!(apart.leg_flow(1), None, "соседний подход — не хвост");
}

/// Одна двусторонняя дорога в узел кольца — не нога, даже если рядом
/// другая кончается в том же узле кольца.
#[test]
fn two_ways_into_one_ring_node_are_no_y_approach() {
    let ring = faceted_ring(12);
    let node = ring.points[3];
    let apex = CENTER + Vec2::new(0.0, 70.0);
    let leg = |points: Vec<Vec2>| RoadLine {
        oneway: false,
        ..street(points, 7.6)
    };
    let (_, rings) = reshaped(&[ring, leg(vec![apex, node]), leg(vec![apex, node])]);
    assert_eq!(rings.leg_flow(1), None);
    assert_eq!(rings.leg_flow(2), None);
}

#[test]
fn a_long_loop_is_not_a_ring() {
    let mut points = vec![
        CENTER,
        CENTER + Vec2::new(120.0, 0.0),
        CENTER + Vec2::new(120.0, 20.0),
        CENTER + Vec2::new(0.0, 20.0),
    ];
    points.push(points[0]);
    let loop_road = RoadLine {
        oneway: true,
        ..street(points.clone(), 6.0)
    };
    let (paths, rings) = reshaped(&[loop_road]);
    assert!(rings.list.is_empty(), "прямоугольник — не кольцо");
    assert_eq!(paths[0], points);
}

/// Обходной съезд идёт вдоль кольца снаружи, в узлы его не заходя, и кромки
/// их разошлись на метр (Тула, витрина 04, юго-восток): щель закрыта
/// асфальтом, иначе в ней серпом проступает тротуар. Улица, упёршаяся в
/// кольцо поперёк, перепонки не получает — там угол бордюра.
#[test]
fn a_slip_road_along_the_ring_is_webbed_to_it() {
    // кромка кольца — в 30 + 4 м от центра, кромка съезда — на метр дальше
    let slip = RoadLine {
        oneway: true,
        ..street(
            (0..=15)
                .map(|step| on_circle(0.05 + 0.01 * step as f32, RADIUS + 4.0 + 1.0 + 3.5))
                .collect(),
            7.0,
        )
    };
    let across = street(
        vec![on_circle(0.6, RADIUS + 40.0), on_circle(0.6, RADIUS)],
        7.0,
    );
    let (_, rings) = reshaped(&[faceted_ring(12), slip, across]);
    assert_eq!(rings.webs.len(), 1, "{:?}", rings.webs);
    let web = &rings.webs[0];
    assert!(
        web.iter()
            .all(|point| point.distance(CENTER) > RADIUS - 1.0),
        "перепонка снаружи кольца"
    );
    let (low, high) = web
        .iter()
        .map(|point| (*point - CENTER).to_angle().rem_euclid(TAU) / TAU)
        .fold((1.0_f32, 0.0_f32), |(low, high), turn| {
            (low.min(turn), high.max(turn))
        });
    assert!(
        low < 0.07 && high > 0.18,
        "перепонка на всю длину съезда: {low}..{high}"
    );
}
