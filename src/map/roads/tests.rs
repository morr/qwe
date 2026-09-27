use super::*;
use crate::map::footprint::casing_width;
use crate::map::meshing::distance_to_path;
use crate::map::osm::model::{
    KerbParking, RailKind, RailLine, RoadNode, SIDEWALK_WIDTH_RANGE, SidewalkSide, sidewalk_band,
};
use crate::map::osm::{Highway, fixture};
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

/// Восемнадцать дорожных слоёв снизу вверх, ровно в том порядке, в каком они
/// уходят в мир: десять лент и восемь слоёв краски над своим асфальтом —
/// колея траекторий узла (маска, потом наложение) ниже линий, островки колец
/// над асфальтом стоянок.
const LAYERS: [&str; 18] = [
    "alleys",
    "sidewalks",
    "road_medians",
    "roads",
    paint::PAINT_WEAR_MASK,
    paint::PAINT_WEAR,
    paint::PAINT_ZEBRAS,
    paint::PAINT_LANES,
    paint::PAINT_AXES,
    "lot_sidewalks",
    "lot_lines",
    paint::PAINT_ISLANDS,
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
fn a_street_builds_eighteen_layers_bottom_up() {
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

#[test]
fn only_the_bridge_shadow_is_blended() {
    let (layers, _) = mesh_roads(&one_street(), RoadStyle::default(), RoadShape::default());

    // фактурный материал — у всего, что асфальт, тротуар или дорожка;
    // блендинг — ровно у полупрозрачной тени настила
    for layer in &layers {
        let expected = match layer.name {
            "bridge_shadows" => MaterialSpec::Blend,
            "sidewalks" | "lot_sidewalks" => MaterialSpec::Surface(SurfaceKind::Sidewalk),
            "alleys" => MaterialSpec::Surface(SurfaceKind::Alley),
            "road_medians" => MaterialSpec::Surface(SurfaceKind::Grass),
            "roads" | "bridges" => MaterialSpec::Surface(SurfaceKind::Street),
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
    mesh_roads(map, style, RoadShape::default()).1.junctions
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
    map.parking.push(fixture::area(
        AreaKind::Parking,
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
        mesh_roads(&map, style, RoadShape::default()).1.zebras[0]
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
    assert!(report.junctions > 0, "узел у ближней половины есть");
    let axes = &layer(&layers, paint::PAINT_AXES).builder;
    let positions = axes.positions_for_test();
    assert!(positions.iter().any(|at| at[0] < 200.0) && positions.iter().any(|at| at[0] > 400.0));
    // разрыв — в «до разрыва» полосы краски: внутри него оно отрицательно
    // осевая самой примыкающей улицы лежит ниже половин — её не считаем
    assert!(
        axes.ribbon_coords_for_test()
            .expect("у краски атрибут есть")
            .iter()
            .zip(positions)
            .filter(|(_, at)| at[1] > 100.0)
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

/// Трамвай уходит с проспекта на полпути: полотно кончается, дальше газон.
/// Торец полотна — ровный, а между ним и носом газона — асфальт, не
/// тротуар и не земля (Советская у Коминтерна).
#[test]
fn a_tram_bed_ends_in_asphalt_up_to_the_nose_of_the_lawn() {
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
    let nodes = RoadNodes::new(&map.roads);
    let axes = axis::street_axes(
        &map.roads,
        &map.rails,
        &map.network,
        &nodes,
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
    assert_eq!(junctions.count(), 1);
    assert_eq!(positive(&junctions.median_base()[0]), 1);
    assert_eq!(dead_ends(&junctions.median_base()[1]), 1);
    // краска: примыкание уступает — его линии рвутся у стежка
    assert_eq!(positive(&junctions.paint().breaks[1]), 1);
    // ряд: стежка нет — улица цела, оба торца примыкания — тупики
    assert_eq!(junctions.row().junctions, 0);
    assert_eq!(positive(&junctions.row().breaks[0]), 0);
    assert_eq!(positive(&junctions.row().breaks[1]), 0);
    assert_eq!(dead_ends(&junctions.row().breaks[1]), 2);
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
    assert_eq!(report.junctions, 1);
    assert_eq!(report.clusters, 0);
    assert_eq!(report.through, 1);
    assert_eq!(report.leading, 1);
    assert_eq!(report.zebras, [1, 0]);
    assert_eq!(report.stop_lines, 1);
    assert_eq!(report.pockets, 0);
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
    assert_eq!(report.junctions, 2);
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
    assert_eq!(report.leading, 2, "узел ведут обе половины");
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
/// колею асфальта: его разрыв кладётся и в `NodePaint::breaks`, и в
/// `NodePaint::asphalt`.
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
    let tee = timeless(&a_tee());
    assert_eq!((tee.junctions, tee.clusters, tee.through), (1, 0, 1));
    let (avenue, _) = an_avenue_crossed_by_a_street();
    let avenue = timeless(&avenue);
    assert_eq!(
        (avenue.junctions, avenue.clusters, avenue.through),
        (2, 1, 2)
    );
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
