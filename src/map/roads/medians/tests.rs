use super::super::fixtures::{
    an_avenue_crossed_by_a_street, divided_avenue, layer, vertices_of, with_network,
};
use super::super::{
    RoadBuild, RoadShape, RoadStyle, TRAM_BAND_COLOR, VergeZoomBucket, build_roads, mesh_roads,
    tram_band, tram_lay,
};
use super::*;
use crate::map::osm::model::{RailKind, RailLine, RoadNode, RoadNodeKind};
use crate::map::osm::{Highway, MapData, RoadClass, RoadLine, fixture};
use crate::map::roads::Drawn;
use crate::map::roads::junctions::Junctions;
use crate::map::roads::network::RoadNetwork;
use crate::map::roads::node_paint::{CrossingMode, NodePaintStyle};
use crate::map::roads::paint;
use crate::map::shapes::shape_area;

/// Проспект [`fixture::divided_avenue`] с настоящей сетью улиц:
/// разделительному нужен `street_of` из [`RoadNetwork`].
fn avenue(gap: f32) -> MapData {
    let (roads, _) = fixture::divided_avenue(gap, None);
    MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    }
}

/// Слои, в которые кладёт [`draw`]: улицы, тротуары, трава.
struct Layers {
    streets: MeshBuilder,
    sidewalks: MeshBuilder,
    grass: MeshBuilder,
}

/// [`draw`] на карте `map` без узлов и островков: краску он не кладёт, а
/// отдаёт — painter тесту не нужен.
fn drawn_medians(map: &MapData, markings: bool) -> (MedianDrawing, Layers) {
    let drawn = Drawn::for_test(map);
    let style = NodePaintStyle {
        crossings: CrossingMode::Off,
        stop_lines: false,
    };
    let junctions = Junctions::new(&drawn, map, &[], style);
    let mut layers = Layers {
        streets: MeshBuilder::with_surface_coords(),
        sidewalks: MeshBuilder::with_surface_coords(),
        grass: MeshBuilder::with_surface_coords(),
    };
    let street_of = |road: usize| map.network.street_of(road).map(|(street, _)| street);
    let drawing = draw(
        drawn.pairs(),
        &MedianInputs {
            base: junctions.median_base(),
            paint: junctions.paint(),
            zebras: junctions.node_paint().marks().zebras(),
            half: &|road| drawn.road(road).width / 2.0,
            markings,
            pure_merge: &|_| false,
            reach_gores: &|midline| vec![midline.to_vec()],
            street_of: &street_of,
            closer: &|_, _, _, _| None,
            on_bridge: &|_| false,
        },
        &mut layers.streets,
        &mut layers.sidewalks,
        &mut layers.grass,
        &mut MeshBuilder::with_surface_coords(),
    );
    (drawing, layers)
}

/// Узкая разделительная — асфальт в слое улиц и одна двойная сплошная во
/// всю длину, отданная списком; торцы — асфальтовые, травы и бордюра нет.
#[test]
fn a_paved_median_hands_back_its_double_solid_instead_of_painting_it() {
    let (drawing, layers) = drawn_medians(&avenue(0.6), true);
    assert_eq!(drawing.paved.len(), 1);
    assert!(drawing.lawn_kerbs.is_empty());
    assert!(!layers.streets.is_empty(), "асфальт между половинами");
    assert!(layers.sidewalks.is_empty() && layers.grass.is_empty());
    assert_eq!(drawing.painted.len(), 1, "{:?}", drawing.painted);
    let (midline, breaks, _) = &drawing.painted[0];
    // без перекрёстков — только торцы половин, нулевой длины
    assert!(
        breaks.iter().all(|gap| gap.reach == 0.0),
        "разрывы узлов без узлов: {breaks:?}"
    );
    assert!(polyline_length(midline) > 300.0, "осевая во всю пару");
    assert_eq!(drawing.ends.len(), 2);
    assert!(
        drawing
            .ends
            .iter()
            .all(|(_, end)| matches!(end, MedianEnd::Paved(_)))
    );
    // без разметки — ни осевой, ни торцов для слияний
    let (drawing, _) = drawn_medians(&avenue(0.6), false);
    assert!(drawing.painted.is_empty() && drawing.ends.is_empty());
    assert_eq!(drawing.paved.len(), 1, "асфальт лежит и без разметки");
}

/// Широкая — газон: бордюр в слое тротуаров, трава в своём, контуры
/// бордюра и торцы-носы — вызывающему; ни мощёных, ни краски.
#[test]
fn a_lawn_median_hands_back_its_kerbs_and_nose_ends() {
    let (drawing, layers) = drawn_medians(&avenue(8.0), true);
    assert!(drawing.paved.is_empty() && drawing.painted.is_empty());
    assert!(!drawing.lawn_kerbs.is_empty());
    assert!(!layers.sidewalks.is_empty() && !layers.grass.is_empty());
    assert!(!drawing.ends.is_empty());
    assert!(
        drawing
            .ends
            .iter()
            .all(|(_, end)| matches!(end, MedianEnd::Lawn(_)))
    );
    assert!(drawing.bed_caps().is_empty(), "полотна нет — и торцов его");
}

fn rect(min: Vec2, max: Vec2) -> Shape {
    let ring = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    vec![oriented(&ring, true)]
}

/// Шов асфальтовой разделительной с газонной (Советская в Туле, пример
/// 16): огрызок звена у торца, повёрнутый к общей точке, снимается, и
/// двойная сплошная идёт прямо до бордюра носа — без крюка.
#[test]
fn the_median_line_drops_the_seam_stub_and_reaches_the_nose() {
    let kerb = rect(Vec2::new(24.0, -2.0), Vec2::new(40.0, 2.0));
    let mut line = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.5, 0.3)];
    reach_nose(&mut line, &[kerb]);
    assert!(line.iter().all(|at| at.y.abs() < 1e-3), "крюк: {line:?}");
    let tip = *line.last().unwrap();
    assert!(
        (tip.x - (24.0 - PAINT_NOSE_CLEARANCE)).abs() <= NOSE_PROBE + 1e-3,
        "линия кончается не у носа: {tip}"
    );
    // начало, перед которым газона нет, — как было
    assert_eq!(line[0], Vec2::ZERO);
}

#[test]
fn a_median_line_without_a_lawn_ahead_stays_as_it_was() {
    let far = rect(Vec2::new(40.0, -2.0), Vec2::new(60.0, 2.0));
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.5, 0.3)];
    let mut line = original.clone();
    reach_nose(&mut line, &[far]);
    assert_eq!(line, original);
}

/// Между кромками половин всё, что не газон, — асфальт: срезанный носом
/// клин и кусок у перекрёстка, до которого газон не доходит.
#[test]
fn what_the_lawn_leaves_between_the_edges_is_asphalt() {
    let ring = oriented(
        &[
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 4.0),
            Vec2::new(0.0, 4.0),
        ],
        true,
    );
    let kerb = rect(Vec2::new(10.0, 0.0), Vec2::new(40.0, 4.0));
    let cuts = uncovered(ring, vec![kerb]);
    let area: f32 = cuts.iter().map(shape_area).sum();
    // бордюр раздут на `CUT_MARGIN`
    assert!(
        (area - (10.0 - CUT_MARGIN) * 4.0).abs() < 0.1,
        "асфальта {area} м²"
    );
}

/// Разделительная в 4 м между трёхполосными половинами (10.9 м): оси
/// разнесены на 14.9 м. Поперечная двухполосная улица рвёт обе оси на
/// 4.8 м — друг против друга, и разделительную она открывает; с одной
/// шириной разделительной (4 + 4.8 + 4.8 < 14.9) разрывы друг друга не
/// видели. Улица, примыкающая к одной половине в 40 м от перекрёстка, —
/// нет.
#[test]
fn breaks_face_each_other_across_the_axes_of_wide_halves() {
    let at = |y: f32| vec![Vec2::new(0.0, y), Vec2::new(100.0, y)];
    let median = Median::lawn_for_test(at(0.0), [at(-2.0), at(2.0)]);
    let halves = [10.9 / 2.0; 2];
    let cross = |y: f32| Break {
        at: Vec2::new(50.0, y),
        reach: 4.8,
    };
    let aside = Break {
        at: Vec2::new(90.0, -7.45),
        reach: 4.8,
    };
    let found = crossing_breaks(&median, halves, [&[cross(-7.45), aside], &[cross(7.45)]]);
    assert_eq!(found, vec![cross(-7.45), cross(7.45)]);
}

/// Половины 10.9 и 7.6 м, оси в 10.8 м: середина между осями в 5.4 от
/// широкой, и её кромка (5.45) зашла за середину. Контур асфальта кроет
/// зазор между кромками целиком и заходит под обе ленты; уводимая от
/// середины, кромка широкой шла под узкую, и зазор светился землёй
/// (Орёл, витрина 05).
#[test]
fn the_paved_fill_covers_the_gap_when_the_wide_edge_crosses_the_midline() {
    let xs = [0.0, 20.0];
    let at = |y: f32| xs.map(|x| Vec2::new(x, y)).to_vec();
    let midline = at(5.4);
    let inner = [at(5.45), at(7.0)];
    let ring = edges_ring(&midline, &inner, FILL_OVERLAP).expect("кромки есть");
    let (low, high) = ring
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), p| {
            (low.min(p.y), high.max(p.y))
        });
    assert!(
        low <= 5.45 - FILL_OVERLAP + 1e-3 && high >= 7.0 + FILL_OVERLAP - 1e-3,
        "контур {low}..{high} не кроет зазор 5.45..7.0 с нахлёстом"
    );
}

/// Разрыв краски со стоп-линией и зеброй — переходом шириной в зебру там,
/// где она пересекает ось; разрыв без зебры остаётся как был.
#[test]
fn a_paint_break_with_a_zebra_becomes_a_zebra_wide_crossing() {
    let zebra = Zebra {
        from: Vec2::new(12.0, -6.0),
        to: Vec2::new(12.0, 6.0),
        osm: true,
    };
    let cut = [
        Break {
            at: Vec2::new(10.0, 0.0),
            reach: 5.5,
        },
        Break {
            at: Vec2::new(60.0, 0.0),
            reach: 3.0,
        },
    ];
    let (crossings, rest) = split_zebras(&cut, &[zebra]);
    assert_eq!(
        crossings,
        vec![(
            Break {
                at: Vec2::new(12.0, 0.0),
                reach: ZEBRA_LENGTH / 2.0
            },
            zebra
        )]
    );
    assert_eq!(rest, vec![cut[1]]);
}

/// Луч от торца вверх: первая ось поперёк него — связка в 6 м; ось за
/// пределом поиска и параллельная лучу — не в счёт.
#[test]
fn the_closing_reach_is_the_first_axis_across_the_ray() {
    let across = |y: f32| vec![Vec2::new(-5.0, y), Vec2::new(5.0, y)];
    let parallel = vec![Vec2::new(3.0, 0.0), Vec2::new(3.0, 20.0)];
    let paths = [across(9.0), across(6.0), parallel.clone()];
    let found = closing_reach(Vec2::ZERO, Vec2::Y, 12.0, paths.iter().map(Vec::as_slice));
    assert!((found.unwrap() - 6.0).abs() < 1e-4, "{found:?}");
    let far = [across(15.0), parallel];
    assert_eq!(
        closing_reach(Vec2::ZERO, Vec2::Y, 12.0, far.iter().map(Vec::as_slice)),
        None
    );
}

/// Разделительная кончилась, где кончились пробы пары, а в 6 м впереди
/// зазор замыкает связка между половинами (Рязань, витрина 03, Вокзальная
/// над Первомайским): карман между торцом и связкой — асфальт, шире кромок
/// и до оси связки. Торец с перекрёстком на ходу — нет: зазор там
/// открывает перекрёсток.
#[test]
fn a_pocket_between_the_median_end_and_a_closing_link_is_asphalt() {
    let at = |y: f32| vec![Vec2::new(0.0, y), Vec2::new(20.0, y)];
    let median = Median::lawn_for_test(at(0.0), [at(-2.5), at(2.5)]);
    // замыкает только впереди конца, не начала
    let closer = |_: Vec2, heading: Vec2, _: f32| (heading.x > 0.0).then_some(6.0);
    let caps = end_caps(&median, &[], &closer);
    let [cap] = caps.as_slice() else {
        panic!("один карман: {caps:?}");
    };
    for inside in [
        Vec2::new(20.5, 0.0),
        Vec2::new(25.9, 0.0),
        Vec2::new(23.0, 2.4),
        Vec2::new(23.0, -2.4 - CAP_WIDER),
    ] {
        assert!(point_in_shape(inside, cap), "{inside} вне кармана {cap:?}");
    }
    let beside = Vec2::new(23.0, 2.6 + CAP_WIDER);
    for outside in [Vec2::new(26.5, 0.0), beside, Vec2::new(19.0, 0.0)] {
        assert!(!point_in_shape(outside, cap), "{outside} в кармане {cap:?}");
    }
    // перекрёсток на ходу торца — кармана нет
    let junction = Break {
        at: Vec2::new(24.0, 0.0),
        reach: 5.0,
    };
    assert!(end_caps(&median, &[junction], &closer).is_empty());
    // разрыв сбоку, на оси чужой половины за кромкой — не перекрёсток торца
    let aside = Break {
        at: Vec2::new(21.0, 9.0),
        reach: 5.0,
    };
    assert_eq!(end_caps(&median, &[aside], &closer).len(), 1);
}

/// Обрывок осевой между разрывом узла (он лежит на оси половины, в трёх
/// метрах сбоку) и торцом — тоже разрыв; длинный кусок по другую сторону
/// остаётся, осевая без разрывов — тоже.
#[test]
fn a_median_stub_past_a_break_is_bridged() {
    let midline = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let mut breaks = vec![Break {
        at: Vec2::new(14.0, 3.0),
        reach: 5.0,
    }];
    bridge_short_pieces(&midline, &mut breaks);
    assert_eq!(breaks.len(), 2, "{breaks:?}");
    let stub = breaks[1];
    assert!(
        stub.at.x - stub.reach <= 19.0 && stub.at.x + stub.reach >= 20.0,
        "{stub:?}"
    );
    let mut none = Vec::new();
    bridge_short_pieces(&[Vec2::ZERO, Vec2::new(4.0, 0.0)], &mut none);
    assert!(none.is_empty());
}

#[test]
fn paired_halves_share_a_paved_median_and_keep_sidewalks_outside() {
    let (map, apart) = divided_avenue(0.6);
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [1, 0, 0]);
    let middle = 100.0 + apart / 2.0;
    // двойная сплошная — по середине между половинами
    let axes = layer(&layers, paint::PAINT_AXES)
        .builder
        .positions_for_test();
    assert!(!axes.is_empty(), "двойная сплошная есть");
    assert!(
        axes.iter().all(|at| (at[1] - middle).abs() < 1.5),
        "осевые только по середине"
    );
    // под зазором — асфальт, а не тротуар
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    let half = (3.0 * 3.3 + 1.0) / 2.0;
    let (low, high) = (100.0 + half, 100.0 + apart - half);
    assert!(
        sidewalks
            .iter()
            .all(|at| at[1] <= low + 0.01 || at[1] >= high - 0.01),
        "тротуар со стороны пары"
    );
    assert!(
        sidewalks.iter().any(|at| at[1] < 100.0 - half - 1.0),
        "а с внешней стороны он есть"
    );
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (at[1] - middle).abs() < 0.01 && at[0] > 150.0)
    );
    assert!(layer(&layers, "road_medians").builder.is_empty());
}

/// Двойная сплошная доходит до перекрёстка так же, как линии полос: пробы пары
/// теряют соседа за несколько метров до узла, и середина кончалась там (отчёт
/// автора, пример 2 витрины).
#[test]
fn the_double_line_reaches_the_junction_like_the_lane_lines() {
    let (mut map, apart) = divided_avenue(0.6);
    for road in &mut map.roads {
        road.points.insert(1, Vec2::new(300.0, road.points[0].y));
    }
    map.roads.push(fixture::street(
        vec![
            Vec2::new(300.0, 40.0),
            Vec2::new(300.0, 100.0),
            Vec2::new(300.0, 100.0 + apart),
            Vec2::new(300.0, 170.0),
        ],
        12.0,
    ));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let nearest_before = |name: &str| {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|at| at[0] < 300.0 && (at[1] - 100.0 - apart / 2.0).abs() < apart)
            .map(|at| at[0])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let (axis, lanes) = (
        nearest_before(paint::PAINT_AXES),
        nearest_before(paint::PAINT_LANES),
    );
    assert!(axis.is_finite() && lanes.is_finite());
    // полоса краски выходит за видимый конец линии — гасит её шейдер по
    // «до разрыва», — так что двойная обязана доходить не меньше линий полос
    assert!(
        axis > lanes - 0.5,
        "двойная кончается у x = {axis}, линии полос — у x = {lanes}"
    );
}

/// Улица, примыкающая только к ближней половине, разделительную не открывает:
/// двойная сплошная идёт мимо узла без разрыва (пример 12 витрины).
#[test]
fn a_street_into_one_half_does_not_open_the_median() {
    let (mut map, _) = divided_avenue(0.6);
    map.roads[0].points.insert(1, Vec2::new(300.0, 100.0));
    map.roads.push(fixture::street(
        vec![Vec2::new(300.0, 40.0), Vec2::new(300.0, 100.0)],
        7.6,
    ));
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(report.junctions.count > 0, "узел у ближней половины есть");
    let axes = &layer(&layers, paint::PAINT_AXES).builder;
    let positions = axes.positions_for_test();
    assert!(positions.iter().any(|at| at[0] < 200.0) && positions.iter().any(|at| at[0] > 400.0));
    // разрыв — в «до разрыва» полосы краски: внутри него оно отрицательно
    // осевая самой примыкающей улицы лежит ниже половин — её не считаем, как
    // и торцы: там кончаются обе половины, и их торцы друг против друга
    assert!(
        axes.ribbon_coords_for_test()
            .expect("у краски атрибут есть")
            .iter()
            .zip(positions)
            .filter(|(_, at)| at[1] > 100.0 && (150.0..450.0).contains(&at[0]))
            .all(|(coords, _)| coords[2] > 0.0),
        "двойная сплошная рвётся у узла одной половины"
    );
}

#[test]
fn a_wide_gap_between_halves_is_a_lawn_with_a_kerb() {
    let (map, apart) = divided_avenue(8.0);
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [0, 1, 0]);
    let inner = (3.0 * 3.3 + 1.0) / 2.0;
    let grass = layer(&layers, "road_medians").builder.positions_for_test();
    assert!(!grass.is_empty(), "газон есть");
    for at in grass {
        assert!(
            at[1] >= 100.0 + inner + MEDIAN_KERB - 0.05
                && at[1] <= 100.0 + apart - inner - MEDIAN_KERB + 0.05,
            "газон заходит на бордюр или полотно: {at:?}"
        );
    }
    // осевой краски у газона нет
    assert!(layer(&layers, paint::PAINT_AXES).builder.is_empty());
}

/// Газон за перекрёстком начинается у его носа, а не у первой вершины оси за
/// разрывом: вершины прямого проспекта стоят в десятках метров, и газона на
/// всём пролёте не было — голый асфальт без осевой (Калуга, витрина 02).
#[test]
fn a_lawn_starts_at_its_nose_past_a_crossing() {
    // газон чуть шире асфальтовой разделительной, поперечная — проспект
    let (mut map, apart) = divided_avenue(3.4);
    map.roads.push(RoadLine {
        highway: Highway::Primary,
        lanes: Some(4),
        ..fixture::street(
            vec![
                Vec2::new(300.0, 40.0),
                Vec2::new(300.0, 100.0),
                Vec2::new(300.0, 100.0 + apart),
                Vec2::new(300.0, 160.0 + apart),
            ],
            14.2,
        )
    });
    map.roads[0].points.insert(1, Vec2::new(300.0, 100.0));
    map.roads[1]
        .points
        .insert(1, Vec2::new(300.0, 100.0 + apart));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let grass = layer(&layers, "road_medians").builder.positions_for_test();
    let nearest = grass
        .iter()
        .map(|at| at[0])
        .filter(|&x| x > 300.0)
        .fold(f32::INFINITY, f32::min);
    // нос — за зеброй и стоп-линией, но не за полтысячи метров у конца оси
    assert!(
        nearest < 335.0,
        "газон за перекрёстком начинается у x = {nearest}"
    );
    // и через сам перекрёсток не идёт: у двух вершин оси по краям проспекта
    // обе вне разрыва, и газон лежал одним куском поперёк поперечной
    assert!(
        grass.iter().all(|at| (at[0] - 300.0).abs() > 5.0),
        "газон на перекрёстке"
    );
}

/// Проспект с газоном в 8 м и двумя зебрами `crossing:island=yes` поперёк
/// у x = 300 и 306 — по узлу на каждой половине, дорожка через оба.
/// Возвращает карту и расстояние между осями.
fn island_zebra_avenue() -> (MapData, f32) {
    let (mut map, apart) = divided_avenue(8.0);
    let [near, far] = [Vec2::new(300.0, 100.0), Vec2::new(306.0, 100.0 + apart)];
    map.roads[0].points.insert(1, near);
    map.roads[1].points.insert(1, far);
    // переход — узел дорожки поперёк: без неё точка на одном way не узел
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::street(
            vec![near - Vec2::Y * 12.0, near, far, far + Vec2::Y * 12.0],
            3.5,
        )
    });
    let mut map = with_network(map.roads);
    for pos in [near, far] {
        map.road_nodes.push(RoadNode {
            pos,
            kind: RoadNodeKind::Crossing {
                signals: true,
                island: true,
                marked: true,
            },
            osm_id: None,
        });
    }
    (map, apart)
}

/// Две зебры `crossing:island=yes` посреди квартала, со сдвигом вдоль оси
/// (Вокзальная в Рязани, витрина 03): газон разделительной не рвётся на
/// разрывы краски и не пропадает, а прорезан проходом — травы нет только
/// на ширину зебры, а бордюр (плитка островка) идёт через проход насквозь.
#[test]
fn island_zebras_cut_a_passage_through_the_lawn_instead_of_dropping_it() {
    let (map, apart) = island_zebra_avenue();
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let grass = layer(&layers, "road_medians").builder.positions_for_test();
    // трава подходит к проходу вплотную с обеих сторон, а не носом за
    // десять метров до разрыва краски
    let before = grass
        .iter()
        .map(|at| at[0])
        .filter(|&x| x < 300.0)
        .fold(f32::MIN, f32::max);
    let after = grass
        .iter()
        .map(|at| at[0])
        .filter(|&x| x > 300.0)
        .fold(f32::MAX, f32::min);
    assert!(before > 296.5, "газон до перехода кончается у x = {before}");
    assert!(after < 303.5, "газон за переходом начинается у x = {after}");
    // и не на самом проходе: контур травы обходит его по краям
    let middle = 100.0 + apart / 2.0;
    assert!(
        grass.iter().all(|at| (at[0] - 300.0).abs() > 1.5),
        "трава на проходе"
    );
    // бордюр островка — через проход
    let kerb = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(
        kerb.iter()
            .any(|at| (at[1] - middle).abs() < apart / 2.0 - 5.0 && at[0] < 298.0)
            && kerb
                .iter()
                .any(|at| (at[1] - middle).abs() < apart / 2.0 - 5.0 && at[0] > 302.0),
        "бордюр островка по обе стороны прохода"
    );
}

/// Тот же зазор, но по нему идёт трамвай (Советская в Туле): газона нет,
/// половины расширяются до середины асфальтом полотна, двойная сплошная —
/// по середине, между путями, а над рельсами — светлая полоса.
#[test]
fn a_tram_between_halves_widens_both_halves_to_the_middle() {
    let (mut map, apart) = divided_avenue(5.0);
    let middle = 100.0 + apart / 2.0;
    for track in [middle - 1.7, middle + 1.7] {
        map.rails.push(RailLine {
            kind: RailKind::Tram,
            ..fixture::rail(vec![Vec2::new(80.0, track), Vec2::new(520.0, track)], 1.2)
        });
    }
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [1, 0, 1]);
    assert_eq!(report.tram_bands, 2, "полоса над каждым путём");
    assert!(
        layer(&layers, "road_medians").builder.is_empty(),
        "газона нет"
    );
    // и бордюра газона в слое тротуаров — тоже: между кромками пусто
    let inner = (3.0 * 3.3 + 1.0) / 2.0;
    let edges = [100.0 + inner, 100.0 + apart - inner];
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(
        sidewalks
            .iter()
            .filter(|at| at[0] > 200.0 && at[0] < 400.0)
            .all(|at| at[1] <= edges[0] + 0.01 || at[1] >= edges[1] - 0.01),
        "тротуар между половинами"
    );
    // асфальт полотна — от кромки до кромки, серым улиц, и светлая полоса
    let roads = &layer(&layers, "roads").builder;
    let road = ROAD_COLOR.to_linear().to_f32_array();
    let band = TRAM_BAND_COLOR.to_linear().to_f32_array();
    let bed: Vec<f32> = roads
        .positions_for_test()
        .iter()
        .zip(roads.colors_for_test())
        .filter(|(at, color)| **color == road && at[0] > 200.0 && at[0] < 400.0)
        .map(|(at, _)| at[1])
        .filter(|y| *y > edges[0] - 0.1 && *y < edges[1] + 0.1)
        .collect();
    assert!(
        bed.iter().any(|y| (y - edges[0]).abs() < 0.1)
            && bed.iter().any(|y| (y - edges[1]).abs() < 0.1),
        "асфальт полотна кроет зазор: {bed:?}"
    );
    let banded: Vec<f32> = roads
        .positions_for_test()
        .iter()
        .zip(roads.colors_for_test())
        .filter(|(_, color)| **color == band)
        .map(|(at, _)| at[1])
        .collect();
    assert!(!banded.is_empty(), "полоса над рельсами есть");
    assert!(
        banded
            .iter()
            .all(|y| (y - middle).abs() <= 1.7 + tram_band::TRAM_BAND_WIDTH / 2.0 + 0.01),
        "полоса — над путями: {banded:?}"
    );
    // двойная сплошная — по середине
    let axes = layer(&layers, paint::PAINT_AXES)
        .builder
        .positions_for_test();
    let centres: Vec<f32> = axes
        .chunks(2)
        .map(|pair| (pair[0][1] + pair[1][1]) / 2.0)
        .collect();
    assert!(!centres.is_empty());
    assert!(
        centres.iter().all(|centre| (centre - middle).abs() < 0.05),
        "двойная сплошная не по середине: {centres:?}"
    );
}

/// R24, Советская в Туле: пути OSM лежат на два метра южнее середины между
/// половинами и в 2.8 м друг от друга. Нарисованы они — и светлая полоса над
/// ними — симметрично от двойной сплошной, в [`tram_lay::TRACK_SPACING`]
/// друг от друга; за концом проспекта путь возвращается туда, где его провёл
/// картограф.
#[test]
fn tram_tracks_in_a_bed_lie_symmetric_about_the_drawn_middle() {
    let (mut map, apart) = divided_avenue(5.0);
    let middle = 100.0 + apart / 2.0;
    for track in [middle - 3.4, middle - 0.6] {
        map.rails.push(RailLine {
            kind: RailKind::Tram,
            ..fixture::rail(vec![Vec2::new(0.0, track), Vec2::new(520.0, track)], 1.2)
        });
    }
    let RoadBuild {
        layers,
        report,
        tram: tracks,
        ..
    } = build_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [1, 0, 1]);
    assert_eq!(tracks.0.len(), 2);
    let half = tram_lay::TRACK_SPACING / 2.0;
    // путь с запада на восток: высота у `x` — по звену, которое его проходит
    let y_at = |points: &[Vec2], x: f32| {
        points.windows(2).find_map(|pair| {
            (pair[0].x <= x && pair[1].x >= x && pair[1].x > pair[0].x).then(|| {
                pair[0]
                    .lerp(pair[1], (x - pair[0].x) / (pair[1].x - pair[0].x))
                    .y
            })
        })
    };
    for (track, expected) in tracks.0.iter().zip([middle - half, middle + half]) {
        for x in [200.0, 250.0, 300.0, 350.0, 400.0] {
            let y = y_at(&track.points, x).expect("путь проходит полотно");
            assert!(
                (y - expected).abs() < 0.05,
                "путь у x = {x} на {y}, а не на {expected}: {:?}",
                track.points
            );
        }
        // на своём полотне за концом проспекта — где был
        let start = track.points[0];
        assert!(
            (start.y - expected).abs() > 0.5,
            "путь за проспектом сдвинут: {start}"
        );
    }
    // светлая полоса над рельсами — симметрично от середины
    let band = vertices_of(&layers, "roads", TRAM_BAND_COLOR);
    assert!(!band.is_empty(), "полоса над рельсами есть");
    let (low, high) = band.iter().fold((f32::MAX, f32::MIN), |(low, high), at| {
        (low.min(at[1]), high.max(at[1]))
    });
    let reach = half + tram_band::TRAM_BAND_WIDTH / 2.0;
    assert!(
        (low - (middle - reach)).abs() < 0.1 && (high - (middle + reach)).abs() < 0.1,
        "полоса {low}..{high}, середина {middle}"
    );
}

/// Трамвай уходит с проспекта на полпути: полотно кончается, дальше газон.
/// Торец полотна — ровный, а между ним и носом газона — асфальт, не
/// тротуар и не земля (Советская у Коминтерна).
#[test]
fn a_tram_bed_ends_in_asphalt_up_to_the_nose_of_the_lawn() {
    let (map, apart) = tram_bed_then_lawn();
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [1, 1, 1], "полотно и газон");
    let inner = (3.0 * 3.3 + 1.0) / 2.0;
    let gap = |at: &&[f32; 3]| at[1] > 100.0 + inner + 0.1 && at[1] < 100.0 + apart - inner - 0.1;
    // асфальт заходит за торец полотна — к носу газона
    let roads = layer(&layers, "roads").builder.positions_for_test();
    let reach = roads
        .iter()
        .filter(gap)
        .map(|at| at[0])
        .filter(|x| (295.0..320.0).contains(x))
        .fold(f32::MIN, f32::max);
    assert!(reach > 302.0, "асфальт полотна кончается у торца: {reach}");
    // а трава газона на месте
    assert!(!layer(&layers, "road_medians").builder.is_empty());
}

/// Проспект с зазором 5 м из двух way на половину: до x = 300 между
/// половинами трамвай (полотно), дальше газон.
fn tram_bed_then_lawn() -> (MapData, f32) {
    let (map, apart) = divided_avenue(5.0);
    let split = |road: &RoadLine| -> [RoadLine; 2] {
        let [from, to] = [road.points[0], road.points[1]];
        let middle = Vec2::new(300.0, from.y);
        [
            RoadLine {
                points: vec![from, middle],
                ..road.clone()
            },
            RoadLine {
                points: vec![middle, to],
                ..road.clone()
            },
        ]
    };
    let mut map = MapData {
        roads: split(&map.roads[0])
            .into_iter()
            .chain(split(&map.roads[1]))
            .collect(),
        ..map
    };
    let middle = 100.0 + apart / 2.0;
    map.rails.push(RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(vec![Vec2::new(80.0, middle), Vec2::new(300.0, middle)], 1.2)
    });
    (map, apart)
}

/// Вершины слоёв, которых касается цикл разделительных, и число линий краски.
fn median_loop_counts(map: &MapData) -> (Vec<usize>, usize) {
    let (layers, report) = mesh_roads(
        map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let counts = ["roads", "sidewalks", "road_medians", paint::PAINT_AXES]
        .map(|name| layer(&layers, name).builder.vertex_count())
        .to_vec();
    (counts, report.paint_lines)
}

/// Пин перед переносом цикла разделительных в `medians::draw`: полотно,
/// газон, двойная сплошная, асфальт от торца полотна до носа — вершины
/// каждого слоя, которого цикл касается, в том же порядке пуша.
#[test]
fn the_median_loop_lays_the_same_vertices() {
    let (map, _) = tram_bed_then_lawn();
    assert_eq!(median_loop_counts(&map), (vec![217, 176, 28, 6], 9));
    // газон, прорезанный проходами по двум зебрам
    let map = island_zebra_avenue().0;
    assert_eq!(median_loop_counts(&map), (vec![132, 108, 36, 0], 16));
}

/// Полотно шире [`network::pairs::TRAM_BED_MAX_GAP`] — обособленное, на
/// траве: газон остаётся.
#[test]
fn a_tram_on_a_wide_median_keeps_the_lawn() {
    let (mut map, apart) = divided_avenue(12.0);
    let middle = 100.0 + apart / 2.0;
    map.rails.push(RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(vec![Vec2::new(80.0, middle), Vec2::new(520.0, middle)], 1.2)
    });
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [0, 1, 0]);
    assert_eq!(report.tram_bands, 0, "путь в траве — без полосы");
    assert!(!layer(&layers, "road_medians").builder.is_empty());
}

/// Разделительная открывается по **базовым** разрывам (`marking_breaks`), а
/// не по разрывам асфальта: половины ведут узел, и `NodePaint::asphalt` снял
/// с них разрыв — колея идёт сквозь, — а двойная сплошная у поперечной всё
/// равно рвётся. Перевести медианы на разрывы асфальта — провести её через
/// перекрёсток.
#[test]
fn the_median_base_keeps_the_break_a_leading_road_lost() {
    let (map, apart) = an_avenue_crossed_by_a_street();
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.medians, [1, 0, 0]);
    assert_eq!(report.junctions.leading, 2, "узел ведут обе половины");
    let middle = 100.0 + apart / 2.0;
    // двойная сплошная — вдоль середины; осевая поперечной на ней — полоса
    // поперёк, у самого x = 300, её вершины не в счёт
    let double: Vec<f32> = layer(&layers, paint::PAINT_AXES)
        .builder
        .positions_for_test()
        .iter()
        .filter(|at| (at[1] - middle).abs() < 2.0 && (at[0] - 300.0).abs() > 1.5)
        .map(|at| at[0])
        .collect();
    assert!(double.iter().any(|&x| x < 250.0) && double.iter().any(|&x| x > 350.0));
    let nearest = double
        .iter()
        .map(|x| (x - 300.0).abs())
        .fold(f32::INFINITY, f32::min);
    assert!(
        nearest > 4.0,
        "двойная сплошная через перекрёсток: {nearest}"
    );
}
