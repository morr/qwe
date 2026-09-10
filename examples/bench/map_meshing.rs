//! Офлайн-замер сборки слоёв карты: сколько вершин и сколько миллисекунд
//! стоит каждый режим отрисовки зданий и слой машин.
//!
//! Bevy-приложение не поднимается, окна нет — и в этом весь смысл. Мерить
//! сборку в живом приложении на macOS **нельзя**: невидимому или свёрнутому
//! окну система урезает приоритет потоков (App Nap), и одна и та же сборка,
//! показывающая 116 мс на активном экране, показывает пять секунд на
//! заблокированном. Здесь нет ни окна, ни GPU — только те же билдеры, что
//! зовёт `spawn_buildings`, на той же карте из кеша Overpass.
//!
//! Профиль тот же dev, что и у игры, поэтому цифры сравнимы с строкой
//! `building meshing:` в логе.
//!
//! ```text
//! cargo run --example map_meshing            # Тула, все пять режимов
//! cargo run --example map_meshing -- Paris   # другой город из кеша
//! ```

#[path = "../common/mod.rs"]
mod common;

use qwe::city::City;
use qwe::map::{
    BuildingHeightMode, BuildingPlan, BuildingZoomBucket, measure_cars, measure_layers,
};

fn main() {
    let city = std::env::args()
        .nth(1)
        .map_or(City::Tula, |name| match name.as_str() {
            "Paris" => City::Paris,
            "Berlin" => City::Berlin,
            "London" => City::London,
            "Tokyo" => City::Tokyo,
            "NewYork" => City::NewYork,
            other => panic!("unknown city {other}"),
        });
    let map = common::load_map(city);
    println!(
        "{city:?}: {} buildings, {} roads",
        map.buildings.len(),
        map.roads.len()
    );

    for mode in BuildingHeightMode::ALL {
        // ступень 0 — с оборудованием на кровле, 1 — без
        for bucket in [0usize, 1] {
            let plan = BuildingPlan {
                mode,
                bucket: BuildingZoomBucket::for_zoom(if bucket == 0 { 0.1 } else { 10.0 }),
                shadows: true,
            };
            let costs = measure_layers(&map.buildings, &map.roads, plan);
            let vertices: usize = costs.iter().map(|cost| cost.vertices).sum();
            let elapsed: f64 = costs
                .iter()
                .map(|cost| cost.elapsed.as_secs_f64() * 1000.0)
                .sum();
            let breakdown: Vec<String> = costs
                .iter()
                .map(|cost| {
                    format!(
                        "{} {:.0}ms/{}k",
                        cost.name,
                        cost.elapsed.as_secs_f64() * 1000.0,
                        cost.vertices / 1000
                    )
                })
                .collect();
            println!(
                "{:>18} clutter {:<5} {:>7} verts {:>7.1} ms   [{}]",
                mode.label(),
                bucket == 0,
                vertices,
                elapsed,
                breakdown.join(", ")
            );
        }
    }

    let (cars, vertices) = measure_cars(&map.roads);
    println!("{cars} parked cars, {vertices} verts");
}
