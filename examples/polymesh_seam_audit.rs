//! Временный аудит шва: какие вершины на общей кромке чанков остались без пары
//! на другой стороне — то, на чём падает `debug_assert` в `stitch_chunks`.
//!
//! ```text
//! cargo run --release --example polymesh_seam_audit -- [radius ...]
//! ```

use bevy::math::{UVec2, Vec2};
use qwe::city::City;
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::build_polymesh_from_map;

const CITY: City = City::Tula;
const SEAM_EPSILON: f32 = 1e-3;

fn main() {
    let radii: Vec<f32> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            (2..=10).map(|step| step as f32 / 10.0).collect()
        } else {
            args.iter().map(|a| a.parse().expect("radius")).collect()
        }
    };

    let map = load_map();
    for radius in radii {
        let build = build_polymesh_from_map(&map, radius).expect("not cancelled");
        let (grid, chunk_size) = build.chunks();
        let mesh = &build.mesh;
        let mut lonely: Vec<(usize, usize, Vec2, bool)> = Vec::new();

        for y in 0..grid.y {
            for x in 0..grid.x {
                let chunk = (y * grid.x + x) as usize;
                for (neighbour, along_y) in [
                    (x + 1 < grid.x).then(|| (chunk + 1, true)),
                    (y + 1 < grid.y).then(|| (chunk + grid.x as usize, false)),
                ]
                .into_iter()
                .flatten()
                {
                    let node = |nx: u32, ny: u32| {
                        Vec2::new(
                            quantized(nx as f32 * chunk_size.x),
                            quantized(ny as f32 * chunk_size.y),
                        )
                    };
                    let (start, end) = if along_y {
                        (node(x + 1, y), node(x + 1, y + 1))
                    } else {
                        (node(x, y + 1), node(x + 1, y + 1))
                    };
                    let here =
                        mesh.layers[chunk].get_vertices_on_segment(to_poly(start), to_poly(end));
                    let there = mesh.layers[neighbour]
                        .get_vertices_on_segment(to_poly(start), to_poly(end));
                    if here.is_empty() || there.is_empty() {
                        continue;
                    }
                    // обе стороны, а не только `here`: в стежке считается лишь
                    // одна, но асимметрия ломает шов в любую сторону
                    for (mine, other, from, to) in [
                        (&here, &there, chunk, neighbour),
                        (&there, &here, neighbour, chunk),
                    ] {
                        for &vertex in mine.iter() {
                            let world = from_poly(mesh.layers[from].vertices[vertex].coords);
                            if [start, end]
                                .iter()
                                .any(|c| c.distance_squared(world) <= SEAM_EPSILON * SEAM_EPSILON)
                            {
                                continue;
                            }
                            let matched = other.iter().any(|&o| {
                                from_poly(mesh.layers[to].vertices[o].coords)
                                    .distance_squared(world)
                                    <= SEAM_EPSILON * SEAM_EPSILON
                            });
                            if !matched {
                                let dangerous = covered(&mesh.layers[to], to as u32, other, world);
                                lonely.push((from, to, world, dangerous));
                            }
                        }
                    }
                }
            }
        }

        println!(
            "radius {radius}: {} unmatched seam vertices, {} of them splitting an edge the \
             neighbour keeps whole ({}x{} chunks)",
            lonely.len(),
            lonely.iter().filter(|(_, _, _, bad)| *bad).count(),
            grid.x,
            grid.y
        );
        for (from, to, point, dangerous) in lonely.iter().take(20) {
            let cell = |c: usize| UVec2::new(c as u32 % grid.x, c as u32 / grid.x);
            println!(
                "  chunk {:?} has {point:?}, chunk {:?} does not{}",
                cell(*from),
                cell(*to),
                if *dangerous {
                    " — AND HAS A SEAM EDGE ACROSS IT"
                } else {
                    ""
                }
            );
            // окрестность точки: что лежит на шве у обоих соседей и где шов
            // пересекают контуры препятствий
            let vertical = (point.x / chunk_size.x).fract().abs() < 1e-4;
            for &layer in &[*from, *to] {
                let mut near: Vec<f32> = mesh.layers[layer]
                    .vertices
                    .iter()
                    .map(|v| from_poly(v.coords))
                    .filter(|p| p.distance(*point) < 2.0)
                    .map(|p| if vertical { p.y } else { p.x })
                    .collect();
                near.sort_by(f32::total_cmp);
                println!("    layer {:?} within 2 m: {near:?}", cell(layer));
            }
            let mut crossings: Vec<f32> = Vec::new();
            let line = if vertical { point.x } else { point.y };
            for contour in &build.obstacles {
                for index in 0..contour.len() {
                    let a = contour[index];
                    let b = contour[(index + 1) % contour.len()];
                    let (from_c, to_c, other_from, other_to) = if vertical {
                        (a.x, b.x, a.y, b.y)
                    } else {
                        (a.y, b.y, a.x, b.x)
                    };
                    if (from_c < line) == (to_c < line) {
                        continue;
                    }
                    let ratio = (line - from_c) / (to_c - from_c);
                    let value = quantized(other_from + (other_to - other_from) * ratio);
                    if (value - if vertical { point.y } else { point.x }).abs() < 2.0 {
                        crossings.push(value);
                    }
                }
            }
            crossings.sort_by(f32::total_cmp);
            crossings.dedup();
            println!("    obstacle crossings of the seam line: {crossings:?}");
        }
    }
}

/// Опасен ли непарный шов: есть ли у соседа настоящее ребро, накрывающее эту
/// точку — пара соседних по шву вершин, лежащих в одном его полигоне. Если
/// есть, наша лишняя вершина рвёт это ребро надвое, и переход через шов
/// находится только с одной стороны. Если нет, у соседа там стена.
fn covered(layer: &polyanya::Layer, layer_index: u32, list: &[usize], point: Vec2) -> bool {
    list.windows(2).any(|pair| {
        let (first, second) = (pair[0], pair[1]);
        let a = from_poly(layer.vertices[first].coords);
        let b = from_poly(layer.vertices[second].coords);
        let along = b - a;
        let length_squared = along.length_squared();
        if length_squared == 0.0 {
            return false;
        }
        let ratio = (point - a).dot(along) / length_squared;
        if !(0.0..=1.0).contains(&ratio) {
            return false;
        }
        // после сшивки индексы помечены номером слоя в старших 8 битах
        let others = &layer.vertices[second].polygons;
        layer.vertices[first].polygons.iter().any(|&polygon| {
            polygon != u32::MAX && polygon >> 24 == layer_index && others.contains(&polygon)
        })
    })
}

const SEAM_QUANTUM: f32 = 0.01;

fn quantized(value: f32) -> f32 {
    (value / SEAM_QUANTUM).round() * SEAM_QUANTUM
}

fn to_poly(point: Vec2) -> polyanya_glam::Vec2 {
    polyanya_glam::Vec2::new(point.x, point.y)
}

fn from_poly(point: polyanya_glam::Vec2) -> Vec2 {
    Vec2::new(point.x, point.y)
}

fn load_map() -> MapData {
    let path = overpass::cache_path(CITY);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no OSM cache at {}: {error}. run the app once to download it",
            path.display()
        )
    });
    parse::parse(&json, CITY).expect("failed to parse cached OSM json")
}
