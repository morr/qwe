//! Один кластер узлов: разрывы по рангам и знакам, кромки плеч по чужому
//! асфальту, зебры и стоп-линии на плечах, карманы и горла съездов.

use std::collections::BTreeMap;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::zebras::{ArmPlan, RingEntry, align_pair, join_zebras, zebra_at};
use super::{
    ARM_CROSSING_REACH, Arm, Crossing, CrossingMode, EDGE_INSET, Junction, JunctionArm, NodeAlong,
    NodePaint, NodePaintStyle, PAINT_CLEAR, PASS_ALIGN, Partner, PavedIsland, Pocket, SIGN_REACH,
    STOP_WIDTH, StopLine, Throat, Walk, ZEBRA_LENGTH, ZEBRA_SETBACK, zone,
};
use crate::map::footprint::distance_to_polyline;
use crate::map::grid::Grid;
use crate::map::meshing::Break;
use crate::map::osm::{Highway, MapData, RoadLine, RoadNodeKind, TrafficSide};
use crate::map::roads::junctions::{JUNCTION_MARGIN, SharedNode, Visit, node_key};
use crate::map::roads::lane_count;
use crate::map::roads::paint::{axis_offset, lane_frame};
use crate::map::roads::shape::lane_width;

pub(super) const STOP_GAP: f32 = 1.0;
/// Сколько свободной дороги нужно за стоп-линией, м: две машины в очереди.
/// Меньше — линия на перемычке между половинами разделённой улицы, где ждать
/// негде: машина встала бы на соседний перекрёсток или на зебру через газон
/// (Рязань, витрина 07: Горького через газон Есенина, 28 м между узлами).
const STOP_QUEUE_ROOM: f32 = 10.0;

/// Сколько дороги должно остаться за краской плеча, м.
const ARM_TAIL: f32 = 8.0;
/// Сколько дороги нужно зебре по правилу от кромки узла до следующего узла
/// той же дороги, м. Короче — перемычка между двумя узлами (ветки
/// треугольника развилки в 22 и 31 м, Тула, витрина 06): зебры с обоих её
/// концов и стоп-линия между ними теснятся на пятнадцати метрах, и пешеход
/// переходит на внешних плечах.
pub(super) const RULE_ZEBRA_ROOM: f32 = 30.0;
/// Размеченный переход OSM на той же улице ближе этого к точке узла — и
/// зебры по правилу на плечах этой улицы нет: переход узла уже есть по
/// данным, м. Та же досягаемость, на которой переход становится зеброй
/// самого плеча ([`ARM_CROSSING_REACH`]).
const RULE_ZEBRA_DATA_REACH: f32 = ARM_CROSSING_REACH;
/// Ранг ([`class_rank`]) улицы, без которой в кластере зебры по правилу нет,
/// если узел не под светофором: `tertiary`.
const RULE_ZEBRA_RANK: u8 = 2;
/// Косинус, под которым два чужих плеча из разных узлов кластера смотрят в
/// противоположные стороны и считаются одной поперечной улицей — крестовиной
/// вразбежку: 40° от прямой.
const STAGGER_ALIGN: f32 = 0.766;
/// Хорда направления плеча для крестовины вразбежку, м: первое звено OSM
/// бывает в полметра.
const STAGGER_CHORD: f32 = 10.0;

/// Хорда излома для [`PASS_ALIGN`], м: направление плеча от узла и хорды
/// назад и вперёд от вершин пути под разрывом. Короткая, чтобы мерить угол,
/// а не поворот: дуга оси (`ThroughBend` в `axis.rs`, радиус от 30 м)
/// расходится на ней на единицы градусов, угол OSM — целиком. Хорда на
/// [`STAGGER_CHORD`] от узла захватывала и дугу, и плавный поворот вторичной
/// улицы рвался посреди дуги (Тула, 5325, 4424).
pub(super) const PASS_CHORD: f32 = 2.0;

/// Ранг класса `highway` в узле. Ранг дороги — он вдвое плюс единица без
/// знака на плече (`paint_cluster`): так знак понижает его на полступени.
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

/// Как далеко от узла кромка плеча ищется по асфальту кластера, м: дальше —
/// плечо целиком перемычка внутри узла ([`JunctionArm::link`]), и кромка
/// остаётся на полуширине соседа.
pub(super) const EDGE_SEARCH: f32 = 25.0;
/// Шаг этого поиска, м.
const EDGE_STEP: f32 = 0.5;
/// Как далеко от узла ищется, где подход к кольцу вышел из его асфальта
/// ([`leaves_ring`]), м. Подход вписан по касательной и идёт по асфальту
/// кольца дольше, чем плечо перекрёстка по чужому: въезд с востока в кольцо
/// витрины 04 выходит из него целиком лишь метров через тридцать, и за
/// [`EDGE_SEARCH`] въезд не находился вовсе — ни линии уступи дорогу, ни
/// разрыва, и его линия полос тянулась по кольцу к его оси.
const RING_EDGE_SEARCH: f32 = 60.0;
/// Как далеко ищется, где из чужого асфальта вышли линии перемычки
/// (`lines_edge`), м. Съезд отходит от главной под острым углом с её оси: у
/// Фрунзе (Тула, 3814, 4100, R6) жилой съезд уходит под 11° от оси шести
/// полос, и его осевая выходит из их асфальта лишь метров через 36 — за
/// [`EDGE_SEARCH`] кромка линий не находилась, и сплошная съезда шла
/// наискось через полосы главной.
const LINES_EDGE_SEARCH: f32 = 60.0;

/// Где сечение плеча выходит из асфальта кластера — чужих дорог и
/// замощённых островов `paved`: длина от узла `from` по ходу `dir`, не
/// меньше `reach` окна `(reach, search)`. Сечение — две точки в `half` по обе
/// стороны оси; чужая дорога — путь и полуширина. Плечо, что за `search`
/// (кромка — [`EDGE_SEARCH`]) или до конца своей дороги так и не вышло, —
/// `None`: перемычка.
fn clear_reach(
    walk: &Walk,
    from: f32,
    dir: f32,
    (reach, search): (f32, f32),
    half: f32,
    foreign: &[(&[Vec2], f32)],
    paved: &[PavedIsland],
) -> Option<f32> {
    let clear = |ahead: f32| {
        walk.at(from + dir * ahead).is_some_and(|(point, tangent)| {
            let normal = tangent.perp() * half;
            [point + normal, point - normal].iter().all(|&side| {
                foreign
                    .iter()
                    .all(|&(path, width)| distance_to_polyline(side, path) >= width)
                    && !paved.iter().any(|island| island.contains(side))
            })
        })
    };
    let mut ahead = reach;
    while ahead <= search {
        if clear(ahead) {
            return Some(ahead);
        }
        ahead += EDGE_STEP;
    }
    None
}

/// Где точка сечения плеча в `offset` вбок от оси (по левой нормали пути)
/// вышла из асфальта колец `rings` (путь и полуширина): длина от узла `from`
/// по ходу `dir` и сама точка. `None` — не вышла за [`RING_EDGE_SEARCH`] или
/// до конца дороги.
fn leaves_ring(
    walk: &Walk,
    from: f32,
    dir: f32,
    offset: f32,
    rings: &[(&[Vec2], f32)],
) -> Option<(f32, Vec2)> {
    let mut ahead = 0.0;
    while ahead <= RING_EDGE_SEARCH {
        let (point, tangent) = walk.at(from + dir * ahead)?;
        let side = point + tangent.perp() * offset;
        if rings
            .iter()
            .all(|&(path, half)| distance_to_polyline(side, path) >= half)
        {
            return Some((ahead, side));
        }
        ahead += EDGE_STEP;
    }
    None
}

/// Горло съезда на оси кольца `ring`: от `lead` метров перед проекцией узла
/// `node` до проекции точки `last`, где из асфальта кольца вышла последняя
/// сторона сечения съезда, со стороной съезда относительно оси. Дуге, что в
/// узле кончается, — только отрезок `lead` перед ним. `None` — горло на этом
/// пути не лежит.
fn throat_on(ring: &[Vec2], node: Vec2, last: Vec2, lead: f32) -> Option<Throat> {
    let walk = Walk::new(ring);
    let closed = is_closed(ring);
    let (a, b) = (walk.project(node), walk.project(last));
    let mut delta = b - a;
    if closed && delta.abs() > walk.total / 2.0 {
        // через шов замкнутого кольца — короткой дорогой
        delta -= delta.signum() * walk.total;
    }
    // дуга, что в узле кончается (обе проекции в её торце), получает только
    // хвост `lead` перед узлом; кольцо одностороннее, точки — по ходу, и горло
    // лежит за узлом по ходу пути
    let direction = if delta.abs() >= EDGE_STEP {
        delta.signum()
    } else if !closed && walk.total - a < EDGE_STEP && lead > 0.0 {
        delta = 0.0;
        1.0
    } else {
        return None;
    };
    // и назад от узла на `lead`: торец ленты съезда в узле уже лежит на
    // наружной полосе, и штрих, разрезанный узлом, оставался огрызком
    let start = a - direction * lead;
    let length = delta.abs() + lead;
    let mid = start + direction * length / 2.0;
    let mid = if closed {
        mid.rem_euclid(walk.total)
    } else {
        mid.clamp(0.0, walk.total)
    };
    let (at, _) = walk.at(mid)?;
    let (on, tangent) = walk.at(b)?;
    let side = (last - on).dot(tangent.perp()).signum();
    Some(Throat {
        gap: Break {
            at,
            reach: length / 2.0,
        },
        side,
    })
}

/// Насколько дальняя линия полос дороги отстоит от её оси, м: рама полос
/// ([`lane_frame`]) без крайней полосы, со сдвигом осевой двусторонней.
/// `None` — полоса одна, линий нет.
fn line_half(road: &RoadLine, side: TrafficSide) -> Option<f32> {
    let lanes = lane_count(road);
    if lanes < 2 || !road.lane_markings {
        return None;
    }
    let frame = lane_frame(lanes);
    let shift = axis_offset(road, lanes, side).unwrap_or(0.0).abs();
    Some(frame.high - lane_width() + shift)
}

impl NodePaint {
    pub(super) fn paint_cluster(
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
            on_ring,
            nodes_along,
            paved,
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
        // куда плечо уходит от узла: хорда на [`STAGGER_CHORD`] по его пути
        let heading = |arm: &Arm| -> Vec2 {
            let walk = Walk::new(paths[arm.road].as_ref());
            let along = (walk.project(arm.at) + arm.dir * STAGGER_CHORD).clamp(0.0, walk.total);
            walk.at(along).map_or(Vec2::ZERO, |(point, _)| {
                (point - arm.at).normalize_or_zero()
            })
        };
        // куда нарисованная ось уходит от узла сразу за ним: хорда на
        // [`PASS_CHORD`] от проекции узла на путь
        let leaving = |arm: &Arm| -> Option<Vec2> {
            let walk = Walk::new(paths[arm.road].as_ref());
            let from = walk.project(arm.at);
            walk.chord(from, (from + arm.dir * PASS_CHORD).clamp(0.0, walk.total))
        };
        // излом нарисованной оси на плече до `reach` от узла: вершина пути,
        // у которой хорды по [`PASS_CHORD`] назад и вперёд расходятся круче
        // [`PASS_ALIGN`]. Дуга поворота расходится полого, угол — нет
        let kinked = |arm: &Arm, reach: f32| {
            let walk = Walk::new(paths[arm.road].as_ref());
            let from = walk.project(arm.at);
            let chord = |from: f32, to: f32| {
                walk.chord(from.clamp(0.0, walk.total), to.clamp(0.0, walk.total))
            };
            walk.along.iter().any(|&at| {
                let ahead = (at - from) * arm.dir;
                ahead > 0.0
                    && ahead <= reach
                    && chord(at - PASS_CHORD, at)
                        .zip(chord(at, at + PASS_CHORD))
                        .is_some_and(|(back, on)| back.dot(on) < PASS_ALIGN)
            })
        };
        // двусторонняя улица поворачивает в узле: среди её плеч нет пары на
        // одной прямой в пределах [`PASS_ALIGN`], или ось ломается на плече
        // под разрывом, до `reach`. Только двусторонняя — у неё осевая;
        // половина разделённой улицы расходится от узла развилки полого, и её
        // линии полос проходили насквозь (Тула, витрина 16)
        let turns = |road: usize, reach: f32| {
            let own: Vec<&Arm> = arms
                .iter()
                .filter(|arm| street(arm.road) == street(road))
                .collect();
            own.iter().all(|arm| !drawn[arm.road].oneway)
                && (!own.iter().enumerate().any(|(index, arm)| {
                    // направления не снять — не повод рвать: считается прямой
                    own[index + 1..].iter().any(|other| {
                        leaving(arm)
                            .zip(leaving(other))
                            .is_none_or(|(a, b)| a.dot(b) <= -PASS_ALIGN)
                    })
                }) || own.iter().any(|arm| kinked(arm, reach)))
        };
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
            let paired: Vec<usize> = partners(road)
                .into_iter()
                .map(|partner| street(partner.road))
                .collect();
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
        // зебра по правилу — только там, где пешеходу её и рисуют: у улицы не
        // ниже `tertiary` или под светофором, и никогда у кольца. Двум
        // жилым улицам разметку переходов никто не наносит (у Яндекса на
        // таких узлах ни одной), а у кольца переходы стоят поодаль от въезда
        // и приходят в OSM нодами — луч за лучом по зебре было выдумкой
        let major = visits
            .keys()
            .any(|&road| class_rank(drawn[road].highway) >= RULE_ZEBRA_RANK);
        let at_ring = !ring_arms.is_empty() || visits.keys().any(|&road| on_ring(road));
        // угол двух улиц ([`is_corner`]) — не перекрёсток: линии рвутся, но
        // тихо ([`NodePaint::quiet`]), и ни зебру, ни стоп-линию, ни
        // траектории угол сам не зовёт — уступать там некому
        let corner = cluster.iter().all(|node| !node.is_junction());
        let rule_zebras = (signalized || major) && !at_ring;
        // дорога самого кольца: дуга (`roads/rings.rs`) или кольцо одним way
        let ring_road = |road: usize| on_ring(road) || is_closed(&drawn[road].points);

        let mut broken: BTreeMap<usize, f32> = BTreeMap::new();
        let mut reaches: BTreeMap<usize, f32> = BTreeMap::new();
        let mut leading: Vec<usize> = Vec::new();
        // ведущие, чьи линии рвутся только из-за излома оси
        let mut bent_roads: Vec<usize> = Vec::new();
        for (&road, list) in &visits {
            let others = others(road);
            let own = rank(road);
            // два плеча чужих улиц в одном узле — это крестовина, а не
            // примыкание, даже если OSM режет поперечную на две улицы (у Макса
            // Смирнова в Туле, 5968, 1582, юг односторонний, север нет, и
            // «насквозь» она не проходила — главная шла пунктиром через
            // перекрёсток). В одном узле, а не в кластере: примыкания с разных
            // сторон вразбежку (Циолковского, 17 м) главную не рвут
            let foreign: Vec<&Arm> = arms
                .iter()
                .filter(|arm| {
                    others
                        .iter()
                        .any(|&other| street(other) == street(arm.road))
                })
                .collect();
            // крестовина рвёт и старшую дорогу, какого бы ранга ни была
            // поперечная: через поле перекрёстка на четыре стороны линий не
            // кладут, а осевая насквозь — двойная сплошная поперёк проезда
            // поперечной, запрет левого поворота (Тула, Лейтейзена × Сойфера,
            // 4468, 3849 — tertiary на 6 полос через жилую). Не рвёт её только
            // примыкание сбоку — одно чужое плечо в узле. Два чужих плеча,
            // что смотрят в разные стороны на одной прямой, — крестовина и
            // вразбежку, если между их узлами меньше ширины самой дороги:
            // OSM сводит Вересаева к Первомайской двумя ways в двух узлах в
            // 6 м (Тула, 3472, 2677). Щорса и переулок у Циолковского в 17 м
            // — два примыкания
            let crossed = foreign.iter().enumerate().any(|(index, arm)| {
                foreign[index + 1..].iter().any(|other| {
                    let apart = other.at.distance(arm.at);
                    apart < JUNCTION_MARGIN
                        || apart < drawn[road].width
                            && heading(arm).dot(heading(other)) < -STAGGER_ALIGN
                })
            });
            // ведёт узел: проходит насквозь, и уступать некому — ни дороге
            // выше рангом, ни такой же проходящей или крестовине. Кольцо ведёт
            // всегда: у него приоритет, въезды ему уступают
            // Подход к кольцу не ведёт, даже если сеть продолжает им улицу
            // дуги: въезд с Болдина в кольцо 50-й Армии (Тула, витрина 04,
            // восток) шёл «насквозь», без стоп-линии, и линии его полос
            // тянулись до оси кольца.
            // Дуга кольца ведёт, даже кончаясь в узле: следующую дугу OSM
            // режет в каждом въезде, и «насквозь» она не проходит — линии
            // кольца у въезда становились сплошными подхода.
            let leads = (passes(road) || ring_road(road))
                && (drawn[road].is_roundabout()
                    || (!at_ring || ring_road(road))
                        && !crossed
                        && !others.iter().any(|&other| {
                            let theirs = rank(other);
                            theirs > own || (theirs == own && passes(other))
                        }));
            let widest = others
                .iter()
                .map(|&other| drawn[other].width / 2.0)
                .fold(0.0_f32, f32::max);
            let reach = widest + JUNCTION_MARGIN;
            reaches.insert(road, reach);
            // Ведёт, но не насквозь по линиям: двусторонняя, чья нарисованная
            // ось в узле поворачивает круче [`PASS_ALIGN`], — осевая сплошной
            // легла бы через узел углом, скруглить её там негде (Путейская у
            // моста, Тула, 2638, 3153 — R22). Линии рвутся, приоритет
            // остаётся: ни стоп-линии, ни зебры поперёк неё узел не зовёт
            let bent = leads && !ring_road(road) && turns(road, reach);
            if bent && !signalized {
                bent_roads.push(road);
            }
            let yields = signalized || !leads || bent;
            let here: Vec<Vec2> = list.iter().map(|(_, at)| *at).collect();
            if leads {
                leading.push(road);
                self.asphalt[road].retain(|found| !here.contains(&found.at) || found.reach == 0.0);
            }
            let breaks = &mut self.breaks[road];
            breaks.retain(|found| !here.contains(&found.at) || found.reach == 0.0);
            if !yields {
                self.through += 1;
                self.solid[road].extend(here.iter().map(|&at| Break { at, reach }));
                continue;
            }
            broken.insert(road, reach);
            let from = breaks.len();
            breaks.extend(here.iter().map(|&at| Break { at, reach }));
            // соседние узлы кластера на одной дороге — один разрыв
            for pair in here.windows(2) {
                breaks.push(Break {
                    at: (pair[0] + pair[1]) / 2.0,
                    reach: pair[0].distance(pair[1]) / 2.0,
                });
            }
            if corner {
                // и торец дороги в узле (разрыв нулевого вылета) — тоже тихий
                let quiet = breaks
                    .iter()
                    .enumerate()
                    .filter(|&(index, found)| index >= from || here.contains(&found.at))
                    .map(|(_, found)| *found);
                self.quiet[road].extend(quiet);
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
            self.pockets[road][end] = Some(Pocket {
                lanes,
                gap: Break {
                    at: arm.at,
                    reach: reaches[&road],
                },
            });
        }

        // кромка плеча — не полуширина соседа от точки узла, а место, где
        // сечение плеча выходит из асфальта чужих дорог кластера: у луча,
        // что уходит из узла под острым углом, чужой асфальт тянется дальше,
        // и стрелки со стоп-линией ложились внутрь перекрёстка (пример 06,
        // горловина развилки). Плечо, что из асфальта узла так и не вышло, —
        // перемычка ([`JunctionArm::link`]). Кроме кольца: подход вписан в
        // него по касательной и идёт по его асфальту десятки метров — кромка
        // ушла бы за переход (пример 04, юг)
        let reach_of = |road: usize| reaches.get(&road).copied().unwrap_or(JUNCTION_MARGIN);
        let foreign_of = |road: usize| -> Vec<(&[Vec2], f32)> {
            others(road)
                .into_iter()
                .map(|other| (paths[other].as_ref(), drawn[other].width / 2.0))
                .collect()
        };
        let arm_edge = |arm: &Arm, walk: &Walk| -> (f32, bool) {
            let from = walk.project(arm.at);
            let reach = reach_of(arm.road);
            if at_ring {
                return (from + arm.dir * reach, false);
            }
            let half = drawn[arm.road].width / 2.0 - EDGE_INSET;
            match clear_reach(
                walk,
                from,
                arm.dir,
                (reach, EDGE_SEARCH),
                half,
                &foreign_of(arm.road),
                paved,
            ) {
                Some(ahead) => (from + arm.dir * ahead, false),
                None => (from + arm.dir * reach, true),
            }
        };
        // Кромка линий перемычки: где из чужого асфальта вышли её линии полос,
        // а не всё сечение. Ветка развилки под острым углом (Вокзальная из
        // Первомайского, Рязань, витрина 03) целиком из асфальта главной не
        // выходит и за [`EDGE_SEARCH`] — перемычка с кромкой на полуширине
        // соседа, и её линия шла по полосам главной, перекрещиваясь с их
        // линией. Ищется дальше кромки — до [`LINES_EDGE_SEARCH`]: съезд с оси
        // широкой улицы под острым углом выходит из её асфальта не сразу
        // (Фрунзе, R6). `None` — линии не вышли и тут, кромка остаётся как была.
        let lines_edge = |arm: &Arm, walk: &Walk| -> Option<f32> {
            let road = drawn[arm.road];
            let half = line_half(road, map.traffic_side)?;
            let from = walk.project(arm.at);
            let reach = reach_of(arm.road);
            clear_reach(
                walk,
                from,
                arm.dir,
                (reach, LINES_EDGE_SEARCH),
                half,
                &foreign_of(arm.road),
                paved,
            )
            .map(|ahead| from + arm.dir * ahead)
        };
        // Въезд в кольцо: где подход выходит из асфальта самого кольца. Подход
        // вписан по касательной, и полуширина кольца от узла по его оси —
        // ещё середина кольца: стоп-линия поперёк подхода ложилась через полосы
        // кольца до бордюра острова, и сплошные подхода тянулись за ней (пример
        // 04, юг и восток). Линия уступи дорогу встаёт на кромку кольца —
        // от места, где из асфальта кольца вышла одна сторона её отрезка, до
        // места, где вышла другая (как на месте: вдоль кромки кольца, а не
        // поперёк подхода), кромка плеча — где вышло всё сечение. Чужие
        // дороги кластера, кроме кольца, не в счёт: иначе кромка ушла бы за
        // переход (пример 04, юг).
        let ring_roads: Vec<(&[Vec2], f32)> = visits
            .keys()
            .filter(|&&road| ring_road(road))
            .map(|&road| (paths[road].as_ref(), drawn[road].width / 2.0))
            .collect();
        let ring_entry = |arm: &Arm, walk: &Walk| -> Option<RingEntry> {
            let road = drawn[arm.road];
            if ring_roads.is_empty() || ring_road(arm.road) {
                return None;
            }
            let from = walk.project(arm.at);
            // сторона бордюра — как у `stop_line_at`: справа по ходу к узлу
            let kerb = arm.dir
                * match map.traffic_side {
                    TrafficSide::Right => 1.0,
                    TrafficSide::Left => -1.0,
                }
                * (road.width / 2.0 - EDGE_INSET);
            let far = if road.oneway {
                -kerb
            } else {
                kerb.signum() * EDGE_INSET
            };
            let exit = |offset: f32| leaves_ring(walk, from, arm.dir, offset, &ring_roads);
            let (near, from_point) = exit(far)?;
            let (kerbside, to_point) = exit(kerb)?;
            let (other, _) = exit(-kerb)?;
            Some(RingEntry {
                edge: from + arm.dir * near.max(kerbside).max(other),
                line: (from_point, to_point),
            })
        };
        // узел для траекторий: кромка каждого плеча на оси его дороги
        let junction_arms = arms
            .iter()
            .map(|arm| (arm, true))
            .chain(ring_arms.iter().map(|arm| (arm, false)))
            .filter(|(arm, _)| !drawn[arm.road].bridge)
            .map(|(arm, cleared)| {
                let walk = Walk::new(paths[arm.road].as_ref());
                let (edge, link) = if cleared {
                    arm_edge(arm, &walk)
                } else {
                    (walk.project(arm.at) + arm.dir * reach_of(arm.road), false)
                };
                let path = walk.path;
                // у кольца длина идёт по кругу через шов
                let closed = is_closed(path);
                JunctionArm {
                    road: arm.road,
                    edge: if closed {
                        edge.rem_euclid(walk.total)
                    } else {
                        edge.clamp(0.0, walk.total)
                    },
                    dir: arm.dir,
                    link,
                }
            })
            .collect();
        self.junctions.push(Junction {
            arms: junction_arms,
            leading,
        });

        // Горло съезда: съезд уходит с кольца по касательной, и от узла до
        // места, где его сечение вышло из асфальта кольца, наружная полоса
        // кольца — уже съезд. Её линия там рвётся, а не идёт пунктиром поперёк
        // горла вразнобой со сплошной съезда (пример 04, юг).
        if !ring_roads.is_empty() {
            for arm in &arms {
                let road = drawn[arm.road];
                if ring_road(arm.road) || road.bridge || !road.oneway || incoming(road, arm.dir) {
                    continue;
                }
                let walk = Walk::new(paths[arm.road].as_ref());
                let from = walk.project(arm.at);
                let half = road.width / 2.0 - EDGE_INSET;
                let sides = [half, -half]
                    .map(|offset| leaves_ring(&walk, from, arm.dir, offset, &ring_roads));
                let [Some(left), Some(right)] = sides else {
                    continue;
                };
                let (_, last) = if left.0 >= right.0 { left } else { right };
                for &ring in visits.keys().filter(|&&road| ring_road(road)) {
                    let lead = road.width / 2.0;
                    if let Some(throat) = throat_on(paths[ring].as_ref(), arm.at, last, lead) {
                        self.throats[ring].push(throat);
                    }
                }
            }
        }

        // зебры и стоп-линии на плечах, что рвутся: сперва где встать зебре
        let first = ZEBRA_SETBACK + ZEBRA_LENGTH / 2.0;
        let mut plans: Vec<ArmPlan> = Vec::new();
        for arm in &arms {
            // излом оси рвёт линии ведущей, но не зовёт краски поперёк неё; угол
            // двух улиц — ничьей
            if !broken.contains_key(&arm.road)
                || drawn[arm.road].bridge
                || bent_roads.contains(&arm.road)
                || corner
            {
                continue;
            }
            let walk = Walk::new(paths[arm.road].as_ref());
            let from = walk.project(arm.at);
            let dir = arm.dir;
            let (edge, link) = arm_edge(arm, &walk);
            let ring = ring_entry(arm, &walk);
            let edge = ring.as_ref().map_or(edge, |entry| entry.edge);
            // переход по данным стоит, где стоит: от кромки по полуширине
            // соседа, как прежде, — толкать его за кромку по асфальту значило
            // бы выбросить с короткого плеча (пример 04, юг)
            let near = from + dir * reach_of(arm.road);
            // переход плеча — от узла до [`ARM_CROSSING_REACH`] за кромкой
            let osm = crossings[arm.road]
                .iter()
                .enumerate()
                .filter(|(_, crossing)| {
                    !crossing.used
                        && (crossing.along - from) * dir >= 0.0
                        && (crossing.along - near) * dir <= ARM_CROSSING_REACH
                })
                .min_by(|(_, a), (_, b)| {
                    ((a.along - from) * dir).total_cmp(&((b.along - from) * dir))
                })
                .map(|(index, crossing)| (index, crossing.along));
            // до следующего узла той же дороги, не из этого кластера
            let ahead: Vec<NodeAlong> = nodes_along[arm.road]
                .iter()
                .filter(|node| cluster.iter().all(|own| own.at != node.at))
                .filter(|node| (node.along - edge) * dir > 0.0)
                .copied()
                .collect();
            let room = ahead
                .iter()
                .map(|node| (node.along - edge) * dir)
                .fold(f32::INFINITY, f32::min);
            // где за плечом начинается асфальт следующего узла
            let next_edge = ahead
                .iter()
                .map(|node| (node.along - edge) * dir - node.reach)
                .fold(f32::INFINITY, f32::min);
            // переход этой же улицы по данным у самого узла — на любом её
            // плече: мапер разметил, где здесь переходят, и зебра по правилу
            // на другом плече встала бы второй в двух десятках метров от
            // первой (Тула, витрина 12: Т Халтурины с Красноармейским)
            let crossed_by_data = visits
                .keys()
                .filter(|&&road| street(road) == street(arm.road))
                .any(|&road| {
                    let walk = Walk::new(paths[road].as_ref());
                    crossings[road].iter().any(|crossing| {
                        walk.at(crossing.along).is_some_and(|(point, _)| {
                            point.distance(arm.at) <= RULE_ZEBRA_DATA_REACH
                        })
                    })
                });
            let zebra = match osm {
                Some((_, along)) => {
                    let ahead = ((along - near) * dir).max(first);
                    Some((near + dir * ahead, true))
                }
                // связка — не улица, пешеходу там переходить незачем. На
                // регулируемом узле хватает тротуаров по обе стороны самого
                // плеча: у Ложевой (Тула, витрина 08) тротуаров нет по тегу, и
                // Т со светофорами оставался без единой зебры, хотя пешеход
                // Пролетарской переходит её плечо на свой зелёный
                None => (style.crossings == CrossingMode::Generated
                    && rule_zebras
                    && sidewalk(arm.road)
                    && (sidewalk_streets >= 2
                        || (signalized && drawn[arm.road].sidewalk().both()))
                    && room >= RULE_ZEBRA_ROOM
                    && !crossed_by_data
                    && !drawn[arm.road].is_unpaved_street()
                    && !link
                    && !drawn[arm.road].highway.is_link())
                .then_some((edge + dir * first, false)),
            };
            plans.push(ArmPlan {
                arm: *arm,
                walk,
                edge,
                osm: osm.map(|(index, _)| index),
                zebra,
                link,
                ring_line: ring.map(|entry| entry.line),
                next_edge,
            });
        }
        // половины разделённой улицы переходят одной зеброй: вторая встаёт
        // на линию первой — той, что по данным, иначе той, что дальше; по
        // асфальтовой разделительной обе станут одной планкой
        let mut joined: Vec<[usize; 2]> = Vec::new();
        for a in 0..plans.len() {
            for b in a + 1..plans.len() {
                let pair = |from: usize, to: usize| {
                    partners(plans[from].arm.road)
                        .into_iter()
                        .find(|partner| partner.road == plans[to].arm.road)
                };
                if let Some(partner) = pair(a, b).or_else(|| pair(b, a)) {
                    align_pair(&mut plans, a, b);
                    if partner.paved {
                        joined.push([a, b]);
                    }
                }
            }
        }
        let first_zebra = self.zebras.len();
        // какая зебра какого плеча — для слияния пар
        let mut zebra_of: Vec<Option<usize>> = vec![None; plans.len()];
        for (plan_index, plan) in plans.into_iter().enumerate() {
            let ArmPlan {
                arm,
                walk,
                edge,
                osm,
                zebra,
                link,
                ring_line,
                next_edge,
            } = plan;
            let road = drawn[arm.road];
            let dir = arm.dir;
            // Кромка узла — полуширина соседа от точки узла; у луча, что
            // вливается в соседа под острым углом, там ещё его асфальт, и
            // стоп-линия (зебра по правилу — тоже) ложилась обрывком посреди
            // перекрёстка (пример 08, связка в Пролетарскую). Такая краска не
            // рисуется.
            let in_other = |along: f32| {
                walk.at(along).is_some_and(|(point, _)| {
                    visits.keys().any(|&other| {
                        other != arm.road
                            && distance_to_polyline(point, paths[other].as_ref())
                                < drawn[other].width / 2.0 - EDGE_INSET
                    })
                })
            };
            let zebra = zebra.filter(|&(center, osm)| osm || !in_other(center));
            // стоп-линию зовёт то же, что и зебру: переход, светофор, знак или
            // улица не ниже `tertiary`; две жилые без знаков — ни того, ни
            // другого (пример 13, у Яндекса крестовина пуста)
            // на грунтовке стоп-линии нет ничем не званой: краски на грунте не
            // бывает, и обрывки линии уступи дорогу поперёк щебня читались
            // мусором (Калуга, 06: знак `give_way` на Новаторском) — знак
            // остаётся знаком
            let called = !road.is_unpaved_street()
                && (zebra.is_some() || signalized || major || sign(arm.road).is_some());
            // на перемычке сложного узла стоп-линии нет: она легла бы на
            // замощённый остров между его узлами (пример 06)
            // у кольца без зебры — линия уступи дорогу по его кромке; кромка
            // плеча там, где из кольца вышло всё сечение, и краска подхода
            // рвётся до неё
            let ring_line = ring_line.filter(|_| zebra.is_none());
            // у кольца приоритет: поперёк его дуги стоп-линии нет — она
            // ложилась через все его полосы у въезда (пример 04, юг)
            let stop = (style.stop_lines
                && called
                && !link
                && incoming(road, dir)
                && !ring_road(arm.road))
            .then(|| {
                let behind = match zebra {
                    Some((center, _)) => center + dir * (ZEBRA_LENGTH / 2.0 + STOP_GAP),
                    None if ring_line.is_some() => edge,
                    None => edge + dir * ZEBRA_SETBACK,
                };
                behind + dir * STOP_WIDTH / 2.0
            })
            .filter(|&at| ring_line.is_some() || !in_other(at))
            .filter(|&at| {
                // очереди за линией нужно место: до асфальта следующего узла
                // и до зебры перехода по пути к ней
                let back = at + dir * STOP_WIDTH / 2.0;
                let zebra_ahead = crossings[arm.road]
                    .iter()
                    .map(|crossing| (crossing.along - back) * dir - ZEBRA_LENGTH / 2.0)
                    .filter(|&gap| gap > -ZEBRA_LENGTH)
                    .fold(f32::INFINITY, f32::min);
                let queue = ((edge - back) * dir + next_edge).min(zebra_ahead);
                ring_line.is_some() || queue >= STOP_QUEUE_ROOM
            });
            let outer = [
                zebra.map(|(center, _)| center + dir * ZEBRA_LENGTH / 2.0),
                stop.map(|at| at + dir * STOP_WIDTH / 2.0),
            ]
            .into_iter()
            .flatten()
            .max_by(|a, b| (a * dir).total_cmp(&(b * dir)));
            // Плечо без краски — уходящая из узла односторонняя, плечо без
            // зебры и стоп-линии — рвётся всё равно до кромки, а не на
            // полуширине соседа: на пологой крестовине чужой асфальт тянется
            // вдоль плеча на десяток метров, и линии полос шли по полю
            // перекрёстка (Орёл, витрина 05: Московская под 23° к паре
            // Пушкина — соседу в одну полосу хватало трёх метров)
            let Some(outer) = outer else {
                let edge = if link {
                    lines_edge(&arm, &walk).unwrap_or(edge)
                } else {
                    edge
                };
                // кромка на самой досягаемости разрыва — разрыв уже есть
                let node = walk.project(arm.at);
                if (edge - node).abs() > reach_of(arm.road) + EDGE_STEP / 2.0 {
                    self.breaks[arm.road].extend(walk.gap(node, edge));
                    self.spills.extend(walk.spills(edge, edge));
                }
                continue;
            };
            // плечо короче краски с хвостом — ничего: это перемычка внутри
            // сложного узла, а не подход к нему
            if walk.at(outer + dir * (PAINT_CLEAR + ARM_TAIL)).is_none() {
                continue;
            }
            if let Some(index) = osm {
                crossings[arm.road][index].used = true;
            }
            if let Some((center, osm)) = zebra
                && let Some(found) = zebra_at(&walk, road, center, osm)
            {
                zebra_of[plan_index] = Some(self.zebras.len());
                self.zebras.push(found);
                self.zebra_roads.push(arm.road);
            }
            if let Some((from, to)) = ring_line.filter(|_| stop.is_some()) {
                // въезд кольцу уступает всегда, светофор — не уступает
                self.stop_lines.push(StopLine {
                    from,
                    to,
                    yields: !signalized,
                });
            } else if let Some(at) = stop {
                let yields = !signalized && sign(arm.road) == Some(Sign::GiveWay);
                // линия параллельна планке зебры этого плеча
                let parallel = zebra
                    .and_then(|(center, _)| walk.at(center))
                    .map(|(_, direction)| direction);
                self.stop_lines.extend(stop_line_at(
                    &walk,
                    road,
                    at,
                    dir,
                    map.traffic_side,
                    yields,
                    parallel,
                ));
            }
            // линии рвутся от узла: кромка по асфальту бывает дальше
            // полуширины соседа, и между ними оставался штрих
            let node = walk.project(arm.at);
            self.breaks[arm.road].extend(walk.gap(node, outer + dir * PAINT_CLEAR));
            self.spills
                .extend(walk.spills(edge, outer + dir * PAINT_CLEAR));
        }
        // пары по асфальтовой разделительной — одной планкой: полосы шейдер
        // считает от её края, и на двух планках они сбивались на шве
        let mut merged: Vec<usize> = Vec::new();
        for [a, b] in joined {
            let (Some(a), Some(b)) = (zebra_of[a], zebra_of[b]) else {
                continue;
            };
            if let Some(one) = join_zebras(&self.zebras[a], &self.zebras[b]) {
                self.zebras[a] = one;
                merged.push(b);
            }
        }
        merged.sort_unstable();
        for index in merged.into_iter().rev() {
            debug_assert!(index >= first_zebra);
            self.zebras.remove(index);
            self.zebra_roads.remove(index);
        }
    }

    /// Переход не у узла: зебра с разрывом линий вокруг и стоп-линиями по
    /// обе стороны, если он регулируемый.
    pub(super) fn paint_crossing(
        &mut self,
        road: &RoadLine,
        path: &[Vec2],
        index: usize,
        crossing: &Crossing,
        style: NodePaintStyle,
        side: TrafficSide,
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
        self.zebra_roads.push(index);
        let mut reach = ZEBRA_LENGTH / 2.0;
        if style.stop_lines && crossing.signals {
            let offset = ZEBRA_LENGTH / 2.0 + STOP_GAP + STOP_WIDTH / 2.0;
            let parallel = walk.at(center).map(|(_, direction)| direction);
            for dir in [-1.0, 1.0] {
                if incoming(road, dir) {
                    self.stop_lines.extend(stop_line_at(
                        &walk,
                        road,
                        center + dir * offset,
                        dir,
                        side,
                        false,
                        parallel,
                    ));
                }
            }
            reach = offset + STOP_WIDTH / 2.0;
        }
        let (from, to) = (center - reach - PAINT_CLEAR, center + reach + PAINT_CLEAR);
        self.breaks[index].extend(walk.gap(from, to));
        self.spills.extend(walk.spills(from, to));
    }
}

/// Замкнута ли ломаная — кольцо одним way.
fn is_closed(points: &[Vec2]) -> bool {
    points.len() > 2 && points[0] == points[points.len() - 1]
}

/// То, что `paint_cluster` читает, одним аргументом.
pub(super) struct Context<'a, P> {
    pub(super) drawn: &'a [&'a RoadLine],
    pub(super) paths: &'a [P],
    pub(super) map: &'a MapData,
    pub(super) marks: &'a HashMap<(i32, i32), RoadNodeKind>,
    pub(super) near_marks: &'a Grid<usize>,
    pub(super) style: NodePaintStyle,
    pub(super) street: &'a dyn Fn(usize) -> usize,
    pub(super) sidewalk: &'a dyn Fn(usize) -> bool,
    pub(super) partners: &'a dyn Fn(usize) -> Vec<Partner>,
    pub(super) on_ring: &'a dyn Fn(usize) -> bool,
    /// Узлы на каждой дороге.
    pub(super) nodes_along: &'a [Vec<NodeAlong>],
    /// Замощённые острова треугольников узлов.
    pub(super) paved: &'a [PavedIsland],
}

impl<P> Clone for Context<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for Context<'_, P> {}

/// Есть ли на плече, уходящем от узла в сторону `dir`, полосы к узлу:
/// односторонняя едет по ходу точек, к узлу — только с плеча к началу.
fn incoming(road: &RoadLine, dir: f32) -> bool {
    !road.oneway || dir < 0.0
}

/// Стоп-линия на длине `at` поперёк полос, едущих к узлу с плеча `dir`:
/// у двусторонней — от оси до кромки по стороне движения, у односторонней —
/// во всю ширину. Со зеброй на плече (`align` — направление её планки) линия
/// той параллельна: обе меряет одна ось, но на своих станциях, и на изгибе
/// устья они расходились углом (Одоевский путепровод, R37-04).
fn stop_line_at(
    walk: &Walk,
    road: &RoadLine,
    at: f32,
    dir: f32,
    side: TrafficSide,
    yields: bool,
    align: Option<Vec2>,
) -> Option<StopLine> {
    let (point, along) = walk.at(at)?;
    let direction = align.unwrap_or(along);
    let travel = -direction * dir;
    let right = Vec2::new(travel.y, -travel.x);
    let kerb = match side {
        TrafficSide::Right => right,
        TrafficSide::Left => -right,
    } * (road.width / 2.0 - EDGE_INSET);
    // у двусторонней — от осевой: у нечётной она не посередине
    let from = if road.oneway {
        point - kerb
    } else {
        let axis = axis_offset(road, lane_count(road), side).unwrap_or(0.0);
        point + direction.perp() * axis + kerb.normalize_or_zero() * EDGE_INSET
    };
    Some(StopLine {
        from,
        to: point + kerb,
        yields,
    })
}
