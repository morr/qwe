//! Вклад куска улиц в главный проход ленты (блок «Ленты» в
//! `RoadCommon::new`): куски их слоёв, краска, колея, подход колец, улицы
//! больших стоянок. Кусок — непрерывный диапазон порядка заливки; улица от
//! улицы внутри него не зависит — входы только чтение, записи идут в свои
//! сборщики вклада. Раскладка в `RoadCommon::new` дописывает вклады
//! ([`MeshBuilder::append`], [`Painter::append_from`]) в порядке кусков, так
//! что порядок кусков каждого слоя тот же, что у серийного прохода.
//!
//! Куски разрезаются на мостах: их сборщики (`Bridges`) общие, и ветка моста
//! остаётся серийной на своём месте в порядке заливки — мосту достаётся
//! кусок из одной улицы, чей вклад несёт только краску и колею. Кольцо
//! заливается первой не-мостовой дугой в порядке заливки (R15) — решение
//! считает [`fill_arcs`] заранее, и вклад дуги кладёт кусок кольца в свой
//! слой заливки без общего состояния. Состояние раскладки полос тротуарного
//! сборщика — тоже серийное: клинья тротуара читают то, что стоит в
//! сборщике ([`MeshBuilder::set_lanes`]), а ставит его последняя мощёная
//! дорожка до этой улицы — [`sidewalk_states`] считает то же течение
//! состояний заранее, и улица вклада ставит его себе перед своими клиньями.

use std::ops::Range;

use bevy::prelude::*;

use super::drawn::Drawn;
use super::islands::push_street_fill;
use super::junctions::Junctions;
use super::lots::{Grounds, Street};
use super::paint::{MergeRamp, Station};
use super::ribbons::push_sidewalk_to_seams;
use super::rings::ApproachBand;
use super::ruts::LaneRuts;
use super::{ALLEY_COLOR, ROAD_COLOR, SIDEWALK_COLOR, UNPAVED_ROAD_COLOR, RoadStyle};
use super::{bridges, footbridge, road_lanes, tapers};
use crate::map::meshing::{Break, LaneFrame, MeshBuilder};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine, TrafficSide};
use crate::map::roads::paint::Painter;

/// Улиц в куске параллельного прохода ленты: накладные расходы куска —
/// свои сборщики и дописывание их в слои раскладкой — амортизируются длиной,
/// а работа улиц внутри куска делится по потокам.
const STREETS_PER_CHUNK: usize = 96;

/// Вклад куска прохода ленты. Куски слоя — свои сборщики на класс заливки
/// (у мощёной дорожки заливка идёт в её полосу тротуара — тот же сборщик,
/// что у серийного прохода); кольцо — в слой заливки своей первой
/// не-мостовой дуги, как у серийного прохода.
pub(super) struct ChunkRibbon {
    pub(super) streets: MeshBuilder,
    pub(super) sidewalks: MeshBuilder,
    pub(super) alleys: MeshBuilder,
    pub(super) unpaved: MeshBuilder,
    pub(super) painter: Option<Painter>,
    pub(super) ruts: Vec<LaneRuts>,
    pub(super) approach: Vec<ApproachBand>,
    pub(super) grounds: Vec<(usize, Street)>,
}

/// Общие чтения прохода ленты: всё, что улица читает, — построено до
/// прохода и не меняется.
pub(super) struct RibbonLane<'a, 'm> {
    pub(super) prepared: &'a Drawn<'m>,
    pub(super) drawn: &'a [&'m RoadLine],
    pub(super) order: &'a [usize],
    pub(super) ribbon: &'a [std::borrow::Cow<'m, [Vec2]>],
    pub(super) ramps: &'a [Option<MergeRamp>],
    pub(super) stations: &'a [Station],
    pub(super) junctions: &'a Junctions,
    pub(super) seam_gaps: &'a [Vec<Break>],
    pub(super) kerb: &'a super::corners::KerbReturns,
    pub(super) style: RoadStyle,
    pub(super) traffic_side: TrafficSide,
    pub(super) at_ring: &'a [bool],
    pub(super) outlines: &'a [bool],
    pub(super) grounds: &'a Grounds<'m>,
}

/// Позиция первой не-мостовой дуги каждого кольца в порядке заливки: её
/// заливка ставит кольцо (`ring_filled` серийного прохода), и до неё дуги
/// кладут колею по его замкнутой оси.
pub(super) fn fill_arcs(
    prepared: &Drawn<'_>,
    drawn: &[&RoadLine],
    order: &[usize],
    outlines: &[bool],
) -> Vec<Option<usize>> {
    let mut first: Vec<Option<usize>> = vec![None; prepared.rings().list.len()];
    for (position, &index) in order.iter().enumerate() {
        if outlines[index] || drawn[index].bridge {
            continue;
        }
        if let Some(ring) = prepared.rings().index_of(index) {
            if first[ring].is_none() {
                first[ring] = Some(position);
            }
        }
    }
    first
}

/// Разрез порядка заливки на куски [`STREETS_PER_CHUNK`], разрезанный на
/// мостах: мосту — кусок из одной улицы, чьё тело раскладка строит серийно.
pub(super) fn chunks(drawn: &[&RoadLine], order: &[usize]) -> Vec<Range<usize>> {
    let mut chunks: Vec<Range<usize>> = Vec::new();
    let mut start = 0;
    for (position, &index) in order.iter().enumerate() {
        if drawn[index].bridge {
            if start < position {
                chunks.push(start..position);
            }
            chunks.push(position..position + 1);
            start = position + 1;
        } else if position - start >= STREETS_PER_CHUNK {
            chunks.push(start..position);
            start = position;
        }
    }
    if start < order.len() {
        chunks.push(start..order.len());
    }
    chunks
}

/// Состояние тротуарного сборщика ([`MeshBuilder::set_lanes`]) на момент
/// полосы тротуара каждой улицы порядка заливки: клинья тротуара читают
/// раскладку, стоящую в сборщике, а ставит её последняя мощёная дорожка до
/// этой улицы — у серийного прохода состояние течёт между улицами. Кусок
/// вклада берёт состояние на своём начале и дальше течёт им сам, и
/// координаты геометрии сходятся с серийным проходом.
pub(super) fn sidewalk_states(
    drawn: &[&RoadLine],
    order: &[usize],
    outlines: &[bool],
) -> Vec<Option<LaneFrame>> {
    let mut lanes = None;
    order
        .iter()
        .map(|&index| {
            let state = lanes;
            let road = drawn[index];
            if !road.bridge
                && !outlines[index]
                && road.class == RoadClass::Alley
                && road.is_paved_path()
            {
                lanes = road_lanes(road);
            }
            state
        })
        .collect()
}

impl<'a, 'm> RibbonLane<'a, 'm> {
    /// Вклад куска `range` порядка заливки; `sidewalk_lanes` — состояние
    /// тротуарного сборщика на его начале ([`sidewalk_states`]), `fill_arc` —
    /// первые дуги колец ([`fill_arcs`]).
    pub(super) fn chunk(
        &self,
        range: Range<usize>,
        mut sidewalk_lanes: Option<LaneFrame>,
        fill_arc: &[Option<usize>],
    ) -> ChunkRibbon {
        let mut part = ChunkRibbon {
            streets: MeshBuilder::with_surface_coords(),
            sidewalks: MeshBuilder::with_surface_coords(),
            alleys: MeshBuilder::with_surface_coords(),
            unpaved: MeshBuilder::with_surface_coords(),
            painter: None,
            ruts: Vec::new(),
            approach: Vec::new(),
            grounds: Vec::new(),
        };
        for position in range {
            let index = self.order[position];
            let road = self.drawn[index];
            // замкнутая линия площади (`highway=*` + `area=yes`) — контур
            // заливки выше, а не кольцо ленты: вклада у неё нет
            if self.outlines[index] {
                continue;
            }
            // полоса тротуара читает раскладку, стоящую в сборщике к этой
            // улице; мощёная дорожка позже поставит свою
            let band_lanes = sidewalk_lanes;
            let ring_index = self.prepared.rings().index_of(index);
            let paved_path = road.is_paved_path();
            let unpaved_street = road.is_unpaved_street();
            let color = match road.class {
                RoadClass::Street if unpaved_street => UNPAVED_ROAD_COLOR,
                RoadClass::Street => ROAD_COLOR,
                RoadClass::Alley if paved_path => SIDEWALK_COLOR,
                RoadClass::Alley => ALLEY_COLOR,
            };
            let points: &[Vec2] = &self.ribbon[index];
            // колея гаснет по разрывам асфальта; у ведущей узла их там нет
            let asphalt = self.junctions.asphalt();
            let breaks = asphalt.of(index);
            let lanes = road_lanes(road);
            // колея полос ляжет по этой оси и этой раскладке — её и запомнить
            // для оверлея (`roads/ruts.rs`); у кольца — по его замкнутой оси,
            // пока кольцо не залито первой не-мостовой дугой
            if let Some(frame) = lanes {
                let pushed = match ring_index {
                    None => true,
                    Some(ring) => fill_arc[ring].is_none_or(|first| position <= first),
                };
                if pushed {
                    part.ruts.push(LaneRuts {
                        axis: match ring_index {
                            Some(ring) => self.prepared.rings().list[ring].path.clone(),
                            None => points.to_vec(),
                        },
                        frame,
                    });
                }
            }
            // линии краски — по той же оси, разрывам и клиньям, что и асфальт
            if self.style.markings {
                let wedges = super::paint::wedge_ends(
                    points,
                    self.prepared.tapers(),
                    self.drawn,
                    index,
                    self.traffic_side,
                );
                // у половины слияния на шве — тихий разрыв в узле (S3)
                let base = self.junctions.paint().of(index);
                let (cut, quiet): (Vec<Break>, Vec<Break>);
                let line_breaks = if self.seam_gaps[index].is_empty() {
                    base
                } else {
                    cut = [base.cut, &self.seam_gaps[index]].concat();
                    quiet = [base.quiet, &self.seam_gaps[index]].concat();
                    super::paint::LineBreaks {
                        cut: &cut,
                        quiet: &quiet,
                        ..base
                    }
                };
                let painter =
                    part.painter.get_or_insert_with(|| Painter::new(self.traffic_side));
                painter.paint(
                    road,
                    points,
                    line_breaks,
                    wedges,
                    self.junctions.node_paint().pockets().of(index),
                    self.ramps[index],
                    self.stations[index],
                );
            }
            // мосты — раскладке: их сборщики (`Bridges`) общие, и фартуки
            // моста идут в слой улиц на его месте в порядке заливки; краска и
            // колея моста уже во вкладе
            if road.bridge {
                continue;
            }
            // клинья у швов со сменой сечения: торцы, срезанные под них, и сами
            // клинья от ширины узкого соседа (`roads/tapers.rs`)
            let ends = self.prepared.taper_ends(index);
            let [head, body, tail] = if ends == [None; 2] {
                [None, None, None]
            } else {
                tapers::split(points, ends.map(|end| end.map(|taper| taper.length)))
            };
            let body: &[Vec2] = body.as_deref().unwrap_or(points);
            // торец под клин и торец плеча, кончающегося в узле, — прямые: узел
            // закрывают скругления и наружные углы (`roads/corners.rs`); торец
            // со стежком уже не в узле
            let butt = self.kerb.butt(index);
            let stitched_end = self.prepared.stitched_end(index);
            let trimmed = [
                head.is_some() || (butt[0] && !stitched_end[0]),
                tail.is_some() || (butt[1] && !stitched_end[1]),
            ];
            // торец на шве земля/мост (`bridges::seam_ends`) — прямой, как бортик
            // настила: у настила — всё (R14), у подхода — полоса тротуара и
            // обочина, чей полудиск светился по сторонам настила (R32)
            let seam = bridges::seam_ends(self.drawn, self.prepared.nodes(), index);
            let banded = [trimmed[0] || seam[0], trimmed[1] || seam[1]];
            let wedges = tapers::wedge_run(&head, &tail, ends);

            // тротуар кольца — одной лентой на всё кольцо (`push_ring_edges`):
            // с тем состоянием раскладки, какое у серийного прохода стояло в
            // сборщике к этой улице
            part.sidewalks.set_lanes(band_lanes);
            if let Some(sidewalk) =
                self.prepared.sidewalk_drawn(index).filter(|_| ring_index.is_none())
            {
                // у половины разделённой улицы тротуара со стороны пары нет;
                // тело клиновой половины начинается за головным клином — куски
                // пары сдвигаются на его длину. На самом клине тротуар как был
                // (ниже): там его кроет газон или его бордюр
                let (sides, total) = (road.sidewalk().sides(), polyline_length(body));
                let head_length = head.as_deref().map_or(0.0, polyline_length);
                let stitch = self.prepared.stitch_offset(index) - head_length;
                let pieces = self.prepared.pairs().band_pieces(index, sides, stitch, total);
                // на шве земля/мост полоса сходит клином в бортик настила; торец
                // под клином сечения — не шов тела
                let curb_reach = bridges::seam_curb_reach(self.drawn, self.prepared.nodes(), index);
                let seam_reach = [
                    curb_reach[0].filter(|_| head.is_none()),
                    curb_reach[1].filter(|_| tail.is_none()),
                ];
                push_sidewalk_to_seams(
                    &mut part.sidewalks,
                    body,
                    [road.width, sidewalk],
                    pieces.as_deref(),
                    SIDEWALK_COLOR.to_linear(),
                    banded,
                    seam_reach,
                );
                if self.at_ring[index] {
                    part.approach.push(ApproachBand {
                        axis: body.to_vec(),
                        width: road.width + 2.0 * sidewalk,
                        butt: banded,
                    });
                }
                // клин тротуара — по сторонам, как у асфальта (`roads/tapers.rs`)
                tapers::push_sidewalks(
                    &mut part.sidewalks,
                    &wedges,
                    self.drawn,
                    self.prepared,
                    index,
                    points,
                    road,
                    body,
                    sidewalk,
                    SIDEWALK_COLOR.to_linear(),
                );
            }
            // слой заливки берётся после полосы тротуара: мощёная дорожка
            // ложится в тот же слой, а полоса выше брала его сама
            let fill: &mut MeshBuilder = match road.class {
                RoadClass::Street if unpaved_street => &mut part.unpaved,
                RoadClass::Street => &mut part.streets,
                RoadClass::Alley if paved_path => &mut part.sidewalks,
                RoadClass::Alley => &mut part.alleys,
            };
            // Кромка клина половины разделённой улицы со стороны пары прямая
            // сама: разводка пары ставит ось клина по его суженной полуширине
            // (`Pairs::align`), и сужается одна внешняя кромка.
            // у узла слияния колея плывёт за линиями краски — по той же рампе;
            // профиль — по длине `points`, а тело начинается за клином у начала
            match (lanes, self.ramps[index]) {
                (Some(frame), Some(ramp)) => {
                    let head = head.as_deref().map_or(0.0, polyline_length);
                    let profile = ramp
                        .lane_profile(frame, polyline_length(points))
                        .into_iter()
                        .map(|(along, frame)| (along - head, frame))
                        .collect::<Vec<(f32, LaneFrame)>>();
                    fill.set_lane_profile(lanes, profile);
                }
                _ => fill.set_lanes(lanes),
            }
            // асфальт, упёршийся в грунтовку, — до её кромки (`corners.rs`)
            let setback = self.kerb.setback(index);
            let set_back: Option<Vec<Vec2>> =
                (setback != [0.0; 2] && head.is_none() && tail.is_none())
                    .then(|| tapers::cut(body, setback[0], polyline_length(body) - setback[1]))
                    .filter(|cut| cut.len() >= 2);
            let fill_body = set_back.as_deref().unwrap_or(body);
            match ring_index {
                // первая не-мостовая дуга кольца — его замкнутая заливка по
                // оси кольца (R15), одна на кольцо
                Some(ring) if fill_arc[ring] == Some(position) => {
                    let ring = &self.prepared.rings().list[ring];
                    let breaks: Vec<Break> = ring
                        .roads
                        .iter()
                        .flat_map(|&arc| asphalt.of(arc).iter().copied())
                        .collect();
                    push_street_fill(
                        fill,
                        &ring.path,
                        ring.width,
                        color.to_linear(),
                        &breaks,
                        [false; 2],
                    );
                }
                // дуги после первой: асфальт кольца уже залит
                Some(_) => {}
                None => push_street_fill(
                    fill,
                    fill_body,
                    road.width,
                    color.to_linear(),
                    breaks,
                    trimmed,
                ),
            }
            fill.set_lanes(lanes);
            tapers::push_asphalt(
                fill,
                &wedges,
                self.drawn,
                road,
                body,
                breaks,
                lanes,
                color.to_linear(),
                self.traffic_side,
            );
            // дорожка на мостик расширяется в него клином (S5)
            footbridge::lay_flares(fill, self.drawn, self.prepared, road, points, index, color);
            if road.class == RoadClass::Street && !road.passage {
                part.grounds.extend(self.grounds.streets_for(road, points));
            }
            // мощёная дорожка ставила колею в тротуарный сборщик — её унаследует
            // полоса тротуара следующей улицы
            if road.class == RoadClass::Alley && paved_path {
                sidewalk_lanes = lanes;
            }
        }
        part
    }
}
