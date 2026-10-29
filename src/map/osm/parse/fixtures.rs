//! Общая часть сцен тестов разбора: константы карты вокруг гео-центра Тулы,
//! готовые сцены ([`fixture`], [`wood_scene`], [`tree_row`], [`skewed_house`])
//! и мостик из сцены в элементный цикл ([`read`]). Билдеры элементов ответа —
//! `crate::map::osm::fixture`; здесь — то, чем тест разбора собирает сцену
//! и зовёт один проход, без JSON и без остальных шагов конвейера.

use bevy::math::Vec2;

use super::{Pending, ReadReport, read_elements};
use crate::city::City;
use crate::map::osm::fixture::{Overpass, closed, square};
use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{GeoBounds, OverpassResponse};
use crate::settings::MAP_SIZE;

/// Фикстуры строятся вокруг гео-центра Тулы — города по умолчанию.
pub(super) const CITY: City = City::Tula;

/// Центр карты: сцены собираются вокруг него, и в тех же метрах пишутся
/// проверки — фикстуре незачем говорить в градусах.
pub(super) const CENTER: Vec2 = Vec2::new(MAP_SIZE.x / 2.0, MAP_SIZE.y / 2.0);

/// Половина стороны обычной сцены: дом, пруд, отрезок дороги.
pub(super) const HALF: f32 = 55.0;
/// Половина стороны лесной сцены — посадке нужно место.
pub(super) const WOOD_HALF: f32 = 110.0;

/// Углы квадратной сцены вокруг центра карты: юго-западный, юго-восточный,
/// северо-восточный, северо-западный.
pub(super) fn corners(half: f32) -> (Vec2, Vec2, Vec2, Vec2) {
    let (min, max) = (CENTER - Vec2::splat(half), CENTER + Vec2::splat(half));
    (min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y))
}

/// Мини-ответ Overpass: way-здание, дорога-мост, relation-вода из двух
/// половинок с дыркой-островом.
pub(super) fn fixture() -> Overpass {
    let (sw, se, ne, nw) = corners(HALF);
    Overpass::new(CITY)
        .area(&[("building", "yes")], square(CENTER, HALF))
        .way(&[("highway", "secondary"), ("bridge", "yes")], vec![sw, ne])
        .way(&[("highway", "proposed")], vec![sw, ne])
        .relation(
            &[("natural", "water")],
            &[
                ("outer", vec![sw, se, ne]),
                ("outer", vec![ne, nw, sw]),
                ("inner", closed(square(CENTER, HALF / 4.0))),
            ],
        )
}

/// Лес во всю сцену — фон тестов посадки: деревья растут только в лесных
/// полигонах, поэтому каждое правило проверяется как вычитание из него.
pub(super) fn wood_scene() -> Overpass {
    Overpass::new(CITY).area(&[("natural", "wood")], square(CENTER, WOOD_HALF))
}

/// Ряд деревьев поперёк сцены — общее начало тестов `natural=tree_row`.
pub(super) fn tree_row(tags: &[(&str, &str)]) -> MapData {
    let (sw, se, ..) = corners(HALF);
    Overpass::new(CITY).way(tags, vec![sw, se]).parse()
}

/// Ромб частного дома из Тулы (way 968419942, углы 79°–100°) вокруг `at`.
pub(super) fn skewed_house(at: Vec2) -> Vec<Vec2> {
    [
        Vec2::new(412.56, 3158.32),
        Vec2::new(426.85, 3167.85),
        Vec2::new(420.76, 3178.18),
        Vec2::new(407.79, 3169.97),
    ]
    .map(|point| point - Vec2::new(416.99, 3168.58) + at)
    .to_vec()
}

pub(super) fn right_angles(ring: &[Vec2]) -> bool {
    (0..ring.len()).all(|index| {
        let (prev, at, next) = (
            ring[(index + ring.len() - 1) % ring.len()],
            ring[index],
            ring[(index + 1) % ring.len()],
        );
        (prev - at).normalize().dot((next - at).normalize()).abs() < 0.01
    })
}

/// Сырая карта из сцены — ровно то, что отдаёт элементный цикл, без доводки.
pub(super) fn read(scene: &Overpass) -> (MapData, Pending, ReadReport) {
    let response: OverpassResponse =
        serde_json::from_str(&scene.json()).expect("фикстура строит валидный JSON");
    read_elements(&response, &GeoBounds::for_city(CITY))
}
