//! **Устье пары на перекрёстке** — нечистое слияние (R33): в узле, где обе
//! половины разделённой улицы кончаются на двустороннем продолжении, есть ещё
//! поперечная улица. OSM сводит оси половин в точку узла, и половины сходились
//! клином прямо в перекрёсток: восточная половина Попова в Белгороде
//! (201967599) — с изломом в 10 м от узла, и клин слияния по её оси вспухал и
//! загибался, а зебра по данным лежала на одной западной.
//!
//! Здесь такая пара доходит до перекрёстка **параллельно**: каждая половина —
//! на своей стороне устья продолжения, в `полуширина продолжения − своя
//! полуширина` от его оси, так что наружные кромки половин продолжают
//! кромки продолжения, а внутренние сходятся на его оси. Торец половины
//! встаёт на ось поперечной улицы — **своим узлом** на ней (вершина
//! вставляется в её ось, [`RoadNodes::insert`]): для скруглений, разрывов и
//! краски это обычная разделённая улица у перекрёстка, два примыкания рядом,
//! которые краска узла склеивает в один кластер (зебры пары — в одну линию). Узел
//! слиянием больше не находится — клина слияния нет.
//!
//! Только нарисованные оси: навмеш и разбор видят OSM как есть.

use std::borrow::Cow;

use bevy::prelude::*;

use super::{away, merges};
use crate::map::along::densify;
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;
use crate::map::roads::junctions::node_key;
use crate::map::roads::network::pairs::Pairs;
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::tapers::Tapers;
use crate::map::roads::{is_carriageway, smoothstep};

/// Сдвиг устья меньше этого, м, не разводится: половины и так у кромок.
const MOUTH_MIN_OFFSET: f32 = 0.5;
/// Косинус угла между лучом поперечной улицы и нормалью продолжения, от
/// которого устье ложится на этот луч: косой луч унёс бы торец далеко вбок.
const MOUTH_ARM_ALIGN: f32 = 0.5;
/// Сколько прямого пути оставляет устье у узла, м: половина идёт
/// параллельно продолжению, скругление бордюра ложится на прямой край.
const MOUTH_STRAIGHT: f32 = 16.0;
/// Длина, на которой сдвиг половины сходит на нет за прямым участком, м.
const MOUTH_FADE: f32 = 20.0;
/// Шаг, которым ось половины догущается под сдвиг, м.
const MOUTH_STEP: f32 = 2.0;

/// Развести половины нечистых слияний по устьям на поперечной улице —
/// сдвигом нарисованных осей `paths` и новыми узлами в `nodes`. Отдаёт,
/// сколько слияний разведено.
pub fn split_mouths(
    roads: &[RoadLine],
    paths: &mut [Cow<[Vec2]>],
    nodes: &mut RoadNodes,
    pairs: &mut Pairs,
    network: &RoadNetwork,
    wedges: &Tapers,
) -> usize {
    let refs: Vec<&RoadLine> = roads.iter().collect();
    let found = merges(&refs, paths, nodes, pairs, network);
    let mut split = 0;
    for merge in found.list.iter().filter(|merge| !merge.pure) {
        let node = merge.node;
        let key = node_key(node);
        let street = merge.street;
        // ось пары у узла — биссектриса её половин (обе смотрят от узла), а
        // не продолжения: продолжение бывает под углом к паре, и половины,
        // сведённые параллельно ему, гнулись бы за прямым участком
        let headings = [
            away(paths[merge.halves[0]].as_ref(), true),
            away(paths[merge.halves[1]].as_ref(), false),
        ];
        let Some(along) = (headings[0] + headings[1]).try_normalize() else {
            continue;
        };
        let normal = along.perp();
        // лучи поперечных улиц: дорога, вершина узла, соседняя вершина
        let mut arms: Vec<(usize, usize, usize, Vec2)> = Vec::new();
        for &other in nodes.roads_at(node) {
            if merge.roads().contains(&other) || !is_carriageway(&roads[other]) {
                continue;
            }
            let path = paths[other].as_ref();
            let Some(vertex) = path.iter().position(|point| node_key(*point) == key) else {
                continue;
            };
            for next in [vertex.wrapping_sub(1), vertex + 1] {
                if let Some(direction) = path.get(next).and_then(|&point| (point - node).try_normalize())
                {
                    arms.push((other, vertex, next, direction));
                }
            }
        }
        // устье каждой половины: луч, вдоль которого её сторона, и точка на нём
        let mouths: Vec<Option<Mouth>> = merge
            .halves
            .iter()
            .enumerate()
            .map(|(index, &half)| {
                let offset = (roads[street].width - roads[half].width) / 2.0;
                if offset < MOUTH_MIN_OFFSET {
                    return None;
                }
                // въезжающая половина кончается в узле, выезжающая начинается
                let at_end = index == 0;
                let other = headings[1 - index];
                let side = if (headings[index] - other).dot(normal) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                let (road, vertex, next, direction) = arms
                    .iter()
                    .copied()
                    .max_by(|a, b| (a.3.dot(normal) * side).total_cmp(&(b.3.dot(normal) * side)))?;
                let cosine = direction.dot(normal) * side;
                if cosine < MOUTH_ARM_ALIGN {
                    return None;
                }
                let reach = offset / cosine;
                let link = paths[road][next].distance(node);
                if reach >= link - MOUTH_STEP {
                    return None;
                }
                Some(Mouth {
                    half,
                    at_end,
                    side,
                    offset,
                    road,
                    vertex,
                    next,
                    at: node + direction * reach,
                })
            })
            .collect();
        let [Some(first), Some(second)] = [mouths[0], mouths[1]] else {
            continue;
        };
        // вершина устья — в ось поперечной улицы; второе устье на той же
        // дороге ищет свою вершину заново: первая вставка сдвинула индексы
        for mouth in [first, second] {
            let path = paths[mouth.road].to_mut();
            let Some(vertex) = path.iter().position(|point| node_key(*point) == key) else {
                continue;
            };
            let slot = if mouth.next > mouth.vertex {
                vertex + 1
            } else {
                vertex
            };
            path.insert(slot, mouth.at);
            nodes.insert(mouth.at, vec![mouth.half, mouth.road]);
        }
        for mouth in [first, second] {
            let shifted = shift_half(paths[mouth.half].as_ref(), node, normal, mouth);
            paths[mouth.half] = Cow::Owned(shifted);
        }
        // пара — и у самого устья: без тротуара между половинами, зебры
        // обеих в одну линию; середина — по сдвинутым осям
        let halves = [first.half, second.half];
        let totals = halves.map(|half| polyline_length(paths[half].as_ref()));
        // пара слева от половины: по ходу её точек — сторона, противоположная
        // её собственной по нормали; у въезжающей путь идёт к узлу
        let left = [first, second].map(|mouth| {
            let toward = if mouth.at_end { -along } else { along };
            toward.perp().dot(normal * -mouth.side) > 0.0
        });
        pairs.join_mouth(
            halves,
            [MOUTH_STRAIGHT; 2],
            totals,
            [first.at_end, second.at_end],
            left,
        );
        for half in halves {
            pairs.relay_medians(half, paths, roads, wedges);
        }
        split += 1;
    }
    split
}

/// Устье одной половины: куда встаёт её торец и на какой стороне оси
/// продолжения (`side` — знак по нормали) она идёт.
#[derive(Clone, Copy)]
struct Mouth {
    half: usize,
    /// Узел — конец пути половины (въезжающая), иначе начало.
    at_end: bool,
    side: f32,
    offset: f32,
    road: usize,
    vertex: usize,
    next: usize,
    at: Vec2,
}

/// Ось половины `path`, сведённая к устью: торец — в `mouth.at`, у узла
/// `node` ось на `mouth.offset` от оси продолжения по нормали `normal`
/// ([`MOUTH_STRAIGHT`]), дальше сдвиг сходит на нет за [`MOUTH_FADE`].
fn shift_half(path: &[Vec2], node: Vec2, normal: Vec2, mouth: Mouth) -> Vec<Vec2> {
    let mut walk: Vec<Vec2> = path.to_vec();
    if mouth.at_end {
        walk.reverse();
    }
    // догущается только участок сдвига: дальше вершины как были
    let reach = MOUTH_STRAIGHT + MOUTH_FADE;
    let mut travelled = 0.0;
    let mut cut = walk.len();
    for (index, link) in walk.windows(2).enumerate() {
        travelled += link[0].distance(link[1]);
        if travelled >= reach {
            cut = index + 2;
            break;
        }
    }
    let mut dense = densify(&walk[..cut.min(walk.len())], MOUTH_STEP);
    dense.extend_from_slice(&walk[cut.min(walk.len())..]);
    let mut out = Vec::with_capacity(dense.len());
    let mut along = 0.0;
    let mut previous = dense[0];
    for (index, &point) in dense.iter().enumerate() {
        along += point.distance(previous);
        previous = point;
        if index == 0 {
            out.push(mouth.at);
            continue;
        }
        // только наружу: где пара уже разошлась шире устья, ось на месте
        let weight = 1.0 - smoothstep(((along - MOUTH_STRAIGHT) / MOUTH_FADE).clamp(0.0, 1.0));
        let lateral = (point - node).dot(normal) * mouth.side;
        let push = (mouth.offset - lateral).max(0.0);
        out.push(point + normal * mouth.side * push * weight);
    }
    if mouth.at_end {
        out.reverse();
    }
    out
}
