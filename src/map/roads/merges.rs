//! **Слияние** — узел, где обе половины разделённой улицы кончаются на одном
//! двустороннем way того же класса: разделённый проспект переходит в
//! обычную улицу (Демидовская Плотина у Карла Маркса, Тула, 3 + 3 полосы в
//! четыре). До этого узел рисовался перекрёстком: три плеча — значит прямые
//! торцы и наружные углы между ними (`roads/corners.rs`), а оси половин OSM
//! сводит в точку узла, и наружная кромка каждой половины у узла стояла на её
//! полуширине от него — ближе, чем кромка продолжения. Кромка шла уступом, и
//! наружный угол закрывал его острым веером.
//!
//! Здесь узел слияния — не перекрёсток: между его тремя плечами нет ни
//! торцов, ни углов, ленты кончаются круглыми торцами, как у продолжения
//! дороги, а наружная кромка каждой половины **сходится к кромке продолжения
//! своей стороны** полосой асфальта (и тротуара за ней) снаружи от ленты
//! половины ([`merge_bands`]). Ширина полосы у узла — разница полуширин
//! продолжения и половины, дальше она плавно (smoothstep) сходит на нет за
//! длину клина: ручка `Taper` × разница ширин, как у клина шва
//! (`roads/tapers.rs`), не больше [`MERGE_MAX_SHARE`] пути половины. У каждой
//! стороны своя разница, так что это клин «по кромкам»: где кромки на одной
//! прямой, его нет.
//!
//! Только картинка: `RoadLine` и оси не меняются.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use super::network::pairs::Pairs;
use super::network::{RoadNetwork, RoadNodes};
use super::paint::{MergeRamp, lane_frame};
use super::shape::lane_width;
use super::{is_carriageway, lane_count, smoothstep};
use crate::map::meshing::{LaneFrame, miter_offsets};
use crate::map::osm::RoadLine;
use crate::map::osm::model::{point_at_arc_length, polyline_length};
use crate::map::shapes::{Shape, oriented};

/// Косинус угла, в котором обе половины уходят от узла в одну сторону, а
/// продолжение — в обратную: 40°. Половины в OSM сходятся к узлу клином
/// градусов в 10–30, продолжение смотрит почти ровно назад.
const MERGE_ALIGN: f32 = 0.766;
/// То же для Y-развилки без пары ([`merges`]): 70°. Ветки въезда и съезда у
/// кольца расходятся к его узлам на 48–58° по хорде в двадцать метров (Тула,
/// южный подход и Болдина, R16).
const FORK_ALIGN: f32 = 0.342;
/// Направление плеча — хорда на столько метров от узла: первое звено OSM
/// бывает в полметра.
const ARM_REACH: f32 = 20.0;
/// Разница полуширин, которую слияние ещё не выравнивает, м.
const MERGE_MIN_STEP: f32 = 0.1;
/// Доля пути половины, которую может занять клин слияния.
const MERGE_MAX_SHARE: f32 = 0.6;
/// Шаг выборки пути половины под клином, м: кромка по smoothstep — кривая,
/// по вершинам OSM её не нарисовать.
const MERGE_STEP: f32 = 2.0;
/// На сколько полоса заходит под ленту половины, м.
const MERGE_OVERLAP: f32 = 0.05;
/// Насколько могут разойтись концы соседних ways улицы на нарисованных осях,
/// чтобы клин всё ещё шёл с одного на другой, м: разводка пары сдвигает
/// торец куска на метр, а сверяется только сосед по улице — спутать не с чем.
const SEAM_SLACK: f32 = 2.0;

/// Одно слияние: узел, половины и продолжение.
#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    pub node: Vec2,
    /// Половины: та, что в узел въезжает (узел — её конец), и та, что из него
    /// выезжает (узел — её начало).
    pub halves: [usize; 2],
    /// Двусторонний way, продолжающий улицу за узлом.
    pub street: usize,
    /// Торец продолжения в узле: `0` — начало, `1` — конец.
    pub street_end: usize,
    /// В узле нет других проезжих частей: он и для краски не перекрёсток.
    /// Слияние на перекрёстке (Демидовская Плотина, пять дорог) рвёт
    /// разметку, как любой узел.
    pub pure: bool,
}

impl Merge {
    /// Три дороги слияния: половины и продолжение.
    pub fn roads(&self) -> [usize; 3] {
        [self.halves[0], self.halves[1], self.street]
    }
}

/// Все слияния: торцы плеч (`[дорога][торец]`) и сами узлы.
#[derive(Default)]
pub struct Merges {
    pub list: Vec<Merge>,
    ends: Vec<[bool; 2]>,
}

impl Merges {
    /// Торец `end` дороги `road` — плечо слияния.
    pub fn is_merged(&self, road: usize, end: usize) -> bool {
        self.ends.get(road).is_some_and(|ends| ends[end])
    }

    /// Точка — узел слияния без других проезжих частей ([`Merge::pure`]).
    pub fn is_pure_node(&self, point: Vec2) -> bool {
        self.list
            .iter()
            .any(|merge| merge.pure && merge.node.distance(point) < NODE_SLACK)
    }
}

/// Насколько точка разрыва может отстоять от узла слияния, чтобы считаться
/// им, м: разрывы кладутся по точке узла, но через ключ в 5 см.
const NODE_SLACK: f32 = 0.1;

/// Слияния по **нарисованным** осям `paths`: в узле кончаются две половины
/// одной пары, одна въезжает, другая выезжает, обе уходят от узла в одну
/// сторону, а двусторонний way того же `Highway` — в обратную. Мосты и арки
/// (`carves_navmesh`) не участвуют.
///
/// Пара — по улицам (`network`), а не по way у узла ([`Pairs::is_paired`] —
/// по кускам пар дороги): OSM режет половину у узла на короткие ways в 16–22 м, и на
/// таком куске пары не набирается — она лежит на соседнем way той же улицы.
pub fn merges(
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    nodes: &RoadNodes,
    pairs: &Pairs,
    network: &RoadNetwork,
) -> Merges {
    let mut found = Merges {
        list: Vec::new(),
        ends: vec![[false; 2]; roads.len()],
    };
    for (half, road) in roads.iter().enumerate() {
        // каждое слияние — от въезжающей половины, один раз
        let path = paths[half].as_ref();
        if !road.oneway || road.carves_navmesh() || path.len() < 2 {
            continue;
        }
        let node = path[path.len() - 1];
        let at_node = nodes.roads_at(node);
        let into = away(path, true);
        // В узле нет проезжих частей, кроме этих трёх.
        let pure_with = |partner: usize, street: usize| {
            at_node.iter().all(|&other| {
                [half, partner, street].contains(&other) || !is_carriageway(roads[other])
            })
        };
        // продолжение: двусторонний way того же класса в обратную сторону; им
        // бывает и мост (Орёл, Р-119 у кольца, R16) — ветки сходятся на его
        // торце
        let street_for = |partner: usize| {
            let out = away(paths[partner].as_ref(), false);
            let mean = (into + out).normalize_or_zero();
            at_node.iter().copied().find_map(|other| {
                let candidate = roads[other];
                let path = paths[other].as_ref();
                if candidate.oneway || candidate.highway != road.highway || path.len() < 2 {
                    return None;
                }
                let end = if path[0] == node {
                    0
                } else if path[path.len() - 1] == node {
                    1
                } else {
                    return None;
                };
                (away(path, end == 1).dot(mean) < -MERGE_ALIGN).then_some((other, end))
            })
        };
        // Половины пары — или **Y-развилка** (R16): двусторонний way делится
        // на въезд и съезд без разделительной, пары на коротких расходящихся
        // ветках не набирается. Развилку берём только в чистом узле — угол
        // сетки односторонних улиц с двусторонней рядом остаётся перекрёстком
        // — и ветки её расходятся шире половин пары: до [`FORK_ALIGN`].
        let found_merge = at_node.iter().copied().find_map(|other| {
            let path = paths[other].as_ref();
            if other == half
                || !roads[other].oneway
                || roads[other].carves_navmesh()
                || roads[other].highway != road.highway
                || path.len() < 2
                || path[0] != node
            {
                return None;
            }
            let alignment = away(path, false).dot(into);
            let is_pair = paired(half, other, pairs, network);
            if alignment <= if is_pair { MERGE_ALIGN } else { FORK_ALIGN } {
                return None;
            }
            let (street, end) = street_for(other)?;
            // развилка грунтовки асфальтом не мостится: клин ветки ложился
            // асфальтовым языком на грунт (Тула, 371, 4078)
            let unpaved = [half, other, street]
                .iter()
                .any(|&road| roads[road].is_unpaved_street());
            let pure = pure_with(other, street);
            let accepted = if is_pair {
                !roads[street].carves_navmesh() || pure
            } else {
                !unpaved && pure
            };
            accepted.then_some((other, street, end))
        });
        let Some((partner, street, street_end)) = found_merge else {
            continue;
        };
        found.ends[half][1] = true;
        found.ends[partner][0] = true;
        found.ends[street][street_end] = true;
        let pure = pure_with(partner, street);
        found.list.push(Merge {
            node,
            halves: [half, partner],
            street,
            street_end,
            pure,
        });
    }
    found
}

/// Лежат ли дороги `half` и `other` в одной паре: сами или любые ways их
/// улиц.
fn paired(half: usize, other: usize, pairs: &Pairs, network: &RoadNetwork) -> bool {
    if pairs.is_paired(half, other) {
        return true;
    }
    let street = |road: usize| network.street_of(road).map(|(street, _)| street);
    let (Some(own), Some(theirs)) = (street(half), street(other)) else {
        return false;
    };
    network.streets[own].ways.iter().any(|way| {
        pairs
            .partners(way.road)
            .any(|partner| street(partner.road) == Some(theirs))
    })
}

/// Way на пути от узла слияния ([`walk`]): сколько метров от узла до его
/// начала по пути и пройден ли он против своих точек.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stretch {
    road: usize,
    from: f32,
    reversed: bool,
}

/// Нарисованный путь дороги `road` от узла слияния — её конца (`node_at_end`)
/// или начала — и дальше по ways её улицы, пока не наберётся `reach` метров.
fn from_node(
    road: usize,
    node_at_end: bool,
    reach: f32,
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
) -> Vec<Vec2> {
    walk(road, node_at_end, reach, paths, network).0
}

/// [`from_node`] — и ways, из которых путь сложен, по порядку от узла.
fn walk(
    road: usize,
    node_at_end: bool,
    reach: f32,
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
) -> (Vec<Vec2>, Vec<Stretch>) {
    let mut points = paths[road].as_ref().to_vec();
    if node_at_end {
        points.reverse();
    }
    let mut stretches = vec![Stretch {
        road,
        from: 0.0,
        reversed: node_at_end,
    }];
    let Some((street, at)) = network.street_of(road) else {
        return (points, stretches);
    };
    let street = &network.streets[street];
    // узел — выход way по порядку улицы: от него идём к её началу
    let backward = node_at_end != street.ways[at].reversed;
    let mut index = at;
    while polyline_length(&points) < reach {
        let next = if backward {
            index.checked_sub(1)
        } else {
            (index + 1 < street.ways.len()).then_some(index + 1)
        };
        let Some(next) = next else {
            break;
        };
        index = next;
        let way = paths[street.ways[index].road].as_ref();
        let (Some(&last), Some(&first), Some(&end)) = (points.last(), way.first(), way.last())
        else {
            break;
        };
        // разводка пар двигает оси, и шов соседних ways сходится не бит в бит
        let reversed = if first.distance(last) < SEAM_SLACK {
            false
        } else if end.distance(last) < SEAM_SLACK {
            true
        } else {
            break;
        };
        stretches.push(Stretch {
            road: street.ways[index].road,
            from: polyline_length(&points),
            reversed,
        });
        if reversed {
            points.extend(way.iter().rev().skip(1));
        } else {
            points.extend(way.iter().skip(1));
        }
    }
    (points, stretches)
}

/// Направление пути от его торца: от конца (`from_end`) назад или от начала
/// вперёд — хорда на [`ARM_REACH`].
fn away(path: &[Vec2], from_end: bool) -> Vec2 {
    let origin = if from_end {
        path[path.len() - 1]
    } else {
        path[0]
    };
    let mut far = origin;
    let mut walked = 0.0;
    let points: Box<dyn Iterator<Item = &Vec2>> = if from_end {
        Box::new(path.iter().rev())
    } else {
        Box::new(path.iter())
    };
    let mut previous = origin;
    for &point in points {
        walked += point.distance(previous);
        previous = point;
        far = point;
        if walked >= ARM_REACH {
            break;
        }
    }
    (far - origin).normalize_or_zero()
}

/// Полоса, которой кромка одной половины сходится к кромке продолжения.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeBand {
    pub half: usize,
    /// Длина клина по пути половины от узла, м.
    pub length: f32,
    /// Асфальт — в слой улиц, под ленты.
    pub asphalt: Vec<Vec2>,
    /// Тротуар за ним, если у половины снаружи он есть.
    pub sidewalk: Option<Vec<Vec2>>,
}

/// Полосы, которыми кромки половин сходятся к кромкам продолжения: асфальт и,
/// где у половины снаружи тротуар шириной `sidewalk(половина, сторона)`
/// (`Drawn::sidewalk_on`, сторона `[слева, справа]` по точкам way), тротуар
/// за ним. `per_meter` — длина клина на метр разницы ширин. Клин идёт от узла
/// по пути половины и дальше по ways её улицы (`network`): у узла OSM режет
/// половину на короткие ways, и клин в 30–60 м на одном таком не умещается.
pub fn merge_bands(
    merge: &Merge,
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
    sidewalk: impl Fn(usize, usize) -> Option<f32>,
    per_meter: f32,
) -> Vec<MergeBand> {
    let wide = roads[merge.street].width / 2.0;
    let [first, second] = merge.halves;
    let mut bands = Vec::new();
    for (half, partner, node_at_end) in [(first, second, true), (second, first, false)] {
        let road = roads[half];
        let narrow = road.width / 2.0;
        let step = wide - narrow;
        if step < MERGE_MIN_STEP {
            continue;
        }
        let wanted = 2.0 * step * per_meter;
        let path = from_node(half, node_at_end, wanted / MERGE_MAX_SHARE, paths, network);
        let length = wanted.min(polyline_length(&path) * MERGE_MAX_SHARE);
        if length < MERGE_STEP {
            continue;
        }
        // снаружи — сторона, противоположная паре
        let Some(partner_left) =
            partner_left(merge.node, &path, partner, !node_at_end, paths, network)
        else {
            continue;
        };
        let outward = if partner_left { -1.0 } else { 1.0 };
        let (points, along) = resample(&path, length);
        let normals: Vec<Vec2> = miter_offsets(&points, false, 1.0)
            .into_iter()
            .map(|normal| normal * outward)
            .collect();
        // у узла — кромка продолжения, за клином — своя
        let reach = |at: f32| wide - step * smoothstep(at / length);
        let band = |inner: f32, extra: f32| -> Vec<Vec2> {
            let outer = points
                .iter()
                .zip(&normals)
                .zip(&along)
                .map(|((&point, &normal), &at)| point + normal * (reach(at) + extra));
            let inner: Vec<Vec2> = points
                .iter()
                .zip(&normals)
                .map(|(&point, &normal)| point + normal * inner)
                .collect();
            outer.chain(inner.into_iter().rev()).collect()
        };
        // тротуар по тегу — на той стороне, что снаружи, `[слева, справа]` по
        // точкам way: у въезжающей половины путь от узла развёрнут
        let outer_left = !partner_left != node_at_end;
        bands.push(MergeBand {
            half,
            length,
            asphalt: band(narrow - MERGE_OVERLAP, 0.0),
            sidewalk: sidewalk(half, usize::from(!outer_left)).map(|width| band(narrow, width)),
        });
    }
    bands
}

/// Лежит ли пара `partner` (её узел — конец, если `partner_at_end`) слева от
/// пути половины `path`, идущего от узла `node`: её точка в хорде от узла
/// левее его.
fn partner_left(
    node: Vec2,
    path: &[Vec2],
    partner: usize,
    partner_at_end: bool,
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
) -> Option<bool> {
    let theirs = from_node(partner, partner_at_end, ARM_REACH, paths, network);
    let own = *path.get(1)?;
    let other = *theirs.last()?;
    let ahead = along_path(path, ARM_REACH).unwrap_or(own) - node;
    Some(ahead.perp_dot(other - node) > 0.0)
}

/// Раскладки клина слияния по ways половин ([`MergeRamp`]): каркас каждой
/// половины сводится в свою сторону каркаса продолжения. В узле полосы
/// половины лежат между её наружной кромкой и осью продолжения, по сетке
/// продолжения; сетка сдвигается так, чтобы наружная линия осталась
/// наружной, — лишние полосы гаснут у пары, как в клине шва. Линии идут
/// сеткой целиком, поэтому не пересекаются.
///
/// Длина — ручка `Taper` × сдвиг (кромки, сетки, не меньше полосы), не больше
/// [`MERGE_MAX_SHARE`] пути половины; от узла дальше по ways её улицы.
pub fn merge_ramps(
    merge: &Merge,
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
    per_meter: f32,
) -> Vec<(usize, MergeRamp)> {
    let street = roads[merge.street];
    let lanes = lane_count(street);
    let width = lane_width();
    let reach = f32::from(lanes) * width / 2.0;
    let grid = if lanes % 2 == 1 { width / 2.0 } else { 0.0 };
    let [first, second] = merge.halves;
    let mut ramps = Vec::new();
    for (half, partner, node_at_end) in [(first, second, true), (second, first, false)] {
        let near = from_node(half, node_at_end, ARM_REACH, paths, network);
        let Some(left) = partner_left(merge.node, &near, partner, !node_at_end, paths, network)
        else {
            continue;
        };
        // по ходу движения: путь въезжающей от узла идёт против него
        let partner_on_left = left != node_at_end;
        // в узле: от наружной кромки продолжения до его оси, по его сетке
        let at_node = |body: LaneFrame| {
            let (low, high, outer) = if partner_on_left {
                (-reach, 0.0, body.low)
            } else {
                (0.0, reach, body.high)
            };
            let edge = if partner_on_left { low } else { high };
            let target = body.origin + edge - outer;
            LaneFrame {
                origin: grid + ((target - grid) / width).round() * width,
                low,
                high,
            }
        };
        let body = lane_frame(lane_count(roads[half]));
        let frame = at_node(body);
        let step = street.width / 2.0 - roads[half].width / 2.0;
        let shift = (2.0 * step)
            .max(2.0 * (frame.origin - body.origin).abs())
            .max(width);
        let wanted = per_meter * shift;
        let (path, stretches) = walk(half, node_at_end, wanted / MERGE_MAX_SHARE, paths, network);
        let length = wanted.min(polyline_length(&path) * MERGE_MAX_SHARE);
        if length < MERGE_STEP {
            continue;
        }
        for stretch in stretches {
            let way = roads[stretch.road];
            // ways одного хода с половиной: по ним рама way — рама движения
            if stretch.from >= length || !way.oneway || stretch.reversed != node_at_end {
                break;
            }
            let own = polyline_length(paths[stretch.road].as_ref());
            ramps.push((
                stretch.road,
                MergeRamp {
                    frame: at_node(lane_frame(lane_count(way))),
                    length,
                    start: if stretch.reversed {
                        stretch.from + own
                    } else {
                        stretch.from
                    },
                    away: !stretch.reversed,
                },
            ));
        }
    }
    ramps
}

/// Сколько метров от узла слияния ищется, где кончается разделительная, м.
const AXIS_REACH: f32 = 60.0;
/// Сколько асфальта осевая слияния оставляет до носа газона, м.
const NOSE_CLEARANCE: f32 = 1.0;

/// Где у пары половин кончается разделительная — до него доходит осевая
/// слияния ([`merge_axis`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MedianEnd {
    /// Торец асфальтовой середины: между половинами до него асфальт, и
    /// осевая смыкается с двойной сплошной разделительной.
    Paved(Vec2),
    /// Точка контура газона: осевая встаёт перед носом с отступом
    /// [`NOSE_CLEARANCE`] — и не дальше, чем кромки половин сходятся.
    Lawn(Vec2),
}

impl MedianEnd {
    fn at(self) -> Vec2 {
        match self {
            Self::Paved(at) | Self::Lawn(at) => at,
        }
    }
}

/// Ближайший к узлу слияния из `ends` в [`AXIS_REACH`].
fn nearest_end(merge: &Merge, ends: &[MedianEnd]) -> Option<MedianEnd> {
    ends.iter()
        .copied()
        .filter(|end| end.at().distance(merge.node) <= AXIS_REACH)
        .min_by(|a, b| {
            a.at()
                .distance(merge.node)
                .total_cmp(&b.at().distance(merge.node))
        })
}

/// Асфальт между половинами от узла слияния до носа газона их пары — контур
/// между осями половин, под их лентами: где кромки половин разошлись, а газон
/// ещё не начался, лежал клин голой земли (Рязанская, пример 26). Заходит за
/// острие носа на [`NOSE_FILL_BEYOND`] за вычетом бордюра газона `kerbs` —
/// трава лежит под асфальтом улиц, и срез поперёк острия оставлял у углов
/// бордюра клочки земли. Без газона в [`AXIS_REACH`] — пусто: у асфальтовой
/// середины асфальт и так до узла.
pub fn nose_fill(
    merge: &Merge,
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
    ends: &[MedianEnd],
    kerbs: &[Shape],
) -> Vec<Shape> {
    let Some(MedianEnd::Lawn(nose)) = nearest_end(merge, ends) else {
        return Vec::new();
    };
    let [first, second] = merge.halves;
    let own = from_node(first, true, AXIS_REACH, paths, network);
    let theirs = from_node(second, false, AXIS_REACH, paths, network);
    let (mut outline, mut back) = (vec![merge.node], Vec::new());
    let mut beyond = None;
    let mut at = MERGE_STEP;
    while at <= AXIS_REACH {
        let (Some(a), Some(b)) = (along_path(&own, at), along_path(&theirs, at)) else {
            return Vec::new();
        };
        outline.push(a);
        back.push(b);
        let middle = (a + b) / 2.0;
        match beyond {
            Some(end) if at >= end => break,
            None if (nose - middle).dot(middle - merge.node) <= 0.0 => {
                beyond = Some(at + NOSE_FILL_BEYOND);
            }
            _ => {}
        }
        at += MERGE_STEP;
    }
    if beyond.is_none() || outline.len() < 3 {
        return Vec::new();
    }
    outline.extend(back.into_iter().rev());
    let fill = oriented(&outline, true);
    let near: Vec<Shape> = kerbs
        .iter()
        .filter(|shape| {
            shape.first().is_some_and(|outer| {
                outer
                    .iter()
                    .any(|point| Vec2::from(*point).distance(nose) < NOSE_FILL_NEAR)
            })
        })
        .cloned()
        .collect();
    if near.is_empty() {
        return vec![vec![fill]];
    }
    vec![vec![fill]].overlay(&near, OverlayRule::Difference, FillRule::NonZero)
}

/// Насколько асфальт слияния заходит за острие носа газона, м ([`nose_fill`]):
/// с запасом на скругление носа.
const NOSE_FILL_BEYOND: f32 = 4.0;
/// Бордюр газона, вычитаемый из асфальта слияния, — тот, чей контур ближе
/// этого к острию носа, м.
const NOSE_FILL_NEAR: f32 = 10.0;

/// Осевая продолжения, заведённая за узел слияния: по середине между путями
/// половин от узла — до ближайшего к узлу из `ends` в [`AXIS_REACH`], перед
/// газоном — с отступом (между половинами до носа лежит [`nose_fill`]). Без
/// разделительной — не дальше, чем кромки половин сходятся: где они
/// разошлись, между ними земля.
pub fn merge_axis(
    merge: &Merge,
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    network: &RoadNetwork,
    ends: &[MedianEnd],
) -> Vec<Vec2> {
    let [first, second] = merge.halves;
    let own = from_node(first, true, AXIS_REACH, paths, network);
    let theirs = from_node(second, false, AXIS_REACH, paths, network);
    let apart = (roads[first].width + roads[second].width) / 2.0;
    let nearest = nearest_end(merge, ends);
    let target = nearest.map(|end| match end {
        MedianEnd::Paved(at) => (at, 0.0),
        MedianEnd::Lawn(at) => (at, NOSE_CLEARANCE),
    });
    let mut line = vec![merge.node];
    let mut at = MERGE_STEP;
    while at <= AXIS_REACH {
        let (Some(a), Some(b)) = (along_path(&own, at), along_path(&theirs, at)) else {
            break;
        };
        let middle = (a + b) / 2.0;
        let last = line[line.len() - 1];
        match target {
            // миновали торец: дальше осевую ведёт разделительная
            Some((end, clearance)) if (end - middle).dot(middle - last) <= 0.0 => {
                let heading = (middle - last).normalize_or_zero();
                let tip = end - heading * clearance;
                // точки, уже зашедшие за отступ, — долой
                while line.len() > 1 && (tip - line[line.len() - 1]).dot(heading) <= 0.0 {
                    line.pop();
                }
                if (tip - line[line.len() - 1]).dot(heading) > 0.0 {
                    line.push(tip);
                }
                break;
            }
            // кромки разошлись, а разделительной нет: между половинами земля
            _ if nearest.is_none() && a.distance(b) > apart => break,
            _ => line.push(middle),
        }
        at += MERGE_STEP;
    }
    line
}

/// Точка пути в `at` метрах от начала; путь короче — `None`.
fn along_path(path: &[Vec2], at: f32) -> Option<Vec2> {
    (path.len() >= 2 && at <= polyline_length(path)).then(|| point_at_arc_length(path, at))
}

/// Путь от начала до `length`, с вершиной не реже [`MERGE_STEP`], и длина
/// каждой вершины от начала.
fn resample(path: &[Vec2], length: f32) -> (Vec<Vec2>, Vec<f32>) {
    let mut points = vec![path[0]];
    let mut along = vec![0.0];
    let mut walked = 0.0;
    for pair in path.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let link = from.distance(to);
        if link <= 0.0 {
            continue;
        }
        let steps = (link / MERGE_STEP).ceil().max(1.0) as usize;
        for step in 1..=steps {
            let at = walked + link * step as f32 / steps as f32;
            if at >= length {
                points.push(from.lerp(to, (length - walked) / link));
                along.push(length);
                return (points, along);
            }
            points.push(from.lerp(to, step as f32 / steps as f32));
            along.push(at);
        }
        walked += link;
    }
    (points, along)
}

#[cfg(test)]
mod tests;
