//! Формы подсветки элемента под курсором: чистая геометрия того, что панель
//! инспектора рисует прозрачно-жёлтым при включённой галочке highlight, и меш
//! этих форм одним цветом.

use bevy::prelude::*;
use i_overlay::mesh::float::style::LineCap;

use crate::map::footprint::FENCE_BAND_WIDTH;
use crate::map::osm::model::{MapData, PolyArea};
use crate::map::shapes::{is_ring, ring_of, stroke};
use crate::map::{MeshBuilder, RoadStyle};

use super::{FeatureRef, InspectIndex, NODE_REACH};

/// Форма одного полигонального куска элемента: внешнее кольцо и дырки.
///
/// Кусков у элемента бывает много — обводка ломаной распадается на кольца,
/// полосы-генераты кладутся квадом на отрезок осевой, — поэтому
/// [`feature_shape`] отвечает списком.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureShape {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
}

/// Форма элемента пробы для подсветки: что рисовать прозрачно-жёлтым, когда
/// в панели инспектора включена галочка highlight (`ui/inspector.rs`). Чистая
/// функция над [`MapData`] и индексом: площади — контуром с дырками, линии —
/// обводкой на своей полной ширине (`shapes::stroke`), полосы-генераты —
/// квадом на отрезок осевой между двумя офсетами, узлы и деревья — дисками,
/// карманы у бордюра — полигоном из индекса ([`InspectIndex::kerb_pockets`],
/// их в [`MapData`] нет).
///
/// Ширины те же, что у зоны срабатывания находки ([`InspectIndex::hits`]):
/// форма накрывает ровно то, на что находка отвечает. Дерево видит ту же
/// плотность, что и находка, — [`InspectIndex::hits`] отвечает деревом, только
/// пока его ранг в префиксе плотности. Две приблизительности здесь — свойства
/// самого индекса, не формы: полоса стыкуется пер-сегментно, без скруглений
/// (в изгибе осевой стык квадов чуть уже зоны), а зум-LOD обочин не виден ни
/// тому, ни другому.
pub fn feature_shape(
    index: &InspectIndex,
    map: &MapData,
    roads: &RoadStyle,
    feature: FeatureRef,
) -> Option<Vec<FeatureShape>> {
    match feature {
        FeatureRef::Building(i) => area_shape(map.buildings.get(i as usize)?),
        FeatureRef::Water(i) => area_shape(map.water.get(i as usize)?),
        FeatureRef::Park(i) => area_shape(map.parks.get(i as usize)?),
        FeatureRef::Wood(i) => area_shape(map.woods.get(i as usize)?),
        FeatureRef::Grass(i) => area_shape(map.grass.get(i as usize)?),
        FeatureRef::Sand(i) => area_shape(map.sand.get(i as usize)?),
        FeatureRef::Landuse(i) => area_shape(map.landuse.get(i as usize)?),
        FeatureRef::Pocket(i) => area_shape(map.pockets.get(i as usize)?),
        FeatureRef::Parking(i) => area_shape(map.parking.get(i as usize)?),
        FeatureRef::Pitch(i) => area_shape(map.pitches.get(i as usize)?),
        FeatureRef::RoadArea(i) => {
            let area = map.road_areas.get(i as usize)?;
            Some(vec![FeatureShape {
                outer: area.outline.clone(),
                holes: vec![],
            }])
        }
        FeatureRef::Road(i) => {
            let road = map.roads.get(i as usize)?;
            Some(stroke_shapes(&road.points, road.width))
        }
        FeatureRef::Rail(i) => {
            let rail = map.rails.get(i as usize)?;
            Some(stroke_shapes(&rail.points, rail.width))
        }
        FeatureRef::Waterway(i) => {
            let line = map.water_lines.get(i as usize)?;
            Some(stroke_shapes(&line.points, line.width))
        }
        FeatureRef::Pipe(i) => {
            let pipe = map.pipes.get(i as usize)?;
            Some(stroke_shapes(&pipe.points, pipe.width))
        }
        FeatureRef::Wall(i) => {
            let wall = map.walls.get(i as usize)?;
            Some(stroke_shapes(&wall.points, wall.width))
        }
        FeatureRef::Fence(i) => {
            let fence = map.fences.get(i as usize)?;
            Some(stroke_shapes(&fence.points, FENCE_BAND_WIDTH))
        }
        FeatureRef::SidewalkBand { road: i, side } => {
            let road = map.roads.get(i as usize)?;
            let profile = road.sidewalk();
            let band = profile.band()?;
            let side = side as usize;
            (roads.sidewalks && profile.on(side).is_some()).then(|| {
                let half = road.width / 2.0;
                band_shapes(&road.points, half, half + band, side as u8)
            })
        }
        FeatureRef::RoadNode(i) => {
            let node = map.road_nodes.get(i as usize)?;
            Some(vec![disk(node.pos, NODE_REACH)])
        }
        FeatureRef::Structure(i) => {
            let structure = map.structures.get(i as usize)?;
            Some(vec![disk(structure.at, structure.radius)])
        }
        FeatureRef::Tree(rank) => {
            let (pos, radius) = map.trees.positions().get(rank as usize)?;
            Some(vec![disk(*pos, *radius)])
        }
        FeatureRef::KerbPocket(i) => {
            let pocket = index.kerb_pockets.get(i as usize)?;
            Some(vec![FeatureShape {
                outer: pocket.outline.clone(),
                holes: vec![],
            }])
        }
        FeatureRef::Piece(i) => {
            let piece = index.pieces.get(i as usize)?;
            (!piece.contour.is_empty()).then(|| {
                vec![FeatureShape {
                    outer: piece.contour.clone(),
                    holes: piece.holes.clone(),
                }]
            })
        }
    }
}

/// Меш подсветки: все куски формы одним цветом. Полигон с дырками идёт в
/// `earcut` целиком — двор внутри дома остаётся неподсвеченным.
pub fn mesh_feature(shapes: &[FeatureShape], color: LinearRgba) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    for shape in shapes {
        builder.push_polygon(&shape.outer, &shape.holes, color);
    }
    builder
}

/// Площадь — своя геометрия как есть.
fn area_shape(area: &PolyArea) -> Option<Vec<FeatureShape>> {
    (!area.outer.is_empty()).then(|| {
        vec![FeatureShape {
            outer: area.outer.clone(),
            holes: area.holes.clone(),
        }]
    })
}

/// Обводка ломаной на полной ширине: список колец, какие дала обводка
/// (самопересечения и кольца распадаются на куски).
fn stroke_shapes(points: &[Vec2], width: f32) -> Vec<FeatureShape> {
    stroke(points, width, LineCap::Butt, is_ring(points))
        .into_iter()
        .map(|contour| FeatureShape {
            outer: ring_of(&contour),
            holes: vec![],
        })
        .collect()
}

/// Полоса вдоль оси на одной стороне: квад на каждый отрезок, без сглаживания
/// и скруглений — та же зона, по которой находка мерит расстояние до осевой.
fn band_shapes(points: &[Vec2], inner: f32, outer: f32, side: u8) -> Vec<FeatureShape> {
    points
        .windows(2)
        .filter_map(|segment| {
            let direction = (segment[1] - segment[0]).normalize_or_zero();
            if direction == Vec2::ZERO {
                return None;
            }
            let perp = direction.perp();
            let perp = if side == 0 { perp } else { -perp };
            Some(FeatureShape {
                outer: vec![
                    segment[0] + perp * inner,
                    segment[1] + perp * inner,
                    segment[1] + perp * outer,
                    segment[0] + perp * outer,
                ],
                holes: vec![],
            })
        })
        .collect()
}

/// Диск многоугольником из 24 граней — подсветка узла, цилиндра, кроны.
fn disk(center: Vec2, radius: f32) -> FeatureShape {
    let sides = 24;
    FeatureShape {
        outer: (0..sides)
            .map(|i| {
                center + Vec2::from_angle(i as f32 / sides as f32 * std::f32::consts::TAU) * radius
            })
            .collect(),
        holes: vec![],
    }
}
