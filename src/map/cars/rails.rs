//! **Машины не стоят на железной дороге.** Ряд у бордюра, двор и стоянка
//! ничего не знают о путях: улица, которую пересекает станционный путь,
//! ставила машины прямо на рельсы и вплотную к балласту (Тула, 2605, 3110).
//! Поэтому у расстановки один общий последний шаг — [`RailKeepout`]: машина,
//! кузов которой ближе [`RAIL_CAR_GAP`] к плечу балласта любого пути, не
//! ставится. Одно правило на все три расстановки, а не три копии в каждой.
//!
//! Трамвай в зону не входит: он идёт по проезжей части, и зона вокруг него
//! сняла бы ряды со всех трамвайных улиц.
//!
//! Путь на мосту (`RailLine::bridge`) зону держит, хотя он в воздухе: машина
//! (`Z_CAR`) рисуется поверх плиты путепровода (`Z_RAIL_BRIDGE`), и ряд под
//! ним снимается так же, как под мостом улицы.

use bevy::math::Vec2;

use super::body::Car;
use crate::map::grid::Grid;
use crate::map::osm::model::closest_on_segment;
use crate::map::osm::{RailKind, RailLine};
use crate::map::rail::deck_width;

/// Зазор между кузовом и кромкой балластного плеча, м. С плечом
/// магистрального пути (5 м × 1.22 / 2 ≈ 3 м) кузов стоит не ближе 5.5 м от
/// оси — у переезда машина встаёт перед ним, а не на нём.
pub const RAIL_CAR_GAP: f32 = 2.5;
/// Полудлина и полуширина кузова с запасом на самый крупный силуэт
/// (`body::CarShape`: 5.3 × 1.95 м), м.
const CAR_HALF_LENGTH: f32 = 2.7;
const CAR_HALF_WIDTH: f32 = 1.0;
/// Ячейка сетки звеньев путей, м.
const CELL: f32 = 32.0;

/// Зона запрета вокруг путей: звенья с полушириной плеча.
pub struct RailKeepout {
    links: Grid<(Vec2, Vec2, f32)>,
}

impl RailKeepout {
    pub fn new(rails: &[RailLine]) -> Self {
        let mut links = Grid::new(CELL);
        for rail in rails.iter().filter(|rail| rail.kind != RailKind::Tram) {
            let reach = deck_width(rail) / 2.0 + RAIL_CAR_GAP;
            for pair in rail.points.windows(2) {
                links.insert_segment(
                    pair[0],
                    pair[1],
                    reach + CAR_HALF_LENGTH,
                    (pair[0], pair[1], reach),
                );
            }
        }
        Self { links }
    }

    /// Заходит ли кузов машины в зону какого-нибудь пути: расстояние от
    /// центра до звена меньше зоны плюс вынос кузова в сторону звена.
    pub fn blocks(&self, car: &Car) -> bool {
        let across = car.along.perp();
        self.links.at(car.at).iter().any(|&(from, to, reach)| {
            let nearest = closest_on_segment(car.at, from, to);
            let Some(toward) = (nearest - car.at).try_normalize() else {
                return true;
            };
            let extent = CAR_HALF_LENGTH * car.along.dot(toward).abs()
                + CAR_HALF_WIDTH * across.dot(toward).abs();
            car.at.distance(nearest) < reach + extent
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::cars::body::CarShape;
    use crate::map::osm::fixture;
    use bevy::color::Color;

    fn car(at: Vec2, along: Vec2) -> Car {
        Car {
            at,
            along,
            color: Color::WHITE,
            shape: CarShape::from_share(0.0),
        }
    }

    /// Путь по оси y через начало координат.
    fn keepout() -> RailKeepout {
        RailKeepout::new(&[fixture::rail(
            vec![Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)],
            5.0,
        )])
    }

    #[test]
    fn a_car_on_the_track_is_blocked() {
        assert!(keepout().blocks(&car(Vec2::ZERO, Vec2::X)));
    }

    #[test]
    fn a_car_crossing_the_track_is_blocked_further_out_than_one_alongside() {
        let keepout = keepout();
        // поперёк пути: кузов выносит к нему полудлину
        assert!(keepout.blocks(&car(Vec2::new(7.5, 0.0), Vec2::X)));
        // вдоль пути на том же расстоянии — только полуширину
        assert!(!keepout.blocks(&car(Vec2::new(7.5, 0.0), Vec2::Y)));
        assert!(!keepout.blocks(&car(Vec2::new(9.0, 0.0), Vec2::X)));
    }

    #[test]
    fn a_tram_track_keeps_no_zone() {
        let mut tram = fixture::rail(vec![Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 1.2);
        tram.kind = RailKind::Tram;
        assert!(!RailKeepout::new(&[tram]).blocks(&car(Vec2::ZERO, Vec2::X)));
    }

    #[test]
    fn a_track_on_a_bridge_keeps_its_zone() {
        let mut bridge = fixture::rail(vec![Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 5.0);
        bridge.bridge = true;
        assert!(RailKeepout::new(&[bridge]).blocks(&car(Vec2::ZERO, Vec2::X)));
    }
}
