//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов, детерминированные деревья в парках.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use bevy::math::Vec2;

use crate::city::City;
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

/// Метров на этаж, когда в OSM есть только `building:levels`. Без этого
/// перевода высота была бы почти только у Нью-Йорка: `height` там проставлен у
/// 97% зданий (LiDAR-импорт), а в Европе его нет и у 2% — там маппят этажи
/// (Париж 64%, Берлин 59%, Лондон 50%, Тула 31%).
const METERS_PER_LEVEL: f32 = 3.0;

/// Границы правдоподобия высоты, м. В OSM попадаются и `height=0`, и опечатки
/// на порядок; всё за пределами трактуем как отсутствие тега — лучше дефолт
/// потребителя, чем километровый сарай.
const BUILDING_HEIGHT_RANGE: RangeInclusive<f32> = 2.0..=600.0;

/// Значения `entrance`, через которые человек не ходит: `no` — «это не вход»
/// (в Париже таких 604), `garage` — ворота для машины, `emergency` — запертая
/// пожарная дверь. Всё остальное (`yes`, `main`, `staircase`, `home`, `shop`,
/// `service`, `exit`) — дверь как дверь.
const NON_WALKABLE_ENTRANCES: [&str; 3] = ["no", "garage", "emergency"];

/// Шаг квантования координат при привязке входа к зданию, 1/м. Вход в OSM —
/// как правило общий узел контура здания (Тула 82%, Берлин 79%, Париж 65%),
/// так что после одной и той же проекции координаты совпадают точно;
/// сантиметровая сетка — страховка от шума f32, а не поиск ближайшего.
const ENTRANCE_SNAP_SCALE: f32 = 100.0;

pub fn parse(json: &str, city: City) -> Result<MapData, String> {
    let response: OverpassResponse =
        serde_json::from_str(json).map_err(|error| format!("overpass json: {error}"))?;
    let bounds = GeoBounds::for_city(city);

    let mut map = MapData::default();
    let mut skipped_open_rings = 0usize;
    // Overpass отдаёт ноды раньше way, так что здания на этот момент ещё не
    // разобраны: копим входы и раскладываем по домам после цикла
    let mut entrances = Vec::new();

    for element in &response.elements {
        match element.kind.as_str() {
            "node" => {
                if let Some(position) = parse_entrance(element, &bounds) {
                    entrances.push(position);
                }
            }
            "way" => parse_way(element, &bounds, &mut map),
            "relation" => parse_relation(element, &bounds, &mut map, &mut skipped_open_rings),
            _ => {}
        }
    }

    if skipped_open_rings > 0 {
        // не error: кольца, порванные краем bbox, ожидаемы
        eprintln!("osm parse: {skipped_open_rings} unclosed relation rings skipped");
    }

    let orphaned = attach_entrances(&mut map, &entrances);
    if orphaned > 0 {
        // ожидаемо: вход бывает отдельной нодой у крыльца, а не узлом контура,
        // либо принадлежит зданию, которое не попало в bbox
        eprintln!(
            "osm parse: {orphaned} of {} entrances match no building",
            entrances.len()
        );
    }

    map.trees = plant_trees(&map);
    Ok(map)
}

/// Нода `entrance=*` → позиция на карте. Не вход или значение из
/// [`NON_WALKABLE_ENTRANCES`] — `None`.
fn parse_entrance(element: &Element, bounds: &GeoBounds) -> Option<Vec2> {
    let entrance = element.tags.get("entrance")?.as_str();
    if NON_WALKABLE_ENTRANCES.contains(&entrance) {
        return None;
    }
    Some(bounds.project(element.lat?, element.lon?))
}

/// Раскладка входов по зданиям: вход ищется среди вершин контуров, потому что
/// в OSM он и есть узел контура. Возвращает число входов, не нашедших дом.
///
/// Общий узел двух домов (сплошная застройка) попадает в таблицу один раз —
/// вход достанется одному из них, и это не важно: дверь всё равно там же.
fn attach_entrances(map: &mut MapData, entrances: &[Vec2]) -> usize {
    let key = |point: Vec2| {
        (
            (point.x * ENTRANCE_SNAP_SCALE).round() as i32,
            (point.y * ENTRANCE_SNAP_SCALE).round() as i32,
        )
    };

    let mut by_vertex: HashMap<(i32, i32), usize> = HashMap::new();
    for (index, building) in map.buildings.iter().enumerate() {
        for &vertex in &building.outer {
            by_vertex.insert(key(vertex), index);
        }
    }

    let mut orphaned = 0;
    for &entrance in entrances {
        match by_vertex.get(&key(entrance)) {
            Some(&index) => map.buildings[index].entrances.push(entrance),
            None => orphaned += 1,
        }
    }
    orphaned
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

/// Число из значения тега OSM. Единица измерения по умолчанию — метр, но
/// маппят и с суффиксом (`12 m`, `12.5 metres`), и с запятой (`12,5`), и через
/// точку с запятой, когда значений несколько (`3;4` — берём первое), и в футах
/// с дюймами (`40'`, `40'6"`). Не разобралось — `None`.
fn parse_measure(value: &str) -> Option<f32> {
    let value = value.split(';').next()?.trim();

    if let Some((feet, inches)) = value.split_once('\'') {
        let feet: f32 = feet.trim().parse().ok()?;
        let inches: f32 = inches
            .trim()
            .trim_end_matches('"')
            .trim()
            .parse()
            .unwrap_or(0.0);
        return Some(feet * 0.3048 + inches * 0.0254);
    }

    // числовой префикс: всё, начиная с первого нецифрового символа, — единица
    let cleaned = value.replace(',', ".");
    let end = cleaned
        .find(|character: char| {
            !(character.is_ascii_digit() || character == '.' || character == '-')
        })
        .unwrap_or(cleaned.len());
    cleaned[..end].parse().ok()
}

/// Высота здания в метрах: `height` как есть, иначе этажи
/// (`building:levels` + `roof:levels`, второй по схеме S3DB в первый не входит)
/// по [`METERS_PER_LEVEL`]. Оба тега разом почти не встречаются, так что это не
/// «уточнение», а две независимые ветки данных.
fn building_height(tags: &HashMap<String, String>) -> Option<f32> {
    let plausible = |meters: f32| BUILDING_HEIGHT_RANGE.contains(&meters).then_some(meters);

    if let Some(meters) = tags
        .get("height")
        .and_then(|value| parse_measure(value))
        .and_then(plausible)
    {
        return Some(meters);
    }

    let levels = tags
        .get("building:levels")
        .and_then(|value| parse_measure(value))?;
    let roof_levels = tags
        .get("roof:levels")
        .and_then(|value| parse_measure(value))
        .unwrap_or(0.0);
    plausible((levels + roof_levels) * METERS_PER_LEVEL)
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
            height: area_height(kind, &element.tags),
            entrances: Vec::new(),
        },
    );
}

/// Высота имеет смысл только у зданий: у пруда и газона её не бывает даже при
/// случайно проставленном теге.
fn area_height(kind: AreaKind, tags: &HashMap<String, String>) -> Option<f32> {
    matches!(kind, AreaKind::Building | AreaKind::Kremlin)
        .then(|| building_height(tags))
        .flatten()
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
    let height = area_height(kind, &element.tags);

    for outer in outers {
        let holes = inners
            .iter()
            .filter(|inner| point_in_polygon(inner[0], &outer))
            .cloned()
            .collect();
        push_area(
            map,
            PolyArea {
                outer,
                holes,
                kind,
                height,
                entrances: Vec::new(),
            },
        );
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

    /// Здание без размеченных входов остаётся с пустым списком — потребитель
    /// обязан работать и так (Токио: входов почти нет).
    #[test]
    fn a_building_without_entrances_keeps_an_empty_list() {
        let map = parse(&fixture(), CITY).unwrap();
        assert_eq!(map.buildings.len(), 1);
        assert!(map.buildings[0].entrances.is_empty());
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
}
