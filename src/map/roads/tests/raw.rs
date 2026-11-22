use super::super::fixtures::{layer, one_street};
use super::super::*;

/// Сырой OSM, второй уровень (`RawOsm::Draw`): улица — одна простая лента по
/// оси шириной из разбора, без тротуара, разметки и колеи; без флага та же
/// улица получает тротуар и краску.
#[test]
fn raw_osm_draw_lays_a_bare_ribbon_per_way() {
    use crate::map::osm::parse::RawOsm;
    let vertices = |layers: &[LayerMesh], name: &str| {
        layers
            .iter()
            .filter(|layer| layer.name == name)
            .map(|layer| layer.builder.vertex_count())
            .sum::<usize>()
    };
    let paint = |layers: &[LayerMesh]| {
        layers
            .iter()
            .filter(|layer| matches!(layer.material, MaterialSpec::Paint(_)))
            .map(|layer| layer.builder.vertex_count())
            .sum::<usize>()
    };

    let cooked = one_street();
    let (layers, _) = mesh_roads(
        &cooked,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(vertices(&layers, "sidewalks") > 0);
    assert!(paint(&layers) > 0);

    let mut raw = one_street();
    raw.knobs.raw = RawOsm::Draw;
    let (layers, report) = mesh_roads(
        &raw,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(vertices(&layers, "sidewalks"), 0);
    assert_eq!(paint(&layers), 0);
    // одна лента по полуширине от оси, с прямыми торцами ровно в концах way
    let road = layer(&layers, "roads").builder.positions_for_test();
    assert!(!road.is_empty() && road.len() <= 8, "{} вершин", road.len());
    assert!(road.iter().all(|at| (at[1] - 100.0).abs() < 6.01));
    assert!(road.iter().all(|at| (99.99..=600.01).contains(&at[0])));
    assert_eq!(report.kerb_returns, 0);
}
