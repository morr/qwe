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
//! Паркуются вдоль **всякой** проезжей части, а не только вдоль магистралей:
//! отбор идёт тем же [`is_carriageway`], которым `map::roads` решает, где
//! рисовать тротуар и разметку. Ширина в `RoadLine` — рисовальная константа
//! класса, а не измеренная ширина улицы, так что порог по ней читается не как
//! «узкая улица», а как «не магистраль»; на снимке города плотнее всего
//! запаркованы как раз жилые кварталы.
//!
//! Расстановка детерминирована (ГПСЧ Лемера, засеянный первой точкой улицы),
//! так что от запуска к запуску ряд стоит одинаково. Виден он только вблизи:
//! [`CarZoomBucket`] снимает слой целиком, когда машина становится мельче
//! пары пикселей.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RoadLine};
use crate::map::roads::is_carriageway;
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SHADOW_COLOR, SHADOW_DIR, shadow_length_scale};
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
/// Какая доля мест занята. Сплошной ряд от перекрёстка до перекрёстка
/// выглядит как автосалон; у настоящей улицы ряд рваный.
const OCCUPANCY: f32 = 0.45;
/// Отступ от торца нарисованной ленты, м: машина, поставленная вплотную к
/// концу улицы, свисала бы с него. Про перекрёстки этот отступ ничего не
/// знает — way кончается где угодно, а перекрёсток восстанавливается по
/// общим нодам (`map::roads::junctions`).
const END_MARGIN: f32 = 2.0;

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
        let mut rng = Lcg::new(seed_from_point(
            road.points.first().copied().unwrap_or(Vec2::ZERO),
        ));
        // ряд с каждой стороны: отступ от кромки внутрь проезжей части
        let offset = road.width / 2.0 - CURB_GAP - CAR_WIDTH / 2.0;
        for side in [-1.0, 1.0] {
            park_along(&mut cars, road, side * offset, &mut rng);
        }
    }
    cars
}

/// Улица, вдоль которой паркуются: настоящая проезжая часть — то же
/// [`is_carriageway`], которым отбирает тротуары и разметку `map::roads`, —
/// но не мост (на мосту не стоят) и не кольцо (по кольцу едут, а не
/// паркуются). Разметке мост и кольцо нужны, машинам нет, поэтому оба
/// условия здесь, а не внутри предиката.
fn parkable(road: &RoadLine) -> bool {
    is_carriageway(road) && !road.bridge && !road.roundabout
}

/// Ряд вдоль одной стороны: шагом [`CAR_PITCH`] по **всей** ломаной улицы, со
/// сдвигом `offset` поперёк и с пропусками.
///
/// Шаг идёт по дуговой координате целой улицы, а не по каждому её звену
/// порознь: звено ломаной в городе сплошь и рядом короче двух отступов, и
/// пошаговый обход `windows(2)` выбрасывал их целиком (в кеше Тулы — половину
/// сегментов и треть длины), а на каждой вершине сбрасывал шаг, отчего ряд то
/// рвался, то удваивался.
fn park_along(cars: &mut Vec<Car>, road: &RoadLine, offset: f32, rng: &mut Lcg) {
    let (along, total) = arclengths(&road.points);
    if total <= 2.0 * END_MARGIN {
        return;
    }
    // последняя **поставленная** машина этой стороны: на изломе внутренний
    // ряд сжимается, и место, наехавшее на соседа, пропускается. Проверка по
    // мировому расстоянию, а не по дуговой координате, — она ловит и излом,
    // и любую другую кривизну
    let mut last: Option<Vec2> = None;
    let mut step = END_MARGIN;
    while step <= total - END_MARGIN {
        let at = step;
        step += CAR_PITCH;
        let Some((point, direction)) = place_on_path(&road.points, &along, at) else {
            continue;
        };
        let place = point + Vec2::new(-direction.y, direction.x) * offset;
        if last.is_some_and(|previous| previous.distance(place) < CAR_LENGTH) {
            continue;
        }
        if rng.next_f32() >= OCCUPANCY {
            continue;
        }
        last = Some(place);
        cars.push(Car {
            at: place,
            along: direction,
            color: CAR_COLORS
                [(rng.next_f32() * CAR_COLORS.len() as f32) as usize % CAR_COLORS.len()],
        });
    }
}

/// Накопленные длины по точкам ломаной и её полная длина — та же форма, что
/// у лент в `map::meshing`, но своя: обобщать ради одного вызова нечего.
fn arclengths(points: &[Vec2]) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (index, &point) in points.iter().enumerate() {
        if index > 0 {
            total += point.distance(points[index - 1]);
        }
        along.push(total);
    }
    (along, total)
}

/// Точка ломаной на дуговой координате `at` и направление звена, на которое
/// она попала: звено ищется бинарным поиском по `along`, позиция внутри него —
/// интерполяцией.
fn place_on_path(points: &[Vec2], along: &[f32], at: f32) -> Option<(Vec2, Vec2)> {
    let last = points.len().checked_sub(2)?;
    let index = match along.binary_search_by(|value| value.total_cmp(&at)) {
        Ok(index) => index.min(last),
        Err(index) => index.saturating_sub(1).min(last),
    };
    let direction = (points[index + 1] - points[index]).try_normalize()?;
    Some((points[index] + direction * (at - along[index]), direction))
}

/// Меш слоя: сначала **все** тени, потом **все** кузова — тень соседней
/// машины иначе легла бы поверх кузова той, что нарисована раньше.
fn mesh_cars(cars: &[Car]) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let shadow = SHADOW_COLOR.to_linear();
    let offset = SHADOW_DIR * (CAR_HEIGHT * shadow_length_scale());
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
    fn a_residential_street_gets_a_row() {
        // 8 м — `residential`/`unclassified`: основная масса улиц города
        let residential = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 8.0);
        let cars = park_cars(std::slice::from_ref(&residential));
        assert!(!cars.is_empty());
        // и ряды на ней не наезжают на осевую: между ними остаётся проезд
        for car in &cars {
            assert!(
                car.at.y.abs() - CAR_WIDTH / 2.0 > 0.5,
                "ряд на осевой: {}",
                car.at.y
            );
        }

        // 5 м — `service`, проезд: там не паркуются
        let service = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
        assert!(park_cars(std::slice::from_ref(&service)).is_empty());
    }

    #[test]
    fn the_row_crosses_the_bends_of_a_polyline() {
        // десять звеньев по 10 м: каждое короче прежних двух отступов, и
        // посегментный обход не ставил на них ни одной машины
        let points = (0..=10).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let with_vertices = park_cars(std::slice::from_ref(&street(points, 12.0)));
        let straight = street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 12.0);
        let one_span = park_cars(std::slice::from_ref(&straight));

        // вершины на прямой не меняют ничего: шаг идёт по длине улицы
        assert!(!with_vertices.is_empty());
        assert_eq!(with_vertices.len(), one_span.len());
        for (car, twin) in with_vertices.iter().zip(&one_span) {
            assert!(car.at.distance(twin.at) < 0.01, "{} / {}", car.at, twin.at);
        }
    }

    #[test]
    fn cars_never_overlap_on_a_sharp_bend() {
        // излом в 90°: с внутренней стороны ряд сжимается, и место, наехавшее
        // на соседа, обязано быть пропущено
        let road = street(
            vec![
                Vec2::new(0.0, 100.0),
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
            ],
            12.0,
        );
        let cars = park_cars(std::slice::from_ref(&road));
        assert!(cars.len() > 10, "{}", cars.len());
        for (index, car) in cars.iter().enumerate() {
            for other in &cars[index + 1..] {
                let gap = car.at.distance(other.at);
                assert!(
                    gap >= CAR_LENGTH - 0.01,
                    "{gap} м между {} и {}",
                    car.at,
                    other.at
                );
            }
        }
    }

    #[test]
    fn a_street_shorter_than_its_end_margins_stays_empty() {
        let stub = street(vec![Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0)], 12.0);
        assert!(park_cars(std::slice::from_ref(&stub)).is_empty());
    }

    #[test]
    fn a_roundabout_stays_empty() {
        let mut ring = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        ring.roundabout = true;
        assert!(park_cars(std::slice::from_ref(&ring)).is_empty());
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
