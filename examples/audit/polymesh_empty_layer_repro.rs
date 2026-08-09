//! Репро падения polymesh: чанк, целиком накрытый препятствием (река,
//! сплошная застройка), даёт слой без полигонов, и `Layer::bake` на нём уходит
//! в бесконечную рекурсию `BVH2d::build` — процесс умирает с `stack overflow`
//! ещё до лога `polymesh baked`.
//!
//! Проходит все города панели и печатает, сколько чанков у каждого пустые.
//! Недостающую выгрузку OSM докачивает в тот же кеш, которым пользуется игра.
//!
//! ```text
//! cargo run --example polymesh_empty_layer_repro -- [radius] [city ...]
//! ```

use qwe::city::City;
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::build_polymesh_from_map;

const OVERPASS_URLS: [&str; 4] = [
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let radius: f32 = args
        .first()
        .map(|a| a.parse().expect("radius"))
        .unwrap_or(0.2);
    let cities: Vec<City> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|name| {
                City::ALL
                    .into_iter()
                    .find(|city| city.slug() == name.to_lowercase())
                    .unwrap_or_else(|| panic!("unknown city {name}"))
            })
            .collect()
    } else {
        City::ALL.to_vec()
    };

    for city in cities {
        let map = load_map(city);
        let build = build_polymesh_from_map(&map, radius).expect("not cancelled");
        let empty: Vec<(u32, u32)> = build
            .mesh
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.polygons.is_empty())
            .map(|(index, _)| {
                let (grid, _) = build.chunks();
                (index as u32 % grid.x, index as u32 / grid.x)
            })
            .collect();
        println!(
            "== {:?} radius {radius}: {} layers, {} of them with zero polygons: {empty:?}",
            city,
            build.mesh.layers.len(),
            empty.len(),
        );
    }
}

fn load_map(city: City) -> MapData {
    let path = overpass::cache_path(city);
    if !path.exists() {
        println!("== {city:?}: no OSM cache, downloading ...");
        let query = overpass::overpass_query(city);
        let json = OVERPASS_URLS
            .iter()
            .find_map(|url| match ureq::post(*url).send(&query) {
                Ok(mut response) => response.body_mut().read_to_string().ok(),
                Err(error) => {
                    println!("   {url} failed: {error}");
                    None
                }
            })
            .expect("every overpass mirror failed");
        std::fs::create_dir_all(path.parent().expect("cache dir")).expect("create cache dir");
        std::fs::write(&path, &json).expect("write cache");
    }
    let json = std::fs::read_to_string(&path).expect("read OSM cache");
    parse::parse(&json, city).expect("failed to parse cached OSM json")
}
