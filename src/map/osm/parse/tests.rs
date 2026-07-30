use super::*;
// посадка деревьев переехала в соседний модуль, но проверяется она через
// весь конвейер — от JSON Overpass до `map.trees`
use crate::map::osm::model::distance_to_segment;
use crate::map::osm::planting::{
    TREE_CROWN_REACH, TREE_MIN_SPACING, TREE_SHORE_CLEARANCE, TREE_WALL_CLEARANCE, near_area_edge,
};
use crate::settings::MAP_SIZE;

/// Фикстуры строятся вокруг гео-центра Тулы — города по умолчанию.
const CITY: City = City::Tula;

/// Мини-ответ Overpass: way-здание, дорога-мост, relation-вода из двух
/// половинок с дыркой-островом.
fn fixture() -> String {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.0005; // ~55 м по широте
    format!(
        r#"{{"elements": [
  {{"type": "way", "id": 1, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "way", "id": 2, "tags": {{"highway": "secondary", "bridge": "yes"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {e}, "lon": {c}}}]}},
  {{"type": "way", "id": 3, "tags": {{"highway": "proposed"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {e}, "lon": {c}}}]}},
  {{"type": "relation", "id": 4, "tags": {{"natural": "water"}},
    "members": [
      {{"type": "way", "role": "outer", "geometry": [
        {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}}, {{"lat": {e}, "lon": {c}}}]}},
      {{"type": "way", "role": "outer", "geometry": [
        {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}}, {{"lat": {a}, "lon": {b}}}]}},
      {{"type": "way", "role": "inner", "geometry": [
        {{"lat": {i1}, "lon": {j1}}}, {{"lat": {i1}, "lon": {j2}}},
        {{"lat": {i2}, "lon": {j2}}}, {{"lat": {i2}, "lon": {j1}}},
        {{"lat": {i1}, "lon": {j1}}}]}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
        i1 = lat - d / 4.0,
        i2 = lat + d / 4.0,
        j1 = lon - d / 4.0,
        j2 = lon + d / 4.0,
    )
}

#[test]
fn parses_building_road_and_multipolygon() {
    let map = parse(&fixture(), CITY).unwrap();

    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.buildings[0].outer.len(), 4);

    // proposed отброшен, secondary-мост остался
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].width, 12.0);
    assert!(map.roads[0].bridge);

    assert_eq!(map.water.len(), 1);
    let water = &map.water[0];
    assert_eq!(water.holes.len(), 1);
    // центр — в дырке-острове: суша
    assert!(!point_in_area(MAP_SIZE / 2.0, water));
    // точка между границей и островом — вода
    let near_edge = MAP_SIZE / 2.0 + Vec2::new(0.0, 40.0);
    assert!(point_in_area(near_edge, water));
}

#[test]
fn parses_rails_and_drops_station_furniture() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.0005;
    // путь, платформа (отбрасывается), заброшенная ветка и трамвай на улице
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 20, "tags": {{"railway": "rail"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {e}, "lon": {c}}}]}},
  {{"type": "way", "id": 21, "tags": {{"railway": "platform"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {e}, "lon": {c}}}]}},
  {{"type": "way", "id": 22, "tags": {{"railway": "abandoned"}},
    "geometry": [{{"lat": {a}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}}]}},
  {{"type": "way", "id": 23, "tags": {{"railway": "tram", "highway": "residential"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();

    assert_eq!(map.rails.len(), 3, "platform must not become a rail");
    assert_eq!(map.rails[0].width, 5.0);
    assert_eq!(map.rails[0].kind, RailKind::Active);
    assert_eq!(map.rails[1].kind, RailKind::Disused);

    // трамвайный путь на улице — это и улица, и путь: way попадает в оба списка
    assert_eq!(map.rails[2].kind, RailKind::Active);
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].width, 8.0);
    assert_eq!(map.roads[0].points, map.rails[2].points);

    // рельсы существуют только для картинки — навмеша они не касаются
    assert!(map.walls.is_empty());
    assert!(map.buildings.is_empty());
}

#[test]
fn trees_are_deterministic_and_inside_the_wood() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"natural": "wood"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let first = parse(&json, CITY).unwrap();
    let second = parse(&json, CITY).unwrap();
    assert!(!first.trees.is_empty());
    assert_eq!(first.trees, second.trees);
    for &(pos, radius) in &first.trees {
        assert!(point_in_area(pos, &first.woods[0]), "{pos:?}");
        assert!((2.5..=4.0).contains(&radius));
    }
    for (index, &(pos, _)) in first.trees.iter().enumerate() {
        for &(other, _) in &first.trees[index + 1..] {
            assert!(
                pos.distance(other) >= TREE_MIN_SPACING,
                "trees too close: {pos:?} vs {other:?}"
            );
        }
    }
}

#[test]
fn trees_avoid_a_pond_inside_the_wood() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    // пруд занимает северо-восточную четверть массива
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"natural": "wood"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "way", "id": 11, "tags": {{"natural": "water"}},
    "geometry": [
      {{"lat": {lat}, "lon": {lon}}}, {{"lat": {lat}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {lon}}},
      {{"lat": {lat}, "lon": {lon}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.water.len(), 1);
    assert!(!map.trees.is_empty());
    let pond = &map.water[0];
    for &(pos, _) in &map.trees {
        assert!(!point_in_area(pos, pond), "tree in the pond at {pos:?}");
        assert!(
            !near_area_edge(pos, pond, TREE_SHORE_CLEARANCE),
            "tree on the shoreline at {pos:?}"
        );
    }
}

/// Дом целиком в пруду выбрасывается, дом на берегу с одним углом в воде —
/// остаётся (пирс, набережная).
#[test]
fn buildings_standing_in_water_are_dropped() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    // пруд — квадрат вокруг центра; первый дом внутри него, второй сидит на
    // южном берегу и заходит в воду только верхней парой углов
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"natural": "water"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "way", "id": 11, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {lat}, "lon": {lon}}}, {{"lat": {lat}, "lon": {h}}},
      {{"lat": {g}, "lon": {h}}}, {{"lat": {g}, "lon": {lon}}},
      {{"lat": {lat}, "lon": {lon}}}]}},
  {{"type": "way", "id": 12, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {f}, "lon": {lon}}}, {{"lat": {f}, "lon": {h}}},
      {{"lat": {g}, "lon": {h}}}, {{"lat": {g}, "lon": {lon}}},
      {{"lat": {f}, "lon": {lon}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
        f = lat - d * 2.0,
        g = lat - d * 0.5,
        h = lon + d * 0.5,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.water.len(), 1);
    assert_eq!(map.buildings.len(), 1, "only the shore building survives");
    // у выжившего есть угол вне воды
    let survivor = &map.buildings[0];
    assert!(
        survivor
            .outer
            .iter()
            .any(|&point| !point_in_area(point, &map.water[0]))
    );
}

#[test]
fn trees_avoid_grass_and_sand_inside_the_wood() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    // луг — восточная половина массива, песок — северо-западная четверть
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"natural": "wood"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "way", "id": 11, "tags": {{"landuse": "meadow"}},
    "geometry": [
      {{"lat": {a}, "lon": {lon}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {lon}}},
      {{"lat": {a}, "lon": {lon}}}]}},
  {{"type": "way", "id": 12, "tags": {{"natural": "beach"}},
    "geometry": [
      {{"lat": {lat}, "lon": {b}}}, {{"lat": {lat}, "lon": {lon}}},
      {{"lat": {e}, "lon": {lon}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {lat}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.woods.len(), 1);
    assert_eq!(map.grass.len(), 1);
    assert_eq!(map.sand.len(), 1);
    assert!(!map.trees.is_empty());
    for &(pos, _) in &map.trees {
        assert!(
            !point_in_area(pos, &map.grass[0]),
            "tree on grass at {pos:?}"
        );
        assert!(!point_in_area(pos, &map.sand[0]), "tree on sand at {pos:?}");
    }
}

/// Дом посреди массива: крона не должна наползать на стену — раньше
/// проверка была только «центр внутри полигона», и дерево вырастало
/// впритык к фасаду (Тула, павильон в парке).
#[test]
fn trees_keep_the_crown_off_walls_and_kerbs() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    // дом — центральная четверть массива, дорожка режет массив по диагонали
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"natural": "wood"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "way", "id": 11, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {h1}, "lon": {k1}}}, {{"lat": {h1}, "lon": {k2}}},
      {{"lat": {h2}, "lon": {k2}}}, {{"lat": {h2}, "lon": {k1}}},
      {{"lat": {h1}, "lon": {k1}}}]}},
  {{"type": "way", "id": 12, "tags": {{"highway": "footway"}},
    "geometry": [{{"lat": {a}, "lon": {b}}}, {{"lat": {e}, "lon": {c}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
        h1 = lat - d / 4.0,
        h2 = lat + d / 4.0,
        k1 = lon - d / 4.0,
        k2 = lon + d / 4.0,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.roads.len(), 1);
    assert!(!map.trees.is_empty());
    let house = &map.buildings[0];
    let path = &map.roads[0];
    for &(pos, radius) in &map.trees {
        assert!(
            !point_in_area(pos, house),
            "tree inside the house at {pos:?}"
        );
        assert!(
            !near_area_edge(pos, house, radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE),
            "crown on the wall at {pos:?}"
        );
        let kerb = path
            .points
            .windows(2)
            .map(|segment| distance_to_segment(pos, segment[0], segment[1]))
            .fold(f32::INFINITY, f32::min)
            - path.width / 2.0;
        assert!(kerb > radius, "tree on the kerb at {pos:?}, gap {kerb}");
    }
}

/// Открытая часть парка — поле: деревья растут только в лесных полигонах,
/// парк без `natural=wood` остаётся пустым.
#[test]
fn a_park_without_wood_grows_no_trees() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.001;
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 10, "tags": {{"leisure": "park"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.parks.len(), 1);
    assert!(map.woods.is_empty());
    assert!(map.trees.is_empty());
}

#[test]
fn measures_parse_with_units_commas_and_feet() {
    assert_eq!(parse_measure("12"), Some(12.0));
    assert_eq!(parse_measure("12.5"), Some(12.5));
    assert_eq!(parse_measure("12,5"), Some(12.5));
    assert_eq!(parse_measure("12 m"), Some(12.0));
    assert_eq!(parse_measure("12.5 metres"), Some(12.5));
    // несколько значений — берём первое
    assert_eq!(parse_measure("3;4"), Some(3.0));
    assert_eq!(parse_measure("40'"), Some(12.192));
    let inches = parse_measure("40'6\"").unwrap();
    assert!((inches - 12.3444).abs() < 1e-3, "{inches}");
    assert_eq!(parse_measure("tall"), None);
    assert_eq!(parse_measure(""), None);
}

fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// Арки в Туле размечены двумя разными тегами, а подземный туннель —
/// третьим, и он аркой быть не должен.
#[test]
fn arches_are_recognised_by_tunnel_and_covered_but_not_by_a_real_tunnel() {
    assert!(is_building_passage(&tags(&[(
        "tunnel",
        "building_passage"
    )])));
    assert!(is_building_passage(&tags(&[(
        "covered",
        "building_passage"
    )])));
    assert!(is_building_passage(&tags(&[("covered", "yes")])));
    assert!(!is_building_passage(&tags(&[("tunnel", "yes")])));
    assert!(!is_building_passage(&tags(&[("bridge", "yes")])));
    assert!(!is_building_passage(&tags(&[])));
}

#[test]
fn height_prefers_the_metric_tag_then_falls_back_to_levels() {
    // ветка Нью-Йорка: метры из LiDAR-импорта
    assert_eq!(building_height(&tags(&[("height", "31.4")])), Some(31.4));
    // ветка Европы: этажи
    assert_eq!(
        building_height(&tags(&[("building:levels", "5")])),
        Some(15.0)
    );
    // roof:levels по схеме S3DB в building:levels не входит
    assert_eq!(
        building_height(&tags(&[("building:levels", "5"), ("roof:levels", "1")])),
        Some(18.0)
    );
    // проставлены оба — верим метрам, а не пересчёту
    assert_eq!(
        building_height(&tags(&[("height", "20"), ("building:levels", "5")])),
        Some(20.0)
    );
    assert_eq!(building_height(&tags(&[("building", "yes")])), None);
}

#[test]
fn implausible_heights_are_treated_as_missing() {
    assert_eq!(building_height(&tags(&[("height", "0")])), None);
    assert_eq!(building_height(&tags(&[("height", "12000")])), None);
    assert_eq!(building_height(&tags(&[("building:levels", "0")])), None);
    assert_eq!(building_height(&tags(&[("building:levels", "-1")])), None);
    // мусор в метрах не должен глушить этажи
    assert_eq!(
        building_height(&tags(&[("height", "9999"), ("building:levels", "4")])),
        Some(12.0)
    );
}

/// Высота доезжает до `MapData` и из way, и из relation, а на воде её нет.
#[test]
fn parsed_areas_carry_building_height_only() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.0005;
    let json = format!(
        r#"{{"elements": [
  {{"type": "way", "id": 1, "tags": {{"building": "yes", "building:levels": "9"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}},
  {{"type": "relation", "id": 2, "tags": {{"building": "yes", "height": "42 m"}},
    "members": [
      {{"type": "way", "role": "outer", "geometry": [
        {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
        {{"lat": {e}, "lon": {c}}}, {{"lat": {a}, "lon": {b}}}]}}]}},
  {{"type": "way", "id": 3, "tags": {{"natural": "water", "height": "5"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.buildings.len(), 2);
    let heights: Vec<Option<f32>> = map
        .buildings
        .iter()
        .map(|building| building.height)
        .collect();
    assert!(heights.contains(&Some(27.0)), "{heights:?}");
    assert!(heights.contains(&Some(42.0)), "{heights:?}");
    assert_eq!(map.water.len(), 1);
    assert_eq!(map.water[0].height, None);
}

/// Вход-узел контура достаётся своему зданию; `entrance=no` и ворота
/// гаража отбрасываются, а вход в стороне от домов остаётся сиротой.
#[test]
fn entrances_attach_to_the_building_whose_outline_they_sit_on() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.0005;
    let json = format!(
        r#"{{"elements": [
  {{"type": "node", "id": 100, "lat": {a}, "lon": {b}, "tags": {{"entrance": "main"}}}},
  {{"type": "node", "id": 101, "lat": {a}, "lon": {c}, "tags": {{"entrance": "staircase"}}}},
  {{"type": "node", "id": 102, "lat": {e}, "lon": {c}, "tags": {{"entrance": "no"}}}},
  {{"type": "node", "id": 103, "lat": {e}, "lon": {b}, "tags": {{"entrance": "garage"}}}},
  {{"type": "node", "id": 104, "lat": {lat}, "lon": {lon}, "tags": {{"entrance": "yes"}}}},
  {{"type": "way", "id": 1, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.buildings.len(), 1);
    // main и staircase — на контуре; no и garage отброшены как значения,
    // а «yes» в центре квартала ни одной вершине не соответствует
    let entrances = &map.buildings[0].entrances;
    assert_eq!(entrances.len(), 2, "{entrances:?}");
    for entrance in entrances {
        assert!(
            map.buildings[0]
                .outer
                .iter()
                .any(|vertex| vertex.distance(*entrance) < 0.01),
            "entrance off the outline: {entrance:?}"
        );
    }
}

/// Две ноды `entrance` в одной точке — обычное дело в Париже; на карте
/// они обязаны стать одной дверью, а не двумя одинаковыми целями.
#[test]
fn entrances_at_the_same_point_collapse_into_one() {
    let (lat, lon) = (CITY.geo_center().x, CITY.geo_center().y);
    let d = 0.0005;
    let json = format!(
        r#"{{"elements": [
  {{"type": "node", "id": 100, "lat": {a}, "lon": {b}, "tags": {{"entrance": "main"}}}},
  {{"type": "node", "id": 101, "lat": {a}, "lon": {b}, "tags": {{"entrance": "yes"}}}},
  {{"type": "way", "id": 1, "tags": {{"building": "yes"}},
    "geometry": [
      {{"lat": {a}, "lon": {b}}}, {{"lat": {a}, "lon": {c}}},
      {{"lat": {e}, "lon": {c}}}, {{"lat": {e}, "lon": {b}}},
      {{"lat": {a}, "lon": {b}}}]}}
]}}"#,
        a = lat - d,
        e = lat + d,
        b = lon - d,
        c = lon + d,
    );

    let map = parse(&json, CITY).unwrap();
    assert_eq!(map.buildings[0].entrances.len(), 1);
}

/// Здание без размеченных в OSM входов не остаётся без двери: их
/// досочиняет генератор (`entrances/`), иначе в Токио, где размечено
/// 0.9% домов, целей у населения почти не было бы.
#[test]
fn a_building_without_osm_entrances_gets_generated_ones() {
    let map = parse(&fixture(), CITY).unwrap();
    assert_eq!(map.buildings.len(), 1);
    let building = &map.buildings[0];
    assert!(!building.entrances.is_empty());
    for entrance in &building.entrances {
        let on_outline = (0..building.outer.len()).any(|index| {
            distance_to_segment(
                *entrance,
                building.outer[index],
                building.outer[(index + 1) % building.outer.len()],
            ) < 0.01
        });
        assert!(
            on_outline,
            "generated entrance off the outline: {entrance:?}"
        );
    }
}

#[test]
fn kremlin_buildings_classified_by_historic_tag() {
    let element = Element {
        kind: "way".into(),
        id: 1,
        tags: [
            ("building".to_string(), "yes".to_string()),
            ("historic".to_string(), "citywalls".to_string()),
        ]
        .into_iter()
        .collect(),
        lat: None,
        lon: None,
        geometry: None,
        members: None,
    };
    assert_eq!(area_kind(&element), Some(AreaKind::Kremlin));
}
