//! Планки зебр: разрыв чужих линий поперёк планки, слияние половин и
//! переходов по данным, дубли.

use bevy::prelude::*;

use super::{Arm, CLUSTER_CELL, EDGE_INSET, PAINT_CLEAR, Walk, ZEBRA_LENGTH, ZEBRA_SETBACK, Zebra};
use crate::map::along::arclengths;
use crate::map::grid::Grid;
use crate::map::meshing::Break;
use crate::map::osm::RoadLine;
use crate::map::roads::is_carriageway;
use crate::map::roads::network::pairs::TRAM_BED_MAX_GAP;

/// Насколько зебры могут зайти одна на другую краями, м.
const OVERLAP_SLACK: f32 = 0.2;
/// Зебры двух половин сливаются в одну планку ([`join_zebras`]), если они
/// параллельны (косинус), лежат на одной прямой с точностью до метра и между
/// ними не шире самой широкой асфальтовой разделительной с отступами
/// [`EDGE_INSET`] от обеих кромок. Самая широкая — трамвайное полотно, оно
/// мощёное до [`TRAM_BED_MAX_GAP`] (разделительная по ручке `Median gap` —
/// до 6 м), и [`OVERLAP_SLACK`] на то, что планки ложатся не по пробе.
const JOIN_PARALLEL: f32 = 0.95;
const JOIN_OFFSET: f32 = 1.0;
const JOIN_GAP: f32 = TRAM_BED_MAX_GAP + 2.0 * EDGE_INSET + OVERLAP_SLACK;
/// Зебры по данным на разных проезжих частях, чьи торцы ближе этого, м, —
/// одна планка ([`join_touching`]): порядка ширины самой зебры.
const TOUCH_GAP: f32 = ZEBRA_LENGTH;
/// Косинус наибольшего угла между планкой и общей хордой пары, чтобы она ещё
/// продолжала другую, — 30°.
const TOUCH_ALIGN: f32 = 0.866;

/// Ячейка сетки планок ([`breaks_across_zebras`]), м.
const ZEBRA_PLANK_CELL: f32 = 24.0;
/// Полуширина самой широкой улицы с запасом, м: на столько планка в сетке
/// раздувается, чтобы ссылка звена любой дороги нашла её (запрос раздувается
/// на полуширину дороги и [`PAINT_CLEAR`]).
const WIDEST_HALF: f32 = 16.0;

/// Планка зебры режет и **чужие** линии: разрыв получает только своя дорога
/// (`paint_crossing`, планка плеча), и сквозь полотно другой проезжей части,
/// чьё сечение через планку идёт, тянулись первые штрихи её пунктира — у
/// Y-слияния двух односторонних в двустороннюю зебра стояла в метрах от узла,
/// и ветки начинались короткими косыми обрывками прямо от неё (Тула,
/// Y-развилка у кольца, N6). Каждой проезжей части, чьё сечение (ось ±
/// полуширина) заходит в прямоугольник планки с запасом [`PAINT_CLEAR`], —
/// разрыв на этом интервале; короткие куски после него закрывает общий проход
/// [`bridge_short_runs`]. Плечи своего узла не задеваются: планка плеча и так
/// стоит за кромкой ([`ArmPlan`]), в метре чужого асфальта, и [`PAINT_CLEAR`]
/// до неё не дотягивает. Мост не режется: его асфальт — другой слой, и
/// на нём линии свои. Зебры здесь ещё не слиты в одну планку и не
/// прорежены: продублированная лежит в другой с точностью до
/// [`OVERLAP_SLACK`], слитая накрывает обе — с запасом [`PAINT_CLEAR`]
/// разницы нет.
pub(super) fn breaks_across_zebras(
    drawn: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    zebras: &[Zebra],
    zebra_roads: &[usize],
    breaks: &mut [Vec<Break>],
) {
    if zebras.is_empty() {
        return;
    }
    let reach = ZEBRA_LENGTH / 2.0 + PAINT_CLEAR;
    let mut planks: Grid<usize> = Grid::new(ZEBRA_PLANK_CELL);
    for (index, zebra) in zebras.iter().enumerate() {
        planks.insert_segment(zebra.from, zebra.to, reach + WIDEST_HALF, index);
    }
    for (road, path) in paths.iter().enumerate() {
        let path = path.as_ref();
        if path.len() < 2 || !is_carriageway(drawn[road]) || drawn[road].bridge {
            continue;
        }
        let half = drawn[road].width / 2.0;
        let (along, _) = arclengths(path);
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for link in 0..path.len() - 1 {
            let (a, b) = (path[link], path[link + 1]);
            let d = b - a;
            let len = d.length();
            if len <= 0.0 {
                continue;
            }
            let (min, max) = (a.min(b), a.max(b));
            let pad = half + PAINT_CLEAR;
            if planks
                .near_each(min - Vec2::splat(pad), max + Vec2::splat(pad))
                .next()
                .is_none()
            {
                continue;
            }
            let normal = (d / len).perp();
            for &index in planks.near_each(min - Vec2::splat(pad), max + Vec2::splat(pad)) {
                let zebra = &zebras[index];
                if zebra_roads.get(index) == Some(&road) {
                    continue;
                }
                // сечение дороги (ось ± полуширина поперёк её хода) входит в
                // прямоугольник планки с запасом: по каждой оси планки —
                // |t·d·axis + (a−центр)·axis| ≤ полуразмер + [`PAINT_CLEAR`]
                // + полуширина·|нормаль звена·axis|, интервал по t
                let u = match (zebra.to - zebra.from).try_normalize() {
                    Some(u) => u,
                    None => continue,
                };
                let centre = (zebra.from + zebra.to) / 2.0;
                let axes = [
                    (u, zebra.from.distance(zebra.to) / 2.0 + PAINT_CLEAR),
                    (u.perp(), ZEBRA_LENGTH / 2.0 + PAINT_CLEAR),
                ];
                let mut span = (0.0_f32, 1.0_f32);
                for (axis, limit) in axes {
                    let limit = limit + half * normal.dot(axis).abs();
                    let offset = (a - centre).dot(axis);
                    let run = d.dot(axis);
                    let bound = (span.0, span.1);
                    span = if run.abs() < 1e-6 {
                        if offset.abs() <= limit {
                            bound
                        } else {
                            (1.0, 0.0)
                        }
                    } else {
                        let mut low = (-limit - offset) / run;
                        let mut high = (limit - offset) / run;
                        if low > high {
                            std::mem::swap(&mut low, &mut high);
                        }
                        (bound.0.max(low), bound.1.min(high))
                    };
                    if span.0 >= span.1 {
                        break;
                    }
                }
                if span.0 < span.1 {
                    spans.push((along[link] + span.0 * len, along[link] + span.1 * len));
                }
            }
        }
        if spans.is_empty() {
            continue;
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let walk = Walk::new(path);
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(spans.len());
        for &(low, high) in &spans {
            match merged.last_mut() {
                Some(last) if low <= last.1 + 0.1 => last.1 = last.1.max(high),
                _ => merged.push((low, high)),
            }
        }
        for (low, high) in merged {
            breaks[road].extend(walk.gap(low, high));
        }
    }
}

/// Плечо, которое рвётся, и где на нём встанет зебра: длина центра и по
/// данным ли она.
pub(super) struct ArmPlan<'a> {
    pub(super) arm: Arm,
    pub(super) walk: Walk<'a>,
    /// Длина кромки узла на пути плеча.
    pub(super) edge: f32,
    /// Переход OSM, которым стала зебра, — индекс в переходах дороги.
    pub(super) osm: Option<usize>,
    pub(super) zebra: Option<(f32, bool)>,
    /// Плечо — перемычка сложного узла ([`JunctionArm::link`]).
    pub(super) link: bool,
    /// Въезд в кольцо: линия по его кромке ([`RingEntry::line`]).
    pub(super) ring_line: Option<(Vec2, Vec2)>,
    /// Сколько дороги от кромки до асфальта следующего узла на плече, м
    /// (бесконечность — узла дальше нет).
    pub(super) next_edge: f32,
}

/// Въезд в кольцо: где подход выходит из асфальта самого кольца.
pub(super) struct RingEntry {
    /// Длина на пути плеча, где из кольца вышло всё сечение.
    pub(super) edge: f32,
    /// Отрезок линии уступи дорогу — от середины (у односторонней — от левой
    /// кромки) к бордюру, каждый конец там, где из кольца вышла его сторона.
    pub(super) line: (Vec2, Vec2),
}

impl ArmPlan<'_> {
    /// Центр зебры и направление от узла наружу.
    fn zebra_frame(&self) -> Option<(Vec2, Vec2)> {
        let (center, _) = self.zebra?;
        let (point, direction) = self.walk.at(center)?;
        Some((point, direction * self.arm.dir))
    }
}

/// Зебры двух половин одной улицы — на одну линию поперёк неё: вторая
/// сдвигается к той, что по данным, а если обе по правилу — к дальней от
/// узла. Два перехода OSM — по узлу на каждой половине — маппер ставит не на
/// одну прямую (на 02 они разошлись на метр), и тогда обе встают посередине
/// между ними: сдвиг в метр остаётся внутри длины самой планки. Не ближе
/// кромки узла.
pub(super) fn align_pair(plans: &mut [ArmPlan], a: usize, b: usize) {
    let (Some((point_a, out_a)), Some((point_b, out_b))) =
        (plans[a].zebra_frame(), plans[b].zebra_frame())
    else {
        return;
    };
    if out_a.dot(out_b) < 0.5 {
        return;
    }
    let out = (out_a + out_b).normalize();
    let osm = |plan: &ArmPlan| plan.zebra.is_some_and(|(_, osm)| osm);
    let (line, moved) = match (osm(&plans[a]), osm(&plans[b])) {
        (true, true) => (
            (point_a.dot(out) + point_b.dot(out)) / 2.0,
            vec![(a, point_a, out_a), (b, point_b, out_b)],
        ),
        (true, false) => (point_a.dot(out), vec![(b, point_b, out_b)]),
        (false, true) => (point_b.dot(out), vec![(a, point_a, out_a)]),
        (false, false) if point_a.dot(out) >= point_b.dot(out) => {
            (point_a.dot(out), vec![(b, point_b, out_b)])
        }
        (false, false) => (point_b.dot(out), vec![(a, point_a, out_a)]),
    };
    for (index, from, along) in moved {
        let shift = (line - from.dot(out)) / along.dot(out);
        let plan = &mut plans[index];
        let dir = plan.arm.dir;
        let Some((center, osm)) = plan.zebra else {
            continue;
        };
        let nearest = plan.edge + dir * (ZEBRA_SETBACK + ZEBRA_LENGTH / 2.0);
        let center = center + dir * shift;
        plan.zebra = Some((
            if (center - nearest) * dir < 0.0 {
                nearest
            } else {
                center
            },
            osm,
        ));
    }
}

/// Зебра поперёк `road` на длине `at`.
pub(super) fn zebra_at(walk: &Walk, road: &RoadLine, at: f32, osm: bool) -> Option<Zebra> {
    let (point, direction) = walk.at(at)?;
    let across = direction.perp() * (road.width / 2.0 - EDGE_INSET);
    Some(Zebra {
        from: point - across,
        to: point + across,
        osm,
    })
}

/// Одна планка из зебр двух половин: от дальнего края одной до дальнего края
/// другой. Только если они легли на одну прямую ([`align_pair`]) и между ними
/// не больше [`JOIN_GAP`] — иначе это не пара через узкую разделительную.
pub(super) fn join_zebras(a: &Zebra, b: &Zebra) -> Option<Zebra> {
    let across = (a.to - a.from).try_normalize()?;
    let theirs = (b.to - b.from).try_normalize()?;
    if across.dot(theirs).abs() < JOIN_PARALLEL
        || (b.from - a.from).perp_dot(across).abs() > JOIN_OFFSET
        || (b.to - a.from).perp_dot(across).abs() > JOIN_OFFSET
    {
        return None;
    }
    let ends = [a.from, a.to, b.from, b.to];
    let along = ends.map(|point| (point - a.from).dot(across));
    let (low, high) = along
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &at| {
            (low.min(at), high.max(at))
        });
    // зазор между половинами: вся длина минус обе планки
    let gap = high - low - a.from.distance(a.to) - b.from.distance(b.to);
    (gap <= JOIN_GAP).then(|| Zebra {
        from: a.from + across * low,
        to: a.from + across * high,
        osm: a.osm || b.osm,
    })
}

/// Две зебры по данным на разных проезжих частях, чьи торцы почти касаются, —
/// одной сплошной планкой через обе (roads list R21): переход OSM через обе
/// половины Красноармейского у островка (Тула, 3122 3876) — по узлу на каждой
/// половине, и каждая зебра вставала поперёк своей, со сдвигом, торец к торцу —
/// ступенькой. Пара — два торца ближе [`TOUCH_GAP`], и обе планки смотрят
/// вдоль общей хорды не круче [`TOUCH_ALIGN`]: не бок о бок вдоль улицы и не
/// углом. Новая планка — от дальнего торца одной до дальнего торца другой,
/// одним направлением. Половины пары на газоне ([`Partner::paved`]) остаются
/// двумя зебрами, каждая до своей кромки, — `lawn(a, b)`. Пары жадно, по
/// возрастанию зазора; каждая зебра — в одной паре.
pub(super) fn join_touching(
    zebras: Vec<Zebra>,
    roads: &[usize],
    lawn: impl Fn(usize, usize) -> bool,
) -> Vec<Zebra> {
    debug_assert_eq!(zebras.len(), roads.len());
    let mut near: Grid<usize> = Grid::new(CLUSTER_CELL);
    for (index, zebra) in zebras.iter().enumerate().filter(|(_, zebra)| zebra.osm) {
        near.insert_segment(zebra.from, zebra.to, TOUCH_GAP, index);
    }
    let mut pairs: Vec<(f32, usize, usize, Zebra)> = Vec::new();
    for (a, zebra) in zebras.iter().enumerate().filter(|(_, zebra)| zebra.osm) {
        let (min, max) = (zebra.from.min(zebra.to), zebra.from.max(zebra.to));
        for b in near.near(min - TOUCH_GAP, max + TOUCH_GAP) {
            if b <= a || roads[a] == roads[b] || lawn(roads[a], roads[b]) {
                continue;
            }
            if let Some((gap, one)) = touching(zebra, &zebras[b]) {
                pairs.push((gap, a, b, one));
            }
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
    let mut taken = vec![false; zebras.len()];
    let mut joined = Vec::new();
    for (_, a, b, one) in pairs {
        if taken[a] || taken[b] {
            continue;
        }
        taken[a] = true;
        taken[b] = true;
        joined.push(one);
    }
    // сплошные — первыми: переход в узле шва стоит на обеих дорогах шва, и
    // вторую такую же зебру снимет [`without_overlaps`], а не планку
    joined
        .into_iter()
        .chain(
            zebras
                .into_iter()
                .zip(taken)
                .filter(|(_, taken)| !taken)
                .map(|(zebra, _)| zebra),
        )
        .collect()
}

/// Зазор между ближними торцами двух зебр и планка от дальнего торца одной до
/// дальнего другой — если они почти касаются и лежат вдоль этой планки.
fn touching(a: &Zebra, b: &Zebra) -> Option<(f32, Zebra)> {
    let (gap, far_a, far_b) = [(a.to, a.from), (a.from, a.to)]
        .into_iter()
        .flat_map(|(near_a, far_a)| {
            [(b.from, b.to), (b.to, b.from)]
                .map(|(near_b, far_b)| (near_a.distance(near_b), far_a, far_b))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))?;
    if gap > TOUCH_GAP {
        return None;
    }
    let chord = (far_b - far_a).try_normalize()?;
    let aligned = |zebra: &Zebra| {
        (zebra.to - zebra.from)
            .try_normalize()
            .is_some_and(|across| across.dot(chord).abs() >= TOUCH_ALIGN)
    };
    // ближние торцы — между дальними: планки идут друг за другом, а не
    // одна поверх другой
    let length = far_a.distance(far_b);
    let beyond = |zebra: &Zebra| zebra.from.distance(zebra.to) >= length;
    if !aligned(a) || !aligned(b) || beyond(a) || beyond(b) {
        return None;
    }
    Some((
        gap,
        Zebra {
            from: far_a,
            to: far_b,
            osm: true,
        },
    ))
}

/// Зебры, что легли одна на другую — две ветки развилки у одного узла,
/// переход OSM рядом с правилом, — одной: по данным остаётся, из
/// сгенерированных — первая. Зебры двух половин одной улицы стоят бок о бок
/// и друг друга не задевают.
pub(super) fn without_overlaps(zebras: Vec<Zebra>) -> Vec<Zebra> {
    let mut order: Vec<usize> = (0..zebras.len()).collect();
    order.sort_by_key(|&index| !zebras[index].osm);
    let bounds = |zebra: &Zebra| {
        let pad = ZEBRA_LENGTH / 2.0;
        (
            zebra.from.min(zebra.to) - pad,
            zebra.from.max(zebra.to) + pad,
        )
    };
    let mut kept: Vec<Zebra> = Vec::with_capacity(zebras.len());
    let mut near: Grid<usize> = Grid::new(CLUSTER_CELL);
    for index in order {
        let zebra = zebras[index];
        let (min, max) = bounds(&zebra);
        if !near
            .near_each(min, max)
            .any(|&other| overlaps(&zebra, &kept[other]) || overlaps(&kept[other], &zebra))
        {
            near.insert(min, max, kept.len());
            kept.push(zebra);
        }
    }
    kept
}

/// Задевает ли отрезок зебры `a` плашку зебры `b`.
fn overlaps(a: &Zebra, b: &Zebra) -> bool {
    let span = b.to - b.from;
    let Some(across) = span.try_normalize() else {
        return false;
    };
    let along = across.perp();
    let center = (b.from + b.to) / 2.0;
    let (half_span, half_length) = (span.length() / 2.0, ZEBRA_LENGTH / 2.0);
    (0..=8).any(|step| {
        let point = a.from.lerp(a.to, step as f32 / 8.0) - center;
        point.dot(across).abs() < half_span - OVERLAP_SLACK
            && point.dot(along).abs() < half_length - OVERLAP_SLACK
    })
}
