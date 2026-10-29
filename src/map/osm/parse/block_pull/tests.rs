use super::super::ParseKnobs;
use super::super::fixtures::{CENTER, CITY};
use super::*;
use crate::map::osm::MapData;
use crate::map::osm::fixture::{
    Overpass, building, closed, rect, residential_half, square, street,
};
use crate::map::osm::model::sidewalk_band;
use crate::map::osm::model::{AreaKind, Highway, Pavement, PolyArea, RoadClass, SidewalkSide};
use bevy::math::Vec2;

/// Край квартала, которому до нарисованного полотна осталась пара метров,
/// дотягивается **под** асфальт; квартал в стороне остаётся на месте, и улица
/// внутри квартала его границу к себе не стягивает — зелень только растёт.
#[test]
fn a_block_edge_is_pulled_under_the_asphalt() {
    // residential 7.6 м: край полотна с тротуаром — 3.8 + 1.67 от оси
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let street = vec![
        CENTER - Vec2::new(400.0, 0.0),
        CENTER + Vec2::new(400.0, 0.0),
    ];
    let block = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, -40.0),
            CENTER + Vec2::new(x + 80.0, -gap),
        )
    };
    let near = block(-300.0, 6.5);
    let far = block(-200.0, 12.0);
    // улица идёт внутри квартала, в 7 м от его южной границы
    let around = rect(
        CENTER + Vec2::new(-100.0, -7.0),
        CENTER + Vec2::new(-20.0, 60.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential"), ("sidewalk", "both")], street)
        .area(&[("landuse", "residential")], near)
        .area(&[("landuse", "residential")], far.clone())
        .area(&[("landuse", "residential")], around)
        .parse();

    let edges = |ring: &[Vec2]| {
        ring.iter()
            .map(|vertex| vertex.y - CENTER.y)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
                (low.min(y), high.max(y))
            })
    };
    let (_, top) = edges(&map.landuse[0].outer);
    assert!(
        (top + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "край квартала не заведён под полотно: {top}"
    );
    assert!(
        map.landuse[1]
            .outer
            .iter()
            .zip(&far)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "квартал в стороне от улицы тронут"
    );
    let (low, high) = edges(&map.landuse[2].outer);
    assert!(
        (low + 7.0).abs() < 0.01 && (high - 60.0).abs() < 0.01,
        "улица внутри квартала стянула его границу к себе: {low}..{high}"
    );
}

/// Дотягивание кварталов до дорог — тоже само по себе и тоже с ненулевым
/// счётчиком: у ближнего квартала под асфальт уходит весь его верхний край,
/// дальний не трогают.
#[test]
fn pulling_the_blocks_to_the_roads_runs_on_its_own() {
    // residential 7.6 м: край полотна с тротуаром — 3.8 + 1.67 от оси
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let block = |ring: Vec<Vec2>| PolyArea {
        kind: AreaKind::Residential,
        ..building(ring, Vec::new())
    };
    let near = rect(
        CENTER + Vec2::new(-40.0, -40.0),
        CENTER + Vec2::new(40.0, -6.5),
    );
    let far = rect(
        CENTER + Vec2::new(60.0, -40.0),
        CENTER + Vec2::new(140.0, -14.0),
    );
    let mut map = MapData {
        roads: vec![street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            2.0 * residential_half(ParseKnobs::DEFAULT),
        )],
        landuse: vec![block(near), block(far.clone())],
        ..MapData::default()
    };

    let stretched = pull_areas_to_roads(&mut map).blocks;
    assert!(stretched >= 2, "дотянуто вершин: {stretched}");
    let top = map.landuse[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (top + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "край квартала не заведён под полотно: {top}"
    );
    assert!(
        map.landuse[1]
            .outer
            .iter()
            .zip(&far)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "квартал в стороне от улицы тронут"
    );
}

/// Квартал нарисован до бордюра, а тротуар замаплен дорожкой внутри него
/// (Берлин, витрина 03): край квартала между дорожкой и улицей уходит под
/// дорожку — иначе двор торчит из-под тротуара серпом. Без улицы снаружи
/// дорожка край не трогает: зелень только растёт.
#[test]
fn a_block_drawn_to_the_kerb_is_tucked_under_its_sidewalk_footway() {
    let footway = RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..street(
            vec![
                CENTER + Vec2::new(-400.0, -7.0),
                CENTER + Vec2::new(400.0, -7.0),
            ],
            3.5,
        )
    };
    let carriageway = RoadLine {
        sidewalks: [SidewalkSide::None; 2],
        ..street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            2.0 * residential_half(ParseKnobs::DEFAULT),
        )
    };
    // край квартала — в 5 м от оси улицы: в четверти метра над тротуаром
    let ring = rect(
        CENTER + Vec2::new(-40.0, -40.0),
        CENTER + Vec2::new(40.0, -5.0),
    );
    let top = |roads: Vec<RoadLine>| {
        let mut map = MapData {
            roads,
            landuse: vec![PolyArea {
                kind: AreaKind::Residential,
                ..building(ring.clone(), Vec::new())
            }],
            ..MapData::default()
        };
        pull_areas_to_roads(&mut map);
        map.landuse[0]
            .outer
            .iter()
            .map(|vertex| vertex.y - CENTER.y)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    // тротуара у улицы нет (`sidewalk=separate`) — край идёт под её бордюр,
    // а не под полосу несуществующего тротуара
    let pulled = top(vec![carriageway.clone()]);
    assert!(
        (pulled + residential_half(ParseKnobs::DEFAULT) - LANDUSE_OVERLAP).abs() < 0.02,
        "край не у бордюра: {pulled}"
    );
    let tucked = top(vec![carriageway.clone(), footway.clone()]);
    // до оси дорожки: край прямой между вершинами, дорожка гнётся
    assert!(
        (tucked + 7.0).abs() < 0.02,
        "край не ушёл под тротуар: {tucked}"
    );
    assert!(
        (top(vec![footway.clone()]) + 5.0).abs() < 0.01,
        "без улицы край тронут"
    );
    // угол квартала лежит под другой дорожкой — переходом к улице: он уходит
    // под тротуар вместе с краем, а не остаётся клином двора (Тула, витрина 21)
    let crossing = RoadLine {
        points: vec![
            CENTER + Vec2::new(40.0, -30.0),
            CENTER + Vec2::new(40.0, 0.0),
        ],
        ..footway.clone()
    };
    let cornered = top(vec![carriageway, footway, crossing]);
    assert!(
        (cornered + 7.0).abs() < 0.02,
        "угол под переходом не ушёл под тротуар: {cornered}"
    );
}

/// Край квартала в обочине — между кромкой стороны без тротуара
/// (`sidewalk=separate`) и дорожкой за ней — уходит к тому, что ближе: к
/// дорожке — под неё, к кромке — под асфальт. Раньше он стоял «под полотном»
/// по краю с тротуаром другой стороны и оставался в обочине, а обочина лежит
/// под кварталом — угол двора торчал из плитки тёмным штрихом (Тула, витрина
/// 15, угол Ленина и Советской).
#[test]
fn a_block_edge_in_the_verge_of_a_bare_side_goes_to_the_nearer_of_its_rims() {
    // улица на восток: тротуар слева (с севера), справа — дорожка за обочиной
    let scene = |edge: f32, footway: f32| {
        let mut road = street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            2.0 * residential_half(ParseKnobs::DEFAULT),
        );
        road.sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
        let path = RoadLine {
            pavement: Some(Pavement::Paved),
            ..crate::map::osm::fixture::footway(vec![
                CENTER + Vec2::new(-400.0, -footway),
                CENTER + Vec2::new(400.0, -footway),
            ])
        };
        let mut map = MapData {
            roads: vec![road, path],
            landuse: vec![PolyArea {
                kind: AreaKind::Residential,
                ..building(
                    rect(
                        CENTER + Vec2::new(-40.0, -40.0),
                        CENTER + Vec2::new(40.0, -edge),
                    ),
                    Vec::new(),
                )
            }],
            ..MapData::default()
        };
        pull_areas_to_roads(&mut map);
        map.landuse[0]
            .outer
            .iter()
            .map(|vertex| vertex.y - CENTER.y)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    // в 5 м от оси: за кромкой (3.8), но в полосе по карте (3.8 + 1.67), в
    // 0.75 м от полосы дорожки — под дорожку
    let tucked = scene(5.0, 7.5);
    assert!(
        (tucked + 7.5).abs() < 0.02,
        "край остался в обочине: {tucked}"
    );
    // в 4.5 м — к кромке ближе, чем к дорожке в 9 м: под асфальт
    let pulled = scene(4.5, 9.0);
    assert!(
        (pulled + residential_half(ParseKnobs::DEFAULT) - LANDUSE_OVERLAP).abs() < 0.02,
        "край не дотянут до кромки: {pulled}"
    );
}

/// Щепка двора на плитке обочины вырезается **площадью**, что бы ни решили
/// вершины: угол квартала, оставшийся у кромки, пока соседи ушли под дорожку,
/// давал щепку на плитке (Тула, витрина 15, развилка дорожек у Ленина, 15).
/// Край выреза — кромка обочины без полуметра, под лентой дорожки. Широкая
/// полоса двора до бордюра — газон по данным — остаётся; квартал, обочины не
/// задевший, остаётся теми же кольцами; у широкой обочины вырезается только
/// плитка у бордюра, газон под двором — та же трава.
#[test]
fn a_sliver_of_block_on_the_verge_tiles_is_cut_back_under_the_footway() {
    let half = residential_half(ParseKnobs::DEFAULT);
    // квартал ниже улицы — клином с углом `tip` метров от оси
    let wedge = |tip: f32| {
        vec![
            CENTER + Vec2::new(-20.0, -40.0),
            CENTER + Vec2::new(20.0, -40.0),
            CENTER + Vec2::new(20.0, -10.0),
            CENTER + Vec2::new(0.0, -tip),
            CENTER + Vec2::new(-20.0, -10.0),
        ]
    };
    // улица на восток, обочина `verge` справа (с юга)
    let scene = |verge: f32, ring: Vec<Vec2>| {
        let mut road = street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            2.0 * half,
        );
        road.verges = [0.0, verge];
        let block = PolyArea {
            kind: AreaKind::Residential,
            ..building(ring, Vec::new())
        };
        let mut map = MapData {
            roads: vec![road],
            landuse: vec![block.clone()],
            ..MapData::default()
        };
        let cut = verges::cut_verges_from_blocks(&mut map);
        (cut, block, map.landuse)
    };
    let top = |blocks: &[PolyArea]| {
        blocks
            .iter()
            .flat_map(|block| &block.outer)
            .map(|vertex| vertex.y - CENTER.y)
            .fold(f32::NEG_INFINITY, f32::max)
    };

    // угол в метре за кромкой, обочина в 3 м: клин полтора метра высотой —
    // щепка, срезан до 0.5 м от края обочины
    let (cut, _, blocks) = scene(3.0, wedge(half + 1.0));
    assert_eq!(cut, 1);
    assert_eq!(blocks.len(), 1, "квартал развалился: {blocks:?}");
    let edge = -(half + 3.0 - LANDUSE_OVERLAP);
    assert!(
        (top(&blocks) - edge).abs() < 0.02,
        "двор на плитке обочины: верх {} при крае выреза {edge}",
        top(&blocks)
    );

    // квартал до кромки полосой в 40 м: два метра двора на плитке — газон,
    // не щепка, и остаётся
    let (cut, block, blocks) = scene(
        3.0,
        rect(
            CENTER + Vec2::new(-20.0, -40.0),
            CENTER + Vec2::new(20.0, -(half + 0.5)),
        ),
    );
    assert_eq!(cut, 0);
    assert_eq!(blocks[0].outer, block.outer);

    // угол за обочиной: квартал не тронут, кольца те же
    let (cut, block, blocks) = scene(3.0, wedge(half + 3.0));
    assert_eq!(cut, 0);
    assert_eq!(blocks[0].outer, block.outer);

    // обочина в 8 м — газон под плиткой у бордюра (0.5 м): угол в метре за
    // кромкой лежит на газоне и остаётся
    let (cut, block, blocks) = scene(8.0, wedge(half + 1.0));
    assert_eq!(cut, 0);
    assert_eq!(blocks[0].outer, block.outer);
}

/// Порядок [`finish_parse`]: сечения (шаг 0) раньше дотягивания кварталов
/// (шаг 6) — квартал тянется к краю четырёхполосной улицы, а не к ширине
/// класса, с которой way вышел из чтения.
#[test]
fn blocks_are_pulled_to_the_width_the_sections_gave() {
    let street = vec![
        CENTER - Vec2::new(400.0, 0.0),
        CENTER + Vec2::new(400.0, 0.0),
    ];
    let block = rect(
        CENTER + Vec2::new(-300.0, -60.0),
        CENTER + Vec2::new(-220.0, -12.0),
    );
    let map = Overpass::new(CITY)
        .way(
            &[
                ("highway", "residential"),
                ("lanes", "4"),
                ("sidewalk", "both"),
            ],
            street,
        )
        .area(&[("landuse", "residential")], block)
        .parse();

    let width = map.roads[0].width;
    assert!(width > 12.0, "сечение не дошло до ширины: {width}");
    let edge = width / 2.0 + sidewalk_band(width);
    let top = map.landuse[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (top + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "квартал дотянут не к краю по сечению: {top}, край {edge}"
    );
}

/// Угол квартала у перекрёстка двух улиц дотягивается под **оба** полотна, а
/// не под одно ближайшее: иначе у скругления оставался треугольник голой земли
/// (Тула, витрина 13). Угол тянется и с зазора больше обычного предела — он
/// отступает от перекрёстка по биссектрисе.
#[test]
fn a_block_corner_at_a_crossing_is_pulled_under_both_streets() {
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    for gap in [3.0, 7.0] {
        let corner = edge + gap;
        let map = Overpass::new(CITY)
            .way(
                &[("highway", "residential"), ("sidewalk", "both")],
                vec![
                    CENTER - Vec2::new(200.0, 0.0),
                    CENTER,
                    CENTER + Vec2::new(200.0, 0.0),
                ],
            )
            .way(
                &[("highway", "residential"), ("sidewalk", "both")],
                vec![
                    CENTER - Vec2::new(0.0, 200.0),
                    CENTER,
                    CENTER + Vec2::new(0.0, 200.0),
                ],
            )
            .area(
                &[("landuse", "residential")],
                rect(
                    CENTER + Vec2::splat(corner),
                    CENTER + Vec2::splat(corner + 60.0),
                ),
            )
            .parse();
        let near = map.landuse[0]
            .outer
            .iter()
            .map(|vertex| *vertex - CENTER)
            .min_by(|a, b| a.length().total_cmp(&b.length()))
            .unwrap();
        let under = edge - LANDUSE_OVERLAP;
        assert!(
            (near.x - under).abs() < 0.05 && (near.y - under).abs() < 0.05,
            "зазор {gap}: угол {near} не под обоими полотнами ({under})"
        );
    }
}

/// Дырка в квартале, сквозь которую идёт улица, **сжимается** к ней: зелень у
/// дырки — тот же край двора, и подходить к полотну обязан он.
#[test]
fn a_street_in_a_courtyard_shrinks_the_hole_to_its_asphalt() {
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let map = Overpass::new(CITY)
        .way(
            &[("highway", "residential"), ("sidewalk", "both")],
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
        )
        .relation(
            &[("landuse", "residential"), ("type", "multipolygon")],
            &[
                ("outer", closed(square(CENTER, 100.0))),
                (
                    "inner",
                    closed(rect(
                        CENTER + Vec2::new(-60.0, -7.0),
                        CENTER + Vec2::new(60.0, 7.0),
                    )),
                ),
            ],
        )
        .parse();

    let hole = &map.landuse[0].holes[0];
    let reach = hole
        .iter()
        .map(|vertex| (vertex.y - CENTER.y).abs())
        .fold(0.0, f32::max);
    assert!(
        (reach - (edge - LANDUSE_OVERLAP)).abs() < 0.02,
        "край дырки не подошёл к полотну: {reach}"
    );
}
