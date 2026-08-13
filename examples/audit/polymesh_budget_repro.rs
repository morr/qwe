//! Регрессионный репро двух живых паник «polymesh search diverged».
//!
//! Оба запроса сняты с падений реальной игры (Тула, радиус 0.4, чанки по
//! умолчанию): длинные коридорные маршруты, которые при бюджете
//! «10 извлечений на открытый полигон» исчерпывали его и роняли игру, а при
//! ×2 сходились — то есть бюджет был занижен, геометрия цела (отсюда
//! сегодняшние 40 на полигон, см. `SEARCH_POPS_PER_POLYGON`).
//!
//! ```text
//! cargo run --example polymesh_budget_repro
//! ```
//!
//! Ожидание: оба запроса FOUND. Паника здесь означает, что бюджет снова тесен
//! (или дивергенция настоящая) — сначала гляньте, сходится ли запрос при
//! увеличенном `SEARCH_POPS_PER_POLYGON`, и только потом вините геометрию.

#[path = "../common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::Vec2;

use qwe::city::City;
use qwe::navigation::{build_polymesh_from_map, find_path_polymesh};

const CITY: City = City::Tula;
/// Радиус агента, на котором жила игра в момент паник.
const RADIUS: f32 = 0.4;

/// Запросы из двух живых паник (координаты из сообщений).
const FAILURES: [(Vec2, Vec2); 2] = [
    (Vec2::new(2962.1887, 123.922745), Vec2::new(2077.0, 2703.0)),
    (Vec2::new(1504.407, 2907.4124), Vec2::new(5273.0, 733.25)),
];

fn main() {
    let map = common::load_map(CITY);
    let _navmesh = common::build_navmesh(&map, CITY);
    let started = Instant::now();
    let build = build_polymesh_from_map(&map, RADIUS).expect("build was not cancelled");
    println!("polymesh built in {:?}", started.elapsed());

    for (index, (from, to)) in FAILURES.iter().enumerate() {
        let started = Instant::now();
        let path = find_path_polymesh(&build, *from, *to);
        let elapsed = started.elapsed().as_secs_f32() * 1000.0;
        println!(
            "query {index} ({:.0} m): {} in {elapsed:>8.2} ms, waypoints {}",
            from.distance(*to),
            if path.is_some() { "FOUND" } else { "MISS" },
            path.as_ref().map(Vec::len).unwrap_or(0),
        );
    }
}
