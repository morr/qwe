use super::super::fixtures::{CITY, HALF, corners, inferred_street, paved_footway};
use super::super::verges;
use super::*;
use crate::map::osm::fixture::{Overpass, street};
use crate::map::osm::model::{Highway, Pavement};
use crate::map::roads::network::RoadNetwork;
use bevy::math::Vec2;

/// Проба обочины, вставшая в след перекрёстка, не меряет через устье: за ним
/// параллельная дорожка той же стороны тянула профиль обочины через
/// перекрёсток, и газон обочины выходил за устье языком в чужой квартал
/// (Тула, Первомайская у Вересаева: профиль у конца прыгал с 1.9 до 7.6 м).
#[test]
fn a_probe_inside_a_crossing_footprint_does_not_measure_across_it() {
    // улица вдоль x; дорожка у кромки до x = 80, перекрёсток на x = 90,
    // за устьем — параллельная дорожка той же стороны в 11 м от оси
    let street = street(vec![Vec2::ZERO, Vec2::new(95.0, 0.0)], 8.0);
    let crossing =
        crate::map::osm::fixture::street(vec![Vec2::new(90.0, 15.0), Vec2::new(90.0, -15.0)], 20.0);
    let mut roads = vec![
        street,
        crossing,
        paved_footway(-6.0, 0.0, 80.0),
        paved_footway(-11.0, 85.0, 95.0),
    ];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    let profile = &roads[0].verge_profile[1];
    assert!(
        profile.iter().all(|&(_, verge)| verge < 5.0),
        "профиль через устье: {profile:?}"
    );
    assert!(
        roads[0].verge_at(1, 90.0) < 5.0,
        "обочина на устье {:?}",
        roads[0].verge_profile
    );
}

/// Шов много-way улицы — не устье: улица, собранная из нескольких way, у
/// стыка собственных звеньев не должна терять обочину. Короткое звено,
/// пробы которого стоят в следе соседнего звена той же улицы, всё равно
/// меряет дорожку вдоль и получает обочину (Тула, Фрунзе: три звена к югу
/// от путепровода стояли без обочины вовсе, сосед — с газоном в 15 м).
#[test]
fn a_probe_on_a_seam_of_its_own_street_measures_the_footway() {
    // улица 0..100 и её продолжение 100..110 — одна улица сети улиц
    let long = street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0);
    let stub = street(vec![Vec2::new(100.0, 0.0), Vec2::new(110.0, 0.0)], 8.0);
    let half = long.width / 2.0;
    let axis = half + 2.0 + paved_footway(0.0, 0.0, 1.0).width / 2.0;
    // дорожка за газоном в два метра идёт вдоль всей улицы, через шов
    let mut roads = vec![long, stub, paved_footway(axis, 0.0, 110.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    // у короткого звена две пробы, и первая стоит в следе соседа по своей же
    // улице — выжившей половины мало для доли SEPARATE_SHARE, и звено
    // теряло обочину и отдачу дорожки целиком
    assert!(
        roads[1].verges[0] > 0.0,
        "звено за швом без обочины: {:?}",
        roads[1].verges
    );
    assert_eq!(roads[1].sidewalks[0], SidewalkSide::None);
    assert!(roads[0].verges[0] > 0.0);
    assert_eq!(roads[0].sidewalks[0], SidewalkSide::None);
}

/// Сторона с дорожкой вдоль — без полосы по тегу, отданная ей или с полосой —
/// получает обочину от кромки до оси дорожки; сторона без дорожки — нет.
#[test]
fn a_side_left_to_a_footway_gets_a_verge_up_to_it() {
    let street = inferred_street(Highway::Residential);
    let half = street.width / 2.0;
    let axis = half + 2.0 + paved_footway(0.0, 0.0, 1.0).width / 2.0;
    // слева дорожка за газоном в два метра, справа — ничего
    let mut roads = vec![street.clone(), paved_footway(axis, 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    assert_eq!(roads[0].sidewalks[0], SidewalkSide::None);
    assert!(
        (roads[0].verges[0] - (axis - half)).abs() < 0.01,
        "{:?}",
        roads[0].verges
    );
    assert_eq!(roads[0].verges[1], 0.0);
    // по тегу `sidewalk=separate`: полосы нет, дорожка у самой кромки
    let separate = RoadLine {
        sidewalks: [SidewalkSide::None, SidewalkSide::Tagged],
        ..street.clone()
    };
    let close = half + 0.5;
    let mut roads = vec![separate, paved_footway(close, 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    assert!(
        (roads[0].verges[0] - 0.5).abs() < 0.01,
        "{:?}",
        roads[0].verges
    );
    // полоса осталась — обочина под ней, до оси дорожки
    let mut roads = vec![street, paved_footway(half + 1.0, 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    assert_eq!(roads[0].sidewalks[0], SidewalkSide::Inferred);
    assert!(
        (roads[0].verges[0] - 1.0).abs() < 0.01,
        "{:?}",
        roads[0].verges
    );
    assert_eq!(roads[0].verges[1], 0.0);
}

/// Дорожка, уходящая от улицы, ведёт обочину за собой: у каждой пробы —
/// своя ширина (`RoadLine::verge_at`), а не одна медиана на всю сторону.
#[test]
fn a_verge_follows_a_footway_drifting_away() {
    let street = inferred_street(Highway::Residential);
    let half = street.width / 2.0;
    let drifting = RoadLine {
        pavement: Some(Pavement::Paved),
        ..crate::map::osm::fixture::footway(vec![
            Vec2::new(0.0, half + 3.0),
            Vec2::new(200.0, half + 9.0),
        ])
    };
    let mut roads = vec![street, drifting];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    let road = &roads[0];
    assert!(road.verges[0] > 0.0);
    let (near, far) = (road.verge_at(0, 10.0), road.verge_at(0, 190.0));
    assert!((near - 3.3).abs() < 0.2, "у начала {near}");
    assert!((far - 8.7).abs() < 0.2, "у конца {far}");
    assert_eq!(road.verge_at(1, 100.0), 0.0, "справа дорожки нет");
}

/// Дорожка в 13 м от кромки: у двусторонней улицы до неё мостится обочина
/// (Тула, Фрунзе), у половины разделённой — нет: так далеко по её внутренней
/// стороне лежит уже встречная половина.
#[test]
fn a_two_way_street_reaches_a_farther_footway_than_a_half() {
    let street = inferred_street(Highway::Residential);
    let half = street.width / 2.0;
    let axis = half + 13.0;
    let mut roads = vec![street.clone(), paved_footway(axis, 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    assert!(
        (roads[0].verges[0] - 13.0).abs() < 0.01,
        "{:?}",
        roads[0].verges
    );
    assert_eq!(
        roads[0].sidewalks[0],
        SidewalkSide::Inferred,
        "полоса остаётся"
    );
    let one_way = RoadLine {
        oneway: true,
        ..street
    };
    let mut roads = vec![one_way, paved_footway(axis, 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    verges::measure_footways_beside_streets(&mut roads, &network);
    assert_eq!(roads[0].verges, [0.0; 2]);
}

/// Шаг разбора целиком: `footway=sidewalk` вдоль улицы без тега доходит до
/// стороны улицы, по тегам OSM, как в Туле.
#[test]
fn a_mapped_sidewalk_beside_an_untagged_street_replaces_its_band() {
    let (sw, se, ..) = corners(HALF);
    let offset = Vec2::new(0.0, 11.0);
    let map = Overpass::new(CITY)
        .way(&[("highway", "primary"), ("lanes", "4")], vec![sw, se])
        .way(
            &[("highway", "footway"), ("footway", "sidewalk")],
            vec![sw + offset, se + offset],
        )
        .parse();
    let street = map
        .roads
        .iter()
        .find(|road| road.highway == Highway::Primary)
        .unwrap();
    // `sw → se` идёт на восток: дорожка севернее — слева
    assert_eq!(
        street.sidewalks,
        [SidewalkSide::None, SidewalkSide::Inferred]
    );
}
