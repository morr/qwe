//! Разбор одного way по тегам ([`parse_way`]) и сторона движения карты
//! ([`driving_side`]).

use crate::map::osm::model::{
    FenceLine, MapData, PipeLine, PolyArea, RailLine, RoadArea, RoadClass, RoadLine, TrafficSide,
    TreeRow, WallLine, WaterLine,
};
use crate::map::osm::overpass::{Element, GeoBounds};

use super::project_points;
use super::{
    area_colours, area_height, area_kind, area_storeys, area_use, as_ring, crown_radius,
    fence_kind, has_lane_markings, is_building_passage, is_oneway, is_oneway_backward,
    is_parking_aisle, is_road_underground, is_roundabout, is_underground, parse_structure_way,
    path_width, pipe_width, push_area, rail_class, road_area_kind, road_class, row_spacing,
    service_track, tagged_lanes, tagged_lanes_backward, tagged_parking, tagged_pavement,
    tagged_sidewalks, tagged_turns, untagged_sidewalks, water_class, water_width,
};

/// Ширина стены Кремля, м.
const WALL_WIDTH: f32 = 3.0;

pub(in crate::map::osm::parse) fn parse_way(
    element: &Element,
    bounds: &GeoBounds,
    map: &mut MapData,
) {
    let Some(geometry) = &element.geometry else {
        return;
    };
    let points = project_points(geometry, bounds);
    if points.len() < 2 {
        return;
    }
    // `bridge=*`, кроме `no`, — одно правило для пути и для дороги
    let bridge = element
        .tags
        .get("bridge")
        .is_some_and(|value| value != "no");

    // Площадь дороги — первой и без `return`: `highway=*` + `area=yes` обязан
    // дойти и до дорожной ветки, где он был линией и до v15
    if let Some(kind) = road_area_kind(&element.tags)
        && let Some(outline) = as_ring(&points)
    {
        map.road_areas.push(RoadArea {
            outline,
            kind,
            osm_id: Some(element.id),
        });
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
            service: service_track(&element.tags),
            bridge,
            osm_id: Some(element.id),
        });
    }

    // Надземный трубопровод — теплотрасса на опорах. Тоже до дорог и тоже без
    // `return`: труба идёт эстакадой над улицей, и такой way в OSM носит оба
    // тега — стоя после дорожной ветки, она бы до него не добралась.
    if element.tags.get("man_made").map(String::as_str) == Some("pipeline")
        && let Some(width) = pipe_width(&element.tags)
    {
        map.pipes.push(PipeLine {
            points: points.clone(),
            width,
            osm_id: Some(element.id),
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
            osm_id: Some(element.id),
        });
    }

    // ограда участка — снова до дорог и снова без `return`: обнесённый забором
    // квартал в OSM — это один way с `barrier=fence` **и** `landuse=residential`,
    // и он обязан стать и оградой, и кварталом (с `return` здесь Тула теряла три
    // площадки, три стоянки, квартал и парк), а забор вдоль тропы висит на том же
    // way, что и `highway=*`, и ниже дорожной ветки её `return` съедал бы его
    // целиком (в Париже и Лондоне по одному такому way, в остальных пяти городах
    // ни одного). Ограда — не стена: рисуется, но навмеша не касается.
    if let Some(kind) = fence_kind(&element.tags) {
        map.fences.push(FenceLine {
            points: points.clone(),
            kind,
            gates: Vec::new(),
            osm_id: Some(element.id),
        });
    }

    // подземное не рисуем — то же правило, что у рельсов и водотоков. У дорог
    // оно долго отсутствовало, и подземный переход выходил на карту обычной
    // дорожкой: в Токио так рисовались 1399 way из 12 859 (10.9%), в Лондоне
    // 1985, в Париже 1473, в Берлине 883, в Туле 35. Подавляющее большинство —
    // `highway=footway|steps` метрополитена. Что тег `tunnel` на этом way
    // может описывать вовсе не дорогу — вопрос [`is_road_underground`]
    if let Some(highway) = element.tags.get("highway") {
        let Some((width, class, highway)) = road_class(highway) else {
            return;
        };
        if is_road_underground(&element.tags) {
            return;
        }
        let passage = is_building_passage(&element.tags);
        // ширина дорожки — по тегу и виду; мостик и арка остаются при
        // ширине класса: их лента — коридор, который режет навмеш, и узкий
        // `width=1` на мостике сузил бы переход через реку до одного тайла
        let width = if class == RoadClass::Alley && !bridge && !passage {
            path_width(&element.tags)
        } else {
            width
        };
        // `oneway=-1` — поток против порядка точек; разворачиваем здесь, чтобы
        // ниже по конвейеру «направление way» и «направление движения» были
        // одним и тем же. Рельс и водоток той же ноды это не касается: их
        // ветки отработали выше и взяли `points` в исходном порядке
        let mut points = points;
        if is_oneway_backward(&element.tags) {
            points.reverse();
        }
        map.roads.push(RoadLine {
            points,
            width,
            class,
            highway,
            bridge,
            passage,
            oneway: is_oneway(&element.tags),
            roundabout: is_roundabout(&element.tags),
            lanes: tagged_lanes(&element.tags),
            lanes_backward: tagged_lanes_backward(&element.tags),
            parking_aisle: is_parking_aisle(&element.tags),
            turns: tagged_turns(&element.tags),
            lane_markings: has_lane_markings(&element.tags),
            sidewalks: tagged_sidewalks(&element.tags)
                .unwrap_or_else(|| untagged_sidewalks(&element.tags)),
            verges: [0.0; 2],
            verge_profile: Default::default(),
            parking: tagged_parking(&element.tags),
            pavement: tagged_pavement(&element.tags),
            osm_id: Some(element.id),
        });
        return;
    }

    if element.tags.get("barrier").map(String::as_str) == Some("city_wall") {
        map.walls.push(WallLine {
            points,
            width: WALL_WIDTH,
            osm_id: Some(element.id),
        });
        return;
    }

    // Цилиндр промзоны — резервуар, силос, труба, башня. Эта ветка, в
    // отличие от рельсовой и трубопроводной, **прерывает разбор**: труба,
    // размеченная way с
    // `building=yes` (в Туле такая одна из восьми), — тот же самый объект, и
    // коробка под цилиндром была бы им обоим сразу. Заодно у неё не
    // заводятся ни двери, ни навмеш-препятствие, чего трубе и не нужно.
    if let Some(structure) = parse_structure_way(element, &points) {
        map.structures.push(structure);
        return;
    }

    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(outer) = as_ring(&points) else {
        return;
    };
    // высота читается до того, как кольцо уедет в `PolyArea`: торговому этажу
    // нужен контур, а не только теги — и тот же класс, которым дом рисуется
    let building_use = area_use(kind, &element.tags);
    let height = area_height(kind, &element.tags, building_use, &outer);
    push_area(
        map,
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            building_use,
            height,
            storeys: area_storeys(kind, &element.tags),
            entrances: Vec::new(),
            colours: area_colours(kind, &element.tags),
            osm_id: Some(element.id),
        },
    );
}

/// Сторона движения — `driving_side` на административной границе, внутри
/// которой лежит центр карты (запрос `is_in` в `overpass_query`). На дорогах
/// тег почти не ставят: в OSM он живёт на стране и наследуется всем внутри.
///
/// Границ с тегом может прийти несколько (страна и заморская территория,
/// регион-исключение внутри страны) — берётся самая мелкая, с наибольшим
/// `admin_level`. Непонятное значение пропускается, как отсутствие тега.
pub(in crate::map::osm::parse) fn driving_side(elements: &[Element]) -> Option<TrafficSide> {
    elements
        .iter()
        .filter(|element| element.kind == "relation")
        .filter_map(|element| {
            let side = match element.tags.get("driving_side")?.as_str() {
                "right" => TrafficSide::Right,
                "left" => TrafficSide::Left,
                _ => return None,
            };
            let level = element
                .tags
                .get("admin_level")
                .and_then(|level| level.parse::<u8>().ok())
                .unwrap_or(0);
            Some((level, side))
        })
        .max_by_key(|(level, _)| *level)
        .map(|(_, side)| side)
}
