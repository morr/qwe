//! Сборка слоя краски: [`Painter`] кладёт линии полос, осевые, зебры,
//! стоп-линии, стрелки, островки колец и колею траекторий в свои меши.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};

use super::frames::{
    BIRTH_EDGE, BIRTH_SLACK, BIRTH_WIDTH, WedgeEnd, births, lane_frame, wedge_frame,
};
use super::spans::{approach_spans, hook_breaks, insert_at, near_spans, split_at_spans};
use super::{
    APPROACH, ARROW_BRANCH, ARROW_BRANCH_REACH, ARROW_HEAD, ARROW_HEAD_WIDTH, ARROW_LENGTH,
    ARROW_MARK_REACH, ARROW_REPEAT, ARROW_REPEAT_CLEAR, ARROW_SETBACK, ARROW_STEM, AXIS_STRIP,
    ArrowMarks, BIRTH_FADE, DOUBLE_AXIS_LANES, EDGE_WIDTH, HOOK_OVERLAP, LANE_STRIP, LINE_SLACK,
    LineBreaks, LineKind, MergeRamp, PAINT_COLOR, PaintPass, RAMP_STEP, RUT_OFFSET, RUT_SIGMA,
    Station, ZEBRA_FILL, ZEBRA_PERIOD, axis_offset,
};
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::{
    Break, LaneFrame, MeshBuilder, PaintStation, break_distances, break_profile, miter_offsets,
};
use crate::map::osm::{RoadLine, TrafficSide};
use crate::map::roads::node_paint::{Pocket, StopLine, ZEBRA_LENGTH, Zebra};
use crate::map::roads::shape::lane_width;
use crate::map::roads::turns::{JunctionWear, LaneArrow};
use crate::map::roads::{is_carriageway, lane_count};
use crate::map::shapes::{Shape, is_ring, ring_of};
use crate::map::surface::{LayerMesh, MaterialSpec};
use crate::settings::{
    Z_BRIDGE_PAINT, Z_ROAD_ISLANDS, Z_ROAD_PAINT, Z_ROAD_WEAR, Z_ROAD_WEAR_MASK,
};

/// Острейший угол, рад, до которого вершина внутренней кромки обводки
/// островка ещё ставится острой (`LineJoin::Miter` у `i_overlay`).
const EDGE_MITER: f32 = 0.05;

/// Обводка островка и то, что внутри неё, — **площадями** (R27): обводка —
/// кольцо между контуром и контуром, ужатым на [`EDGE_WIDTH`], штриховка —
/// ужатый контур. Лентой вдоль контура обводка была прежде, и контур клина
/// приходит из размыкания со скруглёнными углами — дуги радиусом 0.3 м по
/// вершине на 9 см: полоса в 0.6 м полуширины перехлёстывала на них сама
/// себя — наплывы шире линии на каждой вершине, шов двойной полупрозрачной
/// краски, хвостик за угол, — а у острой вершины линия не сходилась в точку
/// (усик назад). Ещё штриховка лежала на всём контуре, под обводкой, и
/// краска там ложилась дважды. Площадь кромки ни с чем не перекрывается, и
/// у острой вершины две линии обводки сливаются в клин и сходятся в точку.
pub(super) fn island_band(shape: &Shape) -> (Vec<Shape>, Vec<Shape>) {
    let inside: Vec<Shape> =
        shape.outline(&OutlineStyle::new(-EDGE_WIDTH).line_join(LineJoin::Miter(EDGE_MITER)));
    let band = if inside.is_empty() {
        vec![shape.clone()]
    } else {
        vec![shape.clone()].overlay(&inside, OverlayRule::Difference, FillRule::NonZero)
    };
    (band, inside)
}

/// «До разрыва» у поперечной краски: разрывов у неё нет, шейдер её не гасит.
const NO_BREAK: f32 = 1.0e4;
/// Полуширина полосы под стоп-линию и под зебру, м: краска плюс сглаживание
/// на дальнем зуме, где она ещё видна.
const STOP_STRIP: f32 = 0.6;
const ZEBRA_STRIP: f32 = ZEBRA_LENGTH / 2.0 + 0.6;

/// Полуширина полосы под колею траектории, м: обе колеи с краями в 3σ.
const WEAR_STRIP: f32 = RUT_OFFSET + 3.0 * RUT_SIGMA;

/// Меши слоя краски: линии полос и осевые, по улицам и по мостам отдельно,
/// и зебры. Стоп-линии и стрелки — в меше линий полос: и видны они до того
/// же зума. Под всеми — колея траекторий узла: одни и те же
/// полосы двумя мешами, маской и наложением ([`PaintPass`]).
pub struct Painter {
    pub(super) wear_mask: MeshBuilder,
    pub(super) wear: MeshBuilder,
    pub(super) lanes: MeshBuilder,
    pub(super) axes: MeshBuilder,
    pub(super) zebras: MeshBuilder,
    pub(super) islands: MeshBuilder,
    pub(super) bridge_lanes: MeshBuilder,
    pub(super) bridge_axes: MeshBuilder,
    pub lines: usize,
    /// Сторона движения карты: по ней линия полос знает, куда едут её
    /// полосы, и сплошную кладёт только на подходе к узлу.
    side: TrafficSide,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new(TrafficSide::default())
    }
}

impl Painter {
    /// Пустые меши краски карты со стороной движения `side`.
    pub fn new(side: TrafficSide) -> Self {
        Self {
            side,
            wear_mask: MeshBuilder::with_surface_coords(),
            wear: MeshBuilder::with_surface_coords(),
            lanes: MeshBuilder::with_surface_coords(),
            axes: MeshBuilder::with_surface_coords(),
            zebras: MeshBuilder::with_surface_coords(),
            islands: MeshBuilder::with_surface_coords(),
            bridge_lanes: MeshBuilder::with_surface_coords(),
            bridge_axes: MeshBuilder::with_surface_coords(),
            lines: 0,
        }
    }
}

/// Имена слоёв краски — по ним [`PaintTag`] находит свой меш, а BRP — сущность.
pub const PAINT_WEAR_MASK: &str = "road_paint_wear_mask";
pub const PAINT_WEAR: &str = "road_paint_wear";
pub const PAINT_LANES: &str = "road_paint_lanes";
pub const PAINT_AXES: &str = "road_paint_axes";
pub const PAINT_ZEBRAS: &str = "road_paint_zebras";
pub const PAINT_ISLANDS: &str = "road_paint_islands";
pub const BRIDGE_PAINT_LANES: &str = "bridge_paint_lanes";
pub const BRIDGE_PAINT_AXES: &str = "bridge_paint_axes";

impl Painter {
    /// Линии проезжей части `road`, нарисованной по `points` (ось улицы со
    /// стежками), с разрывами краски и узлами насквозь `breaks`
    /// (`roads/node_paint.rs`), клиньями `wedges`, карманами у торцов
    /// `pockets`, раскладкой у узла слияния `ramp` и началом длины улицы
    /// `station`.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        road: &RoadLine,
        points: &[Vec2],
        LineBreaks {
            cut: breaks,
            solid: through,
            throats,
            quiet,
        }: LineBreaks,
        wedges: [Option<WedgeEnd>; 2],
        pockets: [Option<Pocket>; 2],
        ramp: Option<MergeRamp>,
        station: Station,
    ) {
        // `lane_markings=no` — ни осевой, ни границ полос; краску узла
        // (зебры, стоп-линии) это не трогает. По грунту линий не бывает
        if !is_carriageway(road) || !road.lane_markings || road.is_unpaved_street() {
            return;
        }
        let lanes = lane_count(road);
        if lanes < 2 {
            return;
        }
        let closed = is_ring(points);
        // путь режется по всем разрывам сразу, с карманами: вершины на
        // изломах «до разрыва» нужны и линиям кармана, и прочим
        let mut all = breaks.to_vec();
        all.extend(pockets.iter().flatten().map(|pocket| pocket.gap));
        all.extend(throats.iter().map(|throat| throat.gap));
        let (mut path, mut along, mut to_break) = break_profile(points, closed, &all, 0.5);
        if path.len() < 2 {
            return;
        }
        // крючки на концах линий у разрывов узлов — разрыв до излома; тихий
        // разрыв продлевается тихим (сплошной подхода у угла нет и так)
        let (hooked, hooked_quiet): (Vec<Break>, Vec<Break>);
        let (breaks, quiet) = {
            let hooks = if closed {
                Vec::new()
            } else {
                hook_breaks(&path, &along, &break_distances(&path, false, breaks))
            };
            if hooks.is_empty() {
                (breaks, quiet)
            } else {
                // разрывы минус тихие: крючок, найденный и без тихих, —
                // громкий; крючок, который даёт только тихий разрыв, — тихий
                let unquiet: Vec<Break> = breaks
                    .iter()
                    .filter(|found| !quiet.contains(found))
                    .copied()
                    .collect();
                let unquiet_hooks =
                    hook_breaks(&path, &along, &break_distances(&path, false, &unquiet));
                hooked_quiet = quiet
                    .iter()
                    .copied()
                    .chain(hooks.iter().copied().filter(|hook| {
                        !unquiet_hooks
                            .iter()
                            .any(|found| found.at.distance(hook.at) < HOOK_OVERLAP)
                    }))
                    .collect();
                hooked = [breaks, &hooks].concat();
                all.extend(&hooks);
                (path, along, to_break) = break_profile(points, closed, &all, 0.5);
                (&hooked[..], &hooked_quiet[..])
            }
        };
        let total = along[along.len() - 1];
        // вершины на концах клиньев: раскладка ломается там, и GPU должен
        // интерполировать её по прямой с обеих сторон
        if !closed {
            if let Some(head) = wedges[0] {
                insert_at(&mut path, &mut along, &mut to_break, head.length);
            }
            if let Some(tail) = wedges[1] {
                insert_at(&mut path, &mut along, &mut to_break, total - tail.length);
            }
            // и по клину слияния — часто: раскладка там по кривой
            if let Some(ramp) = ramp {
                let steps = (ramp.length / RAMP_STEP).ceil() as usize;
                for step in 0..=steps {
                    let at = ramp.along(ramp.length * step as f32 / steps.max(1) as f32);
                    insert_at(&mut path, &mut along, &mut to_break, at);
                }
            }
        }
        let body = lane_frame(lanes);
        // Рождение линии на клине (R25): вершина там, где новая полоса
        // набрала [`BIRTH_WIDTH`], и вторая вплотную перед ней со стороны
        // шва — линия встаёт сразу полной, без растяжки прозрачности по звену
        if !closed {
            for (index, wedge) in wedges.iter().enumerate() {
                let Some(wedge) = *wedge else {
                    continue;
                };
                let end = index == 1;
                for at in births(body, wedge_frame(body, wedge, end), wedge.length) {
                    let (at, before) = if end {
                        (total - at, total - at + BIRTH_EDGE)
                    } else {
                        (at, at - BIRTH_EDGE)
                    };
                    if ramp.is_some_and(|ramp| ramp.frame_at(body, at).is_some()) {
                        continue;
                    }
                    insert_at(&mut path, &mut along, &mut to_break, at);
                    insert_at(&mut path, &mut along, &mut to_break, before);
                }
            }
        }
        // клин слияния — своя раскладка, и линии в ней проявляются, как прежде
        let ramped: Vec<bool> = along
            .iter()
            .map(|&at| ramp.is_some_and(|ramp| ramp.frame_at(body, at).is_some()))
            .collect();
        let frames: Vec<LaneFrame> = along
            .iter()
            .map(|&at| {
                // клин слияния ведёт раскладку сам: торец в узле не шов
                if let Some(frame) = ramp.and_then(|ramp| ramp.frame_at(body, at)) {
                    return frame;
                }
                match wedges {
                    [Some(head), _] if !closed && at < head.length => {
                        wedge_frame(body, head, false).lerp(body, at / head.length)
                    }
                    [_, Some(tail)] if !closed && at > total - tail.length => {
                        wedge_frame(body, tail, true).lerp(body, (total - at) / tail.length)
                    }
                    _ => body,
                }
            })
            .collect();
        let Station { start, reversed } = station;
        let street_along: Vec<f32> = along
            .iter()
            .map(|&at| if reversed { start - at } else { start + at })
            .collect();
        let miters = miter_offsets(&path, closed, 1.0);
        // «до узла насквозь» — узел в вершине пути (дорога его проходит), и
        // между вершинами расстояние линейно
        let to_through = if through.is_empty() {
            vec![f32::INFINITY; path.len()]
        } else {
            break_distances(&path, closed, through)
        };
        // «до разрыва» по набору разрывов линии: разрыв кармана у торца —
        // только у линий, которым за узлом нет места (вне раскладки узкого
        // продолжения). Ключ — маска торцов, где линия в кармане
        let in_pocket = |offset: f32| -> usize {
            (0..2)
                .filter(|&end| {
                    pockets[end].is_some_and(|pocket| {
                        let narrow = lane_frame(pocket.lanes);
                        offset < narrow.low + LINE_SLACK || offset > narrow.high - LINE_SLACK
                    })
                })
                .map(|end| 1 << end)
                .sum()
        };
        // и горла съездов: бит 2 — горла слева по ходу пути, бит 3 — справа;
        // линия в горле — строго по его сторону оси
        let throat_bit = |side: f32| if side > 0.0 { 1 << 2 } else { 1 << 3 };
        let in_throat = |offset: f32| -> usize {
            throats
                .iter()
                .filter(|throat| offset * throat.side > LINE_SLACK)
                .map(|throat| throat_bit(throat.side))
                .fold(0, |mask, bit| mask | bit)
        };
        let every = (0..2)
            .filter(|&end| pockets[end].is_some())
            .map(|end| 1 << end)
            .sum::<usize>()
            | throats
                .iter()
                .map(|throat| throat_bit(throat.side))
                .fold(0, |mask, bit| mask | bit);
        let mut profiles: [Option<Vec<f32>>; 16] = Default::default();
        profiles[every] = Some(to_break);
        // `calm` — без тихих разрывов (`quiet`): по нему считается сплошная
        // подхода; без тихих он совпадает с полным
        let mut calm_profiles: [Option<Vec<f32>>; 16] = Default::default();
        let profile = |mask: usize, calm: bool| {
            let mut chosen: Vec<Break> = breaks
                .iter()
                .filter(|found| !calm || !quiet.contains(found))
                .copied()
                .collect();
            chosen.extend(
                (0..2)
                    .filter(|&end| mask & (1 << end) != 0)
                    .filter_map(|end| pockets[end].map(|pocket| pocket.gap)),
            );
            chosen.extend(
                throats
                    .iter()
                    .filter(|throat| mask & throat_bit(throat.side) != 0)
                    .map(|throat| throat.gap),
            );
            break_distances(&path, closed, &chosen)
        };

        // осевая — граница потоков тела, на сетке его раскладки
        let axis_at = axis_offset(road, lanes, self.side);
        let lowest = frames
            .iter()
            .map(|frame| (frame.low - frame.origin) / lane_width())
            .fold(f32::INFINITY, f32::min);
        let highest = frames
            .iter()
            .map(|frame| (frame.high - frame.origin) / lane_width())
            .fold(f32::NEG_INFINITY, f32::max);
        for k in lowest.floor() as i32..=highest.ceil() as i32 {
            let step = k as f32 * lane_width();
            let axis = axis_at.is_some_and(|at| (body.origin + step - at).abs() < 1e-3);
            let kind = match (axis, lanes >= DOUBLE_AXIS_LANES) {
                (true, true) => LineKind::Double,
                (true, false) => LineKind::Axis,
                (false, _) => LineKind::Lane,
            };
            // На клине расширения осевая — как у узкой части (R25): двойная
            // сплошная встаёт с конца клина, где полос уже столько, сколько
            // ей нужно. Куски клина с узкой частью под [`DOUBLE_AXIS_LANES`]
            let single: Vec<(f32, f32)> = if kind == LineKind::Double && !closed {
                [(wedges[0], false), (wedges[1], true)]
                    .into_iter()
                    .filter_map(|(wedge, end)| {
                        let wedge = wedge.filter(|wedge| wedge.lanes < DOUBLE_AXIS_LANES)?;
                        Some(if end {
                            (total - wedge.length, total)
                        } else {
                            (0.0, wedge.length)
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let kind = if single.is_empty() {
                kind
            } else {
                LineKind::Axis
            };
            let offsets: Vec<f32> = frames.iter().map(|frame| frame.origin + step).collect();
            let alphas: Vec<f32> = frames
                .iter()
                .zip(&offsets)
                .zip(&ramped)
                .map(|((frame, &at), &ramped)| {
                    let room = (at - frame.low).min(frame.high - at);
                    if ramped {
                        (room / (BIRTH_FADE * lane_width())).clamp(0.0, 1.0)
                    } else if room >= BIRTH_WIDTH * lane_width() - BIRTH_SLACK {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect();
            let line: Vec<Vec2> = path
                .iter()
                .zip(&miters)
                .zip(&offsets)
                .map(|((&point, &miter), &offset)| point + miter * offset)
                .collect();
            let mask = in_pocket(body.origin + step) | in_throat(body.origin + step);
            // сплошная подхода — только к разрывам узлов и карманов: горло
            // съезда линию кольца рвёт, но кольцо пунктирное и перед ним
            let approach_mask = mask & 0b11;
            for key in [mask, approach_mask] {
                if profiles[key].is_none() {
                    profiles[key] = Some(profile(key, false));
                }
                if !quiet.is_empty() && calm_profiles[key].is_none() {
                    calm_profiles[key] = Some(profile(key, true));
                }
            }
            let calm = if quiet.is_empty() {
                &profiles
            } else {
                &calm_profiles
            };
            let (Some(to_break), Some(calm_break), Some(approach_break)) =
                (&profiles[mask], &calm[mask], &calm[approach_mask])
            else {
                unreachable!("профили посчитаны выше");
            };
            let stations: Vec<PaintStation> = (0..path.len())
                .map(|index| PaintStation {
                    along: street_along[index],
                    to_break: to_break[index],
                    alpha: alphas[index],
                })
                .collect();
            let (builder, half) = match (kind, road.bridge) {
                (LineKind::Lane, false) => (&mut self.lanes, LANE_STRIP),
                (LineKind::Lane, true) => (&mut self.bridge_lanes, LANE_STRIP),
                (_, false) => (&mut self.axes, AXIS_STRIP),
                (_, true) => (&mut self.bridge_axes, AXIS_STRIP),
            };
            let color = PAINT_COLOR.to_linear();
            if closed {
                if alphas.iter().all(|&alpha| alpha > 0.0) {
                    builder.push_paint_strip(&line, true, half, &stations, kind.code(), color);
                    self.lines += 1;
                }
                continue;
            }
            // линия полос: сплошная — только на подходе к узлу по ходу её
            // полос, на выезде из узла пунктир сразу
            let (line, stations, solid) = match kind {
                LineKind::Lane => {
                    let forward =
                        flows_forward(road, body.origin + step, axis_at.unwrap_or(0.0), self.side);
                    let spans = approach_spans(&along, approach_break, forward);
                    split_at_spans(line, stations, &along, &spans)
                }
                // осевая обслуживает оба потока: сплошная по обе стороны
                // разрыва — и у узла, который улица проходит насквозь
                LineKind::Axis => {
                    let mut spans = near_spans(&along, calm_break, APPROACH);
                    spans.extend(near_spans(&along, &to_through, APPROACH));
                    split_at_spans(line, stations, &along, &spans)
                }
                _ => {
                    let solid = vec![false; line.len().saturating_sub(1)];
                    (line, stations, solid)
                }
            };
            // двойная — звенья вне клиньев с одиночной осевой (`single`); по
            // длине улицы на станциях — обратно в метры пути
            let doubled: Vec<bool> = stations
                .windows(2)
                .map(|pair| {
                    let middle = (pair[0].along + pair[1].along) / 2.0;
                    let at = if reversed {
                        start - middle
                    } else {
                        middle - start
                    };
                    !single.is_empty() && !single.iter().any(|&(from, to)| from <= at && at <= to)
                })
                .collect();
            let code = |solid: bool, doubled: bool| match kind {
                _ if doubled => LineKind::Double.code(),
                LineKind::Lane if solid => LineKind::Solid.code(),
                LineKind::Lane => LineKind::Dashed.code(),
                LineKind::Axis if solid => LineKind::AxisSolid.code(),
                LineKind::Axis => LineKind::AxisDashed.code(),
                _ => kind.code(),
            };
            // куски, где линия видна хоть на одном конце звена, одного вида
            let key = |index: usize| {
                (index + 1 < line.len()
                    && stations[index].alpha.max(stations[index + 1].alpha) > 0.0)
                    .then(|| (solid[index], doubled[index]))
            };
            let mut from: Option<usize> = None;
            for index in 0..line.len() {
                let here = key(index);
                if let Some(first) = from
                    && here != key(first)
                {
                    builder.push_paint_strip(
                        &line[first..=index],
                        false,
                        half,
                        &stations[first..=index],
                        code(solid[first], doubled[first]),
                        color,
                    );
                    self.lines += 1;
                    from = None;
                }
                if from.is_none() && here.is_some() {
                    from = Some(index);
                }
            }
        }
    }

    /// Двойная сплошная по середине асфальтовой разделительной парных половин
    /// (`roads/medians.rs`), с разрывами перекрёстков обеих половин `breaks`.
    /// У каждой половины своей осевой нет — она односторонняя.
    pub fn paint_median(&mut self, midline: &[Vec2], breaks: &[Break], on_deck: bool) {
        let (path, along, to_break) = break_profile(midline, false, breaks, 0.5);
        if path.len() < 2 {
            return;
        }
        let stations: Vec<PaintStation> = along
            .iter()
            .zip(&to_break)
            .map(|(&along, &to_break)| PaintStation {
                along,
                to_break,
                alpha: 1.0,
            })
            .collect();
        // разделительная двух настилов лежит в их слое — и краска над ним
        let builder = if on_deck {
            &mut self.bridge_axes
        } else {
            &mut self.axes
        };
        builder.push_paint_strip(
            &path,
            false,
            AXIS_STRIP,
            &stations,
            LineKind::Double.code(),
            PAINT_COLOR.to_linear(),
        );
        self.lines += 1;
    }

    /// Осевая продолжения, заведённая от узла слияния между половинами
    /// (`roads/merges.rs::merge_axis`): сплошная, двойная — если двойная у
    /// продолжения, в `lanes` полос ([`DOUBLE_AXIS_LANES`]). Разрывов на ней
    /// нет: узел слияния — не перекрёсток.
    pub fn paint_merge_axis(&mut self, line: &[Vec2], lanes: u8) {
        if line.len() < 2 {
            return;
        }
        let (along, _) = arclengths(line);
        let stations: Vec<PaintStation> = along
            .iter()
            .map(|&along| PaintStation {
                along,
                to_break: NO_BREAK,
                alpha: 1.0,
            })
            .collect();
        let kind = if lanes >= DOUBLE_AXIS_LANES {
            LineKind::Double
        } else {
            LineKind::AxisSolid
        };
        self.axes.push_paint_strip(
            line,
            false,
            AXIS_STRIP,
            &stations,
            kind.code(),
            PAINT_COLOR.to_linear(),
        );
        self.lines += 1;
    }

    /// Зебра: плашка поперёк проезжей части, полосы по координате поперёк
    /// дороги рисует шейдер — вдаль они гаснут в ровный светлый тон. Первое
    /// звено начинается у самого `from`: планка из целых звеньев
    /// ([`whole_bars`]) ими и кончается.
    pub fn paint_zebra(&mut self, zebra: &Zebra) {
        let width = zebra.from.distance(zebra.to);
        self.zebras.push_paint_strip(
            &[zebra.from, zebra.to],
            false,
            ZEBRA_STRIP,
            &transverse_stations(ZEBRA_FIRST_BAR, width),
            LineKind::Zebra.code(),
            PAINT_COLOR.to_linear(),
        );
        self.lines += 1;
    }

    /// Стоп-линия поперёк полос, у «уступи дорогу» — прерывистая.
    pub fn paint_stop_line(&mut self, line: &StopLine) {
        let kind = if line.yields {
            LineKind::Yield
        } else {
            LineKind::Stop
        };
        self.lanes.push_paint_strip(
            &[line.from, line.to],
            false,
            STOP_STRIP,
            &transverse_stations(0.0, line.from.distance(line.to)),
            kind.code(),
            PAINT_COLOR.to_linear(),
        );
        self.lines += 1;
    }

    /// Колея траекторий узлов (`roads/turns.rs`) — **как тень**: колея
    /// поверх колеи светлее не делается. Полоса вдоль каждой кривой и каждого
    /// хвоста, по две колеи в [`RUT_OFFSET`] по сторонам; сила — в альфе
    /// вершины: у кривой целая, у хвоста сходит в ноль вглубь полосы, где её
    /// сменяет колея асфальта. Полосы кладутся дважды, в маску и в наложение
    /// ([`PaintPass`]): маска оставляет в каждом пикселе **наибольшую** колею
    /// из всех, наложение светлит асфальт на неё один раз, сколько бы полос
    /// там ни легло.
    ///
    /// Рисуется только по асфальту (`asphalt_pieces`): у острого угла
    /// поворота полоса доставала кромку краем и перелезала бордюрную дугу
    /// на плитку тротуара — светлая полоса вдоль кромки сужалась клином и
    /// кончалась тёмным треугольником у дальнего края ленты (Тула, Фрунзе ×
    /// Красноармейский, юго-западный угол рта).
    pub(in crate::map::roads) fn paint_turn_wear(
        &mut self,
        junctions: &[JunctionWear],
        on_asphalt: impl Fn(Vec2) -> bool,
    ) {
        for junction in junctions {
            for curve in &junction.curves {
                for piece in Self::asphalt_pieces(curve, &on_asphalt) {
                    self.push_wear(&piece, |_| 1.0);
                }
            }
            for tail in &junction.tails {
                for piece in Self::asphalt_pieces(tail, &on_asphalt) {
                    self.push_wear(&piece, |share| 1.0 - share);
                }
            }
        }
    }

    /// Полоса колеи вдоль `line` с силой `strength(доля длины)` — в оба меша.
    fn push_wear(&mut self, line: &[Vec2], strength: impl Fn(f32) -> f32) {
        let (along, total) = arclengths(line);
        let stations: Vec<PaintStation> = along
            .iter()
            .map(|&at| PaintStation {
                along: at,
                to_break: NO_BREAK,
                alpha: strength(at / total.max(1e-3)),
            })
            .collect();
        for builder in [&mut self.wear_mask, &mut self.wear] {
            builder.push_paint_strip(
                line,
                false,
                WEAR_STRIP,
                &stations,
                LineKind::Wear.code(),
                PAINT_COLOR.to_linear(),
            );
        }
    }

    /// Куски полилинии колеи, целиком лежащие на асфальте (`on_asphalt`).
    /// Звено остаётся, когда асфальт стоит под обоими его концами и под обеими
    /// колеями по сторонам (нормаль звена на [`RUT_OFFSET`]): краем полосы
    /// колея перелезала кромку поворота на плитку тротуара. Торец у кромки
    /// прямой — как у стоп-линии, обрезанной тем же асфальтом.
    pub(super) fn asphalt_pieces(
        line: &[Vec2],
        on_asphalt: &impl Fn(Vec2) -> bool,
    ) -> Vec<Vec<Vec2>> {
        let mut pieces = Vec::new();
        let mut current: Vec<Vec2> = Vec::new();
        for window in line.windows(2) {
            let (from, to) = (window[0], window[1]);
            let ruts = (to - from).normalize_or_zero().perp() * RUT_OFFSET;
            let inside = [Vec2::ZERO, ruts, -ruts]
                .into_iter()
                .all(|offset| on_asphalt(from + offset) && on_asphalt(to + offset));
            if inside {
                if current.is_empty() {
                    current.push(from);
                }
                current.push(to);
            } else if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            pieces.push(current);
        }
        pieces
    }

    /// Островок у кольца (`roads/gores.rs`): обводка каждого контура и
    /// штриховка заливкой — полосы идут поперёк `across`. Обводка и
    /// штриховка не перекрываются ([`island_band`]).
    pub(in crate::map::roads) fn paint_island(&mut self, shape: &Shape, across: Vec2) {
        let color = PAINT_COLOR.to_linear();
        if shape.is_empty() {
            return;
        }
        let (band, inside) = island_band(shape);
        // обводка — площадью: координата поперёк у неё ноль, и шейдер
        // линии кроет её целиком
        let passes = [
            (&band, Vec2::ZERO, LineKind::Edge),
            (&inside, across, LineKind::Hatch),
        ];
        for (pieces, across, kind) in passes {
            for piece in pieces {
                let mut rings = piece.iter().map(ring_of);
                let Some(outer) = rings.next() else {
                    continue;
                };
                let holes: Vec<Vec<Vec2>> = rings.collect();
                self.islands
                    .push_paint_area(&outer, &holes, across, NO_BREAK, kind.code(), color);
            }
        }
        self.lines += 1;
    }

    /// Где встанет кончик стрелки: на [`ARROW_SETBACK`] за самой дальней от
    /// узла поперечной краской на её полосе — зеброй или стоп-линией, — а без
    /// них за [`ARROW_SETBACK`] от кромки узла. Зебра отступает от кромки по
    /// месту перехода, и отступ от одной кромки клал стрелку на зебру.
    pub(in crate::map::roads) fn arrow_setback(arrow: &LaneArrow, marks: &ArrowMarks) -> f32 {
        let forward = arrow.travel.normalize_or_zero();
        let reach = Vec2::splat(ARROW_MARK_REACH);
        let mut farthest: f32 = 0.0;
        for index in marks.grid.near(arrow.at - reach, arrow.at + reach) {
            let mark = marks.marks[index];
            let Some(behind) = mark.crossing_along(arrow.at, forward) else {
                continue;
            };
            if (-ARROW_LENGTH..=ARROW_MARK_REACH).contains(&behind) {
                farthest = farthest.max(behind + mark.half);
            }
        }
        farthest + ARROW_SETBACK
    }

    /// Второй ряд стрелок, как у Яндекса и по ГОСТ 1.18: кончик в
    /// [`ARROW_REPEAT`] за первым, если ось полосы за ним есть ещё на
    /// [`ARROW_REPEAT_CLEAR`], между рядами нет поперечной краски и ряд не
    /// в разрыве `breaks` своей дороги — иначе он лёг бы на соседний узел или
    /// его переход.
    pub(in crate::map::roads) fn repeat_setback(
        arrow: &LaneArrow,
        setback: f32,
        marks: &ArrowMarks,
        breaks: &[Break],
    ) -> Option<f32> {
        let repeat = setback + ARROW_REPEAT;
        let (_, total) = arclengths(&arrow.back);
        if total < repeat + ARROW_LENGTH + ARROW_REPEAT_CLEAR {
            return None;
        }
        let forward = arrow.travel.normalize_or_zero();
        let occupied = repeat - ARROW_SETBACK..=repeat + ARROW_LENGTH + ARROW_SETBACK;
        let in_break = breaks
            .iter()
            .filter(|found| found.reach > 0.0)
            .any(|found| {
                let offset = arrow.at - found.at;
                let behind = offset.dot(forward);
                offset.perp_dot(forward).abs() < found.reach + lane_width()
                    && behind + found.reach >= *occupied.start()
                    && behind - found.reach <= *occupied.end()
            });
        if in_break {
            return None;
        }
        let span = setback + ARROW_LENGTH..=repeat + ARROW_LENGTH + ARROW_SETBACK;
        let reach = Vec2::splat(*span.end());
        let blocked = marks
            .grid
            .near(arrow.at - reach, arrow.at + reach)
            .into_iter()
            .any(|index| {
                marks.marks[index]
                    .crossing_along(arrow.at, forward)
                    .is_some_and(|behind| span.contains(&behind))
            });
        (!blocked).then_some(repeat)
    }

    /// Стрелка на полосе подхода: стебель вдоль хода, наконечник, если прямо
    /// можно, и отвод с наконечником в каждую разрешённую сторону. Кончик —
    /// в `setback` от кромки узла по оси полосы ([`Self::arrow_setback`]),
    /// чтобы стрелка не легла на стоп-линию и переход.
    pub(in crate::map::roads) fn paint_arrow(&mut self, arrow: &LaneArrow, setback: f32) {
        // кончик и хвост — на оси полосы, по её длине от кромки: на изогнутом
        // подходе прямая от кромки уводила стрелку с полосы. Короткая ось (way
        // кончается раньше) — по прямой, как раньше
        let (along, total) = arclengths(&arrow.back);
        let on_back = |distance: f32| {
            (0.0..=total)
                .contains(&distance)
                .then(|| place_on_path(&arrow.back, &along, distance))
                .flatten()
                .map(|(point, _)| point)
        };
        let (tip, tail) = match (on_back(setback), on_back(setback + ARROW_LENGTH)) {
            (Some(tip), Some(tail)) => (tip, tail),
            _ => {
                let forward = arrow.travel.normalize_or_zero();
                (
                    arrow.at - forward * setback,
                    arrow.at - forward * (setback + ARROW_LENGTH),
                )
            }
        };
        let forward = (tip - tail).normalize_or_zero();
        if forward == Vec2::ZERO {
            return;
        }
        let left = forward.perp();
        // точка стрелки в её раме: `x` — вдоль хода от хвоста, `y` — влево
        let to_world = |x: f32, y: f32| tail + forward * x + left * y;
        let turn = arrow.turn;
        let branch_at = ARROW_LENGTH * ARROW_BRANCH;
        let stem_end = if turn.through {
            ARROW_LENGTH - ARROW_HEAD
        } else {
            branch_at
        };
        let mut shapes: Vec<Vec<Vec2>> = Vec::new();
        let bar = |from: Vec2, to: Vec2| -> Vec<Vec2> {
            let side = (to - from).normalize_or_zero().perp() * ARROW_STEM / 2.0;
            vec![from - side, to - side, to + side, from + side]
        };
        let head = |base: Vec2, along: Vec2| -> Vec<Vec2> {
            let side = along.perp() * ARROW_HEAD_WIDTH / 2.0;
            vec![base - side, base + along * ARROW_HEAD, base + side]
        };
        shapes.push(bar(to_world(0.0, 0.0), to_world(stem_end, 0.0)));
        if turn.through {
            shapes.push(head(to_world(stem_end, 0.0), forward));
        }
        for (allowed, sign) in [(turn.left, 1.0), (turn.right, -1.0)] {
            if !allowed {
                continue;
            }
            let start = to_world(branch_at, 0.0);
            let reach = ARROW_BRANCH_REACH;
            let end = to_world(branch_at + reach, sign * reach);
            let along = (end - start).normalize_or_zero();
            shapes.push(bar(start, end));
            shapes.push(head(end, along));
        }
        let color = PAINT_COLOR.to_linear();
        for shape in &shapes {
            self.lanes.push_paint_area(
                shape,
                &[],
                forward,
                NO_BREAK,
                LineKind::Arrow.code(),
                color,
            );
        }
        self.lines += 1;
    }

    /// Восемь слоёв краски: маска и наложение колеи узлов, краска улиц над
    /// асфальтом улиц, островки колец над асфальтом стоянок, мосты над
    /// настилом.
    pub fn layers(self) -> [LayerMesh; 8] {
        [
            (
                self.wear_mask,
                Z_ROAD_WEAR_MASK,
                PAINT_WEAR_MASK,
                PaintPass::WearMask,
            ),
            (self.wear, Z_ROAD_WEAR, PAINT_WEAR, PaintPass::Wear),
            (self.zebras, Z_ROAD_PAINT, PAINT_ZEBRAS, PaintPass::Lines),
            (
                self.islands,
                Z_ROAD_ISLANDS,
                PAINT_ISLANDS,
                PaintPass::Lines,
            ),
            (self.lanes, Z_ROAD_PAINT, PAINT_LANES, PaintPass::Lines),
            (self.axes, Z_ROAD_PAINT, PAINT_AXES, PaintPass::Lines),
            (
                self.bridge_lanes,
                Z_BRIDGE_PAINT,
                BRIDGE_PAINT_LANES,
                PaintPass::Lines,
            ),
            (
                self.bridge_axes,
                Z_BRIDGE_PAINT,
                BRIDGE_PAINT_AXES,
                PaintPass::Lines,
            ),
        ]
        .map(|(builder, z, name, pass)| LayerMesh::new(builder, z, name, MaterialSpec::Paint(pass)))
    }
}

/// Станции поперечной краски длиной `width` от длины `start`: длина идёт
/// поперёк дороги, от одной кромки к другой, разрывов нет.
fn transverse_stations(start: f32, width: f32) -> [PaintStation; 2] {
    [start, start + width].map(|along| PaintStation {
        along,
        to_break: NO_BREAK,
        alpha: 1.0,
    })
}

/// Звено зебры поперёк дороги, м: полоса краски периода [`ZEBRA_PERIOD`].
const ZEBRA_BAR: f32 = ZEBRA_PERIOD * ZEBRA_FILL;
/// Длина по планке, с которой начинается первое звено: шейдер кладёт звенья
/// серединой на `k · период + период / 2`.
pub(super) const ZEBRA_FIRST_BAR: f32 = (ZEBRA_PERIOD - ZEBRA_BAR) / 2.0;

/// Едут ли полосы у линии со сдвигом `offset` от оси (плюс — влево по ходу
/// точек) по ходу точек дороги. Односторонняя — вся по ходу (`oneway=-1`
/// развёрнут парсом); у двусторонней по ходу — полосы своей стороны от
/// осевой `axis` ([`axis_offset`]).
fn flows_forward(road: &RoadLine, offset: f32, axis: f32, side: TrafficSide) -> bool {
    road.oneway
        || match side {
            TrafficSide::Right => offset < axis,
            TrafficSide::Left => offset > axis,
        }
}
