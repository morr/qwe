use super::*;

fn square() -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ]
}

/// Крошка в десятую квадратного метра на краю Тулы — там, где стоянка
/// 441737398 оставила её у угла забора: от начала координат шнурование
/// давало ей целый метр, от своей вершины — её собственную площадь.
#[test]
fn a_small_ring_far_from_the_origin_keeps_its_area() {
    let corner = Vec2::new(5775.27, 3192.40);
    let crumb = vec![
        corner,
        corner + Vec2::new(0.5, 0.0),
        corner + Vec2::new(0.5, 0.4),
    ];
    assert!(
        (ring_area(&crumb) - 0.1).abs() < 1e-3,
        "{}",
        ring_area(&crumb)
    );
    // знак — прежний: обход против часовой стрелки положителен
    assert!(signed_ring_area(&crumb) > 0.0);
    assert!(signed_ring_area(&crumb.iter().rev().copied().collect::<Vec<_>>()) < 0.0);
    assert_eq!(signed_ring_area(&[]), 0.0);
}

#[test]
fn point_in_polygon_square() {
    let ring = square();
    assert!(point_in_polygon(Vec2::new(5.0, 5.0), &ring));
    assert!(!point_in_polygon(Vec2::new(15.0, 5.0), &ring));
    assert!(!point_in_polygon(Vec2::new(-1.0, 5.0), &ring));
}

#[test]
fn point_in_area_respects_holes() {
    let area = PolyArea {
        outer: square(),
        holes: vec![vec![
            Vec2::new(4.0, 4.0),
            Vec2::new(6.0, 4.0),
            Vec2::new(6.0, 6.0),
            Vec2::new(4.0, 6.0),
        ]],
        kind: AreaKind::Building,
        building_use: BuildingUse::Other,
        height: None,
        storeys: None,
        entrances: Vec::new(),
        colours: Colours::default(),
        osm_id: None,
    };
    assert!(point_in_area(Vec2::new(2.0, 2.0), &area));
    assert!(!point_in_area(Vec2::new(5.0, 5.0), &area));
}

#[test]
fn ring_area_square() {
    assert!((ring_area(&square()) - 100.0).abs() < 1e-3);
}

/// Коробку решают трое: пятно, этажность и высота, — и отказывает любой.
/// Дом выше оболочки или с четвёртым этажом — жилой корпус с магазином
/// внизу либо многоэтажный ТЦ, а не коробка, сколько бы места он ни
/// занимал. Не сказано ни этажей, ни высоты — дом остаётся коробкой: её
/// выведет `buildings/heights.rs`, и ровно по этому предикату.
#[test]
fn a_retail_box_is_its_footprint_its_storeys_and_its_height() {
    let retail = |storeys, height| PolyArea {
        outer: vec![
            Vec2::ZERO,
            Vec2::new(60.0, 0.0),
            Vec2::new(60.0, 40.0),
            Vec2::new(0.0, 40.0),
        ],
        holes: Vec::new(),
        kind: AreaKind::Building,
        building_use: BuildingUse::Retail,
        height,
        storeys,
        entrances: Vec::new(),
        colours: Colours::default(),
        osm_id: None,
    };

    assert!(is_big_box(&retail(None, None)));
    assert!(is_big_box(&retail(
        Some(BIG_BOX_MAX_LEVELS),
        Some(BIG_BOX_MAX_HEIGHT)
    )));
    // ТРЦ «Макси»: два торговых уровня, 12.5 м
    assert!(is_big_box(&retail(Some(2.0), Some(12.5))));
    // ТЦ «Империя»: четыре жилых этажа — те же двенадцать метров, но не
    // коробка, и различает их только этажность
    assert!(!is_big_box(&retail(Some(4.0), Some(12.0))));
    // «Пятёрочка»: девять жилых этажей с `shop=supermarket` на первом
    assert!(!is_big_box(&retail(Some(9.0), Some(27.0))));
    // этажей не размечено — отвечает потолок высоты, и он один
    assert!(is_big_box(&retail(None, Some(BIG_BOX_MAX_HEIGHT))));
    assert!(!is_big_box(&retail(None, Some(BIG_BOX_MAX_HEIGHT + 0.5))));

    // пятно по-прежнему обязательно: та же высота на встройке в 200 м²
    let mut shop = retail(Some(1.0), Some(8.0));
    shop.outer = square();
    assert!(!is_big_box(&shop));
}

#[test]
fn polyline_length_sums_the_links() {
    let path = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
    assert!((polyline_length(&path) - 7.0).abs() < 1e-4);
}

#[test]
fn point_at_arc_length_walks_across_a_corner() {
    let path = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
    assert!(point_at_arc_length(&path, 1.5).distance(Vec2::new(1.5, 0.0)) < 1e-4);
    // ровно на изломе и за ним — второе звено
    assert!(point_at_arc_length(&path, 5.0).distance(Vec2::new(3.0, 2.0)) < 1e-4);
}

/// За концом обрыв, а не экстраполяция: расставлять что-либо дальше
/// ломаной некуда.
#[test]
fn point_at_arc_length_stops_at_the_end() {
    let path = [Vec2::ZERO, Vec2::new(3.0, 0.0)];
    assert_eq!(point_at_arc_length(&path, 99.0), Vec2::new(3.0, 0.0));
}

/// Звено нулевой длины пропускается, а не делит на ноль — в OSM-геометрии
/// после округления координат такие встречаются.
#[test]
fn point_at_arc_length_skips_a_zero_length_link() {
    let path = [Vec2::ZERO, Vec2::ZERO, Vec2::new(4.0, 0.0)];
    assert!(point_at_arc_length(&path, 2.0).distance(Vec2::new(2.0, 0.0)) < 1e-4);
}

/// Навмеш прорезают ровно мост и арка — по любому из двух флагов.
#[test]
fn a_bridge_or_an_arch_carves_the_navmesh() {
    let mut road = crate::map::osm::fixture::street(vec![Vec2::ZERO, Vec2::X], 7.0);
    assert!(!road.carves_navmesh());
    road.bridge = true;
    assert!(road.carves_navmesh());
    road.bridge = false;
    road.passage = true;
    assert!(road.carves_navmesh());
}

/// Самая длинная сторона — сторона контура по ходу его вершин, а не ось
/// описанного прямоугольника; у пустого контура её нет.
#[test]
fn the_longest_side_is_a_side_of_the_outline() {
    let area = |outer: Vec<Vec2>| PolyArea {
        outer,
        holes: Vec::new(),
        kind: AreaKind::Parking(LotKind::Kerbside),
        building_use: BuildingUse::Other,
        height: None,
        storeys: None,
        entrances: Vec::new(),
        colours: Colours::default(),
        osm_id: None,
    };
    // длинная сторона — верхняя, от (30, 4) к (0, 4)
    let lot = area(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, -1.0),
        Vec2::new(30.0, 4.0),
        Vec2::new(0.0, 4.0),
    ]);
    assert_eq!(lot.longest_side(), Some(Vec2::new(-1.0, 0.0)));
    assert_eq!(area(Vec::new()).longest_side(), None);
}

fn street(width: f32) -> RoadLine {
    crate::map::osm::fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], width)
}

/// Мост — проезжая часть: полоса у него по классу есть (со своих слоёв его
/// убирает рендер, а не профиль); у проезда полосы нет вовсе.
#[test]
fn a_bridge_keeps_its_band_and_a_service_drive_has_none() {
    let mut bridge = street(8.0);
    bridge.bridge = true;
    assert_eq!(bridge.sidewalk().band(), Some(sidewalk_band(8.0)));
    let mut service = street(8.0);
    service.highway = Highway::Service;
    let profile = service.sidewalk();
    assert_eq!(profile.band(), None);
    assert_eq!(profile.on(0), None);
    assert_eq!(
        profile.kerb_edge(4.0, 1.2),
        5.2,
        "без полосы — бордюр стоянки"
    );
    assert_eq!(profile.verge_edge(4.0, 2.0), None);
}

/// `sidewalk=right` — тротуар только справа по ходу: слева `on` пусто,
/// а край по карте один на обе стороны — с тротуаром.
#[test]
fn a_right_sidewalk_is_on_one_side_and_its_edge_on_both() {
    let mut road = street(8.0);
    road.sidewalks = [SidewalkSide::None, SidewalkSide::Tagged];
    let profile = road.sidewalk();
    let band = sidewalk_band(8.0);
    assert_eq!(profile.on(0), None);
    assert_eq!(profile.on(1), Some(band));
    assert_eq!(profile.any(), Some(band));
    assert!(!profile.both());
    assert_eq!(profile.mapped_edge(4.0), 4.0 + band);
}

/// Без тротуара с обеих сторон край по карте — голая кромка, а обочина
/// дома и бордюр стоянки остаются по классу: тег их не трогает.
#[test]
fn without_sidewalks_the_mapped_edge_is_the_kerb_and_the_verge_stays() {
    let mut road = street(8.0);
    road.sidewalks = [SidewalkSide::None; 2];
    let profile = road.sidewalk();
    let band = sidewalk_band(8.0);
    assert_eq!(profile.any(), None);
    assert_eq!(profile.mapped_edge(4.0), 4.0);
    assert_eq!(profile.verge_edge(4.0, 2.0), Some(4.0 + band + 2.0));
    assert_eq!(profile.kerb_edge(4.0, 1.2), 4.0 + band);
}
