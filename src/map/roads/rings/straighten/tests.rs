use super::super::{Rings, reshape};
use super::*;
use crate::map::osm::RoadClass;
use crate::map::osm::fixture::street;

const CENTER: Vec2 = Vec2::new(200.0, 100.0);
const RADIUS: f32 = 21.0;

/// Точка под углом `degrees` от центра кольца в `reach` метрах.
fn polar(degrees: f32, reach: f32) -> Vec2 {
    CENTER + Vec2::from_angle(degrees.to_radians()) * reach
}

/// Кольцо против часовой стрелки в 24 грани — узел каждые 15°.
fn ring() -> RoadLine {
    let mut points: Vec<Vec2> = (0..24)
        .map(|side| polar(side as f32 * 15.0, RADIUS))
        .collect();
    points.push(points[0]);
    RoadLine {
        oneway: true,
        ..street(points, 7.0)
    }
}

fn two_way(points: Vec<Vec2>) -> RoadLine {
    RoadLine {
        oneway: false,
        ..street(points, 7.6)
    }
}

/// Север кольца Рязани (витрина 05): нога из развилки в десяти метрах от оси
/// в узел под 135°, улица через развилку кончается в стыке в трёх с
/// половиной метрах от оси, и поперечная от стыка вдоль кольца доходит до
/// узла под 75°.
fn ryazan_north() -> (Vec<RoadLine>, Vec2, Vec2) {
    let (entry, exit) = (polar(135.0, RADIUS), polar(75.0, RADIUS));
    let (fork, joint) = (polar(122.0, 31.0), polar(107.0, 24.6));
    let roads = vec![
        ring(),
        two_way(vec![fork, entry]),
        two_way(vec![polar(150.0, 120.0), fork, joint]),
        two_way(vec![polar(95.0, 120.0), polar(100.0, 60.0), joint, exit]),
    ];
    (roads, entry, exit)
}

fn legs(roads: &[RoadLine]) -> Rings {
    let nodes = RoadNodes::new(roads);
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    reshape(roads, &nodes, &mut paths)
}

#[test]
fn a_y_tail_along_the_ring_becomes_two_legs_from_a_fork_off_it() {
    let (mut roads, entry, exit) = ryazan_north();
    let ring_before = roads[0].points.clone();
    assert_eq!(straighten_tails(&mut roads).tails, 1);
    assert_eq!(roads[0].points, ring_before, "кольцо не трогается");
    assert_eq!(roads.len(), 5, "конец поперечной — отдельным way");
    let fork = roads[1].points[0];
    // развилка — на луче из середины дуги между ногами, отнесена от оси
    let off = fork.distance(CENTER) - RADIUS;
    assert!(
        (FORK_MIN..FORK_MIN + 6.0).contains(&off),
        "развилка в {off} м от оси"
    );
    let bearing = (fork - CENTER).to_angle().to_degrees();
    assert!((bearing - 105.0).abs() < 3.0, "развилка под {bearing}°");
    assert_eq!(roads[1].points, vec![fork, entry], "нога — прямо");
    assert_eq!(
        roads[2].points.last(),
        Some(&fork),
        "улица через развилку кончается в ней"
    );
    assert_eq!(roads[2].points.len(), 2, "кусок до стыка ушёл");
    assert_eq!(
        roads[3].points.last(),
        Some(&fork),
        "поперечная приходит в развилку, а не вдоль кольца"
    );
    assert_eq!(roads[4].points, vec![fork, exit], "вторая нога — прямо");
    // и дальше это обычный «Y»: обе ноги — въезд и съезд
    let rings = legs(&roads);
    assert!(rings.leg_flow(1).is_some());
    assert!(rings.leg_flow(4).is_some());
    assert_eq!(rings.leg_flow(2), None);
    assert_eq!(rings.leg_flow(3), None);
}

#[test]
fn other_approaches_are_not_straightened() {
    let (entry, exit) = (polar(135.0, RADIUS), polar(75.0, RADIUS));
    let apex = polar(105.0, 36.0);
    let cases = [
        // «Y» из двух своих way
        vec![
            ring(),
            two_way(vec![apex, entry]),
            two_way(vec![exit, apex]),
            two_way(vec![polar(105.0, 120.0), apex]),
        ],
        // двусторонний подход по лучу
        vec![ring(), two_way(vec![polar(105.0, 120.0), exit])],
        // короткая дорога из развилки без хвоста
        vec![
            ring(),
            two_way(vec![apex, entry]),
            two_way(vec![polar(150.0, 120.0), apex, polar(60.0, 120.0)]),
        ],
    ];
    for mut roads in cases {
        let before: Vec<Vec<Vec2>> = roads.iter().map(|road| road.points.clone()).collect();
        assert_eq!(straighten_tails(&mut roads).tails, 0);
        let after: Vec<Vec<Vec2>> = roads.iter().map(|road| road.points.clone()).collect();
        assert_eq!(after, before);
    }
}

/// Дорожка, что пересекает хвост посреди куска вдоль кольца, осталась бы
/// висеть: такой хвост не выпрямляется.
#[test]
fn a_tail_crossed_by_another_road_is_left_as_it_is() {
    let (mut roads, _, exit) = ryazan_north();
    let middle = polar(90.0, 22.5);
    let across = roads.len() - 1;
    let joint = polar(107.0, 24.6);
    roads[across].points = vec![polar(95.0, 120.0), polar(100.0, 60.0), joint, middle, exit];
    roads.push(RoadLine {
        class: RoadClass::Alley,
        ..street(vec![polar(90.0, 60.0), middle], 2.0)
    });
    assert_eq!(straighten_tails(&mut roads).tails, 0);
}
