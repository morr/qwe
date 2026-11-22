//! Элементная половина разбора: цикл по элементам Overpass
//! ([`read_elements`]) и разбор нод.

use bevy::math::Vec2;

use crate::map::osm::contours::OsmContours;
use crate::map::osm::model::{MapData, RoadNode, Structure, TreeNode, ring_vertex_mean};
use crate::map::osm::overpass::{Element, GeoBounds, OverpassResponse};

use super::ReadReport;
use super::{
    NON_WALKABLE_ENTRANCES, as_ring, crown_radius, driving_side, parse_relation, parse_way,
    road_node_kind, structure_height, structure_kind, structure_radius, structure_size,
};

/// Что элементный цикл прочёл, но положить в `MapData` ещё не может.
#[derive(Default)]
pub(in crate::map::osm::parse) struct Pending {
    /// Входы: Overpass отдаёт ноды раньше way, так что на момент разбора ноды
    /// зданий ещё нет.
    pub(in crate::map::osm::parse) entrances: Vec<Vec2>,
}

/// Элементы Overpass — в сырую `MapData` и то, что ещё некуда положить
/// ([`Pending`]).
///
/// **Ничего не доводит.** Дома ещё стоят в воде, храмы без веры, контуры
/// косые, дверей нет, деревья не посажены — всё это [`super::finish_parse`].
pub(in crate::map::osm::parse) fn read_elements(
    response: &OverpassResponse,
    bounds: &GeoBounds,
) -> (MapData, Pending, ReadReport) {
    let mut map = MapData::default();
    let traffic_side = driving_side(&response.elements);
    if let Some(side) = traffic_side {
        map.traffic_side = side;
    }
    let mut unclosed_rings = 0usize;
    let mut pending = Pending::default();

    for element in &response.elements {
        match element.kind.as_str() {
            "node" => {
                if let Some(position) = parse_entrance(element, bounds) {
                    pending.entrances.push(position);
                }
                if let Some(node) = parse_tree_node(element, bounds) {
                    map.tree_nodes.push(node);
                }
                if let Some(structure) = parse_structure_node(element, bounds) {
                    map.structures.push(structure);
                }
                if let Some(node) = parse_road_node(element, bounds) {
                    map.road_nodes.push(node);
                }
            }
            "way" => parse_way(element, bounds, &mut map),
            "relation" => parse_relation(element, bounds, &mut map, &mut unclosed_rings),
            _ => {}
        }
    }

    // снимок для оверлея контуров — до единого доводочного прохода
    let started = std::time::Instant::now();
    map.osm_contours = OsmContours::of(&map);
    let report = ReadReport {
        traffic_side,
        unclosed_rings,
        contour_points: map.osm_contours.points(),
        contouring: started.elapsed(),
    };
    (map, pending, report)
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

/// Нода `man_made=*` → цилиндр промзоны. Контура у ноды нет, поэтому радиус
/// берётся из тега `diameter`, а если и его нет — типовой для рода
/// ([`structure_size`]). В Туле нодами размечены четыре заводские трубы из
/// восьми.
fn parse_structure_node(element: &Element, bounds: &GeoBounds) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let (radius, height) = structure_size(kind);
    Some(Structure {
        at: bounds.project(element.lat?, element.lon?),
        radius: structure_radius(&element.tags).unwrap_or(radius),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
        osm_id: Some(element.id),
    })
}

/// Way `man_made=*` → тот же цилиндр, но радиус считается по контуру: он
/// заведомо честнее тега, которого в данных обычно и нет.
///
/// Центр — [`ring_vertex_mean`], а не центроид площади
/// ([`ring_area_centroid`]): контур цилиндра в OSM рисуют равномерным
/// многоугольником, на нём это одно и то же, а у вытянутого контура (силосный
/// корпус, размеченный прямоугольником) среднее вершин ближе к тому, что глаз
/// считает серединой.
pub(in crate::map::osm::parse) fn parse_structure_way(
    element: &Element,
    points: &[Vec2],
) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let ring = as_ring(points)?;
    let at = ring_vertex_mean(&ring)?;
    let spread = ring.iter().map(|point| at.distance(*point)).sum::<f32>() / ring.len() as f32;
    let (_, height) = structure_size(kind);
    Some(Structure {
        at,
        radius: spread.max(f32::EPSILON),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
        osm_id: Some(element.id),
    })
}

/// Нода `highway=crossing|traffic_signals|…` → дорожный узел ([`RoadNode`]);
/// вид — по белому списку `road_node_kind`.
fn parse_road_node(element: &Element, bounds: &GeoBounds) -> Option<RoadNode> {
    Some(RoadNode {
        kind: road_node_kind(&element.tags)?,
        pos: bounds.project(element.lat?, element.lon?),
        osm_id: Some(element.id),
    })
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
