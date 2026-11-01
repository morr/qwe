//! Проба кармана земли: какой `MapData::pockets` полигон лежит под точкой,
//! чьим покрытием он замкнут и как его край прошёл по стыкам кусков полос.
//!
//! ```text
//! cargo run --example pocket_probe -- x y [city]
//! ```

#[path = "../common/mod.rs"]
mod common;

use bevy::prelude::Vec2;
use qwe::city::City;
use qwe::map::osm::model::{AreaKind, distance_to_area};

/// Длина куска полосы покрытия в звеньях оси (`parse/pockets.rs`, приватно) —
/// дублируется здесь, чтобы рисунок показал те же торцы кусков.
const RUN: usize = 16;

fn main() {
    let mut args = std::env::args().skip(1);
    let x: f32 = args
        .next()
        .unwrap_or_else(|| "usage: pocket_probe x y [city]".into())
        .parse()
        .expect("x is not a number");
    let y: f32 = args
        .next()
        .unwrap_or_else(|| "usage: pocket_probe x y [city]".into())
        .parse()
        .expect("y is not a number");
    let at = Vec2::new(x, y);
    let city = args.next().map_or(City::Tula, |name| {
        City::ALL
            .into_iter()
            .find(|city| city.slug() == name.to_lowercase())
            .unwrap_or_else(|| panic!("unknown city {name}"))
    });
    let map = common::load_map(city);

    // распределение посева по видам и площадям — смена закона покрытия видна
    // на всём городе, а не только у пробы
    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    for area in &map.pockets {
        *kinds.entry(format!("{:?}", area.kind).leak()).or_default() += 1;
    }
    println!("== pocket kinds: {kinds:?}");
    let mut areas: Vec<f32> = map
        .pockets
        .iter()
        .map(|a| signed_area(&a.outer).abs())
        .collect();
    areas.sort_by(f32::total_cmp);
    if let (Some(first), Some(median), Some(last)) =
        (areas.first(), areas.get(areas.len() / 2), areas.last())
    {
        println!("== pocket areas: min {first:.2}, median {median:.2}, max {last:.2} m^2");
    }

    let started = std::time::Instant::now();
    let contours = qwe::map::corner_paving_contours(&map);
    println!(
        "== corner paving: {} contours in {:.1} ms",
        contours.len(),
        started.elapsed().as_secs_f32() * 1e3
    );

    let bounds_of = |outer: &[Vec2]| {
        (
            outer.iter().fold(Vec2::MAX, |a, &b| a.min(b)),
            outer.iter().fold(Vec2::MIN, |a, &b| a.max(b)),
        )
    };
    let near = |bounds: (Vec2, Vec2), reach: f32| {
        bounds.0.x <= at.x + reach
            && bounds.1.x >= at.x - reach
            && bounds.0.y <= at.y + reach
            && bounds.1.y >= at.y - reach
    };

    println!("== pockets within 25 m of {at:.0}");
    for (index, area) in map.pockets.iter().enumerate() {
        if !near(bounds_of(&area.outer), 25.0) {
            continue;
        }
        println!(
            "pocket #{index} kind {:?} area {:.1} m^2, {} vertices",
            area.kind,
            signed_area(&area.outer).abs(),
            area.outer.len()
        );
        for point in &area.outer {
            println!("   {:8.2} {:8.2}", point.x, point.y);
        }
    }

    println!("== roads within 60 m (distance from the point to the axis)");
    for road in &map.roads {
        if !near(bounds_of(&road.points), 60.0) {
            continue;
        }
        let reach = road.sidewalk().mapped_edge(road.width / 2.0);
        let distance = road
            .points
            .windows(2)
            .map(|segment| {
                qwe::map::osm::model::closest_on_segment(at, segment[0], segment[1]).distance(at)
            })
            .fold(f32::INFINITY, f32::min);
        println!(
            "way {:?} class {:?} highway {:?} width {:.1} reach {:.1} verges {:?} points {} axis dist {:.1}",
            road.osm_id,
            road.class,
            road.highway,
            road.width,
            reach,
            road.verges,
            road.points.len(),
            distance,
        );
        for point in &road.points {
            println!("      {:8.2} {:8.2}", point.x, point.y);
        }
    }

    println!("== landuse / grass within 60 m (distance from the point to the ring)");
    for (label, areas) in [("landuse", &map.landuse), ("grass", &map.grass)] {
        for (index, area) in areas.iter().enumerate() {
            if !near(bounds_of(&area.outer), 60.0) {
                continue;
            }
            println!(
                "{label} #{index} kind {:?} osm {:?} ring dist {:.1}",
                area.kind,
                area.osm_id,
                distance_to_area(at, area)
            );
        }
    }

    let svg = draw(&map, bounds_of, at);
    let path = std::env::temp_dir().join("pocket_probe.svg");
    std::fs::write(&path, svg).expect("write svg");
    println!("== svg at {}", path.display());

    let drawn = draw_layers(&map, at);
    let path = std::env::temp_dir().join("pocket_drawn.svg");
    std::fs::write(&path, drawn).expect("write svg");
    println!("== drawn layers at {}", path.display());

    render_drawn(&map, at);
}

/// Офлайн-рендер нарисованных дорожных слоёв: сетка точек, верхний по z слой
/// побеждает — цвет по имени слоя. Контур кармана и красная точка — поверх.
/// Пишет ppm рядом с svg; картинка отвечает «что именно видно на карте».
fn render_drawn(map: &qwe::map::osm::MapData, at: Vec2) {
    let (layers, _report) = qwe::map::mesh_roads(
        map,
        Default::default(),
        Default::default(),
        qwe::map::VergeZoomBucket::for_zoom(0.05),
    );
    let half = 9.0_f32;
    let size = 540; // 60 px на метр
    let named: &[(&str, [u8; 3])] = &[
        ("roads", [204, 68, 68]),
        ("sidewalks", [68, 102, 204]),
        ("road_verges", [68, 170, 68]),
        ("road_verge_lawns", [153, 204, 153]),
        ("road_verge_yards", [102, 170, 102]),
        ("alleys", [221, 136, 68]),
        ("road_paint_zebras", [170, 102, 170]),
        ("road_medians", [238, 238, 238]),
        ("lot_sidewalks", [180, 180, 180]),
        ("lot_lines", [240, 240, 240]),
    ];
    let color_of = |name: &str| -> [u8; 3] {
        named
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, color)| *color)
            .unwrap_or([221, 221, 221])
    };
    // карманы и точка — поверх всего
    let pocket_color = [255, 238, 0];
    let mut image = vec![255u8; size * size * 3];
    let mut sorted: Vec<_> = layers.iter().collect();
    sorted.sort_by(|a, b| a.z.total_cmp(&b.z));
    let to_pixel = |point: Vec2| -> (f32, f32) {
        (
            (point.x - (at.x - half)) / (2.0 * half) * size as f32,
            (point.y - (at.y - half)) / (2.0 * half) * size as f32,
        )
    };
    for layer in &sorted {
        let color = color_of(layer.name);
        for triangle in layer.builder.triangles() {
            // рамка треугольника в пикселях, обрезанная картинкой; вне окна — мимо
            let corners: Vec<(f32, f32)> = triangle
                .iter()
                .map(|&point| to_pixel(point + layer.offset))
                .collect();
            let (mut low_c, mut low_r, mut high_c, mut high_r) =
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for &(c, r) in &corners {
                low_c = low_c.min(c);
                low_r = low_r.min(r);
                high_c = high_c.max(c);
                high_r = high_r.max(r);
            }
            if high_c < 0.0 || low_c > size as f32 || high_r < 0.0 || low_r > size as f32 {
                continue;
            }
            let [a, b, c] = [
                Vec2::new(corners[0].0, corners[0].1),
                Vec2::new(corners[1].0, corners[1].1),
                Vec2::new(corners[2].0, corners[2].1),
            ];
            let (ab, bc, ca) = (b - a, c - b, a - c);
            for row in (low_r as usize).min(size)..(high_r as usize + 1).min(size) {
                for column in (low_c as usize).min(size)..(high_c as usize + 1).min(size) {
                    let point = Vec2::new(column as f32 + 0.5, row as f32 + 0.5);
                    let (ap, bp, cp) = (point - a, point - b, point - c);
                    let (s1, s2, s3) = (ab.perp().dot(ap), bc.perp().dot(bp), ca.perp().dot(cp));
                    if (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0)
                        || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
                    {
                        let pixel = &mut image[(row * size + column) * 3..][..3];
                        pixel.copy_from_slice(&color);
                    }
                }
            }
        }
    }
    // контуры карманов — жёлтым по границе
    for area in &map.pockets {
        let ring = &area.outer;
        for index in 0..ring.len() {
            let edge = (ring[index], ring[(index + 1) % ring.len()]);
            let steps = (edge.0.distance(edge.1) * 20.0) as usize;
            for step in 0..=steps {
                let t = step as f32 / steps.max(1) as f32;
                let point = edge.0.lerp(edge.1, t);
                let column = ((point.x - (at.x - half)) / (2.0 * half) * size as f32) as usize;
                let row = ((point.y - (at.y - half)) / (2.0 * half) * size as f32) as usize;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (c, r) = (column + dx, row + dy);
                    if c < size && r < size {
                        let p = &mut image[(r * size + c) * 3..][..3];
                        p.copy_from_slice(&pocket_color);
                    }
                }
            }
        }
    }
    let path = std::env::temp_dir().join("pocket_drawn.ppm");
    let header = format!("P6\n{size} {size}\n255\n");
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(&image);
    std::fs::write(&path, bytes).expect("write ppm");
    println!(
        "== rendered layers at {} ({} px = {} m)",
        path.display(),
        size,
        2.0 * half
    );
}

fn signed_area(ring: &[Vec2]) -> f32 {
    ring.iter()
        .zip(ring.iter().skip(1).chain(ring.iter().take(1)))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        / 2.0
}

/// Нарисованные дорожные слои (`mesh_roads` — тот же вызов, что и в игре)
/// возле точки, в порядке z, поверх — контуры карманов. Цвет — по имени слоя,
/// легенда печатается в stdout.
fn draw_layers(map: &qwe::map::osm::MapData, at: Vec2) -> String {
    let (layers, _report) = qwe::map::mesh_roads(
        map,
        Default::default(),
        Default::default(),
        qwe::map::VergeZoomBucket::for_zoom(0.05),
    );
    let (low, high) = (at - Vec2::splat(9.0), at + Vec2::splat(9.0));
    let in_window = |points: &[Vec2]| {
        points
            .iter()
            .any(|p| p.x >= low.x && p.x <= high.x && p.y >= low.y && p.y <= high.y)
    };
    // ключевые слои у перекрёстка — контрастно, остальное блёкло
    let named: &[(&str, &str)] = &[
        ("roads", "#cc4444"),
        ("sidewalks", "#4466cc"),
        ("road_verges", "#44aa44"),
        ("road_verge_lawns", "#99cc99"),
        ("road_verge_yards", "#66aa66"),
        ("alleys", "#dd8844"),
        ("road_paint_zebras", "#aa66aa"),
        ("road_medians", "#eeeeee"),
    ];
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1100' height='1100' viewBox='{} {} {} {}'>\n",
        low.x,
        -high.y,
        high.x - low.x,
        high.y - low.y
    ));
    let mut names: Vec<&str> = Vec::new();
    for layer in &layers {
        if !names.contains(&layer.name) {
            names.push(layer.name);
        }
    }
    for layer in &layers {
        let color = named
            .iter()
            .find(|(name, _)| *name == layer.name)
            .map(|(_, color)| *color)
            .unwrap_or("#dddddd");
        for triangle in layer.builder.triangles() {
            if !in_window(&triangle) {
                continue;
            }
            out.push_str("<path d='");
            for (index, point) in triangle.iter().enumerate() {
                out.push_str(&format!(
                    "{}{:.2} {:.2} ",
                    if index == 0 { "M" } else { "L" },
                    point.x + layer.offset.x,
                    -(point.y + layer.offset.y)
                ));
            }
            out.push_str(&format!(
                "Z' fill='{color}' fill-opacity='0.92' stroke='{color}' stroke-width='0.5'/>\n"
            ));
        }
    }
    // контуры карманов и кольца квартала — ориентиры поверх нарисованного
    let outline = |points: &[Vec2], stroke: &str, width: f32| {
        let mut s = String::from("<path d='");
        for (index, point) in points.iter().enumerate() {
            s.push_str(&format!(
                "{}{:.2} {:.2} ",
                if index == 0 { "M" } else { "L" },
                point.x,
                -point.y
            ));
        }
        s.push_str(&format!(
            "Z' fill='none' stroke='{stroke}' stroke-width='{width}' vector-effect='non-scaling-stroke'/>\n"
        ));
        s
    };
    for area in map.landuse.iter().chain(&map.grass) {
        if in_window(&area.outer) {
            out.push_str(&outline(&area.outer, "#3366aa", 1.5));
        }
    }
    for area in &map.pockets {
        if in_window(&area.outer) {
            out.push_str(&outline(&area.outer, "#ffee00", 2.0));
        }
    }
    out.push_str(&format!(
        "<circle cx='{:}' cy='{:}' r='1.0' fill='red'/>\n",
        at.x, -at.y
    ));
    out.push_str("</svg>\n");
    out
}

/// Четверка точек: кусок оси, смещённый на `inner`..`outer` в сторону
/// `side` (+1 — слева по ходу, -1 — справа). Торцы квадратные, стыки без
/// среза — ровно то, чем покрытие грубеет против рисунка.
fn offset_quad(chunk: &[Vec2], inner: f32, outer: f32, side: f32) -> Vec<Vec2> {
    let perp_at = |point: Vec2| -> Vec2 {
        let direction = chunk
            .windows(2)
            .find_map(|segment| {
                if segment[0] == point {
                    Some(segment[1] - segment[0])
                } else if segment[1] == point {
                    Some(point - segment[0])
                } else {
                    None
                }
            })
            .unwrap_or(Vec2::ZERO);
        direction.normalize_or_zero().perp() * side
    };
    let mut quad: Vec<Vec2> = chunk.iter().map(|&p| p + perp_at(p) * outer).collect();
    quad.extend(chunk.iter().rev().map(|&p| p + perp_at(p) * inner));
    quad
}

/// Карман и всё, что его замыкает, в svg (координаты — метры карты, ось y
/// перевёрнута под svg).
fn draw(
    map: &qwe::map::osm::MapData,
    bounds_of: impl Fn(&[Vec2]) -> (Vec2, Vec2),
    at: Vec2,
) -> String {
    let (low, high) = (at - Vec2::splat(30.0), at + Vec2::splat(30.0));
    let visible = |bounds: (Vec2, Vec2)| {
        bounds.1.x >= low.x && bounds.0.x <= high.x && bounds.1.y >= low.y && bounds.0.y <= high.y
    };
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1200' height='1200' viewBox='{} {} {} {}'>\n",
        low.x,
        -high.y,
        high.x - low.x,
        high.y - low.y
    ));
    let ring = |points: &[Vec2], fill: &str, stroke: &str, width: f32| {
        let mut s = String::from("<path d='");
        for (index, point) in points.iter().enumerate() {
            s.push_str(&format!(
                "{}{:.2} {:.2} ",
                if index == 0 { "M" } else { "L" },
                point.x,
                -point.y
            ));
        }
        s.push_str(&format!(
            "Z' fill='{fill}' fill-opacity='0.5' stroke='{stroke}' stroke-width='{width}' vector-effect='non-scaling-stroke'/>\n"
        ));
        s
    };
    let polyline = |points: &[Vec2], stroke: &str| {
        let mut s = String::from("<path d='");
        for (index, point) in points.iter().enumerate() {
            s.push_str(&format!(
                "{}{:.2} {:.2} ",
                if index == 0 { "M" } else { "L" },
                point.x,
                -point.y
            ));
        }
        s.push_str(&format!(
            "' fill='none' stroke='{stroke}' stroke-width='2' stroke-dasharray='4 2' vector-effect='non-scaling-stroke'/>\n"
        ));
        s
    };

    for area in map.landuse.iter().chain(&map.grass) {
        if !visible(bounds_of(&area.outer)) {
            continue;
        }
        let fill = if area.kind == AreaKind::Industrial {
            "#cc9966"
        } else {
            "#ddddaa"
        };
        out.push_str(&ring(&area.outer, fill, "#888844", 1.0));
    }

    // полосы покрытия дорог: куски по RUN звеньев с квадратными торцами —
    // как их видит объединение карманов; стыки кусков и есть подозреваемые
    for road in &map.roads {
        if !visible(bounds_of(&road.points)) {
            continue;
        }
        let reach = road.sidewalk().mapped_edge(road.width / 2.0);
        let last = road.points.len() - 1;
        for start in (0..last).step_by(RUN) {
            let chunk = &road.points[start..=(start + RUN).min(last)];
            out.push_str(&ring(
                &offset_quad(chunk, -reach, reach, 1.0),
                "#999999",
                "#ff0000",
                1.0,
            ));
        }
        // обочина — полосой от кромки до кромки плюс обочина
        for (side, verge) in road.verges.iter().enumerate() {
            if *verge <= 0.0 {
                continue;
            }
            let sign = if side == 0 { 1.0 } else { -1.0 };
            for start in (0..last).step_by(RUN) {
                let chunk = &road.points[start..=(start + RUN).min(last)];
                out.push_str(&ring(
                    &offset_quad(chunk, reach, reach + verge, sign),
                    "#aaddaa",
                    "#00aa00",
                    1.0,
                ));
            }
        }
        out.push_str(&polyline(&road.points, "#0000ff"));
    }

    for area in &map.pockets {
        if !visible(bounds_of(&area.outer)) {
            continue;
        }
        out.push_str(&ring(&area.outer, "#ffee00", "#000000", 2.0));
    }

    out.push_str(&format!(
        "<circle cx='{:}' cy='{:}' r='1.2' fill='red'/>\n",
        at.x, -at.y
    ));
    out.push_str("</svg>\n");
    out
}
