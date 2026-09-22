use std::borrow::Cow;

use bevy::prelude::*;

use super::{ALIGN_TRANSITION, MEDIAN_GAP, PAIR_MIN, PAVED_MIN_GAP, Pairs};
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, RoadLine};
use crate::map::roads::network::{RoadNetwork, RoadNodes};

/// Половина проспекта: одностороннее полотно в `lanes` полос по `points`.
fn half(points: Vec<Vec2>, lanes: u8) -> RoadLine {
    RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(lanes),
        ..street(points, lanes as f32 * 3.3 + 1.0)
    }
}

/// Две встречные половины в `lanes` полос вдоль x длиной `length` с зазором
/// `gap` между кромками.
fn avenue(lanes: u8, gap: f32, length: f32) -> Vec<RoadLine> {
    let apart = lanes as f32 * 3.3 + 1.0 + gap;
    vec![
        half(vec![Vec2::ZERO, Vec2::new(length, 0.0)], lanes),
        half(vec![Vec2::new(length, apart), Vec2::new(0.0, apart)], lanes),
    ]
}

/// Пары и разведённые оси — так, как их отдаёт `axis::street_axes` без
/// сглаживания.
fn aligned(roads: &[RoadLine]) -> (Pairs, Vec<Vec<Vec2>>) {
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    let mut pairs = Pairs::new(roads, &paths, MEDIAN_GAP);
    pairs.align(
        &mut paths,
        roads,
        &RoadNetwork::new(roads),
        &RoadNodes::new(roads),
    );
    let paths = paths.into_iter().map(Cow::into_owned).collect();
    (pairs, paths)
}

fn pairs_of(roads: &[RoadLine]) -> Pairs {
    aligned(roads).0
}

#[test]
fn two_opposite_halves_side_by_side_are_one_pair() {
    let pairs = pairs_of(&avenue(3, 0.6, 200.0));
    assert_eq!(pairs.count(), [1, 0], "одна разделительная, асфальтом");
    let median = &pairs.medians[0];
    assert_eq!(median.roads, [0, 1]);
    assert!((median.gap - 0.6).abs() < 1e-3, "зазор {}", median.gap);
    let middle = (3.0 * 3.3 + 1.0 + 0.6) / 2.0;
    assert!(
        median
            .midline
            .iter()
            .all(|point| (point.y - middle).abs() < 1e-3),
        "середина между осями"
    );
    let [first, second] = [&pairs.runs[0], &pairs.runs[1]];
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert!(first[0].left, "вторая половина слева по ходу первой");
    assert!(second[0].left, "и первая — слева по ходу второй");
    assert!(first[0].to - first[0].from > 190.0);
}

#[test]
fn a_wide_lawn_is_a_median_too_but_not_a_paved_one() {
    let pairs = pairs_of(&avenue(2, 8.0, 200.0));
    assert_eq!(pairs.count(), [0, 1]);
    assert!(pairs.medians[0].gap > MEDIAN_GAP);
}

#[test]
fn halves_running_the_same_way_are_not_a_pair() {
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].points.reverse();
    assert_eq!(pairs_of(&roads).count(), [0, 0], "попутные — не пара");
}

#[test]
fn two_way_streets_and_other_classes_are_not_halves() {
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].oneway = false;
    assert_eq!(
        pairs_of(&roads).count(),
        [0, 0],
        "двусторонняя — не половина"
    );
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].highway = Highway::Secondary;
    assert_eq!(pairs_of(&roads).count(), [0, 0], "другой класс — не пара");
}

#[test]
fn a_short_stretch_side_by_side_is_two_slips_meeting() {
    let pairs = pairs_of(&avenue(2, 0.6, PAIR_MIN - 2.0));
    assert_eq!(pairs.count(), [0, 0]);
}

#[test]
fn a_continuation_end_to_end_is_not_beside() {
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 2),
        half(vec![Vec2::new(200.0, 0.5), Vec2::new(101.0, 0.5)], 2),
    ];
    assert_eq!(pairs_of(&roads).count(), [0, 0]);
}

/// Точка ломаной, идущей вдоль x, на абсциссе `x`: вершин после прореживания
/// мало, и искать ближайшую вершину нельзя.
fn at_x(path: &[Vec2], x: f32) -> Vec2 {
    path.windows(2)
        .find_map(|pair| {
            let (low, high) = (pair[0].x.min(pair[1].x), pair[0].x.max(pair[1].x));
            (low <= x && x <= high && high > low)
                .then(|| pair[0].lerp(pair[1], (x - pair[0].x) / (pair[1].x - pair[0].x)))
        })
        .expect("ломаная проходит эту абсциссу")
}

/// Расстояние от точки до ломаной.
fn distance_to(path: &[Vec2], at: Vec2) -> f32 {
    path.windows(2)
        .map(|pair| crate::map::osm::model::closest_on_segment(at, pair[0], pair[1]).distance(at))
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn a_wandering_gap_is_straightened_away_from_the_ends() {
    // зазор между кромками плывёт от 0.2 до 1.8 м по длине 300 м
    let width = 2.0 * 3.3 + 1.0;
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(300.0, 0.0)], 2),
        half(
            vec![Vec2::new(300.0, width + 1.8), Vec2::new(0.0, width + 0.2)],
            2,
        ),
    ];
    let (pairs, paths) = aligned(&roads);
    let gap = pairs.medians[0].gap;
    assert!((gap - 1.0).abs() < 0.1, "зазор — медиана по куску: {gap}");
    for x in [60.0, 150.0, 240.0] {
        let at = at_x(&paths[0], x);
        let apart = distance_to(&paths[1], at);
        assert!(
            (apart - width - gap).abs() < 0.05,
            "у x = {x} между осями {apart}, а не {}",
            width + gap
        );
    }
    // у концов разводка сходит на нет: концы осей на месте
    assert_eq!(paths[0][0], Vec2::ZERO);
    assert_eq!(paths[1][0], roads[1].points[0]);
}

#[test]
fn halves_laid_over_each_other_are_pushed_apart() {
    let roads = avenue(2, -0.6, 200.0);
    let (pairs, paths) = aligned(&roads);
    assert_eq!(pairs.count(), [1, 0]);
    assert_eq!(pairs.medians[0].gap, PAVED_MIN_GAP);
    let width = 2.0 * 3.3 + 1.0;
    let apart = distance_to(&paths[1], at_x(&paths[0], 100.0));
    assert!(
        (apart - width - PAVED_MIN_GAP).abs() < 0.05,
        "между осями {apart}"
    );
}

#[test]
fn a_node_shared_with_a_cross_street_stays_put() {
    let mut roads = avenue(2, -0.6, 200.0);
    roads[0].points.insert(1, Vec2::new(100.0, 0.0));
    let apart = roads[1].points[0].y;
    roads.push(street(
        vec![Vec2::new(100.0, -40.0), Vec2::new(100.0, 0.0)],
        7.6,
    ));
    let (_, paths) = aligned(&roads);
    assert!(
        paths[0].contains(&Vec2::new(100.0, 0.0)),
        "узел с поперечной улицей не сдвинут"
    );
    // а за переходом от узла половина уже разведена
    let away = at_x(&paths[0], 100.0 + ALIGN_TRANSITION + 8.0);
    assert!(away.y < -0.2, "половина отошла от пары: {away}");
    assert!(paths[1].iter().all(|point| point.y >= apart - 1e-3));
}

#[test]
fn the_median_lies_between_the_inner_kerbs() {
    let (pairs, paths) = aligned(&avenue(3, 6.0, 200.0));
    let median = &pairs.medians[0];
    assert!(!median.is_paved());
    assert_eq!(median.midline.len(), median.inner[0].len());
    let width = 3.0 * 3.3 + 1.0;
    for ((mid, first), second) in median
        .midline
        .iter()
        .zip(&median.inner[0])
        .zip(&median.inner[1])
    {
        assert!((first.distance(*second) - 6.0).abs() < 0.05, "газон 6 м");
        assert!((mid.distance(*first) - 3.0).abs() < 0.05);
        assert!((distance_to(&paths[0], *first) - width / 2.0).abs() < 0.05);
    }
}
