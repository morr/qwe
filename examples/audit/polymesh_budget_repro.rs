//! Регрессионный репро живых паник «polymesh search diverged».
//!
//! Тульские запросы сняты с падений реальной игры (радиус 0.4, чанки по
//! умолчанию): длинные коридорные маршруты, которые при бюджете
//! «10 извлечений на открытый полигон» исчерпывали его и роняли игру, а при
//! ×2 сходились — то есть бюджет был занижен, геометрия цела (отсюда
//! сегодняшние 40 на полигон, см. `SEARCH_POPS_PER_POLYGON`).
//!
//! Белгородский (радиус 0.2) — другой класс: коридор всего в 3010 полигонов,
//! а поиск ходил по кольцам на швах чанков, и 84% извлечений были точными
//! повторами узлов. Бюджет тут ни при чём — лечится дедупликацией с первого
//! извлечения в вендоренном polyanya (`SearchInstance::new`, `recording`).
//!
//! ```text
//! cargo run --example polymesh_budget_repro
//! ```
//!
//! Ожидание: ни одной паники. Тульский запрос 1 на нынешней карте даёт MISS за
//! десятую долю миллисекунды — туда больше нет пути, и это не регрессия; FOUND
//! обязаны давать тульский 0 и белгородский. Паника здесь означает, что бюджет
//! снова тесен или поиск снова крутит одни и те же узлы. Сначала гляньте,
//! сходится ли запрос при увеличенном `SEARCH_POPS_PER_POLYGON`, и сколько
//! извлечений он тратит на открытый полигон: единицы — бюджет, десятки —
//! повторы (посчитайте точные повторы ключа `is_new` в polyanya). Геометрию
//! вините последней.

#[path = "../common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::Vec2;

use qwe::city::City;
use qwe::navigation::{build_polymesh_from_map, find_path_polymesh};

/// Живые паники: город, радиус агента в момент падения и запросы из сообщений.
const FAILURES: [(City, f32, &[(Vec2, Vec2)]); 2] = [
    (
        City::Tula,
        0.4,
        &[
            (Vec2::new(2962.1887, 123.922745), Vec2::new(2077.0, 2703.0)),
            (Vec2::new(1504.407, 2907.4124), Vec2::new(5273.0, 733.25)),
        ],
    ),
    (
        City::Belgorod,
        0.2,
        &[(Vec2::new(451.0, 2839.0), Vec2::new(1047.0, 5407.0))],
    ),
];

fn main() {
    for (city, radius, queries) in FAILURES {
        let mut map = common::load_map(city);
        let _navmesh = common::build_navmesh(&mut map, city);
        let started = Instant::now();
        let build = build_polymesh_from_map(&map, radius).expect("build was not cancelled");
        println!(
            "{city:?} r={radius}: polymesh built in {:?}",
            started.elapsed()
        );

        for (index, (from, to)) in queries.iter().enumerate() {
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
}
