use super::super::fixtures::{layer, one_street, timeless, with_network};
use super::*;
use crate::map::osm::SidewalkSide;
use crate::map::osm::fixture;
use crate::map::osm::fixture::street;
use crate::map::roads::{RoadShape, RoadStyle, VergeZoomBucket, mesh_roads};

fn primary() -> RoadLine {
    RoadLine {
        highway: Highway::Primary,
        ..street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.0)
    }
}

#[test]
fn a_primary_parks_in_pockets_and_a_residential_street_on_the_lane() {
    let road = primary();
    assert_eq!(kerb_parking(&road, 1), KerbParking::Pocket);
    let residential = street(road.points.clone(), 8.0);
    assert_eq!(kerb_parking(&residential, 1), KerbParking::Lane);
    let bare = RoadLine {
        sidewalks: [SidewalkSide::Tagged, SidewalkSide::None],
        ..primary()
    };
    assert_eq!(kerb_parking(&bare, 1), KerbParking::No, "врезать некуда");
    let tagged = RoadLine {
        parking: [KerbParking::Lane, KerbParking::No],
        ..primary()
    };
    assert_eq!(kerb_parking(&tagged, 0), KerbParking::Lane);
    assert_eq!(kerb_parking(&tagged, 1), KerbParking::No);
}

#[test]
fn a_tagged_pocket_stops_short_of_a_junction_and_runs_through_a_way_end() {
    let road = RoadLine {
        parking: [KerbParking::Pocket; 2],
        ..primary()
    };
    let junction = Break {
        at: Vec2::new(120.0, 0.0),
        reach: 8.0,
    };
    let sides = kerbsides(
        &road,
        &road.points,
        &[junction],
        TrafficSide::Right,
        &KerbLots::new(&[]),
    );
    assert_eq!(sides.len(), 2);
    let pockets = &sides[0].pockets;
    assert_eq!(pockets.len(), 2);
    let reach = 8.0 + POCKET_CLEARANCE;
    assert_eq!(pockets[0].from, 0.0);
    assert!((pockets[0].to - (120.0 - reach)).abs() < 1e-3);
    assert_eq!(pockets[0].tapers, [false, true], "у торца way — без скоса");
    assert_eq!(pockets[1].tapers, [true, false]);
    assert!(sides[0].pocket_at(60.0).is_some());
    assert!(sides[0].pocket_at(120.0).is_none());
    assert!(!sides[0].lane, "мимо кармана на магистрали не стоят");
}

fn tagged(points: Vec<Vec2>, parking: [KerbParking; 2]) -> RoadLine {
    RoadLine {
        parking,
        ..street(points, 8.0)
    }
}

fn city_sides(roads: &[RoadLine], breaks: Vec<Vec<Break>>) -> Vec<Vec<Kerbside>> {
    let paths: Vec<Vec<Vec2>> = roads.iter().map(|road| road.points.clone()).collect();
    let breaks = RowBreaks::for_test(breaks);
    all_kerbsides(
        roads,
        &RoadNodes::new(roads),
        &paths,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&[]),
    )
}

/// Карман через торец way — один карман, и [`POCKET_MIN`] меряется по
/// нему целиком. Ул. Дзержинского (Тула, 5994, 3185): way в 28 м от
/// перехода и way в 23 м до проезда во двор. Кусок на втором после скоса
/// короче двух машин и отбрасывался, а первый обрывался в торец way
/// ступенькой; вместе это карман, со скосами на внешних торцах.
#[test]
fn a_pocket_split_by_a_way_end_is_measured_whole() {
    let pocket = [KerbParking::Pocket; 2];
    let roads = [
        tagged(vec![Vec2::ZERO, Vec2::new(28.0, 0.0)], pocket),
        tagged(vec![Vec2::new(28.0, 0.0), Vec2::new(51.0, 0.0)], pocket),
    ];
    let crossing = Break {
        at: Vec2::ZERO,
        reach: 2.0,
    };
    let driveway = Break {
        at: Vec2::new(51.0, 0.0),
        reach: 3.0,
    };
    let sides = city_sides(&roads, vec![vec![crossing], vec![driveway]]);
    for (road, tapers) in [(0, [true, false]), (1, [false, true])] {
        for side in &sides[road] {
            assert_eq!(side.pockets.len(), 1, "{side:?}");
            assert_eq!(side.pockets[0].tapers, tapers, "{side:?}");
        }
    }
}

/// Та же пара, но короче: вместе меньше двух машин — ни куска, ни
/// ступеньки на соседе.
#[test]
fn a_short_pocket_across_a_way_end_goes_whole() {
    let pocket = [KerbParking::Pocket; 2];
    let roads = [
        tagged(vec![Vec2::ZERO, Vec2::new(12.0, 0.0)], pocket),
        tagged(vec![Vec2::new(12.0, 0.0), Vec2::new(30.0, 0.0)], pocket),
    ];
    let junction = |at: f32| Break {
        at: Vec2::new(at, 0.0),
        reach: 0.0,
    };
    let sides = city_sides(&roads, vec![vec![junction(0.0)], vec![junction(30.0)]]);
    assert!(
        sides.iter().flatten().all(|side| side.pockets.is_empty()),
        "{sides:?}"
    );
}

/// Там, где сосед продолжает карман, торец остаётся открытым — и когда
/// сосед нарисован навстречу: сторона у него та же на земле, но другого
/// знака. На запрещённой стороне соседа карман первого закрыт скосом.
#[test]
fn a_pocket_runs_on_into_a_reversed_way_on_the_same_kerb() {
    let roads = [
        tagged(
            vec![Vec2::ZERO, Vec2::new(100.0, 0.0)],
            [KerbParking::Pocket; 2],
        ),
        // навстречу: его левая сторона — правая первого
        tagged(
            vec![Vec2::new(200.0, 0.0), Vec2::new(100.0, 0.0)],
            [KerbParking::Pocket, KerbParking::No],
        ),
    ];
    let sides = city_sides(&roads, vec![Vec::new(), Vec::new()]);
    let first = |side: f32| {
        sides[0]
            .iter()
            .find(|kerbside| kerbside.side == side)
            .unwrap()
            .pockets[0]
    };
    assert_eq!(first(-1.0).tapers, [true, false], "справа — продолжается");
    assert_eq!(first(1.0).tapers, [true, true], "слева — запрет у соседа");
}

/// Магистраль, разрезанная OSM на два way по ходу: карман с обеих сторон
/// идёт через торец насквозь — скосы только на внешних торцах улицы.
#[test]
fn a_kerb_pocket_continues_across_the_way_end_of_its_own_street() {
    let primary = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        parking: [KerbParking::Pocket; 2],
        ..street(points, 14.0)
    };
    let roads = [
        primary(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)]),
        primary(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)]),
    ];
    let sides = city_sides(&roads, vec![Vec::new(), Vec::new()]);
    for (road, tapers) in [(0, [true, false]), (1, [false, true])] {
        assert_eq!(sides[road].len(), 2);
        for side in &sides[road] {
            assert_eq!(side.pockets.len(), 1, "{side:?}");
            assert_eq!(side.pockets[0].tapers, tapers, "{side:?}");
        }
    }
}

/// Шов двух way под прямым углом — не продолжение (`RoadNodes::next_way`,
/// излом круче `MAX_BEND`): карман не огибает угол, оба торца со скосом.
/// До `next_way` сосед искался по совпадению точки при любом изломе.
#[test]
fn a_kerb_pocket_does_not_turn_a_right_angle_way_end() {
    let pocket = [KerbParking::Pocket; 2];
    let roads = [
        tagged(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], pocket),
        tagged(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], pocket),
    ];
    let sides = city_sides(&roads, vec![Vec::new(); 2]);
    for side in sides.iter().flatten() {
        assert_eq!(side.pockets.len(), 1, "{side:?}");
        assert_eq!(side.pockets[0].tapers, [true, true], "{side:?}");
    }
}

/// Т-узел из трёх торцов: карман прямой улицы идёт на её продолжение, а
/// не на примыкающий way, и у примыкающего торец закрыт скосом. Разрывов
/// нет нарочно: в городе узел трёх участников ряда рвёт карманы сам
/// (`junctions::marking_breaks`), и здесь проверяется только выбор соседа.
#[test]
fn a_kerb_pocket_stops_at_a_t_junction_side_way() {
    let pocket = [KerbParking::Pocket; 2];
    let roads = [
        tagged(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], pocket),
        // примыкающий — раньше продолжения по индексу
        tagged(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], pocket),
        tagged(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], pocket),
    ];
    let sides = city_sides(&roads, vec![Vec::new(); 3]);
    for (road, tapers) in [(0, [true, false]), (1, [true, true]), (2, [false, true])] {
        for side in &sides[road] {
            assert_eq!(side.pockets.len(), 1, "{road}: {side:?}");
            assert_eq!(side.pockets[0].tapers, tapers, "{road}: {side:?}");
        }
    }
}

/// Без тега карманы редкие и короткие: каждый со скосами и в пределах
/// длины по правилу, между соседними — тротуар, на улице в два километра
/// карманами занята малая часть бордюра, но не ноль; и тот же ответ на
/// второй вызов — лента и машины его не разойдутся.
#[test]
fn rule_pockets_are_rare_short_and_repeatable() {
    let road = RoadLine {
        highway: Highway::Primary,
        ..street(vec![Vec2::new(3.7, 1.2), Vec2::new(2003.7, 1.2)], 14.0)
    };
    // перекрёсток каждые 150 м — кварталы
    let breaks: Vec<Break> = (1..13)
        .map(|block| Break {
            at: Vec2::new(3.7 + 150.0 * block as f32, 1.2),
            reach: 8.0,
        })
        .collect();
    let sides = kerbsides(
        &road,
        &road.points,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&[]),
    );
    let again = kerbsides(
        &road,
        &road.points,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&[]),
    );
    let mut covered = 0.0;
    for (side, repeat) in sides.iter().zip(&again) {
        assert_eq!(side.pockets, repeat.pockets, "посев от улицы");
        for pocket in &side.pockets {
            let length = pocket.to - pocket.from;
            assert!(
                (RULE_POCKET_LENGTH.0..=RULE_POCKET_LENGTH.1).contains(&length),
                "{pocket:?}"
            );
            assert_eq!(pocket.tapers, [true, true]);
            covered += length;
        }
        for pair in side.pockets.windows(2) {
            assert!(pair[1].from - pair[0].to >= RULE_POCKET_GAP.0 - 1e-3);
        }
    }
    let share = covered / (2.0 * 2000.0);
    assert!(
        share > 0.02 && share < 0.3,
        "доля бордюра в карманах {share}"
    );
}

/// Стоянка за тротуаром справа от `from` до `to` по x: её край в 2.5 м за
/// наружным краем кармана улицы шириной 14. Двор, а не карман вдоль улицы:
/// карман гасит **любая** стоянка перед ним, не только своя.
fn lot_on_the_right(from: f32, to: f32) -> PolyArea {
    crate::map::osm::fixture::area(
        crate::map::osm::AreaKind::Parking(crate::map::osm::LotKind::Yard),
        vec![
            Vec2::new(from, -12.0),
            Vec2::new(to, -12.0),
            Vec2::new(to, -40.0),
            Vec2::new(from, -40.0),
        ],
    )
}

/// Без тега бухта рядом со стоянкой не ставится, остальные бухты — ни
/// на той стороне, ни на другой — не сдвигаются.
#[test]
fn a_rule_pocket_is_not_laid_beside_a_parking_lot() {
    let road = RoadLine {
        highway: Highway::Primary,
        ..street(vec![Vec2::new(3.7, 1.2), Vec2::new(2003.7, 1.2)], 14.0)
    };
    // перекрёсток каждые 150 м — кварталы, и в каких-то из них бухты есть
    let breaks: Vec<Break> = (1..13)
        .map(|block| Break {
            at: Vec2::new(3.7 + 150.0 * block as f32, 1.2),
            reach: 8.0,
        })
        .collect();
    let bare = kerbsides(
        &road,
        &road.points,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&[]),
    );
    let lots = [lot_on_the_right(0.0, 2000.0)];
    let beside = kerbsides(
        &road,
        &road.points,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&lots),
    );
    assert!(!bare[0].pockets.is_empty(), "правило кладёт бухты справа");
    assert!(beside[0].pockets.is_empty(), "{:?}", beside[0].pockets);
    assert_eq!(
        beside[1].pockets, bare[1].pockets,
        "левую сторону стоянка не трогает"
    );

    let lots = [lot_on_the_right(1000.0, 2000.0)];
    let half = kerbsides(
        &road,
        &road.points,
        &breaks,
        TrafficSide::Right,
        &KerbLots::new(&lots),
    );
    let before: Vec<_> = bare[0].pockets.iter().filter(|p| p.to < 990.0).collect();
    let after: Vec<_> = half[0].pockets.iter().collect();
    assert_eq!(after, before, "бухты до стоянки остаются где были");
}

/// По тегу карман прерывается перед стоянкой на её длину, со скосами за
/// её краями, и продолжается за ней.
#[test]
fn a_tagged_pocket_breaks_off_beside_a_parking_lot() {
    let road = RoadLine {
        parking: [KerbParking::Pocket; 2],
        ..primary()
    };
    let lots = [lot_on_the_right(80.0, 120.0)];
    let sides = kerbsides(
        &road,
        &road.points,
        &[],
        TrafficSide::Right,
        &KerbLots::new(&lots),
    );
    let right = &sides[0].pockets;
    assert_eq!(right.len(), 2, "{right:?}");
    assert!(right[0].to <= 80.0 && right[0].to > 60.0, "{right:?}");
    assert!(right[1].from >= 120.0 && right[1].from < 140.0, "{right:?}");
    assert_eq!(right[0].tapers, [false, true]);
    assert!(sides[0].pocket_at(100.0).is_none());
    assert_eq!(sides[1].pockets.len(), 1, "слева стоянки нет");
}

#[test]
fn the_outline_lies_on_its_side_and_narrows_at_the_taper() {
    let road = primary();
    let pocket = Pocket {
        from: 20.0,
        to: 80.0,
        tapers: [true, true],
    };
    let contour = outline(&road.points, &pocket, -1.0, [7.0, 9.5]);
    assert!(contour.iter().all(|at| at.y < 0.0), "справа — к югу");
    let outer_x: Vec<f32> = contour
        .iter()
        .filter(|at| (at.y + 9.5).abs() < 1e-3)
        .map(|at| at.x)
        .collect();
    let min = outer_x.iter().copied().fold(f32::INFINITY, f32::min);
    assert!((min - (20.0 + POCKET_TAPER)).abs() < 1e-3);
}

/// Зебра в метре от торца way рвёт ряд и на way, что продолжает улицу
/// за торцом, — от торца на остаток полузебры ([`CROSSING_SPILL`]).
#[test]
fn a_zebra_at_a_way_end_spills_onto_the_next_way() {
    let roads = [
        street(
            vec![Vec2::ZERO, Vec2::new(99.0, 0.0), Vec2::new(100.0, 0.0)],
            8.0,
        ),
        street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 8.0),
    ];
    let zebra = RoadNode {
        pos: Vec2::new(99.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
        osm_id: None,
    };
    let found = crossing_breaks(&roads, &RoadNodes::new(&roads), &[zebra]);
    assert_eq!(
        found,
        vec![
            (
                0,
                Break {
                    at: Vec2::new(99.0, 0.0),
                    reach: ZEBRA_LENGTH / 2.0
                }
            ),
            (
                1,
                Break {
                    at: Vec2::new(100.0, 0.0),
                    reach: ZEBRA_LENGTH / 2.0 - 1.0
                }
            ),
        ]
    );
}

/// Зебра у торца way в Т-узле переливается только на продолжение улицы,
/// не на примыкающий way: у того в узле свой разрыв перекрёстка, шире.
#[test]
fn a_zebra_at_a_way_end_spills_only_onto_the_continuation() {
    let roads = [
        street(
            vec![Vec2::ZERO, Vec2::new(99.0, 0.0), Vec2::new(100.0, 0.0)],
            8.0,
        ),
        street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 8.0),
        street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 8.0),
    ];
    let zebra = RoadNode {
        pos: Vec2::new(99.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
        osm_id: None,
    };
    let spilled: Vec<usize> = crossing_breaks(&roads, &RoadNodes::new(&roads), &[zebra])
        .into_iter()
        .map(|(road, _)| road)
        .collect();
    assert_eq!(spilled, vec![0, 1]);
}

/// Карманы по тегу `street_side` с обеих сторон: асфальт за кромкой, тротуар
/// отодвинут за карман. Правило без тега ставит их редко
/// (`pockets::sparse_pockets`), а тут нужен карман наверняка.
#[test]
fn a_primary_gets_pockets_in_its_sidewalks() {
    let mut map = one_street();
    map.roads[0].highway = Highway::Primary;
    map.roads[0].parking = [KerbParking::Pocket; 2];
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.kerb_pockets, 2);
    let edge = 100.0 + 6.0 + POCKET_WIDTH;
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(roads.iter().any(|at| (at[1] - edge).abs() < 0.01));
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(sidewalks.iter().any(|at| at[1] > edge + 1.0));
}

/// Пин перед `Drawn::sidewalk_on`: карман по тегу со стороны без тротуара —
/// асфальт за кромкой есть, тротуара за ним нет.
#[test]
fn a_pocket_on_the_side_without_a_sidewalk_pushes_no_sidewalk() {
    let mut map = one_street();
    map.roads[0].highway = Highway::Primary;
    map.roads[0].parking = [KerbParking::Pocket; 2];
    map.roads[0].sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.kerb_pockets, 2);
    let edge = 6.0 + POCKET_WIDTH;
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(roads.iter().any(|at| (at[1] - (100.0 - edge)).abs() < 0.01));
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(sidewalks.iter().any(|at| at[1] > 100.0 + edge + 1.0));
    assert!(
        sidewalks.iter().all(|at| at[1] > 100.0 - 6.0 - 0.01),
        "справа по ходу тротуара нет — ни у ленты, ни у кармана"
    );
}

/// Зебра OSM у стыка двух way одной улицы рвёт карманы и на продолжении
/// (`pockets::crossing_breaks`) — счётчик карманов под пином.
#[test]
fn a_zebra_at_a_way_end_breaks_the_kerb_pockets_of_both_ways() {
    let primary = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        parking: [KerbParking::Pocket; 2],
        ..fixture::street(points, 14.0)
    };
    let roads = vec![
        primary(vec![
            Vec2::ZERO,
            Vec2::new(99.0, 0.0),
            Vec2::new(100.0, 0.0),
        ]),
        primary(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)]),
    ];
    let zebra = RoadNode {
        pos: Vec2::new(99.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
        osm_id: None,
    };
    let mut map = with_network(roads);
    assert_eq!(timeless(&map).kerb_pockets, 4);
    map.road_nodes.push(zebra);
    // карманы те же четыре: зебра их укорачивает, а не делит
    assert_eq!(timeless(&map).kerb_pockets, 4);
}
