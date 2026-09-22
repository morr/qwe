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
//!   из крайней у бордюра, в дальний — только из крайней у оси. Односторонняя
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

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use super::lane_count;
use super::network::sections::STREET_LANE_WIDTH;
use super::node_paint::{Junction, JunctionArm};
use super::paint::lane_frame;
use crate::map::along::{arclengths, place_on_path};
use crate::map::osm::{LaneTurn, RoadLine, TrafficSide};

/// Хвост траектории за кромкой узла, м — столько же, на скольких колея
/// полосы набирает силу от разрыва (`WEAR_FADE` в `surface.wgsl`).
pub const TURN_TAIL: f32 = 5.0;
/// Манёвр с углом меньше этого — прямо, рад (35°).
const STRAIGHT: f32 = 0.61;
/// Больше этого — разворот, рад (150°).
const U_TURN: f32 = 2.62;
/// Насколько хорда звена кривой может отойти от дуги, м: по 15° на звено
/// колея на повороте шла заметными гранями.
const SAGITTA: f32 = 0.03;
/// Звено не мельче этого угла, рад (3°).
const ARC_STEP_MIN: f32 = 0.052;

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
}

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
}

/// Полосы плеча в одном направлении, от бордюра к оси, и манёвры из них по
/// `turn:lanes` (тоже от бордюра), если тег сошёлся с раскладкой.
struct ArmLanes {
    lanes: Vec<LaneEnd>,
    turns: Option<Vec<LaneTurn>>,
}

impl Turns {
    /// Траектории по узлам `junctions` дорог `drawn`, нарисованных по `paths`.
    pub fn new(
        drawn: &[&RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        junctions: &[Junction],
        side: TrafficSide,
    ) -> Self {
        let mut turns = Self::default();
        if drawn.len() != paths.len() {
            return turns;
        }
        for junction in junctions {
            turns.junction(drawn, paths, junction, side);
        }
        turns
    }

    fn junction(
        &mut self,
        drawn: &[&RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        junction: &Junction,
        side: TrafficSide,
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
        for (a, from) in junction.arms.iter().enumerate() {
            let Some(first) = ins[a].lanes.first() else {
                continue;
            };
            for (b, to) in junction.arms.iter().enumerate() {
                let Some(target) = outs[b].lanes.first() else {
                    continue;
                };
                if a == b || (from.road == to.road && from.dir == to.dir) {
                    continue;
                }
                let angle = first.travel.angle_to(target.travel);
                if angle.abs() > U_TURN {
                    continue;
                }
                let near_is_left = side == TrafficSide::Left;
                let maneuver = if angle.abs() < STRAIGHT {
                    Maneuver::Straight
                } else if (angle > 0.0) == near_is_left {
                    Maneuver::Near
                } else {
                    Maneuver::Far
                };
                // прямо по ведущей — колеёй асфальта
                if maneuver == Maneuver::Straight
                    && junction.leading.contains(&from.road)
                    && junction.leading.contains(&to.road)
                {
                    continue;
                }
                for (start, end) in pairs(&ins[a], outs[b].lanes.len(), maneuver, side) {
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
        }
        if !wear.curves.is_empty() {
            self.wear.push(wear);
        }
    }
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
        .map(|index| frame.low + (f32::from(index) + 0.5) * STREET_LANE_WIDTH)
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

/// Какие полосы в какие: `(входящая, выходящая)`, обе от бордюра.
fn pairs(
    from: &ArmLanes,
    out: usize,
    maneuver: Maneuver,
    side: TrafficSide,
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
