use super::*;
use crate::map::osm::fixture::street;
use crate::map::osm::{RoadNode, SidewalkSide};
use crate::map::roads::junctions::marking_breaks;
use crate::map::roads::network::RoadNetwork;
use crate::map::roads::network::pairs::PairRun;

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
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    NodePaint::for_test(&Drawn::for_test(&map), &base, &map, &[], style)
}

/// Разрывы дороги, что не тупики.
fn gaps(paint: &NodePaint, road: usize) -> Vec<Break> {
    paint
        .lines()
        .of(road)
        .cut
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
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.lines().of(0).cut);
    assert!(!gaps(&paint, 1).is_empty());
    assert_eq!(paint.through, 1);
    // осевая главной у примыкания сплошная: узел насквозь — в её `solid`
    assert!(
        paint
            .lines()
            .of(0)
            .solid
            .iter()
            .any(|found| found.at == NODE && found.reach > 0.0),
        "{:?}",
        paint.lines().of(0).solid
    );
    assert!(paint.lines().of(1).solid.is_empty());
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

/// Две жилые улицы зебру по правилу не получают — ни тротуары, ни стоп-линия
/// её не зовут; светофор в узле — получают. Та же жилая у `tertiary` —
/// получает (`a_minor_street_does_not_break_the_main_one`).
#[test]
fn two_residential_streets_get_no_rule_zebra_unless_signalized() {
    let quiet = paint_of(
        vec![through(Highway::Residential), side()],
        Vec::new(),
        EVERYTHING,
    );
    assert!(quiet.zebras.is_empty(), "{:?}", quiet.zebras);
    assert!(quiet.stop_lines.is_empty(), "и стоп-линий без знака тоже");
    let stop = RoadNode {
        pos: NODE + Vec2::new(0.0, -6.0),
        kind: RoadNodeKind::Stop,
    };
    // знак стоит на вершине примыкания — так его и ищет `sign`
    let signed_side = road(
        vec![Vec2::new(100.0, -80.0), stop.pos, NODE],
        8.0,
        Highway::Residential,
        2,
    );
    let signed = paint_of(
        vec![through(Highway::Residential), signed_side],
        vec![stop],
        EVERYTHING,
    );
    assert!(signed.zebras.is_empty());
    assert_eq!(signed.stop_lines.len(), 1, "знак «Стоп» — стоп-линия есть");
    let signals = RoadNode {
        pos: NODE,
        kind: RoadNodeKind::TrafficSignals,
    };
    let lit = paint_of(
        vec![through(Highway::Residential), side()],
        vec![signals],
        EVERYTHING,
    );
    assert!(!lit.zebras.is_empty());
}

/// Узел, где дорога — дуга кольца, зебры по правилу не получает ни на одном
/// луче, какой бы ни был класс: переходы у кольца приходят нодами OSM.
#[test]
fn a_ring_node_gets_no_rule_zebras() {
    let roads = vec![through(Highway::Tertiary), side()];
    let mut map = MapData { roads, ..default() };
    map.network = RoadNetwork::new(&map.roads);
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let paint = |drawn: &Drawn| NodePaint::for_test(drawn, &base, &map, &[], EVERYTHING);
    assert_eq!(paint(&Drawn::for_test(&map)).zebras.len(), 1);
    assert!(paint(&Drawn::for_test(&map).with_ring(0)).zebras.is_empty());
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

/// Грунтовое примыкание к `tertiary` — без краски: ни зебры по правилу, ни
/// стоп-линии по рангу узла или по знаку, ни зебры на переходе OSM (Калуга,
/// витрина 06). Асфальтовое — с ними
/// (`a_minor_street_does_not_break_the_main_one`).
#[test]
fn an_unpaved_arm_gets_no_paint() {
    let at = Vec2::new(100.0, -40.0);
    let give_way = RoadNode {
        pos: Vec2::new(100.0, -6.0),
        kind: RoadNodeKind::GiveWay,
    };
    let side = RoadLine {
        pavement: Some(crate::map::osm::model::Pavement::Unpaved),
        ..road(
            vec![Vec2::new(100.0, -80.0), at, give_way.pos, NODE],
            8.0,
            Highway::Residential,
            2,
        )
    };
    let crossing = RoadNode {
        pos: at,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    };
    let paint = paint_of(
        vec![through(Highway::Tertiary), side],
        vec![crossing, give_way],
        EVERYTHING,
    );
    assert!(paint.zebras.is_empty(), "{:?}", paint.zebras);
    assert!(paint.stop_lines.is_empty());
}

/// Размеченный переход OSM на одном плече улицы снимает зебру по правилу с
/// другого её плеча: узел уже переходят по данным, и вторая зебра в двух
/// десятках метров от первой — лишняя (Тула, витрина 12). Без перехода оба
/// плеча получают свою.
#[test]
fn an_osm_crossing_of_the_street_drops_the_rule_zebra_on_its_other_arm() {
    let at = Vec2::new(100.0, -20.0);
    let across = road(
        vec![Vec2::new(100.0, -80.0), at, NODE, Vec2::new(100.0, 80.0)],
        8.0,
        Highway::Residential,
        2,
    );
    let bare = paint_of(
        vec![through(Highway::Tertiary), across.clone()],
        Vec::new(),
        EVERYTHING,
    );
    assert_eq!(bare.zebras.len(), 2, "{:?}", bare.zebras);
    let crossing = RoadNode {
        pos: at,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    };
    let mapped = paint_of(
        vec![through(Highway::Tertiary), across],
        vec![crossing],
        EVERYTHING,
    );
    assert_eq!(mapped.zebras.len(), 1, "{:?}", mapped.zebras);
    let zebra = mapped.zebras[0];
    assert!(
        (zebra.from.y + 20.0).abs() < 1.0,
        "зебра — по данным: {zebra:?}"
    );
}

#[test]
fn an_equal_side_street_does_not_break_the_through_one_either() {
    let paint = paint_of(
        vec![through(Highway::Residential), side()],
        Vec::new(),
        EVERYTHING,
    );
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.lines().of(0).cut);
    assert!(!gaps(&paint, 1).is_empty());
}

#[test]
fn an_equal_crossing_breaks_both_and_paints_every_arm() {
    let across = road(
        vec![Vec2::new(100.0, -100.0), NODE, Vec2::new(100.0, 100.0)],
        8.0,
        Highway::Tertiary,
        2,
    );
    let paint = paint_of(
        vec![through(Highway::Tertiary), across],
        Vec::new(),
        EVERYTHING,
    );
    assert!(!gaps(&paint, 0).is_empty());
    assert!(!gaps(&paint, 1).is_empty());
    assert_eq!(paint.zebras.len(), 4);
    assert_eq!(paint.stop_lines.len(), 4);
}

/// Крестовина `tertiary` рвёт и старшую primary: через поле перекрёстка
/// линий полос нет ни у одной из дорог (Орёл, витрина 05). Жилая крестовина
/// и примыкание `tertiary` главную не рвут.
#[test]
fn a_tertiary_crossing_breaks_the_primary_too_but_a_residential_one_does_not() {
    let across = |highway| {
        road(
            vec![Vec2::new(100.0, -100.0), NODE, Vec2::new(100.0, 100.0)],
            8.0,
            highway,
            2,
        )
    };
    let crossed = paint_of(
        vec![through(Highway::Primary), across(Highway::Tertiary)],
        Vec::new(),
        EVERYTHING,
    );
    assert!(
        !gaps(&crossed, 0).is_empty(),
        "{:?}",
        crossed.lines().of(0).solid
    );
    assert!(!gaps(&crossed, 1).is_empty());
    assert!(crossed.junctions[0].leading.is_empty());

    let quiet = paint_of(
        vec![through(Highway::Primary), across(Highway::Residential)],
        Vec::new(),
        EVERYTHING,
    );
    assert!(gaps(&quiet, 0).is_empty(), "{:?}", quiet.lines().of(0).cut);
    assert_eq!(quiet.junctions[0].leading, vec![0]);

    let side = road(
        vec![Vec2::new(100.0, -80.0), NODE],
        8.0,
        Highway::Tertiary,
        2,
    );
    let tee = paint_of(
        vec![through(Highway::Primary), side],
        Vec::new(),
        EVERYTHING,
    );
    assert!(gaps(&tee, 0).is_empty(), "{:?}", tee.lines().of(0).cut);
}

/// Плечо без зебры и стоп-линии — односторонняя уходит из узла — рвётся до
/// кромки, где его сечение вышло из чужого асфальта, а не на полуширине
/// соседа: на пологой крестовине (23°, Орёл, витрина 05) чужая полоса
/// тянется вдоль плеча на десяток метров, и линии шли по полю перекрёстка.
#[test]
fn a_bare_arm_of_a_shallow_crossing_breaks_up_to_its_edge() {
    let mut main = road(
        vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
        7.6,
        Highway::Primary,
        2,
    );
    main.oneway = true;
    let slope = Vec2::from_angle(23f32.to_radians());
    let mut across = road(
        vec![NODE - slope * 80.0, NODE, NODE + slope * 80.0],
        4.3,
        Highway::Secondary,
        1,
    );
    across.oneway = true;
    let paint = paint_of(
        vec![main, across],
        Vec::new(),
        NodePaintStyle {
            crossings: CrossingMode::Off,
            stop_lines: true,
        },
    );
    // за узлом по ходу — ни зебры, ни стоп-линии: разрыв до кромки
    let beyond = gaps(&paint, 0)
        .iter()
        .map(|found| found.at.x + found.reach - NODE.x)
        .fold(f32::MIN, f32::max);
    // бок сечения (полуширина без отступа) выходит из соседа за
    // (2.15 + 3.5·cos 23°) / sin 23° ≈ 13.8 м
    assert!(beyond > 13.0, "{beyond} {:?}", paint.lines().of(0).cut);
}

/// Ведущая узла теряет разрыв асфальта (колея сквозь), но в базе он
/// остаётся: база не переписывается, по ней открываются разделительные.
#[test]
fn a_leading_road_loses_its_asphalt_break_but_not_its_base_one() {
    let roads = vec![through(Highway::Primary), side()];
    let map = MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    };
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let paint = NodePaint::for_test(&Drawn::for_test(&map), &base, &map, &[], EVERYTHING);
    assert_eq!(paint.junctions[0].leading, vec![0]);
    let at_node = |breaks: &[Break]| {
        breaks
            .iter()
            .any(|found| found.at == NODE && found.reach > 0.0)
    };
    assert!(at_node(&base[0]));
    assert!(
        !at_node(paint.asphalt().of(0)),
        "{:?}",
        paint.asphalt().of(0)
    );
    assert!(at_node(paint.asphalt().of(1)));
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
            paint
                .asphalt()
                .of(road)
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
    let drawn = Drawn::for_test(&map);
    // стежок — до оси улицы
    assert_eq!(drawn.stitched_end(1), [false, true]);
    let at = Vec2::new(90.0, 0.0);
    let ribbon = drawn.axis(1, Axis::Ribbon);
    assert!(ribbon[ribbon.len() - 1].distance(at) < 1e-3, "{ribbon:?}");
    let base = marking_breaks(&map.roads, is_carriageway, &drawn.stitches().targets).breaks;
    let paint = NodePaint::for_test(&drawn, &base, &map, &[], EVERYTHING);
    assert_eq!(paint.junctions.len(), 1);
    assert_eq!(paint.junctions[0].leading, vec![0]);
    assert!(gaps(&paint, 0).is_empty(), "{:?}", paint.lines().of(0).cut);
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

/// Т со светофором, где у главной тротуаров нет по тегу (Ложевая, Тула,
/// витрина 08): улиц с тротуарами в узле одна, и по правилу «двух улиц» зебр
/// не было вовсе. На регулируемом узле зебра встаёт поперёк плеча с
/// тротуарами по обе стороны; без светофора — по-прежнему нет.
#[test]
fn a_signalled_t_crosses_the_walked_arm_even_without_a_second_walked_street() {
    let paint = |marks: Vec<RoadNode>| {
        let mut map = MapData {
            roads: vec![
                RoadLine {
                    sidewalks: [SidewalkSide::None; 2],
                    ..through(Highway::Tertiary)
                },
                side(),
            ],
            road_nodes: marks,
            ..default()
        };
        map.network = RoadNetwork::new(&map.roads);
        let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
        NodePaint::for_test(&Drawn::for_test(&map), &base, &map, &[], EVERYTHING)
    };
    let signalled = paint(vec![RoadNode {
        pos: NODE,
        kind: RoadNodeKind::TrafficSignals,
    }]);
    assert_eq!(signalled.zebras.len(), 1, "{:?}", signalled.zebras);
    let zebra = signalled.zebras[0];
    assert!(
        (zebra.from.y - zebra.to.y).abs() < 1e-3 && zebra.from.y < 0.0,
        "поперёк примыкания: {zebra:?}"
    );
    assert!(paint(Vec::new()).zebras.is_empty());
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
        paint.lines().of(0).cut
    );
    // три жилые — зебр по правилу нет (у Яндекса на Циолковского ни одной)
    assert!(paint.zebras.is_empty(), "{:?}", paint.zebras);
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
        assert!(covered(x), "x = {x}: {:?}", paint.lines().of(0).cut);
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
        vec![through(Highway::Tertiary), side()],
        vec![RoadNode {
            pos: Vec2::new(100.0, -80.0),
            kind: RoadNodeKind::GiveWay,
        }],
        EVERYTHING,
    );
    // знак в 80 м — дальше, чем его ищут: линия сплошная (стоит она у
    // `tertiary`; у двух жилых без знака её не было бы вовсе)
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

/// Разделённая жилая поперёк третичной: по переходу OSM на каждой половине, и
/// маппер поставил их на метр с лишним вразнобой (Тула, витрина 02: 0.9 и
/// 1.2 м). Обе зебры встают на одну линию посередине между узлами — не
/// ступенькой, какой лежали, пока пара «OSM + OSM» не выравнивалась вовсе.
/// Между половинами газон — зебр две, каждая до своего бордюра.
#[test]
fn two_osm_zebras_of_a_divided_street_meet_halfway() {
    let paint = divided_street_crossing(false);
    let north: Vec<&Zebra> = paint
        .zebras
        .iter()
        .filter(|zebra| zebra.osm && zebra.from.y > 0.0)
        .collect();
    assert_eq!(north.len(), 2, "{:?}", paint.zebras);
    for zebra in north {
        assert!((zebra.from.y - 12.6).abs() < 1e-3, "{zebra:?}");
    }
}

/// Та же пара по асфальтовой разделительной — одна планка от кромки до
/// кромки через обе половины, как у Яндекса на 02: у двух полосы сбивались на
/// шве.
#[test]
fn a_paved_divided_street_is_crossed_by_one_zebra() {
    let paint = divided_street_crossing(true);
    let north: Vec<&Zebra> = paint
        .zebras
        .iter()
        .filter(|zebra| zebra.osm && zebra.from.y > 0.0)
        .collect();
    assert_eq!(north.len(), 1, "{:?}", paint.zebras);
    let zebra = north[0];
    assert!((zebra.from.y - 12.6).abs() < 1e-3, "{zebra:?}");
    let [low, high] = [zebra.from.x.min(zebra.to.x), zebra.from.x.max(zebra.to.x)];
    assert!((low - (94.0 - 3.8 + EDGE_INSET)).abs() < 1e-3, "{zebra:?}");
    assert!(
        (high - (106.0 + 3.8 - EDGE_INSET)).abs() < 1e-3,
        "{zebra:?}"
    );
}

/// Трамвайное полотно шириной почти в [`TRAM_BED_MAX_GAP`] между кромками —
/// тоже асфальт (`PairRun::paved`), и через него тоже одна планка: зазор
/// между планками — полотно и два отступа от кромок.
#[test]
fn the_widest_tram_bed_is_crossed_by_one_zebra() {
    let gap = TRAM_BED_MAX_GAP - 0.2;
    let apart = 7.6 + gap;
    let paint = divided_street_crossing_apart(true, apart);
    let north: Vec<&Zebra> = paint
        .zebras
        .iter()
        .filter(|zebra| zebra.osm && zebra.from.y > 0.0)
        .collect();
    assert_eq!(north.len(), 1, "{:?}", paint.zebras);
    let zebra = north[0];
    let [low, high] = [zebra.from.x.min(zebra.to.x), zebra.from.x.max(zebra.to.x)];
    let edge = apart / 2.0 + 3.8 - EDGE_INSET;
    assert!((low - (100.0 - edge)).abs() < 1e-3, "{zebra:?}");
    assert!((high - (100.0 + edge)).abs() < 1e-3, "{zebra:?}");
}

/// Разделённая жилая (половины на x 94 и 106, по переходу OSM на каждой)
/// поперёк третичной; `paved` — асфальт ли между половинами.
fn divided_street_crossing(paved: bool) -> NodePaint {
    divided_street_crossing_apart(paved, 12.0)
}

/// [`divided_street_crossing`] с осями половин в `apart` метрах друг от
/// друга, симметрично вокруг x 100.
fn divided_street_crossing_apart(paved: bool, apart: f32) -> NodePaint {
    let [west, east] = [100.0 - apart / 2.0, 100.0 + apart / 2.0];
    let half = |x: f32, down: bool, crossing: f32| {
        let mut points = vec![
            Vec2::new(x, 80.0),
            Vec2::new(x, crossing),
            Vec2::new(x, 0.0),
            Vec2::new(x, -80.0),
        ];
        if !down {
            points.reverse();
        }
        RoadLine {
            oneway: true,
            ..road(points, 7.6, Highway::Residential, 2)
        }
    };
    let crossing = |pos: Vec2| RoadNode {
        pos,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    };
    let mut map = MapData {
        roads: vec![
            road(
                vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)],
                8.0,
                Highway::Tertiary,
                2,
            ),
            half(west, true, 12.0),
            half(east, false, 13.2),
        ],
        road_nodes: vec![
            crossing(Vec2::new(west, 12.0)),
            crossing(Vec2::new(east, 13.2)),
        ],
        ..default()
    };
    map.roads[0]
        .points
        .splice(1..1, [Vec2::new(west, 0.0), Vec2::new(east, 0.0)]);
    map.network = RoadNetwork::new(&map.roads);
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    // половины — пара на всю длину, мощёная или с газоном, как скажет тест
    let run = |partner: usize| {
        vec![PairRun::for_test(
            0.0,
            160.0,
            partner,
            true,
            apart - 7.6,
            paved,
        )]
    };
    let drawn = Drawn::for_test(&map)
        .with_pairs(1, run(2))
        .with_pairs(2, run(1));
    NodePaint::for_test(&drawn, &base, &map, &[], EVERYTHING)
}

/// Связка вливается в улицу под острым углом: у точки узла, откуда меряется
/// кромка (полуширина соседа и метр), под связкой ещё асфальт улицы.
/// Стоп-линия там легла бы обрывком посреди перекрёстка (витрина 08) — её нет;
/// у того же примыкания под прямым углом она есть.
#[test]
fn no_stop_line_inside_the_asphalt_of_another_road() {
    let join = |from: Vec2| RoadLine {
        oneway: true,
        ..road(vec![from, NODE], 4.3, Highway::Residential, 1)
    };
    let main = || {
        road(
            vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
            14.0,
            Highway::Primary,
            4,
        )
    };
    let square = paint_of(
        vec![main(), join(Vec2::new(100.0, -80.0))],
        Vec::new(),
        EVERYTHING,
    );
    assert_eq!(square.stop_lines.len(), 1);
    let shallow = paint_of(
        vec![main(), join(Vec2::new(20.0, -6.0))],
        Vec::new(),
        EVERYTHING,
    );
    assert!(shallow.stop_lines.is_empty(), "{:?}", shallow.stop_lines);
}

/// Двусторонняя третичная со светофорами поперёк пары односторонних с газоном
/// между половинами (оси в `apart` метрах, симметрично вокруг x 100), и на
/// перемычке посередине — переход OSM.
fn signals_across_a_pair(apart: f32) -> NodePaint {
    let [west, east] = [100.0 - apart / 2.0, 100.0 + apart / 2.0];
    let half = |x: f32, down: bool| {
        let mut points = vec![Vec2::new(x, 80.0), Vec2::new(x, 0.0), Vec2::new(x, -80.0)];
        if !down {
            points.reverse();
        }
        RoadLine {
            oneway: true,
            ..road(points, 7.6, Highway::Secondary, 2)
        }
    };
    let signals = |pos: Vec2| RoadNode {
        pos,
        kind: RoadNodeKind::TrafficSignals,
    };
    let mut map = MapData {
        roads: vec![
            road(
                vec![
                    Vec2::ZERO,
                    Vec2::new(west, 0.0),
                    Vec2::new(100.0, 0.0),
                    Vec2::new(east, 0.0),
                    Vec2::new(200.0, 0.0),
                ],
                8.0,
                Highway::Tertiary,
                2,
            ),
            half(west, true),
            half(east, false),
        ],
        road_nodes: vec![
            signals(Vec2::new(west, 0.0)),
            signals(Vec2::new(east, 0.0)),
            RoadNode {
                pos: Vec2::new(100.0, 0.0),
                kind: RoadNodeKind::Crossing {
                    signals: true,
                    island: false,
                    marked: true,
                },
            },
        ],
        ..default()
    };
    map.network = RoadNetwork::new(&map.roads);
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let run = |partner: usize| {
        vec![PairRun::for_test(
            0.0,
            160.0,
            partner,
            true,
            apart - 7.6,
            false,
        )]
    };
    let drawn = Drawn::for_test(&map)
        .with_pairs(1, run(2))
        .with_pairs(2, run(1));
    NodePaint::for_test(&drawn, &base, &map, &[], EVERYTHING)
}

/// Стоп-линии на перемычке между половинами — только если за ними есть где
/// ждать: на 28 м между осями (Рязань, витрина 07) очередь за линией встала бы
/// на соседний перекрёсток или на зебру через газон, и линий там нет; на
/// 60 м они есть. Подходы снаружи и сами половины — со стоп-линиями всегда.
#[test]
fn a_stop_line_needs_room_for_a_queue_behind_it() {
    // поперёк третичной (линия по y) между осями половин
    let inside = |paint: &NodePaint, apart: f32| {
        paint
            .stop_lines
            .iter()
            .filter(|line| {
                (line.from.x - line.to.x).abs() < 0.1 && (line.from.x - 100.0).abs() < apart / 2.0
            })
            .count()
    };
    let narrow = signals_across_a_pair(28.0);
    assert_eq!(inside(&narrow, 28.0), 0, "{:?}", narrow.stop_lines);
    assert_eq!(narrow.stop_lines.len(), 4, "{:?}", narrow.stop_lines);
    let wide = signals_across_a_pair(60.0);
    assert_eq!(inside(&wide, 60.0), 2, "{:?}", wide.stop_lines);
    assert_eq!(wide.stop_lines.len(), 6, "{:?}", wide.stop_lines);
}

/// Ветка треугольника развилки идёт от узла до узла по замощённому острову
/// (`corners::small_islands`): из асфальта узла она не выходит — перемычка,
/// ни стоп-линии, ни стрелок (пример 06, горловина). Без острова та же ветка —
/// обычное плечо.
#[test]
fn an_arm_across_a_paved_island_is_a_link() {
    let roads = || {
        vec![
            RoadLine {
                oneway: true,
                ..road(
                    vec![Vec2::new(140.0, 30.0), NODE],
                    7.6,
                    Highway::Secondary,
                    2,
                )
            },
            through(Highway::Secondary),
        ]
    };
    let paint = |paved: &[Vec<Vec2>]| {
        let mut map = MapData {
            roads: roads(),
            ..default()
        };
        map.network = RoadNetwork::new(&map.roads);
        let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
        NodePaint::for_test(&Drawn::for_test(&map), &base, &map, paved, EVERYTHING)
    };
    let link = |paint: &NodePaint| {
        paint.junctions[0]
            .arms
            .iter()
            .find(|arm| arm.road == 0)
            .unwrap()
            .link
    };
    let open = paint(&[]);
    assert!(!link(&open));
    assert_eq!(open.stop_lines.len(), 1);
    let island = vec![
        Vec2::new(90.0, -1.0),
        Vec2::new(160.0, -1.0),
        Vec2::new(160.0, 40.0),
        Vec2::new(90.0, 40.0),
    ];
    let paved = paint(&[island]);
    assert!(link(&paved));
    assert!(paved.stop_lines.is_empty(), "{:?}", paved.stop_lines);
}

/// Кромка плеча — там, где его сечение выходит из асфальта соседа, а не в
/// полуширине соседа от точки узла: у пологого примыкания стоп-линия встаёт
/// за асфальтом главной, а не пропадает внутри него (пример 06, горловина).
#[test]
fn the_edge_of_a_shallow_arm_is_where_it_leaves_the_other_asphalt() {
    let join = RoadLine {
        oneway: true,
        ..road(
            vec![Vec2::new(40.0, -35.0), NODE],
            7.6,
            Highway::Tertiary,
            2,
        )
    };
    let main = road(
        vec![Vec2::ZERO, NODE, Vec2::new(200.0, 0.0)],
        14.0,
        Highway::Primary,
        4,
    );
    let paint = paint_of(vec![main, join], Vec::new(), EVERYTHING);
    let [line] = paint.stop_lines.as_slice() else {
        panic!("{:?}", paint.stop_lines);
    };
    for end in [line.from, line.to] {
        assert!(end.y < -7.0 + EDGE_INSET, "{end} — в асфальте главной");
    }
    let [junction] = paint.junctions.as_slice() else {
        panic!("один узел");
    };
    let arm = junction.arms.iter().find(|arm| arm.road == 1).unwrap();
    let reach = 7.0 + JUNCTION_MARGIN;
    let length = (NODE - Vec2::new(40.0, -35.0)).length();
    assert!(
        length - arm.edge > reach + 1.0,
        "кромка дальше полуширины: {}",
        length - arm.edge
    );
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

/// Ветка развилки под 21° (Вокзальная из Первомайского, Рязань, витрина 03):
/// из асфальта главной в четыре полосы её сечение не выходит и за
/// `EDGE_SEARCH` — перемычка, и линия ветки начиналась на полуширине соседа,
/// посреди полос главной, крест-накрест с их линией. Её линия рвётся до
/// места, где ось ветки вышла из асфальта главной: 7.1 / sin 21° ≈ 19.8 м.
#[test]
fn a_fork_branch_keeps_its_line_off_the_lanes_of_the_main_road() {
    let oneway = |points: Vec<Vec2>, width: f32, highway: Highway, lanes: u8| RoadLine {
        oneway: true,
        ..road(points, width, highway, lanes)
    };
    let heading = Vec2::from_angle((180f32 - 21.0).to_radians());
    let paint = paint_of(
        vec![
            oneway(vec![Vec2::new(200.0, 0.0), NODE], 14.2, Highway::Primary, 4),
            oneway(vec![NODE, Vec2::ZERO], 14.2, Highway::Primary, 4),
            oneway(
                vec![NODE, NODE + heading * 80.0],
                7.6,
                Highway::Secondary,
                2,
            ),
        ],
        vec![RoadNode {
            pos: NODE,
            kind: RoadNodeKind::TrafficSignals,
        }],
        NodePaintStyle {
            crossings: CrossingMode::Off,
            stop_lines: true,
        },
    );
    let arm = paint.junctions[0]
        .arms
        .iter()
        .find(|arm| arm.road == 2)
        .expect("плечо ветки");
    assert!(arm.link, "ветка из асфальта главной не выходит — перемычка");
    let beyond = gaps(&paint, 2)
        .iter()
        .map(|found| (found.at - NODE).length() + found.reach)
        .fold(f32::MIN, f32::max);
    assert!(
        beyond > 19.0,
        "линия ветки с {beyond} м — в полосах главной: {:?}",
        paint.lines().of(2).cut
    );
}
