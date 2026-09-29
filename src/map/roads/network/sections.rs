//! **Сечение** участка улицы — сколько на нём полос; ширина дороги —
//! следствие сечения: `полосы × ширина полосы + кромка с двух сторон`.
//!
//! До этого ширина бралась по классу (`primary` 16 м, `residential` 8 м), а
//! полосы — по тегу, и полоса выходила то 2.5 м, то 5: однополосная
//! односторонняя половина проспекта рисовалась те же 16 м, что и
//! четырёхполосный двусторонний участок, и пара половин читалась дорогой в
//! 32 м. Теперь полоса одна по городу — ширина полосы улицы (ручка
//! `Lane width`, 3.3 м по умолчанию) на улице, на [`SERVICE_LANE_NARROWING`]
//! уже в проезде, — и шире дорога становится только числом полос. Ширину
//! полосы улицы проход получает **аргументом** — от разбора, который получил
//! её так же; глобали рисования (`shape::lane_width()`) он не читает.
//!
//! Полосы участка: тег `lanes` (`lanes:forward` + `lanes:backward`, если
//! общего нет), без тега — от ближайшего по длине улицы участка с тегом
//! ([`super::streets`]), без такого — по классу ([`default_lanes`]). Потом
//! **одиночный скачок** — участок короче [`SPIKE_MAX_LENGTH`], у которого с
//! обеих сторон одно и то же число полос, а у него другое, — срезается до
//! соседей: 2→4→2 на полусотне метров — это расширение у светофора или
//! ошибка тега, и рисовать его двумя ступеньками хуже, чем не рисовать.
//!
//! **Единственный этап переработки дорог, который двигает модель**:
//! `RoadLine::width` пересчитывается здесь, на разборе, потому что её читают
//! проходы, идущие следом, — дома отодвигаются от тротуаров, стоянки
//! подтягиваются к дорогам, — а за ними бордюры мостов и машины.

use std::time::Duration;

use bevy::prelude::*;

use super::streets::RoadNetwork;
use crate::map::along::{arclengths, place_on_path};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{Highway, MapData, RoadLine};
use crate::map::shapes::is_ring;

/// Насколько полоса дворового проезда уже полосы улицы, м: 3.0 против 3.3.
/// Ширина полосы улицы — одна на весь город, ручка `Lane width`
/// ([`RoadShape::lane_width`](crate::map::roads::shape::RoadShape::lane_width)).
const SERVICE_LANE_NARROWING: f32 = 0.3;
/// Кромка проезжей части с каждой стороны, м: лоток у бордюра, по которому
/// не едут.
pub const EDGE_WIDTH: f32 = 0.5;
/// Самый длинный участок, который ещё считается одиночным скачком числа
/// полос, м.
pub const SPIKE_MAX_LENGTH: f32 = 60.0;

/// Ширина полосы по классу при ширине полосы улицы `street_lane`; `None` — у
/// дорожки сечения нет.
pub fn lane_width(highway: Highway, street_lane: f32) -> Option<f32> {
    match highway {
        Highway::Path => None,
        Highway::Service => Some(street_lane - SERVICE_LANE_NARROWING),
        _ => Some(street_lane),
    }
}

/// Число полос по классу, когда ни тег, ни соседи по улице не подсказали.
/// Одностороннему — половина двустороннего, но не меньше одной полосы;
/// кроме `tertiary`: одностороннее полотно такой улицы — то же полотно, что
/// у двусторонней, только ехать по нему в одну сторону (центр Ростова,
/// Рязани, Калуги — две-три полосы), и одна полоса в 4.3 м читалась
/// проездом рядом с тротуаром втрое шире. Половина разделённой `tertiary`
/// тоже встаёт в две: бульвар 2 + 2 правдоподобнее, чем 1 + 1.
pub fn default_lanes(highway: Highway, oneway: bool) -> u8 {
    let two_way = match highway {
        Highway::Motorway | Highway::Trunk | Highway::Primary | Highway::Secondary => 4,
        Highway::Service | Highway::Path => 1,
        Highway::MotorwayLink
        | Highway::TrunkLink
        | Highway::PrimaryLink
        | Highway::SecondaryLink
        | Highway::TertiaryLink
        | Highway::Tertiary
        | Highway::Residential
        | Highway::Unclassified
        | Highway::LivingStreet => 2,
    };
    if !oneway || highway == Highway::Tertiary {
        two_way
    } else {
        (two_way / 2).max(1)
    }
}

/// Кольцо шире этого радиуса, м, без тега `lanes` получает не меньше
/// [`WIDE_RING_LANES`] полос: по большому кольцу едут в два ряда (Рязань,
/// площадь Мичурина, — радиус 52 м). Кольцо в парке в 20 м (Рязань, витрина
/// 05) остаётся в одну полосу, как у Яндекса.
const WIDE_RING_RADIUS: f32 = 30.0;
const WIDE_RING_LANES: u8 = 2;

/// Число полос дороги без тега и без соседа с тегом: по классу
/// ([`default_lanes`]), а на большом кольце — не меньше двух.
fn inferred_lanes(road: &RoadLine) -> u8 {
    let lanes = default_lanes(road.highway, road.oneway);
    let wide_ring = road.highway != Highway::Service
        && road.is_roundabout()
        && ring_radius(&road.points).is_some_and(|radius| radius >= WIDE_RING_RADIUS);
    if wide_ring {
        lanes.max(WIDE_RING_LANES)
    } else {
        lanes
    }
}

/// Радиус кольца по его way, м: у замкнутого — по длине окружности, у дуги —
/// окружность через её концы и середину. `None` — дуга прямая или из двух
/// точек.
fn ring_radius(points: &[Vec2]) -> Option<f32> {
    if is_ring(points) {
        return Some(polyline_length(points) / std::f32::consts::TAU);
    }
    if points.len() < 3 {
        return None;
    }
    let (a, c) = (points[0], points[points.len() - 1]);
    // середина — по длине дуги, а не по номеру вершины
    let (along, length) = arclengths(points);
    let (b, _) = place_on_path(points, &along, length / 2.0)?;
    let (ab, bc, ca) = (a.distance(b), b.distance(c), c.distance(a));
    let twice_area = (b - a).perp_dot(c - a).abs();
    (twice_area > 1e-3).then(|| ab * bc * ca / (2.0 * twice_area))
}

/// Ширина проезжей части из сечения при ширине полосы улицы `street_lane`;
/// `None` — у дорожки сечения нет.
pub fn section_width(highway: Highway, lanes: u8, street_lane: f32) -> Option<f32> {
    lane_width(highway, street_lane).map(|lane| f32::from(lanes) * lane + 2.0 * EDGE_WIDTH)
}

/// Что сделал проход — строкой `osm parse:`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SectionReport {
    /// Улиц всего и из них склеенных из двух ways и больше.
    pub streets: usize,
    pub glued: usize,
    /// Участков с сечением: по тегу, от соседа по улице, по классу.
    pub tagged: usize,
    pub from_street: usize,
    pub by_class: usize,
    /// Одиночных скачков, срезанных до соседей.
    pub spikes: usize,
    pub elapsed: Duration,
}

impl std::fmt::Display for SectionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let SectionReport {
            streets,
            glued,
            tagged,
            from_street,
            by_class,
            spikes,
            elapsed,
        } = *self;
        write!(
            f,
            "osm parse: {streets} streets ({glued} glued from several ways), lanes {tagged} \
             tagged / {from_street} from the street / {by_class} by class, {spikes} lane \
             spikes cut, in {elapsed:?}"
        )
    }
}

/// Склеить улицы, вывести сечение каждого участка и пересчитать по нему
/// ширину при ширине полосы улицы `street_lane`. Сеть остаётся в
/// [`MapData::network`].
pub fn apply(map: &mut MapData, street_lane: f32) -> SectionReport {
    let started = std::time::Instant::now();
    let network = RoadNetwork::new(&map.roads);
    let roads = &map.roads;
    let mut report = SectionReport {
        streets: network.streets.len(),
        glued: network.glued(),
        ..Default::default()
    };

    let mut lanes: Vec<Option<u8>> = roads
        .iter()
        .map(|road| lane_width(road.highway, street_lane).and(road.lanes))
        .collect();
    report.tagged = lanes.iter().flatten().count();
    for street in &network.streets {
        let members: Vec<usize> = street.ways.iter().map(|way| way.road).collect();
        report.from_street += fill_from_street(&members, roads, &mut lanes);
        report.spikes += cut_spikes(&members, roads, &mut lanes);
    }

    for (road, lanes) in map.roads.iter_mut().zip(&lanes) {
        if lane_width(road.highway, street_lane).is_none() {
            continue;
        }
        let lanes = lanes.unwrap_or_else(|| {
            report.by_class += 1;
            inferred_lanes(road)
        });
        road.lanes = Some(lanes);
        if let Some(width) = section_width(road.highway, lanes, street_lane) {
            road.width = width;
        }
    }
    settle_splits(&network, &mut map.roads);
    map.network = network;
    report.elapsed = started.elapsed();
    report
}

/// Участкам без тега — число полос ближайшего по длине улицы участка с
/// тегом. Расстояние — между серединами участков вдоль улицы. Возвращает,
/// скольким участкам досталось.
fn fill_from_street(members: &[usize], roads: &[RoadLine], lanes: &mut [Option<u8>]) -> usize {
    let lengths: Vec<f32> = members
        .iter()
        .map(|&road| polyline_length(&roads[road].points))
        .collect();
    // середина участка вдоль улицы
    let mut middles = Vec::with_capacity(members.len());
    let mut run = 0.0;
    for length in &lengths {
        middles.push(run + length / 2.0);
        run += length;
    }
    let tagged: Vec<(f32, u8)> = members
        .iter()
        .zip(&middles)
        .filter_map(|(&road, &at)| lanes[road].map(|lanes| (at, lanes)))
        .collect();
    if tagged.is_empty() {
        return 0;
    }
    let mut filled = 0;
    for (&road, &at) in members.iter().zip(&middles) {
        if lanes[road].is_some() {
            continue;
        }
        let nearest = tagged
            .iter()
            .min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs()))
            .map(|&(_, lanes)| lanes);
        lanes[road] = nearest;
        filled += 1;
    }
    filled
}

/// Деление полос двусторонней по потокам ([`RoadLine::lanes_backward`]) —
/// после того, как число полос улеглось. Тег, не сходящийся с итоговым
/// числом (срезанный скачок, `lanes:backward` больше `lanes`), снимается;
/// участок без деления берёт его у ближайшего участка своей улицы с тем же
/// числом полос — по ходу улицы, так что встречно нарисованный way получает
/// его зеркально. Иначе на шве с участком без тега осевая нечётной улицы
/// прыгала бы на полполосы (Ростов, Текучёва: `lanes=5, lanes:forward=3` и
/// соседний кусок `lanes=6`, срезанный до пяти).
fn settle_splits(network: &RoadNetwork, roads: &mut [RoadLine]) {
    for road in roads.iter_mut() {
        let valid = !road.oneway
            && road
                .lanes
                .zip(road.lanes_backward)
                .is_some_and(|(lanes, back)| back > 0 && back < lanes);
        if !valid {
            road.lanes_backward = None;
        }
    }
    for street in &network.streets {
        // каждый участок и его середина вдоль улицы
        let mut run = 0.0;
        let mut places = Vec::with_capacity(street.ways.len());
        for way in &street.ways {
            let length = polyline_length(&roads[way.road].points);
            places.push((*way, run + length / 2.0));
            run += length;
        }
        // (середина, полос, против хода улицы)
        let known: Vec<(f32, u8, u8)> = places
            .iter()
            .filter_map(|&(way, at)| {
                let road = &roads[way.road];
                let (lanes, back) = (road.lanes?, road.lanes_backward?);
                Some((at, lanes, if way.reversed { lanes - back } else { back }))
            })
            .collect();
        if known.is_empty() {
            continue;
        }
        for &(way, at) in &places {
            let road = &roads[way.road];
            if road.oneway || road.lanes_backward.is_some() {
                continue;
            }
            let Some(lanes) = road.lanes else { continue };
            let nearest = known
                .iter()
                .filter(|&&(_, count, _)| count == lanes)
                .min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs()));
            if let Some(&(_, _, back)) = nearest {
                roads[way.road].lanes_backward =
                    Some(if way.reversed { lanes - back } else { back });
            }
        }
    }
}

/// Срезать одиночные скачки числа полос до соседей. Возвращает, сколько
/// срезано.
fn cut_spikes(members: &[usize], roads: &[RoadLine], lanes: &mut [Option<u8>]) -> usize {
    // улица серией участков с одинаковым числом полос: (полосы, длина, участки)
    let mut runs: Vec<(Option<u8>, f32, Vec<usize>)> = Vec::new();
    for &road in members {
        let length = polyline_length(&roads[road].points);
        match runs.last_mut() {
            Some((value, total, roads)) if *value == lanes[road] => {
                *total += length;
                roads.push(road);
            }
            _ => runs.push((lanes[road], length, vec![road])),
        }
    }
    let mut cut = 0;
    for index in 1..runs.len().saturating_sub(1) {
        let (before, after) = (runs[index - 1].0, runs[index + 1].0);
        let (value, length, _) = &runs[index];
        if before.is_some() && before == after && *value != before && *length < SPIKE_MAX_LENGTH {
            for &road in &runs[index].2 {
                lanes[road] = before;
            }
            runs[index].0 = before;
            cut += 1;
        }
    }
    cut
}

#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    use super::*;
    use crate::map::osm::fixture::street;
    use crate::map::roads::shape::LANE_WIDTH_DEFAULT;

    fn piece(from: f32, to: f32, lanes: Option<u8>) -> RoadLine {
        let mut road = street(vec![Vec2::new(from, 0.0), Vec2::new(to, 0.0)], 8.0);
        road.lanes = lanes;
        road
    }

    fn sections(roads: Vec<RoadLine>) -> (MapData, SectionReport) {
        let mut map = MapData {
            roads,
            ..Default::default()
        };
        let report = apply(&mut map, LANE_WIDTH_DEFAULT);
        (map, report)
    }

    #[test]
    fn width_follows_the_lanes() {
        let (map, _) = sections(vec![piece(0.0, 100.0, Some(2)), {
            let mut half = piece(0.0, 100.0, Some(3));
            half.points = vec![Vec2::new(0.0, 40.0), Vec2::new(100.0, 40.0)];
            half.oneway = true;
            half
        }]);
        assert!((map.roads[0].width - 7.6).abs() < 1e-4);
        assert!((map.roads[1].width - 10.9).abs() < 1e-4);
    }

    /// Деление потоков — от соседа по улице с тем же числом полос, зеркально
    /// у встречно нарисованного way (Ростов, Текучёва); у соседа с другим
    /// числом полос — не берётся, а тег, что с числом не сходится, снимается.
    #[test]
    fn an_untagged_piece_takes_the_split_of_its_street() {
        let tagged = RoadLine {
            lanes_backward: Some(2),
            ..piece(0.0, 100.0, Some(5))
        };
        // тот же поток, но way нарисован навстречу улице
        let mut reversed = piece(100.0, 160.0, Some(5));
        reversed.points.reverse();
        let other = piece(160.0, 300.0, Some(4));
        let wrong = RoadLine {
            lanes_backward: Some(6),
            ..piece(300.0, 400.0, Some(4))
        };
        let (map, _) = sections(vec![tagged, reversed, other, wrong]);
        assert_eq!(map.roads[0].lanes_backward, Some(2));
        assert_eq!(map.roads[1].lanes_backward, Some(3), "зеркально");
        assert_eq!(
            map.roads[2].lanes_backward, None,
            "у четырёх полос деления нет"
        );
        assert_eq!(map.roads[3].lanes_backward, None, "шесть назад из четырёх");
    }

    #[test]
    fn an_untagged_piece_takes_the_lanes_of_its_street() {
        let (map, report) = sections(vec![
            piece(0.0, 100.0, Some(4)),
            piece(100.0, 150.0, None),
            piece(150.0, 400.0, Some(2)),
        ]);
        // середина второго участка ближе к первому, чем к третьему
        assert_eq!(map.roads[1].lanes, Some(4));
        assert_eq!(report.from_street, 1);
        assert_eq!(report.by_class, 0);
    }

    #[test]
    fn an_untagged_street_falls_back_to_its_class() {
        let (map, report) = sections(vec![piece(0.0, 100.0, None)]);
        assert_eq!(map.roads[0].lanes, Some(2));
        assert_eq!(report.by_class, 1);
    }

    #[test]
    fn a_one_way_tertiary_keeps_the_lanes_of_a_two_way_one() {
        let one_way = |highway: Highway| RoadLine {
            highway,
            oneway: true,
            ..piece(0.0, 100.0, None)
        };
        let (map, _) = sections(vec![
            one_way(Highway::Tertiary),
            one_way(Highway::Residential),
            one_way(Highway::Secondary),
        ]);
        let lanes: Vec<Option<u8>> = map.roads.iter().map(|road| road.lanes).collect();
        assert_eq!(
            lanes,
            [Some(2), Some(1), Some(2)],
            "односторонняя tertiary — две, жилая — одна, половина secondary — две"
        );
    }

    #[test]
    fn a_wide_ring_without_the_tag_gets_two_lanes() {
        let ring = |radius: f32| {
            let mut points: Vec<Vec2> = (0..24)
                .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * radius)
                .collect();
            points.push(points[0]);
            RoadLine {
                highway: Highway::Unclassified,
                oneway: true,
                roundabout: true,
                points,
                ..piece(0.0, 1.0, None)
            }
        };
        // и дуга большого кольца — по окружности через её концы и середину
        let arc = RoadLine {
            points: (0..=6)
                .map(|step| Vec2::from_angle(step as f32 * 0.2) * 50.0 + Vec2::new(500.0, 0.0))
                .collect(),
            ..ring(50.0)
        };
        let (map, _) = sections(vec![ring(50.0), ring(20.0), arc]);
        let lanes: Vec<Option<u8>> = map.roads.iter().map(|road| road.lanes).collect();
        assert_eq!(lanes, [Some(2), Some(1), Some(2)]);
    }

    #[test]
    fn a_short_lane_spike_is_cut() {
        let (map, report) = sections(vec![
            piece(0.0, 200.0, Some(2)),
            piece(200.0, 240.0, Some(4)),
            piece(240.0, 400.0, Some(2)),
        ]);
        assert_eq!(map.roads[1].lanes, Some(2));
        assert_eq!(report.spikes, 1);
    }

    #[test]
    fn a_long_widening_stays() {
        let (map, report) = sections(vec![
            piece(0.0, 200.0, Some(2)),
            piece(200.0, 300.0, Some(4)),
            piece(300.0, 400.0, Some(2)),
        ]);
        assert_eq!(map.roads[1].lanes, Some(4));
        assert_eq!(report.spikes, 0);
    }

    #[test]
    fn a_footway_keeps_its_width_and_no_lanes() {
        let (map, _) = sections(vec![crate::map::osm::fixture::footway(vec![
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ])]);
        assert_eq!(map.roads[0].lanes, None);
        assert!((map.roads[0].width - 3.5).abs() < 1e-4);
    }
}
