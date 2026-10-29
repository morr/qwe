use super::super::fixtures::{layer, one_street};
use super::*;
use crate::map::osm::model::{Pavement, SidewalkSide};
use crate::map::osm::{AreaKind, LotKind, MapData, RoadLine, fixture};
use crate::map::roads::{RoadShape, RoadStyle, mesh_roads};
use crate::map::surface::LayerMesh;

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
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
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
    // улица 12 м: кромка в 6 м от оси; до дорожки — и под неё
    let [tiles, lawn] = verged(3.0);
    assert!(
        (tiles - (9.0 + VERGE_UNDER_PATH)).abs() < 0.05 && lawn == 0.0,
        "узкая: {tiles} / {lawn}"
    );
    let [tiles, lawn] = verged(12.0);
    assert!(
        (tiles - (6.0 + VERGE_KERB)).abs() < 0.05,
        "плитка широкой — полосой у бордюра: {tiles}"
    );
    assert!(
        (lawn - (18.0 + VERGE_UNDER_PATH)).abs() < 0.05,
        "газон — до дорожки: {lawn}"
    );
}

/// На средней и дальней ступени зума ([`VergeLods`], от `VERGE_ZOOM_MAX`)
/// широкая обочина не рисуется вовсе — ни плиткой у бордюра, ни своим
/// газоном: полоса в 5–11 м приглушённой травы двора рядом с проезжей
/// читалась как второе полотно улицы (Первомайская у парка в Туле, NEW-1).
/// Узкая рисуется и там, а на ближней ступени рисуется всё, как раньше.
#[test]
fn a_wide_verge_is_not_drawn_past_the_near_zoom_step() {
    let reach = |layers: &[LayerMesh], name: &str| {
        layer(layers, name)
            .builder
            .positions_for_test()
            .iter()
            .map(|at| at[1] - 100.0)
            .fold(0.0_f32, f32::max)
    };
    let mut map = one_street();
    map.roads[0].verges = [12.0, 0.0];
    let far = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(VERGE_ZOOM_MAX),
    );
    for name in ["road_verges", "road_verge_yards", "road_verge_lawns"] {
        assert_eq!(reach(&far.0, name), 0.0, "{name} на средней ступени пуст");
    }
    // ближняя — как раньше: полоса плитки у бордюра, газон до дорожки
    let near = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(
        (reach(&near.0, "road_verges") - (6.0 + VERGE_KERB)).abs() < 0.05,
        "плитка у бордюра: {}",
        reach(&near.0, "road_verges")
    );
    assert!(
        (reach(&near.0, "road_verge_yards") - (18.0 + VERGE_UNDER_PATH)).abs() < 0.05,
        "газон до дорожки: {}",
        reach(&near.0, "road_verge_yards")
    );

    // узкая обочина мостится и на средней ступени: две полосы плитки у
    // кромки — тротуар, не второе полотно
    let mut narrow = one_street();
    narrow.roads[0].verges = [3.0, 0.0];
    let far_narrow = mesh_roads(
        &narrow,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(VERGE_ZOOM_MAX),
    );
    assert!(
        (reach(&far_narrow.0, "road_verges") - (9.0 + VERGE_UNDER_PATH)).abs() < 0.05,
        "узкая на средней ступени — плиткой до дорожки: {}",
        reach(&far_narrow.0, "road_verges")
    );

    // и заливка у угла между широкими обочинами — тоже: лента стороны не
    // рисуется, и её угол не остаётся пятном плитки у дуги
    let mut corner = MapData::default();
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
        corner.roads.push(RoadLine {
            sidewalks: [SidewalkSide::None; 2],
            verges: [12.0; 2],
            ..fixture::street(points, 12.0)
        });
    }
    let far_corner = mesh_roads(
        &corner,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(VERGE_ZOOM_MAX),
    );
    assert_eq!(
        reach(&far_corner.0, "road_verges"),
        0.0,
        "у угла нет плитки обочины"
    );
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
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
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
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
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
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
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
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
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
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let tiles = layer(&layers, "road_verges").builder.positions_for_test();
    let reach = tiles.iter().map(|at| at[0]).fold(f32::INFINITY, f32::min);
    assert!(
        (reach - (100.0 - VERGE_END_OVERLAP)).abs() < 0.01,
        "обочина начинается у x = {reach}"
    );
}

/// Обочина заходит под свою дорожку на [`VERGE_UNDER_PATH`]: профиль снят по
/// сырым осям, а ленты легли по сглаженным, и край плитки, стоявший ровно на
/// оси дорожки, отходил от её ленты — вдоль дорожки светилась нить земли
/// (S6, Орёл, Орджоникидзе у Ляшко).
#[test]
fn a_verge_reaches_under_its_path() {
    for (verge, kerb) in [(3.0, "plain"), (6.0, "lawn")] {
        let mut map = one_street();
        map.roads[0].verges = [verge, 0.0];
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        let name = if kerb == "lawn" {
            "road_verge_yards"
        } else {
            "road_verges"
        };
        let edge = 106.0 + verge;
        let fill = &layer(&layers, name).builder;
        assert!(
            fill.covers_for_test(Vec2::new(300.0, edge + VERGE_UNDER_PATH - 0.1)),
            "{kerb}: край обочины не заходит под дорожку"
        );
        assert!(!fill.covers_for_test(Vec2::new(300.0, edge + VERGE_UNDER_PATH + 0.1)));
        // и по месту — так же
        map.roads[0].verge_profile = [vec![(0.0, verge), (500.0, verge)], Vec::new()];
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        let fill = &layer(&layers, name).builder;
        assert!(fill.covers_for_test(Vec2::new(300.0, edge + VERGE_UNDER_PATH - 0.1)));
    }
}
