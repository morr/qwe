//! Машины вдоль дворовых проездов (`highway=service`).
//!
//! Ряд у бордюра ([`super::park_cars`]) стоит только вдоль улиц
//! ([`crate::map::roads::pockets::parkable`]), и двор многоэтажек без этого
//! слоя читался пустой серой петлёй на газоне — на аэрофото тульского двора
//! машины стоят вдоль каждого проезда. OSM про них не говорит ничего, так что
//! ряд **выводится**: детерминированно, от первой точки проезда, как и ряд
//! улицы.
//!
//! Чем он отличается от ряда у бордюра:
//! - **одна сторона** — правая по ходу точек: проезд в 4 м с машинами по обе
//!   стороны не проехать, а какая из сторон — всё равно, лишь бы одна и та же
//!   при каждой сборке;
//! - **наполовину на газоне**: бордюра у проезда нет, и машину ставят к краю
//!   асфальта так, что на нём остаётся полоса [`ON_ASPHALT`] — остальное на
//!   траве, как во дворе и бывает;
//! - **только среди многоэтажек**: плотность — доля между [`LOW_STOREYS`] и
//!   [`HIGH_STOREYS`] этажности квартала ([`Districts::storeys_at`]); в частном
//!   секторе машину ставят за забор, а не вдоль проезда, и там её нет вовсе;
//! - **место проверяется по соседям** ([`Blocked`]): проезд идёт в паре метров
//!   от фасада, по краю стоянки, вдоль дорожки, — и ряд, поставленный вслепую,
//!   лёг бы кузовом в дом или на чужую ленту. У улицы с этим справлялся
//!   бордюр, у проезда его нет.

use std::borrow::Cow;

use bevy::math::{Rot2, Vec2};

use super::body::{self, Car, CarShape};
use super::district::Districts;
use super::{
    BREAK_BEND_SLACK_HALF_WIDTHS, CAR_PITCH, DISTRICT_STEP, END_MARGIN, JUNCTION_CLEARANCE,
    PARK_SKEW_DEGREES, PARK_SLOP,
};
use crate::map::along::{arclengths, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::Break;
use crate::map::osm::model::{Highway, distance_to_segment, point_in_area, ring_bounds};
use crate::map::osm::{MapData, PolyArea, RoadLine};
use crate::map::roads::network::RoadNodes;
use crate::map::roads::pockets::RowBreaks;
use crate::map::seed::{Lcg, seed_from_point};

/// Проезд короче этого, м, — въезд к подъезду или в гараж: ряд на нём стоял бы
/// поперёк чужого выезда.
const MIN_DRIVE_LENGTH: f32 = 20.0;
/// Сколько ширины машины остаётся на асфальте проезда, м; остальное — на
/// газоне.
const ON_ASPHALT: f32 = 0.6;
/// Этажность квартала, ниже которой у проезда не стоят, и от которой ряд
/// занят в полную долю. Два этажа — частный сектор (`cars::district`
/// читает его так же), пять — панельная секция.
const LOW_STOREYS: f32 = 2.0;
const HIGH_STOREYS: f32 = 5.0;
/// Доля занятых мест у проезда относительно ползунка `CarStyle::occupancy`:
/// двор заставлен, но не сплошь — у подъездов и мусорных площадок не стоят.
const YARD_SHARE: f32 = 0.8;
/// Зазор от кузова до стены, края стоянки или чужой ленты, м.
const CLEARANCE: f32 = 0.4;
/// Клетка индекса соседей, м: двор, а не квартал.
const CELL: f32 = 40.0;

/// Проезд, вдоль которого во дворе ставят машины: `highway=service`, не
/// проезд стоянки (у неё свои места), не мост, не арка, не кольцо и не
/// короткий въезд.
pub(super) fn is_yard_drive(road: &RoadLine) -> bool {
    road.highway == Highway::Service
        && !road.parking_aisle
        && !road.bridge
        && !road.passage
        && !road.is_roundabout()
        && arclengths(&road.points).1 >= MIN_DRIVE_LENGTH
}

/// Чем занято место вокруг проезда: пятна, в которые машина не встаёт (дома,
/// стоянки, вода, площадки), и ленты других дорог.
pub(super) struct Blocked<'a> {
    areas: Vec<&'a PolyArea>,
    areas_by_cell: Grid<u32>,
    /// Звенья чужих лент: дорога, начало, конец и полуширина нарисованного.
    links: Vec<(usize, Vec2, Vec2, f32)>,
    links_by_cell: Grid<u32>,
}

impl<'a> Blocked<'a> {
    pub(super) fn new(map: &'a MapData) -> Self {
        let mut areas = Vec::new();
        let mut areas_by_cell = Grid::new(CELL);
        for area in map
            .buildings
            .iter()
            .chain(&map.parking)
            .chain(&map.water)
            .chain(&map.pitches)
        {
            if area.outer.is_empty() {
                continue;
            }
            let (min, max) = ring_bounds(&area.outer);
            areas_by_cell.insert(min, max, areas.len() as u32);
            areas.push(area);
        }
        let mut links = Vec::new();
        let mut links_by_cell = Grid::new(CELL);
        for (index, road) in map.roads.iter().enumerate() {
            let reach = road.sidewalk().mapped_edge(road.width / 2.0);
            for pair in road.points.windows(2) {
                links_by_cell.insert_segment(
                    pair[0],
                    pair[1],
                    reach + CLEARANCE,
                    links.len() as u32,
                );
                links.push((index, pair[0], pair[1], reach));
            }
        }
        Self {
            areas,
            areas_by_cell,
            links,
            links_by_cell,
        }
    }

    /// Свободна ли точка `at` от чужого: не внутри пятна и не ближе
    /// [`CLEARANCE`] к ленте другой дороги, чем её полуширина. Свои дороги
    /// `own` не в счёт — сам проезд, на котором машина стоит краем, и проезды,
    /// сходящиеся с ним торцом: OSM режет один проезд на несколько way, и у
    /// шва машина иначе упиралась бы в продолжение своей же ленты.
    fn free(&self, at: Vec2, own: &[usize]) -> bool {
        let areas_clear = self
            .areas_by_cell
            .at(at)
            .iter()
            .all(|&index| !point_in_area(at, self.areas[index as usize]));
        areas_clear
            && self.links_by_cell.at(at).iter().all(|&index| {
                let (road, from, to, reach) = self.links[index as usize];
                own.contains(&road) || distance_to_segment(at, from, to) >= reach + CLEARANCE
            })
    }

    /// Свободен ли кузов: его центр и четыре угла, раздутые на [`CLEARANCE`].
    fn fits(&self, at: Vec2, along: Vec2, shape: CarShape, own: &[usize]) -> bool {
        let half_length = along * (shape.length() / 2.0 + CLEARANCE);
        let half_width = along.perp() * (shape.width() / 2.0 + CLEARANCE);
        [
            at,
            at + half_length + half_width,
            at + half_length - half_width,
            at - half_length + half_width,
            at - half_length - half_width,
        ]
        .into_iter()
        .all(|point| self.free(point, own))
    }
}

/// Доля занятых мест у проезда в точке `point`: ноль в частном секторе и там,
/// где домов рядом нет, полная [`YARD_SHARE`] ползунка среди многоэтажек.
fn yard_fill(districts: &Districts, point: Vec2) -> f32 {
    districts.storeys_at(point).map_or(0.0, |storeys| {
        ((storeys - LOW_STOREYS) / (HIGH_STOREYS - LOW_STOREYS)).clamp(0.0, 1.0) * YARD_SHARE
    })
}

/// Ряды вдоль всех дворовых проездов карты. `junctions` и `axes` — по индексу
/// дороги во всём срезе, как у [`super::park_cars`].
pub(super) fn park_yards(
    roads: &[RoadLine],
    shared: &RoadNodes,
    junctions: &RowBreaks,
    axes: &[Cow<[Vec2]>],
    occupancy: f32,
    districts: &Districts,
    blocked: &Blocked,
) -> Vec<Car> {
    let mut cars = Vec::new();
    let mut own = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !is_yard_drive(road) {
            continue;
        }
        own.clear();
        own.push(index);
        for end in [road.points.first(), road.points.last()]
            .into_iter()
            .flatten()
        {
            own.extend(
                shared
                    .roads_at(*end)
                    .iter()
                    .filter(|&&other| roads[other].highway == Highway::Service),
            );
        }
        // свой поток ГПСЧ у проезда — от его первой точки, как у улицы
        let mut rng = Lcg::new(seed_from_point(
            road.points.first().copied().unwrap_or(Vec2::ZERO),
        ));
        park_drive(
            &mut cars,
            &axes[index],
            road.width / 2.0,
            &own,
            junctions.of(index),
            occupancy,
            districts,
            blocked,
            &mut rng,
        );
    }
    cars
}

/// Ряд вдоль правой стороны одного проезда — шагом [`CAR_PITCH`] по дуговой
/// координате, с теми же разрывами у узлов, что у ряда улицы.
#[allow(clippy::too_many_arguments)]
fn park_drive(
    cars: &mut Vec<Car>,
    points: &[Vec2],
    half_road: f32,
    own: &[usize],
    junctions: &[Break],
    occupancy: f32,
    districts: &Districts,
    blocked: &Blocked,
    rng: &mut Lcg,
) {
    let (along, total) = arclengths(points);
    if total <= 2.0 * END_MARGIN {
        return;
    }
    let mut last: Option<(Vec2, f32)> = None;
    let mut around: Option<(f32, f32)> = None;
    let mut step = END_MARGIN;
    while step <= total - END_MARGIN {
        let at = step;
        step += CAR_PITCH;
        let Some((point, direction)) = place_on_path(points, &along, at) else {
            continue;
        };
        let shape = CarShape::from_share(rng.next_f32());
        // правая сторона по ходу точек; машина — краем на асфальте
        let across = -direction.perp();
        let place = point + across * (half_road + shape.width() / 2.0 - ON_ASPHALT);
        let clear = |junction: &Break| {
            (place - junction.at).dot(direction).abs() - shape.length() / 2.0
                >= junction.reach + JUNCTION_CLEARANCE
                || place.distance(junction.at)
                    > junction.reach + JUNCTION_CLEARANCE + half_road * BREAK_BEND_SLACK_HALF_WIDTHS
        };
        if !junctions.iter().all(clear) {
            continue;
        }
        if last.is_some_and(|(previous, previous_length)| {
            previous.distance(place) < (previous_length + shape.length()) / 2.0
        }) {
            continue;
        }
        let fill = match around {
            Some((read_at, fill)) if at - read_at < DISTRICT_STEP => fill,
            _ => {
                let fill = yard_fill(districts, point);
                around = Some((at, fill));
                fill
            }
        };
        // бросок — раньше пробы: проба по соседям дороже всего остального, а
        // незанятому месту и частному сектору (доля ноль) она не нужна
        if rng.next_f32() >= (occupancy * fill).clamp(0.0, 1.0)
            || !blocked.fits(place, direction, shape, own)
        {
            continue;
        }
        last = Some((place, shape.length()));
        let skew = Rot2::degrees(rng.bell4() * PARK_SKEW_DEGREES);
        cars.push(Car {
            at: place + across * (rng.bell4() * PARK_SLOP),
            along: skew * direction,
            color: body::color_from_share(rng.next_f32()),
            shape,
        });
    }
}
