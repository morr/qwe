use super::*;
use crate::map::osm::fixture;
use crate::map::osm::model::{KerbParking, RoadArea, RoadAreaKind, TreeSet, WaterKind, WaterLine};
use crate::map::roads::{DrawnPiece, PieceKind};
use crate::settings::Z_ROAD_VERGE_YARD;

/// Запрос с дефолтным стилем дорог — все выведенные полосы отвечают.
fn query(index: &InspectIndex, map: &MapData, tree_density: f32, at: Vec2) -> Vec<Hit> {
    index.hits(map, tree_density, &RoadStyle::default(), at)
}

#[test]
fn a_point_inside_the_feature_hits_it() {
    let map = MapData {
        buildings: vec![fixture::building(
            fixture::square(Vec2::new(100.0, 100.0), 10.0),
            vec![],
        )],
        roads: vec![fixture::street(
            vec![Vec2::new(80.0, 100.0), Vec2::new(120.0, 100.0)],
            8.0,
        )],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);

    // внутри здания и внутри полотна дороги: здание сверху
    let hits = query(&index, &map, 0.0, Vec2::new(100.0, 102.0));
    let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["building", "road residential 8.0 m, sidewalks"]
    );
    assert!(hits[0].z > hits[1].z);

    // внутри здания, но за краем полос тротуара: дороги в ответе нет
    let (_, sidewalk_edge, _) = band_edges(0.0);
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(100.0, 100.0 + sidewalk_edge + 0.1),
    );
    let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
    assert_eq!(labels, vec!["building"]);
}

#[test]
fn a_point_outside_every_feature_hits_nothing() {
    let map = MapData {
        buildings: vec![fixture::building(
            fixture::square(Vec2::new(100.0, 100.0), 10.0),
            vec![],
        )],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(200.0, 200.0));
    assert!(hits.is_empty());
}

#[test]
fn a_hole_punches_the_answer_through() {
    let map = MapData {
        buildings: vec![fixture::building(
            fixture::square(Vec2::new(100.0, 100.0), 10.0),
            vec![fixture::square(Vec2::new(100.0, 100.0), 4.0)],
        )],
        water: vec![fixture::water_area(
            fixture::square(Vec2::new(100.0, 100.0), 3.0),
            vec![],
        )],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(100.0, 100.0));
    let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
    // сквозь дыру здания видна вода под ним
    assert_eq!(labels, vec!["water"]);
}

#[test]
fn a_bridge_road_answers_as_a_bridge_layer() {
    let map = MapData {
        roads: vec![fixture::bridge(
            vec![Vec2::new(0.0, 50.0), Vec2::new(60.0, 50.0)],
            8.0,
        )],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(30.0, 50.0));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].label, "road residential 8.0 m, bridge, sidewalks");
    assert_eq!(hits[0].z, Z_BRIDGE);
}

#[test]
fn a_culvert_is_not_drawn_so_it_is_not_a_hit() {
    // труба (`tunnel=culvert`) не рисуется вовсе — в ответе пробы ей не место
    let map = MapData {
        water_lines: vec![WaterLine {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)],
            width: 2.5,
            kind: WaterKind::Stream,
            tunnel: true,
            osm_id: None,
        }],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(25.0, 0.0));
    assert!(hits.is_empty());
}

#[test]
fn an_open_waterway_is_a_hit() {
    let map = MapData {
        water_lines: vec![WaterLine {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)],
            width: 2.5,
            kind: WaterKind::Stream,
            tunnel: false,
            osm_id: None,
        }],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(25.0, 0.5));
    assert_eq!(hits[0].label, "waterway stream 2.5 m");
}

#[test]
fn trees_answer_by_the_density_prefix() {
    let map = MapData {
        trees: TreeSet::of([
            (Vec2::new(100.0, 100.0), 4.0, 0.0),
            (Vec2::new(120.0, 100.0), 4.0, 0.7),
        ]),
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);

    // порог 0.7 выше плотности 0.5 — дерева в ответе нет
    let hits = query(&index, &map, 0.5, Vec2::new(120.0, 100.0));
    assert!(hits.is_empty());
    // при плотности 0.8 видно уже всё
    let hits = query(&index, &map, 0.8, Vec2::new(120.0, 100.0));
    assert_eq!(hits[0].label, "tree");
}

#[test]
fn duplicates_of_one_kind_collapse() {
    let map = MapData {
        grass: vec![
            fixture::area(
                AreaKind::Grass,
                fixture::square(Vec2::new(100.0, 100.0), 20.0),
            ),
            fixture::area(
                AreaKind::Grass,
                fixture::square(Vec2::new(100.0, 100.0), 10.0),
            ),
        ],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, Vec2::new(100.0, 100.0));
    // две травы одного места — одна находка, земля фоном не считается
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].label, "grass");
}

/// Улица вдоль x на оси `y`: ширина 8, тротуар и обочина по аргументам.
fn banded_street(y: f32, verges: [f32; 2], sidewalks: [SidewalkSide; 2]) -> RoadLine {
    RoadLine {
        verges,
        sidewalks,
        ..fixture::street(vec![Vec2::new(0.0, y), Vec2::new(1000.0, y)], 8.0)
    }
}

/// Улица с карманом у бордюра по тегу `parking:<side>` — обе стороны.
fn pocket_street(y: f32) -> RoadLine {
    RoadLine {
        parking: [KerbParking::Pocket; 2],
        ..fixture::street(vec![Vec2::new(0.0, y), Vec2::new(200.0, y)], 8.0)
    }
}

/// Улица с карманом у бордюра по правилу — магистраль с тротуаром, но
/// без тега `parking:<side>`: карманы кладёт правило (`sparse_pockets`)
/// по ГПСЧ от первой точки, редкие и короткие, не вдоль всей стороны.
fn rule_pocket_street(y: f32) -> RoadLine {
    RoadLine {
        parking: [KerbParking::Untagged; 2],
        highway: Highway::Secondary,
        ..fixture::street(vec![Vec2::new(0.0, y), Vec2::new(200.0, y)], 8.0)
    }
}

/// Кромки полос такой улицы — расстояния от оси, м: полотно, край
/// тротуара, край обочины.
fn band_edges(verge: f32) -> (f32, f32, f32) {
    let half = 4.0;
    let band = crate::map::osm::model::sidewalk_band(8.0);
    (half, half + band, half + band + verge)
}

/// Прямоугольный кусок обочины для тестов сетки кусков: полоса от кромки
/// тротуара (`y0`) до внешнего края (`y1`) на стороне `side` (+1.0 — сторона
/// 0, −1.0 — сторона 1) дороги `road` (улица тестов лежит на y = 70).
fn verge_piece(x0: f32, x1: f32, y0: f32, y1: f32, side: f32, road: u32) -> DrawnPiece {
    DrawnPiece {
        contour: vec![
            Vec2::new(x0, 70.0 + side * y0),
            Vec2::new(x1, 70.0 + side * y0),
            Vec2::new(x1, 70.0 + side * y1),
            Vec2::new(x0, 70.0 + side * y1),
        ],
        holes: vec![],
        kind: PieceKind::VergeYard,
        road: Some(road),
        z: Z_ROAD_VERGE_YARD,
    }
}

/// Карман у бордюра — находка индекса и подсвечивается: контура в
/// `MapData` нет, его вывел закон стоянки по тегу `parking:<side>`.
#[test]
fn a_kerb_pocket_answers_as_a_hit_and_highlights() {
    let map = MapData {
        roads: vec![pocket_street(70.0)],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);

    // середина кармана: за кромкой полотна, внутри его полной ширины
    let point = Vec2::new(100.0, 75.0);
    let hits = query(&index, &map, 0.0, point);
    let pocket = hits
        .iter()
        .find(|hit| hit.label.starts_with("parking pocket"))
        .unwrap_or_else(|| panic!("кармана нет в ответе: {hits:?}"));
    assert!(pocket.label.contains("tagged"), "{}", pocket.label);

    // подсветка рисует тот же полигон, на который находка отвечает
    let shapes = feature_shape(&index, &map, &RoadStyle::default(), pocket.feature)
        .unwrap_or_else(|| panic!("у кармана нет формы"));
    assert!(point_in_polygon(point, &shapes[0].outer));
}

/// Карман по правилу — та же находка с подсветкой, но подпись без
/// «tagged»: тега `parking:<side>` у улицы нет, причину вывело правило.
/// Где именно встал карман, решает ГПСЧ правила, — точка запроса
/// берётся из того же контура, а не с потолка.
#[test]
fn a_rule_pocket_answers_without_the_tagged_mark() {
    let map = MapData {
        roads: vec![rule_pocket_street(70.0)],
        ..default()
    };
    let outlines = kerb_pocket_outlines(&map, &RoadShape::default());
    let Some(outline) = outlines.first() else {
        panic!("правило не положило карман на улицу");
    };
    let point = outline.contour.iter().sum::<Vec2>() / outline.contour.len() as f32;

    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let hits = query(&index, &map, 0.0, point);
    let pocket = hits
        .iter()
        .find(|hit| hit.label.starts_with("parking pocket"))
        .unwrap_or_else(|| panic!("кармана нет в ответе: {hits:?}"));
    assert_eq!(pocket.label, "parking pocket (auto, inferred)");

    // подсветка рисует тот же полигон, на который находка отвечает
    let shapes = feature_shape(&index, &map, &RoadStyle::default(), pocket.feature)
        .unwrap_or_else(|| panic!("у кармана нет формы"));
    assert!(point_in_polygon(point, &shapes[0].outer));
}

#[test]
fn a_sidewalk_band_answers_inside_itself_and_not_beyond() {
    let map = MapData {
        roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (_, sidewalk_edge, _) = band_edges(0.0);

    // внутри полосы — тротуар дорисован дорогой, слой тротуаров
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
    );
    let sidewalks: Vec<&Hit> = hits
        .iter()
        .filter(|hit| hit.label.starts_with("sidewalk"))
        .collect();
    assert_eq!(sidewalks.len(), 1);
    assert_eq!(sidewalks[0].label, "sidewalk (auto, inferred)");
    assert_eq!(sidewalks[0].z, Z_SIDEWALK);

    // за краем полосы ответа нет — дороги под точкой тоже
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 70.0 + sidewalk_edge + 0.1),
    );
    assert!(hits.is_empty());
}

#[test]
fn a_one_sided_sidewalk_answers_only_on_its_side() {
    // тротуар по тегу слева (по ходу точек — +y), справа снят разбором:
    // сторона None находки не даёт вовсе
    let map = MapData {
        roads: vec![banded_street(
            70.0,
            [0.0; 2],
            [SidewalkSide::Tagged, SidewalkSide::None],
        )],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (_, sidewalk_edge, _) = band_edges(0.0);
    let offset = sidewalk_edge - 0.1;

    let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + offset));
    assert_eq!(hits[0].label, "sidewalk (auto, tagged)");
    let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 - offset));
    assert!(hits.is_empty());
}

#[test]
fn tagged_and_inferred_sides_label_their_reason() {
    let map = MapData {
        roads: vec![
            banded_street(70.0, [0.0; 2], [SidewalkSide::Tagged; 2]),
            banded_street(500.0, [0.0; 2], [SidewalkSide::Inferred; 2]),
        ],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (_, sidewalk_edge, _) = band_edges(0.0);

    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
    );
    assert!(
        hits.iter()
            .any(|hit| hit.label == "sidewalk (auto, tagged)")
    );
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 500.0 + sidewalk_edge - 0.1),
    );
    assert!(
        hits.iter()
            .any(|hit| hit.label == "sidewalk (auto, inferred)")
    );
}

#[test]
fn the_sidewalks_knob_hides_the_generated_bands() {
    let map = MapData {
        roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (_, sidewalk_edge, _) = band_edges(0.0);
    let style = RoadStyle {
        sidewalks: false,
        ..RoadStyle::default()
    };

    let hits = index.hits(
        &map,
        0.0,
        &style,
        Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
    );
    assert!(hits.is_empty());
}

#[test]
fn a_verge_piece_answers_up_to_its_outer_edge() {
    let map = MapData {
        roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Inferred; 2])],
        ..default()
    };
    let (_, sidewalk_edge, verge_edge) = band_edges(2.0);
    let piece = verge_piece(490.0, 510.0, sidewalk_edge, verge_edge, 1.0, 0);
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[piece]);

    // за краем тротуара — нарисованная полоса обочины, ответ ведёт кусок
    let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + verge_edge - 0.1));
    assert_eq!(hits[0].label, "verge yard (auto)");
    assert_eq!(hits[0].z, Z_ROAD_VERGE_YARD);
    assert_eq!(hits[0].src, HitSource::Generated(HitReason::Inferred));

    // за внешним краем куска ответа нет
    let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + verge_edge + 0.1));
    assert!(hits.is_empty());
}

#[test]
fn the_outer_edge_of_a_long_piece_is_found_across_the_cell_line() {
    // полнота сетки кусков: полоса вставляется по своим габаритам и
    // дотягивается до y = 62.3 — за границу ячейки 64, которую прежний
    // pad дорог (width/2 = 4) не задевал вовсе
    let map = MapData {
        roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Inferred; 2])],
        ..default()
    };
    let (_, sidewalk_edge, verge_edge) = band_edges(2.0);
    // сторона 1 (−y): полоса дотягивается вниз, за границу ячейки 64
    let piece = verge_piece(490.0, 510.0, sidewalk_edge, verge_edge, -1.0, 0);
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[piece]);

    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 70.0 - verge_edge + 0.01),
    );
    assert!(hits.iter().any(|hit| hit.label == "verge yard (auto)"));
}

#[test]
fn an_osm_walkway_and_a_generated_sidewalk_are_two_rows() {
    let map = MapData {
        roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
        road_areas: vec![RoadArea {
            outline: fixture::rect(Vec2::new(495.0, 72.0), Vec2::new(510.0, 80.0)),
            kind: RoadAreaKind::Walkway,
            osm_id: None,
        }],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (_, sidewalk_edge, _) = band_edges(0.0);

    // OSM-площадь дорожки и дорисованный тротуар — разные находки,
    // пусть и на одном слое
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(500.0, 70.0 + sidewalk_edge - 0.5),
    );
    let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
    assert!(labels.contains(&"walkway"));
    assert!(labels.contains(&"sidewalk (auto, inferred)"));
}

#[test]
fn identical_bands_of_two_roads_collapse() {
    // две улицы в девяти метрах: точка посередине лежит в полосе тротуара
    // обеих — одинаковые находки на одном слое схлопываются в одну
    let map = MapData {
        roads: vec![
            banded_street(65.5, [0.0; 2], [SidewalkSide::Inferred; 2]),
            banded_street(74.5, [0.0; 2], [SidewalkSide::Inferred; 2]),
        ],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);

    let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0));
    let sidewalks = hits
        .iter()
        .filter(|hit| hit.label.starts_with("sidewalk"))
        .count();
    assert_eq!(sidewalks, 1);
}

#[test]
fn the_osm_id_prints_as_a_tail() {
    let hits = vec![
        Hit {
            z: 1.0,
            label: "road residential 8.0 m".into(),
            src: HitSource::Osm,
            id: Some(179102449),
            feature: FeatureRef::Road(0),
        },
        Hit {
            z: 0.0,
            label: "tree".into(),
            src: HitSource::Osm,
            id: None,
            feature: FeatureRef::Tree(0),
        },
    ];
    let text = format_hits(&hits, 12);
    assert!(text.contains("road residential 8.0 m #179102449"));
    // у находки без элемента хвоста нет
    assert!(text.ends_with("tree"));
}

#[test]
fn a_parsed_feature_carries_its_osm_id() {
    // улица с тротуаром по тегу: и находка полотна, и дорисованная полоса
    // наследуют way id своей дороги
    let overpass = fixture::Overpass::new(crate::city::City::Tula).way(
        &[("highway", "residential"), ("sidewalk", "both")],
        vec![Vec2::new(1000.0, 1400.0), Vec2::new(1400.0, 1400.0)],
    );
    let map = overpass.parse();
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);

    let road = &map.roads[0];
    assert_eq!(road.osm_id, Some(1));

    let hits = query(&index, &map, 0.0, Vec2::new(1200.0, 1400.0));
    let road_hit = hits
        .iter()
        .find(|hit| hit.label.starts_with("road "))
        .expect("the way is a road hit");
    assert_eq!(road_hit.id, Some(1));

    let half = road.width / 2.0;
    let band = road.sidewalk().band().expect("tagged sidewalk");
    let hits = query(
        &index,
        &map,
        0.0,
        Vec2::new(1200.0, 1400.0 + half + band - 0.1),
    );
    let sidewalk = hits
        .iter()
        .find(|hit| hit.label.starts_with("sidewalk"))
        .expect("the band is a hit");
    assert_eq!(sidewalk.id, Some(1));
    assert_eq!(sidewalk.src, HitSource::Generated(HitReason::Tagged));
}

#[test]
fn every_hit_carries_its_feature_reference() {
    let map = MapData {
        buildings: vec![fixture::building(
            fixture::square(Vec2::new(100.0, 100.0), 10.0),
            vec![],
        )],
        roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Tagged; 2])],
        water: vec![fixture::water_area(
            fixture::square(Vec2::new(200.0, 200.0), 10.0),
            vec![],
        )],
        trees: TreeSet::of([(Vec2::new(300.0, 100.0), 4.0, 0.0)]),
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (half, sidewalk_edge, verge_edge) = band_edges(2.0);
    let feature = |at: Vec2| {
        query(&index, &map, 0.5, at)
            .into_iter()
            .next()
            .map(|hit| hit.feature)
    };

    // полосы-генераты ссылаются на свою дорогу и сторону (0 — слева, +y)
    assert_eq!(
        feature(Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1)),
        Some(FeatureRef::SidewalkBand { road: 0, side: 0 })
    );
    // состав обочины отвечают отрисованные куски; в этом индексе их нет
    assert_eq!(feature(Vec2::new(500.0, 70.0 + verge_edge - 0.1)), None);
    // а с куском — находка ведёт кусок, и подсветка накрывает точку
    let piece_at = Vec2::new(500.0, 70.0 + verge_edge - 0.5);
    let piece = verge_piece(490.0, 510.0, sidewalk_edge, verge_edge, 1.0, 0);
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[piece]);
    let hits = query(&index, &map, 0.5, piece_at);
    let piece_hit = hits
        .iter()
        .find(|hit| hit.label == "verge yard (auto)")
        .expect("the drawn piece is a hit");
    assert_eq!(piece_hit.feature, FeatureRef::Piece(0));
    assert_eq!(piece_hit.id, map.roads[0].osm_id);
    assert_eq!(piece_hit.z, Z_ROAD_VERGE_YARD);
    let shapes = feature_shape(&index, &map, &RoadStyle::default(), piece_hit.feature)
        .expect("the piece has a shape");
    assert!(
        shapes
            .iter()
            .all(|shape| point_in_polygon(piece_at, &shape.outer))
    );
    // полотно, здание, вода, дерево — адреса своих элементов
    assert_eq!(
        feature(Vec2::new(500.0, 70.0 + half - 0.1)),
        Some(FeatureRef::Road(0))
    );
    assert_eq!(
        feature(Vec2::new(100.0, 102.0)),
        Some(FeatureRef::Building(0))
    );
    assert_eq!(feature(Vec2::new(200.0, 200.0)), Some(FeatureRef::Water(0)));
    assert_eq!(feature(Vec2::new(300.0, 100.0)), Some(FeatureRef::Tree(0)));
    assert_eq!(feature(Vec2::new(1000.0, 1000.0)), None);
}

#[test]
fn a_band_shape_lies_on_its_side_only() {
    // тротуар слева (side 0, +y), справа снят — форма есть только слева
    let map = MapData {
        roads: vec![banded_street(
            70.0,
            [0.0; 2],
            [SidewalkSide::Tagged, SidewalkSide::None],
        )],
        ..default()
    };
    let style = RoadStyle::default();
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let (half, sidewalk_edge, _) = band_edges(0.0);

    let shapes = feature_shape(
        &index,
        &map,
        &style,
        FeatureRef::SidewalkBand { road: 0, side: 0 },
    )
    .expect("the left side has a sidewalk");
    let dy: Vec<f32> = shapes[0].outer.iter().map(|point| point.y - 70.0).collect();
    assert!(
        dy.iter()
            .all(|&d| d >= half - 1e-4 && d <= sidewalk_edge + 1e-4)
    );

    assert!(
        feature_shape(
            &index,
            &map,
            &style,
            FeatureRef::SidewalkBand { road: 0, side: 1 }
        )
        .is_none(),
        "the right side has no sidewalk"
    );
}

#[test]
fn a_building_shape_carries_its_hole() {
    let map = MapData {
        buildings: vec![fixture::building(
            fixture::square(Vec2::new(100.0, 100.0), 10.0),
            vec![fixture::square(Vec2::new(100.0, 100.0), 4.0)],
        )],
        ..default()
    };

    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let shapes = feature_shape(&index, &map, &RoadStyle::default(), FeatureRef::Building(0))
        .expect("the building has a shape");
    assert_eq!(shapes.len(), 1);
    assert_eq!(
        shapes[0].outer,
        fixture::square(Vec2::new(100.0, 100.0), 10.0)
    );
    assert_eq!(
        shapes[0].holes,
        vec![fixture::square(Vec2::new(100.0, 100.0), 4.0)]
    );
}

#[test]
fn the_band_shape_covers_what_the_hit_answers() {
    // зона формы совпадает с зоной срабатывания находки: точка внутри
    // полосы и в форме, и в находке; за кромкой — ни там, ни там
    let map = MapData {
        roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Tagged; 2])],
        ..default()
    };
    let index = InspectIndex::rebuild(&map, &RoadShape::default(), &[]);
    let style = RoadStyle::default();
    let (half, sidewalk_edge, _) = band_edges(0.0);
    let shapes = feature_shape(
        &index,
        &map,
        &style,
        FeatureRef::SidewalkBand { road: 0, side: 0 },
    )
    .expect("the sidewalk band has a shape");
    assert_eq!(shapes.len(), 1, "one segment, one quad");
    let contains = |at: Vec2| point_in_polygon(at, &shapes[0].outer) && shapes[0].holes.is_empty();

    let probe = |at: Vec2| {
        query(&index, &map, 0.0, at)
            .into_iter()
            .next()
            .map(|hit| hit.label.starts_with("sidewalk"))
            .unwrap_or(false)
    };
    // полотно — находка дороги, в форме полосы точки нет
    let at = Vec2::new(500.0, 70.0 + half - 0.1);
    assert!(!probe(at) && !contains(at));
    // внутри полосы — и находка, и форма
    let at = Vec2::new(500.0, 70.0 + half + 0.05);
    assert!(probe(at) && contains(at));
    let at = Vec2::new(500.0, 70.0 + sidewalk_edge - 0.05);
    assert!(probe(at) && contains(at));
    // за внешней кромкой — ни находки, ни формы
    let at = Vec2::new(500.0, 70.0 + sidewalk_edge + 0.1);
    assert!(!probe(at) && !contains(at));
}

#[test]
fn an_unbuilt_index_answers_nothing() {
    let map = MapData::default();
    let hits = query(&InspectIndex::default(), &map, 0.0, Vec2::ZERO);
    assert!(hits.is_empty());
}

#[test]
fn the_cap_cuts_the_list_with_a_tail() {
    let hits: Vec<Hit> = (0..5)
        .map(|i| Hit {
            z: i as f32,
            label: format!("x{i}"),
            src: HitSource::Osm,
            id: None,
            feature: FeatureRef::Tree(0),
        })
        .collect();
    assert_eq!(format_hits(&hits, 12).lines().count(), 5);
    let capped = format_hits(&hits, 3);
    assert_eq!(capped.lines().count(), 4);
    assert!(capped.ends_with("+2 more"));
}

/// Дефолт окна просмотра — игра (`Whole`), не витрина: игра ресурс не
/// пишет, и «снаружи» по умолчанию прятал бы ей панель инспектора и гасил
/// попадания с первого кадра.
#[test]
fn the_default_viewport_is_the_whole_map() {
    assert_eq!(InspectViewport::default(), InspectViewport::Whole);
}
