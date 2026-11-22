use super::super::fixtures::{layer, one_street, with_network};
use super::super::*;
use crate::map::footprint::casing_width;
use crate::map::osm::model::{SIDEWALK_WIDTH_RANGE, SidewalkSide, sidewalk_band};
use crate::map::osm::{Highway, fixture};
use crate::map::parking::LOT_KERB;

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

/// Двухполосная улица переходит в четырёхполосную (`a_section_seam_is_one_taper`);
/// у широкой тротуар только слева по ходу (`sidewalk=left`, к северу).
/// Клин тротуара кладётся по сторонам: слева — от полосы узкой к своей,
/// справа — голая кромка от полуширины узкой к своей полуширине, а не
/// зеркало левой полосы.
#[test]
fn a_one_sided_sidewalk_wedge_keeps_the_bare_kerb_on_the_untagged_side() {
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
