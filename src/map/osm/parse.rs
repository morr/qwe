//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов, детерминированные деревья в парках.

use bevy::math::Vec2;

use crate::map::osm::model::{
    AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine, distance_to_segment, point_in_area,
    point_in_polygon, ring_area, ring_bounds,
};
use crate::map::osm::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};
use crate::settings::MAP_SIZE;

/// Ширина стены Кремля, м.
const WALL_WIDTH: f32 = 3.0;
/// Плотность деревьев: одно на столько м² леса (1600 / 1.3 — плотность
/// поднята на 30% против первоначальной).
const TREE_AREA_PER_TREE: f32 = 1230.0;
/// Совпадение концов way при сборке колец, м (общие OSM-узлы дают
/// идентичные координаты; эпсилон страхует от шума проекции).
const RING_JOIN_EPSILON: f32 = 0.01;

pub fn parse(json: &str) -> Result<MapData, String> {
    let response: OverpassResponse =
        serde_json::from_str(json).map_err(|error| format!("overpass json: {error}"))?;
    let bounds = GeoBounds::from_settings();

    let mut map = MapData::default();
    let mut skipped_open_rings = 0usize;

    for element in &response.elements {
        match element.kind.as_str() {
            "way" => parse_way(element, &bounds, &mut map),
            "relation" => parse_relation(element, &bounds, &mut map, &mut skipped_open_rings),
            _ => {}
        }
    }

    if skipped_open_rings > 0 {
        // не error: кольца, порванные краем bbox, ожидаемы
        eprintln!("osm parse: {skipped_open_rings} unclosed relation rings skipped");
    }

    map.trees = plant_trees(&map);
    Ok(map)
}

/// Классификация элемента по тегам → вид площадного объекта.
fn area_kind(element: &Element) -> Option<AreaKind> {
    let tags = &element.tags;
    if tags.contains_key("building") {
        // исторические стены/башни Кремля подкрашиваются отдельно
        let historic = tags.get("historic").map(String::as_str);
        return Some(match historic {
            Some("citywalls" | "castle" | "city_gate" | "fort") => AreaKind::Kremlin,
            _ => AreaKind::Building,
        });
    }
    let natural = tags.get("natural").map(String::as_str);
    let landuse = tags.get("landuse").map(String::as_str);
    if natural == Some("water") || tags.get("waterway").map(String::as_str) == Some("riverbank") {
        return Some(AreaKind::Water);
    }
    if matches!(natural, Some("sand" | "beach")) {
        return Some(AreaKind::Sand);
    }
    // луг проверяется до парка: газон внутри парка — отдельный светлый слой
    if matches!(landuse, Some("grass" | "meadow"))
        || matches!(natural, Some("grassland" | "meadow"))
    {
        return Some(AreaKind::Grass);
    }
    if natural == Some("wood") || landuse == Some("forest") {
        return Some(AreaKind::Wood);
    }
    if matches!(
        tags.get("leisure").map(String::as_str),
        Some("park" | "garden")
    ) || landuse == Some("recreation_ground")
    {
        return Some(AreaKind::Park);
    }
    None
}

/// Ширина и класс по значению highway; `None` — дорогу не рисуем.
fn road_class(highway: &str) -> Option<(f32, RoadClass)> {
    Some(match highway {
        "motorway" | "trunk" | "primary" => (16.0, RoadClass::Street),
        "secondary" => (12.0, RoadClass::Street),
        "tertiary" => (10.0, RoadClass::Street),
        "residential" | "unclassified" | "living_street" => (8.0, RoadClass::Street),
        "service" => (5.0, RoadClass::Street),
        "footway" | "path" | "pedestrian" | "cycleway" | "steps" | "track" => {
            (3.5, RoadClass::Alley)
        }
        _ => return None,
    })
}

fn project_points(points: &[LatLon], bounds: &GeoBounds) -> Vec<Vec2> {
    points
        .iter()
        .map(|point| bounds.project(point.lat, point.lon))
        .collect()
}

/// Закрытая полилиния way → открытое кольцо (без повторённой последней точки).
fn as_ring(points: &[Vec2]) -> Option<Vec<Vec2>> {
    if points.len() < 4 || points.first() != points.last() {
        return None;
    }
    Some(points[..points.len() - 1].to_vec())
}

fn push_area(map: &mut MapData, area: PolyArea) {
    match area.kind {
        AreaKind::Building | AreaKind::Kremlin => map.buildings.push(area),
        AreaKind::Water => map.water.push(area),
        AreaKind::Park => map.parks.push(area),
        AreaKind::Wood => map.woods.push(area),
        AreaKind::Grass => map.grass.push(area),
        AreaKind::Sand => map.sand.push(area),
    }
}

fn parse_way(element: &Element, bounds: &GeoBounds, map: &mut MapData) {
    let Some(geometry) = &element.geometry else {
        return;
    };
    let points = project_points(geometry, bounds);
    if points.len() < 2 {
        return;
    }

    if let Some(highway) = element.tags.get("highway") {
        let Some((width, class)) = road_class(highway) else {
            return;
        };
        let bridge = element
            .tags
            .get("bridge")
            .is_some_and(|value| value != "no");
        map.roads.push(RoadLine {
            points,
            width,
            class,
            bridge,
        });
        return;
    }

    if element.tags.get("barrier").map(String::as_str) == Some("city_wall") {
        map.walls.push(WallLine {
            points,
            width: WALL_WIDTH,
        });
        return;
    }

    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(outer) = as_ring(&points) else {
        return;
    };
    push_area(
        map,
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
        },
    );
}

fn parse_relation(
    element: &Element,
    bounds: &GeoBounds,
    map: &mut MapData,
    skipped_open_rings: &mut usize,
) {
    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(members) = &element.members else {
        return;
    };

    let outers = assemble_rings(members, "outer", bounds, skipped_open_rings);
    let inners = assemble_rings(members, "inner", bounds, skipped_open_rings);

    for outer in outers {
        let holes = inners
            .iter()
            .filter(|inner| point_in_polygon(inner[0], &outer))
            .cloned()
            .collect();
        push_area(map, PolyArea { outer, holes, kind });
    }
}

/// Сборка замкнутых колец из way-членов relation с заданной ролью:
/// цепочки соединяются по совпадающим концам (с разворотом при
/// необходимости), пока не замкнутся.
fn assemble_rings(
    members: &[Member],
    role: &str,
    bounds: &GeoBounds,
    skipped_open_rings: &mut usize,
) -> Vec<Vec<Vec2>> {
    let mut segments: Vec<Vec<Vec2>> = members
        .iter()
        .filter(|member| member.kind == "way" && member.role == role)
        .filter_map(|member| member.geometry.as_ref())
        .map(|geometry| project_points(geometry, bounds))
        .filter(|points| points.len() >= 2)
        .collect();

    let close = |a: Vec2, b: Vec2| a.distance_squared(b) < RING_JOIN_EPSILON * RING_JOIN_EPSILON;
    let mut rings = Vec::new();

    while let Some(mut ring) = segments.pop() {
        loop {
            if close(ring[0], *ring.last().unwrap()) && ring.len() >= 4 {
                ring.pop();
                rings.push(ring);
                break;
            }

            let tail = *ring.last().unwrap();
            let Some(index) = segments.iter().position(|segment| {
                close(segment[0], tail) || close(*segment.last().unwrap(), tail)
            }) else {
                // не замкнулось (обычно порвано краем bbox): ≥3 точек —
                // насильно замыкаем, иначе выбрасываем
                if ring.len() >= 3 {
                    rings.push(ring);
                } else {
                    *skipped_open_rings += 1;
                }
                break;
            };

            let mut segment = segments.swap_remove(index);
            if !close(segment[0], tail) {
                segment.reverse();
            }
            ring.extend_from_slice(&segment[1..]);
        }
    }
    rings
}

/// Зазор дерева до зданий и кромок дорог, м.
const TREE_CLEARANCE: f32 = 1.5;

/// Зазор до берега, м: больше обычного, чтобы крона (радиус до 4) не свисала
/// над прудом — дерево впритык к воде читается как растущее из воды.
const TREE_SHORE_CLEARANCE: f32 = 3.0;

/// Минимум между центрами деревьев, м: кроны (радиус до 4) могут чуть
/// касаться, но не сливаться в кляксу.
const TREE_MIN_SPACING: f32 = 6.0;

/// Точка ближе `clearance` к любому ребру полигона — внешнему кольцу или
/// кольцу дырки (кольца замкнуты неявно, последнее ребро — от конца к началу).
fn near_area_edge(point: Vec2, area: &PolyArea, clearance: f32) -> bool {
    std::iter::once(&area.outer).chain(&area.holes).any(|ring| {
        (0..ring.len()).any(|index| {
            distance_to_segment(point, ring[index], ring[(index + 1) % ring.len()]) <= clearance
        })
    })
}

/// Деревья: сажаются **только в лесных полигонах** (`natural=wood` /
/// `landuse=forest`) — в OSM именно они, а не парк целиком, несут деревья;
/// открытая часть парка обязана остаться полем. Детерминированный LCG по
/// геометрии массива, плотность ∝ площади, rejection-sampling внутри полигона,
/// только в границах карты и не на зданиях/дорогах (парковые аллеи — тоже
/// дороги), не в воде и не на лугах и песке.
fn plant_trees(map: &MapData) -> Vec<(Vec2, f32)> {
    // AABB-прекомпьют, чтобы не гонять point-in-polygon по всем 3к зданий
    let building_bounds: Vec<(Vec2, Vec2)> = map
        .buildings
        .iter()
        .map(|building| {
            let (min, max) = ring_bounds(&building.outer);
            (min - TREE_CLEARANCE, max + TREE_CLEARANCE)
        })
        .collect();
    let road_bounds: Vec<(Vec2, Vec2)> = map
        .roads
        .iter()
        .map(|road| {
            let (min, max) = ring_bounds(&road.points);
            let pad = road.width / 2.0 + TREE_CLEARANCE;
            (min - pad, max + pad)
        })
        .collect();

    let water_bounds: Vec<(Vec2, Vec2)> = map
        .water
        .iter()
        .map(|area| {
            let (min, max) = ring_bounds(&area.outer);
            (min - TREE_SHORE_CLEARANCE, max + TREE_SHORE_CLEARANCE)
        })
        .collect();

    // луг и пляж деревьев не несут, но кроне свисать на них не запрещено
    let bare: Vec<&PolyArea> = map.grass.iter().chain(&map.sand).collect();
    let bare_bounds: Vec<(Vec2, Vec2)> = bare.iter().map(|area| ring_bounds(&area.outer)).collect();

    let in_bbox = |pos: Vec2, min: Vec2, max: Vec2| {
        pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
    };
    let blocked = |pos: Vec2| {
        map.buildings
            .iter()
            .zip(&building_bounds)
            .any(|(building, &(min, max))| in_bbox(pos, min, max) && point_in_area(pos, building))
            || map
                .roads
                .iter()
                .zip(&road_bounds)
                .any(|(road, &(min, max))| {
                    in_bbox(pos, min, max)
                        && road.points.windows(2).any(|segment| {
                            distance_to_segment(pos, segment[0], segment[1])
                                <= road.width / 2.0 + TREE_CLEARANCE
                        })
                })
            || map
                .water
                .iter()
                .zip(&water_bounds)
                .any(|(area, &(min, max))| {
                    in_bbox(pos, min, max)
                        && (point_in_area(pos, area)
                            || near_area_edge(pos, area, TREE_SHORE_CLEARANCE))
                })
            || bare
                .iter()
                .zip(&bare_bounds)
                .any(|(area, &(min, max))| in_bbox(pos, min, max) && point_in_area(pos, area))
    };

    let mut trees = Vec::new();
    for wood in &map.woods {
        let area = ring_area(&wood.outer);
        let count = ((area / TREE_AREA_PER_TREE) as usize).max(3);
        let (min, max) = ring_bounds(&wood.outer);
        let size = max - min;
        if size.x <= 0.0 || size.y <= 0.0 {
            continue;
        }

        let first = wood.outer[0];
        let mut state: u64 =
            0x9E37_79B9_7F4A_7C15 ^ (first.x.to_bits() as u64) ^ ((first.y.to_bits() as u64) << 32);
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (u32::MAX >> 1) as f32
        };

        let mut planted = 0;
        // лимит попыток — страховка от вырожденных/застроенных полигонов
        let mut attempts = count * 30;
        while planted < count && attempts > 0 {
            attempts -= 1;
            let pos = min + Vec2::new(next() * size.x, next() * size.y);
            if !point_in_area(pos, wood) {
                continue;
            }
            if pos.x < 0.0 || pos.y < 0.0 || pos.x > MAP_SIZE.x || pos.y > MAP_SIZE.y {
                continue;
            }
            if blocked(pos) {
                continue;
            }
            let spacing_sq = TREE_MIN_SPACING * TREE_MIN_SPACING;
            if trees
                .iter()
                .any(|&(other, _)| pos.distance_squared(other) < spacing_sq)
            {
                continue;
            }
            let radius = 2.5 + next() * 1.5;
            trees.push((pos, radius));
            planted += 1;
        }
    }
    trees
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{GEO_CENTER_LAT, GEO_CENTER_LON};

    /// Мини-ответ Overpass: way-здание, дорога-мост, relation-вода из двух
    /// половинок с дыркой-островом.
    fn fixture() -> String {
        let bounds = GeoBounds::from_settings();
        let _ = &bounds;
        let (lat, lon) = (GEO_CENTER_LAT, GEO_CENTER_LON);
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
        let map = parse(&fixture()).unwrap();

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
    fn trees_are_deterministic_and_inside_the_wood() {
        let bounds = GeoBounds::from_settings();
        let _ = &bounds;
        let (lat, lon) = (GEO_CENTER_LAT, GEO_CENTER_LON);
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

        let first = parse(&json).unwrap();
        let second = parse(&json).unwrap();
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
        let (lat, lon) = (GEO_CENTER_LAT, GEO_CENTER_LON);
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

        let map = parse(&json).unwrap();
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

    #[test]
    fn trees_avoid_grass_and_sand_inside_the_wood() {
        let (lat, lon) = (GEO_CENTER_LAT, GEO_CENTER_LON);
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

        let map = parse(&json).unwrap();
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

    /// Открытая часть парка — поле: деревья растут только в лесных полигонах,
    /// парк без `natural=wood` остаётся пустым.
    #[test]
    fn a_park_without_wood_grows_no_trees() {
        let (lat, lon) = (GEO_CENTER_LAT, GEO_CENTER_LON);
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

        let map = parse(&json).unwrap();
        assert_eq!(map.parks.len(), 1);
        assert!(map.woods.is_empty());
        assert!(map.trees.is_empty());
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
            geometry: None,
            members: None,
        };
        assert_eq!(area_kind(&element), Some(AreaKind::Kremlin));
    }
}
