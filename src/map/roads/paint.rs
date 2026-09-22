//! **Слой краски** — линии полос проезжей части геометрией, отсчитанной от
//! оси улицы (`roads/axis.rs`), а не рисунком шейдера асфальта.
//!
//! Пока линии рисовал шейдер поверхностей, они считались из ширины ленты:
//! `round((поперёк + полуширина) / ширина полосы)`, полосы — поровну от центра
//! ленты. На клине (`roads/tapers.rs`) ширина плывёт, и вместе с ней плыли все
//! линии разом; на шве двух ways фаза штрихов начиналась заново. Теперь:
//!
//! - **раскладка полос одна** ([`LaneFrame`]): узел сетки полос и границы
//!   проезжей части поперёк оси. Границы полос — `origin + k ·`
//!   [`STREET_LANE_WIDTH`]. По этой же раскладке кладёт колею шейдер асфальта
//!   (`surface.wgsl`, атрибут `meshing::ATTRIBUTE_RIBBON`), так что колея идёт
//!   ровно между линиями и на клине;
//! - **на клине крайняя полоса рождается**, остальные линии идут без сдвига:
//!   раскладка плывёт от узкого сечения к широкому, границы расходятся вместе
//!   с кромками, а линия, которой у узкого сечения не было, проявляется из
//!   кромки. Смена чётности числа полос (две → три) сдвигает сетку на пол
//!   полосы — это плавный уход линий на длине клина, новая полоса справа по
//!   ходу клина;
//! - **штрихи — по длине улицы**, а не way: фаза не рвётся на шве;
//! - **у узла линия сплошная** за [`APPROACH`] до разрыва перекрёстка; осевая
//!   двусторонней улицы в четыре полосы и больше — двойная сплошная, у́же —
//!   пунктир, как линии полос. У нечётной двусторонней и у односторонней
//!   осевой нет.
//!
//! Линию с полом в 1.3 px, её штрихи, сглаженный край и гашение по зуму рисует
//! шейдер краски (`assets/shaders/paint.wgsl`); геометрия — полоса шире линии
//! под каждую ([`MeshBuilder::push_paint_strip`]). Полосы двух видов — линии
//! полос и осевые — лежат разными мешами: дальше [`LANE_ZOOM_MAX`] линии
//! полос не видны, дальше [`AXIS_ZOOM_MAX`] — и осевые, и их меш прячется
//! ([`PaintLods`], [`show_paint`]) без пересборки.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use super::network::RoadNetwork;
use super::network::sections::STREET_LANE_WIDTH;
use super::node_paint::{Pocket, STOP_WIDTH, StopLine, ZEBRA_LENGTH, Zebra};
use super::tapers::{self, Tapers};
use super::turns::JunctionWear;
use super::{is_carriageway, lane_count};
use crate::map::along::arclengths;
use crate::map::meshing::{
    ATTRIBUTE_RIBBON, Break, LaneFrame, MeshBuilder, PaintStation, break_distances, break_profile,
    miter_offsets,
};
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;
use crate::map::shapes::is_ring;
use crate::map::shapes::{Shape, ring_of};
use crate::map::surface::{LayerMesh, MaterialSpec};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::settings::{
    Z_BRIDGE_PAINT, Z_ROAD_ISLANDS, Z_ROAD_PAINT, Z_ROAD_WEAR, Z_ROAD_WEAR_MASK,
};

const SHADER_PATH: &str = "shaders/paint.wgsl";

/// Ширина линии, м — как у настоящей (10–15 см). На экране не тоньше 1.3 px:
/// тоньше мерцает при сдвиге камеры, и шейдер её расширяет.
const LINE_WIDTH: f32 = 0.15;
/// Штрих и пропуск прерывистой линии, м.
const DASH: f32 = 3.0;
const GAP: f32 = 3.0;
/// Сколько метров до разрыва перекрёстка линия идёт сплошной — перед узлом
/// перестраиваться нельзя.
const APPROACH: f32 = 25.0;
/// Центр каждой из двух линий двойной сплошной от оси, м: зазор между ними
/// полметра.
const DOUBLE_OFFSET: f32 = (0.5 + LINE_WIDTH) / 2.0;
/// Цвет краски — белый с лёгкой желтизной старой разметки; прозрачность —
/// ручка «Paint» ([`RoadPaintStyle::paint`]), не цвет.
const PAINT_COLOR: Color = Color::srgb(0.95, 0.95, 0.93);

/// Дальше этого зума (м на пиксель) линий полос нет: полоса у́же восьми
/// пикселей, и её линии сливаются в серую рябь. Шейдер гасит их к порогу,
/// дальше их меш спрятан ([`PaintLods`]).
pub const LANE_ZOOM_MAX: f32 = 0.4;
/// Дальше этого зума нет и осевых — порог совпадает с машинами и заборами:
/// уличная мелочь уходит вместе, а не по очереди.
pub const AXIS_ZOOM_MAX: f32 = 0.9;
/// Доля порога, с которой шейдер начинает гасить линию: к самому порогу она
/// уже прозрачна, и спрятанный меш не щёлкает.
const FADE_FROM: f32 = 0.8;

/// Полуширина полосы под линию полос и под осевую, м: линия в 1.3 px плюс
/// сглаживание на самом дальнем зуме, где она ещё видна.
const LANE_STRIP: f32 = 0.6;
const AXIS_STRIP: f32 = 1.4;

/// На каком удалении от кромки проезжей части линия, рождающаяся из клина,
/// набирает полную видимость, м: у самой кромки линии нет — новой полосы ещё
/// нет.
const BIRTH_FADE: f32 = STREET_LANE_WIDTH / 2.0;

/// Свежесть краски, 0–1: прозрачность линий. Ручка «Paint» секции Roads.
pub const PAINT_MIN: f32 = 0.0;
pub const PAINT_MAX: f32 = 1.0;
pub const PAINT_STEP: f32 = 0.05;
/// Сила колеи на асфальте — амплитуда светлой полосы под колёсами. Ручка
/// «Wear» секции Roads; до ручки было 7.5 % зашитой константой шейдера.
pub const WEAR_MIN: f32 = 0.0;
pub const WEAR_MAX: f32 = 0.15;
pub const WEAR_STEP: f32 = 0.005;
/// Сила колеи поворотов в узле (`roads/turns.rs`) — ручка «Turn wear». Меньше
/// колеи полос: кривые узла ложатся одна на другую и складываются.
pub const TURN_WEAR_MIN: f32 = 0.0;
pub const TURN_WEAR_MAX: f32 = 0.08;
pub const TURN_WEAR_STEP: f32 = 0.005;

/// Краска и износ дорог: одни юниформы, ничего не пересобирается. Отдельно от
/// `RoadStyle`, правка которого пересобирает все дорожные слои, — та же
/// причина, по которой `TramStyle` и `CarStyle` не поля `RoadStyle`.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "road_paint")]
pub struct RoadPaintStyle {
    /// Непрозрачность линий: свежая краска — 1, стёртая — к нулю.
    pub paint: f32,
    /// Амплитуда колеи (`surface.wgsl`).
    pub wear: f32,
    /// Амплитуда колеи траекторий узла — слой износа (`paint.wgsl`).
    pub turn_wear: f32,
}

impl Default for RoadPaintStyle {
    fn default() -> Self {
        Self {
            paint: 0.85,
            wear: 0.075,
            turn_wear: 0.035,
        }
    }
}

impl RoadPaintStyle {
    /// Значения, зажатые шкалой: старый файл настроек или запись по BRP могут
    /// принести что угодно.
    pub fn paint(self) -> f32 {
        self.paint.clamp(PAINT_MIN, PAINT_MAX)
    }

    pub fn wear(self) -> f32 {
        self.wear.clamp(WEAR_MIN, WEAR_MAX)
    }

    pub fn turn_wear(self) -> f32 {
        self.turn_wear.clamp(TURN_WEAR_MIN, TURN_WEAR_MAX)
    }
}

/// Вид линии — код в `ATTRIBUTE_RIBBON.w` слоя краски.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    /// Граница полос одного направления: прерывистая, у узла сплошная.
    Lane,
    /// Осевая двусторонней улицы в две полосы: как линия полос, но видна
    /// дальше.
    Axis,
    /// Осевая двусторонней улицы от четырёх полос — двойная сплошная.
    Double,
    /// Стоп-линия поперёк полос (`roads/node_paint.rs`).
    Stop,
    /// Она же прерывистой — «уступи дорогу».
    Yield,
    /// Зебра: плашка поперёк проезжей части, полосы рисует шейдер.
    Zebra,
    /// Колея траектории узла (`roads/turns.rs`): не краска, а светлый износ
    /// асфальта в своих мешах, две колеи по сторонам кривой.
    Wear,
    /// Штриховка островка у кольца (`roads/gores.rs`): заливка его контура,
    /// косые полосы рисует шейдер по координате поперёк них.
    Hatch,
    /// Обводка островка — сплошная линия.
    Edge,
}

impl LineKind {
    fn code(self) -> f32 {
        match self {
            Self::Lane => 0.0,
            Self::Axis => 1.0,
            Self::Double => 2.0,
            Self::Stop => 3.0,
            Self::Yield => 4.0,
            Self::Zebra => 5.0,
            Self::Wear => 6.0,
            Self::Hatch => 7.0,
            Self::Edge => 8.0,
        }
    }
}

/// Островок у кольца: шаг косых полос, их ширина и ширина обводки, м.
const HATCH_PERIOD: f32 = 1.6;
const HATCH_WIDTH: f32 = 0.35;
const EDGE_WIDTH: f32 = 0.2;
/// Полуширина полосы под обводку островка, м.
pub(super) const EDGE_STRIP: f32 = 0.6;

/// «До разрыва» у поперечной краски: разрывов у неё нет, шейдер её не гасит.
const NO_BREAK: f32 = 1.0e4;
/// Полуширина полосы под стоп-линию и под зебру, м: краска плюс сглаживание
/// на дальнем зуме, где она ещё видна.
const STOP_STRIP: f32 = 0.6;
const ZEBRA_STRIP: f32 = ZEBRA_LENGTH / 2.0 + 0.6;
/// Период полос зебры поперёк дороги и доля полосы в нём.
const ZEBRA_PERIOD: f32 = 1.0;
const ZEBRA_FILL: f32 = 0.5;
/// Штрих и пропуск прерывистой стоп-линии, м.
const YIELD_DASH: f32 = 0.6;
const YIELD_GAP: f32 = 0.6;
/// Дальше этого зума зебры нет: до него полосы гаснут в ровную плашку
/// (`visible()` по периоду), дальше и плашка — мелочь.
pub const ZEBRA_ZOOM_MAX: f32 = 0.6;
/// Колея траектории: смещение колеса от середины полосы и ширина колеи (σ),
/// м, — как у колеи полос в `surface.wgsl`.
const RUT_OFFSET: f32 = 0.85;
const RUT_SIGMA: f32 = 0.32;
/// Полуширина полосы под колею траектории, м: обе колеи с краями в 3σ.
const WEAR_STRIP: f32 = RUT_OFFSET + 3.0 * RUT_SIGMA;

/// Раскладка полос тела way с `lanes` полосами: границы проезжей части на
/// `± lanes · шаг / 2`, узел сетки — на оси при чётном числе полос и в
/// полполосы от неё при нечётном. Раскладка симметрична: у зеркального way
/// сетка та же.
pub fn lane_frame(lanes: u8) -> LaneFrame {
    let half = f32::from(lanes) * STREET_LANE_WIDTH / 2.0;
    LaneFrame {
        origin: if lanes.is_multiple_of(2) {
            0.0
        } else {
            STREET_LANE_WIDTH / 2.0
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
fn narrow_frame(body: LaneFrame, narrow_lanes: u8, end: bool) -> LaneFrame {
    let narrow = lane_frame(narrow_lanes);
    let shift = (body.origin - narrow.origin).rem_euclid(STREET_LANE_WIDTH);
    LaneFrame {
        origin: if end {
            body.origin + shift
        } else {
            body.origin - shift
        },
        ..narrow
    }
}

/// Раскладка, отражённая поперёк пути: так её видит путь, идущий навстречу.
fn mirrored(frame: LaneFrame) -> LaneFrame {
    LaneFrame {
        origin: -frame.origin,
        low: -frame.high,
        high: -frame.low,
    }
}

/// Раскладки клина `[у шва, у тела]` — в раме **пути клина**, который идёт от
/// шва к телу (`tapers::split`): у торца конца он смотрит против way, и
/// раскладка отражена. Их кладёт в асфальт клина `MeshBuilder::set_lane_taper`.
pub fn wedge_frames(body_lanes: u8, narrow_lanes: u8, end: bool) -> [LaneFrame; 2] {
    let body = lane_frame(body_lanes);
    let narrow = narrow_frame(body, narrow_lanes, end);
    if end {
        [mirrored(narrow), mirrored(body)]
    } else {
        [narrow, body]
    }
}

/// Длина улицы у первой точки каждого way и идёт ли way навстречу улице (тогда
/// вдоль way она убывает). Штрихи идут по длине улицы, поэтому фаза на шве не
/// рвётся.
pub fn street_stations(network: &RoadNetwork, paths: &[impl AsRef<[Vec2]>]) -> Vec<(f32, bool)> {
    let mut stations = vec![(0.0, false); paths.len()];
    if !network.covers(paths.len()) {
        return stations;
    }
    for street in &network.streets {
        let mut run = 0.0;
        for way in &street.ways {
            let length = polyline_length(paths[way.road].as_ref());
            stations[way.road] = (if way.reversed { run + length } else { run }, way.reversed);
            run += length;
        }
    }
    stations
}

/// Что лежит у торца way: клин длиной `length` от сечения соседа в `lanes`
/// полос.
#[derive(Clone, Copy, Debug)]
pub struct WedgeEnd {
    pub length: f32,
    pub lanes: u8,
}

/// Клинья у торцов way так, как их нарежет `tapers::split` по этому пути.
pub fn wedge_ends(
    path: &[Vec2],
    tapers: &Tapers,
    drawn: &[&RoadLine],
    road: usize,
) -> [Option<WedgeEnd>; 2] {
    let ends = tapers.at(road);
    let lengths = tapers::fit(polyline_length(path), ends.map(|end| end.map(|t| t.length)));
    [0, 1].map(|side| {
        Some(WedgeEnd {
            length: lengths[side]?,
            lanes: lane_count(drawn[ends[side]?.narrow]),
        })
    })
}

/// Меши слоя краски: линии полос и осевые, по улицам и по мостам отдельно,
/// и зебры. Стоп-линии и направляющий пунктир — в меше линий полос: и видны
/// они до того же зума. Под всеми — колея траекторий узла: одни и те же
/// полосы двумя мешами, маской и наложением ([`PaintPass`]).
pub struct Painter {
    wear_mask: MeshBuilder,
    wear: MeshBuilder,
    lanes: MeshBuilder,
    axes: MeshBuilder,
    zebras: MeshBuilder,
    islands: MeshBuilder,
    bridge_lanes: MeshBuilder,
    bridge_axes: MeshBuilder,
    pub lines: usize,
}

impl Default for Painter {
    fn default() -> Self {
        Self {
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

/// Какие линии несёт меш краски — по этому [`show_paint`] прячет его с зумом.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintTag {
    Lanes,
    Zebras,
    Axes,
    Wear,
}

impl PaintTag {
    /// Вид меша по имени слоя; `None` — слой не краска.
    pub fn of(name: &str) -> Option<Self> {
        match name {
            PAINT_LANES | BRIDGE_PAINT_LANES => Some(Self::Lanes),
            PAINT_ZEBRAS | PAINT_ISLANDS => Some(Self::Zebras),
            PAINT_AXES | BRIDGE_PAINT_AXES => Some(Self::Axes),
            PAINT_WEAR_MASK | PAINT_WEAR => Some(Self::Wear),
            _ => None,
        }
    }
}

impl Painter {
    /// Линии проезжей части `road`, нарисованной по `points` (ось улицы со
    /// стежками), с разрывами краски `breaks` (`roads/node_paint.rs`),
    /// клиньями `wedges`, карманами у торцов `pockets` и началом длины улицы
    /// `station`.
    pub fn paint(
        &mut self,
        road: &RoadLine,
        points: &[Vec2],
        breaks: &[Break],
        (wedges, pockets): ([Option<WedgeEnd>; 2], [Option<Pocket>; 2]),
        station: (f32, bool),
    ) {
        if !is_carriageway(road) {
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
        let (mut path, mut along, to_break) = break_profile(points, closed, &all, 0.5);
        if path.len() < 2 {
            return;
        }
        let mut to_break = to_break;
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
        }
        let body = lane_frame(lanes);
        let frames: Vec<LaneFrame> = along
            .iter()
            .map(|&at| match wedges {
                [Some(head), _] if !closed && at < head.length => {
                    narrow_frame(body, head.lanes, false).lerp(body, at / head.length)
                }
                [_, Some(tail)] if !closed && at > total - tail.length => {
                    narrow_frame(body, tail.lanes, true).lerp(body, (total - at) / tail.length)
                }
                _ => body,
            })
            .collect();
        let (start, reversed) = station;
        let street_along: Vec<f32> = along
            .iter()
            .map(|&at| if reversed { start - at } else { start + at })
            .collect();
        let miters = miter_offsets(&path, closed, 1.0);
        // «до разрыва» по набору разрывов линии: разрыв кармана у торца —
        // только у линий, которым за узлом нет места (вне раскладки узкого
        // продолжения). Ключ — маска торцов, где линия в кармане
        let in_pocket = |offset: f32| -> usize {
            (0..2)
                .filter(|&end| {
                    pockets[end].is_some_and(|pocket| {
                        let narrow = lane_frame(pocket.lanes);
                        offset < narrow.low + 0.05 || offset > narrow.high - 0.05
                    })
                })
                .map(|end| 1 << end)
                .sum()
        };
        let every = (0..2)
            .filter(|&end| pockets[end].is_some())
            .map(|end| 1 << end)
            .sum::<usize>();
        let mut profiles: [Option<Vec<f32>>; 4] = Default::default();
        profiles[every] = Some(to_break);

        let lowest = frames
            .iter()
            .map(|frame| (frame.low - frame.origin) / STREET_LANE_WIDTH)
            .fold(f32::INFINITY, f32::min);
        let highest = frames
            .iter()
            .map(|frame| (frame.high - frame.origin) / STREET_LANE_WIDTH)
            .fold(f32::NEG_INFINITY, f32::max);
        for k in lowest.floor() as i32..=highest.ceil() as i32 {
            let step = k as f32 * STREET_LANE_WIDTH;
            let axis = !road.oneway && lanes.is_multiple_of(2) && (body.origin + step).abs() < 1e-3;
            let kind = match (axis, lanes >= 4) {
                (true, true) => LineKind::Double,
                (true, false) => LineKind::Axis,
                (false, _) => LineKind::Lane,
            };
            let offsets: Vec<f32> = frames.iter().map(|frame| frame.origin + step).collect();
            let alphas: Vec<f32> = frames
                .iter()
                .zip(&offsets)
                .map(|(frame, &at)| {
                    ((at - frame.low).min(frame.high - at) / BIRTH_FADE).clamp(0.0, 1.0)
                })
                .collect();
            let line: Vec<Vec2> = path
                .iter()
                .zip(&miters)
                .zip(&offsets)
                .map(|((&point, &miter), &offset)| point + miter * offset)
                .collect();
            let mask = in_pocket(body.origin + step);
            let to_break = profiles[mask].get_or_insert_with(|| {
                let mut chosen = breaks.to_vec();
                chosen.extend(
                    (0..2)
                        .filter(|&end| mask & (1 << end) != 0)
                        .filter_map(|end| pockets[end].map(|pocket| pocket.gap)),
                );
                break_distances(&path, closed, &chosen)
            });
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
            // куски, где линия видна хоть на одном конце звена
            let mut from = None;
            for index in 0..path.len() {
                let seen = index + 1 < path.len() && alphas[index].max(alphas[index + 1]) > 0.0;
                match (seen, from) {
                    (true, None) => from = Some(index),
                    (false, Some(first)) => {
                        builder.push_paint_strip(
                            &line[first..=index],
                            false,
                            half,
                            &stations[first..=index],
                            kind.code(),
                            color,
                        );
                        self.lines += 1;
                        from = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Двойная сплошная по середине асфальтовой разделительной парных половин
    /// (`roads/medians.rs`), с разрывами перекрёстков обеих половин `breaks`.
    /// У каждой половины своей осевой нет — она односторонняя.
    pub fn paint_median(&mut self, midline: &[Vec2], breaks: &[Break]) {
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
        self.axes.push_paint_strip(
            &path,
            false,
            AXIS_STRIP,
            &stations,
            LineKind::Double.code(),
            PAINT_COLOR.to_linear(),
        );
        self.lines += 1;
    }

    /// Зебра: плашка поперёк проезжей части, полосы по координате поперёк
    /// дороги рисует шейдер — вдаль они гаснут в ровный светлый тон.
    pub fn paint_zebra(&mut self, zebra: &Zebra) {
        let width = zebra.from.distance(zebra.to);
        self.zebras.push_paint_strip(
            &[zebra.from, zebra.to],
            false,
            ZEBRA_STRIP,
            &transverse_stations(width),
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
            &transverse_stations(line.from.distance(line.to)),
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
    pub(super) fn paint_turn_wear(&mut self, junctions: &[JunctionWear]) {
        for junction in junctions {
            for curve in &junction.curves {
                self.push_wear(curve, |_| 1.0);
            }
            for tail in &junction.tails {
                self.push_wear(tail, |share| 1.0 - share);
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

    /// Островок у кольца (`roads/gores.rs`): обводка каждого контура и
    /// штриховка заливкой — полосы идут поперёк `across`.
    pub(super) fn paint_island(&mut self, shape: &Shape, across: Vec2) {
        let color = PAINT_COLOR.to_linear();
        let mut rings = shape.iter().map(ring_of);
        let Some(outer) = rings.next() else {
            return;
        };
        let holes: Vec<Vec<Vec2>> = rings.collect();
        for contour in std::iter::once(&outer).chain(&holes) {
            let mut closed = contour.clone();
            closed.push(contour[0]);
            let (along, _) = arclengths(&closed);
            let stations: Vec<PaintStation> = along[..contour.len()]
                .iter()
                .map(|&along| PaintStation {
                    along,
                    to_break: NO_BREAK,
                    alpha: 1.0,
                })
                .collect();
            self.islands.push_paint_strip(
                contour,
                true,
                EDGE_STRIP,
                &stations,
                LineKind::Edge.code(),
                color,
            );
        }
        self.islands.push_paint_area(
            &outer,
            &holes,
            across,
            [NO_BREAK, LineKind::Hatch.code()],
            color,
        );
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

/// Станции поперечной краски длиной `width`: длина идёт поперёк дороги, от
/// одной кромки к другой, разрывов нет.
fn transverse_stations(width: f32) -> [PaintStation; 2] {
    [0.0, width].map(|along| PaintStation {
        along,
        to_break: NO_BREAK,
        alpha: 1.0,
    })
}

/// Вершина на длине дуги `at` — вместе с интерполированным «до разрыва».
fn insert_at(path: &mut Vec<Vec2>, along: &mut Vec<f32>, to_break: &mut Vec<f32>, at: f32) {
    let Some(index) = along
        .windows(2)
        .position(|pair| pair[0] < at && at < pair[1])
    else {
        return;
    };
    let t = (at - along[index]) / (along[index + 1] - along[index]);
    path.insert(index + 1, path[index].lerp(path[index + 1], t));
    to_break.insert(
        index + 1,
        to_break[index] + (to_break[index + 1] - to_break[index]) * t,
    );
    along.insert(index + 1, at);
}

/// Ступени слоя краски: до [`LANE_ZOOM_MAX`] — всё, до [`ZEBRA_ZOOM_MAX`] —
/// осевые и зебры, до [`AXIS_ZOOM_MAX`] — осевые, дальше — ничего.
pub enum PaintLods {}

impl ZoomLods for PaintLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [LANE_ZOOM_MAX, ZEBRA_ZOOM_MAX, AXIS_ZOOM_MAX, f32::INFINITY].into_iter()
    }
}

pub type PaintZoomBucket = ZoomBucket<PaintLods>;

/// Меш краски виден, пока его линии на этой ступени есть. Пересборки нет: оба
/// меша строятся один раз, ступень только прячет их — так дальний план не
/// гоняет вершины линий, которых не видно.
pub fn show_paint(bucket: Res<PaintZoomBucket>, mut layers: Query<(&PaintTag, &mut Visibility)>) {
    for (tag, mut visibility) in &mut layers {
        let shown = match tag {
            PaintTag::Lanes => bucket.index < 1,
            PaintTag::Zebras => bucket.index < 2,
            // колея траекторий гаснет вместе с осевыми: к порогу шейдер её
            // уже погасил
            PaintTag::Axes | PaintTag::Wear => bucket.index < 3,
        };
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// Юниформ шейдера краски. Зеркало `PaintParams` в `paint.wgsl`: порядок
/// полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct PaintParams {
    /// Непрозрачность краски — [`RoadPaintStyle::paint`].
    pub paint: f32,
    pub width: f32,
    pub dash: f32,
    pub gap: f32,
    pub approach: f32,
    pub double_offset: f32,
    pub lane_zoom: f32,
    pub axis_zoom: f32,
    pub fade_from: f32,
    pub stop_width: f32,
    pub yield_dash: f32,
    pub yield_gap: f32,
    pub zebra_half: f32,
    pub zebra_period: f32,
    pub zebra_fill: f32,
    pub zebra_zoom: f32,
    /// Колея траекторий узла — [`RoadPaintStyle::turn_wear`].
    pub turn_wear: f32,
    pub rut_offset: f32,
    pub rut_sigma: f32,
    pub lane_width: f32,
    /// Островки у колец (`roads/gores.rs`): шаг и ширина косой полосы,
    /// ширина обводки.
    pub hatch_period: f32,
    pub hatch_width: f32,
    pub edge_width: f32,
}

impl PaintParams {
    pub fn new(style: RoadPaintStyle) -> Self {
        Self {
            paint: style.paint(),
            width: LINE_WIDTH,
            dash: DASH,
            gap: GAP,
            approach: APPROACH,
            double_offset: DOUBLE_OFFSET,
            lane_zoom: LANE_ZOOM_MAX,
            axis_zoom: AXIS_ZOOM_MAX,
            fade_from: FADE_FROM,
            stop_width: STOP_WIDTH,
            yield_dash: YIELD_DASH,
            yield_gap: YIELD_GAP,
            zebra_half: ZEBRA_LENGTH / 2.0,
            zebra_period: ZEBRA_PERIOD,
            zebra_fill: ZEBRA_FILL,
            zebra_zoom: ZEBRA_ZOOM_MAX,
            turn_wear: style.turn_wear(),
            rut_offset: RUT_OFFSET,
            rut_sigma: RUT_SIGMA,
            lane_width: STREET_LANE_WIDTH,
            hatch_period: HATCH_PERIOD,
            hatch_width: HATCH_WIDTH,
            edge_width: EDGE_WIDTH,
        }
    }
}

/// Проход материала краски. Колея траекторий узла ложится **как тень** —
/// перекрытие двух колей не светлее одной, — а обычный блендинг перекрытия
/// складывает. Поэтому она рисуется в два прохода, и кадр между ними хранит
/// её в своей альфе (асфальт под ней непрозрачен, альфа там единица):
///
/// - [`Self::WearMask`] пишет только альфу, операцией `Min`, значение
///   `1 − колея`: в пикселе остаётся **наибольшая** колея из всех полос;
/// - [`Self::Wear`] умножает цвет на `2 − альфа`, то есть на `1 + колея` —
///   ту же колею, что множитель `surface.wgsl`, — и возвращает альфу в
///   единицу. Повторное наложение там, где полосы перекрылись, видит
///   единицу и цвета не меняет.
///
/// Меш маски лежит ниже меша наложения (`Z_ROAD_WEAR_MASK` < `Z_ROAD_WEAR`),
/// и прозрачная фаза рисует их по порядку целиком.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PaintPass {
    #[default]
    Lines,
    WearMask,
    Wear,
}

/// Материал слоя краски: вершинный цвет с альфой рождения линии × линия,
/// которую шейдер рисует по координатам полосы. По одному на проход
/// ([`PaintPass`]) на приложение, хэндлы — в `surface::SurfaceMaterials`
/// рядом с материалами поверхностей.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(PaintKey)]
pub struct PaintMaterial {
    #[uniform(0)]
    pub params: PaintParams,
    pub pass: PaintPass,
}

/// Ключ конвейера материала краски — проход.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaintKey {
    pass: PaintPass,
}

impl From<&PaintMaterial> for PaintKey {
    fn from(material: &PaintMaterial) -> Self {
        Self {
            pass: material.pass,
        }
    }
}

impl Material2d for PaintMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    /// Та же раскладка вершин, что у материала поверхностей: позиция, цвет и
    /// `ATTRIBUTE_RIBBON`. Проходы колеи — свой шейдерный вариант и своё
    /// смешивание ([`PaintPass`]).
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_RIBBON.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        let (define, blend, write_mask) = match key.bind_group_data.pass {
            PaintPass::Lines => return Ok(()),
            PaintPass::WearMask => (
                "WEAR_MASK",
                BlendState {
                    color: BlendComponent::REPLACE,
                    alpha: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Min,
                    },
                },
                ColorWrites::ALPHA,
            ),
            PaintPass::Wear => (
                "WEAR_APPLY",
                BlendState {
                    // цвет × (1 + колея): src — единица, умноженная на dst,
                    // плюс dst × (1 − альфа маски)
                    color: BlendComponent {
                        src_factor: BlendFactor::Dst,
                        dst_factor: BlendFactor::OneMinusDstAlpha,
                        operation: BlendOperation::Add,
                    },
                    // альфа — снова единица
                    alpha: BlendComponent::REPLACE,
                },
                ColorWrites::ALL,
            ),
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(define.into());
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend);
                target.write_mask = write_mask;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
