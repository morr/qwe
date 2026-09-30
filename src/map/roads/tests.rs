use super::network::pairs::Pairs;
use super::*;
use crate::map::footprint::casing_width;
use crate::map::meshing::distance_to_path;
use crate::map::osm::model::{
    KerbParking, Pavement, RailKind, RailLine, RoadNode, SIDEWALK_WIDTH_RANGE, SidewalkSide,
    sidewalk_band,
};
use crate::map::osm::{Highway, LotKind, fixture};
use crate::map::parking::LOT_KERB;

fn road(points: Vec<Vec2>, width: f32, passage: bool) -> RoadLine {
    RoadLine {
        class: RoadClass::Alley,
        passage,
        ..fixture::street(points, width)
    }
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

/// Двадцать пять дорожных слоёв снизу вверх, ровно в том порядке, в каком
/// они уходят в мир: газон островов колец и их трава без канта, газон широких
/// обочин лугом и травой двора, двенадцать лент и восемь слоёв краски над своим асфальтом — колея
/// траекторий узла (маска, потом наложение) ниже линий, островки колец над
/// асфальтом стоянок. Грунтовки — под асфальтом улиц, обочины — под всей
/// зеленью, их газон — под их плиткой, газон острова — под всем, его трава —
/// над замапленной травой. Настил переездов (`rail_crossings`) — между
/// шпалами и сталью путей, то есть над всей краской улиц и под мостами.
const LAYERS: [&str; 25] = [
    "ring_islands",
    "road_verge_lawns",
    "road_verge_yards",
    "road_verges",
    "ring_island_grass",
    "alleys",
    "sidewalks",
    "road_medians",
    "unpaved_roads",
    "roads",
    paint::PAINT_WEAR_MASK,
    paint::PAINT_WEAR,
    paint::PAINT_ZEBRAS,
    paint::PAINT_LANES,
    paint::PAINT_AXES,
    "lot_sidewalks",
    "lot_lines",
    paint::PAINT_ISLANDS,
    "rail_crossings",
    "bridge_shadows",
    "bridge_casings",
    "bridges",
    paint::BRIDGE_PAINT_LANES,
    paint::BRIDGE_PAINT_AXES,
    "walls",
];

fn one_street() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    map
}

fn layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a LayerMesh {
    layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("слой {name} описан на любом стиле"))
}

#[test]
fn a_street_builds_twenty_five_layers_bottom_up() {
    let (layers, report) = mesh_roads(&one_street(), RoadStyle::default(), RoadShape::default());

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

/// Обочина до дорожки: узкая — плиткой целиком, широкая — газоном до
/// дорожки с полосой плитки у бордюра (дворы Фрунзе, районный кадр d2).
#[test]
fn a_wide_verge_is_a_lawn_with_a_paved_kerb_strip() {
    assert_eq!(paved_verge(3.0), 3.0);
    assert_eq!(paved_verge(VERGE_PAVED_MAX), VERGE_PAVED_MAX);
    assert_eq!(paved_verge(15.0), VERGE_KERB);
    assert!(paved_verge(VERGE_PAVED_MAX + 1.0) < VERGE_PAVED_MAX);

    let verged = |verge: f32| {
        let mut map = one_street();
        map.roads[0].verges = [verge, 0.0];
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
        let reach = |name: &str| {
            layer(&layers, name)
                .builder
                .positions_for_test()
                .iter()
                .map(|at| at[1] - 100.0)
                .fold(0.0_f32, f32::max)
        };
        [reach("road_verges"), reach("road_verge_yards")]
    };
    // улица 12 м: кромка в 6 м от оси
    let [tiles, lawn] = verged(3.0);
    assert!(
        (tiles - 9.0).abs() < 0.05 && lawn == 0.0,
        "узкая: {tiles} / {lawn}"
    );
    let [tiles, lawn] = verged(12.0);
    assert!(
        (tiles - (6.0 + VERGE_KERB)).abs() < 0.05,
        "плитка широкой — полосой у бордюра: {tiles}"
    );
    assert!((lawn - 18.0).abs() < 0.05, "газон — до дорожки: {lawn}");
}

/// Обочина по месту переходит от плитки к газону швом поперёк улицы, а не
/// косой кромкой плитки через всю обочину, и короткий провал профиля ниже
/// 4 м не вырезает в газоне зуб плитки (Орёл, витрина 03, L7).
#[test]
fn a_verge_turns_from_tiles_to_lawn_across_the_street() {
    let tiles_past_kerb = |profile: Vec<(f32, f32)>| {
        let mut map = one_street();
        map.roads[0].verges = [6.0, 0.0];
        map.roads[0].verge_profile = [profile, Vec::new()];
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
        // точки кромки плитки за бордюром (кромка — в 6 м от оси): (x, вылет)
        let reach: Vec<(f32, f32)> = layer(&layers, "road_verges")
            .builder
            .positions_for_test()
            .iter()
            .filter(|at| at[1] > 106.01)
            .map(|at| (at[0], at[1] - 106.0))
            .collect();
        reach
    };
    // 3 м до x = 300, от x = 320 — 7 м: дорожка отходит за 20 м, 4 м — у x = 305
    let reach = tiles_past_kerb(vec![(0.0, 3.0), (200.0, 3.0), (220.0, 7.0), (500.0, 7.0)]);
    let slanted: Vec<_> = reach
        .iter()
        .filter(|(x, past)| {
            *x > 305.0 + 2.0 * VERGE_SEAM + 0.01 && (past - VERGE_KERB).abs() >= 0.01
        })
        .collect();
    assert!(
        slanted.is_empty(),
        "плитка сходит на полосу у бордюра косой: {slanted:?}"
    );
    let lawn_start = reach
        .iter()
        .filter(|(_, past)| (past - VERGE_KERB).abs() < 0.01)
        .map(|(x, _)| *x)
        .fold(f32::INFINITY, f32::min);
    assert!(
        (lawn_start - 305.0).abs() < 0.2,
        "газон начинается у x = {lawn_start}, а не там, где обочина проходит 4 м"
    );
    // провал до 3.5 м на десять метров посреди газона
    let reach = tiles_past_kerb(vec![
        (0.0, 7.0),
        (200.0, 7.0),
        (205.0, 3.5),
        (210.0, 7.0),
        (500.0, 7.0),
    ]);
    assert!(
        reach
            .iter()
            .all(|(_, past)| (past - VERGE_KERB).abs() < 0.01),
        "зуб плитки в газоне: {:?}",
        reach
            .iter()
            .filter(|(_, past)| (past - VERGE_KERB).abs() >= 0.01)
            .collect::<Vec<_>>()
    );
}

/// Газон широкой обочины — приглушённой травой двора, а лугом только у
/// замапленного газона или сквера: у двора светлый луг лежал лентой со швом
/// на кромке квартала (районный кадр d2, №42), а у площади или голой земли
/// за обочиной — салатовой лентой на районе (восточная сторона Фрунзе, L1).
#[test]
fn a_wide_verge_is_meadow_only_beside_a_mapped_lawn() {
    // что лежит слева за обочиной: кончается в метре за дорожкой
    let lawns = |beyond: Option<AreaKind>| {
        let mut map = one_street();
        map.roads[0].verges = [12.0, 0.0];
        if let Some(kind) = beyond {
            let area = fixture::area(
                kind,
                vec![
                    Vec2::new(50.0, 110.0),
                    Vec2::new(650.0, 110.0),
                    Vec2::new(650.0, 300.0),
                    Vec2::new(50.0, 300.0),
                ],
            );
            match kind {
                AreaKind::Grass => map.grass.push(area),
                AreaKind::Park => map.parks.push(area),
                AreaKind::Parking(_) => map.parking.push(area),
                _ => map.landuse.push(area),
            }
        }
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
        ["road_verge_lawns", "road_verge_yards"]
            .map(|name| !layer(&layers, name).builder.is_empty())
    };
    let [meadow, yard] = [[true, false], [false, true]];
    assert_eq!(lawns(Some(AreaKind::Grass)), meadow, "у газона — лугом");
    assert_eq!(lawns(Some(AreaKind::Park)), meadow, "у сквера — лугом");
    assert_eq!(
        lawns(Some(AreaKind::Residential)),
        yard,
        "у двора — травой двора"
    );
    assert_eq!(
        lawns(Some(AreaKind::Parking(LotKind::Yard))),
        yard,
        "у площади — травой двора"
    );
    assert_eq!(lawns(None), yard, "у голой земли — травой двора");
}

/// Угол, за которым газон обочин, — площадкой плитки вдоль бордюрной дуги:
/// зебры выходили на траву серпом между двумя газонами (Тула, витрина 01).
#[test]
fn a_corner_between_wide_verges_is_paved_along_the_kerb() {
    let mut map = MapData::default();
    for points in [
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(300.0, 100.0),
            Vec2::new(500.0, 100.0),
        ],
        vec![
            Vec2::new(300.0, -100.0),
            Vec2::new(300.0, 100.0),
            Vec2::new(300.0, 300.0),
        ],
    ] {
        map.roads.push(RoadLine {
            sidewalks: [SidewalkSide::None; 2],
            verges: [12.0; 2],
            ..fixture::street(points, 12.0)
        });
    }
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    // в четверти за углом кромок (306, 106): дальше полосы у бордюра от
    // обеих кромок, но ближе газона
    let pad = layer(&layers, "road_verges")
        .builder
        .positions_for_test()
        .iter()
        .filter(|at| (1.0..6.0).contains(&(at[0] - 306.0)) && (1.0..6.0).contains(&(at[1] - 106.0)))
        .count();
    assert!(pad > 0, "у бордюрной дуги нет плитки");
}

/// Лоскут газона обочины, со всех сторон закрытый мощением, — плиткой: между
/// двумя мощёными дорожками поперёк обочины в метре друг от друга газон
/// оставался зелёным карманом посреди плитки (Тула, угол Халтурина и
/// Гоголевской, R9). Газон за дорожками, открытый вдоль улицы, — газон.
#[test]
fn a_lawn_scrap_enclosed_by_paving_is_tiled() {
    let paved = |points: Vec<Vec2>| RoadLine {
        pavement: Some(Pavement::Paved),
        ..fixture::footway(points)
    };
    let mut map = one_street();
    map.roads[0].sidewalks = [SidewalkSide::None; 2];
    map.roads[0].verges = [6.0, 0.0];
    // улица начинается на перекрёстке: лоскуты ищутся у концов в узле
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, -100.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0, 300.0),
        ],
        12.0,
    ));
    // дорожка по кромке обочины и две поперёк неё у конца улицы: между их
    // лентами (по 3.5 м) метр газона от полосы у бордюра до дорожки вдоль
    map.roads.push(paved(vec![
        Vec2::new(100.0, 112.0),
        Vec2::new(600.0, 112.0),
    ]));
    for x in [120.0, 124.5] {
        map.roads
            .push(paved(vec![Vec2::new(x, 106.0), Vec2::new(x, 112.0)]));
    }
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let tiles = &layer(&layers, "road_verges").builder;
    assert!(
        tiles.covers_for_test(Vec2::new(122.25, 108.5)),
        "лоскут между дорожками — газоном"
    );
    assert!(report.lawn_scraps >= 1, "{}", report.lawn_scraps);
    assert!(
        !tiles.covers_for_test(Vec2::new(200.0, 108.5)),
        "газон вдоль улицы замощён"
    );
}

/// Обочина по месту заходит за торец своей улицы внахлёст: у стыка двух way
/// одной улицы между торцами обочин светилась нить (Орёл, витрина 04).
#[test]
fn a_verge_by_place_overlaps_past_its_street_end() {
    let mut map = one_street();
    map.roads[0].verges = [3.0, 0.0];
    map.roads[0].verge_profile = [vec![(0.0, 3.0), (500.0, 3.0)], Vec::new()];
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let tiles = layer(&layers, "road_verges").builder.positions_for_test();
    let reach = tiles.iter().map(|at| at[0]).fold(f32::INFINITY, f32::min);
    assert!(
        (reach - (100.0 - VERGE_END_OVERLAP)).abs() < 0.01,
        "обочина начинается у x = {reach}"
    );
}

#[test]
fn only_the_bridge_shadow_is_blended() {
    let (layers, _) = mesh_roads(&one_street(), RoadStyle::default(), RoadShape::default());

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
        let (layers, report) = mesh_roads(&map, RoadStyle::default(), shape);
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
        let (layers, _) = mesh_roads(&map, style, RoadShape::default());
        layer(&layers, "sidewalks").builder.vertex_count()
    };

    assert_eq!(sidewalk_verts(false), 0);
    assert!(sidewalk_verts(true) > 0);
}

/// Улица с вершиной посередине и вторая, выходящая из неё **этой же** точкой:
/// у Overpass нет id нод, и перекрёсток восстанавливается по совпадению
/// координат, так что общая вершина обязана быть у обеих.
fn a_tee() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(350.0, 100.0),
            Vec2::new(600.0, 100.0),
        ],
        12.0,
    ));
    map.roads.push(fixture::street(
        vec![Vec2::new(350.0, 100.0), Vec2::new(350.0, 400.0)],
        10.0,
    ));
    map
}

fn junctions_with(map: &MapData, markings: bool) -> usize {
    let style = RoadStyle {
        markings,
        ..RoadStyle::default()
    };
    mesh_roads(map, style, RoadShape::default())
        .1
        .junctions
        .count
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
    );
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(!layer(&layers, "unpaved_roads").builder.is_empty());
    assert_eq!(report.paint_lines, 0, "{report}");
    assert!(report.kerb_returns > 0, "углы грунтом: {report}");

    let (mixed, _) = mesh_roads(
        &unpave(a_tee(), &[1]),
        RoadStyle::default(),
        RoadShape::default(),
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

#[test]
fn a_bridge_leaves_the_street_layers_for_the_deck_ones() {
    let mut map = MapData::default();
    map.roads.push(fixture::bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());

    // настил уходит из уличных слоёв в мостовые целиком, и тротуара у него нет
    // никогда: полоса свисала бы с настила над водой
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(layer(&layers, "sidewalks").builder.is_empty());
    assert!(!layer(&layers, "bridges").builder.is_empty());
    // бордюр настила рисуется всегда, независимо от ручки канта
    assert!(!layer(&layers, "bridge_casings").builder.is_empty());
}

/// Переезд в одном уровне (R35) кладёт слой дорог — по улице как она
/// нарисована; путь на мосту переезда не даёт, зато даёт путепровод: плиту,
/// парапет и тень в мостовых слоях.
#[test]
fn a_track_across_a_street_is_a_level_crossing_unless_it_is_on_a_bridge() {
    let with_track = |bridge: bool| {
        let mut map = one_street();
        let mut track = fixture::rail(vec![Vec2::new(300.0, 0.0), Vec2::new(300.0, 200.0)], 5.0);
        track.bridge = bridge;
        map.rails.push(track);
        mesh_roads(&map, RoadStyle::default(), RoadShape::default())
    };

    let (layers, report) = with_track(false);
    assert!(!layer(&layers, "rail_crossings").builder.is_empty());
    assert_eq!(report.level_crossings, 1);
    assert_eq!(report.track_bridges, 0);
    assert!(layer(&layers, "bridges").builder.is_empty());

    let (layers, report) = with_track(true);
    assert!(layer(&layers, "rail_crossings").builder.is_empty());
    assert_eq!(report.level_crossings, 0);
    assert_eq!(report.track_bridges, 1);
    for name in ["bridge_shadows", "bridge_casings", "bridges"] {
        assert!(!layer(&layers, name).builder.is_empty(), "{name}");
    }
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
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

/// Заливка настила кладётся в порядке заливки улиц и несёт раму полос и
/// разрывы асфальта своей улицы: колея шейдера на мосту та же, что на
/// подходе, и гаснет на узле, где мост — не ведущий. Пешеходный мостик в том
/// же меше рамы не получает.
#[test]
fn deck_fill_carries_its_streets_lane_frame() {
    let mut map = MapData::default();
    // широкая улица и мост у́же её, торцом в её средний узел: у моста там
    // разрыв асфальта
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(350.0, 100.0),
            Vec2::new(600.0, 100.0),
        ],
        16.0,
    ));
    map.roads.push(fixture::bridge(
        vec![Vec2::new(350.0, 100.0), Vec2::new(350.0, 400.0)],
        8.0,
    ));
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let coords = layer(&layers, "bridges")
        .builder
        .ribbon_coords_for_test()
        .expect("настил — фактурный слой");
    assert!(!coords.is_empty());
    // рама полос: `[поперёк, до разрыва, low, high]` от узла сетки, у ленты
    // без полос — `[поперёк, до разрыва, полуширина, 0]`
    assert!(
        coords.iter().all(|c| c[2] < 0.0 && c[3] > 0.0),
        "the deck fill lost its street's lane frame"
    );
    let nearest = coords.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
    assert!(
        nearest < 10.0,
        "the deck fill ignores the asphalt break at its junction ({nearest} m)"
    );

    // пешеходный мостик — в том же меше, но без полос
    let mut footbridge = MapData::default();
    footbridge.roads.push(RoadLine {
        class: RoadClass::Alley,
        ..fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 40.0)], 3.0)
    });
    let (layers, _) = mesh_roads(&footbridge, RoadStyle::default(), RoadShape::default());
    let coords = layer(&layers, "bridges")
        .builder
        .ribbon_coords_for_test()
        .unwrap();
    assert!(!coords.is_empty());
    assert!(coords.iter().all(|c| c[3] == 0.0), "a footbridge got lanes");
}

/// Настил, продолжающий наземный подход, кончается у головы моста ровным
/// срезом, как его бортик: полудиск торца ложился поверх асфальта подхода, а
/// у мостика у кромки — светлым полукруглым вырезом на краю проезжей части
/// (R14, Тула, 49777488 → мост 49777171 и тротуар 1506675089 → 1506675090).
/// Свободный торец моста остаётся круглым.
#[test]
fn a_deck_landing_on_its_approach_ends_square() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 0.0), head],
        8.0,
    ));
    map.roads.push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 8.0));
    // тротуар: наземный кусок и мостик — продолжением через свой узел
    let foot = Vec2::new(30.0, 100.0);
    map.roads.push(fixture::footway(vec![Vec2::new(30.0, 0.0), foot]));
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::bridge(vec![foot, Vec2::new(30.0, 160.0)], 3.0)
    });
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let positions = layer(&layers, "bridges").builder.positions_for_test();
    assert!(!positions.is_empty());
    let behind = positions
        .iter()
        .map(|position| head.y - position[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        behind < 1e-3,
        "the deck fill runs {behind} m past the bridge head onto its approach"
    );
    // свободный конец — скруглён: заливка выходит за последнюю точку
    let ahead = positions
        .iter()
        .map(|position| position[1] - 160.0)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(ahead > 1.0, "the free deck end lost its round cap ({ahead} m)");
}

/// Полоса тротуара подхода кончается на голове моста ровным срезом: её
/// полудиск светился по сторонам настила за его бортиком, как тротуар на
/// самом мосту (R32, Орёл, путепровод 1-й Курской).
#[test]
fn an_approach_sidewalk_ends_square_at_the_bridge_head() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    map.roads.push(fixture::street(vec![Vec2::new(0.0, 0.0), head], 14.2));
    map.roads.push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 14.2));
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(!sidewalks.is_empty());
    let past = sidewalks
        .iter()
        .map(|position| position[1] - head.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        past < 1e-3,
        "the approach sidewalk runs {past} m past the bridge head beside the deck"
    );
}

/// Мостик, замапленный внутри настила моста (Тула: 3.6 м от оси при
/// полуширине 3.8), выносится к его кромке, а наземная дорожка, продолжающая
/// его, идёт следом: иначе мостик полосой закрывал край проезжей части, а за
/// головой моста его торец лежал светлым прямоугольником на асфальте подхода
/// (R14).
#[test]
fn a_footbridge_inside_the_deck_moves_to_its_edge() {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)],
        7.6,
    ));
    map.roads.push(fixture::bridge(
        vec![Vec2::new(0.0, 100.0), Vec2::new(0.0, 160.0)],
        7.6,
    ));
    let foot = Vec2::new(-3.6, 101.0);
    map.roads.push(fixture::footway(vec![Vec2::new(-20.0, 80.0), foot]));
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::bridge(vec![foot, Vec2::new(-3.6, 160.0)], 3.5)
    });
    map.network = super::network::RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    // и бортик мостика — за кромкой: на подходе он торчал бы на асфальт
    let need = 7.6 / 2.0 + map.roads[3].curb_reach();
    let footbridge = prepared.axis(3, Axis::Nodal);
    for point in footbridge {
        assert!(
            point.x <= -need + 1e-3,
            "the footbridge stays {} m off the deck axis, inside its edge",
            -point.x
        );
    }
    // наземная дорожка сходится в тот же торец
    let ground = prepared.axis(2, Axis::Nodal);
    let joint = ground[ground.len() - 1];
    assert!(
        joint.distance(footbridge[0]) < 1e-3,
        "the ground footway ends at {joint}, the footbridge starts at {}",
        footbridge[0]
    );
    assert_eq!(ground[0], Vec2::new(-20.0, 80.0), "the far end moved too");
}

/// Мощёная разделительная двух настилов-половин лежит в слое настилов: в слое
/// улиц её закрывала тень моста, и между половинами шла тёмная щель с
/// бортиками по краям — светлый разделитель вместо двойной сплошной подхода
/// (R30).
#[test]
fn a_paved_median_between_two_decks_lies_on_the_deck() {
    let mut map = MapData::default();
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(2),
        ..fixture::bridge(points, 7.6)
    };
    map.roads.push(half(vec![Vec2::new(-4.5, 0.0), Vec2::new(-4.5, 200.0)]));
    map.roads.push(half(vec![Vec2::new(4.5, 200.0), Vec2::new(4.5, 0.0)]));
    map.network = super::network::RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    assert!(
        prepared.pairs().medians().iter().any(|median| median.is_paved()),
        "the two decks make no paved median"
    );
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let middle = Vec2::new(0.0, 100.0);
    assert!(
        layer(&layers, "bridges").builder.covers_for_test(middle),
        "the median between the decks is not on the deck"
    );
}

/// На шве земля/мост со сменой числа полос настил берёт клин, как шов улицы:
/// у головы моста он шириной в подход и расходится до своей, бортик сужается
/// вместе с ним. Без клина ширина прыгала ступенькой ровно на голове, а шов
/// закрывали полудиски торцов (R30, Красный мост в Орле: земля 2 полосы, мост
/// 3).
#[test]
fn a_wider_deck_tapers_from_its_approach() {
    let head = Vec2::new(0.0, 100.0);
    let lanes = |lanes: u8| 3.3 * f32::from(lanes) + 1.0;
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        highway: Highway::Primary,
        lanes: Some(2),
        ..fixture::street(vec![Vec2::new(0.0, 0.0), head], lanes(2))
    });
    map.roads.push(RoadLine {
        highway: Highway::Primary,
        lanes: Some(3),
        ..fixture::bridge(vec![head, Vec2::new(0.0, 200.0)], lanes(3))
    });
    map.network = super::network::RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    assert!(
        prepared.taper_ends(1)[0].is_some(),
        "the deck takes no taper at its head"
    );

    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let near_head = |name: &str| -> f32 {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|position| (position[1] - head.y).abs() < 0.5)
            .map(|position| position[0].abs())
            .fold(0.0, f32::max)
    };
    let narrow = lanes(2) / 2.0;
    let fill = near_head("bridges");
    assert!(
        (fill - narrow).abs() < 0.05,
        "the deck starts {fill} m wide at its head, the approach is {narrow}"
    );
    let curb = map.roads[1].curb_reach() - map.roads[1].width / 2.0;
    let casing = near_head("bridge_casings");
    assert!(
        casing < narrow + curb + 0.05,
        "the curb stands {casing} m out at the head, past the tapered deck"
    );
}

#[test]
fn an_empty_map_still_describes_every_layer() {
    let (layers, report) = mesh_roads(
        &MapData::default(),
        RoadStyle::default(),
        RoadShape::default(),
    );

    assert_eq!(layers.len(), LAYERS.len());
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
    assert_eq!(report.vertices, 0);
}

/// Большая стоянка с дорогой сквозь неё: `through` — односторонняя, уходит за
/// оба края площадки; проезд ряда лежит внутри целиком.
fn ground_with_roads(lot_side: f32) -> MapData {
    let mut map = MapData::default();
    // вид решает разбор по площади: 100 × 100 — большая, 60 × 60 — двор
    let kind = if lot_side * lot_side >= 8000.0 {
        LotKind::Ground
    } else {
        LotKind::Yard
    };
    map.parking.push(fixture::area(
        AreaKind::Parking(kind),
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0 + lot_side, 100.0),
            Vec2::new(100.0 + lot_side, 100.0 + lot_side),
            Vec2::new(100.0, 100.0 + lot_side),
        ],
    ));
    let middle = 100.0 + lot_side / 2.0;
    map.roads.push(RoadLine {
        oneway: true,
        ..fixture::street(
            vec![Vec2::new(0.0, middle), Vec2::new(200.0 + lot_side, middle)],
            5.0,
        )
    });
    map.roads.push(fixture::parking_aisle(vec![
        Vec2::new(middle, 110.0),
        Vec2::new(middle, 90.0 + lot_side),
    ]));
    map
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
    );
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
    assert!(layer(&layers, "lot_lines").builder.is_empty());
}

/// Кольцо радиусом 12 м в начале координат и два односторонних подхода к нему,
/// сходящиеся в одну дорогу в полусотне метров, — въезд и съезд одной улицы.
///
/// `tagged` — несёт ли кольцо `junction=roundabout`, `oneway` — одностороннее ли
/// оно: кольцо без тега в OSM обычное дело.
fn roundabout_with_an_approach(tagged: bool, oneway: bool) -> MapData {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 12.0)
        .collect();
    let arm = |turn: f32, side: f32| RoadLine {
        oneway: true,
        ..fixture::street(
            vec![circle[(24.0 + turn) as usize % 24], Vec2::new(55.0, side)],
            5.0,
        )
    };
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway,
        roundabout: tagged,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads.push(arm(2.0, 1.5));
    map.roads.push(arm(-2.0, -1.5));
    map
}

/// Обочина у дуги кольца — только снаружи: внутри остров и его газон, а
/// снаружи между тротуаром кольца и дорожкой вдоль него лежала голая земля
/// (Калуга, витрина 01).
#[test]
fn a_ring_takes_its_verge_on_the_outer_side_only() {
    let mut map = roundabout_with_an_approach(true, true);
    map.roads[0].verges = [6.0, 6.0];
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

/// Клин между въездом, съездом и кольцом — направляющий островок: асфальт со
/// штриховкой, как рисует Яндекс, — и стоянка для этого не нужна.
#[test]
fn the_wedge_between_the_two_arms_of_a_roundabout_is_hatched() {
    // с тегом — и **без него**: большое кольцо у ТРЦ «Макси» в OSM просто
    // замкнутое одностороннее полотно, и по одному тегу его островки не
    // находились вовсе
    for tagged in [true, false] {
        let map = roundabout_with_an_approach(tagged, true);
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
        // клин — правее кольца, между подходами, у оси x
        let wedged = |at: &&[f32; 3]| (17.0..40.0).contains(&at[0]) && at[1].abs() < 2.5;
        let lines = layer(&layers, paint::PAINT_ISLANDS)
            .builder
            .positions_for_test();
        assert!(
            lines.iter().any(|at| wedged(&at)),
            "клин не заштрихован ({tagged})"
        );
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    // что зашло **под** ленту; полоса под обводку штриховки вылезает из клина
    // на `EDGE_STRIP` — место шейдеру, сама линия в ней тоньше
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
    // ...а штриховка у́же его ровно на этот заход и на полотно не лезет
    let hatching = hatching + paint::EDGE_STRIP;
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    );
    assert!(
        layer(&layers, "ring_island_grass").builder.is_empty(),
        "парк острова перекрашен"
    );
}

#[test]
fn a_two_way_loop_without_the_tag_is_not_a_roundabout() {
    let map = roundabout_with_an_approach(false, false);
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
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
    );
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

/// Два встречных полотна бок о бок — бульвар: между ними не бордюр, а двойная
/// сплошная. Пару находит сеть (`roads/network/pairs.rs`); бульвар у ТРЦ
/// «Макси» — два встречных проезда, как и здесь.
#[test]
fn two_carriageways_side_by_side_get_a_double_line_and_no_kerb_between() {
    let mut map = ground_with_roads(100.0);
    let middle = 150.0;
    map.roads.push(RoadLine {
        oneway: true,
        ..fixture::street(
            vec![Vec2::new(300.0, middle + 5.5), Vec2::new(0.0, middle + 5.5)],
            5.0,
        )
    });
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.medians, [1, 0, 0]);

    let lines = &layer(&layers, "lot_lines").builder;
    assert!(!lines.is_empty());
    for at in lines.positions_for_test() {
        assert!(
            (at[1] - (middle + 2.75)).abs() < 0.5,
            "линия не по оси: {at:?}"
        );
        assert!(
            (100.0..=200.0).contains(&at[0]),
            "линия за площадкой: {at:?}"
        );
    }
    // между осями полотен бордюра нет, снаружи от них он есть
    let kerb = layer(&layers, "lot_sidewalks").builder.positions_for_test();
    assert!(kerb.iter().any(|at| at[1] < middle - 2.5));
    assert!(kerb.iter().any(|at| at[1] > middle + 8.0));
    for at in kerb {
        assert!(
            at[1] < middle + 0.1 || at[1] > middle + 5.4,
            "бордюр на разделительной: {at:?}"
        );
    }
}

#[test]
fn the_sidewalk_knob_takes_the_kerb_off_the_lot_road_too() {
    let style = RoadStyle {
        sidewalks: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(&ground_with_roads(100.0), style, RoadShape::default());
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
        mesh_roads(&map, style, RoadShape::default())
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

/// Разделённый проспект вдоль x: две встречные половины в три полосы с
/// `gap` метров между кромками. Возвращает карту и расстояние между осями.
fn divided_avenue(gap: f32) -> (MapData, f32) {
    let width = 3.0 * 3.3 + 1.0;
    let apart = width + gap;
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(3),
        ..fixture::street(points, width)
    };
    let mut map = MapData::default();
    map.roads
        .push(half(vec![Vec2::new(100.0, 100.0), Vec2::new(500.0, 100.0)]));
    map.roads.push(half(vec![
        Vec2::new(500.0, 100.0 + apart),
        Vec2::new(100.0, 100.0 + apart),
    ]));
    (map, apart)
}

#[test]
fn paired_halves_share_a_paved_median_and_keep_sidewalks_outside() {
    let (map, apart) = divided_avenue(0.6);
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
        let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

/// Карманы по тегу `street_side` с обеих сторон: асфальт за кромкой, тротуар
/// отодвинут за карман. Правило без тега ставит их редко
/// (`pockets::sparse_pockets`), а тут нужен карман наверняка.
#[test]
fn a_primary_gets_pockets_in_its_sidewalks() {
    let mut map = one_street();
    map.roads[0].highway = Highway::Primary;
    map.roads[0].parking = [KerbParking::Pocket; 2];
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.kerb_pockets, 2);
    let edge = 100.0 + 6.0 + pockets::POCKET_WIDTH;
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(roads.iter().any(|at| (at[1] - edge).abs() < 0.01));
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(sidewalks.iter().any(|at| at[1] > edge + 1.0));
}

/// `highway=turning_circle` на торце тупика — круг асфальта шире дороги.
#[test]
fn a_turning_circle_widens_the_dead_end() {
    let mut map = one_street();
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(600.0, 100.0),
        kind: RoadNodeKind::TurningCircle,
    });
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.medians, [0, 1, 0]);
    let inner = (3.0 * 3.3 + 1.0) / 2.0;
    let grass = layer(&layers, "road_medians").builder.positions_for_test();
    assert!(!grass.is_empty(), "газон есть");
    for at in grass {
        assert!(
            at[1] >= 100.0 + inner + medians::MEDIAN_KERB - 0.05
                && at[1] <= 100.0 + apart - inner - medians::MEDIAN_KERB + 0.05,
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report, _, tracks) =
        mesh_roads_with_ruts(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(map, RoadStyle::default(), RoadShape::default());
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
    assert_eq!(median_loop_counts(&map), (vec![207, 172, 24, 6], 9));
    // газон, прорезанный проходами по двум зебрам
    let map = island_zebra_avenue().0;
    assert_eq!(median_loop_counts(&map), (vec![128, 104, 32, 0], 16));
}

/// Пин перед `Drawn::sidewalk_on`: карман по тегу со стороны без тротуара —
/// асфальт за кромкой есть, тротуара за ним нет.
#[test]
fn a_pocket_on_the_side_without_a_sidewalk_pushes_no_sidewalk() {
    let mut map = one_street();
    map.roads[0].highway = Highway::Primary;
    map.roads[0].parking = [KerbParking::Pocket; 2];
    map.roads[0].sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.kerb_pockets, 2);
    let edge = 6.0 + pockets::POCKET_WIDTH;
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(roads.iter().any(|at| (at[1] - (100.0 - edge)).abs() < 0.01));
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(sidewalks.iter().any(|at| at[1] > 100.0 + edge + 1.0));
    assert!(
        sidewalks.iter().all(|at| at[1] > 100.0 - 6.0 - 0.01),
        "справа по ходу тротуара нет — ни у ленты, ни у кармана"
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
    let (_, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    // четыре угла асфальта, а тротуар поворачивает только на северных
    assert_eq!([report.kerb_returns, report.sidewalk_returns], [4, 2]);
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.medians, [0, 1, 0]);
    assert_eq!(report.tram_bands, 0, "путь в траве — без полосы");
    assert!(!layer(&layers, "road_medians").builder.is_empty());
}

#[test]
fn an_arm_is_filled_before_its_leader_even_when_it_leads_elsewhere() {
    use node_paint::{Junction, JunctionArm};
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
    assert_eq!(fill_order(&widths, &leading, &junctions), vec![2, 1, 0]);
    // без узла — прежний ключ
    assert_eq!(fill_order(&widths, &leading, &[]), vec![2, 0, 1]);
}

/// Отчёт без часов: два прогона одной карты обязаны совпасть до поля.
fn timeless(map: &MapData) -> RoadReport {
    let (_, mut report) = mesh_roads(map, RoadStyle::default(), RoadShape::default());
    report.network = Default::default();
    report.elapsed = Default::default();
    report
}

fn with_network(roads: Vec<RoadLine>) -> MapData {
    MapData {
        network: network::RoadNetwork::new(&roads),
        roads,
        ..default()
    }
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
    let arcs = ring_arcs(&map.roads, &axes.rings);
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
        let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
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
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
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
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let lines = &junctions.node_paint().stop_lines;
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
}

/// Точка на круге радиуса `radius` под углом `degrees`.
fn polar(radius: f32, degrees: f32) -> Vec2 {
    Vec2::from_angle(degrees.to_radians()) * radius
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
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
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
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Generated,
            stop_lines: true,
        },
    );
    let lines = &junctions.node_paint().stop_lines;
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

/// Вершины слоя цвета `color` — `[x, y]`.
fn vertices_of(layers: &[LayerMesh], name: &str, color: Color) -> Vec<[f32; 2]> {
    let builder = &layer(layers, name).builder;
    let wanted = color.to_linear().to_f32_array();
    builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
        .filter(|(_, color)| **color == wanted)
        .map(|(at, _)| [at[0], at[1]])
        .collect()
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
fn a_crossing_piece_between_two_halves_carries_no_sidewalk() {
    // Поперечная улица из трёх way: подход с юга, кусок между половинами
    // разделённого проспекта и продолжение на север. Кусок в проёме пары
    // (`across_median`) тротуара не несёт — ни лентой, ни скруглением, ни
    // зеброй по правилу: слои те же, что у куска с `sidewalk=no`.
    // узлы поперечной — вершины на половинах, как в OSM
    let crossed = |gap: f32| {
        let (map, apart) = divided_avenue(gap);
        let (south, north) = (Vec2::new(300.0, 100.0), Vec2::new(300.0, 100.0 + apart));
        let mut roads = map.roads;
        roads[0].points.insert(1, south);
        roads[1].points.insert(1, north);
        roads.push(fixture::street(vec![Vec2::new(300.0, 30.0), south], 8.0));
        roads.push(fixture::street(vec![south, north], 8.0));
        roads.push(fixture::street(vec![north, Vec2::new(300.0, 190.0)], 8.0));
        (with_network(roads), apart)
    };
    let (map, _) = crossed(0.6);
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.medians, [1, 0, 0]);
    assert_eq!(report.drawn.crossings, 0);
    assert_eq!(report.junctions.count, 2);
    let mut untagged = map.roads.clone();
    untagged[3].sidewalks = [SidewalkSide::None; 2];
    let untagged = with_network(untagged);
    let (bare, _) = mesh_roads(&untagged, RoadStyle::default(), RoadShape::default());
    for (tagged, untagged) in layers.iter().zip(&bare) {
        assert_eq!(tagged.name, untagged.name);
        let (with, without) = (
            tagged.builder.positions_for_test(),
            untagged.builder.positions_for_test(),
        );
        let differs = with
            .iter()
            .zip(without)
            .position(|(a, b)| a != b)
            .or_else(|| (with.len() != without.len()).then_some(with.len().min(without.len())));
        let from = differs.unwrap_or(0);
        assert!(
            differs.is_none(),
            "{}: кусок в проёме пары рисуется как без тротуара; расходится с вершины {from} из {} / {}: {:?} / {:?}",
            tagged.name,
            with.len(),
            without.len(),
            &with[from..(from + 12).min(with.len())],
            &without[from..(from + 12).min(without.len())],
        );
    }
    // а у подхода с юга тротуар есть
    let half = (3.0 * 3.3 + 1.0) / 2.0;
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(
        sidewalks
            .iter()
            .any(|at| (at[0] - 300.0).abs() > 4.5 && at[1] > 60.0 && at[1] < 100.0 - half)
    );
    // и без пары тот же кусок его несёт: половины дальше 40 м друг от друга —
    // не пара, и куску между ними тротуар положен
    let (far, _) = crossed(60.0);
    let (layers, report) = mesh_roads(&far, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.medians, [0, 0, 0]);
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(
        sidewalks
            .iter()
            .any(|at| (at[0] - 300.0).abs() > 4.5 && at[1] > 120.0 && at[1] < 140.0),
        "без пары кусок несёт тротуар"
    );
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

/// Проспект из двух половин, поперечная жилая пересекает обе: узлы — вершины
/// на половинах, как в OSM.
fn an_avenue_crossed_by_a_street() -> (MapData, f32) {
    let (mut map, apart) = divided_avenue(0.6);
    let (south, north) = (Vec2::new(300.0, 100.0), Vec2::new(300.0, 100.0 + apart));
    map.roads[0].points.insert(1, south);
    map.roads[1].points.insert(1, north);
    map.roads.push(fixture::street(
        vec![
            Vec2::new(300.0, 30.0),
            south,
            north,
            Vec2::new(300.0, 190.0),
        ],
        8.0,
    ));
    (with_network(map.roads), apart)
}

/// «До разрыва» (`ATTRIBUTE_RIBBON`) у вершин слоя `name`, что прошли
/// фильтр; полигоны (нули) не в счёт. У асфальта это второе число, у полосы
/// краски — третье (`MeshBuilder::push_paint_strip`).
fn to_break_where(layers: &[LayerMesh], name: &str, keep: impl Fn(&[f32; 3]) -> bool) -> Vec<f32> {
    let builder = &layer(layers, name).builder;
    let slot = if name == "roads" { 1 } else { 2 };
    let coords = builder
        .ribbon_coords_for_test()
        .expect("лента с координатами");
    builder
        .positions_for_test()
        .iter()
        .zip(coords)
        .filter(|(at, coord)| **coord != [0.0; 4] && keep(at))
        .map(|(_, coord)| coord[slot])
        .collect()
}

/// Разделительная открывается по **базовым** разрывам (`marking_breaks`), а
/// не по разрывам асфальта: половины ведут узел, и `NodePaint::asphalt` снял
/// с них разрыв — колея идёт сквозь, — а двойная сплошная у поперечной всё
/// равно рвётся. Перевести медианы на разрывы асфальта — провести её через
/// перекрёсток.
#[test]
fn the_median_base_keeps_the_break_a_leading_road_lost() {
    let (map, apart) = an_avenue_crossed_by_a_street();
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
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

/// Счётчики узлов в строке `road meshing:` — ни одним тестом не пиннились.
#[test]
fn junction_counters_of_a_tee_and_an_avenue_crossing() {
    let tee = timeless(&a_tee()).junctions;
    assert_eq!((tee.count, tee.clusters, tee.through), (1, 0, 1));
    let (avenue, _) = an_avenue_crossed_by_a_street();
    let avenue = timeless(&avenue).junctions;
    assert_eq!((avenue.count, avenue.clusters, avenue.through), (2, 1, 2));
}

/// Зебра OSM у стыка двух way одной улицы рвёт карманы и на продолжении
/// (`pockets::crossing_breaks`) — счётчик карманов под пином.
#[test]
fn a_zebra_at_a_way_end_breaks_the_kerb_pockets_of_both_ways() {
    let primary = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        parking: [KerbParking::Pocket; 2],
        ..fixture::street(points, 14.0)
    };
    let roads = vec![
        primary(vec![
            Vec2::ZERO,
            Vec2::new(99.0, 0.0),
            Vec2::new(100.0, 0.0),
        ]),
        primary(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)]),
    ];
    let zebra = RoadNode {
        pos: Vec2::new(99.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    };
    let mut map = with_network(roads);
    assert_eq!(timeless(&map).kerb_pockets, 4);
    map.road_nodes.push(zebra);
    // карманы те же четыре: зебра их укорачивает, а не делит
    assert_eq!(timeless(&map).kerb_pockets, 4);
}

/// Кусок пары `from..to` у половины, идущей на восток: пара слева.
fn run_left(from: f32, to: f32) -> network::pairs::PairRun {
    network::pairs::PairRun::for_test(from, to, 1, true, 0.6, true)
}

/// Вершины тротуара улицы в 10 м с полосой в 2 м вдоль x от 0 до 100 — с
/// кусками пары `runs` и тротуаром по сторонам `sides`.
fn sidewalk_of(runs: &[network::pairs::PairRun], sides: [bool; 2]) -> Vec<[f32; 3]> {
    let mut builder = MeshBuilder::default();
    let body = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let pairs = Pairs::of_runs(vec![runs.to_vec()]);
    let pieces = pairs.band_pieces(0, sides, 0.0, 100.0);
    push_sidewalk(
        &mut builder,
        &body,
        [10.0, 2.0],
        pieces.as_deref(),
        SIDEWALK_COLOR.to_linear(),
        [false; 2],
    );
    builder.positions_for_test().to_vec()
}

#[test]
fn a_sidewalk_without_pairs_is_one_band_on_its_sides() {
    let both = sidewalk_of(&[], [true; 2]);
    assert!(both.iter().any(|at| at[1] > 6.99) && both.iter().any(|at| at[1] < -6.99));
    // слева только: полоса в 12 м, сдвинутая на метр влево
    let left = sidewalk_of(&[], [true, false]);
    assert!(left.iter().all(|at| at[1] > -5.01 && at[1] < 7.01));
    assert!(left.iter().any(|at| at[1] > 6.99) && left.iter().any(|at| at[1] < -4.99));
    assert!(sidewalk_of(&[], [false; 2]).is_empty());
}

#[test]
fn a_short_gap_between_two_runs_gets_no_sidewalk_on_the_pair_side() {
    // дыра в 3 м между кусками с одной стороны — без тротуара с неё
    let bridged = sidewalk_of(&[run_left(10.0, 40.0), run_left(43.0, 80.0)], [true; 2]);
    assert!(
        bridged
            .iter()
            .filter(|at| at[1] > 5.01)
            .all(|at| at[0] < 10.01 || at[0] > 79.99),
        "слева тротуар только до пары и после неё"
    );
    assert!(
        bridged.iter().any(|at| at[1] < -6.99),
        "справа тротуар есть"
    );
    // дыра в 15 м — не шов, с обеих сторон тротуар
    let open = sidewalk_of(&[run_left(10.0, 40.0), run_left(55.0, 80.0)], [true; 2]);
    assert!(
        open.iter()
            .any(|at| at[1] > 6.99 && at[0] > 39.99 && at[0] < 55.01)
    );
    // обрезок короче полуметра не кладётся: у торцов пары тротуар не
    // появляется
    let trimmed = sidewalk_of(&[run_left(0.3, 99.8)], [true; 2]);
    assert!(trimmed.iter().all(|at| at[1] < 5.01));
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
    let (layers, _) = mesh_roads(&cooked, RoadStyle::default(), RoadShape::default());
    assert!(vertices(&layers, "sidewalks") > 0);
    assert!(paint(&layers) > 0);

    let mut raw = one_street();
    raw.knobs.raw = RawOsm::Draw;
    let (layers, report) = mesh_roads(&raw, RoadStyle::default(), RoadShape::default());
    assert_eq!(vertices(&layers, "sidewalks"), 0);
    assert_eq!(paint(&layers), 0);
    // одна лента по полуширине от оси, с прямыми торцами ровно в концах way
    let road = layer(&layers, "roads").builder.positions_for_test();
    assert!(!road.is_empty() && road.len() <= 8, "{} вершин", road.len());
    assert!(road.iter().all(|at| (at[1] - 100.0).abs() < 6.01));
    assert!(road.iter().all(|at| (99.99..=600.01).contains(&at[0])));
    assert_eq!(report.kerb_returns, 0);
}
