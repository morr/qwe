use super::super::attach_entrances;
use super::super::fixtures::{CENTER, CITY, right_angles, skewed_house};
use super::*;
use crate::map::osm::MapData;
use crate::map::osm::fixture::{Overpass, building, square};
use bevy::math::Vec2;

/// Косо обведённый маленький дом выпрямляется в прямоугольник той же площади
/// на том же месте, и вход с его вершины переезжает на угол прямоугольника.
#[test]
fn a_skewed_small_house_is_squared_into_a_rectangle() {
    let ring = skewed_house(CENTER);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ring[1])
        .area(&[("building", "house")], ring.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.outer.len(), 4);
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    let area =
        |ring: &[Vec2]| signed_ring_area(&ring.iter().map(|p| *p - CENTER).collect::<Vec<_>>());
    assert!(
        (area(&house.outer) - area(&ring)).abs() < 0.5,
        "площадь и обход сохраняются"
    );
    let centre = |ring: &[Vec2]| ring.iter().map(|p| *p - CENTER).sum::<Vec2>() / 4.0;
    assert!(centre(&house.outer).distance(centre(&ring)) < 0.5);
    for (from, to) in ring.iter().zip(&house.outer) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
    assert!(
        house
            .entrances
            .iter()
            .any(|door| door.distance(house.outer[1]) < 0.01),
        "вход остался на старой вершине: {:?}",
        house.entrances
    );
}

/// Вход привязывается к дому по сантиметровому ключу, а не по точному
/// совпадению координат, — и по нему же переезжает на выпрямленный контур.
/// Нода входа в паре миллиметров от вершины (шум проекции) не должна оставлять
/// дверь на старом месте, пока дом уезжает на свои метры.
#[test]
fn an_entrance_a_hair_off_its_vertex_moves_with_the_squared_house() {
    let ring = skewed_house(CENTER);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ring[1] + Vec2::splat(0.002))
        .area(&[("building", "house")], ring.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.entrances.len(), 1, "вход привязался по ключу");
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    assert!(
        house.entrances[0].distance(house.outer[1]) < 0.01,
        "вход остался на старой вершине: {} против {}",
        house.entrances[0],
        house.outer[1]
    );
}

/// Не выпрямляются: ровный дом (посев по первой вершине не должен сдвинуться),
/// дом, делящий вершину с соседом (разошлись бы щелью), трапеция и крупное
/// здание.
#[test]
fn only_a_lone_skewed_small_house_is_squared() {
    let exact = square(CENTER, 5.0);
    let lone = skewed_house(CENTER + Vec2::new(60.0, 0.0));
    let terraced = skewed_house(CENTER + Vec2::new(0.0, 60.0));
    let neighbour = vec![
        terraced[0],
        terraced[0] + Vec2::new(0.0, -8.0),
        terraced[0] + Vec2::new(-8.0, -8.0),
        terraced[0] + Vec2::new(-8.0, 0.0),
    ];
    let trapezoid_at = CENTER + Vec2::new(-60.0, 0.0);
    let trapezoid = vec![
        // углы 51° и 129°: перекос 39°, за `SQUARE_SKEW_MAX`
        trapezoid_at + Vec2::new(-10.0, -5.0),
        trapezoid_at + Vec2::new(10.0, -5.0),
        trapezoid_at + Vec2::new(2.0, 5.0),
        trapezoid_at + Vec2::new(-2.0, 5.0),
    ];
    let big = skewed_house(CENTER + Vec2::new(0.0, -60.0))
        .iter()
        .map(|p| (*p - (CENTER + Vec2::new(0.0, -60.0))) * 2.0 + CENTER + Vec2::new(0.0, -60.0))
        .collect::<Vec<_>>();
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], exact.clone())
        .area(&[("building", "house")], lone.clone())
        .area(&[("building", "house")], terraced.clone())
        .area(&[("building", "house")], neighbour)
        .area(&[("building", "house")], trapezoid.clone())
        .area(&[("building", "house")], big.clone())
        .parse();

    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(same(&exact, &map.buildings[0].outer), "ровный дом тронут");
    assert!(
        !same(&lone, &map.buildings[1].outer),
        "одинокий косой дом не выпрямлен"
    );
    assert!(
        same(&terraced, &map.buildings[2].outer),
        "дом с общей вершиной выпрямлен"
    );
    assert!(
        same(&trapezoid, &map.buildings[4].outer),
        "трапеция выпрямлена"
    );
    assert!(
        same(&big, &map.buildings[5].outer),
        "крупное здание выпрямлено"
    );
}

/// Площадные слои в общей вершине считаются **не все**: край стоянки, площадки
/// и водоёма нарисован сам по себе, с разметкой, а на стоянке ещё и машинами,
/// так что уехавший от него дом виден на кадре, — а квартал `landuse` лежит
/// под всем, частные дома сплошь и рядом обведены по его границе, и щели там
/// не видно (решение коммита 174ab8a).
#[test]
fn a_house_sharing_a_vertex_with_a_parking_lot_stays_put_but_one_on_a_block_is_squared() {
    let corner = |ring: &[Vec2]| {
        vec![
            ring[0],
            ring[0] + Vec2::new(0.0, -20.0),
            ring[0] + Vec2::new(-20.0, -20.0),
            ring[0] + Vec2::new(-20.0, 0.0),
        ]
    };
    let by_lot = skewed_house(CENTER + Vec2::new(60.0, 0.0));
    let by_block = skewed_house(CENTER + Vec2::new(-60.0, 0.0));
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], by_lot.clone())
        .area(&[("building", "house")], by_block.clone())
        .area(&[("amenity", "parking")], corner(&by_lot))
        .area(&[("landuse", "residential")], corner(&by_block))
        .parse();

    assert_eq!(map.parking.len(), 1, "стоянка разобрана");
    assert_eq!(map.landuse.len(), 1, "квартал разобран");
    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(
        same(&by_lot, &map.buildings[0].outer),
        "дом с общей вершиной со стоянкой выпрямлен"
    );
    assert!(
        !same(&by_block, &map.buildings[1].outer),
        "дом на границе квартала не выпрямлен"
    );
}

/// Дом с пристройкой, обведённый кривой буквой Г, выпрямляется в Г из прямых
/// углов с тем же числом вершин и почти той же площадью, и вход с вершины
/// переезжает на её угол. Ровная Г и выпуклый шестиугольник не трогаются.
#[test]
fn a_skewed_ell_house_is_squared_into_an_ell() {
    // Тула, way 968378349: углы до 17° мимо прямого, 99 м²
    let ell = [
        (629.8, 3563.2),
        (640.4, 3569.9),
        (645.9, 3561.8),
        (642.2, 3557.9),
        (637.8, 3562.0),
        (631.6, 3558.6),
    ]
    .map(|(x, y)| Vec2::new(x - 638.0, y - 3563.0) + CENTER)
    .to_vec();
    let straight_at = CENTER + Vec2::new(60.0, 0.0);
    let straight = [
        (0.0, 0.0),
        (12.0, 0.0),
        (12.0, 6.0),
        (6.0, 6.0),
        (6.0, 12.0),
        (0.0, 12.0),
    ]
    .map(|(x, y)| straight_at + Vec2::new(x, y))
    .to_vec();
    let hexagon_at = CENTER + Vec2::new(-60.0, 0.0);
    let hexagon = (0..6)
        .map(|index| {
            hexagon_at + Vec2::from_angle(index as f32 * std::f32::consts::TAU / 6.0) * 5.0
        })
        .collect::<Vec<_>>();
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ell[2])
        .area(&[("building", "house")], ell.clone())
        .area(&[("building", "house")], straight.clone())
        .area(&[("building", "house")], hexagon.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.outer.len(), 6);
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    let area =
        |ring: &[Vec2]| signed_ring_area(&ring.iter().map(|p| *p - CENTER).collect::<Vec<_>>());
    assert!(
        (area(&house.outer) / area(&ell) - 1.0).abs() < 0.05,
        "площадь и обход сохраняются"
    );
    for (from, to) in ell.iter().zip(&house.outer) {
        assert!(from.distance(*to) < 1.0, "вершина {from} уехала в {to}");
    }
    assert!(
        house
            .entrances
            .iter()
            .any(|door| door.distance(house.outer[2]) < 0.01),
        "вход остался на старой вершине: {:?}",
        house.entrances
    );
    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(same(&straight, &map.buildings[1].outer), "ровная Г тронута");
    assert!(
        same(&hexagon, &map.buildings[2].outer),
        "шестиугольник выпрямлен"
    );
}

/// Частный сектор Тулы обводят и на 20–32° мимо прямого угла, и крупный
/// `building=house` тоже: оба выпрямляются. Здание без класса той же площади,
/// что крупный дом, остаётся как есть — у него порог скатной когорты.
#[test]
fn a_steeply_skewed_house_and_a_large_house_are_squared() {
    let around = |points: &[(f32, f32)], centre: (f32, f32), at: Vec2| {
        points
            .iter()
            .map(|(x, y)| Vec2::new(x - centre.0, y - centre.1) + at)
            .collect::<Vec<_>>()
    };
    // way 968378337: углы 65°–122°, 49 м²
    let steep = around(
        &[
            (580.8, 3528.2),
            (589.6, 3533.8),
            (592.8, 3529.0),
            (585.2, 3525.4),
        ],
        (586.0, 3529.0),
        CENTER,
    );
    // way 968378335: перекос 21°, 348 м²
    let large_ring = [
        (598.0, 3505.7),
        (626.5, 3521.8),
        (621.3, 3532.2),
        (596.0, 3517.0),
    ];
    let large = around(&large_ring, (610.0, 3519.0), CENTER + Vec2::new(80.0, 0.0));
    let untagged = around(&large_ring, (610.0, 3519.0), CENTER + Vec2::new(-80.0, 0.0));
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], steep.clone())
        .area(&[("building", "house")], large.clone())
        .area(&[("building", "yes")], untagged.clone())
        .parse();

    assert!(
        right_angles(&map.buildings[0].outer),
        "{:?}",
        map.buildings[0].outer
    );
    assert!(
        right_angles(&map.buildings[1].outer),
        "{:?}",
        map.buildings[1].outer
    );
    assert!(
        untagged
            .iter()
            .zip(&map.buildings[2].outer)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "здание без класса крупнее 250 м² выпрямлено"
    );
}

/// Выпрямление косых домиков — тоже само по себе, и видно, что оно сделало.
/// Что именно оно сохраняет (площадь, центр, вход на вершине), проверяет
/// `a_skewed_small_house_is_squared_into_a_rectangle` через весь конвейер;
/// здесь важно только то, что проход зовётся в одиночку.
#[test]
fn squaring_runs_on_its_own() {
    let before = skewed_house(CENTER);
    let mut map = MapData {
        buildings: vec![building(before.clone(), Vec::new())],
        ..MapData::default()
    };

    assert_eq!(square_skewed_houses(&mut map), 1);
    let after = &map.buildings[0].outer;
    assert_eq!(after.len(), 4);
    assert_ne!(*after, before, "контур не тронут");

    assert!(right_angles(after), "углы не стали прямыми");
    for (from, to) in before.iter().zip(after) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
}

/// Порядок проходов — это их интерфейс, и вот доказательство, что он
/// load-bearing: те же два прохода в обратном порядке теряют дверь.
///
/// Разметанный вход держит координату **ноды**, а выпрямление уносит вершину
/// контура. Пока дверь приложена к дому, её уносит тот же сантиметровый ключ;
/// если же выпрямить сначала, ключ вершины уже другой, и приложить дверь
/// становится не к чему. Написать такой тест до шва было нечем: оба прохода
/// жили внутри одного тела `parse`, и переставить их местами было негде.
#[test]
fn squaring_before_attaching_loses_the_door() {
    let ring = skewed_house(CENTER);
    let door = ring[1];
    let house = || MapData {
        buildings: vec![building(ring.clone(), Vec::new())],
        ..MapData::default()
    };

    // как в `finish_parse`: сначала дверь, потом выпрямление
    let mut in_order = house();
    assert_eq!(attach_entrances(&mut in_order, &[door]), 0);
    assert_eq!(square_skewed_houses(&mut in_order), 1);
    let carried = in_order.buildings[0].entrances[0];
    assert!(
        in_order.buildings[0]
            .outer
            .iter()
            .any(|&vertex| vertex.distance(carried) < 0.01),
        "дверь съехала с выпрямленного контура: {carried}"
    );

    // наоборот: дом выпрямлен, ноду двери прикладывать уже не к чему
    let mut reversed = house();
    assert_eq!(square_skewed_houses(&mut reversed), 1);
    assert_eq!(
        attach_entrances(&mut reversed, &[door]),
        1,
        "дверь нашла дом на старой вершине"
    );
    assert!(reversed.buildings[0].entrances.is_empty());
}

/// Прямоугольник по косому четырёхугольнику: углы прямые, площадь, центроид
/// и обход — те же, и вершина `i` встаёт рядом со своей.
#[test]
fn a_fitted_rectangle_keeps_the_area_the_centroid_and_the_winding() {
    let ring = skewed_house(CENTER);
    let quad = [ring[0], ring[1], ring[2], ring[3]];
    let fitted = fit_rectangle(&quad);

    assert!(right_angles(&fitted), "углы не прямые: {fitted:?}");
    let local = |ring: &[Vec2]| ring.iter().map(|point| *point - CENTER).collect::<Vec<_>>();
    let (before, after) = (
        signed_ring_area(&local(&quad)),
        signed_ring_area(&local(&fitted)),
    );
    assert!(
        (after / before - 1.0).abs() < 1e-3,
        "площадь {before} → {after}"
    );
    assert!(
        ring_area_centroid(&local(&quad)).distance(ring_area_centroid(&local(&fitted))) < 0.01,
        "центроид уехал"
    );
    for (from, to) in quad.iter().zip(&fitted) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
}

/// Г по косо обведённой Г: шесть прямых углов на своих местах. Контур, у
/// которого два соседних ребра лежат по одной оси, — не Г, и ответа нет.
#[test]
fn a_fitted_ell_squares_its_corners_and_refuses_what_is_not_an_ell() {
    // Тула, way 968378349: углы до 17° мимо прямого, 99 м²
    let ell = [
        (629.8, 3563.2),
        (640.4, 3569.9),
        (645.9, 3561.8),
        (642.2, 3557.9),
        (637.8, 3562.0),
        (631.6, 3558.6),
    ]
    .map(|(x, y)| Vec2::new(x - 638.0, y - 3563.0) + CENTER)
    .to_vec();

    let fitted = fit_ell(&ell).expect("Г не выпрямилась");
    assert_eq!(fitted.len(), ell.len());
    assert!(right_angles(&fitted), "углы не прямые: {fitted:?}");
    let area = |ring: &[Vec2]| {
        signed_ring_area(&ring.iter().map(|point| *point - CENTER).collect::<Vec<_>>())
    };
    assert!(
        (area(&fitted) / area(&ell) - 1.0).abs() < ELL_AREA_DRIFT,
        "площадь ушла: {} → {}",
        area(&ell),
        area(&fitted)
    );
    for (from, to) in ell.iter().zip(&fitted) {
        assert!(from.distance(*to) < 1.0, "вершина {from} уехала в {to}");
    }

    // прямоугольник с лишней вершиной посередине длинной стороны: рёбра по
    // осям не чередуются
    let not_an_ell = [
        (0.0, 0.0),
        (6.0, 0.0),
        (12.0, 0.0),
        (12.0, 8.0),
        (6.0, 8.0),
        (0.0, 8.0),
    ]
    .map(|(x, y)| CENTER + Vec2::new(x, y))
    .to_vec();
    assert!(fit_ell(&not_an_ell).is_none());
}
