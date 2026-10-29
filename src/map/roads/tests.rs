use super::drawn::centerline;
use super::fixtures::{
    LAYERS, a_tee, an_avenue_crossed_by_a_street, ground_with_roads, junctions_with, layer,
    one_street, road, roundabout_with_an_approach, timeless, vertices_of, with_network,
};
use super::*;
use crate::map::footprint::casing_width;
use crate::map::osm::model::{
    Pavement, RailKind, RailLine, RoadNode, SIDEWALK_WIDTH_RANGE, SidewalkSide, sidewalk_band,
};
use crate::map::osm::{AreaKind, Highway, PolyArea, fixture};
use crate::map::parking::LOT_KERB;
use crate::map::smooth::Smoothing;

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

#[test]
fn casing_is_wider_than_the_fill() {
    // диапазоны самих ширин — тесты `map::footprint`; здесь — что кант
    // геометрически торчит из-под заливки
    let points = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let extent = |width: f32| {
        let mut builder = MeshBuilder::default();
        push_ribbon(
            &mut builder,
            &points,
            width,
            LinearRgba::WHITE,
            RoadJoin::Round,
        );
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let fill = 3.5;
    assert!(extent(fill + 2.0 * casing_width(fill)) > extent(fill));
}

#[test]
fn sidewalks_belong_to_streets_not_service_roads() {
    // проезд — без тротуара, как бы широк он ни был: решает класс, не ширина;
    // жилая улица и магистраль — с ним, в пределах диапазона
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut service = fixture::street(line.clone(), 8.0);
    service.highway = Highway::Service;
    assert_eq!(service.sidewalk().band(), None);
    let residential = fixture::street(line.clone(), 8.0)
        .sidewalk()
        .band()
        .unwrap();
    let mut primary = fixture::street(line, 16.0);
    primary.highway = Highway::Primary;
    let primary = primary.sidewalk().band().unwrap();
    assert!(residential < primary);
    assert!(SIDEWALK_WIDTH_RANGE.contains(&residential));
    assert!(SIDEWALK_WIDTH_RANGE.contains(&primary));
}

/// Ширины тротуара профиля: полоса по классу тег не смотрит (ей пользуются
/// обочина дома и бордюр стоянки), по карте — только при тротуаре хоть с
/// одной стороны; мост — проезжая часть, полоса у него есть (со своих слоёв
/// его убирает рендер, а не ширина).
#[test]
fn the_sidewalk_widths_read_the_class_the_tag_and_the_bridge() {
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut street = fixture::street(line.clone(), 8.0);
    let band = sidewalk_band(8.0);
    street.sidewalks = [SidewalkSide::None; 2];
    assert_eq!(street.sidewalk().band(), Some(band), "полоса по классу");
    assert_eq!(street.sidewalk().any(), None, "по карте — нет");
    assert_eq!(street.sidewalk().kerb(LOT_KERB), band);
    street.sidewalks = [SidewalkSide::None, SidewalkSide::Tagged];
    assert_eq!(street.sidewalk().any(), Some(band), "хоть с одной стороны");
    street.bridge = true;
    assert_eq!(street.sidewalk().band(), Some(band), "мост — с полосой");
    assert_eq!(street.sidewalk().any(), Some(band));
    let mut service = fixture::street(line, 8.0);
    service.highway = Highway::Service;
    assert_eq!(service.sidewalk().kerb(LOT_KERB), 1.2, "проезд — LOT_KERB");
}

#[test]
fn lanes_come_from_the_tag_and_fall_back_to_the_width() {
    let mut street = fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0);
    assert_eq!(
        lane_count(&street),
        2,
        "жилая улица без тега — по полосе в каждую сторону"
    );
    street.width = 12.0;
    assert_eq!(lane_count(&street), 4);
    street.lanes = Some(3);
    assert_eq!(lane_count(&street), 3, "тег важнее ширины");
    street.lanes = Some(8);
    assert_eq!(lane_count(&street), 4, "полоса у́же 2.5 м не бывает");
    street.lanes = None;
    street.oneway = true;
    street.width = 8.0;
    assert_eq!(lane_count(&street), 1, "односторонняя жилая — одна полоса");
    street.width = 16.0;
    assert_eq!(lane_count(&street), 3);
    street.roundabout = true;
    street.lanes = Some(2);
    assert_eq!(lane_count(&street), 2, "кольцо — как любая улица");
}

/// Кольцо узнаётся и **без тега** — по форме: замкнутое одностороннее полотно.
/// Большое кольцо у ТРЦ «Макси» (way 397005605) в OSM просто `oneway=yes`,
/// замкнутый сам на себя, и по одному тегу ни гладкой фигуры, ни островков
/// на подходах не получало бы.
#[test]
fn a_closed_oneway_way_is_a_roundabout_without_the_tag() {
    let corner = Vec2::new(-10.0, -10.0);
    let ring = vec![
        corner,
        Vec2::new(10.0, -10.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(-10.0, 10.0),
        corner,
    ];
    let mut road = fixture::street(ring.clone(), 12.0);
    road.oneway = true;
    assert!(road.is_roundabout());

    road.oneway = false;
    assert!(
        !road.is_roundabout(),
        "двусторонняя петля — не кольцевая развязка"
    );
    assert_eq!(lane_count(&road), 4);

    let mut open = fixture::street(ring[..4].to_vec(), 12.0);
    open.oneway = true;
    assert!(
        !open.is_roundabout(),
        "дуга кольца без тега — обычная улица"
    );
}

#[test]
fn lanes_come_with_the_carriageway() {
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut street = fixture::street(line.clone(), 8.0);
    assert_eq!(road_lanes(&street), Some(paint::lane_frame(2)));
    street.oneway = true;
    assert_eq!(
        road_lanes(&street),
        Some(paint::lane_frame(1)),
        "одна полоса: линий нет, а колея есть"
    );
    assert_eq!(road_lanes(&fixture::passage(line.clone(), 8.0)), None);
    let mut drive = fixture::street(line.clone(), 8.0);
    drive.highway = Highway::Service;
    assert_eq!(road_lanes(&drive), None, "у проезда полос нет");
    assert!(
        road_lanes(&fixture::bridge(line, 8.0)).is_some(),
        "мост несёт полосы своей улицы"
    );
}

#[test]
fn road_style_defaults_draw_sidewalks_and_markings() {
    let style = RoadStyle::default();
    assert!(style.sidewalks);
    assert!(style.markings);
}

fn fortress(outer: Vec<Vec2>) -> PolyArea {
    PolyArea {
        height: Some(12.0),
        ..fixture::area(AreaKind::Kremlin, outer)
    }
}

/// Лента кремлёвской стены не ложится поверх крепостных зданий: остаётся
/// только кусок в стороне от них, а огрызок между двумя зданиями пропадает.
#[test]
fn the_city_wall_ribbon_stays_off_fortress_buildings() {
    let wall = vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)];
    // стена-здание на первых ста метрах, башня на 104…114 — между ними 4 м
    let buildings = [
        fortress(fixture::rect(Vec2::new(-1.0, -2.0), Vec2::new(100.0, 2.0))),
        fortress(fixture::rect(Vec2::new(104.0, -5.0), Vec2::new(114.0, 5.0))),
    ];
    let runs = Fortresses::of(&buildings).bare_runs(&wall);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run = &runs[0];
    assert!(
        // точка на самой кромке башни ничья — кусок начинается не раньше неё
        run[0].x >= 114.0 && run.last().unwrap().x == 200.0,
        "{run:?}"
    );

    // без крепостных зданий лента целиком — единственный рисунок стены
    let alone = Fortresses::of(&[]).bare_runs(&wall);
    assert_eq!(alone, vec![wall.clone()]);
    // и короткая неразрезанная лента не пропадает
    let short = vec![Vec2::new(500.0, 0.0), Vec2::new(505.0, 0.0)];
    assert_eq!(Fortresses::of(&buildings).bare_runs(&short).len(), 1);
}

// --- слои целиком ------------------------------------------------------
//
// Тесты на `mesh_roads`. До шва девять слоёв, три вида материала и вся
// телеметрия области жили внутри `spawn_roads` — 275 строк, взять которые из
// теста было нечем: проверять можно было только хелперы под ними.

#[test]
fn a_street_builds_twenty_five_layers_bottom_up() {
    let (layers, report) = mesh_roads(
        &one_street(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    let names: Vec<&str> = layers.iter().map(|layer| layer.name).collect();
    assert_eq!(names, LAYERS);
    for pair in layers.windows(2) {
        // линии полос и осевые лежат на одной высоте: их полосы не
        // перекрываются, а прячет их ступень зума, не порядок
        let paint_pair = [pair[0].name, pair[1].name]
            .iter()
            .all(|name| paint::PaintTag::of(name).is_some());
        assert!(
            pair[0].z < pair[1].z || paint_pair && pair[0].z == pair[1].z,
            "{} лежит не ниже {}",
            pair[0].name,
            pair[1].name
        );
    }
    assert!(report.vertices > 0);
}

#[test]
fn only_the_bridge_shadow_is_blended() {
    let (layers, _) = mesh_roads(
        &one_street(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    // фактурный материал — у всего, что асфальт, тротуар или дорожка;
    // блендинг — ровно у полупрозрачной тени настила
    for layer in &layers {
        let expected = match layer.name {
            "bridge_shadows" => MaterialSpec::Blend,
            "sidewalks" | "lot_sidewalks" | "road_verges" => {
                MaterialSpec::Surface(SurfaceKind::Sidewalk)
            }
            "alleys" => MaterialSpec::Surface(SurfaceKind::Alley),
            "road_medians" | "ring_islands" | "ring_island_grass" | "road_verge_lawns" => {
                MaterialSpec::Surface(SurfaceKind::Grass)
            }
            "road_verge_yards" => MaterialSpec::Surface(SurfaceKind::Yard),
            "roads" | "bridges" => MaterialSpec::Surface(SurfaceKind::Street),
            "unpaved_roads" => MaterialSpec::Surface(SurfaceKind::Unpaved),
            paint::PAINT_WEAR_MASK => MaterialSpec::Paint(paint::PaintPass::WearMask),
            paint::PAINT_WEAR => MaterialSpec::Paint(paint::PaintPass::Wear),
            name if paint::PaintTag::of(name).is_some() => {
                MaterialSpec::Paint(paint::PaintPass::Lines)
            }
            _ => MaterialSpec::Flat,
        };
        assert_eq!(layer.material, expected, "{}", layer.name);
    }
}

/// Ручки формы доходят до геометрии: радиус угла растягивает скругления
/// бордюра, нулевой допуск оставляет ось по точкам OSM.
#[test]
fn the_shape_knobs_move_the_geometry() {
    let map = a_tee();
    let verts = |shape: RoadShape| {
        let (layers, report) = mesh_roads(
            &map,
            RoadStyle::default(),
            shape,
            VergeZoomBucket::for_zoom(0.0),
        );
        (layer(&layers, "roads").builder.vertex_count(), report)
    };
    let (_, base) = verts(RoadShape::default());
    assert!(base.kerb_returns > 0, "{base}");
    let (tight, _) = verts(RoadShape {
        corner_radius: 0.5,
        ..default()
    });
    let (wide, _) = verts(RoadShape {
        corner_radius: 2.0,
        ..default()
    });
    assert_ne!(tight, wide, "радиус угла меняет дуги скруглений");
}

#[test]
fn the_sidewalk_knob_fills_the_sidewalk_layer() {
    let map = one_street();
    let sidewalk_verts = |sidewalks| {
        let style = RoadStyle {
            sidewalks,
            ..RoadStyle::default()
        };
        let (layers, _) = mesh_roads(
            &map,
            style,
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        layer(&layers, "sidewalks").builder.vertex_count()
    };

    assert_eq!(sidewalk_verts(false), 0);
    assert!(sidewalk_verts(true) > 0);
}

#[test]
fn junctions_are_counted_with_markings_off_too() {
    // перекрёсток гасит и колею асфальта, а она есть без разметки
    assert_eq!(junctions_with(&a_tee(), false), 1);
    assert_eq!(junctions_with(&a_tee(), true), 1);
}

#[test]
fn a_crossing_without_a_shared_node_is_not_a_junction() {
    let mut map = one_street();
    // улица пересекает первую геометрически, но общей вершины у них нет —
    // так в OSM выглядит мост над улицей, и рвать линии он не должен
    map.roads.push(fixture::street(
        vec![Vec2::new(350.0, -100.0), Vec2::new(350.0, 400.0)],
        10.0,
    ));

    assert_eq!(
        junctions_with(&map, true),
        0,
        "перекрёсток восстанавливается по общей ноде, а не по пересечению"
    );
}

/// Грунтовка (`surface=unpaved|gravel|…`) уходит из асфальта в свой слой —
/// без линий краски, и угол двух грунтовок ложится грунтом. Та же Т с
/// асфальтовой улицей: угол — асфальтом, узел асфальтовый.
#[test]
fn an_unpaved_street_draws_in_its_own_layer_without_lines() {
    let unpave = |mut map: MapData, which: &[usize]| {
        for &index in which {
            map.roads[index].pavement = Some(Pavement::Unpaved);
            map.roads[index].sidewalks = [SidewalkSide::None; 2];
        }
        map
    };
    let (layers, report) = mesh_roads(
        &unpave(a_tee(), &[0, 1]),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(!layer(&layers, "unpaved_roads").builder.is_empty());
    assert_eq!(report.paint_lines, 0, "{report}");
    assert!(report.kerb_returns > 0, "углы грунтом: {report}");

    let (mixed, _) = mesh_roads(
        &unpave(a_tee(), &[1]),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(!layer(&mixed, "roads").builder.is_empty());
    assert!(!layer(&mixed, "unpaved_roads").builder.is_empty());
}

/// Асфальтовая улица, упёршаяся в грунтовку, кончается на её кромке: ни
/// лента, ни скругления не заходят на грунт (Тула, 13: торец лежал языком до
/// оси грунтовки).
#[test]
fn an_asphalt_street_stops_at_the_edge_of_the_dirt_road_it_meets() {
    let mut map = a_tee();
    map.roads[0].pavement = Some(Pavement::Unpaved);
    map.roads[0].sidewalks = [SidewalkSide::None; 2];
    map.roads[1].sidewalks = [SidewalkSide::None; 2];
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let asphalt = layer(&layers, "roads").builder.positions_for_test();
    assert!(!asphalt.is_empty());
    // грунтовка по y = 100 шириной 12 — кромка на 106
    let lowest = asphalt.iter().map(|at| at[1]).fold(f32::INFINITY, f32::min);
    assert!(
        lowest > 106.0 - 0.1,
        "асфальт заходит на грунт до y = {lowest}"
    );
}

/// Грунтовка, упёршаяся в асфальт, входит в его кромку как есть: асфальт идёт
/// прямо, без скруглений к ней, и грунт не расходится веером (Калуга, 07).
#[test]
fn a_dirt_road_enters_the_asphalt_without_kerb_returns() {
    let mut map = a_tee();
    map.roads[1].pavement = Some(Pavement::Unpaved);
    for road in &mut map.roads {
        road.sidewalks = [SidewalkSide::None; 2];
    }
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    // асфальт по y = 100 шириной 12 — кромка на 106
    let asphalt = layer(&layers, "roads").builder.positions_for_test();
    let highest = asphalt
        .iter()
        .map(|at| at[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        highest < 106.0 + 0.1,
        "асфальт уходит к грунтовке до y = {highest}"
    );
    // грунтовка по x = 350 шириной 10
    let dirt = layer(&layers, "unpaved_roads").builder.positions_for_test();
    assert!(!dirt.is_empty());
    for at in dirt {
        assert!(
            (at[0] - 350.0).abs() < 5.0 + 0.1,
            "грунт веером до x = {}",
            at[0]
        );
    }
}

/// Асфальт, продолженный грунтовкой, обрывается поперёк: ни его круглый торец
/// не ложится на грунт, ни грунтовый — на асфальт. Излом в 11° — чтобы шов
/// был в сглаживаемой оси (Калуга, 08: дугой сквозь шов узел уходил с оси, и
/// торцы оставались круглыми).
#[test]
fn asphalt_turning_into_dirt_ends_square() {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 100.0), Vec2::new(300.0, 100.0)],
        10.0,
    ));
    map.roads.push(fixture::street(
        vec![Vec2::new(300.0, 100.0), Vec2::new(600.0, 160.0)],
        10.0,
    ));
    map.roads[1].pavement = Some(Pavement::Unpaved);
    for road in &mut map.roads {
        road.sidewalks = [SidewalkSide::None; 2];
    }
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let reach = |name: &str, fold: fn(f32, f32) -> f32, from: f32| {
        let positions = layer(&layers, name).builder.positions_for_test();
        assert!(!positions.is_empty(), "{name}");
        positions.iter().map(|at| at[0]).fold(from, fold)
    };
    let asphalt = reach("roads", f32::max, f32::NEG_INFINITY);
    let dirt = reach("unpaved_roads", f32::min, f32::INFINITY);
    // прямые торцы режутся по биссектрисе излома — полметра за узлом с
    // наружной стороны; круглый торец ушёл бы на полуширину, 5 м
    assert!(
        asphalt < 300.0 + 1.0,
        "асфальт заходит на грунт до x = {asphalt}"
    );
    assert!(
        dirt > 300.0 - 1.0,
        "грунт заходит под асфальт до x = {dirt}"
    );
}

/// Мощёная дорожка ложится плиткой в слой тротуаров, грунтовая и дорожка без
/// решения (собранная руками) — песчаной тропинкой в слой дорожек; со
/// скруглением их узла — так же.
#[test]
fn a_paved_path_is_drawn_in_the_sidewalk_layer() {
    let cross = |pavement: Option<Pavement>| {
        let mut map = MapData::default();
        for points in [
            // Т-узел: общая вершина — скругления в нём
            vec![
                Vec2::new(0.0, 100.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(200.0, 100.0),
            ],
            vec![Vec2::new(100.0, 100.0), Vec2::new(100.0, 200.0)],
        ] {
            map.roads.push(RoadLine {
                pavement,
                ..road(points, 3.5, false)
            });
        }
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        [
            layer(&layers, "alleys").builder.vertex_count(),
            layer(&layers, "sidewalks").builder.vertex_count(),
        ]
    };
    for pavement in [None, Some(Pavement::Unpaved)] {
        let [alleys, sidewalks] = cross(pavement);
        assert!(
            alleys > 0 && sidewalks == 0,
            "{pavement:?}: {alleys} / {sidewalks}"
        );
    }
    let [alleys, sidewalks] = cross(Some(Pavement::Paved));
    assert!(alleys == 0 && sidewalks > 0, "{alleys} / {sidewalks}");
}

#[test]
fn an_empty_map_still_describes_every_layer() {
    let (layers, report) = mesh_roads(
        &MapData::default(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    assert_eq!(layers.len(), LAYERS.len());
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
    assert_eq!(report.vertices, 0);
}

fn extent_x(builder: &MeshBuilder) -> (f32, f32) {
    builder
        .positions_for_test()
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), at| {
            (low.min(at[0]), high.max(at[0]))
        })
}

#[test]
fn a_road_through_a_big_lot_is_drawn_over_it_with_a_kerb() {
    let (layers, _) = mesh_roads(
        &ground_with_roads(100.0),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    // бордюр — только у сквозной дороги и только у площадки: за её контур он
    // выпущен на два метра, до тротуара улицы, и не дальше
    let kerb = &layer(&layers, "lot_sidewalks").builder;
    let (low, high) = extent_x(kerb);
    assert!(
        (97.9..98.5).contains(&low) && (201.5..=202.1).contains(&high),
        "{low}..{high}"
    );
    // проезд ряда прорезал в бордюре устье, асфальт самой дороги — полосу
    let middle = 150.0;
    for at in kerb.positions_for_test() {
        assert!(
            (at[0] - middle).abs() > 1.5,
            "бордюр в устье проезда: {at:?}"
        );
        assert!((at[1] - middle).abs() > 2.0, "бордюр на полотне: {at:?}");
    }
    // дорога одна — разделительной нет
    assert!(layer(&layers, "lot_lines").builder.is_empty());
    // сама улица в своём слое осталась целой: за площадкой её рисует он
    let (low, high) = extent_x(&layer(&layers, "roads").builder);
    assert!(low < 1.0 && high > 299.0, "{low}..{high}");
}

#[test]
fn a_small_lot_still_hides_every_road_on_it() {
    // 60 × 60 — двор: его асфальт и есть проезд, поверх него ничего не кладётся
    let (layers, _) = mesh_roads(
        &ground_with_roads(60.0),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
    assert!(layer(&layers, "lot_lines").builder.is_empty());
}

#[test]
fn the_markings_knob_takes_the_hatching_off() {
    let style = RoadStyle {
        markings: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(
        &roundabout_with_an_approach(true, true),
        style,
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

#[test]
fn the_sidewalk_knob_takes_the_kerb_off_the_lot_road_too() {
    let style = RoadStyle {
        sidewalks: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(
        &ground_with_roads(100.0),
        style,
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
}

/// Зебра по правилу — на плече узла двух улиц с тротуарами **по тегу**
/// (`RoadLine::sidewalks`, как карман у `pockets::kerb_parking`), а не по
/// ручке «Sidewalks»: та лишь прячет ленту, у зебры своя ручка `crossings`.
#[test]
fn rule_zebras_do_not_follow_the_sidewalk_knob() {
    let mut map = a_tee();
    // зебра по правилу — только у улицы не ниже `tertiary`
    map.roads[0].highway = Highway::Tertiary;
    let zebras = |sidewalks| {
        let style = RoadStyle {
            sidewalks,
            ..RoadStyle::default()
        };
        mesh_roads(
            &map,
            style,
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        )
        .1
        .junctions
        .zebras[0]
    };

    assert!(zebras(true) > 0, "у тройника есть зебра по правилу");
    assert_eq!(
        zebras(false),
        zebras(true),
        "ручка тротуаров зебру не трогает"
    );
}

/// `sidewalk=right` — полоса только справа по ходу (к югу у улицы на восток),
/// `separate` с обеих сторон — никакой.
#[test]
fn the_sidewalk_tag_picks_the_side() {
    let sidewalks_with = |sides: [bool; 2]| {
        let mut map = one_street();
        map.roads[0].sidewalks = sides.map(|present| {
            if present {
                SidewalkSide::Tagged
            } else {
                SidewalkSide::None
            }
        });
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        layer(&layers, "sidewalks")
            .builder
            .positions_for_test()
            .to_vec()
    };
    let half = 12.0 / 2.0;
    let right = sidewalks_with([false, true]);
    assert!(!right.is_empty());
    assert!(
        right.iter().all(|at| at[1] <= 100.0 + half + 0.01),
        "слева по ходу тротуара нет"
    );
    assert!(right.iter().any(|at| at[1] < 100.0 - half - 1.0));
    assert!(sidewalks_with([false, false]).is_empty());
}

/// `highway=turning_circle` на торце тупика — круг асфальта шире дороги.
#[test]
fn a_turning_circle_widens_the_dead_end() {
    let mut map = one_street();
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(600.0, 100.0),
        kind: RoadNodeKind::TurningCircle,
    });
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.turning_circles, 1);
    let radius = turning_radius(12.0);
    assert!(radius > 6.0);
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (at[1] - (100.0 + radius)).abs() < 0.01 && (at[0] - 600.0).abs() < 1.0)
    );
}

/// Пин перед `Drawn::sidewalk_on`: улица с тротуаром только слева (к северу)
/// и примыкание с юга — скругления тротуара только там, где он есть.
#[test]
fn a_one_sided_street_turns_its_sidewalk_only_on_its_side() {
    let mut main = fixture::street(vec![Vec2::new(100.0, 100.0), Vec2::new(500.0, 100.0)], 12.0);
    main.points.insert(1, Vec2::new(300.0, 100.0));
    main.sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
    let map = with_network(vec![
        main,
        fixture::street(vec![Vec2::new(300.0, 0.0), Vec2::new(300.0, 100.0)], 8.0),
        fixture::street(vec![Vec2::new(300.0, 100.0), Vec2::new(300.0, 200.0)], 8.0),
    ]);
    let (_, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    // четыре угла асфальта, а тротуар поворачивает только на северных
    assert_eq!([report.kerb_returns, report.sidewalk_returns], [4, 2]);
}

/// Стороны наружного угла лежат на **торцевых кромках лент**: прямой торец
/// идёт поперёк исходного крайнего звена оси ([`meshing::butt_direction`]),
/// а направление луча смотрит на первую вершину ленты после слияния, — у
/// улицы, разрезанной в узле с лёгким изломом, те расходятся, и веер по
/// направлению луча оставлял между собой и торцами клиновидную щель поперёк
/// асфальта (Тула, Фрунзе у съезда, N1).
#[test]
fn an_outer_corner_sides_lie_on_the_ribbon_butt_edges() {
    use crate::map::meshing::butt_direction;
    // улица ЮЮВ—ССВ, разрезанная в узле; у северного way первая вершина
    // через 1.5 м — общая с коротким проездом (пин оси), лента же сливает
    // её (четверть ширины 2 м), и направление луча — уже на следующую
    // вершину; съезд примыкает с ССЗ, узел — перекрёсток
    let node = Vec2::new(300.0, 100.0);
    let near = node + Vec2::from_angle(72.6_f32.to_radians()) * 1.5;
    let far = near + Vec2::from_angle(75.0_f32.to_radians()) * 20.0;
    let across = near + Vec2::new(-6.0, 6.0);
    let map = with_network(vec![
        fixture::street(
            vec![node + Vec2::from_angle(251.8_f32.to_radians()) * 20.0, node],
            8.0,
        ),
        fixture::street(vec![node, near, far], 8.0),
        fixture::street(
            vec![node + Vec2::from_angle(160.0_f32.to_radians()) * 15.0, node],
            8.0,
        ),
        fixture::street(vec![across, near], 5.0),
    ]);
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let returns = corners::kerb_returns(&prepared, 1.0);
    // торец северного way: кромка веера на нём — вторая сторона веера
    // (справа по ходу луча), узел, полуширина и нахлёст вдоль торцевого звена
    let axis = prepared.axis(1, Axis::Ribbon);
    let butt = butt_direction(axis, false).expect("у оси есть крайнее звено");
    let expected = node - butt.perp() * 4.0 + butt * corners::OUTER_OVERLAP;
    let streets: Vec<&[Vec2]> = returns.streets().outlines().collect();
    let found = streets
        .iter()
        .flat_map(|outline| outline.iter())
        .any(|point| point.distance(expected) < 0.02);
    assert!(
        found,
        "стороны веера — по торцевым кромкам: вершины {expected} нет ни в \
         одном {streets:?}"
    );
}

/// Планка зебры рвёт и **чужие** линии: разрыв получала только своя дорога,
/// и у Y-слияния двух односторонних в двустороннюю (Тула, у кольца, N6) ветки
/// начинались короткими косыми обрывками прямо на планке, стоящей в метрах
/// от узла слияния. Углы веток и главной — как у той развилки: въезд уходит
/// из узла на 121°, выезд на 72°, главная на 283°; переход стоит в узле
/// слияния, и зебра вытягивается на продолжение в 3 м от узла
#[test]
fn a_zebra_plank_cuts_the_lines_of_crossing_carriageways() {
    use crate::map::osm::model::{RoadNode, RoadNodeKind};
    let one_way = |points: Vec<Vec2>| RoadLine {
        oneway: true,
        lanes: Some(2),
        highway: Highway::Primary,
        ..fixture::street(points, 7.6)
    };
    let mut map = with_network(vec![
        RoadLine {
            highway: Highway::Primary,
            lanes: Some(4),
            ..fixture::street(vec![Vec2::new(18.0, -77.9), Vec2::new(0.0, 0.0)], 14.2)
        },
        one_way(vec![Vec2::new(-17.2, 28.5), Vec2::new(0.0, 0.0)]),
        one_way(vec![Vec2::new(0.0, 0.0), Vec2::new(12.4, 38.0)]),
    ]);
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(0.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
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
            crossings: CrossingMode::Osm,
            stop_lines: false,
        },
    );
    let paint = junctions.node_paint();
    // своя дорога планку режет
    let own = paint.lines().of(0).cut.len();
    assert!(own > 0, "у главной с зеброй разрыва нет");
    // узел веток — слияние: разрывов асфальта в узле нет (тупиковой с нулевой
    // досягаемостью на другом конце — можно), линии веток идут сквозь, и рвать
    // их могло только полотно главной через планку
    for road in [1, 2] {
        let axis = drawn.axis(road, Axis::Ribbon);
        assert!(
            !paint
                .asphalt()
                .of(road)
                .iter()
                .any(|found| { found.reach > 0.0 && found.at.distance(Vec2::ZERO) < 10.0 }),
            "узел ветки {road} — слияние, разрыва асфальта в нём быть не должно"
        );
        // разрыв от планки: недалеко от неё, короткий (узловой был бы в
        // полуширины главной) — и накрывает планку интервалом. Планка в узле
        // слияния вытягивается на продолжение: центр в 3 м от узла по главной
        let plank = Vec2::new(0.67, -2.92);
        let cut = paint
            .lines()
            .of(road)
            .cut
            .iter()
            .find(|found| found.reach < 5.0 && found.at.distance(plank) < 12.0)
            .unwrap_or_else(|| {
                panic!(
                    "у ветки {road} нет разрыва на планке: cuts {:?}",
                    paint.lines().of(road).cut
                )
            });
        let along = |point: Vec2| crate::map::along::nearest_on_path(axis, point).unwrap().1;
        let plank_along = along(plank);
        let cut_along = along(cut.at);
        assert!(
            cut_along - cut.reach <= plank_along + 0.05
                && plank_along <= cut_along + cut.reach + 0.05,
            "разрыв ветки {road} не накрывает планку: {cut:?}, планка на {plank_along}"
        );
    }
}

#[test]
fn an_arm_is_filled_before_its_leader_even_when_it_leads_elsewhere() {
    use node_paint::{Junction, JunctionArm, PaintJunctions};
    // 0 — главная, ведёт узел; 1 — примыкание шире неё, что само ведёт
    // другой узел дальше: по одному ключу «ведущие последними, узкие раньше»
    // оно ложилось бы на главную
    let widths = [5.0, 10.0, 8.0];
    let leading = [true, true, false];
    let arm = |road| JunctionArm {
        road,
        edge: 0.0,
        dir: 1.0,
        link: false,
    };
    let junctions = [Junction {
        arms: vec![arm(0), arm(0), arm(1)],
        leading: vec![0],
    }];
    assert_eq!(
        fill_order(&widths, &leading, PaintJunctions::of(&junctions)),
        vec![2, 1, 0]
    );
    // без узла — прежний ключ
    assert_eq!(
        fill_order(&widths, &leading, PaintJunctions::of(&[])),
        vec![2, 0, 1]
    );
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

#[test]
fn tram_band_follows_the_nodal_axis() {
    // Проезд кончается в трёх метрах за кромкой тротуара улицы и пришит к
    // ней стежком (`a_dangling_end_short_of_a_street_is_stitched`); путь
    // трамвая идёт по проезду и дальше, через улицу. Полоса над путём
    // берёт ось **без** стежка: она кончается у торца OSM (y = −9), а не у
    // оси улицы (y = 0), куда стежок довёл бы ленту.
    let mut map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 5.0),
    ]);
    map.rails.push(RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(vec![Vec2::new(50.0, -70.0), Vec2::new(50.0, 20.0)], 1.2)
    });
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.stitches, 1);
    assert_eq!(report.tram_bands, 1);
    let band = vertices_of(&layers, "roads", TRAM_BAND_COLOR);
    assert!(!band.is_empty());
    let top = band.iter().map(|at| at[1]).fold(f32::MIN, f32::max);
    assert!(
        top < -5.0,
        "полоса кончается у торца OSM, не у стежка: {top}"
    );
    assert!(top > -12.0, "{top}");
}

#[test]
fn a_one_sided_sidewalk_wedge_keeps_the_bare_kerb_on_the_untagged_side() {
    // Двухполосная улица переходит в четырёхполосную (`a_section_seam_is_one_taper`);
    // у широкой тротуар только слева по ходу (`sidewalk=left`, к северу).
    // Клин тротуара кладётся по сторонам: слева — от полосы узкой к своей,
    // справа — голая кромка от полуширины узкой к своей полуширине, а не
    // зеркало левой полосы.
    let street = |points: Vec<Vec2>, lanes: u8| RoadLine {
        lanes: Some(lanes),
        ..fixture::street(points, f32::from(lanes) * 3.3 + 1.0)
    };
    let mut wide = street(vec![Vec2::new(200.0, 0.0), Vec2::new(400.0, 0.0)], 4);
    wide.sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
    let map = with_network(vec![
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 2),
        wide,
    ]);
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.tapers, 1);
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    let (wide_half, wide_band) = (14.2 / 2.0, sidewalk_band(14.2));
    assert!((wide_band - 3.0).abs() < 1e-3);
    let bottom = sidewalks.iter().map(|at| at[1]).fold(f32::MAX, f32::min);
    assert!(
        bottom >= -wide_half - 0.05,
        "справа кромка голая, тротуара нет: {bottom}"
    );
    // слева клин доходит до своей полосы: полуширина плюс тротуар
    let top = sidewalks
        .iter()
        .filter(|at| at[0] > 200.0 && at[0] < 300.0)
        .map(|at| at[1])
        .fold(f32::MIN, f32::max);
    assert!((top - (wide_half + wide_band)).abs() < 0.05, "{top}");
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

/// Сырой OSM, второй уровень (`RawOsm::Draw`): улица — одна простая лента по
/// оси шириной из разбора, без тротуара, разметки и колеи; без флага та же
/// улица получает тротуар и краску.
#[test]
fn raw_osm_draw_lays_a_bare_ribbon_per_way() {
    use crate::map::osm::parse::RawOsm;
    let vertices = |layers: &[LayerMesh], name: &str| {
        layers
            .iter()
            .filter(|layer| layer.name == name)
            .map(|layer| layer.builder.vertex_count())
            .sum::<usize>()
    };
    let paint = |layers: &[LayerMesh]| {
        layers
            .iter()
            .filter(|layer| paint::PaintTag::of(layer.name).is_some())
            .map(|layer| layer.builder.vertex_count())
            .sum::<usize>()
    };

    let cooked = one_street();
    let (layers, _) = mesh_roads(
        &cooked,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(vertices(&layers, "sidewalks") > 0);
    assert!(paint(&layers) > 0);

    let mut raw = one_street();
    raw.knobs.raw = RawOsm::Draw;
    let (layers, report) = mesh_roads(
        &raw,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(vertices(&layers, "sidewalks"), 0);
    assert_eq!(paint(&layers), 0);
    // одна лента по полуширине от оси, с прямыми торцами ровно в концах way
    let road = layer(&layers, "roads").builder.positions_for_test();
    assert!(!road.is_empty() && road.len() <= 8, "{} вершин", road.len());
    assert!(road.iter().all(|at| (at[1] - 100.0).abs() < 6.01));
    assert!(road.iter().all(|at| (99.99..=600.01).contains(&at[0])));
    assert_eq!(report.kerb_returns, 0);
}

// DIAG-TEMP: `QWE_DIAG_JSON=<срез> QWE_DIAG_CITY=oryol QWE_DIAG=x,y,r QWE_DIAG_PROBES="x,y;x,y"`
#[test]
#[ignore]
fn diag_window() {
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
    if let Ok(spec) = std::env::var("QWE_DIAG") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let prepared = Drawn::new(
            &map,
            &RoadStyle::default(),
            &RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        for i in 0..prepared.len() {
            let d = prepared.road(i);
            let line = prepared.axis(i, Axis::Ribbon);
            if line.iter().any(|p| p.distance(at) < r) {
                eprintln!(
                    "DIAG road {i} hw={:?} class={:?} w={} oneway={} lanes={:?} bw={:?} osm={:?}\n    ribbon={:?}\n    nodal={:?}",
                    d.highway,
                    d.class,
                    d.width,
                    d.oneway,
                    d.lanes,
                    d.lanes_backward,
                    d.points,
                    line,
                    prepared.axis(i, crate::map::roads::Axis::Nodal)
                );
                eprintln!("DIAG   verges={:?} profile={:?}", d.verges, d.verge_profile);
            }
        }
        let returns = super::corners::kerb_returns(&prepared, 1.0);
        for (index, merge) in prepared.merges().list.iter().enumerate() {
            if merge.node.distance(at) < r * 2.0 {
                eprintln!(
                    "DIAG merge {index} node={:?} halves={:?} street={} street_end={} pure={}",
                    merge.node, merge.halves, merge.street, merge.street_end, merge.pure
                );
            }
        }
        for i in 0..prepared.len() {
            if prepared
                .axis(i, crate::map::roads::Axis::Ribbon)
                .iter()
                .any(|p| p.distance(at) < r)
            {
                eprintln!(
                    "DIAG is_merged road {i} = {:?}",
                    [prepared.is_merged(i, 0), prepared.is_merged(i, 1)]
                );
            }
        }
        returns.dump_near(at, r);
        for i in 0..prepared.len() {
            let b = returns.butt(i);
            if b != [false; 2]
                && prepared
                    .axis(i, Axis::Ribbon)
                    .iter()
                    .any(|p| p.distance(at) < r)
            {
                eprintln!("DIAG butt road {i} {b:?}");
            }
        }
        let nodes = prepared.nodes();
        for i in 0..prepared.len() {
            if prepared
                .axis(i, Axis::Ribbon)
                .iter()
                .any(|p| p.distance(at) < r)
            {
                let nodal = prepared.axis(i, crate::map::roads::Axis::Nodal);
                let shared = [0, nodal.len() - 1]
                    .map(|v| (nodes.is_shared(nodal[v]), nodes.roads_at(nodal[v]).to_vec()));
                eprintln!(
                    "DIAG ends road {i} stitched={:?} at={:?} offset={} shared={shared:?}",
                    prepared.stitched_end(i),
                    prepared.stitches().ends[i],
                    prepared.stitch_offset(i),
                );
            }
        }
    }
    if let Ok(spec) = std::env::var("QWE_DIAG_RING") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let near = |ring: &[Vec2]| ring.iter().any(|p| p.distance(at) < r);
        let layers: [(&str, &Vec<crate::map::osm::model::PolyArea>); 10] = [
            ("buildings", &map.buildings),
            ("water", &map.water),
            ("parks", &map.parks),
            ("woods", &map.woods),
            ("grass", &map.grass),
            ("sand", &map.sand),
            ("landuse", &map.landuse),
            ("parking", &map.parking),
            ("pitches", &map.pitches),
            ("pockets", &map.pockets),
        ];
        for (name, areas) in layers {
            for (i, area) in areas.iter().enumerate() {
                if near(&area.outer) {
                    eprintln!("DIAG ring {name}[{i}] kind={:?} outer={area:?}", area.kind);
                }
            }
        }
    }
    // DIAG-TEMP R37-01: та же кормилка лоскутов, что в mesh_roads, — окна
    // газонов и найденные куски у точки: `QWE_DIAG_SCRAPS=x,y,r`,
    // `QWE_DIAG_SCRAPS_FIND=1` добавляет вычитание мощения и куски
    if let Ok(spec) = std::env::var("QWE_DIAG_SCRAPS") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let style = RoadStyle::default();
        let shape = RoadShape::default();
        let prepared = Drawn::new(&map, &style, &shape, VergeZoomBucket::for_zoom(0.0));
        let ribbon: Vec<Vec<Vec2>> = (0..prepared.len())
            .map(|i| prepared.axis(i, Axis::Ribbon).to_vec())
            .collect();
        let returns = super::corners::kerb_returns(&prepared, shape.corner_radius());
        let meadows = super::Meadows::of(&map.parks, &map.grass);
        let mut scraps = super::scraps::Scraps::default();
        // окна газонов и асфальт узлов — тем же укладщиком, что и в
        // mesh_roads (`KerbReturns::lay`); билдеры слоёв выбрасываются,
        // важен вклад в лоскуты
        let mut streets = MeshBuilder::with_surface_coords();
        let mut alleys = MeshBuilder::with_surface_coords();
        let mut unpaved = MeshBuilder::with_surface_coords();
        let mut sidewalk = MeshBuilder::with_surface_coords();
        let mut verge = MeshBuilder::with_surface_coords();
        let mut tiles = MeshBuilder::with_surface_coords();
        let mut lawns = MeshBuilder::with_surface_coords();
        let mut yards = MeshBuilder::with_surface_coords();
        returns.lay(
            super::corners::KerbLayers {
                streets: &mut streets,
                alleys: &mut alleys,
                unpaved: &mut unpaved,
                sidewalks: &mut sidewalk,
                verges: &mut verge,
                lawns: &mut lawns,
                yards: &mut yards,
            },
            &meadows,
            &mut scraps,
        );
        let road_bites = super::scraps::Bites::of(prepared.nodes(), &prepared.roads());
        for index in 0..prepared.len() {
            let road = prepared.road(index);
            let mut verged = prepared.verges_drawn(index);
            if let Some(ring) = prepared
                .rings()
                .index_of(index)
                .and_then(|i| prepared.rings().list.get(i))
            {
                verged[usize::from(!ring.ccw)] = 0.0;
            }
            let bites = if verged[0] > 0.0 || verged[1] > 0.0 {
                ribbon[index][1..ribbon[index].len().saturating_sub(1)]
                    .iter()
                    .filter(|&point| road_bites.bite(index, *point))
                    .copied()
                    .collect()
            } else {
                Vec::new()
            };
            super::push_verges(
                [&mut tiles, &mut lawns, &mut yards],
                &meadows,
                road,
                &ribbon[index],
                returns.butt(index),
                &bites,
                verged,
                &mut scraps,
            );
        }
        for (index, (ring, cuts)) in scraps.lawns().enumerate() {
            let near = ring
                .iter()
                .chain(cuts.iter().flat_map(|cut| cut.iter()))
                .any(|p| p.distance(at) < r);
            if near {
                eprintln!("DIAG lawn window {index} ring={ring:?} cuts={cuts:?}");
            }
        }
        if std::env::var("QWE_DIAG_SCRAPS_FIND").is_ok() {
            // плоское мощение, клинья развилок, мощёные носы и дуги — тем же
            // порядком, что в mesh_roads
            for outline in returns
                .paving()
                .flat()
                .chain(returns.fork_gores().iter().map(Vec::as_slice))
                .chain(returns.paving().curved())
            {
                if scraps.near_lawn(outline, 0.0) {
                    scraps.push_paving(outline);
                }
            }
            for (index, axis) in ribbon.iter().enumerate() {
                let road = prepared.road(index);
                let paved = !road.is_unpaved_street()
                    && (road.class == crate::map::osm::RoadClass::Street || road.is_paved_path());
                if !paved || road.bridge {
                    continue;
                }
                let halves = [0, 1].map(|side| prepared.band_half(index, side));
                if scraps.near_lawn(axis, halves[0].max(halves[1])) {
                    scraps.push_band(axis, halves);
                }
            }
            for area in map.pockets.iter().chain(&map.grass) {
                scraps.push_green(area.outer.clone());
            }
            if std::env::var("QWE_DIAG_SCRAPS_ALL").is_ok() {
                for (ring, area, avg, open, tip) in scraps.debug_pieces() {
                    let Some(&near) = ring.iter().find(|p| p.distance(at) < r) else {
                        continue;
                    };
                    eprintln!(
                        "DIAG piece at {near} area={area:.1} avg={avg:.2} open={open:.1} tip={tip} ring={ring:?}"
                    );
                }
            }
            for shape in scraps.find_green() {
                let ring = crate::map::shapes::ring_of(&shape[0]);
                let Some(&tip) = ring.iter().find(|p| p.distance(at) < r) else {
                    continue;
                };
                eprintln!("DIAG green scrap at {tip} ring={ring:?}");
            }
            for shape in scraps.find() {
                let ring = crate::map::shapes::ring_of(&shape[0]);
                let Some(&tip) = ring.iter().find(|p| p.distance(at) < r) else {
                    continue;
                };
                eprintln!("DIAG scrap at {tip} ring={ring:?}");
            }
        }
    }
    let (mut layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    eprintln!("DIAG report {report}");
    let layout =
        crate::map::parking::ParkingLayout::new(&map.parking, &map.roads, map.traffic_side);
    layers.extend(crate::map::spawn::mesh_surfaces(&map, &layout).0);
    if let Ok(spec) = std::env::var("QWE_DIAG_MESH") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        for layer in &layers {
            let pos = layer.builder.positions_for_test();
            let near: Vec<Vec2> = pos
                .iter()
                .map(|p| Vec2::new(p[0], p[1]))
                .filter(|p| p.distance(at) < r)
                .collect();
            if !near.is_empty() {
                eprintln!("DIAG mesh {} {} {near:?}", layer.name, near.len());
            }
        }
    }
    if let Ok(spec) = std::env::var("QWE_DIAG_COLUMN") {
        let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let (from, to, step) = (Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), v[4]);
        let count = (from.distance(to) / step) as usize;
        for i in 0..=count {
            let at = from.lerp(to, i as f32 / count.max(1) as f32);
            let mut top: Vec<(f32, &str)> = layers
                .iter()
                .filter(|layer| {
                    layer
                        .builder
                        .triangles()
                        .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
                })
                .map(|layer| (layer.z, layer.name))
                .collect();
            top.sort_by(|a, b| b.0.total_cmp(&a.0));
            top.truncate(3);
            eprintln!("PROBE col {at} {top:?}");
        }
    }
    let spec = std::env::var("QWE_DIAG_PROBES").unwrap_or_default();
    for probe in spec.split(';').filter(|s| !s.is_empty()) {
        let v: Vec<f32> = probe.split(',').map(|s| s.parse().unwrap()).collect();
        let at = Vec2::new(v[0], v[1]);
        let only = std::env::var("QWE_DIAG_LAYER").unwrap_or_default();
        for layer in layers.iter().filter(|l| l.name.contains(only.as_str())) {
            let pos = layer.builder.positions_for_test();
            let coords = layer.builder.ribbon_coords_for_test();
            let col = layer.builder.colors_for_test();
            for [a, b, c] in layer.builder.triangles() {
                if crate::map::osm::model::point_in_polygon(at, &[a, b, c]) {
                    let info: Vec<_> = [a, b, c]
                        .iter()
                        .map(|p| {
                            let i = pos.iter().position(|q| Vec2::new(q[0], q[1]) == *p);
                            (
                                *p,
                                i.and_then(|i| coords.map(|c| c[i])),
                                i.map(|i| col[i][3]),
                            )
                        })
                        .collect();
                    eprintln!("PROBE {at} {} {:?}", layer.name, info);
                }
            }
        }
    }
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
