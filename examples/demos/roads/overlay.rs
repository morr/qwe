//! Оверлей сети: улицы [`MapData::network`] поверх примера — каждая своим
//! цветом, толщина линии по числу полос участка, белая точка на каждом шве
//! между ways одной улицы. Им проверяется склейка и сечения
//! (`map::roads::network`): улица, разорванная на перекрёстке не того
//! класса, или участок с чужим числом полос видны сразу.
//!
//! Строка `Network` панели, либо `ROADS_NETWORK=1` при запуске — для
//! автоснимка.

use bevy::prelude::*;
use qwe::map::osm::MapData;
use qwe::map::surface::{LayerMesh, MaterialSpec};
use qwe::map::{MeshBuilder, RibbonCap, RibbonJoin};

/// Поверх зданий и крон, под рамками окон витрины.
const Z_NETWORK: f32 = 29.0;
/// Толщина линии улицы: основа и прибавка на полосу, м.
const LINE_BASE: f32 = 0.6;
const LINE_PER_LANE: f32 = 0.5;
/// Радиус точки шва, м.
const SEAM_RADIUS: f32 = 1.4;

/// Цвета улиц по кругу: соседние улицы почти всегда получают разные.
const PALETTE: [Color; 8] = [
    Color::srgb(0.90, 0.20, 0.25),
    Color::srgb(0.15, 0.55, 0.95),
    Color::srgb(0.20, 0.75, 0.30),
    Color::srgb(0.95, 0.60, 0.10),
    Color::srgb(0.65, 0.30, 0.85),
    Color::srgb(0.10, 0.75, 0.75),
    Color::srgb(0.85, 0.35, 0.65),
    Color::srgb(0.55, 0.55, 0.10),
];

/// Показывать ли оверлей сети.
#[derive(Resource, Clone, Copy, PartialEq)]
pub(crate) struct NetworkOverlay {
    pub visible: bool,
}

impl Default for NetworkOverlay {
    fn default() -> Self {
        Self {
            visible: std::env::var_os("ROADS_NETWORK").is_some(),
        }
    }
}

/// Слой оверлея одного примера.
pub(crate) fn mesh_network(map: &MapData) -> LayerMesh {
    let mut builder = MeshBuilder::default();
    let roads = &map.roads;
    for (index, street) in map.network.streets.iter().enumerate() {
        let color = PALETTE[index % PALETTE.len()].to_linear();
        for way in &street.ways {
            let road = &roads[way.road];
            let lanes = f32::from(road.lanes.unwrap_or(1));
            builder.push_ribbon(
                &road.points,
                false,
                LINE_BASE + LINE_PER_LANE * lanes,
                color,
                RibbonJoin::Round,
                RibbonCap::Butt,
            );
        }
    }
    let seam = Color::WHITE.to_linear();
    for (way, _) in map.network.joints() {
        let at = way.exit(roads);
        let disc: Vec<Vec2> = (0..12)
            .map(|step| {
                let angle = step as f32 / 12.0 * std::f32::consts::TAU;
                at + Vec2::from_angle(angle) * SEAM_RADIUS
            })
            .collect();
        builder.push_polygon(&disc, &[], seam);
    }
    LayerMesh::new(builder, Z_NETWORK, "road_network", MaterialSpec::Flat)
}
