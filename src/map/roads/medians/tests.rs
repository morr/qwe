use bevy::prelude::*;

use super::{MedianDrawing, MedianInputs, draw};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, MapData, RoadLine};
use crate::map::roads::drawn::Drawn;
use crate::map::roads::junctions::Junctions;
use crate::map::roads::merges::MedianEnd;
use crate::map::roads::network::RoadNetwork;
use crate::map::roads::node_paint::{CrossingMode, NodePaintStyle};

/// Разделённый проспект вдоль x от 100 до 500: две встречные половины в три
/// полосы, `gap` метров между кромками.
fn divided_avenue(gap: f32) -> MapData {
    let width = 3.0 * 3.3 + 1.0;
    let apart = width + gap;
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(3),
        ..street(points, width)
    };
    let roads = vec![
        half(vec![Vec2::new(100.0, 100.0), Vec2::new(500.0, 100.0)]),
        half(vec![
            Vec2::new(500.0, 100.0 + apart),
            Vec2::new(100.0, 100.0 + apart),
        ]),
    ];
    MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    }
}

/// Разделительные проспекта — без краски: что положено в три слоя и что
/// отдано вызывающему.
fn drawn_medians(map: &MapData, markings: bool) -> (MedianDrawing, [MeshBuilder; 3]) {
    let drawn = Drawn::for_test(map);
    let junctions = Junctions::new(
        &drawn,
        map,
        &[],
        NodePaintStyle {
            crossings: CrossingMode::Off,
            stop_lines: false,
        },
    );
    let mut layers = [
        MeshBuilder::with_surface_coords(),
        MeshBuilder::with_surface_coords(),
        MeshBuilder::with_surface_coords(),
    ];
    let [streets, sidewalks, grass] = &mut layers;
    let drawing = draw(
        drawn.pairs(),
        &MedianInputs {
            base: junctions.median_base(),
            paint: junctions.paint(),
            markings,
            pure_merge: &|_| false,
            reach_gores: &|_| {},
            street_of: &|road| Some(road),
        },
        streets,
        sidewalks,
        grass,
    );
    (drawing, layers)
}

#[test]
fn a_paved_median_lays_asphalt_and_hands_its_double_line_back() {
    let map = divided_avenue(0.6);
    let (drawing, [streets, sidewalks, grass]) = drawn_medians(&map, true);
    assert_eq!(drawing.paved.len(), 1);
    assert!(!streets.is_empty(), "асфальт по середине");
    assert!(sidewalks.is_empty() && grass.is_empty());
    assert_eq!(drawing.painted.len(), 1, "двойная сплошная — вызывающему");
    let (midline, breaks) = &drawing.painted[0];
    assert!(breaks.is_empty(), "перекрёстков нет");
    assert_eq!(midline.as_slice(), drawing.paved[0].midline());
    assert_eq!(drawing.ends.len(), 2);
    assert!(
        drawing
            .ends
            .iter()
            .all(|(pair, end)| *pair == [Some(0), Some(1)] && matches!(end, MedianEnd::Paved(_)))
    );
    assert!(drawing.lawn_kerbs.is_empty() && drawing.bed_caps().is_empty());
}

#[test]
fn without_markings_a_paved_median_is_asphalt_only() {
    let map = divided_avenue(0.6);
    let (drawing, [streets, ..]) = drawn_medians(&map, false);
    assert_eq!(drawing.paved.len(), 1);
    assert!(!streets.is_empty());
    assert!(drawing.painted.is_empty() && drawing.ends.is_empty());
}

#[test]
fn a_lawn_lays_its_kerb_and_grass_and_hands_its_noses_back() {
    let map = divided_avenue(5.0);
    let (drawing, [streets, sidewalks, grass]) = drawn_medians(&map, true);
    assert!(drawing.paved.is_empty() && drawing.painted.is_empty());
    assert!(streets.is_empty(), "у газона асфальта нет");
    assert!(!sidewalks.is_empty(), "бордюр — в слой тротуаров");
    assert!(!grass.is_empty());
    assert!(!drawing.lawn_kerbs.is_empty());
    assert!(!drawing.ends.is_empty());
    assert!(
        drawing
            .ends
            .iter()
            .all(|(_, end)| matches!(end, MedianEnd::Lawn(_)))
    );
    assert!(drawing.bed_caps().is_empty(), "полотна нет — нет и торцов");
}
