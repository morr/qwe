//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов. Деревья по разобранной карте сажает
//! `super::planting`.

use std::collections::{HashMap, HashSet};

use bevy::math::Vec2;

use super::planting::plant_trees;
use crate::city::City;
use crate::map::osm::entrances::generate_entrances;
use crate::map::osm::model::{
    AreaKind, MapData, PolyArea, RailLine, RoadLine, TreeCompose, TreeNode, TreeRow, TreeRowLayout,
    WallLine, WaterLine, point_in_area, point_in_polygon, ring_bounds,
};
use crate::map::osm::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};

/// Ширина стены Кремля, м.
const WALL_WIDTH: f32 = 3.0;
/// Совпадение концов way при сборке колец, м (общие OSM-узлы дают
/// идентичные координаты; эпсилон страхует от шума проекции).
const RING_JOIN_EPSILON: f32 = 0.01;

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
                if let Some(node) = parse_tree_node(element, &bounds) {
                    map.tree_nodes.push(node);
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

    let drowned = drop_buildings_in_water(&mut map);
    if drowned > 0 {
        eprintln!("osm parse: {drowned} buildings dropped as standing entirely in water");
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
    // размеченных дверей в OSM единицы процентов — остальным дом получает свои
    // по замеру когорт, см. `entrances/`
    let started = std::time::Instant::now();
    let generated = generate_entrances(&mut map);
    eprintln!(
        "osm parse: {} entrances attached, {generated} generated in {:?}",
        entrances.len() - orphaned,
        started.elapsed()
    );

    let started = std::time::Instant::now();
    let (standalone, woods, rows, asked) = plant_trees(&map);
    // «посажено меньше, чем запрошено» — лес уперся в насыщение, потолок
    // плотности стоит выше достижимого (см. `planting::TREE_MIN_SPACING`).
    // Аллеи считаются отдельно и под обе политики: `kept` = `slid` означает,
    // что сдвигать было нечего, а `0 in K tree rows` при `K > 0` — что тег
    // доехал, а посадка по нему не встала никуда. Одиночные ноды выбывают
    // штатно — в лесу и у аллей дерево уже посажено процедурно
    let counts: Vec<String> = TreeRowLayout::ALL
        .iter()
        .map(|&layout| rows.get(layout).len().to_string())
        .collect();
    eprintln!(
        "osm parse: {} trees planted of {asked} asked, {} standalone of {} tree nodes, \
         {} in {} tree rows (keep/slide x osm/slider) in {:?}",
        woods.len(),
        standalone.len(),
        map.tree_nodes.len(),
        counts.join("/"),
        map.tree_rows.len(),
        started.elapsed()
    );
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;
    // сборка по составу **по умолчанию**: парсер о панелях ничего не знает,
    // но и отдавать `MapData` с пустым `trees` не должен — иначе каждый читатель
    // обязан помнить про отдельный шаг сборки. Выбранный игроком состав
    // доложит `map::trees::recompose_row_trees`, и только если он другой
    map.compose_trees(TreeCompose::default());
    Ok(map)
}

/// Дома, целиком стоящие в воде, выбрасываются. В OSM это плавучие рестораны и
/// дебаркадеры (`HMS Belfast` в Лондоне, `Café Barge` в Париже), а в Туле —
/// одинокий сарай посреди Верхнего пруда. Навмеш заливает воду непроходимой,
/// так что до дверей такого дома пешка всё равно не дойдёт, а коробка посреди
/// пруда читается как баг рендера.
///
/// Критерий — **все** вершины контура в воде: дом, зацепившийся за берег
/// (пирс, набережная, дом на сваях у кромки), остаётся. Выброшенных единицы:
/// Тула 1, Берлин 6, Нью-Йорк 17, Лондон и Париж по 28, Токио 0.
///
/// Порядок важен: до раскладки входов и посадки деревьев — иначе дом получит
/// двери, а деревья обойдут стороной пустое место.
fn drop_buildings_in_water(map: &mut MapData) -> usize {
    // AABB-прекомпьют: воды десятки полигонов, зданий десятки тысяч, и почти
    // каждое отсеивается на первой же вершине, не доходя до point-in-polygon
    let bounds: Vec<(Vec2, Vec2)> = map
        .water
        .iter()
        .map(|area| ring_bounds(&area.outer))
        .collect();

    let MapData {
        buildings, water, ..
    } = map;
    let before = buildings.len();
    buildings.retain(|building| {
        !building.outer.iter().all(|point| {
            water.iter().zip(&bounds).any(|(area, &(min, max))| {
                point.x >= min.x
                    && point.x <= max.x
                    && point.y >= min.y
                    && point.y <= max.y
                    && point_in_area(*point, area)
            })
        })
    });
    before - buildings.len()
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

/// Нода `natural=tree` → одиночное дерево. Сажает его (и отсеивает
/// продублированные процедурной посадкой) `planting::plant_standalone`.
fn parse_tree_node(element: &Element, bounds: &GeoBounds) -> Option<TreeNode> {
    if element.tags.get("natural").map(String::as_str) != Some("tree") {
        return None;
    }
    Some(TreeNode {
        pos: bounds.project(element.lat?, element.lon?),
        radius: crown_radius(&element.tags),
    })
}

/// Раскладка входов по зданиям: вход ищется среди вершин контуров, потому что
/// в OSM он и есть узел контура. Возвращает число входов, не нашедших дом.
///
/// Общий узел двух домов (сплошная застройка) попадает в таблицу один раз —
/// вход достанется одному из них, и это не важно: дверь всё равно там же.
///
/// Совпадающие входы схлопываются: в Париже встречаются две ноды `entrance` в
/// одной точке (замер по выгрузке — минимальный зазор 0.00 м), а две двери на
/// одном месте — это две одинаковых цели для пешек и лишний кружок в оверлее.
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
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    for &entrance in entrances {
        let Some(&index) = by_vertex.get(&key(entrance)) else {
            orphaned += 1;
            continue;
        };
        // дубль считаем привязанным, а не сиротой: дом он нашёл
        if taken.insert(key(entrance)) {
            map.buildings[index].entrances.push(entrance);
        }
    }
    orphaned
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

    // Рельсы проверяются до дорог и **не** прерывают разбор: трамвайный путь в
    // OSM сплошь и рядом висит на том же way, что и `highway=*`, и такой way
    // обязан стать и улицей, и путём.
    if let Some(railway) = element.tags.get("railway")
        && let Some((width, kind)) = rail_class(railway)
        && !is_underground(&element.tags)
    {
        map.rails.push(RailLine {
            points: points.clone(),
            width,
            kind,
        });
    }

    // аллея — тоже до дорог и тоже без `return`, по той же причине, что рельсы:
    // ветка не должна затыкаться чужим ранним выходом
    if element.tags.get("natural").map(String::as_str) == Some("tree_row") {
        map.tree_rows.push(TreeRow {
            spacing: row_spacing(&element.tags, &points),
            radius: crown_radius(&element.tags),
            points: points.clone(),
        });
    }

    // водоток — снова до дорог и снова без `return`: ручей в трубе под улицей
    // размечен `waterway=*` на том же way, что и `highway=*`, и обе ветки обязаны
    // отработать. Замкнутый `waterway=riverbank` сюда не попадает — его нет в
    // белом списке, и площадью он остаётся ниже, в `area_kind`.
    if let Some(waterway) = element.tags.get("waterway")
        && let Some((default_width, kind)) = water_class(waterway)
    {
        map.water_lines.push(WaterLine {
            points: points.clone(),
            width: water_width(&element.tags).unwrap_or(default_width),
            kind,
            tunnel: is_underground(&element.tags),
        });
    }

    // подземное не рисуем — то же правило, что у рельсов и водотоков. У дорог
    // оно долго отсутствовало, и подземный переход выходил на карту обычной
    // дорожкой: в Токио так рисовались 1399 way из 12 859 (10.9%), в Лондоне
    // 1985, в Париже 1473, в Берлине 883, в Туле 35. Подавляющее большинство —
    // `highway=footway|steps` метрополитена. Что тег `tunnel` на этом way
    // может описывать вовсе не дорогу — вопрос [`is_road_underground`]
    if let Some(highway) = element.tags.get("highway") {
        let Some((width, class)) = road_class(highway) else {
            return;
        };
        if is_road_underground(&element.tags) {
            return;
        }
        let bridge = element
            .tags
            .get("bridge")
            .is_some_and(|value| value != "no");
        map.roads.push(RoadLine {
            points,
            width,
            class,
            bridge,
            passage: is_building_passage(&element.tags),
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

mod tags;
#[cfg(test)]
mod tests;

// Приватный реэкспорт: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать классификаторы.
use self::tags::{
    NON_WALKABLE_ENTRANCES, area_height, area_kind, crown_radius, is_building_passage,
    is_road_underground, is_underground, rail_class, road_class, row_spacing, water_class,
    water_width,
};
