//! Общая ось пары: разводка половин и середины разделительных.
//!
//! Правила разводки — в документе модуля `pairs` (секция «Общая ось»).

use std::borrow::Cow;

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::{
    ALIGN_TRANSITION, PairRun, Pairs, RUN_BRIDGE, SIMPLIFY_TOLERANCE, facing_half, lay_median,
    point_bits,
};
use crate::ease::smoothstep;
use crate::map::along::{arclengths, nearest_on_path, place_on_path, simplify};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::roads::junctions::node_key;
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::tapers::{self, Tapers};

/// Сколько метров оси у закреплённого узла разводка не трогает вовсе: там
/// ложится скругление бордюра к поперечной улице (`roads/corners.rs`), а оно
/// кладётся только на прямой край — полуширина поперечного проспекта и
/// касательная дуги в 10 м. Переход начинается за этим участком.
const PIN_STRAIGHT: f32 = 16.0;
/// На каком расстоянии от шва двух половин, сведённых в одну точку, м, их
/// оси встают на общую касательную ([`Pairs::align`]).
const SEAM_TAIL: f32 = 0.5;
/// Излом на шве двух половин круче этого, рад, — не продолжение, торцы не
/// выравниваются.
const SEAM_MAX_BEND: f32 = 10.0 * std::f32::consts::PI / 180.0;
/// Сдвиг узла короче этого, м, — не сдвиг: дорогам узла идти некуда.
const MOVE_EPSILON: f32 = 0.01;
/// На каком расстоянии от сдвинутого узла, м, дорога, идущая за ним
/// ([`Pairs::follow_moved_nodes`]), возвращается на своё место.
const FOLLOW_FADE: f32 = 12.0;
/// Шаг вершин разводимой оси, м: на длинном прямом звене сдвиг одних его
/// концов не держал бы зазор посередине.
const ALIGN_STEP: f32 = 4.0;

impl Pairs {
    /// Развести половины на постоянный зазор — сдвигом нарисованных осей
    /// `paths` — и положить середины разделительных по разведённым осям.
    ///
    /// `wedges` — клинья у швов половин (`roads/tapers.rs`): на клине, который
    /// сужает сторону пары, полуширина к паре берётся суженной
    /// ([`facing_half`]). Иначе ось широкого way на шве стояла дальше от
    /// середины, чем ось узкого, — на полразницы ширин: у шва ступенька на
    /// внешней кромке и дыра в землю у внутренней (Тула, витрина 16).
    ///
    /// Отдаёт узлы с чужими дорогами, которые уехали вместе с половиной:
    /// `(точка OSM, куда нарисован)` — их дорогам идти следом
    /// ([`Self::follow_moved_nodes`]).
    pub fn align(
        &mut self,
        paths: &mut [Cow<[Vec2]>],
        roads: &[RoadLine],
        network: &RoadNetwork,
        nodes: &RoadNodes,
        wedges: &Tapers,
    ) -> Vec<(Vec2, Vec2)> {
        let facing = |road: usize, at: f32, total: f32, left: bool| {
            facing_half(roads, wedges, road, at, total, left)
        };
        let mut original: Vec<Option<Vec<Vec2>>> = vec![None; paths.len()];
        for run in self.runs.iter().flatten() {
            original[run.partner].get_or_insert_with(|| paths[run.partner].to_vec());
        }
        let original_lengths: Vec<f32> = original
            .iter()
            .map(|path| path.as_deref().map_or(0.0, polyline_length))
            .collect();
        let original_along: Vec<Vec<f32>> = original
            .iter()
            .map(|path| {
                path.as_deref()
                    .map_or_else(Vec::new, |path| arclengths(path).0)
            })
            .collect();
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
        // подходит через шов, без ступеньки. Узел — точка OSM, а не конец
        // нарисованной оси: сглаженная ось режется на ways в ближайшей к шву
        // точке дуги (`roads/axis.rs`), и по концу оси узел не находился —
        // разводка сходила на нет у каждого шва своей половины. Продолжение —
        // way той же улицы или соосный way за торцом ([`RoadNodes::next_way`]):
        // у перекрёстка улица сети кончается (Красноармейский — tertiary до
        // узла и primary после), а половина идёт дальше, и разводка, сходившая
        // на нет с обеих сторон, сужала проспект у каждого перекрёстка
        let continues = |road: usize, other: usize| {
            other != road
                && ((street(other).is_some() && street(other) == street(road))
                    || [0, 1].into_iter().any(|end| {
                        nodes
                            .next_way(road, end)
                            .is_some_and(|(next, _)| next == other)
                    }))
        };
        let continued = |road: usize, end: bool| {
            let points = &roads[road].points;
            let node = if end {
                points[points.len() - 1]
            } else {
                points[0]
            };
            nodes.roads_at(node).iter().find_map(|&other| {
                if !continues(road, other) {
                    return None;
                }
                let points = &roads[other].points;
                self.runs[other]
                    .iter()
                    .find(|run| {
                        (points[0] == node && run.from <= RUN_BRIDGE)
                            || (points[points.len() - 1] == node
                                && run.to >= lengths[other] - RUN_BRIDGE)
                    })
                    .map(|run| run.gap)
            })
        };
        // шов с клином: зазор меняется не поперёк узла, а по клину широкой
        // стороны. Переход в `ALIGN_TRANSITION` поперёк узла ложился на тот же
        // шов, что и клин, и внешняя кромка сперва шла внутрь со сменой
        // зазора, потом наружу с клином — надлом ~0.2 м (Тула, витрина 16:
        // газон разделительной кончается там же, где 2 полосы становятся 4)
        let seam_wedge = |road: usize, end: bool| -> Option<SeamWedge> {
            let points = &roads[road].points;
            let node = if end {
                points[points.len() - 1]
            } else {
                points[0]
            };
            let fitted = |road: usize, end: usize| {
                let lengths = wedges.at(road).map(|taper| taper.map(|taper| taper.length));
                tapers::fit(polyline_length(&paths[road]), lengths)[end]
            };
            if let Some(taper) = wedges.at(road)[usize::from(end)]
                && continues(road, taper.seam)
            {
                return fitted(road, usize::from(end)).map(SeamWedge::Wide);
            }
            nodes.roads_at(node).iter().find_map(|&other| {
                if !continues(road, other) {
                    return None;
                }
                let others = &roads[other].points;
                (0..2).find_map(|other_end| {
                    let at = if other_end == 1 {
                        others[others.len() - 1]
                    } else {
                        others[0]
                    };
                    let taper = wedges.at(other)[other_end]?;
                    (at == node && taper.seam == road && fitted(other, other_end).is_some())
                        .then_some(SeamWedge::Narrow)
                })
            })
        };
        let mut aligned: Vec<(usize, Vec<Vec2>)> = Vec::new();
        let mut moved: Vec<(Vec2, Vec2)> = Vec::new();
        for (road, runs) in self.runs.iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            let path = &paths[road];
            let ends = [continued(road, false), continued(road, true)];
            let wedged = [seam_wedge(road, false), seam_wedge(road, true)];
            let (mut dense, along) = densify(path, ALIGN_STEP);
            let total = along[along.len() - 1];
            let foreign = |other: usize| other != road && !continues(road, other);
            // узел с чужой дорогой едет вместе с половиной, а дорога идёт
            // следом (`Pairs::follow_moved_nodes`). Держат ось: другая
            // разводимая половина, кольцо, мост или арка — у них своё место,
            // и чужая проезжая улица, проходящая перекрёстком насквозь, — по
            // ней едят поперёк пары, на перекрёстке лежат скругления бордюров
            // и зебры, и кромки половин остаются на месте OSM, пока зазор
            // пары переходит с куска на кусок; иначе переход тянет ось через
            // перекрёсток и уносит его узел вместе с половиной (Тула,
            // Октябрьская у Максима Горького). Выезд-тупик не держит — его
            // узел едет с половиной; прежде держал каждый узел с проездом
            // (`class` не отличает проезд от улицы), и у каждого выезда ось
            // возвращалась к OSM: кромка Красноармейского гуляла на
            // метр–полтора через каждые 50–100 м
            let held = |point: Vec2| {
                let key = node_key(point);
                nodes.roads_at(point).iter().any(|&other| {
                    if !foreign(other) || roads[other].class != RoadClass::Street {
                        return false;
                    }
                    if !self.runs[other].is_empty()
                        || roads[other].is_roundabout()
                        || roads[other].carves_navmesh()
                    {
                        return true;
                    }
                    // проезжая сквозь перекрёсток: вершина посреди чужой оси
                    // либо за торцом есть продолжение; тупик едет с половиной
                    if !roads[other].is_carriageway() {
                        return false;
                    }
                    match roads[other].points.iter().position(|p| node_key(*p) == key) {
                        None => false,
                        Some(0) => nodes.next_way(other, 0).is_some(),
                        Some(last) if last + 1 == roads[other].points.len() => {
                            nodes.next_way(other, 1).is_some()
                        }
                        Some(_) => true,
                    }
                })
            };
            // узел на конце оси — точка OSM: сглаженная ось режется на ways
            // рядом с узлом, а не в нём (`continued` выше)
            let osm = &roads[road].points;
            let last = dense.len() - 1;
            let node_of = |index: usize| match index {
                0 => osm[0],
                _ if index == last => osm[osm.len() - 1],
                _ => dense[index],
            };
            let pinned: Vec<f32> = (0..dense.len())
                .filter(|&index| held(node_of(index)))
                .map(|index| along[index])
                .collect();
            // узлы запоминаются до сдвига: после него по вершине их не найти
            let shared: Vec<bool> = (0..dense.len())
                .map(|index| nodes.is_shared(node_of(index)))
                .collect();
            // и шов с продолжением: обе половины сдвигают его каждая сама, и
            // их концы расходились на сантиметры — узел по ним не находился
            let movable: Vec<Option<Vec2>> = (0..dense.len())
                .map(|index| {
                    let node = node_of(index);
                    (shared[index] && !held(node)).then_some(node)
                })
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
                let Some((run, near, near_along)) = span
                    .iter()
                    .filter(|run| run.from - RUN_BRIDGE <= at && at <= run.to + RUN_BRIDGE)
                    .filter_map(|run| {
                        let path = original[run.partner]
                            .as_deref()
                            .expect("ось пары сохранена до разводки");
                        nearest_on_path(path, *point).map(|(near, along)| (run, near, along))
                    })
                    .min_by(|a, b| a.1.distance(*point).total_cmp(&b.1.distance(*point)))
                else {
                    continue;
                };
                let partner = run.partner;
                let Some(outward) = (*point - near).try_normalize() else {
                    continue;
                };
                // через шов своей половины — к зазору продолжения: у самого
                // узла обе стороны берут середину между своими зазорами; на
                // шве с клином — по клину ([`seam_blend`])
                let mut gap = span_gap(span, at);
                if let (Some(before), true) = (ends[0], from == f32::NEG_INFINITY) {
                    gap = seam_blend(before, gap, at, wedged[0]);
                }
                if let (Some(after), true) = (ends[1], to == f32::INFINITY) {
                    gap = seam_blend(after, gap, total - at, wedged[1]);
                }
                // полуширины половин, обращённые друг к другу, — с клиньями
                let partner_path = original[partner]
                    .as_deref()
                    .expect("ось пары сохранена до разводки");
                let partner_left =
                    place_on_path(partner_path, &original_along[partner], near_along)
                        .is_some_and(|(_, heading)| heading.perp_dot(*point - near) > 0.0);
                let asphalt = facing(road, at, total, run.left)
                    + facing(partner, near_along, original_lengths[partner], partner_left);
                let wanted = point.midpoint(near) + outward * (asphalt + gap) / 2.0;
                *point += (wanted - *point) * weight;
            }
            // узлы остаются вершинами: по их точному месту их находят соседи
            let kept = simplify(&dense, false, SIMPLIFY_TOLERANCE, |index| shared[index]);
            moved.extend(
                movable
                    .iter()
                    .zip(&dense)
                    .filter_map(|(from, &to)| from.map(|from| (from, to)))
                    .filter(|(from, to)| from.distance(*to) > MOVE_EPSILON),
            );
            aligned.push((road, kept.into_iter().map(|index| dense[index]).collect()));
        }
        // узел, сдвинутый концом половины и началом её продолжения, — в одном
        // месте, середине сдвигов: скругления и стежки ищут его по вершине
        let mut meets: HashMap<(i32, i32), (Vec2, f32)> = HashMap::new();
        for (from, to) in &moved {
            let meet = meets.entry(node_key(*from)).or_insert((Vec2::ZERO, 0.0));
            meet.0 += *to;
            meet.1 += 1.0;
        }
        let place = |from: Vec2| {
            let (sum, count) = meets[&node_key(from)];
            sum / count
        };
        let met: HashMap<[u32; 2], Vec2> = moved
            .iter()
            .filter(|(from, _)| meets[&node_key(*from)].1 > 1.0)
            .map(|&(from, to)| (point_bits(to), place(from)))
            .collect();
        if !met.is_empty() {
            for point in aligned.iter_mut().flat_map(|(_, path)| path.iter_mut()) {
                if let Some(&place) = met.get(&point_bits(*point)) {
                    *point = place;
                }
            }
            align_seam_ends(&mut aligned, &met);
        }
        for (from, to) in &mut moved {
            *to = place(*from);
        }
        // чистые швы половин — где нарисованы: у них торцы разделительных
        // сводятся дальше ([`SEAM_JOIN_GAP`])
        let mut seams = Vec::new();
        for (road, path) in &aligned {
            let points = &roads[*road].points;
            for (node, drawn) in [
                (points[0], path[0]),
                (points[points.len() - 1], path[path.len() - 1]),
            ] {
                let streets: Vec<usize> = nodes
                    .roads_at(node)
                    .iter()
                    .copied()
                    .filter(|&other| roads[other].class == RoadClass::Street)
                    .collect();
                if let [a, b] = streets[..]
                    && continues(a, b)
                {
                    seams.push(drawn);
                }
            }
        }
        for (road, path) in aligned {
            paths[road] = Cow::Owned(path);
        }
        for median in &mut self.medians {
            lay_median(median, paths, roads, wedges);
        }
        self.join_ends(&seams);
        moved
    }

    /// Дороги узлов, уехавших с половиной ([`Self::align`]), идут следом:
    /// вершина узла — туда же, куда половина, соседние вершины — с затуханием
    /// на [`FOLLOW_FADE`], до следующего узла. Разводимые половины
    /// сдвинуты своей разводкой и не трогаются.
    ///
    /// Оси снимаются до сдвига: короткая улица между двумя уехавшими узлами
    /// ставит свои торцы на оба новых места, и второй сдвиг по живой оси
    /// находил у торца уже новое место первого узла — узел там ещё не
    /// зарегистрирован ([`RoadNodes::alias`] позже), гаснущий хвост доходил
    /// до торца и сдвигал его на сантиметры в сторону. Дорога выпадала из
    /// узла ([`RoadNodes::is_shared`] — точная ячейка), теряла плечо, и её
    /// круглый торец выходил за дугу бордюра (Тула, Фёдора Смирнова у
    /// Красноармейского).
    pub fn follow_moved_nodes(
        &self,
        paths: &mut [Cow<[Vec2]>],
        nodes: &RoadNodes,
        moved: &[(Vec2, Vec2)],
    ) {
        let originals: Vec<Vec<Vec2>> = paths.iter().map(|path| path.to_vec()).collect();
        let mut seen = HashSet::new();
        for &(from, to) in moved {
            let key = node_key(from);
            if !seen.insert(key) {
                continue;
            }
            let shift = to - from;
            for &road in nodes.roads_at(from) {
                if !self.runs[road].is_empty() {
                    continue;
                }
                let original = &originals[road];
                let Some(vertex) = original.iter().position(|point| node_key(*point) == key) else {
                    continue;
                };
                let points = paths[road].to_mut();
                points[vertex] = to;
                let after = (vertex + 1..original.len()).collect::<Vec<_>>();
                let before = (0..vertex).rev().collect::<Vec<_>>();
                for side in [after, before] {
                    let (mut at, mut walked) = (vertex, 0.0);
                    for next in side {
                        walked += original[next].distance(original[at]);
                        if walked >= FOLLOW_FADE || nodes.is_shared(original[next]) {
                            break;
                        }
                        points[next] += shift * (1.0 - smoothstep(walked / FOLLOW_FADE));
                        at = next;
                    }
                }
            }
        }
    }
}

/// Торцы двух половин, сведённых в одну точку `met`, — на общую касательную:
/// у каждой вершина в [`SEAM_TAIL`] от шва по биссектрисе их направлений.
/// Ленты кончаются торцом поперёк своей оси, и при изломе в пару градусов
/// (картограф свёл мост и подход не по прямой — Красноармейский над каналом)
/// угол бордюра настила заходил на асфальт подхода зубцом; излом уходит
/// внутрь лент, где его кроют их соединения.
fn align_seam_ends(aligned: &mut [(usize, Vec<Vec2>)], met: &HashMap<[u32; 2], Vec2>) {
    let places: HashSet<[u32; 2]> = met.values().map(|place| point_bits(*place)).collect();
    let mut at_place: HashMap<[u32; 2], Vec<(usize, bool)>> = HashMap::new();
    for (slot, (_, path)) in aligned.iter().enumerate() {
        if path.len() < 2 {
            continue;
        }
        for end in [false, true] {
            let point = if end { path[path.len() - 1] } else { path[0] };
            if places.contains(&point_bits(point)) {
                at_place
                    .entry(point_bits(point))
                    .or_default()
                    .push((slot, end));
            }
        }
    }
    // направление от шва вдоль оси
    let away = |path: &[Vec2], end: bool| {
        let (node, next) = if end {
            (path[path.len() - 1], path[path.len() - 2])
        } else {
            (path[0], path[1])
        };
        (next - node).normalize_or_zero()
    };
    for ends in at_place.into_values() {
        let [(first, first_end), (second, second_end)] = ends[..] else {
            continue;
        };
        let (a, b) = (
            away(&aligned[first].1, first_end),
            away(&aligned[second].1, second_end),
        );
        if a.dot(-b) < SEAM_MAX_BEND.cos() {
            continue;
        }
        let Some(tangent) = (a - b).try_normalize() else {
            continue;
        };
        for (slot, end, direction) in [(first, first_end, tangent), (second, second_end, -tangent)]
        {
            let path = &mut aligned[slot].1;
            let (node, neighbour, at) = if end {
                (path[path.len() - 1], path[path.len() - 2], path.len() - 1)
            } else {
                (path[0], path[1], 1)
            };
            // соседняя вершина может быть узлом — не трогается, новая встаёт
            // между ней и швом
            let reach = SEAM_TAIL.min(neighbour.distance(node) / 2.0);
            path.insert(at, node + direction * reach);
        }
    }
}

/// Плавный переход разводки: 0 у конца куска или у узла, 1 за
/// [`ALIGN_TRANSITION`] от них.
fn ease(distance: f32) -> f32 {
    smoothstep(distance / ALIGN_TRANSITION)
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
    before + (after - before) * smoothstep(beyond / ALIGN_TRANSITION + 0.5)
}

/// Какая сторона шва половины несёт клин (`roads/tapers.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum SeamWedge {
    /// Этот way — широкий, клин длиной столько метров идёт от узла по нему.
    Wide(f32),
    /// Клин лежит на продолжении, этот way — узкий.
    Narrow,
}

/// Зазор у шва половины в `beyond` м от узла внутрь своего way: `across` —
/// зазор продолжения за узлом, `own` — свой. Без клина — [`blend`] поперёк
/// узла. С клином переход ложится **на клин**: на узкой стороне зазор свой до
/// самого узла, на широкой — от зазора узкой в узле к своему к концу клина,
/// линейно, как и сам клин. Тогда обе кромки клина — прямые, а переход
/// поперёк узла вместе с клином гнул внешнюю кромку сперва внутрь, потом
/// наружу.
fn seam_blend(across: f32, own: f32, beyond: f32, wedge: Option<SeamWedge>) -> f32 {
    match wedge {
        None => blend(across, own, beyond),
        Some(SeamWedge::Wide(length)) => {
            across + (own - across) * (beyond / length).clamp(0.0, 1.0)
        }
        Some(SeamWedge::Narrow) => own,
    }
}

/// Ломаная с вершинами не реже `step` и длина дуги в каждой вершине.
/// Исходные вершины остаются на месте.
///
/// Точки — те же, что у `along::densify`, но **длины — нет**, и потому это
/// своя функция, а не `along::densify` + `along::arclengths`. Здесь длина —
/// `start + length·t` по звену OSM, там — сумма расстояний между соседними
/// догущёнными точками; разница в последних битах, но выравнивание половин
/// на неё отзывается: замена сдвинула асфальт, тротуары и газоны
/// разделительных на всех четырёх проверенных городах (Берлин — +38
/// вершин дорожных слоёв).
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
