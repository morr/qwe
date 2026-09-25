//! **Парные половины** — разделённая улица, которую OSM рисует двумя
//! встречными односторонними ways бок о бок.
//!
//! Каждая половина рисовалась своей улицей: своим тротуаром с обеих сторон,
//! своими кромками, и между ними оставалась полоса того, что лежит ниже, —
//! светлая нитка тротуара под зазором в полметра или газон с двумя
//! тротуарами посреди проспекта. А там, где картограф свёл половины теснее
//! их ширины, полотна ложились одно на другое, и линии полос одной половины
//! резали линии другой. Здесь пара находится один раз, на нарисованных осях
//! (`roads/axis.rs`), и её читают все, кто рисует улицу: заливка
//! разделительной, двойная сплошная, газон с бордюром, тротуар только с
//! внешней стороны.
//!
//! **Поиск** ([`Pairs::new`]) — так же, как искала разделительную большая
//! стоянка: от каждой точки оси с шагом [`PROBE_STEP`] — ближайшая другая
//! половина, идущая **навстречу** и **рядом** ([`PAIR_PARALLEL`],
//! [`PAIR_SKEW`] — продолжение той же дороги торец в торец соседом не
//! считается), того же класса ([`Highway`](crate::map::osm::Highway)) и не
//! дальше [`PAIR_MAX_GAP`] между кромками. Кусок короче [`PAIR_MIN`] — не
//! пара: так сходятся два съезда.
//!
//! **Общая ось** ([`Pairs::align`]). Зазор между половинами OSM гуляет — в
//! Туле на одной паре от наложения в метр до зазора в полтора. Половины
//! разводятся от середины между ними на постоянное расстояние: зазор куска —
//! его медиана, у асфальтовой разделительной — не у́же [`PAVED_MIN_GAP`].
//! Разводка сходит на нет за [`ALIGN_TRANSITION`] до конца куска и до узла с
//! чужой улицей: узел закреплён (по нему находят друг друга скругления
//! бордюров, разрывы разметки и стежки), а сдвиг в полметра у конца куска
//! читался бы ступенькой. У узла ось ещё и прямая на [`PIN_STRAIGHT`]: на
//! гнутом крае скругление бордюра не помещается. Стык с продолжением той же
//! половины, у которого пара тоже есть, концом куска не считается — там
//! разводка идёт насквозь.
//! Сдвигается только нарисованная ось: точки `RoadLine` читает навмеш.

use std::borrow::Cow;

use bevy::prelude::*;

use super::{RoadNetwork, RoadNodes};
use crate::map::along::{nearest_on_path, simplify};
use crate::map::grid::Grid;
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine};

/// Шаг, которым ось ощупывается на соседа, м.
pub const PROBE_STEP: f32 = 2.0;
/// Самая узкая асфальтовая разделительная после разводки, м: двойная
/// сплошная шириной 0.45 м ложится на неё, не заходя на полосы. Половины,
/// наложенные одна на другую, расходятся до неё.
pub const PAVED_MIN_GAP: f32 = 0.5;
/// Самый широкий газон между половинами, м, при котором они ещё одна улица.
pub const PAIR_MAX_GAP: f32 = 15.0;
/// На сколько кромки половин могут заходить одна на другую, м, — на полосу.
/// Встречные половины не сливаются, как попутные полосы, так что наложение —
/// небрежность картографа: у Красноармейского проспекта оси трёхполосных
/// половин стоят в 9.5 м вместо 11, и разводка их расставляет.
pub const PAIR_OVERLAP: f32 = 3.3;
/// Кусок пары короче этого, м, — не пара: так сходятся два съезда.
pub const PAIR_MIN: f32 = 8.0;
/// Косинус угла между встречными осями, при котором половины ещё идут рядом.
const PAIR_PARALLEL: f32 = 0.9;
/// Доля расстояния, на которую сосед смещён вдоль оси: больше — это торец
/// продолжения, а не бок соседа.
const PAIR_SKEW: f32 = 0.35;
/// Насколько проба может выйти за торец звена соседа, чтобы он ещё шёл рядом,
/// м: полторы пробы — шов или узел, а не продолжение торец в торец.
const END_OVERHANG: f32 = 3.0;
/// Торцы соседних разделительных ближе этого, м, сводятся в одну точку
/// ([`Pairs::join_ends`]).
const JOIN_GAP: f32 = 5.0;
/// За сколько метров до конца куска и до закреплённого узла разводка сходит
/// на нет.
pub const ALIGN_TRANSITION: f32 = 20.0;
/// Сколько метров оси у закреплённого узла разводка не трогает вовсе: там
/// ложится скругление бордюра к поперечной улице (`roads/corners.rs`), а оно
/// кладётся только на прямой край — полуширина поперечного проспекта и
/// касательная дуги в 10 м. Переход начинается за этим участком.
const PIN_STRAIGHT: f32 = 16.0;
/// Шаг вершин разводимой оси, м: на длинном прямом звене сдвиг одних его
/// концов не держал бы зазор посередине.
const ALIGN_STEP: f32 = 4.0;
/// Допуск, с которым разведённая ось и середина разделительной прореживаются
/// обратно, м: сдвиг ведётся по вершинам через [`ALIGN_STEP`] и
/// [`PROBE_STEP`], а на прямой их столько не нужно.
const SIMPLIFY_TOLERANCE: f32 = 0.03;
/// Ячейка сетки звеньев, м.
const CELL: f32 = 32.0;

/// Кусок половины, на котором рядом идёт её пара.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairRun {
    /// Длина по нарисованной оси половины, м: начало и конец куска.
    pub from: f32,
    pub to: f32,
    /// Индекс второй половины.
    pub partner: usize,
    /// Пара лежит слева по ходу половины.
    pub left: bool,
    /// Асфальта или газона между кромками после разводки, м.
    pub gap: f32,
}

/// Разделительная пары: то, что лежит между половинами.
#[derive(Debug, Clone, PartialEq)]
pub struct Median {
    /// Половины: та, по чьей оси она меряется (с меньшим индексом), и её пара.
    pub roads: [usize; 2],
    /// Кусок первой половины, м.
    pub from: f32,
    pub to: f32,
    /// Асфальта или газона между кромками после разводки, м.
    pub gap: f32,
    /// Асфальт между половинами, а не газон: зазор не шире ручки `Median gap`.
    paved: bool,
    /// Середина между осями — по ходу первой половины. До [`Pairs::align`]
    /// пуста.
    pub midline: Vec<Vec2>,
    /// Внутренние кромки половин в тех же точках, что и середина.
    pub inner: [Vec<Vec2>; 2],
}

impl Median {
    /// Асфальт между половинами, а не газон.
    pub fn is_paved(&self) -> bool {
        self.paved
    }

    /// Наибольшее расстояние между осями, м: ширина полосы вдоль середины,
    /// которая кроет всё между половинами.
    pub fn apart(&self) -> f32 {
        self.midline
            .iter()
            .zip(&self.inner[0])
            .map(|(mid, inner)| 2.0 * mid.distance(*inner))
            .fold(0.0, f32::max)
    }
}

/// Парные половины карты.
#[derive(Debug, Clone, Default)]
pub struct Pairs {
    /// По каждой дороге — её куски с парой, по ходу.
    pub runs: Vec<Vec<PairRun>>,
    /// По разделительной на каждую пару кусков — одну, а не с обеих сторон:
    /// две двойные линии, посчитанные от каждой половины, ложились бы одна на
    /// другую со сдвигом в сантиметры.
    pub medians: Vec<Median>,
}

/// Может ли дорога быть половиной разделённой улицы: одностороннее полотно,
/// не кольцо, не мост и не арка. Проезд (`service`) — тоже: бульвар у ТРЦ
/// «Макси» размечен именно так, двумя встречными проездами, а пара ищется
/// только в своём классе. Проезд ряда стоянки — нет: встречные ряды
/// разделяют места, а не разделительная.
pub fn pairable(road: &RoadLine) -> bool {
    road.class == RoadClass::Street
        && !road.parking_aisle
        && road.oneway
        && !road.is_roundabout()
        && !road.carves_navmesh()
        && road.points.len() >= 2
}

/// Зазор, до которого разводится кусок с медианным зазором `gap`;
/// `median_gap` — самая широкая асфальтовая разделительная.
fn target_gap(gap: f32, median_gap: f32) -> f32 {
    if gap <= median_gap {
        gap.max(PAVED_MIN_GAP)
    } else {
        gap
    }
}

/// Точка оси, ощупанная на соседа.
struct Probe {
    along: f32,
    at: Vec2,
    /// Сосед: дорога, ближайшая точка его оси, расстояние между осями.
    beside: Option<(usize, Vec2, f32)>,
}

impl Pairs {
    /// Пары среди `roads`, нарисованных по `paths`. Середины разделительных
    /// ещё пусты — их кладёт [`Pairs::align`].
    pub fn new(roads: &[RoadLine], paths: &[impl AsRef<[Vec2]>], median_gap: f32) -> Self {
        let mut pairs = Self {
            runs: vec![Vec::new(); roads.len()],
            medians: Vec::new(),
        };
        let candidates: Vec<usize> = (0..roads.len())
            .filter(|&index| pairable(&roads[index]) && paths[index].as_ref().len() >= 2)
            .collect();
        if candidates.len() < 2 {
            return pairs;
        }
        let widest = candidates
            .iter()
            .map(|&index| roads[index].width)
            .fold(0.0, f32::max);
        let mut segments: Grid<(usize, usize)> = Grid::new(CELL);
        for &index in &candidates {
            for (at, pair) in paths[index].as_ref().windows(2).enumerate() {
                segments.insert_segment(pair[0], pair[1], 0.0, (index, at));
            }
        }
        for &index in &candidates {
            let path = paths[index].as_ref();
            let reach = (roads[index].width + widest) / 2.0 + PAIR_MAX_GAP;
            let probes: Vec<Probe> = samples(path)
                .into_iter()
                .map(|(along, at, heading)| Probe {
                    along,
                    at,
                    beside: beside(index, at, heading, reach, roads, paths, &segments),
                })
                .collect();
            let mut start = 0;
            while start < probes.len() {
                let Some((partner, ..)) = probes[start].beside else {
                    start += 1;
                    continue;
                };
                let end = probes[start..]
                    .iter()
                    .position(|probe| probe.beside.map(|beside| beside.0) != Some(partner))
                    .map_or(probes.len(), |offset| start + offset);
                pairs.push_run(index, partner, &probes[start..end], roads, median_gap);
                start = end;
            }
        }
        pairs
    }

    fn push_run(
        &mut self,
        index: usize,
        partner: usize,
        probes: &[Probe],
        roads: &[RoadLine],
        median_gap: f32,
    ) {
        let (first, last) = (&probes[0], &probes[probes.len() - 1]);
        if last.along - first.along < PAIR_MIN {
            return;
        }
        let asphalt = (roads[index].width + roads[partner].width) / 2.0;
        let mut gaps: Vec<f32> = probes
            .iter()
            .filter_map(|probe| probe.beside.map(|(_, _, apart)| apart - asphalt))
            .collect();
        gaps.sort_by(f32::total_cmp);
        let gap = target_gap(gaps[gaps.len() / 2], median_gap);
        let (_, near, _) = first.beside.expect("кусок пары — из точек с соседом");
        let heading = probes[1].at - first.at;
        self.runs[index].push(PairRun {
            from: first.along,
            to: last.along,
            partner,
            left: heading.perp_dot(near - first.at) > 0.0,
            gap,
        });
        if index < partner {
            self.medians.push(Median {
                roads: [index, partner],
                from: first.along,
                to: last.along,
                gap,
                paved: gap <= median_gap,
                midline: Vec::new(),
                inner: [Vec::new(), Vec::new()],
            });
        }
    }

    /// Развести половины на постоянный зазор — сдвигом нарисованных осей
    /// `paths` — и положить середины разделительных по разведённым осям.
    pub fn align(
        &mut self,
        paths: &mut [Cow<[Vec2]>],
        roads: &[RoadLine],
        network: &RoadNetwork,
        nodes: &RoadNodes,
    ) {
        let mut original: Vec<Option<Vec<Vec2>>> = vec![None; paths.len()];
        for run in self.runs.iter().flatten() {
            original[run.partner].get_or_insert_with(|| paths[run.partner].to_vec());
        }
        let street = |road: usize| network.street_of(road).map(|(street, _)| street);
        let lengths: Vec<f32> = (0..paths.len())
            .map(|road| {
                if self.runs[road].is_empty() {
                    0.0
                } else {
                    polyline_length(&paths[road])
                }
            })
            .collect();
        // стык с продолжением той же половины, у которого пара у этого же
        // узла тоже есть, — не конец куска
        let continued = |road: usize, node: Vec2| {
            nodes.roads_at(node).iter().any(|&other| {
                other != road
                    && street(other).is_some()
                    && street(other) == street(road)
                    && self.runs[other].iter().any(|run| {
                        let path = &paths[other];
                        (path[0] == node && run.from <= PROBE_STEP)
                            || (path[path.len() - 1] == node
                                && run.to >= lengths[other] - PROBE_STEP)
                    })
            })
        };
        let mut aligned: Vec<(usize, Vec<Vec2>)> = Vec::new();
        for (road, runs) in self.runs.iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            let path = &paths[road];
            let ends = [
                continued(road, path[0]),
                continued(road, path[path.len() - 1]),
            ];
            let (mut dense, along) = densify(path, ALIGN_STEP);
            let total = along[along.len() - 1];
            // узлы с чужими проезжими дорогами — закреплены; переход дорожки
            // — нет, как и у оси улицы (`roads/axis.rs`)
            let pinned: Vec<f32> = dense
                .iter()
                .zip(&along)
                .filter(|(point, _)| {
                    nodes.roads_at(**point).iter().any(|&other| {
                        other != road
                            && roads[other].class == RoadClass::Street
                            && (street(other).is_none() || street(other) != street(road))
                    })
                })
                .map(|(_, &at)| at)
                .collect();
            for (point, &at) in dense.iter_mut().zip(&along) {
                let Some(run) = runs
                    .iter()
                    .find(|run| run.from - 1e-3 <= at && at <= run.to + 1e-3)
                else {
                    continue;
                };
                let from_start = if ends[0] && run.from <= PROBE_STEP {
                    f32::INFINITY
                } else {
                    at - run.from
                };
                let to_end = if ends[1] && run.to >= total - PROBE_STEP {
                    f32::INFINITY
                } else {
                    run.to - at
                };
                let to_pin = pinned
                    .iter()
                    .map(|pin| (pin - at).abs())
                    .fold(f32::INFINITY, f32::min)
                    - PIN_STRAIGHT;
                let weight = ease(from_start.min(to_end)) * ease(to_pin);
                if weight <= 0.0 {
                    continue;
                }
                let partner = original[run.partner]
                    .as_deref()
                    .expect("ось пары сохранена до разводки");
                let Some((near, _)) = nearest_on_path(partner, *point) else {
                    continue;
                };
                let Some(outward) = (*point - near).try_normalize() else {
                    continue;
                };
                let asphalt = (roads[road].width + roads[run.partner].width) / 2.0;
                let wanted = point.midpoint(near) + outward * (asphalt + run.gap) / 2.0;
                *point += (wanted - *point) * weight;
            }
            // узлы остаются вершинами: по их точному месту их находят соседи
            let kept = simplify(&dense, false, SIMPLIFY_TOLERANCE, |index| {
                nodes.is_shared(dense[index])
            });
            aligned.push((road, kept.into_iter().map(|index| dense[index]).collect()));
        }
        for (road, path) in aligned {
            paths[road] = Cow::Owned(path);
        }
        for median in &mut self.medians {
            let [first, second] = median.roads;
            let (path, partner) = (paths[first].as_ref(), paths[second].as_ref());
            let [half_first, half_second] = [roads[first].width / 2.0, roads[second].width / 2.0];
            median.midline.clear();
            median.inner = [Vec::new(), Vec::new()];
            for (_, at, _) in samples(path)
                .into_iter()
                .filter(|(along, ..)| median.from <= *along && *along <= median.to)
            {
                let Some((near, _)) = nearest_on_path(partner, at) else {
                    continue;
                };
                let across = (near - at).normalize_or_zero();
                median.midline.push(at.midpoint(near));
                median.inner[0].push(at + across * half_first);
                median.inner[1].push(near - across * half_second);
            }
            // вершина остаётся, если она нужна хоть одной из трёх линий
            let mut kept = vec![false; median.midline.len()];
            for line in [&median.midline, &median.inner[0], &median.inner[1]] {
                for index in simplify(line, false, SIMPLIFY_TOLERANCE, |_| false) {
                    kept[index] = true;
                }
            }
            let thin = |line: &mut Vec<Vec2>| {
                let mut index = 0;
                line.retain(|_| {
                    index += 1;
                    kept[index - 1]
                });
            };
            thin(&mut median.midline);
            thin(&mut median.inner[0]);
            thin(&mut median.inner[1]);
        }
        self.join_ends();
    }

    /// Свести торцы соседних разделительных, лежащие ближе [`JOIN_GAP`], в
    /// одну точку.
    ///
    /// Половина из двух ways — две пары кусков и две разделительные: одна
    /// кончается последней пробой до шва, другая начинается у шва с другой
    /// стороны, и между ними оставалась пара метров — дыра в двойной сплошной
    /// и островок бордюра стоянки посреди бульвара «Макси».
    fn join_ends(&mut self) {
        let tip = |median: &Median, end: bool| {
            let line = &median.midline;
            (line.len() >= 2).then(|| if end { line[line.len() - 1] } else { line[0] })
        };
        let mut joins: Vec<(usize, bool, Vec2, [Vec2; 2])> = Vec::new();
        for (index, median) in self.medians.iter().enumerate() {
            for end in [false, true] {
                let Some(at) = tip(median, end) else {
                    continue;
                };
                let nearest = self
                    .medians
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .flat_map(|(other, median)| {
                        [false, true].map(|other_end| (other, other_end, tip(median, other_end)))
                    })
                    .filter_map(|(other, other_end, point)| Some((other, other_end, point?)))
                    .filter(|(.., point)| point.distance(at) < JOIN_GAP)
                    .min_by(|a, b| a.2.distance(at).total_cmp(&b.2.distance(at)));
                let Some((other, other_end, point)) = nearest else {
                    continue;
                };
                let pick = |line: &[Vec2]| {
                    if other_end {
                        line[line.len() - 1]
                    } else {
                        line[0]
                    }
                };
                let edge = |own: &[Vec2]| {
                    let own = if end { own[own.len() - 1] } else { own[0] };
                    // кромки у разделительных, посчитанных с разных половин,
                    // идут в разном порядке — берётся ближайшая
                    let theirs = [&self.medians[other].inner[0], &self.medians[other].inner[1]]
                        .map(|line| pick(line))
                        .into_iter()
                        .min_by(|a, b| a.distance(own).total_cmp(&b.distance(own)))
                        .unwrap_or(own);
                    own.midpoint(theirs)
                };
                joins.push((
                    index,
                    end,
                    at.midpoint(point),
                    [edge(&median.inner[0]), edge(&median.inner[1])],
                ));
            }
        }
        for (index, end, mid, [first, second]) in joins {
            let Median { midline, inner, .. } = &mut self.medians[index];
            let [inner_first, inner_second] = inner;
            for (line, point) in [(midline, mid), (inner_first, first), (inner_second, second)] {
                if end {
                    line.push(point);
                } else {
                    line.insert(0, point);
                }
            }
        }
    }

    /// Разделительных с асфальтом и с газоном — по ширине. Отчёт слоя дорог
    /// считает нарисованное (трамвайное полотно мощёное при любой ширине,
    /// `medians::carries_tram`), так что это мерка тестов пар.
    #[cfg(test)]
    pub fn count(&self) -> [usize; 2] {
        let paved = self
            .medians
            .iter()
            .filter(|median| median.is_paved())
            .count();
        [paved, self.medians.len() - paved]
    }
}

/// Плавный переход разводки: 0 у конца куска или у узла, 1 за
/// [`ALIGN_TRANSITION`] от них.
fn ease(distance: f32) -> f32 {
    let t = (distance / ALIGN_TRANSITION).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ближайшая половина рядом с точкой `at` оси дороги `index`, идущей по
/// `heading`, — или никакой.
fn beside(
    index: usize,
    at: Vec2,
    heading: Vec2,
    reach: f32,
    roads: &[RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    segments: &Grid<(usize, usize)>,
) -> Option<(usize, Vec2, f32)> {
    let own = &roads[index];
    let mut best: Option<(usize, Vec2, f32)> = None;
    for &(other, segment) in segments.near_each(at - reach, at + reach) {
        if other == index || roads[other].highway != own.highway {
            continue;
        }
        let path = paths[other].as_ref();
        let (from, to) = (path[segment], path[segment + 1]);
        let Some(other_heading) = (to - from).try_normalize() else {
            continue;
        };
        if heading.dot(other_heading) > -PAIR_PARALLEL {
            continue;
        }
        // основание перпендикуляра на прямой звена, а не ближайшая точка
        // отрезка: у торца соседа ближайшей была бы сама его точка, смещённая
        // вдоль оси, и пара терялась за несколько метров до узла или шва —
        // двойная сплошная не доходила до перекрёстка (отчёт автора). За торец
        // звена основание уходит не дальше [`END_OVERHANG`]
        let length = from.distance(to);
        let along = (at - from).dot(other_heading);
        if along < -END_OVERHANG || along > length + END_OVERHANG {
            continue;
        }
        let near = from + other_heading * along;
        let apart = near - at;
        let distance = apart.length();
        let gap = distance - (own.width + roads[other].width) / 2.0;
        if !(-PAIR_OVERLAP..=PAIR_MAX_GAP).contains(&gap)
            || apart.dot(heading).abs() > PAIR_SKEW * distance
            || best.is_some_and(|(_, _, closest)| closest <= distance)
        {
            continue;
        }
        best = Some((other, near, distance));
    }
    best
}

/// Точки оси с шагом [`PROBE_STEP`]: длина от начала, точка, направление.
pub(in crate::map::roads) fn samples(path: &[Vec2]) -> Vec<(f32, Vec2, Vec2)> {
    let mut points = Vec::new();
    let mut start = 0.0;
    for pair in path.windows(2) {
        let length = pair[0].distance(pair[1]);
        let Some(heading) = (pair[1] - pair[0]).try_normalize() else {
            continue;
        };
        let steps = (length / PROBE_STEP).ceil().max(1.0) as usize;
        let from = usize::from(!points.is_empty());
        for step in from..=steps {
            let t = step as f32 / steps as f32;
            points.push((start + length * t, pair[0].lerp(pair[1], t), heading));
        }
        start += length;
    }
    points
}

/// Ломаная с вершинами не реже `step` и длина дуги в каждой вершине.
/// Исходные вершины остаются на месте.
fn densify(path: &[Vec2], step: f32) -> (Vec<Vec2>, Vec<f32>) {
    let mut points = vec![path[0]];
    let mut along = vec![0.0];
    let mut start = 0.0;
    for pair in path.windows(2) {
        let length = pair[0].distance(pair[1]);
        let steps = (length / step).ceil().max(1.0) as usize;
        for index in 1..=steps {
            let t = index as f32 / steps as f32;
            // исходная вершина — как есть, а не `lerp`: по её точному месту
            // узел находят его соседи
            points.push(if index == steps {
                pair[1]
            } else {
                pair[0].lerp(pair[1], t)
            });
            along.push(start + length * t);
        }
        start += length;
    }
    (points, along)
}

#[cfg(test)]
mod tests;
