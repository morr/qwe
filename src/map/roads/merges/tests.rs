use std::borrow::Cow;

use bevy::prelude::*;

use super::{Merge, Merges, merge_bands, merges};
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, RoadLine};
use crate::map::roads::corners::kerb_returns;
use crate::map::roads::network::pairs::Pairs;
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::shape::RoadShape;
use crate::map::roads::tapers::TAPER_PER_METER;

/// Полотно в `lanes` полос: ширина — как из сечения.
fn width(lanes: u8) -> f32 {
    lanes as f32 * 3.3 + 1.0
}

fn primary(points: Vec<Vec2>, lanes: u8, oneway: bool) -> RoadLine {
    RoadLine {
        highway: Highway::Primary,
        oneway,
        lanes: Some(lanes),
        ..street(points, width(lanes))
    }
}

/// Узел слияния: половины по три полосы с зазором в 3 м вдоль x сходятся в
/// него с запада, продолжение `street` уходит от него.
const HALF_LANES: u8 = 3;

fn node() -> Vec2 {
    Vec2::new(240.0, (width(HALF_LANES) + 3.0) / 2.0)
}

/// Южная половина едет на восток в узел, северная — из узла на запад; третья
/// дорога — `continuation`.
fn divided_into(continuation: RoadLine) -> Vec<RoadLine> {
    let apart = width(HALF_LANES) + 3.0;
    vec![
        primary(
            vec![Vec2::ZERO, Vec2::new(200.0, 0.0), node()],
            HALF_LANES,
            true,
        ),
        primary(
            vec![node(), Vec2::new(200.0, apart), Vec2::new(0.0, apart)],
            HALF_LANES,
            true,
        ),
        continuation,
    ]
}

/// Двусторонняя в четыре полосы на восток от узла.
fn two_way_east() -> RoadLine {
    primary(vec![node(), node() + Vec2::new(160.0, 0.0)], 4, false)
}

/// Пары и разведённые оси, как их отдаёт `axis::street_axes` без
/// сглаживания, и найденные по ним слияния.
fn found(roads: &[RoadLine]) -> (Merges, Pairs, Vec<Vec<Vec2>>) {
    let mut paths: Vec<Cow<[Vec2]>> = roads
        .iter()
        .map(|road| Cow::Borrowed(road.points.as_slice()))
        .collect();
    let nodes = RoadNodes::new(roads);
    let mut pairs = Pairs::new(roads, &paths, RoadShape::default().median_gap(), &[]);
    pairs.align(&mut paths, roads, &RoadNetwork::new(roads), &nodes);
    let paths: Vec<Vec<Vec2>> = paths.into_iter().map(Cow::into_owned).collect();
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    (merges(&drawn, &paths, &nodes, &pairs.runs), pairs, paths)
}

#[test]
fn halves_ending_on_a_two_way_street_of_their_class_are_a_merge() {
    let (merges, _, _) = found(&divided_into(two_way_east()));
    assert_eq!(
        merges.list,
        vec![Merge {
            node: node(),
            halves: [0, 1],
            street: 2,
            street_end: 0,
        }]
    );
    assert!(merges.is_merged(0, 1) && merges.is_merged(1, 0) && merges.is_merged(2, 0));
    assert!(!merges.is_merged(0, 0) && !merges.is_merged(2, 1));
}

#[test]
fn a_street_across_the_node_or_of_another_class_is_no_merge() {
    let across = primary(vec![node(), node() - Vec2::new(0.0, 120.0)], 4, false);
    let secondary = RoadLine {
        highway: Highway::Secondary,
        ..two_way_east()
    };
    let oneway = RoadLine {
        oneway: true,
        ..two_way_east()
    };
    for continuation in [across, secondary, oneway] {
        let (merges, _, _) = found(&divided_into(continuation.clone()));
        assert!(merges.list.is_empty(), "{:?}", continuation.points);
    }
}

#[test]
fn a_merge_node_gets_no_square_ends_and_no_outer_corners() {
    let roads = divided_into(two_way_east());
    let (merges, _, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let rounded: Vec<Option<&[Vec2]>> = paths.iter().map(|path| Some(path.as_slice())).collect();
    let nodes = RoadNodes::new(&roads);
    let returns = |merged: &dyn Fn(usize, usize) -> bool| {
        kerb_returns(
            &drawn,
            &rounded,
            &nodes,
            |_| None,
            |_, _| None,
            |_| [0.0; 2],
            merged,
            1.0,
        )
    };
    // как перекрёсток — торцы прямые (а где плечи разошлись шире
    // развёрнутого, ещё и наружный угол): это и был шип
    let junction = returns(&|_, _| false);
    assert!(junction.butt(0)[1] && junction.butt(1)[0] && junction.butt(2)[0]);
    let merge = returns(&|road, end| merges.is_merged(road, end));
    for road in 0..3 {
        assert_eq!(merge.butt(road), [false; 2], "{road}");
    }
    assert!(merge.roads.is_empty(), "{:?}", merge.roads);
}

#[test]
fn each_outer_kerb_runs_into_the_kerb_of_the_continuation() {
    let roads = divided_into(two_way_east());
    let (merges, pairs, paths) = found(&roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let left = |half: usize, partner: usize| {
        pairs.runs[half]
            .iter()
            .find(|run| run.partner == partner)
            .map(|run| run.left)
    };
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        left,
        |_| Some(3.0),
        TAPER_PER_METER,
    );
    assert_eq!(bands.len(), 2);
    let (wide, narrow) = (width(4) / 2.0, width(HALF_LANES) / 2.0);
    for band in &bands {
        let outer = &band.asphalt;
        // у узла — на полуширине продолжения
        assert!(
            (outer[0].distance(node()) - wide).abs() < 0.05,
            "{}",
            outer[0]
        );
        // наружу, от пары: южная половина — к югу, северная — к северу
        let south = band.half == 0;
        assert_eq!(outer[0].y < node().y, south, "{}", outer[0]);
        // клин — ручка на метр разницы ширин
        let length = 2.0 * (wide - narrow) * TAPER_PER_METER;
        assert!((band.length - length).abs() < 1e-3, "{}", band.length);
        // и тротуар за асфальтом
        let sidewalk = band.sidewalk.as_ref().unwrap();
        assert!((sidewalk[0].distance(node()) - wide - 3.0).abs() < 0.05);
    }
    // за клином кромка — своя: полуширина от оси половины
    let far = bands[0].asphalt.len() / 2 - 1;
    let path = &paths[0];
    let edge = bands[0].asphalt[far];
    let axis = crate::map::meshing::distance_to_path(edge, path);
    assert!((axis - narrow).abs() < 0.05, "{axis}");
}

#[test]
fn a_continuation_no_wider_than_a_half_needs_no_band() {
    let roads = divided_into(primary(
        vec![node(), node() + Vec2::new(160.0, 0.0)],
        2,
        false,
    ));
    let (merges, pairs, paths) = found(&roads);
    assert_eq!(merges.list.len(), 1);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let bands = merge_bands(
        &merges.list[0],
        &drawn,
        &paths,
        |half, partner| {
            pairs.runs[half]
                .iter()
                .find(|run| run.partner == partner)
                .map(|run| run.left)
        },
        |_| None,
        TAPER_PER_METER,
    );
    assert!(bands.is_empty());
}
