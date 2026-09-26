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
//! разводка идёт насквозь. Не считается им и стык двух кусков одной
//! половины через дыру до [`RUN_BRIDGE`] — шов **встречной** половины, где
//! пара сменила way: куски сводятся в участок, и зазор переходит от одного к
//! другому за [`ALIGN_TRANSITION`]. Прежде разводка сходила на нет у каждого
//! такого шва, ось возвращалась к OSM на 40 м, и край проспекта, сведённого
//! картографом внахлёст, «дышал» на метр–полтора через каждые 50–100 м
//! (Красноармейский, Советская — отчёт автора).
//! Сдвигается только нарисованная ось: точки `RoadLine` читает навмеш.
//!
//! **Трамвайное полотно** ([`Median::carries_tram`]). Пара, в зазоре которой
//! на большей части пути лежит `railway=tram` (Советская в Туле: две
//! половины по две полосы и пути между ними в 5 м), — не газон, а асфальт с
//! рельсами: каждая половина едет по нему своей внутренней полосой. Такая
//! разделительная мощёная при любой ширине до [`TRAM_BED_MAX_GAP`]; шире —
//! обособленное полотно на траве, газон. Зазор у полотна — свой у каждого
//! куска, как у любой пары: общий на всю улицу (медиана цепочки) стягивал
//! половины там, где картограф развёл их шире, — между перекрёстками
//! проспект сужался на пару метров, у закреплённых узлов возвращался.
//! Ступеньки на швах нет и так: зазор на стыке кусков переходит плавно.

use std::borrow::Cow;

use bevy::prelude::*;

use super::{RoadNetwork, RoadNodes};
use crate::map::along::{nearest_on_path, simplify};
use crate::map::grid::Grid;
use crate::map::osm::model::{RailKind, RailLine, polyline_length};
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
pub const JOIN_GAP: f32 = 5.0;
/// За сколько метров до конца куска и до закреплённого узла разводка сходит
/// на нет.
pub const ALIGN_TRANSITION: f32 = 20.0;
/// Куски одной половины, между которыми дыра не длиннее этого, м, — один
/// участок разводки ([`Pairs::align`]): пара меняется на каждом шве
/// встречной половины, а у шва несколько проб соседа не находят (8 м на
/// Красноармейском). Сходи разводка на нет у каждого такого стыка, край
/// проспекта «дышал» бы через каждые 50–100 м.
const RUN_BRIDGE: f32 = 12.0;
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
/// Доля проб куска, у середины которых лежит трамвай, начиная с которой
/// разделительная — трамвайное полотно.
const TRAM_SHARE_MIN: f32 = 0.5;
/// Самое широкое трамвайное полотно между кромками половин, м. Шире —
/// обособленное полотно на траве (Воздухофлотская в Туле, 10–24 м между
/// осями): там газон правдоподобен.
pub const TRAM_BED_MAX_GAP: f32 = 8.0;
/// Путь засчитан у середины, если он не дальше этого от неё, даже когда
/// зазор между кромками уже, м: два пути в 3–4 м друг от друга.
const TRAM_REACH_MIN: f32 = 2.0;

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
    /// Асфальт между половинами, а не газон ([`Median::is_paved`]).
    pub paved: bool,
    /// Между половинами — трамвайное полотно ([`Median::carries_tram`]).
    pub tram: bool,
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
    /// Асфальт между половинами, а не газон: зазор не шире ручки `Median gap`
    /// или трамвайное полотно.
    paved: bool,
    /// Трамвайное полотно.
    tram: bool,
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

    /// Трамвайное полотно: на большей части куска у середины между
    /// половинами лежит `railway=tram`, а зазор не шире
    /// [`TRAM_BED_MAX_GAP`]. Всегда мощёное.
    pub fn carries_tram(&self) -> bool {
        self.tram
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
                let run = &probes[start..end];
                pairs.push_run(index, partner, run, roads, median_gap, &tracks);
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
        tracks: &Tracks,
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
        // узла тоже есть, — не конец куска. «У узла» — ближе [`RUN_BRIDGE`]:
        // шов своей половины бывает и швом встречной, и у него несколько проб
        // пары не находят. Отдаёт зазор куска продолжения: к нему зазор
        // подходит через шов, без ступеньки
        let continued = |road: usize, node: Vec2| {
            nodes.roads_at(node).iter().find_map(|&other| {
                if other == road || street(other).is_none() || street(other) != street(road) {
                    return None;
                }
                let path = &paths[other];
                self.runs[other]
                    .iter()
                    .find(|run| {
                        (path[0] == node && run.from <= RUN_BRIDGE)
                            || (path[path.len() - 1] == node
                                && run.to >= lengths[other] - RUN_BRIDGE)
                    })
                    .map(|run| run.gap)
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
            // участок, у конца которого ось продолжает та же половина с
            // парой, тянется до самого конца оси и там не сходит на нет
            let spans: Vec<(&[PairRun], f32, f32)> = spans(runs)
                .into_iter()
                .map(|span| {
                    let (first, last) = (&span[0], &span[span.len() - 1]);
                    let from = if ends[0].is_some() && first.from <= RUN_BRIDGE {
                        f32::NEG_INFINITY
                    } else {
                        first.from
                    };
                    let to = if ends[1].is_some() && last.to >= total - RUN_BRIDGE {
                        f32::INFINITY
                    } else {
                        last.to
                    };
                    (span, from, to)
                })
                .collect();
            for (point, &at) in dense.iter_mut().zip(&along) {
                let Some(&(span, from, to)) = spans
                    .iter()
                    .find(|(_, from, to)| from - 1e-3 <= at && at <= to + 1e-3)
                else {
                    continue;
                };
                let (from_start, to_end) = (at - from, to - at);
                let to_pin = pinned
                    .iter()
                    .map(|pin| (pin - at).abs())
                    .fold(f32::INFINITY, f32::min)
                    - PIN_STRAIGHT;
                let weight = ease(from_start.min(to_end)) * ease(to_pin);
                if weight <= 0.0 {
                    continue;
                }
                // у шва встречной половины ближайшей бывает любая из двух её
                // ways — берётся та, что ближе
                let Some((partner, near)) = span
                    .iter()
                    .filter(|run| run.from - RUN_BRIDGE <= at && at <= run.to + RUN_BRIDGE)
                    .filter_map(|run| {
                        let path = original[run.partner]
                            .as_deref()
                            .expect("ось пары сохранена до разводки");
                        nearest_on_path(path, *point).map(|(near, _)| (run.partner, near))
                    })
                    .min_by(|a, b| a.1.distance(*point).total_cmp(&b.1.distance(*point)))
                else {
                    continue;
                };
                let Some(outward) = (*point - near).try_normalize() else {
                    continue;
                };
                // через шов своей половины — к зазору продолжения: у самого
                // узла обе стороны берут середину между своими зазорами
                let mut gap = span_gap(span, at);
                if let (Some(before), true) = (ends[0], from == f32::NEG_INFINITY) {
                    gap = blend(before, gap, at);
                }
                if let (Some(after), true) = (ends[1], to == f32::INFINITY) {
                    gap = blend(gap, after, at - total);
                }
                let asphalt = (roads[road].width + roads[partner].width) / 2.0;
                let wanted = point.midpoint(near) + outward * (asphalt + gap) / 2.0;
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

    /// Разделительных с асфальтом, с газоном и из них трамвайных полотен.
    pub fn count(&self) -> [usize; 3] {
        let count = |test: fn(&Median) -> bool| self.medians.iter().filter(|m| test(m)).count();
        let paved = count(Median::is_paved);
        [
            paved,
            self.medians.len() - paved,
            count(Median::carries_tram),
        ]
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
                let (from, to) = (points[link], points[link + 1]);
                let along = to - from;
                let t = ((at - from).dot(along) / along.length_squared().max(f32::EPSILON))
                    .clamp(0.0, 1.0);
                (from + along * t).distance(at) <= reach
            })
    }
}

/// Плавный переход разводки: 0 у конца куска или у узла, 1 за
/// [`ALIGN_TRANSITION`] от них.
fn ease(distance: f32) -> f32 {
    let t = (distance / ALIGN_TRANSITION).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Куски половины (по ходу), сведённые в участки разводки: соседние, между
/// которыми не больше [`RUN_BRIDGE`], — один участок.
fn spans(runs: &[PairRun]) -> Vec<&[PairRun]> {
    let mut spans = Vec::new();
    let mut start = 0;
    for index in 1..=runs.len() {
        if index == runs.len() || runs[index].from - runs[index - 1].to > RUN_BRIDGE {
            spans.push(&runs[start..index]);
            start = index;
        }
    }
    spans
}

/// Зазор участка в точке `at`: у каждого куска — свой, а на стыке двух он
/// переходит от одного к другому за [`ALIGN_TRANSITION`], без ступеньки.
fn span_gap(span: &[PairRun], at: f32) -> f32 {
    span.windows(2).fold(span[0].gap, |gap, pair| {
        blend(gap, pair[1].gap, at - (pair[0].to + pair[1].from) / 2.0)
    })
}

/// Зазор `before` до шва и `after` за ним в `beyond` м от шва (за ним — со
/// знаком плюс): переход за [`ALIGN_TRANSITION`], на самом шве — середина.
fn blend(before: f32, after: f32, beyond: f32) -> f32 {
    let t = (beyond / ALIGN_TRANSITION + 0.5).clamp(0.0, 1.0);
    before + (after - before) * t * t * (3.0 - 2.0 * t)
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
