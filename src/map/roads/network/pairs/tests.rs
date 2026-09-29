use std::borrow::Cow;

use bevy::prelude::*;

use super::{ALIGN_TRANSITION, PAIR_MIN, PAVED_MIN_GAP, PairRun, Pairs, TRAM_BED_MAX_GAP};
use crate::map::meshing::distance_to_path;
use crate::map::osm::fixture::{rail, street};
use crate::map::osm::model::{RailKind, RailLine};
use crate::map::osm::{Highway, RoadLine};
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::shape::RoadShape;
use crate::map::roads::tapers::{TAPER_PER_METER, Tapers};

/// Разделительная уже, чем столько, — асфальт: дефолт ручки `Median gap`.
fn median_gap() -> f32 {
    RoadShape::default().median_gap()
}

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
    aligned_with(roads, &[])
}

/// То же — с путями `rails` рядом.
fn aligned_with(roads: &[RoadLine], rails: &[RailLine]) -> (Pairs, Vec<Vec<Vec2>>) {
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    let mut pairs = Pairs::new(roads, &paths, median_gap(), rails);
    let (network, nodes) = (RoadNetwork::new(roads), RoadNodes::new(roads));
    let osm: Vec<&RoadLine> = roads.iter().collect();
    let wedges = Tapers::new(&osm, &network, &nodes, TAPER_PER_METER);
    pairs.align(&mut paths, roads, &network, &nodes, &wedges);
    let paths = paths.into_iter().map(Cow::into_owned).collect();
    (pairs, paths)
}

fn pairs_of(roads: &[RoadLine]) -> Pairs {
    aligned(roads).0
}

#[test]
fn two_opposite_halves_side_by_side_are_one_pair() {
    let pairs = pairs_of(&avenue(3, 0.6, 200.0));
    assert_eq!(pairs.count(), [1, 0, 0], "одна разделительная, асфальтом");
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
    assert_eq!(pairs.count(), [0, 1, 0]);
    assert!(pairs.medians[0].gap > median_gap());
}

#[test]
fn halves_running_the_same_way_are_not_a_pair() {
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].points.reverse();
    assert_eq!(pairs_of(&roads).count(), [0, 0, 0], "попутные — не пара");
}

#[test]
fn two_way_streets_and_other_classes_are_not_halves() {
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].oneway = false;
    assert_eq!(
        pairs_of(&roads).count(),
        [0, 0, 0],
        "двусторонняя — не половина"
    );
    let mut roads = avenue(2, 0.6, 200.0);
    roads[1].highway = Highway::Secondary;
    assert_eq!(
        pairs_of(&roads).count(),
        [0, 0, 0],
        "другой класс — не пара"
    );
}

#[test]
fn a_short_stretch_side_by_side_is_two_slips_meeting() {
    let pairs = pairs_of(&avenue(2, 0.6, PAIR_MIN - 2.0));
    assert_eq!(pairs.count(), [0, 0, 0]);
}

/// Улица, порезанная у моста на куски в десяток метров (Рязань,
/// Первомайский): короткий way нижней половины, продолжающий её длинный,
/// — пара, хотя кусок рядом с соседом короче `PAIR_MIN`; а короткий way
/// верхней, стоящий над швом нижней, не рвёт пару на куски, перескакивая с
/// одного её way на другой.
#[test]
fn a_short_way_continuing_a_paired_half_is_paired_too() {
    let apart = 2.0 * 3.3 + 1.0 + 4.0;
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 2),
        half(vec![Vec2::new(100.0, 0.0), Vec2::new(109.0, 0.0)], 2),
        half(vec![Vec2::new(109.0, apart), Vec2::new(96.0, apart)], 2),
        half(vec![Vec2::new(96.0, apart), Vec2::new(0.0, apart)], 2),
    ];
    let pairs = pairs_of(&roads);
    for road in 0..4 {
        assert!(!pairs.runs[road].is_empty(), "way {road} без пары");
    }
    assert!(
        pairs.runs[2].iter().all(|run| run.partner == 1),
        "короткий верхний — в паре с коротким нижним: {:?}",
        pairs.runs[2]
    );
}

#[test]
fn a_continuation_end_to_end_is_not_beside() {
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 2),
        half(vec![Vec2::new(200.0, 0.5), Vec2::new(101.0, 0.5)], 2),
    ];
    assert_eq!(pairs_of(&roads).count(), [0, 0, 0]);
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
        let apart = distance_to_path(at, &paths[1]);
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
    assert_eq!(pairs.count(), [1, 0, 0]);
    assert_eq!(pairs.medians[0].gap, PAVED_MIN_GAP);
    let width = 2.0 * 3.3 + 1.0;
    let apart = distance_to_path(at_x(&paths[0], 100.0), &paths[1]);
    assert!(
        (apart - width - PAVED_MIN_GAP).abs() < 0.05,
        "между осями {apart}"
    );
}

#[test]
fn a_node_shared_with_a_side_street_moves_with_the_half_and_the_street_follows() {
    // выезд из двора посреди куска. Прежде узел был закреплён, и у каждого
    // выезда ось возвращалась к OSM: кромка Красноармейского гуляла на
    // метр–полтора через каждые 50–100 м
    let mut roads = avenue(2, -0.6, 200.0);
    roads[0].points.insert(1, Vec2::new(100.0, 0.0));
    roads.push(street(
        vec![Vec2::new(100.0, -40.0), Vec2::new(100.0, 0.0)],
        7.6,
    ));
    let mut nodes = RoadNodes::new(&roads);
    let axes = crate::map::roads::axis::street_axes(
        &roads,
        &[],
        &RoadNetwork::new(&roads),
        &mut nodes,
        &RoadShape::default(),
    );
    let paths: Vec<Vec<Vec2>> = axes.paths.iter().map(|path| path.to_vec()).collect();
    let wanted = 2.0 * 3.3 + 1.0 + PAVED_MIN_GAP;
    for (step, apart) in apart_along(&paths, 0, &[1], 40.0, 160.0).into_iter().enumerate() {
        assert!(
            (apart - wanted).abs() < 0.05,
            "у x = {}: между осями {apart}, а не {wanted} — у выезда разводка сошла на нет",
            40.0 + step as f32 * 2.0
        );
    }
    let node = paths[2][paths[2].len() - 1];
    assert!(node.y < -0.2, "узел уехал с половиной: {node}");
    assert!(
        paths[0].contains(&node),
        "улица пришла в вершину половины: {node}"
    );
    assert_eq!(
        nodes.roads_at(node),
        &[0, 2],
        "узел находится и по новому месту"
    );
}

#[test]
fn the_halves_of_a_bridge_are_a_pair_and_are_pushed_apart() {
    // Красноармейский над каналом: без пары оси моста стояли в 9.5 м, и
    // подходы с обеих сторон сходились к ним
    let roads: Vec<RoadLine> = avenue(2, -0.6, 200.0)
        .into_iter()
        .map(|half| RoadLine {
            bridge: true,
            ..half
        })
        .collect();
    let (pairs, paths) = aligned(&roads);
    assert_eq!(pairs.count(), [1, 0, 0], "мост — пара, как любые половины");
    let apart = distance_to_path(at_x(&paths[0], 100.0), &paths[1]);
    let wanted = 2.0 * 3.3 + 1.0 + PAVED_MIN_GAP;
    assert!((apart - wanted).abs() < 0.05, "между осями {apart}");
}

#[test]
fn a_node_shared_with_a_bridge_stays_put() {
    // за узлом, который дорога повторить не может, ось держится на месте
    let mut roads = avenue(2, -0.6, 200.0);
    roads[0].points.insert(1, Vec2::new(100.0, 0.0));
    let apart = roads[1].points[0].y;
    roads.push(RoadLine {
        bridge: true,
        ..street(vec![Vec2::new(100.0, -40.0), Vec2::new(100.0, 0.0)], 7.6)
    });
    let (_, paths) = aligned(&roads);
    assert!(
        paths[0].contains(&Vec2::new(100.0, 0.0)),
        "узел с мостом не сдвинут"
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
        assert!((distance_to_path(*first, &paths[0]) - width / 2.0).abs() < 0.05);
    }
}

/// Трамвайный путь по `points`.
fn tram(points: Vec<Vec2>) -> RailLine {
    RailLine {
        kind: RailKind::Tram,
        ..rail(points, 1.2)
    }
}

/// Путь вдоль x на высоте `y` по всей длине проспекта.
fn track(y: f32, length: f32) -> RailLine {
    tram(vec![Vec2::new(-10.0, y), Vec2::new(length + 10.0, y)])
}

/// Середина зазора проспекта [`avenue`] по y.
fn middle(lanes: u8, gap: f32) -> f32 {
    (lanes as f32 * 3.3 + 1.0 + gap) / 2.0
}

#[test]
fn two_tracks_between_the_halves_are_a_tram_bed() {
    let mid = middle(2, 5.0);
    let rails = [track(mid - 1.7, 200.0), track(mid + 1.7, 200.0)];
    let (pairs, _) = aligned_with(&avenue(2, 5.0, 200.0), &rails);
    assert_eq!(pairs.count(), [1, 0, 1], "полотно — мощёное, газона нет");
    assert!(pairs.runs.iter().flatten().all(|run| run.tram && run.paved));
}

#[test]
fn a_single_track_between_the_halves_is_a_tram_bed_too() {
    let rails = [track(middle(2, 5.0), 200.0)];
    let (pairs, _) = aligned_with(&avenue(2, 5.0, 200.0), &rails);
    assert_eq!(pairs.count(), [1, 0, 1], "однопутка");
}

#[test]
fn a_track_outside_the_halves_is_not_a_tram_bed() {
    let rails = [track(-6.0, 200.0)];
    let (pairs, _) = aligned_with(&avenue(2, 5.0, 200.0), &rails);
    assert_eq!(pairs.count(), [0, 1, 0], "газон");
}

#[test]
fn a_tram_on_a_median_wider_than_a_bed_stays_a_lawn() {
    let gap = TRAM_BED_MAX_GAP + 4.0;
    let rails = [track(middle(2, gap), 200.0)];
    let (pairs, _) = aligned_with(&avenue(2, gap, 200.0), &rails);
    assert_eq!(pairs.count(), [0, 1, 0], "обособленное полотно на траве");
}

/// Расстояние между осями `paths[0]` и `paths[1..]` через каждые 2 м по x
/// от `from` до `to`.
fn apart_along(
    paths: &[Vec<Vec2>],
    first: usize,
    others: &[usize],
    from: f32,
    to: f32,
) -> Vec<f32> {
    (0..=((to - from) / 2.0) as usize)
        .map(|step| {
            let at = at_x(&paths[first], from + step as f32 * 2.0);
            others
                .iter()
                .map(|&other| distance_to_path(at, &paths[other]))
                .fold(f32::INFINITY, f32::min)
        })
        .collect()
}

#[test]
fn a_seam_of_the_partner_half_does_not_let_the_axes_go() {
    // половины наложены на 2 м; встречная — два way со швом у x = 130,
    // своя — один way: пара меняет way посреди куска
    let width = 3.0 * 3.3 + 1.0;
    let apart = width - 2.0;
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(300.0, 0.0)], 3),
        half(vec![Vec2::new(300.0, apart), Vec2::new(130.0, apart)], 3),
        half(vec![Vec2::new(130.0, apart), Vec2::ZERO.with_y(apart)], 3),
    ];
    let (pairs, paths) = aligned(&roads);
    assert_eq!(
        pairs.runs[0].len(),
        2,
        "у своей половины два куска — по way пары"
    );
    let wanted = width + PAVED_MIN_GAP;
    for (step, apart) in apart_along(&paths, 0, &[1, 2], 60.0, 240.0)
        .into_iter()
        .enumerate()
    {
        assert!(
            (apart - wanted).abs() < 0.05,
            "у x = {} между осями {apart}, а не {wanted}: у шва пары разводка сошла на нет",
            60.0 + step as f32 * 2.0
        );
    }
}

#[test]
fn a_seam_of_the_own_half_on_a_smoothed_axis_does_not_let_the_axes_go() {
    // своя половина — два way с изломом в 4° у x = 150, встречная — один
    // way; оси сглажены, как в игре, и сглаженная ось режется на ways не в
    // самом узле шва, а в ближайшей к нему точке дуги (`roads/axis.rs`)
    let width = 3.0 * 3.3 + 1.0;
    let apart = width - 2.0;
    let rise = 150.0 * 4f32.to_radians().tan();
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(150.0, 0.0)], 3),
        half(vec![Vec2::new(150.0, 0.0), Vec2::new(300.0, rise)], 3),
        half(
            vec![
                Vec2::new(300.0, rise + apart),
                Vec2::new(150.0, apart),
                Vec2::new(0.0, apart),
            ],
            3,
        ),
    ];
    let mut nodes = RoadNodes::new(&roads);
    let axes = crate::map::roads::axis::street_axes(
        &roads,
        &[],
        &RoadNetwork::new(&roads),
        &mut nodes,
        &RoadShape::default(),
    );
    assert_ne!(
        axes.paths[0].last(),
        Some(&Vec2::new(150.0, 0.0)),
        "нарисованная ось режется не в узле шва — иначе тест ничего не ловит"
    );
    let paths: Vec<Vec<Vec2>> = axes.paths.iter().map(|path| path.to_vec()).collect();
    let wanted = width + PAVED_MIN_GAP;
    for (step, apart) in apart_along(&paths, 0, &[2], 60.0, 148.0)
        .into_iter()
        .chain(apart_along(&paths, 1, &[2], 152.0, 240.0))
        .enumerate()
    {
        assert!(
            (apart - wanted).abs() < 0.1,
            "шаг {step}: между осями {apart}, а не {wanted}: у шва своей половины разводка \
             сошла на нет"
        );
    }
}

#[test]
fn a_widening_seam_of_the_own_half_meets_and_keeps_the_inner_kerb_straight() {
    // своя половина — две полосы до x = 150, дальше четыре: у шва клин
    // (`roads/tapers.rs`); встречная — один way в две полосы. Ось широкой
    // части OSM уходит наружу на полразницы ширин, и зазор между кромками по
    // всей длине один — 1.4 м
    let shift = (4.0 - 2.0) * 3.3 / 2.0;
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(150.0, 0.0)], 2),
        half(
            vec![
                Vec2::new(150.0, 0.0),
                Vec2::new(200.0, -shift),
                Vec2::new(400.0, -shift),
            ],
            4,
        ),
        half(vec![Vec2::new(400.0, 9.0), Vec2::new(0.0, 9.0)], 2),
    ];
    let (_, paths) = aligned(&roads);
    let (narrow_end, wide_start) = (paths[0][paths[0].len() - 1], paths[1][0]);
    assert!(
        narrow_end.distance(wide_start) < 0.05,
        "оси половины сходятся на шве: {narrow_end} против {wide_start}"
    );
    // зазор между кромками на клине — тот же, что у узкой части: клин сужает
    // внешнюю кромку, внутренняя идёт вдоль встречной
    let [narrow, wide, partner] = [roads[0].width, roads[1].width, roads[2].width];
    let length = (wide - narrow) * TAPER_PER_METER;
    let gap_at = |x: f32, own: &[Vec2], half: f32| {
        y_at(&paths[2], x) - partner / 2.0 - (y_at(own, x) + half)
    };
    let seam = gap_at(149.0, &paths[0], narrow / 2.0);
    assert!((seam - 1.4).abs() < 0.1, "у шва зазор {seam}");
    for at in [10.0, 30.0, 50.0] {
        let x = 150.0 + at;
        let gap = gap_at(x, &paths[1], (narrow + (wide - narrow) * at / length) / 2.0);
        assert!(
            (gap - seam).abs() < 0.1,
            "у x = {x} зазор между кромками {gap}, а у шва {seam}"
        );
    }
}

#[test]
fn a_gap_changing_at_a_taper_seam_changes_along_the_wedge_and_keeps_the_kerbs_straight() {
    // своя половина — две полосы до x = 150, дальше четыре, ось OSM одна
    // прямая; встречная — один way в две полосы. У узкой части между кромками
    // газон в 6 м, у широкой — асфальт в 2.7: зазор меняется на том же шве,
    // что и сечение (Тула, витрина 16)
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(150.0, 0.0)], 2),
        half(vec![Vec2::new(150.0, 0.0), Vec2::new(400.0, 0.0)], 4),
        half(vec![Vec2::new(400.0, 13.6), Vec2::new(0.0, 13.6)], 2),
    ];
    let (_, paths) = aligned(&roads);
    let [narrow, wide] = [roads[0].width, roads[1].width];
    let length = (wide - narrow) * TAPER_PER_METER;
    // внешняя кромка своей половины (встречная — слева, по +y)
    let outer = |x: f32| {
        if x <= 150.0 {
            y_at(&paths[0], x) - narrow / 2.0
        } else {
            let at = ((x - 150.0) / length).min(1.0);
            y_at(&paths[1], x) - (narrow + (wide - narrow) * at) / 2.0
        }
    };
    // до шва кромка прямая: зазор узкой части держится до самого узла
    let straight = outer(120.0);
    for x in [130.0, 140.0, 145.0, 149.0] {
        assert!(
            (outer(x) - straight).abs() < 0.05,
            "у x = {x} кромка {}, а до шва {straight}: смена зазора ушла за узел",
            outer(x)
        );
    }
    // на клине — прямая от кромки у шва к кромке у его конца: смена зазора
    // идёт вместе с клином, а не поперёк узла
    let (from, to) = (outer(150.5), outer(150.0 + length));
    for at in [10.0, 20.0, 30.0, 45.0, 60.0] {
        let expected = from + (to - from) * (at - 0.5) / (length - 0.5);
        let got = outer(150.0 + at);
        assert!(
            (got - expected).abs() < 0.05,
            "в {at} м от шва кромка {got}, а по прямой {expected}: клин с надломом"
        );
    }
}

#[test]
fn median_ends_meet_further_apart_at_a_seam_of_a_half_than_elsewhere() {
    // две разделительные вдоль x, торцы в 6.5 м друг от друга: у чистого шва
    // половины (Тула, витрина 16: газон до шва, асфальт после, 5.07 м) они
    // сводятся, без шва рядом — нет (через перекрёсток, Орёл, витрина 04)
    let median = |from: f32, to: f32| {
        let line = |y: f32| vec![Vec2::new(from, y), Vec2::new(to, y)];
        super::Median::lawn_for_test(line(0.0), [line(-2.0), line(2.0)])
    };
    let joined = |seams: &[Vec2]| {
        let mut pairs = Pairs {
            runs: Vec::new(),
            medians: vec![median(0.0, 100.0), median(106.5, 200.0)],
        };
        pairs.join_ends(seams);
        let first = pairs.medians[0].midline()[pairs.medians[0].midline().len() - 1];
        let second = pairs.medians[1].midline()[0];
        first.distance(second) < 0.01
    };
    assert!(joined(&[Vec2::new(104.0, -6.0)]), "у шва торцы не свелись");
    assert!(!joined(&[]), "без шва торцы в 6.5 м свелись");
    assert!(
        !joined(&[Vec2::new(150.0, -6.0)]),
        "шов далеко, а торцы свелись"
    );
}

/// Высота ломаной `path`, идущей по x, в точке `x`.
fn y_at(path: &[Vec2], x: f32) -> f32 {
    path.windows(2)
        .find(|link| link[0].x.min(link[1].x) <= x && x <= link[0].x.max(link[1].x))
        .map(|link| {
            let t = (x - link[0].x) / (link[1].x - link[0].x);
            link[0].y + (link[1].y - link[0].y) * t
        })
        .expect("x на ломаной")
}

#[test]
fn a_tram_bed_of_two_ways_keeps_its_own_gaps_without_a_step() {
    // половина A — два way по y = 0, половина B — два way, чей зазор
    // сходится от 6 м к 4
    let width = 2.0 * 3.3 + 1.0;
    let roads = vec![
        half(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 2),
        half(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 2),
        half(
            vec![Vec2::new(200.0, width + 6.0), Vec2::new(100.0, width + 5.0)],
            2,
        ),
        half(
            vec![Vec2::new(100.0, width + 5.0), Vec2::new(0.0, width + 4.0)],
            2,
        ),
    ];
    let rails = [tram(vec![
        Vec2::new(-10.0, (width + 3.8) / 2.0),
        Vec2::new(210.0, (width + 6.2) / 2.0),
    ])];
    let (pairs, paths) = aligned_with(&roads, &rails);
    assert_eq!(pairs.count(), [2, 0, 2]);
    // зазор у каждого куска — свой: общий стягивал бы половины там, где
    // картограф развёл их шире, и проспект сужался бы между перекрёстками
    let mut gaps: Vec<f32> = pairs.medians.iter().map(|median| median.gap).collect();
    gaps.sort_by(f32::total_cmp);
    assert!(
        (gaps[0] - 4.5).abs() < 0.1 && (gaps[1] - 5.5).abs() < 0.1,
        "зазоры {gaps:?}"
    );
    // и на шве нет ступеньки: расстояние между осями меняется плавно — метр
    // разницы зазоров за `ALIGN_TRANSITION`, круче всего полтора метра на
    // двадцать, 0.15 м на шаг в 2 м; ступенька была бы всем метром сразу
    let apart = apart_along(&paths, 0, &[2, 3], 20.0, 98.0)
        .into_iter()
        .chain(apart_along(&paths, 1, &[2, 3], 100.0, 180.0))
        .collect::<Vec<_>>();
    let steepest = 2.0 * 1.5 / ALIGN_TRANSITION + 0.02;
    for pair in apart.windows(2) {
        assert!((pair[1] - pair[0]).abs() < steepest, "ступенька {pair:?}");
    }
}

/// Одна дорога с кусками пары `runs`, пара у каждого слева или справа.
fn with_runs(runs: &[(f32, f32, bool)]) -> Pairs {
    let runs = runs
        .iter()
        .map(|&(from, to, left)| PairRun {
            from,
            to,
            partner: 1,
            left,
            gap: 0.6,
            paved: true,
            tram: false,
        })
        .collect();
    Pairs {
        runs: vec![runs],
        ..default()
    }
}

const BOTH: [bool; 2] = [true; 2];
const LEFT: [bool; 2] = [true, false];
const RIGHT: [bool; 2] = [false, true];

#[test]
fn a_band_without_runs_is_whole_or_one_piece_on_its_sides() {
    let none = with_runs(&[]);
    assert_eq!(none.band_pieces(0, BOTH, 0.0, 100.0), None, "режь нечего");
    assert_eq!(
        none.band_pieces(0, LEFT, 0.0, 100.0),
        Some(vec![(0.0, 100.0, LEFT)])
    );
    assert_eq!(none.band_pieces(0, [false; 2], 0.0, 100.0), Some(vec![]));
}

#[test]
fn a_band_loses_the_pair_side_on_a_run_and_in_a_gap_under_the_reach() {
    // дыра в 3 м между кусками с одной стороны — шов, без тротуара с неё;
    // в 15 м — нет
    let pairs = with_runs(&[(15.0, 40.0, true), (43.0, 70.0, true), (85.0, 87.0, true)]);
    assert_eq!(
        pairs.band_pieces(0, BOTH, 0.0, 100.0),
        Some(vec![
            (0.0, 15.0, BOTH),
            (15.0, 40.0, RIGHT),
            (40.0, 43.0, RIGHT),
            (43.0, 70.0, RIGHT),
            (70.0, 85.0, BOTH),
            (85.0, 87.0, RIGHT),
            (87.0, 100.0, BOTH),
        ])
    );
    // шов асфальтовой разделительной с газонной — восемь метров без пары
    // (Советская в Туле): тоже без тротуара
    let seam = with_runs(&[(15.0, 40.0, true), (48.0, 80.0, true)]);
    assert_eq!(
        seam.band_pieces(0, BOTH, 0.0, 100.0),
        Some(vec![
            (0.0, 15.0, BOTH),
            (15.0, 40.0, RIGHT),
            (40.0, 48.0, RIGHT),
            (48.0, 80.0, RIGHT),
            (80.0, 100.0, BOTH),
        ])
    );
    // пара то справа, то слева — шва нет
    let across = with_runs(&[(15.0, 40.0, true), (43.0, 80.0, false)]);
    assert_eq!(
        across.band_pieces(0, BOTH, 0.0, 100.0),
        Some(vec![
            (0.0, 15.0, BOTH),
            (15.0, 40.0, RIGHT),
            (40.0, 43.0, BOTH),
            (43.0, 80.0, LEFT),
            (80.0, 100.0, BOTH),
        ])
    );
}

#[test]
fn a_band_on_one_side_drops_the_pieces_left_bare_and_the_offcuts() {
    // тротуар только слева, пара слева: на куске пары полосы нет вовсе
    let pairs = with_runs(&[(20.0, 60.0, true)]);
    assert_eq!(
        pairs.band_pieces(0, LEFT, 0.0, 100.0),
        Some(vec![(0.0, 20.0, LEFT), (60.0, 100.0, LEFT)])
    );
    // стежок в 5 м сдвигает куски; обрезок в 0.25 м у торца пропущен, а
    // кусок у самого начала снимает сторону пары и до начала
    let pairs = with_runs(&[(0.25, 94.75, false)]);
    assert_eq!(
        pairs.band_pieces(0, BOTH, 5.0, 100.0),
        Some(vec![(0.0, 5.25, LEFT), (5.25, 99.75, LEFT)])
    );
}

/// Кусок пары, кончившийся ближе `PAIR_SIDE_REACH` к концу дороги, снимает
/// тротуар со стороны пары до самого конца: пробы теряют соседа за метры до
/// узла, и там тротуары обеих половин ложились клином на перекрёсток
/// (Калуга, Кирова × Плеханова). Дальше — тротуар возвращается.
#[test]
fn a_run_ending_near_the_end_keeps_the_pair_side_bare_to_the_end() {
    let near = with_runs(&[(20.0, 90.0, true)]);
    assert_eq!(
        near.band_pieces(0, BOTH, 0.0, 100.0),
        Some(vec![
            (0.0, 20.0, BOTH),
            (20.0, 90.0, RIGHT),
            (90.0, 100.0, RIGHT)
        ])
    );
    let far = with_runs(&[(20.0, 80.0, true)]);
    assert_eq!(
        far.band_pieces(0, BOTH, 0.0, 100.0),
        Some(vec![
            (0.0, 20.0, BOTH),
            (20.0, 80.0, RIGHT),
            (80.0, 100.0, BOTH)
        ])
    );
}

