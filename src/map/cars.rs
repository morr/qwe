//! Машины, стоящие вдоль улиц.
//!
//! На снимке города с воздуха машины — второй по узнаваемости признак после
//! самих крыш: улица без единого автомобиля читается как чертёж, чем бы её ни
//! красили. Их и не хватало карте больше всего после того, как дома получили
//! материал кровли, оборудование на ней и настоящие тени.
//!
//! Всё, что здесь есть, — **декорация**: машины не попадают ни в навмеш, ни в
//! симуляцию, пешки ходят сквозь них. Это сознательно: припаркованный ряд
//! вдоль каждой улицы съел бы тротуары, по которым идёт вся толпа.
//!
//! Расстановка детерминирована (ГПСЧ Лемера, засеянный первой точкой улицы),
//! так что от запуска к запуску ряд стоит одинаково. Виден он только вблизи:
//! [`CarZoomBucket`] снимает слой целиком, когда машина становится мельче
//! пары пикселей.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RoadClass, RoadLine};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};
use crate::settings::{CAR_MAX_ZOOM, Z_CAR};

/// Габарит легковой машины, м — «Логан» с точностью до сантиметров.
const CAR_LENGTH: f32 = 4.4;
const CAR_WIDTH: f32 = 1.8;
/// Высота машины, м: по ней считается длина её тени, тем же котангенсом
/// высоты солнца, что у домов.
const CAR_HEIGHT: f32 = 1.5;
/// Шаг парковочного места вдоль улицы, м: машина плюс просвет.
const CAR_PITCH: f32 = 6.0;
/// Насколько край машины отступает от кромки проезжей части, м. Ноль —
/// колесо на кромке; полметра оставляют полосу асфальта между рядом и
/// разметкой, как на настоящей улице.
const CURB_GAP: f32 = 0.5;
/// Улица у́же этого ряда не держит: на шести метрах две встречные машины уже
/// не разъедутся, и никто там не паркуется.
const PARKED_MIN_WIDTH: f32 = 9.0;
/// Какая доля мест занята. Сплошной ряд от перекрёстка до перекрёстка
/// выглядит как автосалон; у настоящей улицы ряд рваный.
const OCCUPANCY: f32 = 0.45;
/// Ближе этого к торцу улицы не паркуются — там перекрёсток.
const END_MARGIN: f32 = 8.0;

/// Палитра кузовов, по долям близкая к тому, что видно на снимке русского
/// города: белый, серебро и серый — половина ряда, чёрный — четверть,
/// остальное цветное.
const CAR_COLORS: [Color; 10] = [
    Color::srgb(0.78, 0.78, 0.77),
    Color::srgb(0.72, 0.73, 0.74),
    Color::srgb(0.60, 0.61, 0.62),
    Color::srgb(0.46, 0.47, 0.48),
    Color::srgb(0.16, 0.16, 0.17),
    Color::srgb(0.20, 0.20, 0.21),
    Color::srgb(0.22, 0.27, 0.38),
    Color::srgb(0.45, 0.14, 0.13),
    Color::srgb(0.29, 0.33, 0.28),
    Color::srgb(0.55, 0.50, 0.42),
];

/// Слой машин — чтобы пересборка по зуму знала, что деспавнить.
#[derive(Component)]
pub struct CarLayerTag;

/// Ступени зума слоя машин: ближе порога — ряды на месте, дальше слоя нет
/// вовсе. Машина в 4.4 м на 0.8 м/px это пять пикселей; ещё дальше ряд
/// превращается в мерцающий пунктир вдоль улицы.
pub enum CarLods {}

impl ZoomLods for CarLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [CAR_MAX_ZOOM, f32::INFINITY].into_iter()
    }
}

pub type CarZoomBucket = ZoomBucket<CarLods>;

/// Одна машина: центр, направление вдоль кузова и цвет.
struct Car {
    at: Vec2,
    along: Vec2,
    color: Color,
}

/// ГПСЧ Лемера (Park–Miller) — тот же, что расставляет кроны и оборудование
/// на кровле.
struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self((seed % 0x7FFF_FFFF).max(1))
    }

    fn next_f32(&mut self) -> f32 {
        self.0 = ((u64::from(self.0) * 48271) % 0x7FFF_FFFF) as u32;
        self.0 as f32 / 2_147_483_647.0
    }
}

/// Замер слоя машин без мира — для офлайн-бенча, по той же причине, что и
/// `buildings::measure_layers`.
pub fn measure_cars(roads: &[RoadLine]) -> (usize, usize) {
    let cars = park_cars(roads);
    let builder = mesh_cars(&cars);
    (cars.len(), builder.vertex_count())
}

/// Пересборка слоя машин: на входе в мир и на пересечении порога зума.
pub fn rebuild_cars(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<CarZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<CarLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if bucket.index > 0 {
        return;
    }
    let cars = park_cars(&map.roads);
    let builder = mesh_cars(&cars);
    let count = cars.len();
    let vertices = builder.vertex_count();
    if builder.is_empty() {
        return;
    }
    // слой с блендингом: тень машины полупрозрачна, кузов — нет
    let material = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    surface::spawn_layer(
        &mut commands,
        &mut meshes,
        builder,
        Z_CAR,
        "cars",
        LayerMaterial::Flat(material),
        CarLayerTag,
    );
    info!("cars: {count} parked ({vertices} verts)");
}

/// Ряды вдоль всех улиц, годных под парковку.
fn park_cars(roads: &[RoadLine]) -> Vec<Car> {
    let mut cars = Vec::new();
    for road in roads {
        if !parkable(road) {
            continue;
        }
        let mut rng = Lcg::new(road_seed(road));
        // ряд с каждой стороны: отступ от кромки внутрь проезжей части
        let offset = road.width / 2.0 - CURB_GAP - CAR_WIDTH / 2.0;
        for side in [-1.0, 1.0] {
            park_along(&mut cars, road, side * offset, &mut rng);
        }
    }
    cars
}

/// Улица, вдоль которой паркуются: проезжая часть, не арка, не мост (на
/// мосту не стоят) и достаточно широкая.
fn parkable(road: &RoadLine) -> bool {
    road.class == RoadClass::Street
        && !road.passage
        && !road.bridge
        && road.width >= PARKED_MIN_WIDTH
}

/// Ряд вдоль одной стороны: шагом [`CAR_PITCH`] по осевой, со сдвигом
/// `offset` поперёк и с пропусками.
fn park_along(cars: &mut Vec<Car>, road: &RoadLine, offset: f32, rng: &mut Lcg) {
    for pair in road.points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let across = Vec2::new(-along.y, along.x);
        let length = from.distance(to);
        if length <= 2.0 * END_MARGIN {
            continue;
        }
        let mut at = END_MARGIN;
        while at <= length - END_MARGIN {
            if rng.next_f32() < OCCUPANCY {
                cars.push(Car {
                    at: from + along * at + across * offset,
                    along,
                    color: CAR_COLORS
                        [(rng.next_f32() * CAR_COLORS.len() as f32) as usize % CAR_COLORS.len()],
                });
            }
            at += CAR_PITCH;
        }
    }
}

/// Меш слоя: сначала **все** тени, потом **все** кузова — тень соседней
/// машины иначе легла бы поверх кузова той, что нарисована раньше.
fn mesh_cars(cars: &[Car]) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let shadow = SHADOW_COLOR.to_linear();
    let offset = shadow_dir() * (CAR_HEIGHT * shadow_length_scale());
    for car in cars {
        builder.push_quad(body(car, offset), shadow);
    }
    for car in cars {
        builder.push_quad(body(car, Vec2::ZERO), car.color.to_linear());
    }
    builder
}

/// Прямоугольник кузова, сдвинутый на `offset` (для тени — по свету).
fn body(car: &Car, offset: Vec2) -> [Vec2; 4] {
    let half_length = car.along * (CAR_LENGTH / 2.0);
    let half_width = Vec2::new(-car.along.y, car.along.x) * (CAR_WIDTH / 2.0);
    let at = car.at + offset;
    [
        at - half_length - half_width,
        at + half_length - half_width,
        at + half_length + half_width,
        at - half_length + half_width,
    ]
}

/// Посев улицы — от её первой точки, как у дверей и кровель: ряд не зависит
/// ни от порядка улиц в выгрузке, ни от пересборки слоя.
fn road_seed(road: &RoadLine) -> u32 {
    let point = road.points.first().copied().unwrap_or(Vec2::ZERO);
    let x = (point.x * 100.0) as i32 as u32;
    let y = (point.y * 100.0) as i32 as u32;
    let mut hash = x ^ y.rotate_left(16);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^= hash >> 16;
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    fn street(points: Vec<Vec2>, width: f32) -> RoadLine {
        fixture::street(points, width)
    }

    #[test]
    fn cars_line_a_street_on_both_sides() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let cars = park_cars(std::slice::from_ref(&road));
        assert!(!cars.is_empty());
        // ряды по обе стороны осевой, внутри проезжей части
        let offset = road.width / 2.0 - CURB_GAP - CAR_WIDTH / 2.0;
        assert!(cars.iter().any(|car| car.at.y > 0.0));
        assert!(cars.iter().any(|car| car.at.y < 0.0));
        for car in &cars {
            assert!((car.at.y.abs() - offset).abs() < 0.01, "{}", car.at.y);
            assert!(car.at.x >= END_MARGIN - 0.01 && car.at.x <= 200.0 - END_MARGIN + 0.01);
        }
    }

    #[test]
    fn a_narrow_lane_and_a_bridge_stay_empty() {
        let narrow = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
        assert!(park_cars(std::slice::from_ref(&narrow)).is_empty());

        let mut bridge = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 14.0);
        bridge.bridge = true;
        assert!(park_cars(std::slice::from_ref(&bridge)).is_empty());
    }

    #[test]
    fn the_row_is_ragged_and_stable() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(400.0, 0.0)], 12.0);
        let cars = park_cars(std::slice::from_ref(&road));
        // мест на 400 м вдвое больше, чем машин: ряд рваный, а не сплошной
        let places = ((400.0 - 2.0 * END_MARGIN) / CAR_PITCH) as usize * 2;
        assert!(cars.len() < places, "{} of {places}", cars.len());
        assert!(cars.len() > places / 5, "{} of {places}", cars.len());
        // и он тот же самый при повторной сборке
        let again = park_cars(std::slice::from_ref(&road));
        assert_eq!(cars.len(), again.len());
        for (car, twin) in cars.iter().zip(&again) {
            assert_eq!(car.at, twin.at);
        }
    }
}
