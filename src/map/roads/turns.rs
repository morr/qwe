//! **Траектории в узле** — кривые «полоса → полоса» для разрешённых манёвров.
//!
//! Колея асфальта (`surface.wgsl`) идёт по полосам и в узле гаснет: машина
//! поперёк перекрёстка едет не по полосе. Но едет она не где придётся, а по
//! манёвру — прямо, направо, налево, — и накатанный центр узла в жизни светлее
//! подходов. Эти кривые:
//!
//! - **разрешения** — `turn:lanes` (`RoadLine::turns`), если число полос в
//!   теге совпало с раскладкой; иначе правило: прямо — из каждой полосы в
//!   свою, в ближний поворот (направо при правостороннем движении) — только
//!   из крайней у бордюра, в дальний — только из крайней у оси; на подходе в
//!   торец Т, где прямо некуда, средние полосы поворачивают в обе стороны, а
//!   крайние — каждая только в свою. Односторонняя
//!   везёт только по ходу точек, `MapData::traffic_side` решает, где бордюр;
//! - **кривая** — кубическая Безье от середины полосы на кромке узла
//!   ([`JunctionArm::edge`]) до середины полосы на кромке плеча-цели,
//!   касательная к обеим, звенья — по стрелке прогиба [`SAGITTA`]; и хвост
//!   [`TURN_TAIL`] вглубь каждой полосы, один на полосу: на хвосте колея
//!   траектории гаснет, а колея полосы, погашенная у разрыва, набирает силу —
//!   одна вливается в другую;
//! - **прямо по ведущей дороге кривой нет**: её колея идёт через узел
//!   асфальтом (`NodePaint::asphalt`). У равных дорог, где ведущей нет, обе
//!   прямые — траекториями, крест-накрест, вполсилы колеи полос;
//! - **разворот** (угол больше [`U_TURN`]) не рисуется.
//!
//! Колея кривых ложится **как тень** — перекрытие не светлее одной колеи
//! (`paint::PaintPass`).

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use super::lane_count;
use super::node_paint::{Junction, JunctionArm};
use super::paint::lane_frame;
use super::shape::lane_width;
use super::tapers;
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::miter_offsets;
use crate::map::osm::{LaneTurn, RoadLine, TrafficSide};

/// Хвост траектории за кромкой узла, м — столько же, на скольких колея
/// полосы набирает силу от разрыва (`WEAR_FADE` в `surface.wgsl`).
pub const TURN_TAIL: f32 = 5.0;
/// Манёвр с углом меньше этого — прямо, рад.
const STRAIGHT: f32 = 35.0 * PI / 180.0;
/// Больше этого — разворот, рад.
const U_TURN: f32 = 150.0 * PI / 180.0;
/// Насколько хорда звена кривой может отойти от дуги, м: по 15° на звено
/// колея на повороте шла заметными гранями.
const SAGITTA: f32 = 0.03;
/// Звено не мельче этого угла, рад.
const ARC_STEP_MIN: f32 = 3.0 * PI / 180.0;

/// Манёвр по отношению к бордюру: ближний поворот — к своему бордюру
/// (направо при правостороннем движении), дальний — через встречные.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Maneuver {
    Straight,
    Near,
    Far,
}

/// Траектории узлов карты.
#[derive(Default)]
pub struct Turns {
    /// Колея по узлам — в слой износа (`paint::Painter::paint_turn_wear`).
    pub wear: Vec<JunctionWear>,
    /// Сколько всего кривых манёвров.
    pub maneuvers: usize,
    /// Стрелки на полосах подходов: по `turn:lanes`, а без тега — у крупных
    /// узлов по тем же манёврам, что и кривые ([`ARROW_MIN_LANES`]).
    pub arrows: Vec<LaneArrow>,
}

/// Стрелка на полосе подхода к узлу: середина полосы на кромке узла, куда по
/// ней едут и какие манёвры из неё разрешены. `left` / `right` — как в
/// `turn:lanes`, по сторонам хода.
///
/// `back` — ось полосы от кромки назад, против хода, на [`ARROW_BACK`]: по ней
/// стрелка встаёт на свой отступ. Прямая от кромки по ходу на самой кромке
/// годится только у прямого подхода — на изогнутом стрелка за двадцать метров
/// съезжала с полосы на газон (Лейпцигер-штрассе, витрина Берлина).
#[derive(Clone, Debug)]
pub struct LaneArrow {
    pub at: Vec2,
    pub travel: Vec2,
    pub turn: LaneTurn,
    pub back: Vec<Vec2>,
}

/// Сколько оси полосы за кромкой узла несёт стрелка, м: дальше всякой зебры и
/// стоп-линии, за которыми она встаёт (`paint::ARROW_MARK_REACH` 30 м), плюс
/// её отступ и длина.
const ARROW_BACK: f32 = 45.0;

/// Без `turn:lanes` стрелки рисуются только на подходе с этим числом полос
/// своего направления и больше — у крупного узла; у двухполосной улицы на
/// каждом дворовом перекрёстке их не рисуют.
pub const ARROW_MIN_LANES: usize = 2;

/// Колея одного узла: кривые манёвров от кромки до кромки и хвосты
/// [`TURN_TAIL`] вглубь полос — по одному на полосу, сколько бы манёвров из
/// неё ни выходило: одинаковые хвосты — лишние вершины.
#[derive(Default)]
pub struct JunctionWear {
    pub curves: Vec<Vec<Vec2>>,
    /// `[у кромки, в глубине полосы]`.
    pub tails: Vec<[Vec2; 2]>,
}

/// Полоса на кромке плеча: середина и направление движения по ней.
#[derive(Clone, Copy, Debug)]
pub(super) struct LaneEnd {
    pub point: Vec2,
    pub travel: Vec2,
    /// Сдвиг середины полосы от оси пути, м (плюс — влево по ходу точек).
    pub offset: f32,
}

/// Полосы плеча в одном направлении, от бордюра к оси, и манёвры из них по
/// `turn:lanes` (тоже от бордюра), если тег сошёлся с раскладкой.
struct ArmLanes {
    lanes: Vec<LaneEnd>,
    turns: Option<Vec<LaneTurn>>,
}

impl Turns {
    /// Траектории по узлам `junctions` дорог `drawn`, нарисованных по `paths`.
    /// `on_ring(дорога)` — дуга ли она кольца (`roads/rings.rs`): у такого
    /// узла стрелок по правилу нет.
    pub fn new(
        drawn: &[&RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        junctions: &[Junction],
        side: TrafficSide,
        on_ring: impl Fn(usize) -> bool,
    ) -> Self {
        let mut turns = Self::default();
        if drawn.len() != paths.len() {
            return turns;
        }
        for junction in junctions {
            turns.junction(drawn, paths, junction, side, &on_ring);
        }
        turns
    }

    fn junction(
        &mut self,
        drawn: &[&RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        junction: &Junction,
        side: TrafficSide,
        on_ring: &impl Fn(usize) -> bool,
    ) {
        let lanes = |arm: &JunctionArm, incoming: bool| {
            arm_lanes(
                drawn[arm.road],
                paths[arm.road].as_ref(),
                arm,
                incoming,
                side,
            )
        };
        let ins: Vec<ArmLanes> = junction.arms.iter().map(|arm| lanes(arm, true)).collect();
        let outs: Vec<ArmLanes> = junction.arms.iter().map(|arm| lanes(arm, false)).collect();
        let mut wear = JunctionWear::default();
        // хвост — один на полосу: входящая `(плечо, полоса, false)`
        let mut tailed: Vec<(usize, usize, bool)> = Vec::new();
        let near_is_left = side == TrafficSide::Left;
        for (a, from) in junction.arms.iter().enumerate() {
            let Some(first) = ins[a].lanes.first() else {
                continue;
            };
            // манёвры полос плеча по правилу — для стрелок, если тега нет
            let mut granted = vec![LaneTurn::default(); ins[a].lanes.len()];
            let maneuver_to = |b: usize| -> Option<Maneuver> {
                let to = &junction.arms[b];
                let target = outs[b].lanes.first()?;
                if a == b || (from.road == to.road && from.dir == to.dir) {
                    return None;
                }
                let angle = first.travel.angle_to(target.travel);
                if angle.abs() > U_TURN {
                    return None;
                }
                Some(if angle.abs() < STRAIGHT {
                    Maneuver::Straight
                } else if (angle > 0.0) == near_is_left {
                    Maneuver::Near
                } else {
                    Maneuver::Far
                })
            };
            // подход в торец Т: прямо некуда, и средние полосы поворачивают
            // в обе стороны — крайние только в свою
            let dead_end =
                !(0..junction.arms.len()).any(|b| maneuver_to(b) == Some(Maneuver::Straight));
            for (b, to) in junction.arms.iter().enumerate() {
                let Some(maneuver) = maneuver_to(b) else {
                    continue;
                };
                let lanes = lane_pairs(&ins[a], outs[b].lanes.len(), maneuver, side, dead_end);
                for &(start, _) in &lanes {
                    let turn = &mut granted[start];
                    match maneuver {
                        Maneuver::Straight => turn.through = true,
                        // налево — поворот к оси при правостороннем
                        Maneuver::Near if near_is_left => turn.left = true,
                        Maneuver::Far if near_is_left => turn.right = true,
                        Maneuver::Near => turn.right = true,
                        Maneuver::Far => turn.left = true,
                    }
                }
                // прямо по ведущей — колеёй асфальта
                if maneuver == Maneuver::Straight
                    && junction.leading.contains(&from.road)
                    && junction.leading.contains(&to.road)
                {
                    continue;
                }
                for (start, end) in lanes {
                    let (from, to) = (ins[a].lanes[start], outs[b].lanes[end]);
                    wear.curves.push(curve(from, to));
                    self.maneuvers += 1;
                    for (key, point, outward) in [
                        ((a, start, false), from.point, -from.travel),
                        ((b, end, true), to.point, to.travel),
                    ] {
                        if !tailed.contains(&key) {
                            tailed.push(key);
                            wear.tails.push([point, point + outward * TURN_TAIL]);
                        }
                    }
                }
            }
            // стрелки: по тегу, иначе по правилу на многополосном подходе;
            // на мосту своя краска, стрелок там нет
            // у кольца манёвр — «въехать в кольцо», и стрелки по правилу там
            // врут: только по тегу; и там, где выбора нет — одно «прямо» на
            // развилке разделённой улицы, где вторая ветка уходит разворотом
            let ring = junction.arms.iter().any(|arm| on_ring(arm.road));
            let turns = match &ins[a].turns {
                Some(tagged) => tagged.clone(),
                None if !ring && ins[a].lanes.len() >= ARROW_MIN_LANES && has_choice(&granted) => {
                    granted
                }
                None => continue,
            };
            if drawn[from.road].bridge {
                continue;
            }
            let path = paths[from.road].as_ref();
            for (lane, turn) in ins[a].lanes.iter().zip(turns) {
                if turn != LaneTurn::default() {
                    self.arrows.push(LaneArrow {
                        at: lane.point,
                        travel: lane.travel,
                        turn,
                        back: lane_back(path, from, lane.offset),
                    });
                }
            }
        }
        if !wear.curves.is_empty() {
            self.wear.push(wear);
        }
    }
}

/// Есть ли у подхода выбор: полосы вместе дают хотя бы два разных манёвра.
/// Стрелка «только прямо» на каждой полосе ничего не говорит водителю.
fn has_choice(turns: &[LaneTurn]) -> bool {
    let any = |pick: fn(&LaneTurn) -> bool| turns.iter().any(pick);
    [
        any(|turn| turn.left),
        any(|turn| turn.through),
        any(|turn| turn.right),
    ]
    .into_iter()
    .filter(|&kind| kind)
    .count()
        >= 2
}

/// Ось входящей полосы со сдвигом `offset` от кромки плеча `arm` назад, против
/// хода, на [`ARROW_BACK`]: первая точка — на кромке.
fn lane_back(path: &[Vec2], arm: &JunctionArm, offset: f32) -> Vec<Vec2> {
    let (along, _) = arclengths(path);
    let total = along.last().copied().unwrap_or(0.0);
    // входящая едет к узлу: по ходу точек, если узел у конца пути
    let toward_end = -arm.dir > 0.0;
    let piece = if toward_end {
        tapers::cut(path, (arm.edge - ARROW_BACK).max(0.0), arm.edge)
    } else {
        tapers::cut(path, arm.edge, (arm.edge + ARROW_BACK).min(total))
    };
    if piece.len() < 2 {
        return Vec::new();
    }
    let offsets = miter_offsets(&piece, false, offset);
    let mut lane: Vec<Vec2> = piece
        .iter()
        .zip(offsets)
        .map(|(point, shift)| *point + shift)
        .collect();
    if toward_end {
        lane.reverse();
    }
    lane
}

/// Полосы плеча `arm` на его кромке: входящие в узел (`incoming`) или
/// выходящие, от бордюра к оси.
fn arm_lanes(
    road: &RoadLine,
    path: &[Vec2],
    arm: &JunctionArm,
    incoming: bool,
    side: TrafficSide,
) -> ArmLanes {
    let none = ArmLanes {
        lanes: Vec::new(),
        turns: None,
    };
    let (along, _) = arclengths(path);
    let Some((point, direction)) = place_on_path(path, &along, arm.edge) else {
        return none;
    };
    // по ходу точек или против — куда едут по полосам этого плеча
    let travel_sign = if incoming { -arm.dir } else { arm.dir };
    if road.oneway && travel_sign < 0.0 {
        return none;
    }
    let count = lane_count(road);
    let frame = lane_frame(count);
    // сторона бордюра по ходу движения, в раме пути (плюс — влево по ходу
    // точек): справа по ходу при правостороннем
    let kerb = match side {
        TrafficSide::Right => -travel_sign,
        TrafficSide::Left => travel_sign,
    };
    let mut offsets: Vec<f32> = (0..count)
        .map(|index| frame.low + (f32::from(index) + 0.5) * lane_width())
        // средняя полоса нечётной двусторонней — ничья, кроме единственной:
        // по однополосной едут в обе стороны
        .filter(|offset| road.oneway || count == 1 || offset * kerb > 1e-3)
        .collect();
    offsets.sort_by(|a, b| (b * kerb).total_cmp(&(a * kerb)));
    let normal = direction.perp();
    let travel = direction * travel_sign;
    let lanes: Vec<LaneEnd> = offsets
        .iter()
        .map(|&offset| LaneEnd {
            point: point + normal * offset,
            travel,
            offset,
        })
        .collect();
    // `turn:lanes` — слева направо по ходу движения; от бордюра — это справа
    // налево при правостороннем
    let tagged = &road.turns[usize::from(travel_sign < 0.0)];
    let turns = (incoming && tagged.len() == lanes.len()).then(|| match side {
        TrafficSide::Right => tagged.iter().rev().copied().collect(),
        TrafficSide::Left => tagged.clone(),
    });
    ArmLanes { lanes, turns }
}

/// Какие полосы в какие: `(входящая, выходящая)`, обе от бордюра. По правилу
/// крайняя к повороту полоса поворачивает (и едет прямо), прочие — только
/// прямо; на подходе в торец Т (`dead_end`) прямо нет, и поворачивают все,
/// кроме крайней с другой стороны.
fn lane_pairs(
    from: &ArmLanes,
    out: usize,
    maneuver: Maneuver,
    side: TrafficSide,
    dead_end: bool,
) -> Vec<(usize, usize)> {
    let count = from.lanes.len();
    if count == 0 || out == 0 {
        return Vec::new();
    }
    // полосы, из которых манёвр разрешён, — от бордюра
    let allowed: Vec<usize> = match &from.turns {
        Some(turns) => (0..count)
            .filter(|&lane| {
                let turn = turns[lane];
                let (near, far) = match side {
                    TrafficSide::Right => (turn.right, turn.left),
                    TrafficSide::Left => (turn.left, turn.right),
                };
                match maneuver {
                    Maneuver::Straight => turn.through,
                    Maneuver::Near => near,
                    Maneuver::Far => far,
                }
            })
            .collect(),
        None => match maneuver {
            Maneuver::Straight => (0..count.min(out)).collect(),
            Maneuver::Near if dead_end && count > 1 => (0..count - 1).collect(),
            Maneuver::Far if dead_end && count > 1 => (1..count).collect(),
            Maneuver::Near => vec![0],
            Maneuver::Far => vec![count - 1],
        },
    };
    match maneuver {
        // дальний поворот считает полосы от оси: крайняя левая — в крайнюю
        // левую
        Maneuver::Far => allowed
            .iter()
            .rev()
            .enumerate()
            .map(|(rank, &lane)| (lane, out - 1 - rank.min(out - 1)))
            .collect(),
        _ => allowed
            .iter()
            .enumerate()
            .map(|(rank, &lane)| (lane, rank.min(out - 1)))
            .collect(),
    }
}

/// Кривая из полосы `from` в полосу `to`: Безье, касательная к обеим. Ей же
/// подход входит в кольцо (`roads/rings.rs`).
pub(super) fn curve(from: LaneEnd, to: LaneEnd) -> Vec<Vec2> {
    let chord = from.point.distance(to.point);
    let angle = from.travel.angle_to(to.travel).abs();
    // плечо контрольных точек: треть хорды у прямой, к четверти окружности —
    // 0.39 хорды, как у дуги
    let arm = chord * (1.0 / 3.0 + 0.06 * (angle / FRAC_PI_2).min(1.0));
    let controls = [
        from.point,
        from.point + from.travel * arm,
        to.point - to.travel * arm,
        to.point,
    ];
    // сдвиг поперёк хода — прямо, но из полосы в соседнюю: S-кривая
    let shift = (to.point - from.point).perp_dot(from.travel).abs();
    // звено — по стрелке прогиба: у дуги радиуса r угол звена, при котором
    // хорда отходит от дуги на [`SAGITTA`]
    let radius = chord / (2.0 * (angle / 2.0).sin()).max(1e-3);
    let step = (2.0 * (1.0 - SAGITTA / radius).clamp(-1.0, 1.0).acos()).max(ARC_STEP_MIN);
    let segments = ((angle / step).ceil() as usize).max(if shift > 0.5 { 4 } else { 1 });
    (0..=segments)
        .map(|step| bezier(controls, step as f32 / segments as f32))
        .collect()
}

fn bezier([a, b, c, d]: [Vec2; 4], t: f32) -> Vec2 {
    let u = 1.0 - t;
    a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t)
}

#[cfg(test)]
mod tests;
