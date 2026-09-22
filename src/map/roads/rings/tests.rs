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
