use std::borrow::Cow;

use bevy::prelude::*;

use super::{MedianEnd, Merge, Merges, merge_axis, merge_bands, merge_ramps, merges};
use crate::map::meshing::LaneFrame;
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, MapData, RoadLine, RoadNode, RoadNodeKind};
use crate::map::roads::corners::kerb_returns;
use crate::map::roads::drawn::{Axis, Drawn};
use crate::map::roads::is_carriageway;
use crate::map::roads::junctions::marking_breaks;
use crate::map::roads::network::pairs::Pairs;
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::node_paint::{CrossingMode, NodePaint, NodePaintStyle};
use crate::map::roads::shape::{RoadShape, lane_width};
use crate::map::roads::tapers::{TAPER_PER_METER, Tapers};

/// Полотно в `lanes` полос: ширина — как из сечения.
fn width(lanes: u8) -> f32 {
    lanes as f32 * 3.3 + 1.0
}

fn primary(points: Vec<Vec2>, lanes: u8, oneway: bool) -> RoadLine {
    RoadLine {
        highway: Highway::Primary,
        oneway,
        lanes: Some(lanes),
        ..street(points, width(lanes))
    }
}

/// Узел слияния: половины по три полосы с зазором в 3 м вдоль x сходятся в
/// него с запада, продолжение `street` уходит от него.
const HALF_LANES: u8 = 3;

fn node() -> Vec2 {
    Vec2::new(240.0, (width(HALF_LANES) + 3.0) / 2.0)
}

/// Южная половина едет на восток в узел, северная — из узла на запад; третья
/// дорога — `continuation`.
fn divided_into(continuation: RoadLine) -> Vec<RoadLine> {
    let apart = width(HALF_LANES) + 3.0;
    vec![
        primary(
            vec![Vec2::ZERO, Vec2::new(200.0, 0.0), node()],
            HALF_LANES,
            true,
        ),
        primary(
            vec![node(), Vec2::new(200.0, apart), Vec2::new(0.0, apart)],
            HALF_LANES,
            true,
        ),
        continuation,
    ]
}

/// Двусторонняя в четыре полосы на восток от узла.
fn two_way_east() -> RoadLine {
    primary(vec![node(), node() + Vec2::new(160.0, 0.0)], 4, false)
}

/// Пары и разведённые оси, как их отдаёт `axis::street_axes` без
/// сглаживания, и найденные по ним слияния.
fn found(roads: &[RoadLine]) -> (Merges, RoadNetwork, Vec<Vec<Vec2>>) {
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    let nodes = RoadNodes::new(roads);
    let network = RoadNetwork::new(roads);
    let mut pairs = Pairs::new(roads, &paths, RoadShape::default().median_gap(), &[]);
    pairs.align(&mut paths, roads, &network, &nodes, &Tapers::default());
    let paths: Vec<Vec<Vec2>> = paths.into_iter().map(Cow::into_owned).collect();
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let merges = merges(&drawn, &paths, &nodes, &pairs, &network);
    (merges, network, paths)
}

#[test]
fn halves_ending_on_a_two_way_street_of_their_class_are_a_merge() {
    let (merges, _, _) = found(&divided_into(two_way_east()));
    assert_eq!(
        merges.list,
        vec![Merge {
            node: node(),
            halves: [0, 1],
            street: 2,
            street_end: 0,
            pure: true,
        }]
    );
    assert!(merges.is_merged(0, 1) && merges.is_merged(1, 0) && merges.is_merged(2, 0));
    assert!(!merges.is_merged(0, 0) && !merges.is_merged(2, 1));
}

#[test]
fn a_street_across_the_node_or_of_another_class_is_no_merge() {
    let across = primary(vec![node(), node() - Vec2::new(0.0, 120.0)], 4, false);
    let secondary = RoadLine {
        highway: Highway::Secondary,
        ..two_way_east()
    };
    let oneway = RoadLine {
        oneway: true,
        ..two_way_east()
    };
    for continuation in [across, secondary, oneway] {
        let (merges, _, _) = found(&divided_into(continuation.clone()));
        assert!(merges.list.is_empty(), "{:?}", continuation.points);
    }
}

#[test]
fn a_merge_node_gets_no_square_ends_and_no_outer_corners() {
    let roads = divided_into(two_way_east());
    let map = MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    };
    let drawn = Drawn::for_test(&map).with_sidewalks(false);
    assert!(drawn.is_merged(0, 1) && drawn.is_merged(1, 0) && drawn.is_merged(2, 0));
    // как перекрёсток — торцы прямые (а где плечи разошлись шире
    // развёрнутого, ещё и наружный угол): это и был шип
    let junction = kerb_returns(
        &Drawn::for_test(&map).with_sidewalks(false).without_merges(),
        1.0,
    );
    assert!(junction.butt(0)[1] && junction.butt(1)[0] && junction.butt(2)[0]);
    let merge = kerb_returns(&drawn, 1.0);
    for road in 0..3 {
        assert_eq!(merge.butt(road), [false; 2], "{road}");
    }
    assert!(merge.roads.is_empty(), "{:?}", merge.roads);
}

#[test]
fn each_outer_kerb_runs_into_the_kerb_of_the_continuation() {
    let roads = divided_into(two_way_east());
    let (merges, network, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        &network,
        |_, _| Some(3.0),
        TAPER_PER_METER,
    );
    assert_eq!(bands.len(), 2);
    let (wide, narrow) = (width(4) / 2.0, width(HALF_LANES) / 2.0);
    for band in &bands {
        let outer = &band.asphalt;
        // у узла — на полуширине продолжения
        assert!(
            (outer[0].distance(node()) - wide).abs() < 0.05,
            "{}",
            outer[0]
        );
        // наружу, от пары: южная половина — к югу, северная — к северу
        let south = band.half == 0;
        assert_eq!(outer[0].y < node().y, south, "{}", outer[0]);
        // клин — ручка на метр разницы ширин
        let length = 2.0 * (wide - narrow) * TAPER_PER_METER;
        assert!((band.length - length).abs() < 1e-3, "{}", band.length);
        // и тротуар за асфальтом
        let sidewalk = band.sidewalk.as_ref().unwrap();
        assert!((sidewalk[0].distance(node()) - wide - 3.0).abs() < 0.05);
    }
    // за клином кромка — своя: полуширина от оси половины
    let far = bands[0].asphalt.len() / 2 - 1;
    let path = &paths[0];
    let edge = bands[0].asphalt[far];
    let axis = crate::map::meshing::distance_to_path(edge, path);
    assert!((axis - narrow).abs() < 0.05, "{axis}");
}

#[test]
fn halves_cut_short_at_the_node_still_merge_and_the_wedge_runs_on_their_street() {
    // OSM режет половину у узла: последний way — только сходящийся кусок в
    // 40 м, пара лежит на соседнем
    let apart = width(HALF_LANES) + 3.0;
    let roads = vec![
        primary(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], HALF_LANES, true),
        primary(vec![Vec2::new(200.0, 0.0), node()], HALF_LANES, true),
        primary(vec![node(), Vec2::new(200.0, apart)], HALF_LANES, true),
        primary(
            vec![Vec2::new(200.0, apart), Vec2::new(0.0, apart)],
            HALF_LANES,
            true,
        ),
        two_way_east(),
    ];
    let (merges, network, paths) = found(&roads);
    assert_eq!(merges.list.len(), 1);
    assert_eq!((merges.list[0].halves, merges.list[0].street), ([1, 2], 4));
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        &network,
        |_, _| None,
        TAPER_PER_METER,
    );
    // клин во всю длину, а не в 0.6 короткого way
    let length = 2.0 * (width(4) - width(HALF_LANES)) / 2.0 * TAPER_PER_METER;
    assert_eq!(bands.len(), 2);
    for band in &bands {
        assert!((band.length - length).abs() < 1e-3, "{}", band.length);
        assert_eq!(band.asphalt[0].y < node().y, band.half == 1);
    }
}

#[test]
fn a_street_crossing_the_node_makes_the_merge_a_junction_for_paint() {
    let mut roads = divided_into(two_way_east());
    roads.push(RoadLine {
        highway: Highway::Residential,
        ..street(vec![node(), node() - Vec2::new(0.0, 80.0)], 8.0)
    });
    let (merges, _, _) = found(&roads);
    assert_eq!(merges.list.len(), 1);
    assert!(!merges.list[0].pure);
    assert!(!merges.is_pure_node(node()));
}

/// Нечистое слияние — пара до перекрёстка (R33, Белгород, Попова ×
/// Павлова): половины доходят до поперечной улицы параллельно, каждая на
/// своей стороне устья продолжения, в `полуширина продолжения − своя` от его
/// оси, торцом на оси поперечной — своим узлом; клина слияния нет.
#[test]
fn halves_merging_at_a_crossing_reach_it_side_by_side() {
    let mut roads = divided_into(two_way_east());
    roads.push(RoadLine {
        highway: Highway::Residential,
        ..street(
            vec![
                node() - Vec2::new(0.0, 80.0),
                node(),
                node() + Vec2::new(0.0, 80.0),
            ],
            8.0,
        )
    });
    let map = MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    };
    let drawn = Drawn::for_test(&map);
    let offset = (width(4) - width(HALF_LANES)) / 2.0;
    let into = drawn.axis(0, Axis::Nodal);
    let out = drawn.axis(1, Axis::Nodal);
    let [into_end, out_start] = [into[into.len() - 1], out[0]];
    // торцы — на оси поперечной, по сторонам оси продолжения
    for (end, side) in [(into_end, -1.0), (out_start, 1.0)] {
        assert!((end.x - node().x).abs() < 1e-3, "{end} is off the cross street");
        let lateral = (end.y - node().y) * side;
        assert!(
            (lateral - offset).abs() < 0.05,
            "the half ends {lateral} m off the continuation axis, not {offset}"
        );
    }
    // и не сходятся к узлу: у самого устья ось не ближе сдвига
    for (path, side) in [(into, -1.0), (out, 1.0)] {
        for point in path.iter().filter(|point| node().x - point.x < 12.0) {
            let lateral = (point.y - node().y) * side;
            assert!(
                lateral > offset - 0.05,
                "{} m before the crossing the half is {lateral} m off the axis, under {offset}",
                node().x - point.x
            );
        }
    }
    assert!(drawn.merges().list.is_empty(), "the crossing is still a merge");
    // пара до самого устья: половины — пара друг другу
    assert!(drawn.pairs().is_paired(0, 1) && drawn.pairs().is_paired(1, 0));
}

#[test]
fn a_pure_merge_node_breaks_no_line_and_holds_the_axis_solid() {
    let roads = divided_into(two_way_east());
    let map = MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    };
    let drawn = Drawn::for_test(&map);
    assert!(drawn.merges().list.iter().any(|merge| merge.pure));
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let at_node = |breaks: &[crate::map::meshing::Break]| {
        breaks.iter().any(|gap| gap.at.distance(node()) < 0.1)
    };
    // как перекрёсток трёх улиц — рвутся все три: это и был разрыв
    assert!((0..3).all(|road| at_node(&base[road])));
    let merged = NodePaint::for_test(
        &drawn,
        &base,
        &map,
        &[],
        NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    for road in 0..3 {
        let lines = merged.lines().of(road);
        assert!(!at_node(lines.cut), "{road}");
        assert!(at_node(lines.solid), "{road}");
    }
    assert!(merged.zebras.is_empty() && merged.stop_lines.is_empty());
    assert!(merged.junctions.is_empty());
}

#[test]
fn each_half_frame_runs_into_its_side_of_the_continuation_frame() {
    let roads = divided_into(two_way_east());
    let (merges, network, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let ramps = merge_ramps(&merges.list[0], &drawn, &paths, &network, TAPER_PER_METER);
    let lane = lane_width();
    assert_eq!(ramps.len(), 2);
    for (road, ramp) in ramps {
        // по ходу обеих половин пара слева: полосы — от кромки продолжения
        // справа до его оси, сетка продолжения (четыре полосы — узел на оси),
        // наружная линия тела (−½ полосы) уходит в наружную (−1)
        assert_eq!(
            ramp.frame,
            LaneFrame {
                origin: 0.0,
                low: -2.0 * lane,
                high: 0.0,
            },
            "{road}"
        );
        // сдвиг — полполосы, кромка — полполосы: клин в полосу
        assert!((ramp.length - lane * TAPER_PER_METER).abs() < 1e-3);
        let length = crate::map::osm::model::polyline_length(&paths[road]);
        let (start, away) = if road == 0 {
            (length, false)
        } else {
            (0.0, true)
        };
        assert!(
            (ramp.start - start).abs() < 1e-3 && ramp.away == away,
            "{road}"
        );
    }
}

#[test]
fn a_ramp_runs_through_the_ways_of_a_half_cut_short_at_the_node() {
    // короткие куски в 15 м у узла — клин в 33 м заходит на соседние ways
    let apart = width(HALF_LANES) + 3.0;
    let [south, north] = [0.0, apart].map(|y| Vec2::new(200.0, y).lerp(node(), 0.625));
    let roads = vec![
        primary(
            vec![Vec2::ZERO, Vec2::new(200.0, 0.0), south],
            HALF_LANES,
            true,
        ),
        primary(vec![south, node()], HALF_LANES, true),
        primary(vec![node(), north], HALF_LANES, true),
        primary(
            vec![north, Vec2::new(200.0, apart), Vec2::new(0.0, apart)],
            HALF_LANES,
            true,
        ),
        two_way_east(),
    ];
    let (merges, network, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let ramps = merge_ramps(&merges.list[0], &drawn, &paths, &network, TAPER_PER_METER);
    let roads: Vec<usize> = ramps.iter().map(|(road, _)| *road).collect();
    assert_eq!(roads, vec![1, 0, 2, 3]);
    // дальний way въезжающей — от узла за коротким
    let short = crate::map::osm::model::polyline_length(&paths[1]);
    let far = ramps[1].1;
    let whole = crate::map::osm::model::polyline_length(&paths[0]);
    assert!((far.start - (short + whole)).abs() < 1e-3 && !far.away);
}

#[test]
fn the_merge_axis_runs_from_the_node_to_the_median_or_until_the_halves_part() {
    let roads = divided_into(two_way_east());
    let (merges, network, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let merge = &merges.list[0];
    // разделительной нет — пока кромки половин не разошлись
    let free = merge_axis(merge, &drawn, &paths, &network, &[]);
    assert_eq!(free[0], node());
    let tip = *free.last().unwrap();
    assert!(tip.x < node().x - 20.0 && tip.x > node().x - 40.0, "{tip}");
    // по середине между половинами
    assert!(free.iter().all(|point| (point.y - node().y).abs() < 0.5));
    // до носа газона — с отступом
    let nose = Vec2::new(node().x - 12.0, node().y);
    let to_nose = merge_axis(merge, &drawn, &paths, &network, &[MedianEnd::Lawn(nose)]);
    let tip = *to_nose.last().unwrap();
    assert!((tip.x - (nose.x + 1.0)).abs() < 0.3, "{tip}");
    // нос дальше, чем кромки сходятся: до носа между половинами асфальт
    // (`nose_fill`), и осевая идёт до него
    let far = Vec2::new(node().x - 50.0, node().y);
    let long = merge_axis(merge, &drawn, &paths, &network, &[MedianEnd::Lawn(far)]);
    assert!(
        (long.last().unwrap().x - (far.x + 1.0)).abs() < 0.3,
        "{long:?}"
    );
    // до асфальтовой середины — сквозь, и смыкается с её торцом
    let paved = merge_axis(merge, &drawn, &paths, &network, &[MedianEnd::Paved(far)]);
    assert!(paved.last().unwrap().distance(far) < 0.01);
}

/// Где кромки половин разошлись, а газон ещё не начался, между ними асфальт
/// до носа — и за острие, но не поверх бордюра газона.
#[test]
fn the_ground_between_parted_halves_is_paved_up_to_the_lawn_nose() {
    use crate::map::shapes::{oriented, point_in_shape};
    let roads = divided_into(two_way_east());
    let (merges, network, paths) = found(&roads);
    let merge = &merges.list[0];
    let nose = Vec2::new(node().x - 50.0, node().y);
    // газон — от острия на запад
    let lawn = |x: f32| Vec2::new(x, node().y);
    let kerb = vec![oriented(
        &[
            lawn(nose.x) + Vec2::new(0.0, -1.5),
            lawn(nose.x - 30.0) + Vec2::new(0.0, -1.5),
            lawn(nose.x - 30.0) + Vec2::new(0.0, 1.5),
            lawn(nose.x) + Vec2::new(0.0, 1.5),
        ],
        true,
    )];
    let fill = super::nose_fill(merge, &paths, &network, &[MedianEnd::Lawn(nose)], &[kerb]);
    let covered = |at: Vec2| fill.iter().any(|shape| point_in_shape(at, shape));
    // между разошедшимися половинами перед носом — асфальт
    assert!(covered(lawn(nose.x + 5.0)), "{fill:?}");
    // за острие, но в стороне от бордюра — тоже
    assert!(
        covered(lawn(nose.x - 2.0) + Vec2::new(0.0, 2.5)),
        "{fill:?}"
    );
    // на газоне — нет
    assert!(!covered(lawn(nose.x - 2.0)), "{fill:?}");
    // без газона — ничего
    assert!(super::nose_fill(merge, &paths, &network, &[], &[]).is_empty());
}

#[test]
fn a_continuation_no_wider_than_a_half_needs_no_band() {
    let roads = divided_into(primary(
        vec![node(), node() + Vec2::new(160.0, 0.0)],
        2,
        false,
    ));
    let (merges, network, paths) = found(&roads);
    assert_eq!(merges.list.len(), 1);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        &network,
        |_, _| None,
        TAPER_PER_METER,
    );
    assert!(bands.is_empty());
}

/// Y-развилка южного подхода к кольцу Тулы (R16): двусторонняя primary в 4
/// полосы с юга кончается в узле, из него съезд въезжает с северо-запада,
/// въезд уходит на северо-северо-восток — ветки по 2 полосы расходятся на 48°,
/// пары между ними нет. `street` — продолжение, `extra` — прочие дороги узла.
fn fork(street: RoadLine, extra: &[RoadLine]) -> Vec<RoadLine> {
    let mut roads = vec![
        primary(
            vec![Vec2::new(-19.6, 27.7), Vec2::new(-8.3, 15.6), Vec2::ZERO],
            2,
            true,
        ),
        primary(vec![Vec2::ZERO, Vec2::new(3.3, 10.1)], 2, true),
        street,
    ];
    roads.extend_from_slice(extra);
    roads
}

fn fork_street() -> RoadLine {
    primary(vec![Vec2::new(6.1, -120.0), Vec2::ZERO], 4, false)
}

#[test]
fn a_two_way_splitting_into_two_oneways_is_a_fork_merge() {
    let (merges, _, _) = found(&fork(fork_street(), &[]));
    assert_eq!(
        merges.list,
        vec![Merge {
            node: Vec2::ZERO,
            halves: [0, 1],
            street: 2,
            street_end: 1,
            pure: true,
        }]
    );
    // и ветки продолжают каждая свою половину: у узла кромка ветки — на
    // полуширине продолжения
    let roads = fork(fork_street(), &[]);
    let (merges, network, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        &network,
        |_, _| None,
        TAPER_PER_METER,
    );
    assert_eq!(bands.len(), 2);
    for band in &bands {
        assert!((band.asphalt[0].length() - width(4) / 2.0).abs() < 0.05);
    }
}

#[test]
fn a_fork_at_a_bridge_head_is_a_merge() {
    // Орёл, Р-119: двусторонний мост делится на въезд и съезд на своём торце
    let bridge = RoadLine {
        bridge: true,
        ..fork_street()
    };
    let (merges, _, _) = found(&fork(bridge, &[]));
    assert_eq!(merges.list.len(), 1);
    assert_eq!(merges.list[0].street, 2);
}

#[test]
fn a_oneway_grid_corner_is_not_a_fork() {
    // угол сетки: односторонняя въезжает с запада, другая уходит на север —
    // между ветками 90°, это перекрёсток
    let corner = vec![
        primary(vec![Vec2::new(-80.0, 0.0), Vec2::ZERO], 2, true),
        primary(vec![Vec2::ZERO, Vec2::new(0.0, 80.0)], 2, true),
        primary(vec![Vec2::ZERO, Vec2::new(60.0, -60.0)], 4, false),
    ];
    assert!(found(&corner).0.list.is_empty());
    // развилка с третьей проезжей частью в узле — тоже перекрёсток
    let side = RoadLine {
        highway: Highway::Residential,
        ..street(vec![Vec2::ZERO, Vec2::new(-60.0, -10.0)], 8.0)
    };
    assert!(found(&fork(fork_street(), &[side])).0.list.is_empty());
}

#[test]
fn a_pure_fork_gets_no_rule_zebra_or_stop_line() {
    let roads = fork(fork_street(), &[]);
    let map = MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    };
    let drawn = Drawn::for_test(&map);
    let base = marking_breaks(&map.roads, is_carriageway, &[]).breaks;
    let paint = NodePaint::for_test(
        &drawn,
        &base,
        &map,
        &[],
        NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    assert!(paint.zebras.is_empty() && paint.stop_lines.is_empty());
    assert!(paint.junctions.is_empty());
    // переход OSM в узле развилки (Болдина, R17) — одна зебра поперёк
    // продолжения, целиком на нём
    let mut map = map;
    map.road_nodes.push(RoadNode {
        pos: Vec2::ZERO,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    });
    let drawn = Drawn::for_test(&map);
    let paint = NodePaint::for_test(
        &drawn,
        &base,
        &map,
        &[],
        NodePaintStyle {
            crossings: CrossingMode::Osm,
            stop_lines: true,
        },
    );
    assert_eq!(paint.zebras.len(), 1, "{:?}", paint.zebras);
    let zebra = paint.zebras[0];
    let middle = (zebra.from + zebra.to) / 2.0;
    assert!(zebra.osm && middle.y < -1.0, "{zebra:?}");
}
