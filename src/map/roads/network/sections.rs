//! **Сечение** участка улицы — сколько на нём полос; ширина дороги —
//! следствие сечения: `полосы × ширина полосы + кромка с двух сторон`.
//!
//! До этого ширина бралась по классу (`primary` 16 м, `residential` 8 м), а
//! полосы — по тегу, и полоса выходила то 2.5 м, то 5: однополосная
//! односторонняя половина проспекта рисовалась те же 16 м, что и
//! четырёхполосный двусторонний участок, и пара половин читалась дорогой в
//! 32 м. Теперь полоса одна по городу — [`shape::lane_width`] на улице (ручка
//! `Lane width`, 3.3 м по умолчанию), на [`SERVICE_LANE_NARROWING`] уже в
//! проезде, — и шире дорога становится только
//! числом полос.
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

use super::streets::RoadNetwork;
use crate::map::osm::model::polyline_length;
use crate::map::osm::{Highway, MapData, RoadLine};
use crate::map::roads::shape;

/// Насколько полоса дворового проезда уже полосы улицы, м: 3.0 против 3.3.
/// Ширина полосы улицы — одна на весь город, ручка `Lane width`
/// ([`shape::lane_width`]).
const SERVICE_LANE_NARROWING: f32 = 0.3;
/// Кромка проезжей части с каждой стороны, м: лоток у бордюра, по которому
/// не едут.
pub const EDGE_WIDTH: f32 = 0.5;
/// Самый длинный участок, который ещё считается одиночным скачком числа
/// полос, м.
pub const SPIKE_MAX_LENGTH: f32 = 60.0;

/// Ширина полосы по классу; `None` — у дорожки сечения нет.
pub fn lane_width(highway: Highway) -> Option<f32> {
    match highway {
        Highway::Path => None,
        Highway::Service => Some(shape::lane_width() - SERVICE_LANE_NARROWING),
        _ => Some(shape::lane_width()),
    }
}

/// Число полос по классу, когда ни тег, ни соседи по улице не подсказали.
/// Одностороннему — половина двустороннего, но не меньше одной полосы.
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
    if oneway {
        (two_way / 2).max(1)
    } else {
        two_way
    }
}

/// Ширина проезжей части из сечения; `None` — у дорожки сечения нет.
pub fn section_width(highway: Highway, lanes: u8) -> Option<f32> {
    lane_width(highway).map(|lane| f32::from(lanes) * lane + 2.0 * EDGE_WIDTH)
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
/// ширину. Сеть остаётся в [`MapData::network`].
pub fn apply(map: &mut MapData) -> SectionReport {
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
        .map(|road| lane_width(road.highway).and(road.lanes))
        .collect();
    report.tagged = lanes.iter().flatten().count();
    for street in &network.streets {
        let members: Vec<usize> = street.ways.iter().map(|way| way.road).collect();
        report.from_street += fill_from_street(&members, roads, &mut lanes);
        report.spikes += cut_spikes(&members, roads, &mut lanes);
    }

    for (road, lanes) in map.roads.iter_mut().zip(&lanes) {
        if lane_width(road.highway).is_none() {
            continue;
        }
        let lanes = lanes.unwrap_or_else(|| {
            report.by_class += 1;
            default_lanes(road.highway, road.oneway)
        });
        road.lanes = Some(lanes);
        if let Some(width) = section_width(road.highway, lanes) {
            road.width = width;
        }
    }
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
        let report = apply(&mut map);
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
