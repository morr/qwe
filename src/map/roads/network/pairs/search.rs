//! Поиск пар — второй половины разделённой улицы.
//!
//! Правила поиска и трамвайного полотна — в документе модуля `pairs`
//! (секции «Поиск» и «Трамвайное полотно»).

use bevy::platform::collections::HashSet;

use bevy::prelude::*;

use super::{
    Median, PAIR_MAX_GAP, PAIR_MIN, PAIR_OVERLAP, PAVED_MIN_GAP, PairRun, Pairs, TRAM_BED_MAX_GAP,
    samples,
};
use crate::map::grid::Grid;
use crate::map::osm::model::{RailKind, RailLine, distance_to_segment};
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::roads::junctions::node_key;

/// Доля своей длины, которую короткий way должен пройти рядом с парой,
/// чтобы кусок короче [`PAIR_MIN`] всё же был парой ([`is_pair_run`]).
const PAIR_COVER: f32 = 0.75;
/// Косинус угла между встречными осями, при котором половины ещё идут рядом.
const PAIR_PARALLEL: f32 = 0.9;
/// Доля расстояния, на которую сосед смещён вдоль оси: больше — это торец
/// продолжения, а не бок соседа.
const PAIR_SKEW: f32 = 0.35;
/// Насколько проба может выйти за торец звена соседа, чтобы он ещё шёл рядом,
/// м: полторы пробы — шов или узел, а не продолжение торец в торец.
const END_OVERHANG: f32 = 3.0;
/// Насколько сосед прошлой пробы может быть дальше ближайшего, м, чтобы
/// остаться соседом ([`beside`]). У шва встречной половины её два way идут
/// торец в торец, и в полосе [`END_OVERHANG`] ближайшим через пробу
/// оказывался то один, то другой — пара рвалась на куски короче
/// [`PAIR_MIN`] и не находилась вовсе (Рязань, Первомайский у моста —
/// тротуары половин легли плиткой на всю разделительную).
const PARTNER_SLACK: f32 = 0.5;

/// Ячейка сетки звеньев, м.
const CELL: f32 = 32.0;
/// Доля проб куска, у середины которых лежит трамвай, начиная с которой
/// разделительная — трамвайное полотно.
const TRAM_SHARE_MIN: f32 = 0.5;

/// Путь засчитан у середины, если он не дальше этого от неё, даже когда
/// зазор между кромками уже, м: два пути в 3–4 м друг от друга.
const TRAM_REACH_MIN: f32 = 2.0;

/// Может ли дорога быть половиной разделённой улицы: одностороннее полотно,
/// не кольцо и не арка. Проезд (`service`) — тоже: бульвар у ТРЦ
/// «Макси» размечен именно так, двумя встречными проездами, а пара ищется
/// только в своём классе. Проезд ряда стоянки — нет: встречные ряды
/// разделяют места, а не разделительная.
///
/// Мост — половина, как любая: без пары его оси стояли там, где их положил
/// картограф, а подходы с обеих сторон сходились к ним — проспект сужался у
/// каждого моста (Красноармейский над каналом: 9.5 м между осями вместо
/// 11.4). Навмеш режет мост по точкам OSM, а разводка раздвигает половины
/// наружу — проходимое остаётся внутри нарисованного настила. Арка — нет:
/// её стены — стены дома.
pub fn pairable(road: &RoadLine) -> bool {
    road.class == RoadClass::Street
        && !road.parking_aisle
        && road.oneway
        && !road.is_roundabout()
        && !road.passage
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
#[derive(Clone)]
struct Probe {
    along: f32,
    at: Vec2,
    /// Сосед: дорога, ближайшая точка его оси, расстояние между осями.
    beside: Option<(usize, Vec2, f32)>,
}

impl Pairs {
    /// Пары среди `roads`, нарисованных по `paths`; `rails` — пути карты,
    /// по трамвайным из них находится полотно. Середины разделительных ещё
    /// пусты — их кладёт [`Pairs::align`].
    pub fn new(
        roads: &[RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        median_gap: f32,
        rails: &[RailLine],
    ) -> Self {
        let mut pairs = Self {
            runs: vec![Vec::new(); roads.len()],
            medians: Vec::new(),
        };
        let tracks = Tracks::new(rails);
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
        // куски короче `PAIR_MIN`, почти во весь свой way — до второго прохода
        let mut short: Vec<(usize, usize, Vec<Probe>)> = Vec::new();
        for &index in &candidates {
            let path = paths[index].as_ref();
            let reach = (roads[index].width + widest) / 2.0 + PAIR_MAX_GAP;
            // сосед прошлой пробы держится, пока он почти так же близок
            // ([`PARTNER_SLACK`]): у шва встречной половины два её way стоят
            // рядом, и ближайший перескакивал с одного на другой через пробу
            let mut previous = None;
            let probes: Vec<Probe> = samples(path)
                .into_iter()
                .map(|(along, at, heading)| {
                    let near = beside(index, at, heading, reach, roads, paths, &segments, previous);
                    previous = near.map(|(partner, ..)| partner);
                    Probe {
                        along,
                        at,
                        beside: near,
                    }
                })
                .collect();
            let total = probes.last().map_or(0.0, |probe| probe.along);
            let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
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
                pieces.push((start, end, partner));
                start = end;
            }
            // длина куска меряется по цепочке, в которой сосед сменился без
            // пропуска: на шве встречной половины короткий way видит пару
            // половиной длины с одним её way и половиной — с другим, и ни
            // один кусок не дотягивал до пары (Красноармейский у моста: way в
            // 16 м — по 7.4 м с каждым), а половина сходилась к мосту
            for chain in pieces.chunk_by(|a, b| a.1 == b.0) {
                let span = probes[chain[chain.len() - 1].1 - 1].along - probes[chain[0].0].along;
                for &(start, end, partner) in chain {
                    let run = &probes[start..end];
                    // осколок у шва, где обе половины меняют way, — не кусок
                    if chain.len() > 1 && run[run.len() - 1].along - run[0].along < PAIR_MIN / 2.0 {
                        continue;
                    }
                    match run_kind(span, total) {
                        RunKind::Pair => {
                            pairs.push_run(index, partner, run, roads, median_gap, &tracks)
                        }
                        RunKind::Short => short.push((index, partner, run.to_vec())),
                        RunKind::None => {}
                    }
                }
            }
        }
        // короткий way, целиком идущий рядом с парой, — пара, если он
        // продолжает половину, у которой пара уже есть: у моста или газона
        // улицу режут на куски в десяток метров, а два сходящихся съезда
        // продолжением пары не бывают
        let paired_ends: HashSet<(i32, i32)> = (0..roads.len())
            .filter(|&index| !pairs.runs[index].is_empty())
            .flat_map(|index| {
                let path = paths[index].as_ref();
                [path[0], path[path.len() - 1]].map(node_key)
            })
            .collect();
        for (index, partner, run) in short {
            let path = paths[index].as_ref();
            if [path[0], path[path.len() - 1]]
                .iter()
                .any(|&end| paired_ends.contains(&node_key(end)))
            {
                pairs.push_run(index, partner, &run, roads, median_gap, &tracks);
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
        tracks: &Tracks,
    ) {
        let (first, last) = (&probes[0], &probes[probes.len() - 1]);
        let asphalt = (roads[index].width + roads[partner].width) / 2.0;
        let mut gaps: Vec<f32> = probes
            .iter()
            .filter_map(|probe| probe.beside.map(|(_, _, apart)| apart - asphalt))
            .collect();
        gaps.sort_by(f32::total_cmp);
        let raw = gaps[gaps.len() / 2];
        // трамвай у середины между осями, в пределах самого зазора
        let railed = probes
            .iter()
            .filter(|probe| {
                probe.beside.is_some_and(|(_, near, apart)| {
                    let reach = ((apart - asphalt) / 2.0).max(TRAM_REACH_MIN);
                    tracks.near(probe.at.midpoint(near), reach)
                })
            })
            .count();
        let tram = raw <= TRAM_BED_MAX_GAP && railed as f32 >= TRAM_SHARE_MIN * probes.len() as f32;
        // полотно — асфальт той ширины, что есть: половины расширяются до
        // середины (`roads/medians.rs`), и разводить их не к чему
        let gap = if tram {
            raw.max(PAVED_MIN_GAP)
        } else {
            target_gap(raw, median_gap)
        };
        let paved = tram || gap <= median_gap;
        let (_, near, _) = first.beside.expect("кусок пары — из точек с соседом");
        let heading = probes[1].at - first.at;
        self.runs[index].push(PairRun {
            from: first.along,
            to: last.along,
            partner,
            left: heading.perp_dot(near - first.at) > 0.0,
            gap,
            paved,
            tram,
        });
        if index < partner {
            self.medians.push(Median {
                roads: [index, partner],
                from: first.along,
                to: last.along,
                gap,
                paved,
                tram,
                midline: Vec::new(),
                inner: [Vec::new(), Vec::new()],
            });
        }
    }
}

/// Звенья трамвайных путей карты — сеткой, чтобы пробы пар не перебирали
/// все пути города.
struct Tracks<'a> {
    rails: &'a [RailLine],
    links: Grid<(usize, usize)>,
}

impl<'a> Tracks<'a> {
    fn new(rails: &'a [RailLine]) -> Self {
        let mut links = Grid::new(CELL);
        for (index, rail) in rails.iter().enumerate() {
            if rail.kind != RailKind::Tram {
                continue;
            }
            for (at, pair) in rail.points.windows(2).enumerate() {
                links.insert_segment(pair[0], pair[1], 0.0, (index, at));
            }
        }
        Self { rails, links }
    }

    /// Лежит ли трамвайный путь не дальше `reach` от точки.
    fn near(&self, at: Vec2, reach: f32) -> bool {
        self.links
            .near_each(at - reach, at + reach)
            .any(|&(rail, link)| {
                let points = &self.rails[rail].points;
                distance_to_segment(at, points[link], points[link + 1]) <= reach
            })
    }
}

/// Что за кусок проб, чья цепочка длиной `span`, у половины длиной `total`.
enum RunKind {
    /// Не короче [`PAIR_MIN`] — пара.
    Pair,
    /// Короче, но почти весь way ([`PAIR_COVER`]) и не короче половины
    /// [`PAIR_MIN`]: пара, если way продолжает половину, у которой пара уже
    /// есть. Way в девять метров между газоном и мостом (Рязань,
    /// Первомайский) идёт рядом с парой целиком — и без пары клал тротуар
    /// плиткой в разделительную.
    Short,
    /// Два съезда сходятся — не пара.
    None,
}

fn run_kind(span: f32, total: f32) -> RunKind {
    if span >= PAIR_MIN {
        RunKind::Pair
    } else if span >= PAIR_MIN / 2.0 && span >= PAIR_COVER * total {
        RunKind::Short
    } else {
        RunKind::None
    }
}

/// Ближайшая половина рядом с точкой `at` оси дороги `index`, идущей по
/// `heading`, — или никакой. Сосед прошлой пробы `prefer` остаётся, если он
/// дальше ближайшего не больше чем на [`PARTNER_SLACK`].
#[allow(clippy::too_many_arguments)]
fn beside(
    index: usize,
    at: Vec2,
    heading: Vec2,
    reach: f32,
    roads: &[RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    segments: &Grid<(usize, usize)>,
    prefer: Option<usize>,
) -> Option<(usize, Vec2, f32)> {
    let own = &roads[index];
    let mut best: Option<(usize, Vec2, f32)> = None;
    let mut preferred: Option<(usize, Vec2, f32)> = None;
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
        {
            continue;
        }
        if Some(other) == prefer && preferred.is_none_or(|(_, _, closest)| distance < closest) {
            preferred = Some((other, near, distance));
        }
        if best.is_some_and(|(_, _, closest)| closest <= distance) {
            continue;
        }
        best = Some((other, near, distance));
    }
    match (preferred, best) {
        (Some(kept), Some((_, _, closest))) if kept.2 <= closest + PARTNER_SLACK => Some(kept),
        _ => best,
    }
}
