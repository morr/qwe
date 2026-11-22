//! Мультиполигоны: разбор relation ([`parse_relation`]), сборка его колец
//! ([`assemble_rings`]) и общий сток площадей разбора ([`push_area`]).

use bevy::math::Vec2;

use crate::map::osm::model::{AreaKind, MapData, PolyArea, point_in_polygon};
use crate::map::osm::overpass::{Element, GeoBounds, Member};

use super::{area_colours, area_height, area_kind, area_storeys, area_use};
use super::{collapse_slivers, project_points};

/// Совпадение концов way при сборке колец, м (общие OSM-узлы дают
/// идентичные координаты; эпсилон страхует от шума проекции).
const RING_JOIN_EPSILON: f32 = 0.01;

/// Закрытая полилиния way → открытое кольцо (без повторённой последней точки).
pub(in crate::map::osm::parse) fn as_ring(points: &[Vec2]) -> Option<Vec<Vec2>> {
    if points.len() < 4 || points.first() != points.last() {
        return None;
    }
    Some(points[..points.len() - 1].to_vec())
}

pub(in crate::map::osm::parse) fn push_area(map: &mut MapData, mut area: PolyArea) {
    area.outer = collapse_slivers(&area.outer);
    area.holes = area
        .holes
        .iter()
        .map(|hole| collapse_slivers(hole))
        .collect();
    if area.outer.len() < 3 {
        return;
    }
    match area.kind {
        AreaKind::Building | AreaKind::Kremlin => map.buildings.push(area),
        AreaKind::Water => map.water.push(area),
        AreaKind::Park => map.parks.push(area),
        AreaKind::Wood => map.woods.push(area),
        AreaKind::Grass => map.grass.push(area),
        AreaKind::Sand => map.sand.push(area),
        AreaKind::Residential | AreaKind::Industrial => map.landuse.push(area),
        AreaKind::Parking(_) => map.parking.push(area),
        AreaKind::Pitch(_) => map.pitches.push(area),
    }
}

pub(in crate::map::osm::parse) fn parse_relation(
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
    let building_use = area_use(kind, &element.tags);
    let colours = area_colours(kind, &element.tags);
    // этажи — на relation, а не на кольцо: тег один на весь мультиполигон,
    // в отличие от высоты, которую торговой коробке меряет её собственное пятно
    let storeys = area_storeys(kind, &element.tags);

    for outer in outers {
        let holes = inners
            .iter()
            .filter(|inner| point_in_polygon(inner[0], &outer))
            .cloned()
            .collect();
        // высота — по каждому кольцу отдельно, а не одна на relation: этаж
        // торговой коробки меряется её пятном, и у ТЦ одним мультиполигоном
        // размечен и корпус, и пристройка под ним
        let height = area_height(kind, &element.tags, building_use, &outer);
        push_area(
            map,
            PolyArea {
                outer,
                holes,
                kind,
                building_use,
                height,
                storeys,
                entrances: Vec::new(),
                colours,
                osm_id: Some(element.id),
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
