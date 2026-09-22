//! Слой припаркованных машин: укладка ряда и сборка слоя.
//!
//! Лежат здесь, а не в `mod.rs`, по образцу девяти сестёр по шву
//! (`fences/tests.rs`, `rail/tests.rs`, `wagons/tests.rs`): модуль слоя
//! читается сверху вниз как слой, а не как слой с полутысячей строк проверок
//! в хвосте.
//! Плотность квартала проверяется своими тестами в [`super::district`] — здесь
//! индекс домов всегда пуст, чтобы правило укладки читалось без неё.

use super::*;
use crate::map::osm::fixture::{self, street};
use crate::map::osm::model::{Highway, KerbParking};
use crate::map::roads::junctions::JUNCTION_MARGIN;

/// Форма дорог с осью по точкам OSM: ряд меряется по той ломаной, что в
/// тесте нарисована.
fn straight() -> RoadShape {
    RoadShape {
        curve_tolerance: 0.0,
        ..default()
    }
}

/// Большая стоянка пустее малой, и доля не выходит за свои края.
#[test]
fn a_bigger_lot_is_emptier() {
    assert_eq!(lot_occupancy(5), LOT_OCCUPANCY_SMALL);
    assert!((lot_occupancy(5000) - LOT_OCCUPANCY_LARGE).abs() < 1e-6);
    let shares: Vec<f32> = [20, 50, 100, 200, 400].map(lot_occupancy).to_vec();
    assert!(
        shares.windows(2).all(|pair| pair[1] < pair[0]),
        "{shares:?}"
    );
}

/// Расстановка по срезу целиком — так же, как её зовёт пересборка слоя.
fn park(roads: &[RoadLine]) -> Vec<Car> {
    park_with(roads, CarStyle::default())
}

fn park_with(roads: &[RoadLine], style: CarStyle) -> Vec<Car> {
    park_driving(roads, style, TrafficSide::Right)
}

/// Домов в сценах этих тестов нет: пустой индекс даёт множитель 1, и они
/// проверяют укладку ряда, не смешанную с плотностью квартала — та
/// проверяется своими тестами в [`super::district`].
fn park_driving(roads: &[RoadLine], style: CarStyle, traffic: TrafficSide) -> Vec<Car> {
    park_cars(
        roads,
        &junctions::marking_breaks(roads, is_carriageway, &[]),
        style,
        &drawn_axes(roads, &straight()),
        traffic,
        &Districts::new(&[]),
    )
}

#[test]
fn cars_line_a_street_on_both_sides() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    let cars = park(std::slice::from_ref(&road));
    assert!(!cars.is_empty());
    // ряды по обе стороны осевой, внутри проезжей части: отступ считается
    // по габариту этой машины, плюс небрежность парковки
    assert!(cars.iter().any(|car| car.at.y > 0.0));
    assert!(cars.iter().any(|car| car.at.y < 0.0));
    for car in &cars {
        let offset = road.width / 2.0 - CURB_GAP - car.shape.width() / 2.0;
        assert!(
            (car.at.y.abs() - offset).abs() <= PARK_SLOP + 0.01,
            "{}",
            car.at.y
        );
        assert!(car.at.x >= END_MARGIN - 0.01 && car.at.x <= 200.0 - END_MARGIN + 0.01);
    }
}

/// Таблица зума и лестница подробности — об одном и том же: у каждой
/// ступени `CarLods` своя подробность, и только у последней её нет
/// (слоя там не строят вовсе).
#[test]
fn the_zoom_table_and_the_detail_ladder_agree() {
    let steps = CarLods::max_zooms().count();
    for bucket in 0..steps - 1 {
        assert!(
            detail_for(bucket).is_some(),
            "ступень {bucket} без подробности"
        );
    }
    assert!(detail_for(steps - 1).is_none(), "последняя ступень рисует");
}

#[test]
fn a_narrow_lane_and_a_bridge_stay_empty() {
    let narrow = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
    assert!(park(std::slice::from_ref(&narrow)).is_empty());

    let mut bridge = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 14.0);
    bridge.bridge = true;
    assert!(park(std::slice::from_ref(&bridge)).is_empty());
}

#[test]
fn a_street_crossing_a_bridge_clears_the_row_under_the_deck() {
    // улица проходит под мостом: общей ноды нет, перекрёстка тоже
    let through = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    let mut bridge = street(vec![Vec2::new(100.0, -80.0), Vec2::new(100.0, 80.0)], 16.0);
    bridge.bridge = true;
    let cars = park(&[through.clone(), bridge.clone()]);

    let cleared = bridge.curb_reach() + JUNCTION_CLEARANCE;
    for car in &cars {
        assert!(
            (car.at.x - 100.0).abs() >= cleared - 0.01,
            "машина на мосту: {}",
            car.at
        );
    }
    assert!(cars.iter().any(|car| car.at.x > 130.0));
    assert!(cars.iter().any(|car| car.at.x < 70.0));

    // первый ряд до моста стоит ровно как без него (после пропуска поток
    // ГПСЧ уже другой — так же, как за перекрёстком, и второй ряд идёт
    // по тому же потоку следом)
    let alone = park(std::slice::from_ref(&through));
    let far = |car: &&Car| car.at.x <= 100.0 - cleared - 10.0 && car.at.y < 0.0;
    let with: Vec<_> = cars.iter().filter(far).map(|car| car.at).collect();
    let without: Vec<_> = alone.iter().filter(far).map(|car| car.at).collect();
    assert_eq!(with, without);
}

#[test]
fn a_residential_street_gets_a_row() {
    // 8 м — `residential`/`unclassified`: основная масса улиц города
    let residential = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 8.0);
    let cars = park(std::slice::from_ref(&residential));
    assert!(!cars.is_empty());
    // и ряды на ней не наезжают на осевую: между ними остаётся проезд
    for car in &cars {
        assert!(
            car.at.y.abs() - car.shape.width() / 2.0 > 0.5,
            "ряд на осевой: {}",
            car.at.y
        );
    }

    // 5 м — `service`, проезд: там не паркуются
    let service = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
    assert!(park(std::slice::from_ref(&service)).is_empty());
}

/// Магистраль без тега стоянки паркуется в карманах: ряд за кромкой
/// проезжей части, на ширину кармана дальше от оси. `parking:*=no` снимает
/// ряд совсем, `lane` возвращает его к бордюру.
#[test]
fn a_primary_parks_in_its_pockets() {
    let primary = RoadLine {
        highway: Highway::Primary,
        ..street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 14.0)
    };
    let style = CarStyle {
        occupancy: 1.0,
        ..default()
    };
    let cars = park_with(std::slice::from_ref(&primary), style);
    assert!(!cars.is_empty());
    for car in &cars {
        assert!(
            car.at.y.abs() > 7.0,
            "машина за кромкой, в кармане: {}",
            car.at.y
        );
    }
    let banned = RoadLine {
        parking: [KerbParking::No; 2],
        ..primary.clone()
    };
    assert!(park_with(std::slice::from_ref(&banned), style).is_empty());
    let lane = RoadLine {
        parking: [KerbParking::Lane; 2],
        ..primary
    };
    let cars = park_with(std::slice::from_ref(&lane), style);
    assert!(!cars.is_empty() && cars.iter().all(|car| car.at.y.abs() < 7.0));
}

/// Та же улица в частном секторе запаркована много реже, чем в
/// микрорайоне, и ползунок занятости остаётся за обоими: он задаёт базу, а
/// квартал — множитель к ней.
#[test]
fn the_same_street_parks_thinner_in_a_private_sector() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(600.0, 0.0)], 8.0);
    let roads = std::slice::from_ref(&road);
    let breaks = junctions::marking_breaks(roads, is_carriageway, &[]);
    let rows = |buildings: &[PolyArea]| {
        park_cars(
            roads,
            &breaks,
            CarStyle::default(),
            &drawn_axes(roads, &straight()),
            TrafficSide::Right,
            &Districts::new(buildings),
        )
        .len()
    };
    // частный сектор: одноэтажные дома по обе стороны улицы
    let houses: Vec<PolyArea> = (0..30)
        .map(|i| PolyArea {
            height: Some(3.2),
            ..fixture::building(
                fixture::square(
                    Vec2::new(i as f32 % 15.0 * 40.0, i as f32 % 2.0 * 60.0 - 30.0),
                    5.0,
                ),
                Vec::new(),
            )
        })
        .collect();
    // микрорайон: те же места, но девятиэтажные секции
    let slabs: Vec<PolyArea> = (0..8)
        .map(|i| PolyArea {
            height: Some(27.0),
            ..fixture::building(
                fixture::square(
                    Vec2::new(i as f32 % 4.0 * 150.0, i as f32 % 2.0 * 60.0 - 30.0),
                    15.0,
                ),
                Vec::new(),
            )
        })
        .collect();
    let (low, high) = (rows(&houses), rows(&slabs));
    assert!(low * 3 < high, "частный сектор {low}, микрорайон {high}");
    // и без домов вокруг остаётся ровно прежнее правило
    assert!(rows(&[]) > low && rows(&[]) < high);
}

#[test]
fn the_row_crosses_the_bends_of_a_polyline() {
    // десять звеньев по 10 м: каждое короче прежних двух отступов, и
    // посегментный обход не ставил на них ни одной машины
    let points = (0..=10).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
    let with_vertices = park(std::slice::from_ref(&street(points, 12.0)));
    let straight = street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 12.0);
    let one_span = park(std::slice::from_ref(&straight));

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
    let cars = park(std::slice::from_ref(&road));
    assert!(cars.len() > 10, "{}", cars.len());
    // просвет — по **полусумме** длин пары: именно на ней два кузова,
    // стоящие носом к корме, и перестают перекрываться. Небрежность
    // парковки (`PARK_SLOP`, поперёк ряда) разыгрывается после проверки и
    // может сдвинуть навстречу обе машины, отсюда допуск в два слопа
    for (index, car) in cars.iter().enumerate() {
        for other in &cars[index + 1..] {
            let gap = car.at.distance(other.at);
            let apart = (car.shape.length() + other.shape.length()) / 2.0 - 2.0 * PARK_SLOP;
            assert!(
                gap >= apart - 0.01,
                "{gap} м между {} ({:?}) и {} ({:?})",
                car.at,
                car.shape,
                other.at,
                other.shape
            );
        }
    }
}

#[test]
fn a_junction_clears_the_row_and_the_row_resumes() {
    // Т-образный: сквозная улица с нодой в узле (перекрёсток и держится
    // на общей ноде) и примыкающая к ней в середине
    let through = street(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(200.0, 0.0),
        ],
        12.0,
    );
    let joining = street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 10.0);
    let cars = park(&[through, joining]);

    // полуширина самой широкой из сошедшихся плюс запас разметки и клиренс
    let node = Vec2::new(100.0, 0.0);
    let cleared = 10.0 / 2.0 + JUNCTION_MARGIN + JUNCTION_CLEARANCE;
    for car in &cars {
        assert!(
            car.at.distance(node) >= cleared - 0.01,
            "машина на перекрёстке: {}",
            car.at
        );
    }
    // и ряд идёт дальше по обе стороны от узла
    assert!(
        cars.iter()
            .any(|car| car.at.x > 130.0 && car.at.y.abs() < 6.0)
    );
    assert!(
        cars.iter()
            .any(|car| car.at.x < 70.0 && car.at.y.abs() < 6.0)
    );
}

#[test]
fn a_way_split_in_the_middle_keeps_the_row() {
    // одна прямая улица, разрезанная на два way в общей ноде: стык двух
    // торцов — не перекрёсток, и ряд идёт сквозь него
    let first = street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 12.0);
    let second = street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    let cars = park(&[first, second]);
    assert!(
        cars.iter().any(|car| (92.0..=108.0).contains(&car.at.x)),
        "ряд разорван на стыке двух way одной улицы"
    );
}

#[test]
fn a_street_shorter_than_its_end_margins_stays_empty() {
    let stub = street(vec![Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0)], 12.0);
    assert!(park(std::slice::from_ref(&stub)).is_empty());
}

#[test]
fn a_one_way_carriageway_parks_on_its_right() {
    // улица на восток: справа по ходу — юг
    let points = vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)];
    let mut oneway = street(points.clone(), 12.0);
    oneway.oneway = true;
    let cars = park(std::slice::from_ref(&oneway));
    assert!(!cars.is_empty());
    for car in &cars {
        assert!(car.at.y < 0.0, "ряд не с той стороны: {}", car.at);
    }

    // та же улица без `oneway` — с обеих сторон
    let both = park(std::slice::from_ref(&street(points, 12.0)));
    assert!(both.iter().any(|car| car.at.y > 0.0));
    assert!(both.iter().any(|car| car.at.y < 0.0));
}

/// Нос машины — по потоку её полосы: при правостороннем движении южный
/// ряд улицы на восток смотрит на восток, северный — на запад, при
/// левостороннем наоборот. Перекос парковки — градусы, так что знак
/// проекции на ось улицы он не меняет.
#[test]
fn cars_face_the_traffic_of_their_own_kerb() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    for (traffic, south_heading) in [(TrafficSide::Right, 1.0), (TrafficSide::Left, -1.0)] {
        let cars = park_driving(std::slice::from_ref(&road), CarStyle::default(), traffic);
        assert!(cars.iter().any(|car| car.at.y > 0.0));
        assert!(cars.iter().any(|car| car.at.y < 0.0));
        for car in &cars {
            let expected = if car.at.y < 0.0 {
                south_heading
            } else {
                -south_heading
            };
            assert!(
                car.along.x * expected > 0.9,
                "{traffic:?}: машина в {} смотрит {}",
                car.at,
                car.along
            );
        }
    }
}

/// Сторона движения переворачивает машины, но не переставляет их: поток
/// ГПСЧ от неё не зависит, и двусторонняя улица Лондона стоит теми же
/// местами, что и такая же в Туле.
#[test]
fn the_traffic_side_turns_the_row_without_moving_it() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    let right = park_driving(
        std::slice::from_ref(&road),
        CarStyle::default(),
        TrafficSide::Right,
    );
    let left = park_driving(
        std::slice::from_ref(&road),
        CarStyle::default(),
        TrafficSide::Left,
    );
    assert_eq!(right.len(), left.len());
    for (a, b) in right.iter().zip(&left) {
        assert_eq!(a.at, b.at);
        assert!(
            (a.along + b.along).length() < 1e-5,
            "{} {}",
            a.along,
            b.along
        );
    }
}

#[test]
fn a_left_hand_one_way_carriageway_parks_on_its_left() {
    // улица на восток: слева по ходу — север, и носы по ходу
    let mut oneway = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    oneway.oneway = true;
    let cars = park_driving(
        std::slice::from_ref(&oneway),
        CarStyle::default(),
        TrafficSide::Left,
    );
    assert!(!cars.is_empty());
    for car in &cars {
        assert!(car.at.y > 0.0, "ряд не с той стороны: {}", car.at);
        assert!(
            car.along.x > 0.9,
            "машина смотрит против потока: {}",
            car.along
        );
    }
}

#[test]
fn occupancy_zero_parks_nothing() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    let empty = CarStyle {
        occupancy: 0.0,
        ..default()
    };
    assert!(park_with(std::slice::from_ref(&road), empty).is_empty());
}

#[test]
fn a_roundabout_stays_empty() {
    let mut ring = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
    ring.roundabout = true;
    assert!(park(std::slice::from_ref(&ring)).is_empty());
}

/// И кольцо **без тега** — тоже: замкнутое одностороннее полотно узнаётся по
/// форме ([`RoadLine::is_roundabout`]). Большое кольцо у ТРЦ «Макси» в OSM
/// просто `oneway=yes`, и по кругу вдоль него стоял ряд припаркованных машин.
#[test]
fn a_closed_oneway_way_stays_empty_without_the_tag() {
    let corner = Vec2::new(0.0, 0.0);
    let mut ring = street(
        vec![
            corner,
            Vec2::new(60.0, 0.0),
            Vec2::new(60.0, 60.0),
            Vec2::new(0.0, 60.0),
            corner,
        ],
        12.0,
    );
    ring.oneway = true;
    assert!(park(std::slice::from_ref(&ring)).is_empty());
}

#[test]
fn the_row_is_ragged_and_stable() {
    let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(400.0, 0.0)], 12.0);
    let cars = park(std::slice::from_ref(&road));
    // мест на 400 м вдвое больше, чем машин: ряд рваный, а не сплошной
    let places = ((400.0 - 2.0 * END_MARGIN) / CAR_PITCH) as usize * 2;
    assert!(cars.len() < places, "{} of {places}", cars.len());
    assert!(cars.len() > places / 5, "{} of {places}", cars.len());
    // и он тот же самый при повторной сборке
    let again = park(std::slice::from_ref(&road));
    assert_eq!(cars.len(), again.len());
    for (car, twin) in cars.iter().zip(&again) {
        assert_eq!(car.at, twin.at);
    }
}

/// Наименьшее расстояние от точки до ломаной — тем же способом, каким
/// глаз проверяет, лежит ли машина на асфальте.
fn distance_to_path(points: &[Vec2], at: Vec2) -> f32 {
    points
        .windows(2)
        .map(|link| {
            let span = link[1] - link[0];
            let t = (at - link[0]).dot(span) / span.length_squared().max(f32::EPSILON);
            at.distance(link[0] + span * t.clamp(0.0, 1.0))
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn the_row_stays_on_the_drawn_asphalt_through_a_bend() {
    // излом 30° на звеньях по 40 м: дуга оси уводит её от вершины на метр с
    // лишним, и ряд по сырым точкам вставал бы за кромкой
    let points = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(40.0, 0.0),
        Vec2::new(40.0 + 40.0 * 0.866, 20.0),
    ];
    let road = street(points, 8.0);
    let style = CarStyle {
        visible: true,
        occupancy: 1.0,
    };
    let cars = park_cars(
        std::slice::from_ref(&road),
        &junctions::marking_breaks(std::slice::from_ref(&road), is_carriageway, &[]),
        style,
        &drawn_axes(std::slice::from_ref(&road), &RoadShape::default()),
        TrafficSide::Right,
        &Districts::new(&[]),
    );
    assert!(!cars.is_empty());
    let drawn = drawn_axes(std::slice::from_ref(&road), &RoadShape::default()).remove(0);
    for car in &cars {
        let off = distance_to_path(&drawn, car.at);
        assert!(
            off <= road.width / 2.0 - car.shape.width() / 2.0 + 0.01,
            "кузов в {off} м от нарисованной осевой"
        );
    }
}

// --- слой целиком ----------------------------------------------------------
//
// Тесты на `mesh_cars`. До шва слой собирался внутри системы Bevy, и ни
// тумблер видимости, ни порог зума проверить было нечем: оба жили за ранним
// возвратом в самой системе, куда тест не дотягивался.

/// Улица, вдоль которой ряд заведомо встаёт (та же, что у
/// `cars_line_a_street_on_both_sides`), и ничего больше: домов нет, так что
/// множитель квартала ровно 1, стоянок нет — только бордюрный ряд.
fn city() -> MapData {
    MapData {
        roads: vec![street(
            vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)],
            12.0,
        )],
        ..default()
    }
}

/// Ближняя ступень: подробный кузов.
fn near_bucket() -> CarZoomBucket {
    CarZoomBucket::for_zoom(0.0)
}

#[test]
fn a_street_builds_one_blended_layer() {
    let (layers, report) = mesh_cars(
        near_bucket(),
        CarStyle::default(),
        straight(),
        &city(),
        &ParkingLayout::default(),
    );

    assert_eq!(layers.len(), 1, "кузова и тени идут одним мешем");
    assert_eq!(layers[0].name, "cars");
    assert_eq!(layers[0].z, Z_CAR);
    // тень машины полупрозрачна, кузов — нет
    assert_eq!(layers[0].material, MaterialSpec::Blend);
    assert_eq!(report.detail, Some(CarDetail::Full));
    assert!(report.cars > 0);
    assert!(report.vertices > 0);
}

#[test]
fn the_toggle_off_draws_nothing() {
    let style = CarStyle {
        visible: false,
        ..CarStyle::default()
    };
    let (layers, report) = mesh_cars(
        near_bucket(),
        style,
        straight(),
        &city(),
        &ParkingLayout::default(),
    );

    // не ранний выход у вызывающего: слой описан и пуст, а деспавн в
    // адаптере безусловен — забыть его негде
    assert!(layers.is_empty());
    assert_eq!(report.detail, None);
    assert_eq!(report.cars, 0);
    assert_eq!(report.vertices, 0);
}

#[test]
fn the_far_bucket_draws_nothing() {
    let far = CarZoomBucket::for_zoom(f32::INFINITY);
    assert_eq!(far.index, CarLods::max_zooms().count() - 1);

    let (layers, report) = mesh_cars(
        far,
        CarStyle::default(),
        straight(),
        &city(),
        &ParkingLayout::default(),
    );

    assert!(layers.is_empty());
    assert_eq!(report.detail, None);
    assert_eq!(report.cars, 0);
}
