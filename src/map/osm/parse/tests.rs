use super::fixtures::{
    CENTER, CITY, HALF, corners, fixture, read, right_angles, skewed_house, wood_scene,
};
use super::lots::PavedLots;
use super::*;
use crate::map::osm::MapData;
use crate::map::osm::fixture::{Overpass, building, rect, residential_half, square, water_area};
use crate::map::osm::model::Highway;
use crate::map::osm::model::sidewalk_band;
use bevy::math::Vec2;

#[test]
fn parses_building_road_and_multipolygon() {
    let map = fixture().parse();

    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.buildings[0].outer.len(), 4);

    // proposed отброшен, secondary-мост остался; без `lanes` — четыре полосы
    // по классу, 4 × 3.3 + две кромки
    assert_eq!(map.roads.len(), 1);
    assert!((map.roads[0].width - 14.2).abs() < 1e-4);
    assert!(map.roads[0].bridge);

    assert_eq!(map.water.len(), 1);
    let water = &map.water[0];
    assert_eq!(water.holes.len(), 1);
    // центр — в дырке-острове: суша
    assert!(!point_in_area(CENTER, water));
    // точка между границей и островом — вода
    assert!(point_in_area(CENTER + Vec2::new(0.0, 40.0), water));
}

/// Сторона движения — с границы, в которой лежит карта: так её отдаёт
/// `out tags` после `is_in`, relation без членов. Из нескольких границ
/// побеждает самая мелкая, без тега — правостороннее.
#[test]
fn takes_the_driving_side_from_the_innermost_boundary() {
    let country = |side| {
        [
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("driving_side", side),
        ]
    };
    let left = Overpass::new(CITY).relation(&country("left"), &[]).parse();
    assert_eq!(left.traffic_side, TrafficSide::Left);

    // регион после страны и перед ней: порядок в ответе ничего не решает
    let region = [
        ("boundary", "administrative"),
        ("admin_level", "4"),
        ("driving_side", "left"),
    ];
    let after = Overpass::new(CITY)
        .relation(&country("right"), &[])
        .relation(&region, &[])
        .parse();
    assert_eq!(after.traffic_side, TrafficSide::Left);
    let before = Overpass::new(CITY)
        .relation(&region, &[])
        .relation(&country("right"), &[])
        .parse();
    assert_eq!(before.traffic_side, TrafficSide::Left);

    let untagged = Overpass::new(CITY)
        .relation(&[("boundary", "administrative"), ("admin_level", "2")], &[])
        .parse();
    assert_eq!(untagged.traffic_side, TrafficSide::Right);
}

/// Три края дороги в разборе — пин до сведения их в профиль тротуара:
/// квартал тянется к **нарисованному** краю (тег `sidewalk=no` его сужает до
/// голой кромки, а односторонний тротуар — один край на обе стороны), а дом
/// отодвигается от **края обочины** по классу, тег не смотрит.
#[test]
fn the_parse_reads_the_mapped_edge_for_blocks_and_the_verge_for_houses() {
    let street = |y: f32| vec![CENTER + Vec2::new(-400.0, y), CENTER + Vec2::new(400.0, y)];
    // квартал под улицей (с юга), зазор 6.5 м от оси
    let block = |y: f32| {
        rect(
            CENTER + Vec2::new(-300.0, y - 40.0),
            CENTER + Vec2::new(-220.0, y - 6.5),
        )
    };
    let house = |y: f32| {
        rect(
            CENTER + Vec2::new(100.0, y + 4.7),
            CENTER + Vec2::new(112.0, y + 14.7),
        )
    };
    let map = Overpass::new(CITY)
        .way(
            &[("highway", "residential"), ("sidewalk", "no")],
            street(0.0),
        )
        .way(
            &[("highway", "residential"), ("sidewalk", "left")],
            street(500.0),
        )
        .area(&[("landuse", "residential")], block(0.0))
        .area(&[("landuse", "residential")], block(500.0))
        .area(&[("building", "yes")], house(0.0))
        .parse();

    let top = |ring: &[Vec2], y: f32| {
        ring.iter()
            .map(|vertex| vertex.y - CENTER.y - y)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let bare = top(&map.landuse[0].outer, 0.0);
    assert!(
        (bare + residential_half(ParseKnobs::DEFAULT) - LANDUSE_OVERLAP).abs() < 0.02,
        "без тротуара квартал тянется к голой кромке: {bare}"
    );
    // тротуар слева (с севера), квартал справа — край всё равно с тротуаром
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let one_sided = top(&map.landuse[1].outer, 500.0);
    assert!(
        (one_sided + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "односторонний тротуар — один край на обе стороны: {one_sided}"
    );
    let reach = edge + SIDEWALK_CLEARANCE;
    let gap = map.buildings[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        (gap - reach).abs() < 0.1,
        "дом отодвинут от обочины по классу, `sidewalk=no` не в счёт: {gap}"
    );
}

/// Ширина полосы — вход разбора ([`ParseKnobs`]), а не глобаль: тест называет
/// её сам, без мьютекса. Разбор с полосой на 0.3 м шире выводит двухполосное
/// сечение на 0.6 м шире, проезд — тоже шире на свою одну полосу, и карта
/// помнит, с какими входами разобрана.
#[test]
fn a_wider_lane_widens_the_section() {
    let scene = Overpass::new(CITY)
        .way(
            &[("highway", "residential")],
            vec![CENTER - Vec2::X * HALF, CENTER + Vec2::X * HALF],
        )
        .way(
            &[("highway", "service")],
            vec![
                CENTER + Vec2::new(-HALF, 40.0),
                CENTER + Vec2::new(HALF, 40.0),
            ],
        );
    let wider = ParseKnobs {
        lane_width: ParseKnobs::DEFAULT.lane_width + 0.3,
        ..ParseKnobs::DEFAULT
    };
    let by_default = scene.parse();
    let widened = scene.parse_with(wider);

    let street = |map: &MapData| {
        map.roads
            .iter()
            .find(|road| road.highway == Highway::Residential)
            .unwrap()
            .width
    };
    let drive = |map: &MapData| {
        map.roads
            .iter()
            .find(|road| road.highway == Highway::Service)
            .unwrap()
            .width
    };
    assert!((street(&by_default) - 2.0 * residential_half(ParseKnobs::DEFAULT)).abs() < 1e-4);
    assert!((street(&by_default) - 7.6).abs() < 1e-4);
    assert!((street(&widened) - 2.0 * residential_half(wider)).abs() < 1e-4);
    assert!((street(&widened) - street(&by_default) - 0.6).abs() < 1e-4);
    assert!((drive(&widened) - drive(&by_default) - 0.3).abs() < 1e-4);
    assert_eq!(by_default.knobs, ParseKnobs::DEFAULT);
    assert_eq!(widened.knobs, wider);
}

/// Чтение элементов ничего не доводит: дом посреди пруда из него выходит
/// живым, и топит его отдельный проход.
#[test]
fn reading_the_elements_leaves_the_passes_undone() {
    let scene = Overpass::new(CITY)
        .area(&[("natural", "water")], square(CENTER, 100.0))
        .area(&[("building", "yes")], square(CENTER, 10.0));

    let (mut map, _, _) = read(&scene);
    assert_eq!(map.buildings.len(), 1, "сарай посреди пруда ещё стоит");

    let drowned = drop_buildings_in_water(&mut map);
    assert_eq!(drowned, 1);
    assert!(map.buildings.is_empty());
}

/// Вторая половина шва тоже отчитывается **значением**: что элементный цикл
/// узнал про сторону движения и сколько колец бросил, раньше можно было
/// прочесть только на stderr.
#[test]
fn reading_the_elements_reports_what_it_skipped() {
    let (sw, se, ..) = corners(HALF);
    let torn = Overpass::new(CITY)
        .relation(
            &[
                ("boundary", "administrative"),
                ("admin_level", "2"),
                ("driving_side", "left"),
            ],
            &[],
        )
        // кольцо из одного куска в две точки: соединять не с чем, замкнуть
        // нечем — такое бросается и считается
        .relation(&[("natural", "water")], &[("outer", vec![sw, se])]);

    let (map, _, report) = read(&torn);
    assert_eq!(report.unclosed_rings, 1);
    assert!(map.water.is_empty(), "порванное кольцо не стало водой");
    assert_eq!(report.traffic_side, Some(TrafficSide::Left));

    // без границы в ответе сторона движения неизвестна, и карта рисуется
    // правосторонней — отчёт говорит именно «неизвестна», а не «правая»
    let (map, _, report) = read(&Overpass::new(CITY));
    assert_eq!(report.unclosed_rings, 0);
    assert!(report.traffic_side.is_none());
    assert_eq!(map.traffic_side, TrafficSide::Right);
}

/// Проход зовётся сам по себе, на карте, собранной руками, — без JSON, без
/// `GeoBounds` и без остальных шести проходов.
#[test]
fn a_pass_runs_on_a_hand_built_map() {
    let mut map = MapData {
        buildings: vec![
            building(square(CENTER, 10.0), Vec::new()),
            building(square(CENTER + Vec2::new(500.0, 0.0), 10.0), Vec::new()),
        ],
        water: vec![water_area(square(CENTER, 100.0), Vec::new())],
        ..MapData::default()
    };

    assert_eq!(drop_buildings_in_water(&mut map), 1);
    assert_eq!(map.buildings.len(), 1, "дом вдали от воды остался");
}

/// Доводка целиком — одним вызовом, и она отчитывается значением: те же
/// счётчики, что уходили в лог восемью `eprintln!`, теперь можно сравнить.
#[test]
fn finishing_the_parse_reports_what_each_pass_did() {
    let scene = Overpass::new(CITY)
        .area(&[("natural", "water")], square(CENTER, 100.0))
        // тонет
        .area(&[("building", "yes")], square(CENTER, 10.0))
        // выпрямляется: косой домик вдали от воды и дорог
        .area(
            &[("building", "house")],
            skewed_house(CENTER + Vec2::new(800.0, 0.0)),
        )
        // сажается и собирается: одиночное дерево вдали от всего остального
        .node(
            &[("natural", "tree"), ("diameter_crown", "10")],
            CENTER + Vec2::new(-800.0, 0.0),
        );

    let (mut map, pending, _) = read(&scene);
    let report = finish_parse(&mut map, &pending, ParseKnobs::default());

    assert_eq!(report.drowned, 1);
    assert_eq!(report.squared, 1);
    assert_eq!(report.entrances_found, 0);
    assert_eq!(report.entrances_orphaned, 0);
    // и счётчики остальных проходов — тоже значением, а не строкой на stderr
    assert_eq!(
        report.pulled.moved, 0,
        "дорог в сцене нет, отодвигать не от чего"
    );
    assert_eq!(report.pulled.left, 0);
    assert_eq!(report.stretched.blocks, 0, "кварталов в сцене нет");
    assert_eq!(
        report.lots,
        PavedLots::default(),
        "и стоянок тоже — ни выросших, ни обрезанных отмосткой"
    );
    assert_eq!(report.planted.tree_nodes, 1);
    assert_eq!(report.planted.standalone, 1);
    assert_eq!(map.buildings.len(), 1, "остался только косой домик");
    // и деревья собраны по составу по умолчанию, а не оставлены несобранными:
    // посаженная одиночка доехала до набора, который читает рендер
    assert_eq!(map.standalone_trees.len(), 1, "дерево посажено");
    assert_eq!(map.trees.len(), 1, "и собрано в набор рендера");
    assert_eq!(map.composed_for, Some(TreeCompose::default()));
}

/// Сцена сырого OSM: косой домик с размеченным входом, дом без входов и
/// жилая улица без тегов `lanes` и `sidewalk*`.
fn raw_scene(raw: RawOsm) -> MapData {
    let (sw, se, ..) = corners(HALF);
    Overpass::new(CITY)
        .node(&[("entrance", "main")], skewed_house(CENTER)[1])
        .area(&[("building", "house")], skewed_house(CENTER))
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(0.0, 30.0), 8.0),
        )
        .way(&[("highway", "residential")], vec![sw, se])
        .parse_with(ParseKnobs {
            raw,
            ..ParseKnobs::DEFAULT
        })
}

/// **Сырой OSM** (`RawOsm::Parse`) пропускает доводочные проходы: косой
/// домик остаётся косым, дом без размеченных входов — без дверей, у улицы без
/// тега нет ни выведенных полос, ни достроенного тротуара. Размеченный вход —
/// данные — остаётся. Без флага те же проходы идут.
#[test]
fn raw_osm_skips_the_finishing_passes() {
    let cooked = raw_scene(RawOsm::Off);
    let raw = raw_scene(RawOsm::Parse);

    let squared = &cooked.buildings[0];
    assert!(right_angles(&squared.outer), "без флага домик выпрямлен");
    let skewed = &raw.buildings[0];
    assert!(!right_angles(&skewed.outer), "сырой домик остаётся косым");
    for (data, kept) in skewed_house(CENTER).iter().zip(&skewed.outer) {
        assert!(
            data.distance(*kept) < 0.01,
            "вершина {data} уехала в {kept}"
        );
    }
    assert_eq!(
        raw.buildings[0].entrances.len(),
        1,
        "размеченный вход — данные"
    );

    assert!(
        !cooked.buildings[1].entrances.is_empty(),
        "без флага двери сочиняются"
    );
    assert!(
        raw.buildings[1].entrances.is_empty(),
        "сырой дом без сочинённых дверей"
    );

    assert_eq!(cooked.roads[0].lanes, Some(2), "без флага полосы по классу");
    assert_eq!(raw.roads[0].lanes, None, "сырая улица без выведенных полос");
    assert!(
        raw.roads[0]
            .sidewalks
            .iter()
            .all(|side| *side == SidewalkSide::None),
        "тротуар без тега снят: {:?}",
        raw.roads[0].sidewalks
    );
    // сеть склеивается и в сыром режиме: её читают отрисовка и оверлей
    assert_eq!(raw.network.streets.len(), 1);
    assert_eq!(raw.knobs.raw, RawOsm::Parse);
}

/// Второй уровень сырого OSM (`RawOsm::Draw`) не сажает сгенерированных
/// деревьев — ни леса, ни аллей; первый уровень сажает, как без флага.
#[test]
fn raw_osm_draw_plants_no_generated_trees() {
    let knobs = |raw| ParseKnobs {
        raw,
        ..ParseKnobs::DEFAULT
    };
    assert!(
        !wood_scene()
            .parse_with(knobs(RawOsm::Parse))
            .wood_trees
            .is_empty()
    );
    let raw = wood_scene().parse_with(knobs(RawOsm::Draw));
    assert!(raw.wood_trees.is_empty());
    assert_eq!(raw.trees.len(), 0);
}

#[cfg(test)]
mod areas;
#[cfg(test)]
mod roads;
#[cfg(test)]
mod trees;
