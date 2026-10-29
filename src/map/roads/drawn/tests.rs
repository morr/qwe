use std::borrow::Cow;

use super::super::fixtures::{
    a_tee, an_avenue_crossed_by_a_street, road, roundabout_with_an_approach, timeless, with_network,
};
use super::*;
use crate::map::meshing::Break;
use crate::map::osm::{Highway, MapData, RoadClass, RoadLine, SidewalkSide, fixture};
use crate::map::roads::axis;
use crate::map::roads::junctions;
use crate::map::roads::network::RoadNodes;
use crate::map::roads::network::pairs::{PROBE_STEP, Partner};
use crate::map::roads::pockets;
use crate::map::roads::rings;
use crate::map::roads::{
    CrossingMode, JunctionCounts, RoadShape, RoadStyle, VergeZoomBucket, is_carriageway, node_paint,
};
use crate::map::smooth::Smoothing;

#[test]
fn every_vector_is_indexed_by_the_map_roads() {
    let map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 80.0)], 8.0),
    ]);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(drawn.len(), 2);
    assert_eq!(drawn.roads().len(), 2);
    assert_eq!(drawn.axes(Axis::Nodal).len(), 2);
    assert_eq!(drawn.axes(Axis::Ribbon).len(), 2);
    assert_eq!(drawn.stats().crossings, 0);
}

#[test]
fn a_driveway_crossing_is_drawn_as_a_street_of_the_narrower_width() {
    // дорожка 3.5 м пересекает тротуар улицы между двумя кусками
    // проезда 5 м: рисуется улицей в 5 м, а на карте остаётся собой
    let footway = RoadLine {
        class: RoadClass::Alley,
        highway: crate::map::osm::Highway::Path,
        ..fixture::street(vec![Vec2::new(50.0, -10.0), Vec2::new(50.0, -18.0)], 3.5)
    };
    let map = with_network(vec![
        fixture::street(
            vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)],
            12.0,
        ),
        fixture::street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, -10.0)], 5.0),
        footway,
        fixture::street(vec![Vec2::new(50.0, -18.0), Vec2::new(50.0, -60.0)], 5.0),
    ]);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(drawn.stats().crossings, 1);
    assert_eq!(drawn.road(2).class, RoadClass::Street);
    assert_eq!(drawn.road(2).width, 5.0);
    assert_eq!(map.roads[2].class, RoadClass::Alley);
    assert_eq!(map.roads[2].width, 3.5);
    let roads = drawn.roads();
    assert!(
        std::ptr::eq(roads[0], &map.roads[0]),
        "без подмены — дорога карты"
    );
}

#[test]
fn stitched_axis_differs_from_nodal_only_at_stitched_ends() {
    // проезд кончается в трёх метрах за кромкой тротуара улицы
    // (`tests.rs::a_dangling_end_short_of_a_street_is_stitched`)
    let map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 5.0),
    ]);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(drawn.stats().stitches, 1);
    assert_eq!(drawn.stitched_end(0), [false; 2]);
    assert_eq!(drawn.stitched_end(1), [false, true]);
    assert_eq!(drawn.axis(0, Axis::Nodal), drawn.axis(0, Axis::Ribbon));
    let (nodal, ribbon) = (drawn.axis(1, Axis::Nodal), drawn.axis(1, Axis::Ribbon));
    assert_eq!(ribbon.len(), nodal.len() + 1);
    assert_eq!(&ribbon[..nodal.len()], nodal);
    assert!(ribbon[ribbon.len() - 1].distance(Vec2::new(50.0, 0.0)) < 1e-3);
    assert_eq!(drawn.stitch_offset(1), 0.0, "стежок у конца, не у начала");
    // облегчённый каркас стежков не ищет
    let nodal = Drawn::nodal(&map, &RoadShape::default());
    assert_eq!(nodal.stats().stitches, 0);
    assert_eq!(nodal.axis(1, Axis::Nodal), nodal.axis(1, Axis::Ribbon));
}

/// Проспект [`fixture::divided_avenue`] со средней вершиной при x = 300:
/// поперечной улице (`crossed_avenue`) нужна общая нода, чем примыкать
/// к половинам.
fn avenue(gap: f32) -> (Vec<RoadLine>, f32) {
    fixture::divided_avenue(gap, Some(300.0))
}

/// Тот же проспект с поперечной улицей из трёх way при x = 300: подход
/// с юга (2), кусок между половинами (3), продолжение на север (4).
fn crossed_avenue(gap: f32) -> MapData {
    let (mut roads, apart) = avenue(gap);
    let (south, north) = (Vec2::new(300.0, 100.0), Vec2::new(300.0, 100.0 + apart));
    roads.push(fixture::street(vec![Vec2::new(300.0, 30.0), south], 8.0));
    roads.push(fixture::street(vec![south, north], 8.0));
    roads.push(fixture::street(vec![north, Vec2::new(300.0, 190.0)], 8.0));
    with_network(roads)
}

#[test]
fn the_piece_between_two_halves_is_across_the_median() {
    let map = crossed_avenue(0.6);
    let drawn = Drawn::for_test(&map);
    assert_eq!(drawn.stats().medians, [1, 0, 0]);
    assert_eq!(drawn.sidewalk_mapped(3), None, "кусок в проёме пары");
    assert_eq!(drawn.sidewalk_drawn(3), None);
    let across = |road: usize| {
        drawn
            .pairs()
            .across_median(road, drawn.axis(road, Axis::Nodal), drawn.nodes())
    };
    assert_eq!(
        (0..5).map(across).collect::<Vec<_>>(),
        [false, false, false, true, false]
    );
    assert!(drawn.sidewalk_mapped(2).is_some(), "подход с юга");
    assert!(drawn.sidewalk_mapped(4).is_some(), "продолжение на север");
}

#[test]
fn a_short_piece_between_halves_that_are_no_pair_is_not_across_the_median() {
    // 20 м между кромками — шире самого широкого газона пары, а кусок
    // между ними короче самого длинного куска в проёме
    let map = crossed_avenue(20.0);
    let drawn = Drawn::for_test(&map);
    assert_eq!(drawn.stats().medians, [0, 0, 0]);
    assert!(drawn.sidewalk_mapped(3).is_some());
    assert!(
        !drawn
            .pairs()
            .across_median(3, drawn.axis(3, Axis::Nodal), drawn.nodes())
    );
}

#[test]
fn beside_reaches_the_slack_past_the_run() {
    let (roads, _) = avenue(0.6);
    let map = with_network(roads);
    let drawn = Drawn::for_test(&map);
    let pairs = drawn.pairs();
    let runs = pairs.runs(0);
    let (first, last) = (runs[0], runs[runs.len() - 1]);
    assert!(first.is_left(), "пара слева от половины, идущей на восток");
    let ([from, to], [_, end]) = (first.span(), last.span());
    // скругление в узле: две пробы слака
    let slack = 2.0 * PROBE_STEP;
    let beside = |at: f32| pairs.beside(0, at, slack);
    assert_eq!(beside((from + to) / 2.0), Some(true));
    assert_eq!(beside(from - 3.9), Some(true));
    assert_eq!(beside(from - 4.1), None);
    assert_eq!(beside(end + 3.9), Some(true));
    assert_eq!(beside(end + 4.1), None);
    // середина клина — без слака
    assert_eq!(pairs.beside(0, end, 0.0), Some(true));
    assert_eq!(pairs.beside(0, end + 0.1, 0.0), None);
    assert_eq!(pairs.beside(1, 200.0, slack), Some(true), "и у встречной");
    let partners: Vec<Partner> = pairs.partners(0).collect();
    assert_eq!(
        partners.first(),
        Some(&Partner {
            road: 1,
            paved: true
        })
    );
    assert!(pairs.is_paired(0, 1) && pairs.is_paired(1, 0));
    assert!(!pairs.is_paired(0, 0));
}

#[test]
fn band_half_takes_the_sidewalk_only_where_the_tag_puts_it() {
    let mut street = fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0);
    street.sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
    let map = with_network(vec![street]);
    let drawn = Drawn::for_test(&map);
    let band = crate::map::osm::model::sidewalk_band(8.0);
    assert_eq!(drawn.sidewalk_drawn(0), Some(band));
    assert_eq!(drawn.sidewalk_mapped(0), Some(band));
    assert_eq!(drawn.band_half(0, 0), 4.0 + band);
    assert_eq!(
        drawn.band_half(0, 1),
        4.0,
        "справа тротуара нет — голая кромка"
    );
    // ручка прячет ленту, но не тротуар по карте
    let hidden = Drawn::for_test(&map).with_sidewalks(false);
    assert_eq!(hidden.sidewalk_drawn(0), None);
    assert_eq!(hidden.sidewalk_mapped(0), Some(band));
    assert_eq!(hidden.band_half(0, 0), 4.0);
}

#[test]
fn sidewalk_on_answers_the_map_the_knob_the_median_gap_and_the_side() {
    let mut map = crossed_avenue(0.6);
    map.roads[2].sidewalks = [SidewalkSide::None, SidewalkSide::Inferred];
    let drawn = Drawn::for_test(&map);
    let band = drawn.sidewalk_drawn(2).expect("подход с юга с тротуаром");
    assert_eq!(drawn.sidewalk_on(2, 0), None, "по тегу слева нет");
    assert_eq!(drawn.sidewalk_on(2, 1), Some(band));
    assert_eq!(drawn.sidewalk_on(3, 0), None, "кусок в проёме пары");
    assert_eq!(drawn.sidewalk_on(3, 1), None);
    let hidden = Drawn::for_test(&map).with_sidewalks(false);
    assert_eq!(hidden.sidewalk_on(2, 1), None, "ручка прячет");
}

#[test]
fn passage_roads_are_not_smoothed() {
    // концы арки приколоты к вершинам контура здания — сглаживать её нельзя
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let nodes = RoadNodes::new(&[]);
    let arch = road(points.clone(), 5.0, true);
    assert_eq!(
        centerline(&arch, Smoothing::Strong, &nodes).as_ref(),
        points.as_slice()
    );
    let ordinary = road(points, 5.0, false);
    assert!(centerline(&ordinary, Smoothing::Strong, &nodes).len() > 3);
}

#[test]
fn a_shared_node_survives_smoothing() {
    // на изломе сквозной улицы кончается поперечная: хорда Chaikin сдвинула
    // бы узел, и торец поперечной повис бы мимо асфальта
    let corner = Vec2::new(20.0, 0.0);
    let through = road(vec![Vec2::ZERO, corner, Vec2::new(40.0, 20.0)], 8.0, false);
    let side = road(vec![Vec2::new(20.0, -30.0), corner], 8.0, false);
    let roads = [through, side];
    let nodes = RoadNodes::new(&roads);
    let drawn = centerline(&roads[0], Smoothing::Strong, &nodes);
    assert!(drawn.contains(&corner), "{drawn:?}");
}

#[test]
fn smoothing_off_borrows_the_osm_centerline() {
    let ordinary = road(vec![Vec2::ZERO, Vec2::new(20.0, 0.0)], 5.0, false);
    assert!(matches!(
        centerline(&ordinary, Smoothing::Off, &RoadNodes::new(&[])),
        Cow::Borrowed(_)
    ));
}

/// Въезд с улицы через тротуар, как его размечает OSM: проезд — дорожка
/// поперёк тротуара — снова проезд. Дорожка — `2`.
fn a_driveway() -> MapData {
    let footway = RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::street(vec![Vec2::new(50.0, -10.0), Vec2::new(50.0, -18.0)], 3.5)
    };
    with_network(vec![
        fixture::street(
            vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)],
            12.0,
        ),
        fixture::street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, -10.0)], 5.0),
        footway,
        fixture::street(vec![Vec2::new(50.0, -18.0), Vec2::new(50.0, -60.0)], 5.0),
    ])
}

#[test]
fn a_driveway_crossing_gets_no_base_break() {
    // Базовые разрывы (`junctions::marking_breaks`) считаются по дорогам
    // карты, краска узлов — по дорогам как рисуются, где переезд уже улица.
    // Разрыва у переезда нет ни в одном: дорожка остаётся `Highway::Path`, и
    // `is_carriageway` её не берёт в обоих — смена класса на `Street` тут
    // ничего не сдвигает.
    let map = a_driveway();
    let report = timeless(&map);
    assert_eq!(report.drawn.crossings, 1);
    let nodes = RoadNodes::new(&map.roads);
    let crossings = network::driveway_crossings(&map.roads, &nodes);
    assert_eq!(crossings, vec![(2, 5.0)]);
    let osm = junctions::marking_breaks(&map.roads, is_carriageway, &[]);
    assert!(osm.breaks[2].is_empty());
    let mut drawn = map.roads.clone();
    drawn[2] = RoadLine {
        class: RoadClass::Street,
        width: 5.0,
        ..map.roads[2].clone()
    };
    let as_drawn = junctions::marking_breaks(&drawn, is_carriageway, &[]);
    assert!(as_drawn.breaks[2].is_empty());
    assert_eq!(osm.breaks, as_drawn.breaks);
}

#[test]
fn a_ring_arc_base_break_reaches_by_the_osm_width() {
    // Где базовые разрывы и краска узлов правда расходятся — дуга кольца:
    // рисуется сечением всего кольца (`ring_arcs`), а базовый разрыв на
    // подходе меряет вылет по ширине дуги из OSM. Перевести базовые разрывы на
    // дороги как рисуются — сдвинуть их на подходах к кольцу.
    let map = a_ring_of_two_arcs();
    let mut nodes = RoadNodes::new(&map.roads);
    let axes = axis::street_axes(
        &map.roads,
        &map.rails,
        &map.network,
        &mut nodes,
        &RoadShape::default(),
    );
    let arcs = rings::ring_arcs(&map.roads, &axes.rings);
    assert_eq!(arcs.len(), 1);
    assert_eq!(arcs[0].0, 0);
    assert_eq!(arcs[0].1.width, 12.0);
    let mut drawn = map.roads.clone();
    drawn[0] = arcs[0].1.clone();
    let osm = junctions::marking_breaks(&map.roads, is_carriageway, &[]);
    let as_drawn = junctions::marking_breaks(&drawn, is_carriageway, &[]);
    assert_ne!(osm.breaks, as_drawn.breaks);
}

/// Кольцо из двух дуг разной ширины и подход: узкая дуга рисуется шириной
/// широкой (`ring_arcs`).
fn a_ring_of_two_arcs() -> MapData {
    let mut map = roundabout_with_an_approach(true, true);
    let circle = map.roads[0].points.clone();
    map.roads[0].points = circle[..=12].to_vec();
    let mut second = map.roads[0].clone();
    second.points = circle[12..].to_vec();
    second.width = 12.0;
    map.roads.push(second);
    // подходы — проезжие части: проезд `Service` краску не рвёт
    for road in &mut map.roads {
        road.highway = Highway::Residential;
    }
    with_network(map.roads)
}

/// Узлы обходятся один раз (`junctions::Junctions::new`): узлы проезжих
/// частей — это узлы участников ряда без прочих проходов, и по дорогам карты
/// они те же, что по дорогам как рисуются: переезд и дуга кольца меняют
/// ширину и класс, а не точки и не `is_carriageway`. Потому база (по дорогам
/// карты) и краска (прежде — по дорогам как рисуются) берут одни узлы.
#[test]
fn one_shared_node_pass_feeds_both_base_and_paint() {
    for map in [a_driveway(), a_ring_of_two_arcs()] {
        let drawn = Drawn::new(
            &map,
            &RoadStyle::default(),
            &RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        assert_eq!(drawn.stats().crossings, 1, "подмена легла");
        let every = junctions::shared_nodes(&map.roads, pockets::is_row_participant);
        assert_eq!(
            junctions::restrict(&every, &map.roads, is_carriageway),
            junctions::shared_nodes(&map.roads, is_carriageway)
        );
        let targets = &drawn.stitches().targets;
        assert_eq!(
            junctions::with_stitches(&map.roads, is_carriageway, targets),
            junctions::with_stitches(&drawn.roads(), is_carriageway, targets)
        );
    }
}

/// Ряд у бордюра стежков не видит, база и краска — видят: торец, дотянутый до
/// оси улицы, для карманов и машин остаётся тупиком (`Drawn::nodal` машин
/// стежков не строит, и ряд ленты обязан стоять там же), а для базы это
/// перекрёсток — улица рвётся, — и линии примыкания у него рвёт краска.
#[test]
fn row_breaks_ignore_stitches_but_paint_breaks_do_not() {
    // примыкание кончается в трёх метрах за кромкой улицы — как в
    // `a_dangling_end_short_of_a_street_is_stitched`, но улицей, а не
    // проездом: база и краска считаются по проезжим частям
    let map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 8.0),
    ]);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(drawn.stats().stitches, 1);
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let positive = |breaks: &[Break]| breaks.iter().filter(|found| found.reach > 0.0).count();
    let dead_ends = |breaks: &[Break]| breaks.iter().filter(|found| found.reach == 0.0).count();
    // база: стежок — узел, улица рвётся на нём, торец примыкания — не тупик
    assert_eq!(junctions.counts().count, 1);
    assert_eq!(positive(&junctions.median_base()[0]), 1);
    assert_eq!(dead_ends(&junctions.median_base()[1]), 1);
    // краска: примыкание уступает — его линии рвутся у стежка
    assert_eq!(positive(junctions.paint().of(1).cut), 1);
    // ряд: стежка нет — улица цела, оба торца примыкания — тупики
    assert_eq!(junctions.row().junctions, 0);
    assert_eq!(positive(junctions.row().of(0)), 0);
    assert_eq!(positive(junctions.row().of(1)), 0);
    assert_eq!(dead_ends(junctions.row().of(1)), 2);
}

/// Счётчики краски узлов в отчёте — перекрёстки, кластеры, проходы главной
/// насквозь, ведущие дороги, зебры, стоп-линии, карманы краски — по карте с
/// одним примыканием жилой к `tertiary`.
#[test]
fn the_report_counts_the_junction_paint() {
    let main = RoadLine {
        highway: Highway::Tertiary,
        lanes: Some(2),
        ..fixture::street(
            vec![Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)],
            7.6,
        )
    };
    let side = RoadLine {
        lanes: Some(2),
        ..fixture::street(vec![Vec2::new(100.0, -80.0), Vec2::new(100.0, 0.0)], 7.6)
    };
    let report = timeless(&with_network(vec![main, side]));
    assert_eq!(
        report.junctions,
        JunctionCounts {
            count: 1,
            clusters: 0,
            through: 1,
            leading: 1,
            zebras: [1, 0],
            stop_lines: 1,
            pockets: 0,
        }
    );
}

#[test]
fn a_dangling_end_short_of_a_street_is_stitched() {
    // проезд кончается в трёх метрах за кромкой тротуара улицы
    let map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 5.0),
    ]);
    let report = timeless(&map);
    assert_eq!(report.drawn.stitches, 1);
    assert_eq!(report.drawn.crossings, 0);
    assert_eq!(report.drawn.tapers, 0);
    assert_eq!(report.drawn.merges, 0);
    assert_eq!(report.merge_edges, 0);
}

#[test]
fn a_section_seam_is_one_taper() {
    let street = |points: Vec<Vec2>, lanes: u8| RoadLine {
        lanes: Some(lanes),
        ..fixture::street(points, f32::from(lanes) * 3.3 + 1.0)
    };
    let map = with_network(vec![
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 2),
        street(vec![Vec2::new(200.0, 0.0), Vec2::new(400.0, 0.0)], 4),
    ]);
    let report = timeless(&map);
    assert_eq!(report.drawn.tapers, 1);
    assert_eq!(report.drawn.stitches, 0);
    assert_eq!(report.drawn.crossings, 0);
    assert_eq!(report.drawn.merges, 0);
    assert_eq!(report.merge_edges, 0);
}

#[test]
fn a_divided_street_merging_into_a_two_way_one_is_one_merge() {
    // половины в три полосы сходятся в узел, двусторонняя в четыре уходит
    // от него на восток — как в `merges/tests.rs`
    let width = |lanes: u8| f32::from(lanes) * 3.3 + 1.0;
    let primary = |points: Vec<Vec2>, lanes: u8, oneway: bool| RoadLine {
        highway: Highway::Primary,
        oneway,
        lanes: Some(lanes),
        ..fixture::street(points, width(lanes))
    };
    let apart = width(3) + 3.0;
    let node = Vec2::new(240.0, apart / 2.0);
    let map = with_network(vec![
        primary(vec![Vec2::ZERO, Vec2::new(200.0, 0.0), node], 3, true),
        primary(
            vec![node, Vec2::new(200.0, apart), Vec2::new(0.0, apart)],
            3,
            true,
        ),
        primary(vec![node, node + Vec2::new(160.0, 0.0)], 4, false),
    ]);
    let report = timeless(&map);
    assert_eq!(report.drawn.merges, 1);
    assert_eq!(report.merge_edges, 2);
    assert_eq!(report.drawn.crossings, 0);
    assert_eq!(report.drawn.stitches, 0);
    assert_eq!(report.drawn.tapers, 0);
    assert_eq!(timeless(&map), report, "отчёт повторяется до поля");
}

/// Счётчики узлов в строке `road meshing:` — ни одним тестом не пиннились.
#[test]
fn junction_counters_of_a_tee_and_an_avenue_crossing() {
    let tee = timeless(&a_tee()).junctions;
    assert_eq!((tee.count, tee.clusters, tee.through), (1, 0, 1));
    let (avenue, _) = an_avenue_crossed_by_a_street();
    let avenue = timeless(&avenue).junctions;
    assert_eq!((avenue.count, avenue.clusters, avenue.through), (2, 1, 2));
}

// DIAG-TEMP: `QWE_DIAG_JSON=<срез> QWE_DIAG_CITY=tula` — улицы, прямые в OSM,
// чья нарисованная ось дрожит вбок: профиль бокового смещения (шаг 2 м) после
// вычитания скользящего среднего (±24 м) — зигзаг и ступени видны, плавный
// съезд и дуга скругления нет. Печатает и позиции швов ways улицы.
#[test]
#[ignore]
fn diag_straight_zigzag() {
    use crate::city::City;
    use crate::map::osm::overpass::OverpassResponse;
    use crate::map::osm::parse::{ParseKnobs, parse_response};
    let path = std::env::var("QWE_DIAG_JSON").unwrap();
    let city = match std::env::var("QWE_DIAG_CITY").unwrap().as_str() {
        "oryol" => City::Oryol,
        _ => City::Tula,
    };
    let osm: OverpassResponse =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let map = parse_response(&osm, city, ParseKnobs::default());
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let (min_len, step) = (120.0, 2.0);
    let mut hits = 0;
    for i in 0..prepared.len() {
        let road = prepared.road(i);
        if road.class != RoadClass::Street || prepared.on_ring(i) {
            continue;
        }
        let raw = &road.points;
        if raw.len() < 3 {
            continue;
        }
        let (a, b) = (raw[0], raw[raw.len() - 1]);
        let len = a.distance(b);
        if len < min_len {
            continue;
        }
        let normal = ((b - a) / len).perp();
        let osm_dev = raw
            .iter()
            .map(|p| (p - a).dot(normal).abs())
            .fold(0.0_f32, f32::max);
        if osm_dev > 1.5 {
            continue;
        }
        let paired = prepared.pairs().has_runs(i);
        let drawn = prepared.axis(i, Axis::Ribbon);
        let (arcs, total) = crate::map::along::arclengths(drawn);
        let mut offs: Vec<f32> = Vec::new();
        let mut s = 0.0;
        while s <= total {
            if let Some((p, _)) = crate::map::along::place_on_path(drawn, &arcs, s) {
                offs.push((p - a).dot(normal));
            }
            s += step;
        }
        // доли одного знака в средней трети; зигзаг — их не меньше трёх
        let (lo_s, hi_s) = (total * 0.2, total * 0.8);
        let mut lobes: Vec<(f32, f32, f32, f32)> = Vec::new(); // (start, end, sum, count)
        for (k, &off) in offs.iter().enumerate() {
            let at = k as f32 * step;
            if at < lo_s || at > hi_s {
                continue;
            }
            match lobes.last_mut() {
                Some(last) if last.2.signum() == off.signum() => {
                    last.1 = at;
                    last.2 += off;
                    last.3 += 1.0;
                }
                _ => lobes.push((at, at, off, 1.0)),
            }
        }
        let zig: Vec<(f32, f32, f32)> = lobes
            .iter()
            .copied()
            .filter(|(from, to, sum, count)| sum.abs() >= 0.25 * count && to - from >= 4.0)
            .map(|(from, to, sum, count)| (from, to, sum / count))
            .collect();
        if zig.len() < 3 {
            continue;
        }
        hits += 1;
        eprintln!(
            "DIAG-ZZ road {i} len={len:.0} axis={total:.0} osm_dev={osm_dev:.2} lobes={} paired={} hw={:?} w={:.1}",
            zig.len(),
            paired,
            road.highway,
            road.width
        );
        eprintln!(
            "    zig {:?}",
            zig.iter()
                .map(|(a, b, m)| format!("{a:.0}-{b:.0}:{m:+.2}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        eprintln!("    start {a:?} end {b:?}");
        if paired {
            for partner in prepared.pairs().partners(i) {
                eprintln!("    partner road={} paved={}", partner.road, partner.paved);
            }
        }
        // вершины нарисованной оси и сырые точки OSM в окне самой глубокой
        // доли: (метр по оси | вбок от хорды) — видно, где ломается ось
        let (win_from, win_to) = (
            zig.iter().map(|z| z.0).fold(f32::MAX, f32::min) - 40.0,
            zig.iter().map(|z| z.1).fold(0.0_f32, f32::max) + 40.0,
        );
        let (arcs_v, _) = crate::map::along::arclengths(drawn);
        let near: Vec<String> = drawn
            .iter()
            .enumerate()
            .filter(|(k, _)| arcs_v[*k] >= win_from && arcs_v[*k] <= win_to)
            .map(|(k, p)| {
                format!(
                    "{:.0}s({:.1},{:.1}):{:+.2}",
                    arcs_v[k],
                    p.x,
                    p.y,
                    (p - a).dot(normal)
                )
            })
            .collect();
        eprintln!("    drawn {}", near.join(" "));
        let raws: Vec<String> = raw
            .iter()
            .map(|p| format!("({:.1},{:.1}):{:+.2}", p.x, p.y, (p - a).dot(normal)))
            .collect();
        eprintln!("    osm {}", raws.join(" "));
        eprintln!(
            "    offs {:?}",
            offs.iter()
                .enumerate()
                .map(|(k, o)| format!("{}:{o:+.2}", k as f32 * step))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    eprintln!("DIAG-ZZ hits={hits}");
}
