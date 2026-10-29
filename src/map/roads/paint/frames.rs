//! Раскладка полос — сетка линий и границы проезжей части поперёк оси.
//!
//! Правила — в документе модуля `paint` (раскладка и клинья).

use bevy::prelude::*;

use super::axis_offset;
use crate::map::meshing::LaneFrame;
use crate::map::osm::model::polyline_length;
use crate::map::osm::{LaneTurn, RoadLine, TrafficSide};
use crate::map::roads::lane_count;
use crate::map::roads::shape::lane_width;
use crate::map::roads::tapers::{self, Taper, Tapers};

/// На клине расширения (R25) новая полоса не размечается, пока не наберёт
/// эту долю ширины полосы: дальше её линия идёт сразу полной. Проявляющийся
/// пунктир ([`BIRTH_FADE`]) в настоящей разметке не встречается; он остался
/// только у клина слияния (`roads/merges.rs`).
pub(super) const BIRTH_WIDTH: f32 = 0.8;
/// Допуск порога [`BIRTH_WIDTH`], м: вершина рождения стоит на нём самом.
pub(super) const BIRTH_SLACK: f32 = 1e-3;
/// Шаг от вершины рождения к шву, м: звено между ними — растяжка
/// прозрачности, и оно в два сантиметра.
pub(super) const BIRTH_EDGE: f32 = 0.02;
/// Сколько делений пополам ищет порог [`BIRTH_WIDTH`] ([`births`]): доля
/// клина до 2⁻²⁴ — точнее, чем держит `f32`.
const BIRTH_BISECTIONS: u32 = 24;

/// Раскладка полос тела way с `lanes` полосами: границы проезжей части на
/// `± lanes · шаг / 2`, узел сетки — на оси при чётном числе полос и в
/// полполосы от неё при нечётном. Раскладка симметрична: у зеркального way
/// сетка та же.
pub fn lane_frame(lanes: u8) -> LaneFrame {
    let half = f32::from(lanes) * lane_width() / 2.0;
    LaneFrame {
        origin: if lanes.is_multiple_of(2) {
            0.0
        } else {
            lane_width() / 2.0
        },
        low: -half,
        high: half,
    }
}

/// Раскладка узкого соседа у торца клина — в раме way, от тела `body`:
/// сетка та же по модулю шага, но узел выбран так, чтобы на пути **от шва к
/// телу** он сдвигался на `[0, шаг)` вперёд — влево по ходу клина. Новая полоса
/// при смене чётности поэтому рождается справа по ходу клина. У торца `end`
/// клин идёт против хода way, отсюда знак.
///
/// У односторонней улицы `drift` ([`wedge_drift`]) — знак стороны, к которой
/// прижаты общие полосы: узел сдвинут на всю разницу полуширин, и на теле
/// полосы узкого сечения стоят у этой кромки, а новые рождаются у другой.
pub(super) fn narrow_frame(
    body: LaneFrame,
    narrow_lanes: u8,
    end: bool,
    drift: Option<f32>,
) -> LaneFrame {
    let narrow = lane_frame(narrow_lanes);
    let origin = match drift {
        Some(sign) => body.origin - sign * (body.high - narrow.high),
        None => {
            let shift = (body.origin - narrow.origin).rem_euclid(lane_width());
            if end {
                body.origin + shift
            } else {
                body.origin - shift
            }
        }
    };
    LaneFrame { origin, ..narrow }
}

/// Куда плывут линии клина улицы `road`: у двусторонней — никуда (`None`,
/// клин симметричен, по полосе с каждой стороны), у односторонней — знак
/// стороны, где полосы общие. Полосы прибавляются у бордюра по
/// `traffic_side` — съезд, карман направо (пример 16 витрины, Советская
/// 4 → 2 к правому съезду); у дальней кромки — если `turn:lanes` широкого
/// участка начинается с полосы только налево, а кончается не полосой только
/// направо: это карман левого поворота. `+` — влево по ходу way.
pub fn wedge_drift(road: &RoadLine, side: TrafficSide) -> Option<f32> {
    if !road.oneway {
        return None;
    }
    let kerb = match side {
        TrafficSide::Right => 1.0,
        TrafficSide::Left => -1.0,
    };
    let turns = &road.turns[0];
    let only_left = |turn: &LaneTurn| turn.left && !turn.through && !turn.right;
    let only_right = |turn: &LaneTurn| turn.right && !turn.through && !turn.left;
    let far_pocket = match side {
        TrafficSide::Right => {
            turns.first().is_some_and(only_left) && !turns.last().is_some_and(only_right)
        }
        TrafficSide::Left => {
            turns.last().is_some_and(only_right) && !turns.first().is_some_and(only_left)
        }
    };
    Some(if far_pocket { -kerb } else { kerb })
}

/// Раскладка, отражённая поперёк пути: так её видит путь, идущий навстречу.
pub(super) fn mirrored(frame: LaneFrame) -> LaneFrame {
    LaneFrame {
        origin: -frame.origin,
        low: -frame.high,
        high: -frame.low,
    }
}

/// Раскладка у шва клина, суженного на одну сторону (`Taper::kept`): кромка
/// `kept` (`+1` — левая по ходу way) идёт прямо, и полосы узкого соседа
/// прижаты к ней — сетка тела, граница с другой стороны придвинута на
/// недостающие полосы. Новые полосы рождаются у сужаемой кромки, вместе с
/// самой кромкой.
fn kept_frame(body: LaneFrame, narrow_lanes: u8, kept: f32) -> LaneFrame {
    let span = f32::from(narrow_lanes) * lane_width();
    if kept > 0.0 {
        LaneFrame {
            low: body.high - span,
            ..body
        }
    } else {
        LaneFrame {
            high: body.low + span,
            ..body
        }
    }
}

/// Где на клине длиной `length` рождаются линии (R25): метры от шва, где
/// линия, которой у шва (раскладка `seam`) места нет, по пути к телу `body`
/// впервые получает [`BIRTH_WIDTH`] полосы с обеих сторон. Раскладка между
/// ними — линейная, место линии — минимум двух линейных, и порог ищется
/// делением пополам.
pub(super) fn births(body: LaneFrame, seam: LaneFrame, length: f32) -> Vec<f32> {
    let need = BIRTH_WIDTH * lane_width();
    let lowest = ((seam.low - seam.origin).min(body.low - body.origin) / lane_width()).floor();
    let highest = ((seam.high - seam.origin).max(body.high - body.origin) / lane_width()).ceil();
    let mut found = Vec::new();
    for k in lowest as i32..=highest as i32 {
        let step = k as f32 * lane_width();
        let room = |t: f32| {
            let frame = seam.lerp(body, t);
            let at = frame.origin + step;
            (at - frame.low).min(frame.high - at)
        };
        if room(0.0) >= need - BIRTH_SLACK || room(1.0) < need - BIRTH_SLACK {
            continue;
        }
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..BIRTH_BISECTIONS {
            let middle = (low + high) / 2.0;
            if room(middle) >= need {
                high = middle;
            } else {
                low = middle;
            }
        }
        found.push(high * length);
    }
    found
}

/// Раскладка у шва клина `wedge` в раме way — по его форме: суженный на одну
/// сторону — [`kept_frame`], симметричный — [`narrow_frame`].
/// Узел, выставленный по осевой соседа ([`WedgeEnd::origin`]), берёт у шва
/// раскладку соседа целиком, какой бы формы ни был клин: сетка стоит на его
/// линиях, и прижатая к кромке раскладка ([`kept_frame`]) со сдвинутым узлом
/// оставляла бы по полполосы у обеих кромок.
pub(super) fn wedge_frame(body: LaneFrame, wedge: WedgeEnd, end: bool) -> LaneFrame {
    if let Some(origin) = wedge.origin {
        return LaneFrame {
            origin,
            ..lane_frame(wedge.lanes)
        };
    }
    match wedge.kept {
        Some(kept) => kept_frame(body, wedge.lanes, kept),
        None => narrow_frame(body, wedge.lanes, end, wedge.drift),
    }
}

/// Раскладки клина `[у шва, у тела]` — в раме **пути клина**, который идёт от
/// шва к телу (`tapers::split`): у торца конца он смотрит против way, и
/// раскладка отражена. Их кладёт в асфальт клина `MeshBuilder::set_lane_taper`.
pub fn wedge_frames(body_lanes: u8, wedge: WedgeEnd, end: bool) -> [LaneFrame; 2] {
    let body = lane_frame(body_lanes);
    let narrow = wedge_frame(body, wedge, end);
    if end {
        [mirrored(narrow), mirrored(body)]
    } else {
        [narrow, body]
    }
}

/// Что лежит у торца way: клин длиной `length` от сечения соседа в `lanes`
/// полос; `drift` — куда плывут линии ([`wedge_drift`]); `kept` — кромка,
/// которую клин не трогает (`Taper::kept`), `None` у симметричного;
/// `origin` — узел сетки у шва, выставленный по осевой соседа
/// ([`seam_origin`]), `None` — по правилу формы клина.
#[derive(Clone, Copy, Debug)]
pub struct WedgeEnd {
    pub length: f32,
    pub lanes: u8,
    pub drift: Option<f32>,
    pub kept: Option<f32>,
    pub origin: Option<f32>,
}

impl WedgeEnd {
    /// Клин длиной `length` у торца `end` дороги `road`, начатый с сечения
    /// `seam` по клину `taper` — у сужающего клина сечение соседа уже своего,
    /// у встречного шире.
    pub fn new(
        road: &RoadLine,
        seam: &RoadLine,
        taper: Taper,
        end: bool,
        length: f32,
        side: TrafficSide,
    ) -> Self {
        Self {
            length,
            lanes: lane_count(seam),
            drift: wedge_drift(road, side),
            kept: taper.kept(),
            origin: seam_origin(road, seam, end, side),
        }
    }
}

/// Узел сетки у шва клина в раме way `road` (у торца `end`) такой, чтобы
/// **осевая шла через шов без скачка**: у шва она стоит там же, где осевая
/// соседа `seam` (у сужающего клина — узкого, у встречного — широкого), и на
/// длине клина плавно уходит на своё место.
/// Иначе у двусторонней, где у соседей разное деление потоков — Ростов,
/// Текучёва: `lanes=5, lanes:forward=3` к шести полосам, 3 + 2 → 3 + 3, —
/// осевая прыгала на полполосы: у пяти она на границе потоков, у шести —
/// посреди полотна. Сосед, нарисованный навстречу, видит осевую зеркально.
/// `None` — осевой нет у одного из двух или сдвиг больше полосы: тогда
/// раскладку у шва решает форма клина ([`wedge_frame`]).
pub fn seam_origin(road: &RoadLine, seam: &RoadLine, end: bool, side: TrafficSide) -> Option<f32> {
    let lanes = lane_count(road);
    let own = axis_offset(road, lanes, side)?;
    let theirs = axis_offset(seam, lane_count(seam), side)?;
    let (&first, &last) = (road.points.first()?, road.points.last()?);
    let seam_at = if end { last } else { first };
    let (&from, &to) = (seam.points.first()?, seam.points.last()?);
    // сосед идёт по ходу way, если в шов приходит его конец (у начала way)
    // или из шва выходит его начало (у конца)
    let enters = to.distance(seam_at) < from.distance(seam_at);
    let target = if enters != end { theirs } else { -theirs };
    // сдвиг — целое число полуполос; округление держит узел ровно на сетке,
    // иначе хвост в 1e-7 добавлял линию за кромкой (`ceil` в `Painter::paint`)
    let half = lane_width() / 2.0;
    let shift = ((target - own) / half).round() * half;
    (shift.abs() <= lane_width() + 1e-3).then(|| lane_frame(lanes).origin + shift)
}

/// Клинья у торцов way так, как их нарежет `tapers::split` по этому пути.
pub fn wedge_ends(
    path: &[Vec2],
    tapers: &Tapers,
    drawn: &[&RoadLine],
    road: usize,
    side: TrafficSide,
) -> [Option<WedgeEnd>; 2] {
    let ends = tapers.at(road);
    let lengths = tapers::fit(polyline_length(path), ends.map(|end| end.map(|t| t.length)));
    [0, 1].map(|end| {
        let taper = ends[end]?;
        Some(WedgeEnd::new(
            drawn[road],
            drawn[taper.seam],
            taper,
            end == 1,
            lengths[end]?,
            side,
        ))
    })
}
