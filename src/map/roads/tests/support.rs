use super::super::*;
use crate::map::osm::{AreaKind, PolyArea, fixture};

fn fortress(outer: Vec<Vec2>) -> PolyArea {
    PolyArea {
        height: Some(12.0),
        ..fixture::area(AreaKind::Kremlin, outer)
    }
}

/// Лента кремлёвской стены не ложится поверх крепостных зданий: остаётся
/// только кусок в стороне от них, а огрызок между двумя зданиями пропадает.
#[test]
fn the_city_wall_ribbon_stays_off_fortress_buildings() {
    let wall = vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)];
    // стена-здание на первых ста метрах, башня на 104…114 — между ними 4 м
    let buildings = [
        fortress(fixture::rect(Vec2::new(-1.0, -2.0), Vec2::new(100.0, 2.0))),
        fortress(fixture::rect(Vec2::new(104.0, -5.0), Vec2::new(114.0, 5.0))),
    ];
    let runs = Fortresses::of(&buildings).bare_runs(&wall);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run = &runs[0];
    assert!(
        // точка на самой кромке башни ничья — кусок начинается не раньше неё
        run[0].x >= 114.0 && run.last().unwrap().x == 200.0,
        "{run:?}"
    );

    // без крепостных зданий лента целиком — единственный рисунок стены
    let alone = Fortresses::of(&[]).bare_runs(&wall);
    assert_eq!(alone, vec![wall.clone()]);
    // и короткая неразрезанная лента не пропадает
    let short = vec![Vec2::new(500.0, 0.0), Vec2::new(505.0, 0.0)];
    assert_eq!(Fortresses::of(&buildings).bare_runs(&short).len(), 1);
}

pub(super) fn extent_x(builder: &MeshBuilder) -> (f32, f32) {
    builder
        .positions_for_test()
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), at| {
            (low.min(at[0]), high.max(at[0]))
        })
}

// DIAG-TEMP: `QWE_DIAG_JSON=<срез> QWE_DIAG_CITY=oryol QWE_DIAG=x,y,r QWE_DIAG_PROBES="x,y;x,y"`
#[test]
#[ignore]
fn diag_window() {
    use crate::city::City;
    use crate::map::osm::overpass::OverpassResponse;
    use crate::map::osm::parse::{ParseKnobs, parse_response};
    let path = std::env::var("QWE_DIAG_JSON").unwrap();
    let city = match std::env::var("QWE_DIAG_CITY").unwrap().as_str() {
        "oryol" => City::Oryol,
        _ => City::Tula,
    };
    let osm: OverpassResponse =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let map = parse_response(&osm, city, ParseKnobs::default());
    if let Ok(spec) = std::env::var("QWE_DIAG") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let prepared = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
        for i in 0..prepared.len() {
            let d = prepared.road(i);
            let line = prepared.axis(i, Axis::Ribbon);
            if line.iter().any(|p| p.distance(at) < r) {
                eprintln!(
                    "DIAG road {i} hw={:?} class={:?} w={} oneway={} lanes={:?} bw={:?} osm={:?}\n    ribbon={:?}\n    nodal={:?}",
                    d.highway,
                    d.class,
                    d.width,
                    d.oneway,
                    d.lanes,
                    d.lanes_backward,
                    d.points,
                    line,
                    prepared.axis(i, crate::map::roads::Axis::Nodal)
                );
                eprintln!("DIAG   verges={:?} profile={:?}", d.verges, d.verge_profile);
            }
        }
        let returns =
            super::super::corners::kerb_returns(&prepared, 1.0, VergeZoomBucket::for_zoom(0.0));
        for (index, merge) in prepared.merges().list.iter().enumerate() {
            if merge.node.distance(at) < r * 2.0 {
                eprintln!(
                    "DIAG merge {index} node={:?} halves={:?} street={} street_end={} pure={}",
                    merge.node, merge.halves, merge.street, merge.street_end, merge.pure
                );
            }
        }
        for i in 0..prepared.len() {
            if prepared
                .axis(i, crate::map::roads::Axis::Ribbon)
                .iter()
                .any(|p| p.distance(at) < r)
            {
                eprintln!(
                    "DIAG is_merged road {i} = {:?}",
                    [prepared.is_merged(i, 0), prepared.is_merged(i, 1)]
                );
            }
        }
        returns.dump_near(at, r);
        for i in 0..prepared.len() {
            let b = returns.butt(i);
            if b != [false; 2]
                && prepared
                    .axis(i, Axis::Ribbon)
                    .iter()
                    .any(|p| p.distance(at) < r)
            {
                eprintln!("DIAG butt road {i} {b:?}");
            }
        }
        let nodes = prepared.nodes();
        for i in 0..prepared.len() {
            if prepared
                .axis(i, Axis::Ribbon)
                .iter()
                .any(|p| p.distance(at) < r)
            {
                let nodal = prepared.axis(i, crate::map::roads::Axis::Nodal);
                let shared = [0, nodal.len() - 1]
                    .map(|v| (nodes.is_shared(nodal[v]), nodes.roads_at(nodal[v]).to_vec()));
                eprintln!(
                    "DIAG ends road {i} stitched={:?} at={:?} offset={} shared={shared:?}",
                    prepared.stitched_end(i),
                    prepared.stitches().ends[i],
                    prepared.stitch_offset(i),
                );
            }
        }
    }
    if let Ok(spec) = std::env::var("QWE_DIAG_RING") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let near = |ring: &[Vec2]| ring.iter().any(|p| p.distance(at) < r);
        let layers: [(&str, &Vec<crate::map::osm::model::PolyArea>); 10] = [
            ("buildings", &map.buildings),
            ("water", &map.water),
            ("parks", &map.parks),
            ("woods", &map.woods),
            ("grass", &map.grass),
            ("sand", &map.sand),
            ("landuse", &map.landuse),
            ("parking", &map.parking),
            ("pitches", &map.pitches),
            ("pockets", &map.pockets),
        ];
        for (name, areas) in layers {
            for (i, area) in areas.iter().enumerate() {
                if near(&area.outer) {
                    eprintln!("DIAG ring {name}[{i}] kind={:?} outer={area:?}", area.kind);
                }
            }
        }
    }
    // DIAG-TEMP R37-01: та же кормилка лоскутов, что в mesh_roads, — окна
    // газонов и найденные куски у точки: `QWE_DIAG_SCRAPS=x,y,r`,
    // `QWE_DIAG_SCRAPS_FIND=1` добавляет вычитание мощения и куски
    if let Ok(spec) = std::env::var("QWE_DIAG_SCRAPS") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        let style = RoadStyle::default();
        let shape = RoadShape::default();
        let prepared = Drawn::new(&map, &style, &shape);
        let ribbon: Vec<Vec<Vec2>> = (0..prepared.len())
            .map(|i| prepared.axis(i, Axis::Ribbon).to_vec())
            .collect();
        let returns = super::super::corners::kerb_returns(
            &prepared,
            shape.corner_radius(),
            VergeZoomBucket::for_zoom(0.0),
        );
        let meadows = super::super::Meadows::of(&map.parks, &map.grass);
        let mut scraps = super::super::scraps::Scraps::default();
        // окна газонов и асфальт узлов — тем же укладщиком, что и в
        // mesh_roads (`KerbReturns::lay`); билдеры слоёв выбрасываются,
        // важен вклад в лоскуты
        let mut streets = MeshBuilder::with_surface_coords();
        let mut alleys = MeshBuilder::with_surface_coords();
        let mut unpaved = MeshBuilder::with_surface_coords();
        let mut sidewalk = MeshBuilder::with_surface_coords();
        let mut verge = MeshBuilder::with_surface_coords();
        let mut tiles = MeshBuilder::with_surface_coords();
        let mut lawns = MeshBuilder::with_surface_coords();
        let mut yards = MeshBuilder::with_surface_coords();
        let mut pieces = Vec::new();
        returns.lay(
            super::super::corners::KerbLayers {
                streets: &mut streets,
                alleys: &mut alleys,
                unpaved: &mut unpaved,
                sidewalks: &mut sidewalk,
                verges: &mut verge,
                lawns: &mut lawns,
                yards: &mut yards,
            },
            &meadows,
            &mut scraps,
            &mut pieces,
        );
        let road_bites = super::super::scraps::Bites::of(prepared.nodes(), &prepared.roads());
        for index in 0..prepared.len() {
            let road = prepared.road(index);
            let mut verged = prepared.verges_drawn(index, VergeZoomBucket::for_zoom(0.0));
            if let Some(ring) = prepared
                .rings()
                .index_of(index)
                .and_then(|i| prepared.rings().list.get(i))
            {
                verged[usize::from(!ring.ccw)] = 0.0;
            }
            let bites = if verged[0] > 0.0 || verged[1] > 0.0 {
                ribbon[index][1..ribbon[index].len().saturating_sub(1)]
                    .iter()
                    .filter(|&point| road_bites.bite(index, *point))
                    .copied()
                    .collect()
            } else {
                Vec::new()
            };
            super::super::verges::push_verges(
                [&mut tiles, &mut lawns, &mut yards],
                &meadows,
                index as u32,
                road,
                &ribbon[index],
                returns.butt(index),
                &bites,
                verged,
                &mut scraps,
                &mut pieces,
            );
        }
        for (index, (ring, cuts)) in scraps.lawns().enumerate() {
            let near = ring
                .iter()
                .chain(cuts.iter().flat_map(|cut| cut.iter()))
                .any(|p| p.distance(at) < r);
            if near {
                eprintln!("DIAG lawn window {index} ring={ring:?} cuts={cuts:?}");
            }
        }
        if std::env::var("QWE_DIAG_SCRAPS_FIND").is_ok() {
            // плоское мощение, клинья развилок, мощёные носы и дуги — тем же
            // порядком, что в mesh_roads
            for outline in returns
                .paving()
                .flat()
                .chain(returns.fork_gores().iter().map(Vec::as_slice))
                .chain(returns.paving().curved())
            {
                if scraps.near_lawn(outline, 0.0) {
                    scraps.push_paving(outline);
                }
            }
            for (index, axis) in ribbon.iter().enumerate() {
                let road = prepared.road(index);
                let paved = !road.is_unpaved_street()
                    && (road.class == crate::map::osm::RoadClass::Street || road.is_paved_path());
                if !paved || road.bridge {
                    continue;
                }
                let halves = [0, 1].map(|side| prepared.band_half(index, side));
                if scraps.near_lawn(axis, halves[0].max(halves[1])) {
                    scraps.push_band(axis, halves);
                }
            }
            for area in map.pockets.iter().chain(&map.grass) {
                scraps.push_green(area.outer.clone());
            }
            if std::env::var("QWE_DIAG_SCRAPS_ALL").is_ok() {
                for (ring, area, avg, open, tip) in scraps.debug_pieces() {
                    let Some(&near) = ring.iter().find(|p| p.distance(at) < r) else {
                        continue;
                    };
                    eprintln!(
                        "DIAG piece at {near} area={area:.1} avg={avg:.2} open={open:.1} tip={tip} ring={ring:?}"
                    );
                }
            }
            for (shape, _) in scraps.find_green() {
                let ring = crate::map::shapes::ring_of(&shape[0]);
                let Some(&tip) = ring.iter().find(|p| p.distance(at) < r) else {
                    continue;
                };
                eprintln!("DIAG green scrap at {tip} ring={ring:?}");
            }
            for (shape, _) in scraps.find() {
                let ring = crate::map::shapes::ring_of(&shape[0]);
                let Some(&tip) = ring.iter().find(|p| p.distance(at) < r) else {
                    continue;
                };
                eprintln!("DIAG scrap at {tip} ring={ring:?}");
            }
        }
    }
    let (mut layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    eprintln!("DIAG report {report}");
    let layout = crate::map::parking::ParkingLayout::new(&map);
    layers.extend(crate::map::spawn::mesh_surfaces(&map, &layout).0);
    if let Ok(spec) = std::env::var("QWE_DIAG_MESH") {
        let v: Vec<f32> = spec.split(',').filter_map(|s| s.parse().ok()).collect();
        let (at, r) = (Vec2::new(v[0], v[1]), v[2]);
        for layer in &layers {
            let pos = layer.builder.positions_for_test();
            let near: Vec<Vec2> = pos
                .iter()
                .map(|p| Vec2::new(p[0], p[1]))
                .filter(|p| p.distance(at) < r)
                .collect();
            if !near.is_empty() {
                eprintln!("DIAG mesh {} {} {near:?}", layer.name, near.len());
            }
        }
    }
    if let Ok(spec) = std::env::var("QWE_DIAG_COLUMN") {
        let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let (from, to, step) = (Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), v[4]);
        let count = (from.distance(to) / step) as usize;
        for i in 0..=count {
            let at = from.lerp(to, i as f32 / count.max(1) as f32);
            let mut top: Vec<(f32, &str)> = layers
                .iter()
                .filter(|layer| {
                    layer
                        .builder
                        .triangles()
                        .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
                })
                .map(|layer| (layer.z, layer.name))
                .collect();
            top.sort_by(|a, b| b.0.total_cmp(&a.0));
            top.truncate(3);
            eprintln!("PROBE col {at} {top:?}");
        }
    }
    let spec = std::env::var("QWE_DIAG_PROBES").unwrap_or_default();
    for probe in spec.split(';').filter(|s| !s.is_empty()) {
        let v: Vec<f32> = probe.split(',').map(|s| s.parse().unwrap()).collect();
        let at = Vec2::new(v[0], v[1]);
        let only = std::env::var("QWE_DIAG_LAYER").unwrap_or_default();
        for layer in layers.iter().filter(|l| l.name.contains(only.as_str())) {
            let pos = layer.builder.positions_for_test();
            let coords = layer.builder.ribbon_coords_for_test();
            let col = layer.builder.colors_for_test();
            for [a, b, c] in layer.builder.triangles() {
                if crate::map::osm::model::point_in_polygon(at, &[a, b, c]) {
                    let info: Vec<_> = [a, b, c]
                        .iter()
                        .map(|p| {
                            let i = pos.iter().position(|q| Vec2::new(q[0], q[1]) == *p);
                            (
                                *p,
                                i.and_then(|i| coords.map(|c| c[i])),
                                i.map(|i| col[i][3]),
                            )
                        })
                        .collect();
                    eprintln!("PROBE {at} {} {:?}", layer.name, info);
                }
            }
        }
    }
}
