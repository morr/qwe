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
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use super::network::RoadNetwork;
use super::network::sections::STREET_LANE_WIDTH;
use super::tapers::{self, Tapers};
use super::{is_carriageway, lane_count};
use crate::map::meshing::{
    ATTRIBUTE_RIBBON, Break, LaneFrame, MeshBuilder, PaintStation, break_profile, miter_offsets,
};
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;
use crate::map::shapes::is_ring;
use crate::map::surface::{LayerMesh, MaterialSpec};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::settings::{Z_BRIDGE_PAINT, Z_ROAD_PAINT};

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
}

impl Default for RoadPaintStyle {
    fn default() -> Self {
        Self {
            paint: 0.85,
            wear: 0.075,
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
}

impl LineKind {
    fn code(self) -> f32 {
        match self {
            Self::Lane => 0.0,
            Self::Axis => 1.0,
            Self::Double => 2.0,
        }
    }
}

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

/// Меши слоя краски: линии полос и осевые, по улицам и по мостам отдельно.
pub struct Painter {
    lanes: MeshBuilder,
    axes: MeshBuilder,
    bridge_lanes: MeshBuilder,
    bridge_axes: MeshBuilder,
    pub lines: usize,
}

impl Default for Painter {
    fn default() -> Self {
        Self {
            lanes: MeshBuilder::with_surface_coords(),
            axes: MeshBuilder::with_surface_coords(),
            bridge_lanes: MeshBuilder::with_surface_coords(),
            bridge_axes: MeshBuilder::with_surface_coords(),
            lines: 0,
        }
    }
}

/// Имена слоёв краски — по ним [`PaintTag`] находит свой меш, а BRP — сущность.
pub const PAINT_LANES: &str = "road_paint_lanes";
pub const PAINT_AXES: &str = "road_paint_axes";
pub const BRIDGE_PAINT_LANES: &str = "bridge_paint_lanes";
pub const BRIDGE_PAINT_AXES: &str = "bridge_paint_axes";

/// Какие линии несёт меш краски — по этому [`show_paint`] прячет его с зумом.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintTag {
    Lanes,
    Axes,
}

impl PaintTag {
    /// Вид меша по имени слоя; `None` — слой не краска.
    pub fn of(name: &str) -> Option<Self> {
        match name {
            PAINT_LANES | BRIDGE_PAINT_LANES => Some(Self::Lanes),
            PAINT_AXES | BRIDGE_PAINT_AXES => Some(Self::Axes),
            _ => None,
        }
    }
}

impl Painter {
    /// Линии проезжей части `road`, нарисованной по `points` (ось улицы со
    /// стежками), с разрывами перекрёстков `breaks`, клиньями `wedges` и
    /// началом длины улицы `station`.
    pub fn paint(
        &mut self,
        road: &RoadLine,
        points: &[Vec2],
        breaks: &[Break],
        wedges: [Option<WedgeEnd>; 2],
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
        let (mut path, mut along, to_break) = break_profile(points, closed, breaks, 0.5);
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

    /// Четыре слоя краски: улицы над асфальтом улиц, мосты над настилом.
    pub fn layers(self) -> [LayerMesh; 4] {
        [
            (self.lanes, Z_ROAD_PAINT, PAINT_LANES),
            (self.axes, Z_ROAD_PAINT, PAINT_AXES),
            (self.bridge_lanes, Z_BRIDGE_PAINT, BRIDGE_PAINT_LANES),
            (self.bridge_axes, Z_BRIDGE_PAINT, BRIDGE_PAINT_AXES),
        ]
        .map(|(builder, z, name)| LayerMesh::new(builder, z, name, MaterialSpec::Paint))
    }
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

/// Ступени слоя краски: до [`LANE_ZOOM_MAX`] — всё, до [`AXIS_ZOOM_MAX`] —
/// осевые, дальше — ничего.
pub enum PaintLods {}

impl ZoomLods for PaintLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [LANE_ZOOM_MAX, AXIS_ZOOM_MAX, f32::INFINITY].into_iter()
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
            PaintTag::Axes => bucket.index < 2,
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
        }
    }
}

/// Материал слоя краски: вершинный цвет с альфой рождения линии × линия,
/// которую шейдер рисует по координатам полосы. Один на приложение, хэндл —
/// в `surface::SurfaceMaterials` рядом с материалами поверхностей.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct PaintMaterial {
    #[uniform(0)]
    pub params: PaintParams,
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
    /// `ATTRIBUTE_RIBBON`.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_RIBBON.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

#[cfg(test)]
mod tests;
