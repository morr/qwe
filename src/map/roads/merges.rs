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

use super::network::RoadNodes;
use super::network::pairs::PairRun;
use crate::map::meshing::miter_offsets;
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;

/// Косинус угла, в котором обе половины уходят от узла в одну сторону, а
/// продолжение — в обратную: 40°. Половины в OSM сходятся к узлу клином
/// градусов в 10–30, продолжение смотрит почти ровно назад.
const MERGE_ALIGN: f32 = 0.766;
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
}

/// Слияния по **нарисованным** осям `paths`: в узле кончаются две половины
/// одной пары (`runs` — куски пар по дорогам), одна въезжает, другая
/// выезжает, обе уходят от узла в одну сторону, а двусторонний way того же
/// `Highway` — в обратную. Мосты и арки (`carves_navmesh`) не участвуют.
pub fn merges(
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    nodes: &RoadNodes,
    runs: &[Vec<PairRun>],
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
        let partner = at_node.iter().copied().find(|&other| {
            let path = paths[other].as_ref();
            other != half
                && runs[half].iter().any(|run| run.partner == other)
                && roads[other].oneway
                && roads[other].highway == road.highway
                && path.len() >= 2
                && path[0] == node
                && away(path, false).dot(into) > MERGE_ALIGN
        });
        let Some(partner) = partner else {
            continue;
        };
        let out = away(paths[partner].as_ref(), false);
        let mean = (into + out).normalize_or_zero();
        let street = at_node.iter().copied().find_map(|other| {
            let candidate = roads[other];
            let path = paths[other].as_ref();
            if candidate.oneway
                || candidate.carves_navmesh()
                || candidate.highway != road.highway
                || path.len() < 2
            {
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
        });
        let Some((street, street_end)) = street else {
            continue;
        };
        found.ends[half][1] = true;
        found.ends[partner][0] = true;
        found.ends[street][street_end] = true;
        found.list.push(Merge {
            node,
            halves: [half, partner],
            street,
            street_end,
        });
    }
    found
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
/// где у половины снаружи тротуар шириной `sidewalk(половина)`, тротуар за
/// ним. `left(половина, пара)` — лежит ли пара слева по ходу половины.
/// `per_meter` — длина клина на метр разницы ширин.
pub fn merge_bands(
    merge: &Merge,
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    left: impl Fn(usize, usize) -> Option<bool>,
    sidewalk: impl Fn(usize) -> Option<f32>,
    per_meter: f32,
) -> Vec<MergeBand> {
    let wide = roads[merge.street].width / 2.0;
    let [first, second] = merge.halves;
    let mut bands = Vec::new();
    for (half, partner, from_end) in [(first, second, true), (second, first, false)] {
        let road = roads[half];
        let narrow = road.width / 2.0;
        let step = wide - narrow;
        if step < MERGE_MIN_STEP {
            continue;
        }
        let Some(partner_left) = left(half, partner) else {
            continue;
        };
        // путь от узла; снаружи — сторона, противоположная паре. У пути,
        // развёрнутого к узлу, лево и право меняются местами
        let mut path = paths[half].as_ref().to_vec();
        if from_end {
            path.reverse();
        }
        let outward = if partner_left != from_end { -1.0 } else { 1.0 };
        let length = (2.0 * step * per_meter).min(polyline_length(&path) * MERGE_MAX_SHARE);
        if length < MERGE_STEP {
            continue;
        }
        let (points, along) = resample(&path, length);
        let normals: Vec<Vec2> = miter_offsets(&points, false, 1.0)
            .into_iter()
            .map(|normal| normal * outward)
            .collect();
        // у узла — кромка продолжения, за клином — своя
        let reach = |at: f32| {
            let t = (at / length).clamp(0.0, 1.0);
            wide - step * t * t * (3.0 - 2.0 * t)
        };
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
        // тротуар по тегу — на той стороне, что снаружи: `[слева, справа]`
        let tagged = road.sidewalks[usize::from(partner_left)];
        bands.push(MergeBand {
            half,
            length,
            asphalt: band(narrow - MERGE_OVERLAP, 0.0),
            sidewalk: sidewalk(half)
                .filter(|_| tagged)
                .map(|width| band(narrow, width)),
        });
    }
    bands
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
