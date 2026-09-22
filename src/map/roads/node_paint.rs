//! **Краска узла** — что слой краски (`roads/paint.rs`) делает на
//! перекрёстке: где линии рвутся, а где главная проходит узел насквозь,
//! зебры и стоп-линии на плечах, карман лишней полосы.
//!
//! Разрывы асфальта (`roads/junctions.rs`) режут каждую сошедшуюся дорогу:
//! по ним гаснет колея и кончаются разделительные. Краске этого мало и
//! слишком много сразу:
//!
//! - **сближенные узлы — один узел.** Узлы, чьи зоны (полуширина самой
//!   широкой дороги плюс [`CLUSTER_ZONE`]) перекрываются, склеиваются в
//!   **кластер**: одни плечи, одна сквозная пара, один разрыв на дорогу. На
//!   улице Циолковского (Тула, 4703, 332) Щорса и переулок примыкают с разных
//!   сторон в 17 м друг от друга, и два разрыва подряд оставляли между собой
//!   осиротевший штрих;
//! - **главная не теряет разметку.** Дорога рвётся в кластере, только если
//!   она в нём кончается или её пересекает (проходит насквозь) дорога не ниже
//!   рангом, либо примыкает дорога выше рангом. Ранг — класс `highway`, знак
//!   `stop`/`give_way` на плече понижает его на полступени. Примыкание
//!   второстепенной улицы линий главной не рвёт; половина разделённой улицы
//!   своей второй половине не соперник. Светофор в кластере рвёт всех;
//! - **зебра и стоп-линия на плече**, которое рвётся: зебра — по узлу
//!   `highway=crossing` на плече (до [`ARM_CROSSING_REACH`]), иначе
//!   ([`CrossingMode::Generated`]) — в [`ZEBRA_SETBACK`] от кромки узла, если
//!   в кластере сошлись две улицы с тротуарами. Стоп-линия — за зеброй, на
//!   встречных узлу полосах; у `give_way` прерывистая. Дворовых проездов тут
//!   нет вовсе: они не проезжая часть и узлов не образуют;
//! - **зебра посреди квартала** — по любому размеченному переходу на улице,
//!   со стоп-линиями с обеих сторон, если переход регулируемый;
//! - **карман**: если у сквозной пары разное число полос, линия широкой
//!   дороги, которой на узкой места нет, кончается у кромки узла сплошной, а
//!   не висит посреди перекрёстка.
//!
//! Всё — точками на **нарисованной** оси улицы (`paths`): зебра и линия
//! полос лежат на одной кривой.

use std::collections::BTreeMap;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::{JUNCTION_MARGIN, SharedNode, Visit, node_key, with_stitches};
use super::network::StitchTarget;
use super::{is_carriageway, lane_count};
use crate::map::along::{arclengths, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::Break;
use crate::map::osm::{Highway, MapData, RoadLine, RoadNodeKind, TrafficSide};

/// Запас зоны узла за полушириной самой широкой его дороги, м — радиус
/// скругления улицы (`roads/corners.rs`): до конца дуги бордюра узел ещё
/// не кончился.
pub const CLUSTER_ZONE: f32 = 6.0;
/// Как далеко от узла ищется знак на плече и переход, м.
const SIGN_REACH: f32 = 30.0;
pub const ARM_CROSSING_REACH: f32 = 35.0;
/// Зебра: длина вдоль дороги и отступ её ближнего края от кромки узла, м.
pub const ZEBRA_LENGTH: f32 = 4.0;
pub const ZEBRA_SETBACK: f32 = 1.0;
/// Отступ зебры и стоп-линии от кромки проезжей части, м.
const EDGE_INSET: f32 = 0.3;
/// Стоп-линия: ширина и зазор до зебры, м.
pub const STOP_WIDTH: f32 = 0.4;
const STOP_GAP: f32 = 1.0;
/// Сколько чистого асфальта линии полос оставляют вокруг зебры и
/// стоп-линии, м.
const PAINT_CLEAR: f32 = 0.5;
/// Сколько дороги должно остаться за краской плеча, м.
const ARM_TAIL: f32 = 8.0;
/// Кусок линий между двумя разрывами короче этого — не рисуется: одинокий
/// штрих между узлом и зеброй читается мусором.
const MIN_RUN: f32 = 6.0;
/// Насколько зебры могут зайти одна на другую краями, м.
const OVERLAP_SLACK: f32 = 0.2;
/// Шаг сетки кластеров, м.
const CLUSTER_CELL: f32 = 50.0;

/// Зебры на плечах узлов.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CrossingMode {
    Off,
    /// Только по данным: `highway=crossing` на улице.
    Osm,
    /// По данным и по правилу — на каждом плече узла двух улиц с тротуарами.
    #[default]
    Generated,
}

impl CrossingMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Osm, Self::Generated];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Osm => "OSM",
            Self::Generated => "OSM + gen",
        }
    }
}

/// Зебра: отрезок поперёк проезжей части по её середине.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Zebra {
    pub from: Vec2,
    pub to: Vec2,
    /// По данным OSM, а не по правилу.
    pub osm: bool,
}

/// Стоп-линия: отрезок поперёк встречных узлу полос.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StopLine {
    pub from: Vec2,
    pub to: Vec2,
    /// Прерывистая: «уступи дорогу».
    pub yields: bool,
}

/// Карман у торца дороги: у продолжения улицы за узлом `lanes` полос, и
/// линии, которым там нет места, кончаются в разрыве `gap`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pocket {
    pub lanes: u8,
    pub gap: Break,
}

/// Плечо узла так, как его видят траектории манёвров (`roads/turns.rs`):
/// дорога уходит от кромки узла — длины `edge` на её нарисованной оси — в
/// сторону `dir` (+1 — к концу).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct JunctionArm {
    pub road: usize,
    pub edge: f32,
    pub dir: f32,
}

/// Узел (кластер) для траекторий: его плечи и дороги, что его **ведут** —
/// проходят насквозь, и уступать им некому. Колея ведущей идёт через узел
/// асфальтом ([`NodePaint::asphalt`]), и прямо по ней траектория не нужна.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Junction {
    pub arms: Vec<JunctionArm>,
    pub leading: Vec<usize>,
}

/// Краска узлов карты.
#[derive(Default)]
pub struct NodePaint {
    /// Разрывы краски по дорогам — вместо разрывов асфальта.
    pub breaks: Vec<Vec<Break>>,
    /// Разрывы асфальта — колеи и разделительных: базовые без тех, что лежали
    /// на ведущей дороге узла. Её колея идёт сквозь.
    pub asphalt: Vec<Vec<Break>>,
    pub junctions: Vec<Junction>,
    /// Карманы у торцов дорог `[начало, конец]`.
    pub pockets: Vec<[Option<Pocket>; 2]>,
    pub zebras: Vec<Zebra>,
    pub stop_lines: Vec<StopLine>,
    /// Кластеры из двух узлов и больше.
    pub clusters: usize,
    /// Узлы, где главная прошла насквозь.
    pub through: usize,
}

/// Что рисовать на узлах.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NodePaintStyle {
    pub crossings: CrossingMode,
    pub stop_lines: bool,
}

/// Ранг дороги в узле: класс `highway`, вдвое — чтобы знак мог понизить его
/// на полступени.
fn class_rank(highway: Highway) -> u8 {
    match highway {
        Highway::Motorway | Highway::Trunk => 5,
        Highway::Primary => 4,
        Highway::Secondary => 3,
        Highway::Tertiary => 2,
        Highway::Residential
        | Highway::Unclassified
        | Highway::LivingStreet
        | Highway::MotorwayLink
        | Highway::TrunkLink
        | Highway::PrimaryLink
        | Highway::SecondaryLink
        | Highway::TertiaryLink => 1,
        Highway::Service | Highway::Path => 0,
    }
}

/// Знак на плече.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sign {
    Stop,
    GiveWay,
}

/// Плечо узла: дорога уходит от узла `at` в сторону `dir` по своей длине
/// (+1 — к концу, −1 — к началу).
#[derive(Clone, Copy, Debug)]
struct Arm {
    road: usize,
    at: Vec2,
    dir: f32,
    /// Узел — торец дороги: 0 — начало, 1 — конец.
    end: Option<usize>,
}

/// Путь дороги с длинами — чтобы ставить точки по длине.
struct Walk<'a> {
    path: &'a [Vec2],
    along: Vec<f32>,
    total: f32,
}

impl<'a> Walk<'a> {
    fn new(path: &'a [Vec2]) -> Self {
        let (along, total) = arclengths(path);
        Self { path, along, total }
    }

    /// Длина до ближайшей к `point` точки пути.
    fn project(&self, point: Vec2) -> f32 {
        let mut best = (f32::INFINITY, 0.0);
        for (index, link) in self.path.windows(2).enumerate() {
            let step = link[1] - link[0];
            let length = step.length_squared();
            let t = if length > 0.0 {
                ((point - link[0]).dot(step) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = (link[0] + step * t).distance_squared(point);
            if distance < best.0 {
                let along = self.along[index] + t * (self.along[index + 1] - self.along[index]);
                best = (distance, along);
            }
        }
        best.1
    }

    /// Точка и направление пути на длине `at`, если она на пути.
    fn at(&self, at: f32) -> Option<(Vec2, Vec2)> {
        (0.0..=self.total)
            .contains(&at)
            .then(|| place_on_path(self.path, &self.along, at))
            .flatten()
    }

    /// Разрыв, закрывающий отрезок длин `[a, b]`.
    fn gap(&self, a: f32, b: f32) -> Option<Break> {
        let (low, high) = (a.min(b).max(0.0), a.max(b).min(self.total));
        let (at, _) = self.at((low + high) / 2.0)?;
        Some(Break {
            at,
            reach: (high - low) / 2.0,
        })
    }
}

/// Переход на дороге.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    along: f32,
    signals: bool,
    used: bool,
}

impl NodePaint {
    /// Краска узлов по дорогам `drawn`, нарисованным по `paths`. `base` —
    /// разрывы асфальта (`junctions::marking_breaks`), `targets` — стежки,
    /// которые тоже узлы, `sidewalk` — есть ли у дороги тротуар, `partners` —
    /// вторые половины разделённой улицы.
    pub fn new(
        drawn: &[&RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        (base, targets): (&[Vec<Break>], &[[Option<StitchTarget>; 2]]),
        map: &MapData,
        style: NodePaintStyle,
        sidewalk: impl Fn(usize) -> bool,
        partners: impl Fn(usize) -> Vec<usize>,
    ) -> Self {
        let mut paint = Self {
            breaks: base.to_vec(),
            asphalt: base.to_vec(),
            pockets: vec![[None; 2]; drawn.len()],
            ..Self::default()
        };
        if drawn.len() != paths.len() || base.len() != drawn.len() {
            return paint;
        }
        let nodes = with_stitches(drawn, is_carriageway, targets);
        let junctions: Vec<&SharedNode> = nodes.iter().filter(|node| node.is_junction()).collect();
        let marks: HashMap<(i32, i32), RoadNodeKind> = map
            .road_nodes
            .iter()
            .map(|node| (node_key(node.pos), node.kind))
            .collect();
        let mut near_marks: Grid<usize> = Grid::new(CLUSTER_CELL);
        for (index, node) in map.road_nodes.iter().enumerate() {
            near_marks.insert(node.pos, node.pos, index);
        }
        let street = |road: usize| {
            map.network
                .street_of(road)
                .map_or(usize::MAX - road, |(street, _)| street)
        };

        // переходы по дорогам — на нарисованной оси
        let mut crossings: Vec<Vec<Crossing>> = vec![Vec::new(); drawn.len()];
        if style.crossings != CrossingMode::Off {
            for node in &nodes {
                let Some(RoadNodeKind::Crossing {
                    signals,
                    marked: true,
                    ..
                }) = marks.get(&node_key(node.at))
                else {
                    continue;
                };
                if node.is_junction() {
                    continue;
                }
                for visit in &node.visits {
                    if drawn[visit.road].bridge {
                        continue;
                    }
                    let walk = Walk::new(paths[visit.road].as_ref());
                    crossings[visit.road].push(Crossing {
                        along: walk.project(node.at),
                        signals: *signals,
                        used: false,
                    });
                }
            }
        }

        for cluster in clusters(drawn, &junctions) {
            paint.paint_cluster(
                &cluster,
                &Context {
                    drawn,
                    paths,
                    map,
                    marks: &marks,
                    near_marks: &near_marks,
                    style,
                    street: &street,
                    sidewalk: &sidewalk,
                    partners: &partners,
                },
                &mut crossings,
            );
        }

        // переходы посреди квартала — и те, что стоят на плече главной
        for (road, list) in crossings.iter().enumerate() {
            for crossing in list.iter().filter(|crossing| !crossing.used) {
                paint.paint_crossing(
                    drawn[road],
                    paths[road].as_ref(),
                    road,
                    crossing,
                    (style, map.traffic_side),
                );
            }
        }
        for (road, breaks) in paint.breaks.iter_mut().enumerate() {
            bridge_short_runs(paths[road].as_ref(), breaks);
        }
        paint.zebras = without_overlaps(std::mem::take(&mut paint.zebras));
        paint
    }

    fn paint_cluster(
        &mut self,
        cluster: &[&SharedNode],
        context: &Context<'_, impl AsRef<[Vec2]>>,
        crossings: &mut [Vec<Crossing>],
    ) {
        let Context {
            drawn,
            paths,
            map,
            marks,
            near_marks,
            style,
            street,
            sidewalk,
            partners,
        } = *context;
        if cluster.len() > 1 {
            self.clusters += 1;
        }
        // проходы дорог через кластер, по дорогам, по порядку вершин
        let mut visits: BTreeMap<usize, Vec<(Visit, Vec2)>> = BTreeMap::new();
        for node in cluster {
            for &visit in &node.visits {
                visits.entry(visit.road).or_default().push((visit, node.at));
            }
        }
        for list in visits.values_mut() {
            list.sort_by_key(|(visit, _)| (visit.vertex, visit.inner));
        }
        let near = |point: Vec2, reach: f32| {
            near_marks
                .near_each(point - reach, point + reach)
                .map(|&index| &map.road_nodes[index])
                .filter(move |node| node.pos.distance(point) <= reach)
        };
        let signalized = cluster.iter().any(|node| {
            near(node.at, SIGN_REACH).any(|mark| match mark.kind {
                RoadNodeKind::TrafficSignals => mark.pos.distance(node.at) <= zone(drawn, node),
                RoadNodeKind::Crossing { signals, .. } => signals,
                _ => false,
            })
        });

        // плечи: куда дорога уходит из кластера. Кусок между двумя узлами
        // одного кластера — не плечо, он внутри узла
        let mut arms: Vec<Arm> = Vec::new();
        // плечи замкнутых колец: зебр и стоп-линий поперёк кольца нет, а
        // траектории по нему есть — въезд, дуга, съезд
        let mut ring_arms: Vec<Arm> = Vec::new();
        let mut continues: BTreeMap<usize, usize> = BTreeMap::new();
        for (&road, list) in &visits {
            let points = &drawn[road].points;
            let last = points.len() - 1;
            if points[0] == points[last] {
                // кольцо узел проходит, плеч поперёк него нет
                *continues.entry(street(road)).or_default() += 2;
                let (_, at) = list[0];
                ring_arms.extend([-1.0, 1.0].map(|dir| Arm {
                    road,
                    at,
                    dir,
                    end: None,
                }));
                continue;
            }
            let (first, at_first) = list[0];
            let (final_visit, at_final) = list[list.len() - 1];
            if first.vertex > 0 || first.inner {
                arms.push(Arm {
                    road,
                    at: at_first,
                    dir: -1.0,
                    end: (first.vertex == last && !first.inner).then_some(1),
                });
            }
            if final_visit.vertex < last {
                arms.push(Arm {
                    road,
                    at: at_final,
                    dir: 1.0,
                    end: (final_visit.vertex == 0).then_some(0),
                });
            }
        }
        for arm in &arms {
            *continues.entry(street(arm.road)).or_default() += 1;
        }
        let passes = |road: usize| continues.get(&street(road)).copied().unwrap_or(0) >= 2;
        let sign = |road: usize| -> Option<Sign> {
            drawn[road]
                .points
                .iter()
                .filter(|point| {
                    cluster
                        .iter()
                        .any(|node| node.at.distance(**point) <= SIGN_REACH)
                })
                .find_map(|point| match marks.get(&node_key(*point)) {
                    Some(RoadNodeKind::Stop) => Some(Sign::Stop),
                    Some(RoadNodeKind::GiveWay) => Some(Sign::GiveWay),
                    _ => None,
                })
        };
        let rank =
            |road: usize| class_rank(drawn[road].highway) * 2 + u8::from(sign(road).is_none());
        let others = |road: usize| -> Vec<usize> {
            let paired: Vec<usize> = partners(road).into_iter().map(street).collect();
            visits
                .keys()
                .copied()
                .filter(|&other| street(other) != street(road) && !paired.contains(&street(other)))
                .collect()
        };
        let sidewalk_streets = {
            let mut found: Vec<usize> = visits
                .keys()
                .copied()
                .filter(|&road| sidewalk(road))
                .map(street)
                .collect();
            found.sort_unstable();
            found.dedup();
            found.len()
        };

        let mut broken: BTreeMap<usize, f32> = BTreeMap::new();
        let mut reaches: BTreeMap<usize, f32> = BTreeMap::new();
        let mut leading: Vec<usize> = Vec::new();
        for (&road, list) in &visits {
            let others = others(road);
            let own = rank(road);
            // ведёт узел: проходит насквозь, и уступать некому — ни дороге
            // выше рангом, ни такой же проходящей. Кольцо ведёт всегда: у него
            // приоритет, въезды ему уступают
            let leads = passes(road)
                && (drawn[road].is_roundabout()
                    || !others.iter().any(|&other| {
                        let theirs = rank(other);
                        theirs > own || (theirs == own && passes(other))
                    }));
            let yields = signalized || !leads;
            let widest = others
                .iter()
                .map(|&other| drawn[other].width / 2.0)
                .fold(0.0_f32, f32::max);
            let reach = widest + JUNCTION_MARGIN;
            reaches.insert(road, reach);
            let here: Vec<Vec2> = list.iter().map(|(_, at)| *at).collect();
            if leads {
                leading.push(road);
                self.asphalt[road].retain(|found| !here.contains(&found.at) || found.reach == 0.0);
            }
            let breaks = &mut self.breaks[road];
            breaks.retain(|found| !here.contains(&found.at) || found.reach == 0.0);
            if !yields {
                self.through += 1;
                continue;
            }
            broken.insert(road, reach);
            breaks.extend(here.iter().map(|&at| Break { at, reach }));
            // соседние узлы кластера на одной дороге — один разрыв
            for pair in here.windows(2) {
                breaks.push(Break {
                    at: (pair[0] + pair[1]) / 2.0,
                    reach: pair[0].distance(pair[1]) / 2.0,
                });
            }
        }

        // карман: сквозная пара, у которой полос за узлом меньше
        for &road in visits.keys() {
            if broken.contains_key(&road) {
                continue;
            }
            let own: Vec<&Arm> = arms.iter().filter(|arm| arm.road == road).collect();
            let [arm] = own.as_slice() else { continue };
            let Some(end) = arm.end else { continue };
            let Some(next) = arms
                .iter()
                .find(|other| other.road != road && street(other.road) == street(road))
            else {
                continue;
            };
            let lanes = lane_count(drawn[next.road]);
            if lanes >= lane_count(drawn[road]) {
                continue;
            }
            let widest = others(road)
                .iter()
                .map(|&other| drawn[other].width / 2.0)
                .fold(0.0_f32, f32::max);
            self.pockets[road][end] = Some(Pocket {
                lanes,
                gap: Break {
                    at: arm.at,
                    reach: widest + JUNCTION_MARGIN,
                },
            });
        }

        // узел для траекторий: кромка каждого плеча на оси его дороги
        let junction_arms = arms
            .iter()
            .chain(&ring_arms)
            .filter(|arm| !drawn[arm.road].bridge)
            .map(|arm| {
                let walk = Walk::new(paths[arm.road].as_ref());
                let reach = reaches.get(&arm.road).copied().unwrap_or(JUNCTION_MARGIN);
                let edge = walk.project(arm.at) + arm.dir * reach;
                let path = walk.path;
                // у кольца длина идёт по кругу через шов
                let closed = path.len() > 2 && path[0] == path[path.len() - 1];
                JunctionArm {
                    road: arm.road,
                    edge: if closed {
                        edge.rem_euclid(walk.total)
                    } else {
                        edge.clamp(0.0, walk.total)
                    },
                    dir: arm.dir,
                }
            })
            .collect();
        self.junctions.push(Junction {
            arms: junction_arms,
            leading,
        });

        // зебры и стоп-линии на плечах, что рвутся: сперва где встать зебре
        let first = ZEBRA_SETBACK + ZEBRA_LENGTH / 2.0;
        let mut plans: Vec<ArmPlan> = Vec::new();
        for arm in &arms {
            let Some(&reach) = broken.get(&arm.road) else {
                continue;
            };
            if drawn[arm.road].bridge {
                continue;
            }
            let walk = Walk::new(paths[arm.road].as_ref());
            let from = walk.project(arm.at);
            let dir = arm.dir;
            let edge = from + dir * reach;
            // переход плеча — от узла до [`ARM_CROSSING_REACH`] за кромкой
            let osm = crossings[arm.road]
                .iter()
                .enumerate()
                .filter(|(_, crossing)| {
                    !crossing.used
                        && (crossing.along - from) * dir >= 0.0
                        && (crossing.along - edge) * dir <= ARM_CROSSING_REACH
                })
                .min_by(|(_, a), (_, b)| {
                    ((a.along - from) * dir).total_cmp(&((b.along - from) * dir))
                })
                .map(|(index, crossing)| (index, crossing.along));
            let zebra = match osm {
                Some((_, along)) => {
                    let ahead = ((along - edge) * dir).max(first);
                    Some((edge + dir * ahead, true))
                }
                // связка — не улица, пешеходу там переходить незачем
                None => (style.crossings == CrossingMode::Generated
                    && sidewalk(arm.road)
                    && sidewalk_streets >= 2
                    && !drawn[arm.road].highway.is_link())
                .then_some((edge + dir * first, false)),
            };
            plans.push(ArmPlan {
                arm: *arm,
                walk,
                edge,
                osm: osm.map(|(index, _)| index),
                zebra,
            });
        }
        // половины разделённой улицы переходят одной зеброй: вторая встаёт
        // на линию первой — той, что по данным, иначе той, что дальше
        for a in 0..plans.len() {
            for b in a + 1..plans.len() {
                let paired = partners(plans[a].arm.road).contains(&plans[b].arm.road)
                    || partners(plans[b].arm.road).contains(&plans[a].arm.road);
                if paired {
                    align_pair(&mut plans, a, b);
                }
            }
        }
        for plan in plans {
            let ArmPlan {
                arm,
                walk,
                edge,
                osm,
                zebra,
            } = plan;
            let road = drawn[arm.road];
            let dir = arm.dir;
            let stop = (style.stop_lines && incoming(road, dir)).then(|| {
                let behind = match zebra {
                    Some((center, _)) => center + dir * (ZEBRA_LENGTH / 2.0 + STOP_GAP),
                    None => edge + dir * ZEBRA_SETBACK,
                };
                behind + dir * STOP_WIDTH / 2.0
            });
            let outer = [
                zebra.map(|(center, _)| center + dir * ZEBRA_LENGTH / 2.0),
                stop.map(|at| at + dir * STOP_WIDTH / 2.0),
            ]
            .into_iter()
            .flatten()
            .max_by(|a, b| (a * dir).total_cmp(&(b * dir)));
            let Some(outer) = outer else { continue };
            // плечо короче краски с хвостом — ничего: это перемычка внутри
            // сложного узла, а не подход к нему
            if walk.at(outer + dir * (PAINT_CLEAR + ARM_TAIL)).is_none() {
                continue;
            }
            if let Some(index) = osm {
                crossings[arm.road][index].used = true;
            }
            if let Some((center, osm)) = zebra {
                self.zebras.extend(zebra_at(&walk, road, center, osm));
            }
            if let Some(at) = stop {
                let yields = !signalized && sign(arm.road) == Some(Sign::GiveWay);
                self.stop_lines.extend(stop_line_at(
                    &walk,
                    road,
                    at,
                    dir,
                    map.traffic_side,
                    yields,
                ));
            }
            self.breaks[arm.road].extend(walk.gap(edge, outer + dir * PAINT_CLEAR));
        }
    }

    /// Переход не у узла: зебра с разрывом линий вокруг и стоп-линиями по
    /// обе стороны, если он регулируемый.
    fn paint_crossing(
        &mut self,
        road: &RoadLine,
        path: &[Vec2],
        index: usize,
        crossing: &Crossing,
        (style, side): (NodePaintStyle, TrafficSide),
    ) {
        let walk = Walk::new(path);
        let center = crossing.along;
        let inside = self.breaks[index].iter().any(|found| {
            found.reach > 0.0 && (walk.project(found.at) - center).abs() < found.reach
        });
        if inside {
            return;
        }
        let Some(zebra) = zebra_at(&walk, road, center, true) else {
            return;
        };
        self.zebras.push(zebra);
        let mut reach = ZEBRA_LENGTH / 2.0;
        if style.stop_lines && crossing.signals {
            let offset = ZEBRA_LENGTH / 2.0 + STOP_GAP + STOP_WIDTH / 2.0;
            for dir in [-1.0, 1.0] {
                if incoming(road, dir) {
                    self.stop_lines.extend(stop_line_at(
                        &walk,
                        road,
                        center + dir * offset,
                        dir,
                        side,
                        false,
                    ));
                }
            }
            reach = offset + STOP_WIDTH / 2.0;
        }
        self.breaks[index]
            .extend(walk.gap(center - reach - PAINT_CLEAR, center + reach + PAINT_CLEAR));
    }
}

/// Плечо, которое рвётся, и где на нём встанет зебра: длина центра и по
/// данным ли она.
struct ArmPlan<'a> {
    arm: Arm,
    walk: Walk<'a>,
    /// Длина кромки узла на пути плеча.
    edge: f32,
    /// Переход OSM, которым стала зебра, — индекс в переходах дороги.
    osm: Option<usize>,
    zebra: Option<(f32, bool)>,
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
/// узла. Не ближе кромки узла.
fn align_pair(plans: &mut [ArmPlan], a: usize, b: usize) {
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
    let (target, moved) = match (osm(&plans[a]), osm(&plans[b])) {
        (true, true) => return,
        (true, false) => (a, b),
        (false, true) => (b, a),
        (false, false) if point_a.dot(out) >= point_b.dot(out) => (a, b),
        (false, false) => (b, a),
    };
    let (to, from, along) = if target == a {
        (point_a, point_b, out_b)
    } else {
        (point_b, point_a, out_a)
    };
    let shift = (to - from).dot(out) / along.dot(out);
    let plan = &mut plans[moved];
    let dir = plan.arm.dir;
    let Some((center, osm)) = plan.zebra else {
        return;
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

/// То, что `paint_cluster` читает, одним аргументом.
struct Context<'a, P> {
    drawn: &'a [&'a RoadLine],
    paths: &'a [P],
    map: &'a MapData,
    marks: &'a HashMap<(i32, i32), RoadNodeKind>,
    near_marks: &'a Grid<usize>,
    style: NodePaintStyle,
    street: &'a dyn Fn(usize) -> usize,
    sidewalk: &'a dyn Fn(usize) -> bool,
    partners: &'a dyn Fn(usize) -> Vec<usize>,
}

impl<P> Clone for Context<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for Context<'_, P> {}

/// Зона узла: полуширина самой широкой его дороги и запас на скругление.
fn zone(drawn: &[&RoadLine], node: &SharedNode) -> f32 {
    node.visits
        .iter()
        .map(|visit| drawn[visit.road].width / 2.0)
        .fold(0.0_f32, f32::max)
        + CLUSTER_ZONE
}

/// Узлы, чьи зоны перекрываются, — кластерами, в порядке первого узла.
fn clusters<'a>(drawn: &[&RoadLine], junctions: &[&'a SharedNode]) -> Vec<Vec<&'a SharedNode>> {
    let zones: Vec<f32> = junctions.iter().map(|node| zone(drawn, node)).collect();
    let mut grid: Grid<usize> = Grid::new(CLUSTER_CELL);
    for (index, node) in junctions.iter().enumerate() {
        grid.insert(node.at - zones[index], node.at + zones[index], index);
    }
    let mut parent: Vec<usize> = (0..junctions.len()).collect();
    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for (a, b) in grid.pairs() {
        if junctions[a].at.distance(junctions[b].at) < zones[a] + zones[b] {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut groups: BTreeMap<usize, Vec<&'a SharedNode>> = BTreeMap::new();
    for (index, node) in junctions.iter().enumerate() {
        groups
            .entry(root(&mut parent, index))
            .or_default()
            .push(node);
    }
    groups.into_values().collect()
}

/// Есть ли на плече, уходящем от узла в сторону `dir`, полосы к узлу:
/// односторонняя едет по ходу точек, к узлу — только с плеча к началу.
fn incoming(road: &RoadLine, dir: f32) -> bool {
    !road.oneway || dir < 0.0
}

/// Зебра поперёк `road` на длине `at`.
fn zebra_at(walk: &Walk, road: &RoadLine, at: f32, osm: bool) -> Option<Zebra> {
    let (point, direction) = walk.at(at)?;
    let across = direction.perp() * (road.width / 2.0 - EDGE_INSET);
    Some(Zebra {
        from: point - across,
        to: point + across,
        osm,
    })
}

/// Стоп-линия на длине `at` поперёк полос, едущих к узлу с плеча `dir`:
/// у двусторонней — от оси до кромки по стороне движения, у односторонней —
/// во всю ширину.
fn stop_line_at(
    walk: &Walk,
    road: &RoadLine,
    at: f32,
    dir: f32,
    side: TrafficSide,
    yields: bool,
) -> Option<StopLine> {
    let (point, direction) = walk.at(at)?;
    let travel = -direction * dir;
    let right = Vec2::new(travel.y, -travel.x);
    let kerb = match side {
        TrafficSide::Right => right,
        TrafficSide::Left => -right,
    } * (road.width / 2.0 - EDGE_INSET);
    let from = if road.oneway {
        point - kerb
    } else {
        point + kerb.normalize_or_zero() * EDGE_INSET
    };
    Some(StopLine {
        from,
        to: point + kerb,
        yields,
    })
}

/// Зебры, что легли одна на другую — две ветки развилки у одного узла,
/// переход OSM рядом с правилом, — одной: по данным остаётся, из
/// сгенерированных — первая. Зебры двух половин одной улицы стоят бок о бок
/// и друг друга не задевают.
fn without_overlaps(zebras: Vec<Zebra>) -> Vec<Zebra> {
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

/// Кусок линий между двумя разрывами короче [`MIN_RUN`] — тоже разрыв.
fn bridge_short_runs(path: &[Vec2], breaks: &mut Vec<Break>) {
    if breaks.len() < 2 || path.len() < 2 {
        return;
    }
    let walk = Walk::new(path);
    let mut spans: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = walk.project(found.at);
            (at - found.reach, at + found.reach)
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = spans[0].1;
    for span in &spans[1..] {
        if span.0 > reach && span.0 - reach < MIN_RUN {
            breaks.extend(walk.gap(reach, span.0));
        }
        reach = reach.max(span.1);
    }
}

#[cfg(test)]
mod tests;
