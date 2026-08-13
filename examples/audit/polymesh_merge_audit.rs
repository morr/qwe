//! Временный аудит: сколько соседних полигонов построенного меша можно было бы
//! слить (объединение выпукло), но они остались раздельными, и насколько
//! «в притык» не проходят по выпуклости те, что слить нельзя.
//!
//! ```text
//! cargo run --release --example polymesh_merge_audit -- [agent_radius]
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashSet;

use bevy::math::Vec2;
use qwe::city::City;
use qwe::navigation::build_polymesh_from_map;

const CITY: City = City::Tula;

fn main() {
    let radius: f32 = std::env::args()
        .nth(1)
        .map(|value| value.parse().expect("radius must be a number"))
        .unwrap_or(0.2);

    let map = common::load_map(CITY);
    let build = build_polymesh_from_map(&map, radius).expect("build was not cancelled");
    let (grid, chunk_size) = build.chunks();
    let polygons: usize = build.mesh.layers.iter().map(|l| l.polygons.len()).sum();
    println!(
        "{}x{} chunks of {:.0}x{:.0} m, {polygons} polygons, radius {radius}",
        grid.x, grid.y, chunk_size.x, chunk_size.y
    );

    let mut pairs = 0usize;
    let mut mergeable = 0usize;
    // насколько сильно вершина объединения уходит в вогнутость, градусы
    let mut worst_reflex: Vec<(f32, Vec2)> = Vec::new();
    let mut buckets = [0usize; 6];
    let mut examples: Vec<(f32, Vec2, usize)> = Vec::new();

    for (layer_index, layer) in build.mesh.layers.iter().enumerate() {
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        for (index, polygon) in layer.polygons.iter().enumerate() {
            let count = polygon.vertices.len();
            for corner in 0..count {
                let first = polygon.vertices[corner] as usize;
                let second = polygon.vertices[(corner + 1) % count] as usize;
                let Some(other) = neighbour(layer, layer_index as u32, first, second, index) else {
                    continue;
                };
                let key = (index.min(other), index.max(other));
                if !seen.insert(key) {
                    continue;
                }
                pairs += 1;
                let Some(joined) = join(layer, index, other, first as u32, second as u32) else {
                    continue;
                };
                let reflex = worst_turn(&joined);
                if reflex.0 <= 0.0 {
                    mergeable += 1;
                    if examples.len() < 10 {
                        examples.push((reflex.0, reflex.1, layer_index));
                    }
                } else {
                    let degrees = reflex.0;
                    let bucket = match degrees {
                        d if d < 0.1 => 0,
                        d if d < 0.5 => 1,
                        d if d < 1.0 => 2,
                        d if d < 5.0 => 3,
                        d if d < 15.0 => 4,
                        _ => 5,
                    };
                    buckets[bucket] += 1;
                    worst_reflex.push((degrees, reflex.1));
                }
            }
        }
    }

    geometry_audit(&build.mesh);

    println!("\nadjacent polygon pairs: {pairs}");
    println!("unions that ARE convex (merge missed): {mergeable}");
    for (degrees, point, layer) in &examples {
        println!("  missed: {degrees:.4} deg at {point:?} in layer {layer}");
    }
    println!("\nunions that are concave, by how much (degrees past straight):");
    for (label, count) in [
        ("< 0.1", buckets[0]),
        ("0.1 - 0.5", buckets[1]),
        ("0.5 - 1", buckets[2]),
        ("1 - 5", buckets[3]),
        ("5 - 15", buckets[4]),
        ("> 15", buckets[5]),
    ] {
        println!(
            "  {label:>9}: {count:>7} ({:.1}%)",
            count as f32 / pairs.max(1) as f32 * 100.0
        );
    }
}

/// Проверка самой смежности, а не только слияния: сколько полигонов реально
/// делят каждое ребро (по индексам вершин) и есть ли T-стыки — вершина, лежащая
/// внутри чужого ребра. И то и другое ломает поиск, и `merge_polygons` их не
/// увидит, потому что сам ходит по тем же `vertex.polygons`.
fn geometry_audit(mesh: &polyanya::Mesh) {
    use std::collections::HashMap;

    let mut shared_by_one = 0usize;
    let mut shared_by_many = 0usize;
    let mut junctions = 0usize;
    let mut junction_examples: Vec<Vec2> = Vec::new();
    let mut lookup_disagrees = 0usize;

    for (layer_index, layer) in mesh.layers.iter().enumerate() {
        let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (index, polygon) in layer.polygons.iter().enumerate() {
            let count = polygon.vertices.len();
            for corner in 0..count {
                let first = polygon.vertices[corner];
                let second = polygon.vertices[(corner + 1) % count];
                edges
                    .entry((first.min(second), first.max(second)))
                    .or_default()
                    .push(index);
            }
        }
        for (&(first, second), owners) in &edges {
            match owners.len() {
                1 => shared_by_one += 1,
                2 => {
                    // то же ребро глазами polyanya: если сосед не находится,
                    // смежность разошлась с геометрией
                    let found = neighbour(
                        layer,
                        layer_index as u32,
                        first as usize,
                        second as usize,
                        owners[0],
                    );
                    if found != Some(owners[1]) {
                        lookup_disagrees += 1;
                    }
                }
                _ => shared_by_many += 1,
            }
        }

        // T-стыки: вершина строго внутри отрезка чужого ребра
        let points: Vec<Vec2> = layer
            .vertices
            .iter()
            .map(|vertex| Vec2::new(vertex.coords.x, vertex.coords.y))
            .collect();
        for &(first, second) in edges.keys() {
            let (a, b) = (points[first as usize], points[second as usize]);
            let along = b - a;
            let length = along.length();
            if length == 0.0 {
                continue;
            }
            for (index, &point) in points.iter().enumerate() {
                if index as u32 == first || index as u32 == second {
                    continue;
                }
                let offset = point - a;
                let ratio = offset.dot(along) / (length * length);
                if !(0.001..0.999).contains(&ratio) {
                    continue;
                }
                if (offset - along * ratio).length() < SEAM_TOLERANCE {
                    junctions += 1;
                    if junction_examples.len() < 5 {
                        junction_examples.push(point);
                    }
                }
            }
        }
    }

    println!("\ngeometry:");
    println!("  edges owned by a single polygon (mesh border): {shared_by_one}");
    println!("  edges owned by 3+ polygons (broken): {shared_by_many}");
    println!("  edges where polyanya's neighbour lookup disagrees: {lookup_disagrees}");
    println!("  T-junctions (vertex inside another edge): {junctions}");
    for point in &junction_examples {
        println!("    at {point:?}");
    }
}

/// Допуск T-стыка: на порядок меньше упрощения контуров.
const SEAM_TOLERANCE: f32 = 0.005;

/// Полигон по ту сторону ребра — та же выборка, что делает `merge_polygons`.
fn neighbour(
    layer: &polyanya::Layer,
    layer_index: u32,
    first: usize,
    second: usize,
    skip: usize,
) -> Option<usize> {
    // после сшивки индексы помечены номером слоя в старших 8 битах; чужие слои
    // отбрасываем — внутри слоя merge_polygons и работал
    let others = &layer.vertices.get(second)?.polygons;
    layer
        .vertices
        .get(first)?
        .polygons
        .iter()
        .find(|&&polygon| {
            polygon != u32::MAX
                && polygon >> 24 == layer_index
                && (polygon & 0x00FF_FFFF) as usize != skip
                && others.contains(&polygon)
        })
        .map(|&polygon| (polygon & 0x00FF_FFFF) as usize)
}

/// Контур объединения двух полигонов через ребро `edge0 -> edge1` — точно так
/// же, как его собирает `merge_polygons` перед проверкой выпуклости.
fn join(
    layer: &polyanya::Layer,
    poly: usize,
    other: usize,
    edge0: u32,
    edge1: u32,
) -> Option<Vec<Vec2>> {
    let mine = &layer.polygons[poly].vertices;
    let theirs = &layer.polygons[other].vertices;
    if !theirs.contains(&edge0) || !theirs.contains(&edge1) {
        return None;
    }
    let mut ring: Vec<Vec2> = Vec::new();
    for index in mine
        .iter()
        .chain(mine.iter())
        .skip_while(|i| **i != edge1)
        .take_while(|i| **i != edge0)
    {
        let coords = layer.vertices[*index as usize].coords;
        ring.push(Vec2::new(coords.x, coords.y));
    }
    for index in theirs
        .iter()
        .chain(theirs.iter())
        .skip_while(|i| **i != edge0)
        .take_while(|i| **i != edge1)
    {
        let coords = layer.vertices[*index as usize].coords;
        ring.push(Vec2::new(coords.x, coords.y));
    }
    (ring.len() >= 3).then_some(ring)
}

/// Самый «вогнутый» поворот контура: положительный угол в градусах — насколько
/// вершина ушла за прямую внутрь (то есть контур невыпуклый), ноль или меньше —
/// контур выпуклый. Обход у polyanya против часовой, значит выпуклость это
/// неотрицательное векторное произведение на каждой вершине.
fn worst_turn(ring: &[Vec2]) -> (f32, Vec2) {
    let count = ring.len();
    let mut worst = (f32::MIN, Vec2::ZERO);
    for index in 0..count {
        let previous = ring[(index + count - 1) % count];
        let current = ring[index];
        let next = ring[(index + 1) % count];
        let incoming = current - previous;
        let outgoing = next - current;
        if incoming.length_squared() == 0.0 || outgoing.length_squared() == 0.0 {
            continue;
        }
        // угол поворота: > 0 — влево (выпукло при CCW), < 0 — вправо (вогнуто)
        let turn = incoming.angle_to(outgoing).to_degrees();
        if -turn > worst.0 {
            worst = (-turn, current);
        }
    }
    worst
}
