use super::super::fixtures::{LAYERS, a_tee, layer, one_street, road, vertices_of, with_network};
use super::super::*;
use crate::map::osm::fixture;
use crate::map::osm::model::{Pavement, RailKind, RailLine, SidewalkSide};

// --- слои целиком ------------------------------------------------------
//
// Тесты на `mesh_roads`. До шва девять слоёв, три вида материала и вся
// телеметрия области жили внутри `spawn_roads` — 275 строк, взять которые из
// теста было нечем: проверять можно было только хелперы под ними.

#[test]
fn a_street_builds_twenty_five_layers_bottom_up() {
    let (layers, report) = mesh_roads(
        &one_street(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    let names: Vec<&str> = layers.iter().map(|layer| layer.name).collect();
    assert_eq!(names, LAYERS);
    for pair in layers.windows(2) {
        // линии полос и осевые лежат на одной высоте: их полосы не
        // перекрываются, а прячет их ступень зума, не порядок
        let paint_pair = [pair[0].material, pair[1].material]
            .iter()
            .all(|material| matches!(material, MaterialSpec::Paint(_)));
        assert!(
            pair[0].z < pair[1].z || paint_pair && pair[0].z == pair[1].z,
            "{} лежит не ниже {}",
            pair[0].name,
            pair[1].name
        );
    }
    assert!(report.vertices > 0);
}

/// Сплит-дверь собирает ту же геометрию, что слитая: общие слои — тот же
/// z-порядок без ступенезависимых, ступень — свои четыре слоя, хвост мощёных
/// клиньев переехал из тротуаров в `sidewalk_scraps`, а сумма ступени —
/// полная картина (свои слои + общие), как у слитой сборки.
#[test]
fn the_split_door_keeps_the_merged_geometry() {
    let map = one_street();
    let style = RoadStyle::default();
    let shape = RoadShape::default();
    let verge = VergeZoomBucket::for_zoom(0.0);
    let RoadBuild {
        layers,
        report,
        pieces,
        ..
    } = build_roads(&map, style, shape, verge);
    let skeleton = road_skeleton(&map, &style, &shape);
    let build = build_roads_steps(&skeleton, &[verge]);

    let step_names: Vec<&str> = build.steps[0]
        .layers
        .iter()
        .map(|layer| layer.name)
        .collect();
    assert_eq!(
        step_names,
        [
            "road_verge_lawns",
            "road_verge_yards",
            "road_verges",
            "sidewalk_scraps"
        ],
        "ступень несёт только то, чем ступени расходятся"
    );
    let common_names: Vec<&str> = {
        // краска едет отдельно (`build.paint`); полная картина общих слоёв —
        // она и общие слои, и краска, в z-порядке, как у слитой сборки
        let mut common: Vec<&LayerMesh> = build
            .common
            .iter()
            .chain(build.paint.iter().map(|layer| &layer.mesh))
            .collect();
        common.sort_by(|a, b| a.z.total_cmp(&b.z));
        common.iter().map(|layer| layer.name).collect()
    };
    let expected: Vec<&str> = LAYERS
        .iter()
        .copied()
        .filter(|name| !step_names.contains(name))
        .collect();
    assert_eq!(
        common_names, expected,
        "общие слои — список слитой сборки без ступенезависимых, тот же порядок"
    );

    let vertices = |list: &[LayerMesh], name: &str| {
        list.iter()
            .find(|layer| layer.name == name)
            .unwrap()
            .builder
            .vertex_count()
    };
    assert_eq!(
        vertices(&layers, "sidewalks"),
        vertices(&build.common, "sidewalks") + vertices(&build.steps[0].layers, "sidewalk_scraps"),
        "тротуары слитой сборки = общие тротуары + хвост ступени"
    );

    let common_vertices: usize = build
        .common
        .iter()
        .chain(build.paint.iter().map(|layer| &layer.mesh))
        .map(|layer| layer.builder.vertex_count())
        .sum();
    let step_vertices: usize = build.steps[0]
        .layers
        .iter()
        .map(|layer| layer.builder.vertex_count())
        .sum();
    assert_eq!(
        build.steps[0].report.vertices, report.vertices,
        "сумма ступени — полная картина: свои слои + общие"
    );
    assert_eq!(common_vertices + step_vertices, report.vertices);
    assert_eq!(
        build.steps[0].pieces.len(),
        pieces.len(),
        "кусочки ступени — те же, что у слитой сборки"
    );
}

#[test]
fn only_the_bridge_shadow_is_blended() {
    let (layers, _) = mesh_roads(
        &one_street(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    // фактурный материал — у всего, что асфальт, тротуар или дорожка;
    // блендинг — ровно у полупрозрачной тени настила
    for layer in &layers {
        let expected = match layer.name {
            "bridge_shadows" => MaterialSpec::Blend,
            "sidewalks" | "lot_sidewalks" | "road_verges" => {
                MaterialSpec::Surface(SurfaceKind::Sidewalk)
            }
            "alleys" => MaterialSpec::Surface(SurfaceKind::Alley),
            "road_medians" | "ring_islands" | "ring_island_grass" | "road_verge_lawns" => {
                MaterialSpec::Surface(SurfaceKind::Grass)
            }
            "road_verge_yards" => MaterialSpec::Surface(SurfaceKind::Yard),
            "roads" | "bridges" => MaterialSpec::Surface(SurfaceKind::Street),
            "unpaved_roads" => MaterialSpec::Surface(SurfaceKind::Unpaved),
            paint::PAINT_WEAR_MASK => MaterialSpec::Paint(paint::PaintPass::WearMask),
            paint::PAINT_WEAR => MaterialSpec::Paint(paint::PaintPass::Wear),
            paint::PAINT_LANES
            | paint::PAINT_AXES
            | paint::PAINT_ZEBRAS
            | paint::PAINT_ISLANDS
            | paint::BRIDGE_PAINT_LANES
            | paint::BRIDGE_PAINT_AXES => MaterialSpec::Paint(paint::PaintPass::Lines),
            _ => MaterialSpec::Flat,
        };
        assert_eq!(layer.material, expected, "{}", layer.name);
    }
}

/// Грунтовка (`surface=unpaved|gravel|…`) уходит из асфальта в свой слой —
/// без линий краски, и угол двух грунтовок ложится грунтом. Та же Т с
/// асфальтовой улицей: угол — асфальтом, узел асфальтовый.
#[test]
fn an_unpaved_street_draws_in_its_own_layer_without_lines() {
    let unpave = |mut map: MapData, which: &[usize]| {
        for &index in which {
            map.roads[index].pavement = Some(Pavement::Unpaved);
            map.roads[index].sidewalks = [SidewalkSide::None; 2];
        }
        map
    };
    let (layers, report) = mesh_roads(
        &unpave(a_tee(), &[0, 1]),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(!layer(&layers, "unpaved_roads").builder.is_empty());
    assert_eq!(report.paint_lines, 0, "{report}");
    assert!(report.kerb_returns > 0, "углы грунтом: {report}");

    let (mixed, _) = mesh_roads(
        &unpave(a_tee(), &[1]),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(!layer(&mixed, "roads").builder.is_empty());
    assert!(!layer(&mixed, "unpaved_roads").builder.is_empty());
}

/// Асфальтовая улица, упёршаяся в грунтовку, кончается на её кромке: ни
/// лента, ни скругления не заходят на грунт (Тула, 13: торец лежал языком до
/// оси грунтовки).
#[test]
fn an_asphalt_street_stops_at_the_edge_of_the_dirt_road_it_meets() {
    let mut map = a_tee();
    map.roads[0].pavement = Some(Pavement::Unpaved);
    map.roads[0].sidewalks = [SidewalkSide::None; 2];
    map.roads[1].sidewalks = [SidewalkSide::None; 2];
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let asphalt = layer(&layers, "roads").builder.positions_for_test();
    assert!(!asphalt.is_empty());
    // грунтовка по y = 100 шириной 12 — кромка на 106
    let lowest = asphalt.iter().map(|at| at[1]).fold(f32::INFINITY, f32::min);
    assert!(
        lowest > 106.0 - 0.1,
        "асфальт заходит на грунт до y = {lowest}"
    );
}

/// Грунтовка, упёршаяся в асфальт, входит в его кромку как есть: асфальт идёт
/// прямо, без скруглений к ней, и грунт не расходится веером (Калуга, 07).
#[test]
fn a_dirt_road_enters_the_asphalt_without_kerb_returns() {
    let mut map = a_tee();
    map.roads[1].pavement = Some(Pavement::Unpaved);
    for road in &mut map.roads {
        road.sidewalks = [SidewalkSide::None; 2];
    }
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    // асфальт по y = 100 шириной 12 — кромка на 106
    let asphalt = layer(&layers, "roads").builder.positions_for_test();
    let highest = asphalt
        .iter()
        .map(|at| at[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        highest < 106.0 + 0.1,
        "асфальт уходит к грунтовке до y = {highest}"
    );
    // грунтовка по x = 350 шириной 10
    let dirt = layer(&layers, "unpaved_roads").builder.positions_for_test();
    assert!(!dirt.is_empty());
    for at in dirt {
        assert!(
            (at[0] - 350.0).abs() < 5.0 + 0.1,
            "грунт веером до x = {}",
            at[0]
        );
    }
}

/// Асфальт, продолженный грунтовкой, обрывается поперёк: ни его круглый торец
/// не ложится на грунт, ни грунтовый — на асфальт. Излом в 11° — чтобы шов
/// был в сглаживаемой оси (Калуга, 08: дугой сквозь шов узел уходил с оси, и
/// торцы оставались круглыми).
#[test]
fn asphalt_turning_into_dirt_ends_square() {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 100.0), Vec2::new(300.0, 100.0)],
        10.0,
    ));
    map.roads.push(fixture::street(
        vec![Vec2::new(300.0, 100.0), Vec2::new(600.0, 160.0)],
        10.0,
    ));
    map.roads[1].pavement = Some(Pavement::Unpaved);
    for road in &mut map.roads {
        road.sidewalks = [SidewalkSide::None; 2];
    }
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let reach = |name: &str, fold: fn(f32, f32) -> f32, from: f32| {
        let positions = layer(&layers, name).builder.positions_for_test();
        assert!(!positions.is_empty(), "{name}");
        positions.iter().map(|at| at[0]).fold(from, fold)
    };
    let asphalt = reach("roads", f32::max, f32::NEG_INFINITY);
    let dirt = reach("unpaved_roads", f32::min, f32::INFINITY);
    // прямые торцы режутся по биссектрисе излома — полметра за узлом с
    // наружной стороны; круглый торец ушёл бы на полуширину, 5 м
    assert!(
        asphalt < 300.0 + 1.0,
        "асфальт заходит на грунт до x = {asphalt}"
    );
    assert!(
        dirt > 300.0 - 1.0,
        "грунт заходит под асфальт до x = {dirt}"
    );
}

/// Мощёная дорожка ложится плиткой в слой тротуаров, грунтовая и дорожка без
/// решения (собранная руками) — песчаной тропинкой в слой дорожек; со
/// скруглением их узла — так же.
#[test]
fn a_paved_path_is_drawn_in_the_sidewalk_layer() {
    let cross = |pavement: Option<Pavement>| {
        let mut map = MapData::default();
        for points in [
            // Т-узел: общая вершина — скругления в нём
            vec![
                Vec2::new(0.0, 100.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(200.0, 100.0),
            ],
            vec![Vec2::new(100.0, 100.0), Vec2::new(100.0, 200.0)],
        ] {
            map.roads.push(RoadLine {
                pavement,
                ..road(points, 3.5, false)
            });
        }
        let (layers, _) = mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        [
            layer(&layers, "alleys").builder.vertex_count(),
            layer(&layers, "sidewalks").builder.vertex_count(),
        ]
    };
    for pavement in [None, Some(Pavement::Unpaved)] {
        let [alleys, sidewalks] = cross(pavement);
        assert!(
            alleys > 0 && sidewalks == 0,
            "{pavement:?}: {alleys} / {sidewalks}"
        );
    }
    let [alleys, sidewalks] = cross(Some(Pavement::Paved));
    assert!(alleys == 0 && sidewalks > 0, "{alleys} / {sidewalks}");
}

#[test]
fn an_empty_map_still_describes_every_layer() {
    let (layers, report) = mesh_roads(
        &MapData::default(),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    assert_eq!(layers.len(), LAYERS.len());
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
    assert_eq!(report.vertices, 0);
}

#[test]
fn tram_band_follows_the_nodal_axis() {
    // Проезд кончается в трёх метрах за кромкой тротуара улицы и пришит к
    // ней стежком (`a_dangling_end_short_of_a_street_is_stitched`); путь
    // трамвая идёт по проезду и дальше, через улицу. Полоса над путём
    // берёт ось **без** стежка: она кончается у торца OSM (y = −9), а не у
    // оси улицы (y = 0), куда стежок довёл бы ленту.
    let mut map = with_network(vec![
        fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
        fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 5.0),
    ]);
    map.rails.push(RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(vec![Vec2::new(50.0, -70.0), Vec2::new(50.0, 20.0)], 1.2)
    });
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.drawn.stitches, 1);
    assert_eq!(report.tram_bands, 1);
    let band = vertices_of(&layers, "roads", TRAM_BAND_COLOR);
    assert!(!band.is_empty());
    let top = band.iter().map(|at| at[1]).fold(f32::MIN, f32::max);
    assert!(
        top < -5.0,
        "полоса кончается у торца OSM, не у стежка: {top}"
    );
    assert!(top > -12.0, "{top}");
}
