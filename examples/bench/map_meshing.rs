//! Офлайн-замер сборки слоёв карты: сколько вершин и сколько миллисекунд
//! стоит каждый режим отрисовки зданий, слой машин, поверхности, дороги,
//! рельсы и трамвай — то есть всё, что собирают `mesh_*` модулей карты.
//!
//! Bevy-приложение не поднимается, окна нет — и в этом весь смысл. Мерить
//! сборку в живом приложении на macOS **нельзя**: невидимому или свёрнутому
//! окну система урезает приоритет потоков (App Nap), и одна и та же сборка,
//! показывающая 116 мс на активном экране, показывает пять секунд на
//! заблокированном. Здесь нет ни окна, ни GPU — только те же билдеры, что
//! зовёт `mesh_buildings`, на той же карте из кеша Overpass.
//!
//! **Абсолютные числа зависят от энергетического состояния машины** (со спящим
//! экраном всё в 2–3 раза медленнее), поэтому сравнивать надо прогон с
//! прогоном, а не прогон со строкой `building meshing:` в логе.
//!
//! **Тени мерятся при дефолтном солнце** (`SunStyle::default()`, 300°/59°), и
//! прогон его печатает: от высоты солнца зависит длина свипа, то есть площадь
//! теневого объединения — самого дорогого слоя здесь. В живом приложении солнце
//! приходит из `settings.toml`, так что и по этой причине лог с прогоном
//! несравним.
//!
//! ```text
//! cargo run --example map_meshing            # Тула, все пять режимов
//! cargo run --example map_meshing -- paris   # другой город кеша, по slug
//! ```

#[path = "../common/mod.rs"]
mod common;

use qwe::city::City;
use qwe::map::{
    BuildingHeightMode, LayerCost, SunStyle, measure_cars, measure_layers, measure_rails,
    measure_roads, measure_surfaces, measure_tram,
};

fn main() {
    // по slug, как в остальных офлайн-инструментах (`polymesh_start_area`):
    // список городов живёт в `City::ALL`, а не копией на каждый пример
    let city = std::env::args().nth(1).map_or(City::Tula, |name| {
        City::ALL
            .into_iter()
            .find(|city| city.slug() == name.to_lowercase())
            .unwrap_or_else(|| panic!("unknown city {name}"))
    });
    let map = common::load_map(city);
    // солнце ставится явно, и прогон его печатает: длина и направление свипа
    // теней живут в процессной глобали, которую в игре пишет `apply_sun`, а
    // приложения здесь нет — без этой строки тени мерились бы при
    // компайл-таймовом дефолте статиков, о котором прогон молчит. Берётся
    // дефолт, а не `settings.toml`: прогон обязан быть сравним с прогоном
    let sun = SunStyle::default();
    qwe::map::apply_sun_style(sun);
    println!(
        "{city:?}: {} buildings, {} roads, sun {:.0}° az / {:.0}° el",
        map.buildings.len(),
        map.roads.len(),
        sun.azimuth(),
        sun.elevation(),
    );

    for mode in BuildingHeightMode::ALL {
        // ступени зума у слоя зданий две, и различает их ровно оборудование на
        // кровле (порог `ROOF_CLUTTER_MAX_ZOOM`): замер берёт сам флаг, а не
        // зум по обе стороны порога
        for clutter in [true, false] {
            let costs = measure_layers(&map.buildings, &map.roads, mode, clutter);
            let (vertices, elapsed, breakdown) = totals(&costs);
            println!(
                "{:>18} clutter {:<5} {vertices:>7} verts {elapsed:>7.1} ms   [{breakdown}]",
                mode.label(),
                clutter,
            );
        }
    }

    // машины — тем же форматом и с теми же миллисекундами: слой сравнивается
    // со зданиевыми (он на порядок дешевле, и это надо видеть, а не помнить)
    let (cars, costs) = measure_cars(&map.buildings, &map.roads, map.traffic_side);
    let (vertices, elapsed, breakdown) = totals(&costs);
    println!(
        "{:>18} {cars:>8} cars {vertices:>7} verts {elapsed:>7.1} ms   [{breakdown}]",
        "parked cars",
    );

    // Остальные слои карты. Своей сборки у этих замеров нет — каждый зовёт тот
    // же `mesh_*`, что и игра; до шва они мерились только строками
    // `road meshing:` / `rail meshing:` / `tram meshing:` из живого приложения,
    // то есть ровно тем способом, который на macOS решает App Nap.
    row("surfaces", &measure_surfaces(&map));
    row("roads", &measure_roads(&map));
    // у рельсов и трамвая ступени зума отличаются не размером, а тем, что
    // нарисовано, поэтому строка на ступень
    for (bucket, costs) in measure_rails(&map.rails) {
        row(&format!("rails b{bucket}"), &costs);
    }
    for (bucket, costs) in measure_tram(&map.rails) {
        row(&format!("tram b{bucket}"), &costs);
    }
}

/// Строка замера: имя, вершины, миллисекунды и разбивка по слоям.
fn row(label: &str, costs: &[LayerCost]) {
    let (vertices, elapsed, breakdown) = totals(costs);
    println!("{label:>18} {vertices:>7} verts {elapsed:>7.1} ms   [{breakdown}]");
}

/// Вершины, миллисекунды и разбивка по слоям одного замера — общие для обоих
/// замеров, чтобы слой машин печатался тем же форматом, что зданиевые.
fn totals(costs: &[LayerCost]) -> (usize, f64, String) {
    let vertices: usize = costs.iter().map(|cost| cost.vertices).sum();
    let elapsed: f64 = costs
        .iter()
        .map(|cost| cost.elapsed.as_secs_f64() * 1000.0)
        .sum();
    // Шаг без вершин печатается миллисекундами, слой без своего времени —
    // вершинами: у сборки, поднятой на шов, время одно на все её слои и стоит
    // строкой `build`, а «0ms» на каждом слое было бы шумом. Пустой слой —
    // такой же слой (`rail_steel` на дальней ступени), и печатается вершинами.
    let breakdown: Vec<String> = costs
        .iter()
        .map(|cost| {
            let ms = cost.elapsed.as_secs_f64() * 1000.0;
            let name = cost.name;
            let thousands = cost.vertices / 1000;
            match (cost.vertices, cost.elapsed.is_zero()) {
                (_, true) => format!("{name} {thousands}k"),
                (0, false) => format!("{name} {ms:.0}ms"),
                (_, false) => format!("{name} {ms:.0}ms/{thousands}k"),
            }
        })
        .collect();
    (vertices, elapsed, breakdown.join(", "))
}
