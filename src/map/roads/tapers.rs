//! **Клинья** — переход между сечениями одной улицы
//! (`network::sections`): там, где у соседних по улице ways разное число
//! полос, более широкий начинается не с полной ширины, а с ширины соседа, и
//! расходится до своей на длине [`TAPER_PER_METER`] × разница ширин. До этого
//! ширина менялась ступенькой прямо в узле шва.
//!
//! Клин кладётся только в **чистом шве** — узле, где сходятся ровно два way
//! одной улицы. На перекрёстке ступенька тонет в асфальте узла, а скругления
//! бордюра (`roads/corners.rs`) считаются по полной ширине дороги, и клин,
//! начатый прямо от угла, разошёлся бы с дугой бордюра.
//!
//! Только картинка: `RoadLine` не меняется, навмеш и разбор видят ширину
//! участка как есть. Машины — тоже, но ряд у бордюра на длину клина
//! прерывается ([`car_clearings`]): иначе он встал бы на тротуар.

use bevy::prelude::*;

use super::network::{RoadNetwork, RoadNodes};
use crate::map::meshing::Break;
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;

/// Длина клина на метр разницы ширин, м: полоса в 3.3 м появляется за
/// 33 м — как отгон уширения на городской улице.
pub const TAPER_PER_METER: f32 = 10.0;
/// Разница ширин, которую клин ещё не выравнивает, м: сантиметры не видны.
const TAPER_MIN_STEP: f32 = 0.1;
/// Самый короткий клин, м: короче — та же ступенька, только скошенная.
const TAPER_MIN_LENGTH: f32 = 1.0;
/// Доля нарисованной длины way, которую может занять один клин: у way бывают
/// клинья с обоих концов, и между ними должна остаться полная ширина.
const TAPER_MAX_SHARE: f32 = 0.45;

/// Клин у одного торца way: длина по разнице ширин (до обрезки по длине
/// нарисованного пути) и дорога, с ширины которой он начинается.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taper {
    pub length: f32,
    pub narrow: usize,
}

/// Клинья карты: у каждой дороги — по торцам `[начало, конец]`.
#[derive(Debug, Default)]
pub struct Tapers {
    ends: Vec<[Option<Taper>; 2]>,
    pub count: usize,
}

impl Tapers {
    /// Клинья по стыкам улиц сети. `drawn` — дороги так, как они рисуются
    /// (переезд через тротуар уже асфальтом), по тем же индексам, что у сети.
    pub fn new(drawn: &[&RoadLine], network: &RoadNetwork, nodes: &RoadNodes) -> Self {
        let mut tapers = Self {
            ends: vec![[None; 2]; drawn.len()],
            count: 0,
        };
        if drawn.is_empty() || !network.covers(drawn.len()) {
            return tapers;
        }
        for (a, b) in network.joints() {
            let (first, second) = (drawn[a.road], drawn[b.road]);
            if a.road == b.road || !takes_taper(first) || !takes_taper(second) {
                continue;
            }
            let node = if a.reversed {
                first.points[0]
            } else {
                first.points[first.points.len() - 1]
            };
            if nodes.roads_at(node).len() != 2 {
                continue;
            }
            let step = (first.width - second.width).abs();
            if step < TAPER_MIN_STEP {
                continue;
            }
            // торец широкого way, которым он стоит в шве
            let (wide, end, narrow) = if first.width > second.width {
                (a.road, !a.reversed, b.road)
            } else {
                (b.road, b.reversed, a.road)
            };
            tapers.ends[wide][usize::from(end)] = Some(Taper {
                length: step * TAPER_PER_METER,
                narrow,
            });
            tapers.count += 1;
        }
        tapers
    }

    /// Клинья у торцов дороги `[начало, конец]`.
    pub fn at(&self, road: usize) -> [Option<Taper>; 2] {
        self.ends.get(road).copied().unwrap_or([None; 2])
    }
}

/// Разрывы ряда припаркованных машин (`map::cars`) на клиньях: ряд стоит на
/// полуширине участка, а в клине бордюр ближе к оси — машина встала бы на
/// тротуар. Разрыв — в узле шва, длиной в клин, по дороге, на которой клин
/// лежит, — `(дорога, разрыв)`.
pub fn car_clearings(roads: &[RoadLine], network: &RoadNetwork) -> Vec<(usize, Break)> {
    let nodes = RoadNodes::new(roads);
    let drawn: Vec<&RoadLine> = roads.iter().collect();
    let tapers = Tapers::new(&drawn, network, &nodes);
    let mut clearings = Vec::with_capacity(tapers.count);
    for (road, ends) in tapers.ends.iter().enumerate() {
        let points = &roads[road].points;
        let share = polyline_length(points) * TAPER_MAX_SHARE;
        for (end, taper) in ends.iter().enumerate() {
            let Some(taper) = taper else { continue };
            let at = if end == 1 {
                points[points.len() - 1]
            } else {
                points[0]
            };
            clearings.push((
                road,
                Break {
                    at,
                    reach: taper.length.min(share),
                },
            ));
        }
    }
    clearings
}

/// Ширину клином выравнивает только ровная проезжая лента: мост рисуется
/// другим слоем со своим бордюром, арка приколота к стене дома.
fn takes_taper(road: &RoadLine) -> bool {
    !road.bridge && !road.passage && road.lanes.is_some()
}

/// Нарисованный путь, разрезанный под клинья: `(клин у начала, середина, клин
/// у конца)`. Оба клина смотрят **от узла шва** — от узкого конца к широкому.
/// Длина клина обрезается до [`TAPER_MAX_SHARE`] пути; клин короче
/// [`TAPER_MIN_LENGTH`] не кладётся, и его конец остаётся в середине.
pub fn split(path: &[Vec2], lengths: [Option<f32>; 2]) -> [Option<Vec<Vec2>>; 3] {
    let total = polyline_length(path);
    let [head, tail] = fit(total, lengths);
    let from = head.unwrap_or(0.0);
    let to = total - tail.unwrap_or(0.0);
    let head_path = head.map(|length| {
        let mut piece = cut(path, 0.0, length);
        piece.dedup();
        piece
    });
    let tail_path = tail.map(|length| {
        let mut piece = cut(path, total - length, total);
        piece.reverse();
        piece.dedup();
        piece
    });
    [head_path, Some(cut(path, from, to)), tail_path]
}

/// Длины клиньев `[у начала, у конца]` на пути длиной `total` — так, как их
/// нарежет [`split`]: не больше [`TAPER_MAX_SHARE`] пути, короче
/// [`TAPER_MIN_LENGTH`] — клина нет. Слой краски (`roads/paint.rs`) плывёт по
/// тем же длинам.
pub fn fit(total: f32, lengths: [Option<f32>; 2]) -> [Option<f32>; 2] {
    lengths.map(|length| {
        length
            .map(|length| length.min(total * TAPER_MAX_SHARE))
            .filter(|&length| length >= TAPER_MIN_LENGTH)
    })
}

/// Кусок ломаной между длинами дуги `from..to`. Им же режется тротуар у
/// разделённой улицы (`roads::push_sidewalk`).
pub(super) fn cut(path: &[Vec2], from: f32, to: f32) -> Vec<Vec2> {
    let mut piece = Vec::new();
    let mut run = 0.0;
    for segment in path.windows(2) {
        let length = segment[0].distance(segment[1]);
        let (start, end) = (run, run + length);
        run = end;
        if end < from || start > to || length <= 0.0 {
            continue;
        }
        let at =
            |along: f32| segment[0].lerp(segment[1], ((along - start) / length).clamp(0.0, 1.0));
        if piece.is_empty() {
            piece.push(at(from.max(start)));
        }
        piece.push(at(to.min(end)));
    }
    piece
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;

    fn seam(widths: [f32; 2], lanes: [u8; 2]) -> (Vec<RoadLine>, Tapers) {
        let mut roads = vec![
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], widths[0]),
            street(
                vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)],
                widths[1],
            ),
        ];
        for (road, lanes) in roads.iter_mut().zip(lanes) {
            road.lanes = Some(lanes);
        }
        let network = RoadNetwork::new(&roads);
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        let tapers = Tapers::new(&drawn, &network, &nodes);
        (roads, tapers)
    }

    #[test]
    fn the_wider_way_takes_the_taper_at_the_seam() {
        let (_, tapers) = seam([8.0, 14.2], [2, 4]);
        assert_eq!(tapers.count, 1);
        assert_eq!(tapers.at(0), [None, None]);
        let [start, end] = tapers.at(1);
        assert_eq!(end, None);
        let start = start.unwrap();
        assert_eq!(start.narrow, 0);
        assert!((start.length - 62.0).abs() < 1e-3);
    }

    #[test]
    fn equal_sections_need_no_taper() {
        let (_, tapers) = seam([8.0, 8.0], [2, 2]);
        assert_eq!(tapers.count, 0);
    }

    #[test]
    fn a_seam_on_a_junction_takes_no_taper() {
        let mut roads = vec![
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 14.2),
            street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 80.0)], 8.0),
        ];
        for road in &mut roads {
            road.lanes = Some(2);
        }
        let network = RoadNetwork::new(&roads);
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        assert_eq!(Tapers::new(&drawn, &network, &nodes).count, 0);
    }

    #[test]
    fn split_hands_out_both_tapers_from_the_seam_inwards() {
        let path = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
        let [head, middle, tail] = split(&path, [Some(20.0), Some(30.0)]);
        assert_eq!(head.unwrap(), vec![Vec2::ZERO, Vec2::new(20.0, 0.0)]);
        assert_eq!(
            middle.unwrap(),
            vec![Vec2::new(20.0, 0.0), Vec2::new(70.0, 0.0)]
        );
        assert_eq!(
            tail.unwrap(),
            vec![Vec2::new(100.0, 0.0), Vec2::new(70.0, 0.0)]
        );
    }

    #[test]
    fn a_taper_never_takes_more_than_its_share_of_a_short_way() {
        let path = [Vec2::ZERO, Vec2::new(40.0, 0.0), Vec2::new(40.0, 20.0)];
        let [head, _, tail] = split(&path, [Some(66.0), None]);
        assert!(tail.is_none());
        let head = head.unwrap();
        let length = crate::map::osm::model::polyline_length(&head);
        assert!((length - 60.0 * TAPER_MAX_SHARE).abs() < 1e-3, "{length}");
    }
}
