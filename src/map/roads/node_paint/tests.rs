use super::*;
use crate::map::osm::RoadNode;
use crate::map::osm::fixture::street;
use crate::map::roads::junctions::marking_breaks;
use crate::map::roads::network::RoadNetwork;

const NODE: Vec2 = Vec2::new(100.0, 0.0);

const EVERYTHING: NodePaintStyle = NodePaintStyle {
    crossings: CrossingMode::Generated,
    stop_lines: true,
};

fn road(points: Vec<Vec2>, width: f32, highway: Highway, lanes: u8) -> RoadLine {
    RoadLine {
        highway,
        lanes: Some(lanes),
        ..street(points, width)
    }
}

/// Сквозная улица по оси x через [`NODE`].
fn through(highway: Highway) -> RoadLine {
    road(
        vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
        8.0,
        highway,
        2,
    )
}

/// Жилая, примыкающая к [`NODE`] снизу.
fn side() -> RoadLine {
    road(
        vec![Vec2::new(100.0, -80.0), NODE],
        8.0,
        Highway::Residential,
        2,
    )
}

fn paint_of(roads: Vec<RoadLine>, marks: Vec<RoadNode>, style: NodePaintStyle) -> NodePaint {
    let mut map = MapData {
        roads,
        road_nodes: marks,
        ..default()
    };
    map.network = RoadNetwork::new(&map.roads);
    let drawn: Vec<&RoadLine> = map.roads.iter().collect();
    let paths: Vec<Vec<Vec2>> = map.roads.iter().map(|road| road.points.clone()).collect();
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    NodePaint::new(
        &drawn,
        &paths,
        &base,
        &[],
        &map,
        style,
        |_| true,
        |_| Vec::new(),
    )
}

/// Разрывы дороги, что не тупики.
fn gaps(paint: &NodePaint, road: usize) -> Vec<Break> {
    paint.breaks[road]
        .iter()
        .copied()
        .filter(|found| found.reach > 0.0)
        .collect()
}

#[test]
fn a_minor_street_does_not_break_the_main_one() {
    let paint = paint_of(
        vec![through(Highway::Tertiary), side()],
        Vec::new(),
        EVERYTHING,
    );
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.breaks[0]);
    assert!(!gaps(&paint, 1).is_empty());
    assert_eq!(paint.through, 1);
    // зебра и стоп-линия — только поперёк примыкания
    assert_eq!(paint.zebras.len(), 1);
    assert_eq!(paint.stop_lines.len(), 1);
    let zebra = paint.zebras[0];
    assert!(
        (zebra.from.y - zebra.to.y).abs() < 1e-3,
        "зебра поперёк примыкания: {zebra:?}"
    );
    assert!(
        zebra.from.y < -4.0 - ZEBRA_SETBACK,
        "за кромкой узла: {zebra:?}"
    );
}

/// Перемычка между двумя узлами короче [`RULE_ZEBRA_ROOM`] за кромкой (ветка
/// развилки, Тула, витрина 06) зебр по правилу не получает ни с одного
/// конца: они теснились с обоих концов на двадцати метрах. Та же улица
/// длиннее — получает обе.
#[test]
fn a_short_link_between_two_nodes_gets_no_rule_zebras() {
    let link = |length: f32| {
        let far = road(
            vec![
                Vec2::new(0.0, length),
                NODE + Vec2::new(0.0, length),
                Vec2::new(200.0, length),
            ],
            8.0,
            Highway::Tertiary,
            2,
        );
        let between = road(
            vec![NODE, NODE + Vec2::new(0.0, length)],
            8.0,
            Highway::Residential,
            2,
        );
        paint_of(
            vec![through(Highway::Tertiary), far, between],
            Vec::new(),
            EVERYTHING,
        )
    };
    let short = link(26.0);
    assert!(short.zebras.is_empty(), "{:?}", short.zebras);
    assert_eq!(short.stop_lines.len(), 2, "стоп-линии остаются");
    assert_eq!(link(60.0).zebras.len(), 2);
}

#[test]
fn an_equal_side_street_does_not_break_the_through_one_either() {
    let paint = paint_of(
        vec![through(Highway::Residential), side()],
        Vec::new(),
        EVERYTHING,
    );
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.breaks[0]);
    assert!(!gaps(&paint, 1).is_empty());
}

#[test]
fn an_equal_crossing_breaks_both_and_paints_every_arm() {
    let across = road(
        vec![Vec2::new(100.0, -100.0), NODE, Vec2::new(100.0, 100.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(
        vec![through(Highway::Residential), across],
        Vec::new(),
        EVERYTHING,
    );
    assert!(!gaps(&paint, 0).is_empty());
    assert!(!gaps(&paint, 1).is_empty());
    assert_eq!(paint.zebras.len(), 4);
    assert_eq!(paint.stop_lines.len(), 4);
}

#[test]
fn an_equal_crossing_keeps_the_asphalt_breaks_of_both() {
    let across = road(
        vec![Vec2::new(100.0, -100.0), NODE, Vec2::new(100.0, 100.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(
        vec![through(Highway::Residential), across],
        Vec::new(),
        EVERYTHING,
    );
    assert!(paint.junctions[0].leading.is_empty());
    for road in 0..2 {
        assert!(
            paint.asphalt[road]
                .iter()
                .any(|found| found.at == NODE && found.reach > 0.0)
        );
    }
    assert_eq!(paint.junctions[0].arms.len(), 4);
}

/// Примыкание, которое OSM не довёл до улицы, а сеть дотянула стежком, —
/// такой же узел: зебра и разрыв на нём, улица цела.
#[test]
fn a_stitched_side_street_is_an_arm_of_the_junction() {
    let mut map = MapData {
        roads: vec![
            through(Highway::Tertiary),
            road(
                vec![Vec2::new(90.0, -80.0), Vec2::new(90.0, -6.0)],
                8.0,
                Highway::Residential,
                2,
            ),
        ],
        ..default()
    };
    map.network = RoadNetwork::new(&map.roads);
    let at = Vec2::new(90.0, 0.0);
    let targets = [
        [None, None],
        [
            None,
            Some(StitchTarget {
                road: 0,
                segment: 0,
                at,
            }),
        ],
    ];
    let drawn: Vec<&RoadLine> = map.roads.iter().collect();
    let mut paths: Vec<Vec<Vec2>> = map.roads.iter().map(|road| road.points.clone()).collect();
    // стежок — до оси улицы
    paths[1].push(at);
    let base = marking_breaks(&map.roads, is_carriageway, &targets).breaks;
    let paint = NodePaint::new(
        &drawn,
        &paths,
        &base,
        &targets,
        &map,
        EVERYTHING,
        |_| true,
        |_| Vec::new(),
    );
    assert_eq!(paint.junctions.len(), 1);
    assert_eq!(paint.junctions[0].leading, vec![0]);
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.breaks[0]);
    assert!(!gaps(&paint, 1).is_empty());
    assert_eq!(paint.zebras.len(), 1, "зебра поперёк примыкания");
    assert_eq!(paint.stop_lines.len(), 1);
}

#[test]
fn signals_break_the_main_street_too() {
    let paint = paint_of(
        vec![through(Highway::Tertiary), side()],
        vec![RoadNode {
            pos: NODE,
            kind: RoadNodeKind::TrafficSignals,
        }],
        EVERYTHING,
    );
    assert!(!gaps(&paint, 0).is_empty());
    assert_eq!(
        paint.stop_lines.len(),
        3,
        "каждое плечо, на встречных полосах"
    );
}

#[test]
fn close_side_streets_from_both_sides_are_one_junction() {
    // улица Циолковского: два примыкания с разных сторон в 17 м
    let (west, east) = (Vec2::new(90.0, 0.0), Vec2::new(107.0, 0.0));
    let main = road(
        vec![Vec2::ZERO, west, east, Vec2::new(200.0, 0.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let north = road(
        vec![west, Vec2::new(90.0, 80.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let south = road(
        vec![east, Vec2::new(107.0, -80.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(vec![main, north, south], Vec::new(), EVERYTHING);
    assert_eq!(paint.clusters, 1);
    assert!(
        gaps(&paint, 0).is_empty(),
        "главная проходит кластер целиком: {:?}",
        paint.breaks[0]
    );
    assert_eq!(paint.zebras.len(), 2);
}

#[test]
fn a_crossing_street_through_close_nodes_breaks_once_without_an_orphan_dash() {
    let (west, east) = (Vec2::new(90.0, 0.0), Vec2::new(107.0, 0.0));
    let main = road(
        vec![Vec2::ZERO, west, east, Vec2::new(200.0, 0.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let north = road(
        vec![Vec2::new(90.0, 80.0), west, Vec2::new(90.0, -80.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let south = road(
        vec![east, Vec2::new(107.0, -80.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(vec![main, north, south], Vec::new(), EVERYTHING);
    // между узлами штриха нет: разрывы у 90 и у 107 слиты одним
    let covered = |x: f32| {
        gaps(&paint, 0)
            .iter()
            .any(|found| (found.at.x - x).abs() <= found.reach + 1e-3)
    };
    for x in [90.0, 95.0, 98.5, 102.0, 107.0] {
        assert!(covered(x), "x = {x}: {:?}", paint.breaks[0]);
    }
}

#[test]
fn an_osm_crossing_mid_block_is_a_zebra_with_a_gap() {
    let at = Vec2::new(50.0, 0.0);
    let road = road(
        vec![Vec2::ZERO, at, Vec2::new(100.0, 0.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(
        vec![road],
        vec![RoadNode {
            pos: at,
            kind: RoadNodeKind::Crossing {
                signals: true,
                island: false,
                marked: true,
            },
        }],
        EVERYTHING,
    );
    assert_eq!(paint.zebras.len(), 1);
    assert!(paint.zebras[0].osm);
    assert_eq!(paint.stop_lines.len(), 2, "регулируемый: с обеих сторон");
    let gap = gaps(&paint, 0);
    assert_eq!(gap.len(), 1);
    assert!(gap[0].at.distance(at) < 1e-3);
    assert!(gap[0].reach > ZEBRA_LENGTH / 2.0 + STOP_GAP);
}

#[test]
fn an_unmarked_crossing_or_crossings_off_paint_no_zebra() {
    let at = Vec2::new(50.0, 0.0);
    let road = road(
        vec![Vec2::ZERO, at, Vec2::new(100.0, 0.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let crossing = |marked| RoadNode {
        pos: at,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked,
        },
    };
    let unmarked = paint_of(vec![road.clone()], vec![crossing(false)], EVERYTHING);
    assert!(unmarked.zebras.is_empty());
    let off = paint_of(
        vec![road],
        vec![crossing(true)],
        NodePaintStyle {
            crossings: CrossingMode::Off,
            ..EVERYTHING
        },
    );
    assert!(off.zebras.is_empty());
}

#[test]
fn an_osm_crossing_on_the_arm_replaces_the_generated_zebra() {
    let at = Vec2::new(100.0, -15.0);
    let side = road(
        vec![Vec2::new(100.0, -80.0), at, NODE],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(
        vec![through(Highway::Tertiary), side],
        vec![RoadNode {
            pos: at,
            kind: RoadNodeKind::Crossing {
                signals: false,
                island: false,
                marked: true,
            },
        }],
        EVERYTHING,
    );
    assert_eq!(paint.zebras.len(), 1);
    assert!(paint.zebras[0].osm);
    assert!((paint.zebras[0].from.y - at.y).abs() < 1e-3);
    // стоп-линия — за зеброй, дальше от узла
    assert!(paint.stop_lines[0].from.y < at.y - ZEBRA_LENGTH / 2.0);
}

#[test]
fn a_one_way_arm_leaving_the_node_has_no_stop_line() {
    let away = RoadLine {
        oneway: true,
        ..road(
            vec![NODE, Vec2::new(100.0, -80.0)],
            8.0,
            Highway::Residential,
            2,
        )
    };
    let paint = paint_of(
        vec![through(Highway::Tertiary), away],
        Vec::new(),
        EVERYTHING,
    );
    assert_eq!(paint.zebras.len(), 1);
    assert!(paint.stop_lines.is_empty());
}

#[test]
fn a_give_way_sign_makes_the_stop_line_dashed() {
    let paint = paint_of(
        vec![through(Highway::Residential), side()],
        vec![RoadNode {
            pos: Vec2::new(100.0, -80.0),
            kind: RoadNodeKind::GiveWay,
        }],
        EVERYTHING,
    );
    // знак в 80 м — дальше, чем его ищут: линия сплошная
    assert!(!paint.stop_lines[0].yields);
    let near = Vec2::new(100.0, -20.0);
    let side = road(
        vec![Vec2::new(100.0, -80.0), near, NODE],
        8.0,
        Highway::Residential,
        2,
    );
    let paint = paint_of(
        vec![through(Highway::Residential), side],
        vec![RoadNode {
            pos: near,
            kind: RoadNodeKind::GiveWay,
        }],
        EVERYTHING,
    );
    assert!(paint.stop_lines[0].yields);
}

#[test]
fn stop_lines_span_the_incoming_half_on_the_traffic_side() {
    let paint = paint_of(
        vec![through(Highway::Tertiary), side()],
        Vec::new(),
        EVERYTHING,
    );
    // жилая идёт снизу к узлу, правостороннее движение: встречные узлу
    // полосы — справа по ходу, то есть с восточной стороны
    let line = paint.stop_lines[0];
    assert!(line.from.x > NODE.x && line.to.x > NODE.x, "{line:?}");
    assert!((line.to.x - NODE.x - (4.0 - EDGE_INSET)).abs() < 1e-3);
}

#[test]
fn crossed_zebras_keep_one_and_side_by_side_ones_both() {
    let zebra = |from: Vec2, to: Vec2, osm| Zebra { from, to, osm };
    let first = zebra(Vec2::new(-4.0, 0.0), Vec2::new(4.0, 0.0), false);
    let crossed = zebra(Vec2::new(0.0, -4.0), Vec2::new(0.0, 4.0), true);
    // вторая половина разделённой улицы — продолжение отрезка, бок о бок
    let beside = zebra(Vec2::new(5.0, 0.0), Vec2::new(12.0, 0.0), false);
    let kept = without_overlaps(vec![first, crossed, beside]);
    assert_eq!(kept, vec![crossed, beside], "по данным остаётся");
}

#[test]
fn a_wider_through_street_leaves_its_extra_lanes_in_a_pocket() {
    let wide = road(vec![Vec2::ZERO, NODE], 14.2, Highway::Tertiary, 4);
    let narrow = road(vec![NODE, Vec2::new(200.0, 0.0)], 7.6, Highway::Tertiary, 2);
    let paint = paint_of(vec![wide, narrow, side()], Vec::new(), EVERYTHING);
    assert!(gaps(&paint, 0).is_empty(), "главная не рвётся");
    let pocket = paint.pockets[0][1].expect("карман у конца широкой");
    assert_eq!(pocket.lanes, 2);
    assert_eq!(pocket.gap.at, NODE);
    assert!(paint.pockets[1] == [None; 2]);
}
