use std::f32::consts::TAU;

use super::super::fixtures::{
    layer, polar, roundabout_with_an_approach, to_break_where, with_network,
};
use super::super::medians::MEDIAN_KERB;
use super::super::{
    Axis, CrossingMode, Drawn, RoadShape, RoadStyle, VergeZoomBucket, junctions, mesh_roads,
    node_paint, paint,
};
use super::RING_KERB_LINE;
use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::fixture::street;
use crate::map::osm::{AreaKind, Highway, MapData, PolyArea, RoadLine, fixture};

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

/// Большое кольцо в три полосы, радиус 40 м, одним замкнутым односторонним
/// way против часовой — и подход `arm` к вершине `node` его 48-угольника
/// (точки подхода — `arm_points(ring)`, вершина берётся из того же круга).
fn a_big_ring_with(arm_points: impl Fn(&[Vec2]) -> Vec<Vec2>) -> MapData {
    let circle: Vec<Vec2> = (0..=48)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 48.0) * 40.0)
        .collect();
    let primary = |points: Vec<Vec2>, lanes: u8| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(lanes),
        ..fixture::street(points, f32::from(lanes) * 3.3 + 1.0)
    };
    let ring = RoadLine {
        roundabout: true,
        ..primary(circle.clone(), 3)
    };
    let arm = primary(arm_points(&circle), 2);
    with_network(vec![ring, arm])
}

/// Обочина у дуги кольца — только снаружи: внутри остров и его газон, а
/// снаружи между тротуаром кольца и дорожкой вдоль него лежала голая земля
/// (Калуга, витрина 01).
#[test]
fn a_ring_takes_its_verge_on_the_outer_side_only() {
    let mut map = roundabout_with_an_approach(true, true);
    map.roads[0].verges = [6.0, 6.0];
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let radii: Vec<f32> = layer(&layers, "road_verge_yards")
        .builder
        .positions_for_test()
        .iter()
        .map(|at| Vec2::new(at[0], at[1]).length())
        .collect();
    assert!(
        radii.iter().any(|&r| r > 12.0 + 4.0 + 5.9),
        "обочина снаружи: {radii:?}"
    );
    // внутрь — не дальше внутренней кромки полотна (торцы-круги лежат под ним)
    assert!(
        radii.iter().all(|&r| r > 12.0 - 4.0 - 0.1),
        "обочина на острове: {radii:?}"
    );
}

/// Остров кольца, к которому не ведёт ни одна дорожка (R28, Белгород): по
/// его кромке — тонкая линия бордюра, а не полоса тротуара, и тротуар
/// подхода не заходит на остров полукругом своего торца. Дорожка, что
/// кончается на острове, возвращает бордюру ширину полосы.
#[test]
fn an_island_no_path_reaches_gets_a_kerb_line_only() {
    // внутренняя кромка полотна — в 12 − 4 м от центра
    let inner = 12.0 - 4.0;
    // ближе всех к центру — из вершин тротуаров, не закрытых газоном острова
    let deepest = |map: &MapData| -> f32 {
        let (layers, _) = mesh_roads(
            map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        let lawn = &layer(&layers, "road_medians").builder;
        layer(&layers, "sidewalks")
            .builder
            .positions_for_test()
            .iter()
            .map(|at| Vec2::new(at[0], at[1]))
            // дорожка на остров идёт по x = 0 в верхней половине — её плитка
            // не в счёт
            .filter(|at| at.y < 0.0 || at.x.abs() > 3.0)
            .filter(|at| !lawn.covers_for_test(*at * 0.999))
            .map(Vec2::length)
            .fold(f32::INFINITY, f32::min)
    };
    let bare = roundabout_with_an_approach(true, true);
    let reach = inner - deepest(&bare);
    assert!(
        (0.0..=RING_KERB_LINE + 0.05).contains(&reach),
        "тротуар на острове без дорожек: {reach} м"
    );
    let mut reached = roundabout_with_an_approach(true, true);
    reached.roads.push(fixture::footway(vec![
        Vec2::new(0.0, 30.0),
        Vec2::new(0.0, 2.0),
    ]));
    let reach = inner - deepest(&reached);
    assert!(
        (MEDIAN_KERB - 0.05..=MEDIAN_KERB + 0.05).contains(&reach),
        "бордюр острова, куда ведёт дорожка: {reach} м"
    );
}

/// Клин между въездом, съездом и кольцом — направляющий островок: асфальт со
/// штриховкой, как рисует Яндекс, — и стоянка для этого не нужна.
#[test]
fn the_wedge_between_the_two_arms_of_a_roundabout_is_hatched() {
    // с тегом — и **без него**: большое кольцо у ТРЦ «Макси» в OSM просто
    // замкнутое одностороннее полотно, и по одному тегу его островки не
    // находились вовсе
    for tagged in [true, false] {
        let map = roundabout_with_an_approach(tagged, true);
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        // клин — правее кольца, между подходами, у оси x: краска его кроет
        // (обводка и штриховка — площади, и вершины у них только на концах
        // клина, так что спрашивается покрытие, а не вершины)
        let islands = &layer(&layers, paint::PAINT_ISLANDS).builder;
        assert!(
            [20.0, 28.0, 36.0]
                .iter()
                .all(|&x| islands.covers_for_test(Vec2::new(x, 0.0))),
            "клин не заштрихован ({tagged})"
        );
        let lines = islands.positions_for_test();
        // и ничего не заштриховано с внешней стороны подходов
        assert!(lines.iter().all(|at| at[0] > 10.0 && at[1].abs() < 9.0));
    }
}

/// Треугольник тротуара между въездом, съездом и кольцом кроет **асфальт**
/// островка, а не его штриховка: он идёт в слой улиц (выше тротуаров) и выпущен
/// на `ASPHALT_PAD` под кромки полотен, чтобы между ним и лентой дороги не
/// оставалось волоска земли. Ни одной ручки `RoadStyle` у этой фигуры нет —
/// держит её только тест.
#[test]
fn the_gore_is_paved_under_the_edges_of_both_arms() {
    let map = roundabout_with_an_approach(true, true);
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.gores, 1, "островок у кольца один");

    // клин целиком: от кромки кольца (радиус 16 м) до острия, где полотна
    // сходятся (x ≈ 45, и асфальт выпущен ещё дальше); подходы — дороги 1 и 2
    // сцены. Своих вершин у лент подходов тут нет — они прямые, и вершины у них
    // только на торцах
    let arms = [
        map.roads[1].points.as_slice(),
        map.roads[2].points.as_slice(),
    ];
    let wedged = |at: &[f32; 3]| (14.0..50.0).contains(&at[0]) && at[1].abs() < 5.0;
    // кромка полотна — в 2.5 м от его оси, так что ближе неё лежит только то,
    // что зашло **под** ленту
    let depth = |at: &[f32; 3]| {
        let at = Vec2::new(at[0], at[1]);
        arms.iter()
            .map(|arm| distance_to_path(at, arm))
            .fold(f32::INFINITY, f32::min)
    };
    let deepest = |name: &str| {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|at| wedged(at))
            .map(&depth)
            .fold(f32::INFINITY, f32::min)
    };
    const UNDER: f32 = 2.3;
    let (asphalt, hatching) = (deepest("roads"), deepest(paint::PAINT_ISLANDS));
    assert!(
        asphalt < UNDER,
        "асфальт островка не зашёл под кромки полотен: {asphalt}"
    );
    // ...а штриховка у́же его ровно на этот заход и на полотно не лезет:
    // обводка — площадь внутри клина, а не лента, вылезающая из него
    assert!(
        hatching.is_finite() && hatching > UNDER,
        "штриховка островка вылезла на полотно: {hatching}"
    );
}

/// Широкий веер: въезд и съезд расходятся от общего узла к кольцу под
/// ±75°, и у кольца между их кромками больше двух радиусов замыкания — оно
/// клин не затягивало, островок выходил обрывком у острия (пример 04,
/// север). Веер берётся целиком — от кольца до узла.
#[test]
fn a_wide_fan_is_hatched_up_to_the_ring() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 20.0)
        .collect();
    let apex = Vec2::new(52.0, 0.0);
    let arm = |from: Vec2| RoadLine {
        oneway: true,
        ..fixture::street(vec![from, apex], 5.0)
    };
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads.push(arm(circle[5]));
    map.roads.push(arm(circle[19]));
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.gores, 1, "{report}");
    let lines = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test();
    // у кольца (его кромка — на 24 м): там подходы в двух десятках метров, и
    // замыкание оставило бы штриховку только у острия, за 35 м
    let nearest = lines
        .iter()
        .filter(|at| at[1].abs() < 5.0)
        .map(|at| at[0])
        .fold(f32::INFINITY, f32::min);
    assert!(nearest < 30.0, "у кольца клин не заштрихован: {nearest}");
}

/// Двусторонний подход одним way — веера из въезда и съезда нет, и клин
/// [`gores::Gores::of`] не находит. Островок ставится по правилу: капля на оси
/// подхода от кромки кольца, подход вокруг неё раздвинут асфальтом.
#[test]
fn a_two_way_approach_gets_a_splitter_island() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 25.0)
        .collect();
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads
        .push(fixture::street(vec![circle[0], Vec2::new(90.0, 0.0)], 7.6));
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.rings[0], 1);
    assert_eq!(report.gores, 1, "островок на подходе один");
    // капля — на оси подхода за кромкой кольца (25 + 4 м), не дальше острия
    let island: Vec<&[f32; 3]> = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test()
        .iter()
        .collect();
    assert!(!island.is_empty());
    assert!(
        island
            .iter()
            .all(|at| (28.0..50.0).contains(&at[0]) && at[1].abs() < 3.0),
        "островок не на подходе"
    );
    // и подход раздвинут: асфальт шире полотна по сторонам островка
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (30.0..35.0).contains(&at[0]) && at[1].abs() > 7.6 / 2.0 + 0.5),
        "подход у островка не расширен"
    );
}

/// Подход, заведённый в узел кольца по касательной (Рязань, витрина 05):
/// ось гнётся в узел по лучу (`rings::reshape`), а островок, если встал, —
/// за кромкой кольца, не на его полотне.
#[test]
fn a_splitter_island_stays_off_the_ring_when_the_approach_comes_in_along_it() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 25.0)
        .collect();
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads.push(fixture::street(
        vec![Vec2::new(31.0, 70.0), Vec2::new(31.0, 16.0), circle[0]],
        7.6,
    ));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let island = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test();
    let edge = 25.0 + 8.0 / 2.0;
    assert!(
        island
            .iter()
            .all(|at| Vec2::new(at[0], at[1]).length() > edge),
        "островок лежит на полотне кольца"
    );
}

/// Y-подход (Рязань, витрина 05): две двусторонние ноги из одного узла в два
/// узла кольца — въезд и съезд, и клин между ними — один островок, за
/// кромкой кольца, между ногами; по островку на каждую ногу не ставится.
#[test]
fn a_y_approach_gets_one_island_between_its_legs() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 25.0)
        .collect();
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    let apex = Vec2::new(0.0, 62.0);
    map.roads.push(fixture::street(vec![apex, circle[4]], 7.6));
    map.roads.push(fixture::street(vec![circle[8], apex], 7.6));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let island: Vec<Vec2> = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test()
        .iter()
        .map(|at| Vec2::new(at[0], at[1]))
        .collect();
    assert!(!island.is_empty(), "островка нет");
    // кромка кольца — по хордам граней обводки клина, обводка островка —
    // полоса: метр допуска, крюк на полотне заходил бы на метры
    let edge = 25.0 + 8.0 / 2.0 - 1.0;
    let nearest = island
        .iter()
        .map(|at| at.length())
        .fold(f32::INFINITY, f32::min);
    assert!(nearest > edge, "островок на полотне кольца: {nearest}");
    assert!(
        island.iter().all(|at| at.x.abs() < 14.0),
        "островок не между ногами: {island:?}"
    );
    // нога — въезд или съезд, в одну полосу
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    for leg in [1, 2] {
        assert!(drawn.road(leg).width < map.roads[leg].width);
        assert_eq!(drawn.road(leg).lanes, Some(1));
    }
}

/// Остров кольца — газон под всем, что на нём замаплено: заливка по оси
/// кольца, до его внутренней кромки; у двустороннего кольца без тега (не
/// кольцо) — ничего.
#[test]
fn a_roundabout_island_is_a_lawn_under_everything() {
    use crate::settings::{Z_GROUND, Z_LANDUSE, Z_ROAD_VERGE};
    let map = roundabout_with_an_approach(true, true);
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let lawn = layer(&layers, "ring_islands");
    assert!(lawn.z > Z_GROUND && lawn.z < Z_ROAD_VERGE.min(Z_LANDUSE));
    let positions = lawn.builder.positions_for_test();
    assert!(!positions.is_empty());
    // вершины — на оси кольца радиусом 12 м, не дальше
    assert!(
        positions
            .iter()
            .all(|at| Vec2::new(at[0], at[1]).length() < 12.5),
        "газон вылез за ось кольца"
    );
    let (layers, _) = mesh_roads(
        &roundabout_with_an_approach(false, false),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "ring_islands").builder.is_empty());
}

/// Замапленная трава острова ложится ещё раз над травой, без канта, — только
/// внутри острова: её кант был бледным кругом внутри газона (Рязань 04, Орёл
/// 02). Парк на острове не кроется (Калуга 05).
#[test]
fn a_roundabout_island_covers_the_rim_of_its_mapped_grass() {
    use crate::settings::{Z_GRASS, Z_SAND};
    let square = |half: f32| {
        vec![
            Vec2::new(-half, -half),
            Vec2::new(half, -half),
            Vec2::new(half, half),
            Vec2::new(-half, half),
        ]
    };
    let mut map = roundabout_with_an_approach(true, true);
    // трава шире острова (ось в 12 м): кроется только его часть
    map.grass.push(PolyArea {
        kind: AreaKind::Grass,
        ..fixture::building(square(20.0), Vec::new())
    });
    map.parks.push(PolyArea {
        kind: AreaKind::Park,
        ..fixture::building(square(5.0), Vec::new())
    });
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let grass = layer(&layers, "ring_island_grass");
    assert!(grass.z > Z_GRASS && grass.z < Z_SAND);
    let positions = grass.builder.positions_for_test();
    assert!(!positions.is_empty(), "трава острова не легла");
    assert!(
        positions
            .iter()
            .all(|at| Vec2::new(at[0], at[1]).length() < 12.5),
        "трава вылезла за остров"
    );
    let (layers, _) = mesh_roads(
        &MapData {
            grass: Vec::new(),
            ..map
        },
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(
        layer(&layers, "ring_island_grass").builder.is_empty(),
        "парк острова перекрашен"
    );
}

#[test]
fn a_two_way_loop_without_the_tag_is_not_a_roundabout() {
    let map = roundabout_with_an_approach(false, false);
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

/// Въезд в кольцо: линия уступи дорогу — на кромке кольца, а не поперёк
/// подхода в полуширине кольца от узла. Подход вписан по касательной, и там
/// ещё середина кольца: линия ложилась через его полосы до бордюра острова
/// (Тула, витрина 04, юг и восток).
#[test]
fn a_ring_entry_yields_on_the_ring_edge() {
    let mut map = roundabout_with_an_approach(true, true);
    // второй подход — въезд: точки к кольцу
    map.roads[2].points.reverse();
    for road in &mut map.roads {
        road.highway = Highway::Tertiary;
    }
    let map = with_network(map.roads);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let lines = junctions.node_paint().marks().stop_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = lines[0];
    assert!(line.yields, "въезд кольцу уступает");
    let ring = drawn.axis(0, Axis::Ribbon);
    let half = drawn.road(0).width / 2.0;
    for end in [line.from, line.to] {
        let off = distance_to_path(end, ring) - half;
        assert!(
            (0.0..0.8).contains(&off),
            "{end:?} от кромки кольца на {off}"
        );
    }
}

/// Кольцо Тулы у R15 точками OSM: шесть дуг primary в 3 и 2 полосы, короткая
/// дуга 1191542887 в 12 м между узлом въезда 595574104 (по касательной) и
/// узлом съезда Радищева. Оба узла — перекрёстки, и дуги клались лентами с
/// прямыми торцами: на кривизне торцы соседних дуг расходились веером, и по
/// внешней кромке оставались клинья-щели.
#[test]
fn a_ring_of_arcs_lays_one_closed_fill_without_seam_slits() {
    let v = |points: &[(f32, f32)]| -> Vec<Vec2> {
        points.iter().map(|&(x, y)| Vec2::new(x, y)).collect()
    };
    let way = |points: Vec<Vec2>, highway: Highway, lanes: u8, ring: bool| RoadLine {
        highway,
        oneway: true,
        roundabout: ring,
        lanes: Some(lanes),
        ..fixture::street(points, f32::from(lanes) * 3.3 + 1.0)
    };
    let arcs = [
        (
            v(&[
                (2563.7, 2427.7),
                (2549.0, 2431.6),
                (2533.6, 2432.1),
                (2521.1, 2427.6),
                (2508.3, 2419.1),
            ]),
            3,
        ),
        (v(&[(2508.3, 2419.1), (2501.9, 2409.3)]), 3),
        (
            v(&[
                (2501.9, 2409.3),
                (2499.9, 2391.4),
                (2502.4, 2377.3),
                (2508.0, 2365.5),
                (2517.9, 2355.7),
                (2525.6, 2350.2),
                (2533.2, 2346.6),
            ]),
            3,
        ),
        (
            v(&[
                (2533.2, 2346.6),
                (2545.0, 2346.1),
                (2555.3, 2348.0),
                (2566.7, 2351.5),
            ]),
            2,
        ),
        (
            v(&[
                (2566.7, 2351.5),
                (2578.0, 2371.6),
                (2581.0, 2383.8),
                (2579.4, 2398.9),
                (2576.5, 2408.5),
                (2569.5, 2423.8),
            ]),
            2,
        ),
        (v(&[(2569.5, 2423.8), (2563.7, 2427.7)]), 3),
    ];
    let mut roads: Vec<RoadLine> = arcs
        .into_iter()
        .map(|(points, lanes)| way(points, Highway::Primary, lanes, true))
        .collect();
    roads.push(way(
        v(&[
            (2528.7, 2463.3),
            (2524.9, 2447.8),
            (2517.4, 2433.2),
            (2508.3, 2419.1),
        ]),
        Highway::Primary,
        1,
        false,
    ));
    roads.push(way(
        v(&[
            (2501.9, 2409.3),
            (2478.0, 2369.8),
            (2464.2, 2355.7),
            (2445.8, 2351.9),
        ]),
        Highway::Residential,
        2,
        false,
    ));
    let map = with_network(roads);
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.rings[0], 1, "шесть дуг — одно кольцо");
    let streets = &layer(&layers, "roads").builder;
    // внешняя кромка кольца у обоих узлов — асфальт сплошь, до трёх
    // десятков сантиметров от кромки широкой дуги
    let centre = Vec2::new(2540.0, 2389.0);
    let half = 10.9 / 2.0;
    for node in [Vec2::new(2508.3, 2419.1), Vec2::new(2501.9, 2409.3)] {
        let outward = (node - centre).normalize();
        for step in -30..=30 {
            let at = node + outward * (half - 0.3) + outward.perp() * (step as f32 * 0.1);
            assert!(
                streets.covers_for_test(at),
                "щель на кромке кольца у {at:?}"
            );
        }
    }
    // одна ширина на всё кольцо — самой широкой дуги (lanes=3), и узкие
    // дуги (lanes=2) залиты ею же, а не своей
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let ring = &prepared.rings().list[0];
    assert!((ring.width - half * 2.0).abs() < 1e-3, "{}", ring.width);
    let path = &ring.path;
    let narrow = Vec2::new(2580.0, 2385.0);
    let at = (1..path.len() - 1)
        .min_by(|&a, &b| {
            path[a]
                .distance(narrow)
                .total_cmp(&path[b].distance(narrow))
        })
        .unwrap();
    let across = (path[at + 1] - path[at - 1]).normalize().perp();
    for side in [-1.0, 1.0] {
        let probe = path[at] + across * side * (half - 0.3);
        assert!(
            streets.covers_for_test(probe),
            "узкая дуга уже кольца у {probe:?}"
        );
    }
}

/// Горло съезда: съезд уходит с кольца по касательной, и от узла до места,
/// где его сечение вышло из асфальта кольца, линия наружной полосы кольца
/// рвётся, а внутренняя идёт насквозь. Пунктир наружной шёл поперёк горла
/// вразнобой со сплошной съезда (Тула, витрина 04, юг).
#[test]
fn a_ring_exit_breaks_the_outer_ring_line_across_its_throat() {
    // съезд из южной вершины (−90°), где кольцо идёт на восток, — наружу
    let map = a_big_ring_with(|circle| {
        vec![
            circle[36],
            polar(42.0, -75.0),
            polar(48.0, -60.0),
            polar(60.0, -48.0),
            polar(90.0, -40.0),
        ]
    });
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let throats = junctions.node_paint().throats(0);
    assert_eq!(throats.len(), 1, "{throats:?}");
    let throat = throats[0];
    // против часовой левая нормаль смотрит внутрь: съезд — справа
    assert_eq!(throat.side, -1.0);
    assert!(throat.gap.reach > 3.0, "{throat:?}");
    // горло — за узлом по ходу кольца, не перед ним
    assert!(throat.gap.at.x > 1.0, "{throat:?}");
    assert!(junctions.node_paint().throats(1).is_empty());

    // краска: в середине горла наружная линия (r + 1.65) погашена,
    // внутренняя (r − 1.65) — нет
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let lanes = &layer(&layers, paint::PAINT_LANES).builder;
    let middle = throat.gap.at.to_angle();
    // середина горла — его доля в полдлины вокруг точки разрыва
    let window = 0.5 * throat.gap.reach / 40.0;
    let at_radius = |radius: f32| -> Vec<f32> {
        lanes
            .ribbon_coords_for_test()
            .unwrap()
            .iter()
            .zip(lanes.positions_for_test())
            .filter(|(_, position)| {
                let point = Vec2::new(position[0], position[1]);
                // вершины полосы краски — по её кромкам, в 0.6 м от линии
                (point.length() - radius).abs() < 0.7 && (point.to_angle() - middle).abs() < window
            })
            .map(|(coords, _)| coords[2])
            .collect()
    };
    let lane = 3.3 / 2.0;
    let outer = at_radius(40.0 + lane);
    let inner = at_radius(40.0 - lane);
    assert!(
        !outer.is_empty() && !inner.is_empty(),
        "{outer:?} {inner:?}"
    );
    assert!(outer.iter().all(|&to_break| to_break < 0.0), "{outer:?}");
    assert!(inner.iter().all(|&to_break| to_break > 0.0), "{inner:?}");
}

/// Въезд, что идёт по асфальту кольца дольше [`node_paint`]-ского поиска
/// кромки плеча (25 м): линия уступи дорогу на кромке кольца есть, и линии
/// полос въезда рвутся, пока он из асфальта кольца не вышел, — а не тянутся по
/// кольцу к его оси (Тула, витрина 04, восток).
#[test]
fn a_long_tangential_ring_entry_yields_and_keeps_its_lines_off_the_ring() {
    // въезд с юго-востока вдоль кольца в восточную вершину (0°), где кольцо
    // идёт на север
    let map = a_big_ring_with(|circle| {
        vec![
            polar(80.0, -90.0),
            polar(60.0, -75.0),
            polar(50.0, -55.0),
            polar(46.5, -35.0),
            polar(44.0, -18.0),
            circle[0],
        ]
    });
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let lines = junctions.node_paint().marks().stop_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].yields, "въезд кольцу уступает");
    // каждая точка оси въезда в асфальте кольца — в разрыве его линий
    let ring = drawn.axis(0, Axis::Ribbon);
    let half = drawn.road(0).width / 2.0;
    let entry = drawn.axis(1, Axis::Ribbon);
    let cut = junctions.paint().of(1).cut;
    let along = |point: Vec2| crate::map::along::nearest_on_path(entry, point).unwrap().1;
    let (arclengths, total) = crate::map::along::arclengths(entry);
    let mut inside = 0;
    for step in 0..(total * 2.0) as usize {
        let at = step as f32 * 0.5;
        let Some((point, _)) = crate::map::along::place_on_path(entry, &arclengths, at) else {
            continue;
        };
        if distance_to_path(point, ring) > half - 0.5 {
            continue;
        }
        inside += 1;
        assert!(
            cut.iter()
                .any(|found| (along(found.at) - at).abs() <= found.reach + 0.1),
            "{point:?} ({at} of {total} m) в асфальте кольца, а линии не рвутся: {cut:?}"
        );
    }
    assert!(inside > 3, "въезд идёт по кольцу: {inside}");
}

/// Островок по правилу (`gores::splitters`) рвёт подход дважды — краску и
/// колею асфальта: его разрыв кладётся и в разрывы краски (`PaintBreaks`), и
/// в разрывы асфальта (`AsphaltBreaks`) — `NodePaint::add_splitter`.
#[test]
fn a_splitter_gap_reaches_both_asphalt_and_paint() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 25.0)
        .collect();
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads
        .push(fixture::street(vec![circle[0], Vec2::new(90.0, 0.0)], 7.6));
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.gores, 1);
    let island = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test();
    let low = island.iter().map(|at| at[0]).fold(f32::INFINITY, f32::min);
    let high = island
        .iter()
        .map(|at| at[0])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(low > 28.0 && high < 50.0, "{low}..{high}");
    // внутри островка, но дальше разрыва кольца (его полуширина + 1 м)
    let inside = |at: &[f32; 3]| at[0] > 31.0 && at[0] < high - 1.0 && at[1].abs() < 3.9;
    for name in ["roads", paint::PAINT_AXES] {
        let found = to_break_where(&layers, name, inside);
        assert!(!found.is_empty(), "{name}");
        assert!(
            found.iter().all(|&to_break| to_break < 0.0),
            "{name}: {found:?}"
        );
        let beyond = to_break_where(&layers, name, |at| at[0] > high + 5.0 && at[1].abs() < 3.9);
        assert!(
            beyond.iter().any(|&to_break| to_break > 0.0),
            "{name} за островком"
        );
    }
}
