use super::super::ParseKnobs;
use super::super::fixtures::{CENTER, CITY};
use super::*;
use crate::map::osm::MapData;
use crate::map::osm::fixture::{Overpass, building, rect, residential_half, street};
use crate::map::osm::model::sidewalk_band;
use bevy::math::Vec2;

/// Дом, стеной заходящий на нарисованный тротуар улицы, отодвигается от неё
/// целиком и тянет за собой соседей по ряду; дом в стороне, дом с общей
/// вершиной и дом, сквозь который идёт улица, остаются на месте.
#[test]
fn a_house_on_the_sidewalk_is_pulled_back_into_the_block() {
    // residential 7.6 м: полоса с тротуаром и зазором — 3.8 + 1.67 + 2 от оси
    let reach = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT))
        + SIDEWALK_CLEARANCE;
    let street = vec![
        CENTER - Vec2::new(300.0, 0.0),
        CENTER + Vec2::new(300.0, 0.0),
    ];
    let house = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, gap),
            CENTER + Vec2::new(x + 12.0, gap + 10.0),
        )
    };
    let on_sidewalk = house(-200.0, 4.7);
    // сосед по ряду на той же линии — сдвигается на тот же сдвиг, хотя сам
    // не наезжает; сосед глубже в квартале — нет
    let in_row = house(-180.0, 6.5);
    let set_back = house(-160.0, 9.5);
    let clear = house(-120.0, 10.0);
    let crossed = house(-100.0, -5.0);
    let terraced = house(0.0, 4.7);
    let neighbour = house(12.0, 4.7);
    // уличный дом с юга: толкать надо в минус по y
    let south = rect(
        CENTER + Vec2::new(100.0, -14.0),
        CENTER + Vec2::new(112.0, -5.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential")], street)
        .area(&[("building", "yes")], on_sidewalk.clone())
        .area(&[("building", "yes")], clear.clone())
        .area(&[("building", "yes")], crossed.clone())
        .area(&[("building", "yes")], terraced.clone())
        .area(&[("building", "yes")], neighbour)
        .area(&[("building", "yes")], south.clone())
        .area(&[("building", "yes")], in_row.clone())
        .area(&[("building", "yes")], set_back.clone())
        .parse();

    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    let gap = |ring: &[Vec2]| {
        ring.iter()
            .map(|vertex| (vertex.y - CENTER.y).abs())
            .fold(f32::INFINITY, f32::min)
    };
    let pulled = &map.buildings[0].outer;
    assert!(
        gap(pulled) >= reach - 0.06,
        "дом остался на тротуаре: {pulled:?}"
    );
    assert!(gap(pulled) < reach + 0.1, "дом унесён дальше нужного");
    assert!(
        pulled
            .iter()
            .zip(&on_sidewalk)
            .all(|(a, b)| (a.x - b.x).abs() < 0.01 && a.y > b.y),
        "сдвиг не поперёк улицы от неё"
    );
    assert!(
        same(&clear, &map.buildings[1].outer),
        "дом в стороне тронут"
    );
    assert!(
        same(&crossed, &map.buildings[2].outer),
        "дом на оси улицы сдвинут"
    );
    assert!(
        same(&terraced, &map.buildings[3].outer),
        "дом с общей вершиной сдвинут"
    );
    let south_pulled = &map.buildings[5].outer;
    assert!(gap(south_pulled) >= reach - 0.06, "{south_pulled:?}");
    assert!(south_pulled.iter().zip(&south).all(|(a, b)| a.y < b.y));

    // ряд держит линию: сосед сдвинут ровно на сдвиг наезжающего дома
    let row_shift = pulled[0].y - on_sidewalk[0].y;
    let neighbour_shift = map.buildings[6].outer[0].y - in_row[0].y;
    assert!(
        (neighbour_shift - row_shift).abs() < 0.01,
        "сосед по ряду сдвинут на {neighbour_shift}, ряд — на {row_shift}"
    );
    assert!(
        same(&set_back, &map.buildings[7].outer),
        "дом в глубине квартала сдвинут вместе с рядом"
    );
}

/// Дому, которому нужно больше предела, не отказывают: он сдвигается на
/// предел и не держит на месте свой ряд; а сдвиг, упёршийся в забор или в
/// соседнее здание, укорачивается, пока между ними не останется зазор.
#[test]
fn a_pull_is_capped_and_stops_short_of_what_stands_behind() {
    let reach = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT))
        + SIDEWALK_CLEARANCE;
    let street = vec![
        CENTER - Vec2::new(300.0, 0.0),
        CENTER + Vec2::new(300.0, 0.0),
    ];
    let house = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, gap),
            CENTER + Vec2::new(x + 12.0, gap + 10.0),
        )
    };
    // стена в метре от оси: нужно 6.76 м, предел 6
    let deep = house(-200.0, 1.0);
    // сосед по той же стороне, в 8 м от глубокого: встал бы с ним в ряд
    let beside = house(-180.0, 4.7);
    // забор в 2.5 м за задней стеной: полный сдвиг 3.06 м его пересёк бы
    let fenced = house(0.0, 4.7);
    let fence = vec![
        CENTER + Vec2::new(-5.0, 17.2),
        CENTER + Vec2::new(17.0, 17.2),
    ];
    // здание в 1.3 м за задней стеной, само улицы не касается
    let crowded = house(150.0, 4.7);
    let behind = rect(
        CENTER + Vec2::new(150.0, 16.0),
        CENTER + Vec2::new(162.0, 26.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential")], street)
        .area(&[("building", "yes")], deep.clone())
        .area(&[("building", "yes")], beside.clone())
        .area(&[("building", "yes")], fenced.clone())
        .way(&[("barrier", "fence")], fence)
        .area(&[("building", "yes")], crowded.clone())
        .area(&[("building", "yes")], behind.clone())
        .parse();

    let shift = |index: usize, ring: &[Vec2]| map.buildings[index].outer[0].y - ring[0].y;
    assert!(
        (shift(0, &deep) - SIDEWALK_SHIFT_MAX).abs() < 0.01,
        "глубокий дом сдвинут на {}",
        shift(0, &deep)
    );
    assert!(
        (shift(1, &beside) - (reach - 4.7)).abs() < 0.06,
        "сосед глубокого дома сдвинут на {}",
        shift(1, &beside)
    );
    let full = reach - 4.7;
    assert!(
        (shift(2, &fenced) - full * 0.5).abs() < 0.01,
        "дом у забора сдвинут на {}",
        shift(2, &fenced)
    );
    assert!(
        (shift(3, &crowded) - full * 0.25).abs() < 0.01,
        "дом перед соседом сдвинут на {}",
        shift(3, &crowded)
    );
    assert!(
        map.buildings[4]
            .outer
            .iter()
            .zip(&behind)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "здание позади тронуто"
    );
}

/// Отодвигание домов от тротуаров — в одиночку, на карте, собранной руками:
/// улица и два дома, а не сцена Overpass. Что именно проход сохраняет и кого
/// оставляет на месте, проверяет `a_house_on_the_sidewalk_is_pulled_back_into_the_block`
/// через весь конвейер; здесь важно, что проход зовётся по имени и что
/// счётчики у него не нулевые.
#[test]
fn pulling_houses_off_the_sidewalks_runs_on_its_own() {
    // residential 7.6 м: полоса с тротуаром и зазором — 3.8 + 1.67 + 2 от оси
    let reach = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT))
        + SIDEWALK_CLEARANCE;
    let on_sidewalk = rect(
        CENTER + Vec2::new(-20.0, 4.7),
        CENTER + Vec2::new(-8.0, 14.7),
    );
    let clear = rect(
        CENTER + Vec2::new(20.0, 20.0),
        CENTER + Vec2::new(32.0, 30.0),
    );
    let mut map = MapData {
        roads: vec![street(
            vec![
                CENTER - Vec2::new(300.0, 0.0),
                CENTER + Vec2::new(300.0, 0.0),
            ],
            2.0 * residential_half(ParseKnobs::DEFAULT),
        )],
        buildings: vec![
            building(on_sidewalk.clone(), Vec::new()),
            building(clear.clone(), Vec::new()),
        ],
        ..MapData::default()
    };

    let pulled = pull_houses_off_sidewalks(&mut map);
    assert_eq!(pulled.moved, 1, "наезжающий дом не сдвинут");
    assert_eq!(pulled.partly, 0, "сдвигу ничто не мешало");
    assert_eq!(pulled.left, 0);

    let gap = map.buildings[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        gap >= reach - 0.06,
        "дом остался на тротуаре: {gap} м от оси"
    );
    assert!(gap < reach + 0.1, "дом унесён дальше нужного: {gap}");
    assert!(
        map.buildings[1]
            .outer
            .iter()
            .zip(&clear)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "дом в стороне от улицы тронут"
    );
}

/// Ближайшая пара точек двух отрезков: у пересекающихся её нет, у
/// параллельных она поперёк, у разминувшихся торцами — концы.
#[test]
fn the_closest_pair_of_two_segments_is_none_only_when_they_cross() {
    let (a, b) = (CENTER, CENTER + Vec2::new(10.0, 0.0));
    assert!(
        closest_between_segments(
            a,
            b,
            CENTER + Vec2::new(5.0, -5.0),
            CENTER + Vec2::new(5.0, 5.0)
        )
        .is_none(),
        "у пересекающихся отрезков ближайшей пары не бывает"
    );

    let (near_a, near_c) = closest_between_segments(
        a,
        b,
        CENTER + Vec2::new(2.0, 3.0),
        CENTER + Vec2::new(8.0, 3.0),
    )
    .expect("параллельные отрезки не пересекаются");
    assert!(
        (near_a.distance(near_c) - 3.0).abs() < 1e-3,
        "зазор {} вместо 3",
        near_a.distance(near_c)
    );
    assert!(
        (near_a.y - CENTER.y).abs() < 1e-3 && (near_c.y - CENTER.y - 3.0).abs() < 1e-3,
        "пара взята не поперёк: {near_a} и {near_c}"
    );

    let (near_a, near_c) = closest_between_segments(
        a,
        b,
        CENTER + Vec2::new(14.0, 0.0),
        CENTER + Vec2::new(20.0, 0.0),
    )
    .expect("отрезки на одной прямой не пересекаются");
    assert!(near_a.distance(b) < 1e-3, "{near_a} вместо конца отрезка");
    assert!(near_c.distance(CENTER + Vec2::new(14.0, 0.0)) < 1e-3);
}
