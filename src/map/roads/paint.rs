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
//!   [`lane_width`]. По этой же раскладке кладёт колею шейдер асфальта
//!   (`surface.wesl`, атрибут `meshing::ATTRIBUTE_RIBBON`), так что колея идёт
//!   ровно между линиями и на клине;
//! - **на клине крайняя полоса рождается**, остальные линии идут без сдвига:
//!   раскладка плывёт от узкого сечения к широкому, границы расходятся вместе
//!   с кромками, а линия, которой у узкого сечения не было, встаёт сразу
//!   полной там, где её полоса набрала [`BIRTH_WIDTH`] ширины ([`births`]);
//!   проявление из кромки ([`BIRTH_FADE`]) осталось только у клина слияния.
//!   Смена чётности числа полос (две → три) сдвигает сетку на пол
//!   полосы — это плавный уход линий на длине клина, новая полоса справа по
//!   ходу клина. У односторонней улицы полосы прибавляются у одной кромки
//!   ([`wedge_drift`]), и общие линии уходят на длине клина к другой. Осевая
//!   на клине расширения — как у узкой части: одиночная, пока узкой части
//!   мало полос для двойной сплошной, двойная — с конца клина;
//! - **штрихи — по длине улицы**, а не way: фаза не рвётся на шве;
//! - **у узла линия сплошная** за [`APPROACH`] до разрыва перекрёстка — у
//!   линии полос только на подходе по ходу её полос, на выезде пунктир сразу
//!   ([`approach_spans`]); у осевой — по обе стороны разрыва и у узла,
//!   который улица проходит насквозь ([`near_spans`]). Осевая
//!   двусторонней улицы в четыре полосы и больше — двойная сплошная, у́же —
//!   пунктир, как линии полос. У нечётной двусторонней и у односторонней
//!   осевой нет.
//!
//! Линию с полом в 1.3 px, её штрихи, сглаженный край и гашение по зуму рисует
//! шейдер краски (`assets/shaders/paint.wesl`); геометрия — полоса шире линии
//! под каждую ([`MeshBuilder::push_paint_strip`]). Полосы двух видов — линии
//! полос и осевые — лежат разными мешами: дальше [`LANE_ZOOM_MAX`] линии
//! полос не видны, дальше [`AXIS_ZOOM_MAX`] — и осевые, и их меш прячется
//! ([`PaintLods`], [`show_paint`]) без пересборки.

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use super::network::RoadNetwork;
use super::node_paint::{STOP_WIDTH, StopLine, Throat, ZEBRA_LENGTH, Zebra};
use super::shape::lane_width;
use crate::ease::smoothstep;

use crate::map::grid::Grid;
use crate::map::meshing::{Break, LaneFrame};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadLine, TrafficSide};

mod frames;
mod ground;
mod material;
mod painter;
mod spans;

pub use self::frames::{WedgeEnd, lane_frame, wedge_ends, wedge_frames};
pub(in crate::map::roads) use self::ground::{PaintGround, trim_stop_line, whole_bars};
pub use self::material::{
    PaintLods, PaintMaterial, PaintParams, PaintPass, PaintZoomBucket, show_paint,
};
pub use self::painter::{
    BRIDGE_PAINT_AXES, BRIDGE_PAINT_LANES, PAINT_AXES, PAINT_ISLANDS, PAINT_LANES, PAINT_WEAR,
    PAINT_WEAR_MASK, PAINT_ZEBRAS, Painter,
};

/// Ширина линии, м — как у настоящей (10–15 см). На экране не тоньше 1.3 px:
/// тоньше мерцает при сдвиге камеры, и шейдер её расширяет.
const LINE_WIDTH: f32 = 0.15;
/// Штрих и пропуск прерывистой линии, м: городская 1.5 по ГОСТу — пропуск
/// втрое длиннее штриха. При 3 / 3 многополосная улица на обзорном зуме
/// читалась частоколом.
const DASH: f32 = 2.0;
const GAP: f32 = 6.0;
/// Сколько метров до разрыва перекрёстка линия идёт сплошной — перед узлом
/// перестраиваться нельзя. Его же читает ряд машин (`map::cars`): вдоль
/// сплошной подхода у узкой улицы не паркуются.
pub(crate) const APPROACH: f32 = 25.0;
/// Центр каждой из двух линий двойной сплошной от оси, м: зазор между ними
/// 15 см, как у настоящей (1.3 по ГОСТ — 10–15 см). Полметра читались двумя
/// отдельными линиями, а не одной двойной.
const DOUBLE_OFFSET: f32 = (0.15 + LINE_WIDTH) / 2.0;
/// Со скольких полос осевая двусторонней улицы — двойная сплошная; у́же — нет.
/// Одно правило и для осевой улицы, и для осевой, заведённой от узла слияния
/// ([`Painter::paint_merge_axis`]).
const DOUBLE_AXIS_LANES: u8 = 4;
/// Допуск, м, с которым линия раскладки считается у края кармана (за
/// раскладкой узкого продолжения) и по сторону горла съезда: линия на самой
/// границе — ещё не в кармане и не в горле.
const LINE_SLACK: f32 = 0.05;
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

/// На какой доле ширины полосы от кромки проезжей части линия, рождающаяся из
/// клина, набирает полную видимость: у самой кромки линии нет — новой полосы
/// ещё нет.
const BIRTH_FADE: f32 = 0.5;

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
    /// Амплитуда колеи (`surface.wesl`).
    pub wear: f32,
    /// Амплитуда колеи траекторий узла — слой износа (`paint.wesl`).
    pub turn_wear: f32,
}

impl Default for RoadPaintStyle {
    fn default() -> Self {
        Self {
            paint: 0.7,
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
    /// Стрелка на полосе подхода (`roads/turns.rs`): заливка своего контура,
    /// гаснет с линиями полос.
    Arrow,
    /// Кусок линии полос вне подхода к узлу — только пунктир. Сплошную у
    /// узла линия полос открытой улицы получает не по близости разрыва, а по
    /// ходу своих полос ([`approach_spans`]): на выезде из узла её нет.
    Dashed,
    /// Кусок линии полос на подходе к узлу по ходу движения — сплошной.
    Solid,
    /// Кусок осевой открытой улицы на перегоне — пунктир. Гаснет с осевыми.
    AxisDashed,
    /// Кусок осевой у разрыва или у узла, который улица проходит насквозь, —
    /// сплошной.
    AxisSolid,
}

/// Что узлы сказали линиям одной дороги (`roads/node_paint.rs`): где они
/// рвутся (`cut`), какие узлы дорога проходит насквозь (`solid`) и где на
/// кольце горла съездов (`throats` — рвут только линии со стороны съезда).
/// `quiet` — те из `cut`, перед которыми линия не сплошная: угол двух улиц
/// (`node_paint::is_corner`) — не перекрёсток, перестраиваться перед ним не
/// запрещено (R22).
/// Одной дороге его выдаёт `node_paint::PaintBreaks::of` — по-другому набор не
/// собирается.
#[derive(Clone, Copy, Default)]
pub struct LineBreaks<'a> {
    pub cut: &'a [Break],
    pub solid: &'a [Break],
    pub throats: &'a [Throat],
    pub quiet: &'a [Break],
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
            Self::Arrow => 9.0,
            Self::Dashed => 10.0,
            Self::Solid => 11.0,
            Self::AxisDashed => 12.0,
            Self::AxisSolid => 13.0,
        }
    }
}

/// Островок у кольца: шаг косых полос, их ширина и ширина обводки, м.
/// Обводка толще штриха — островок читается фигурой, а не пятном: при
/// 0.2 / 0.35 она тонула в штриховке той же плотности.
const HATCH_PERIOD: f32 = 1.6;
const HATCH_WIDTH: f32 = 0.25;
const EDGE_WIDTH: f32 = 0.3;

/// Стрелка на полосе: длина, доля длины до отвода поворота, наконечник
/// (длина и ширина основания), толщина стебля, вынос отвода вбок и вперёд, м;
/// кончик стоит за `ARROW_SETBACK` до самой дальней зебры или стоп-линии на
/// полосе (`Painter::arrow_setback`), что ищутся не дальше `ARROW_MARK_REACH`
/// от кромки узла. Городская стрелка по ГОСТу — пять метров.
const ARROW_LENGTH: f32 = 5.0;
const ARROW_BRANCH: f32 = 0.45;
const ARROW_HEAD: f32 = 1.3;
const ARROW_HEAD_WIDTH: f32 = 0.9;
const ARROW_STEM: f32 = 0.22;
const ARROW_BRANCH_REACH: f32 = 0.8;
const ARROW_SETBACK: f32 = 4.0;
const ARROW_MARK_REACH: f32 = 30.0;
/// Второй ряд стрелок: на сколько его кончик дальше первого и сколько оси
/// полосы должно остаться за ним, м. Ось несёт `turns::ARROW_BACK` 60 м и
/// кончается у предыдущего узла раньше, так что у короткого перегона второго
/// ряда нет.
const ARROW_REPEAT: f32 = 20.0;
const ARROW_REPEAT_CLEAR: f32 = 5.0;

/// Поперечная краска узлов — зебры и стоп-линии отрезком и полутолщиной, — с
/// сеткой для [`Painter::arrow_setback`]: перебор всех на каждую стрелку стоил
/// Туле 4 мс сборки.
pub(super) struct ArrowMarks {
    marks: Vec<Mark>,
    grid: Grid<usize>,
}

/// Отрезок поперечной краски и её полутолщина вдоль дороги, м.
#[derive(Clone, Copy, Debug)]
struct Mark {
    from: Vec2,
    to: Vec2,
    half: f32,
}

impl Mark {
    /// Где отрезок пересекает ось полосы — прямую через `at` вдоль `forward`:
    /// расстояние вдоль `forward` от `at` до пересечения. `None` — отрезок
    /// лежит по одну сторону оси или вдоль неё.
    fn crossing_along(&self, at: Vec2, forward: Vec2) -> Option<f32> {
        let [a, b] = [self.from, self.to].map(|point| forward.perp_dot(point - at));
        (a * b <= 0.0 && a != b).then(|| {
            let crossing = self.from.lerp(self.to, a / (a - b));
            (at - crossing).dot(forward)
        })
    }
}

impl ArrowMarks {
    pub(super) fn new(zebras: &[Zebra], stops: &[StopLine]) -> Self {
        let marks: Vec<Mark> = zebras
            .iter()
            .map(|zebra| Mark {
                from: zebra.from,
                to: zebra.to,
                half: ZEBRA_LENGTH / 2.0,
            })
            .chain(stops.iter().map(|line| Mark {
                from: line.from,
                to: line.to,
                half: STOP_WIDTH / 2.0,
            }))
            .collect();
        let mut grid = Grid::new(ARROW_MARK_REACH);
        for (index, mark) in marks.iter().enumerate() {
            grid.insert_segment(mark.from, mark.to, 0.0, index);
        }
        Self { marks, grid }
    }
}

/// Период полос зебры поперёк дороги и доля полосы в нём.
const ZEBRA_PERIOD: f32 = 1.0;
const ZEBRA_FILL: f32 = 0.5;
/// Штрих и пропуск прерывистой стоп-линии, м.
const YIELD_DASH: f32 = 0.6;
const YIELD_GAP: f32 = 0.6;
/// Дальше этого зума зебры нет: до него полосы гаснут в ровную плашку
/// (`visible()` по периоду), дальше и плашка — мелочь.
pub const ZEBRA_ZOOM_MAX: f32 = 0.6;
/// Профиль колеи, м: смещение колеса от середины полосы и ширина колеи (σ).
/// Один на колею полос (`surface.wesl`, через `surface::SurfaceParams`) и
/// колею траекторий (`paint.wesl`, через [`PaintParams`]).
pub(crate) const RUT_OFFSET: f32 = 0.85;
pub(crate) const RUT_SIGMA: f32 = 0.32;

/// Где way стоит на своей улице: длина улицы у его первой точки и идёт ли он
/// улице навстречу (тогда вдоль way она убывает). Штрихи идут по длине улицы,
/// поэтому фаза на шве не рвётся.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Station {
    pub start: f32,
    pub reversed: bool,
}

/// [`Station`] каждого way по его пути; way вне улицы стоит на нуле.
pub fn street_stations(network: &RoadNetwork, paths: &[impl AsRef<[Vec2]>]) -> Vec<Station> {
    let mut stations = vec![Station::default(); paths.len()];
    if !network.covers(paths.len()) {
        return stations;
    }
    for street in &network.streets {
        let mut run = 0.0;
        for way in &street.ways {
            let length = polyline_length(paths[way.road].as_ref());
            stations[way.road] = Station {
                start: if way.reversed { run + length } else { run },
                reversed: way.reversed,
            };
            run += length;
        }
    }
    stations
}

/// Раскладка way половины у узла слияния (`roads/merges.rs`): в узле —
/// `frame`, сторона продолжения, за `length` метров от узла — своё тело, между
/// ними — плавно (smoothstep, как кромка слияния). `start` — метров от узла
/// до первой точки way, `away` — растёт ли это расстояние по ходу его точек:
/// клин идёт через несколько ways улицы.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MergeRamp {
    pub frame: LaneFrame,
    pub length: f32,
    pub start: f32,
    pub away: bool,
}

impl MergeRamp {
    /// Метров от узла у точки way в `along` метрах от его начала.
    fn distance(self, along: f32) -> f32 {
        if self.away {
            self.start + along
        } else {
            self.start - along
        }
    }

    /// Обратное [`Self::distance`]: метров от начала way у точки клина в
    /// `from_node` метрах от узла.
    fn along(self, from_node: f32) -> f32 {
        if self.away {
            from_node - self.start
        } else {
            self.start - from_node
        }
    }

    /// Раскладка в `along` метрах от начала way с телом `body`; за клином —
    /// `None`. Кромка со стороны пары сжимается к оси продолжения на дальней
    /// половине клина: лишняя полоса кончается там, и её линия гаснет в
    /// стороне от осевой, а не у самой двойной сплошной.
    fn frame_at(self, body: LaneFrame, along: f32) -> Option<LaneFrame> {
        let t = self.distance(along) / self.length;
        (t < 1.0).then(|| {
            let mut frame = self.frame.lerp(body, smoothstep(t));
            let early = self.frame.lerp(body, smoothstep(2.0 * t - 1.0));
            if self.frame.high.abs() < self.frame.low.abs() {
                frame.high = early.high;
            } else {
                frame.low = early.low;
            }
            frame
        })
    }

    /// Раскладка way длиной `total` с телом `body` по его длине — профиль для
    /// колеи асфальта (`MeshBuilder::set_lane_profile`): пары через
    /// [`RAMP_STEP`] по клину, последняя — уже тело. Клин мимо way — пусто.
    ///
    /// Со стороны пары раскладка асфальта не уже, чем у тела: лента половины
    /// у узла ложится поверх соседки, и полоса ленты за сжатой кромкой
    /// каркаса была асфальтом без колеи — гладким пятном у осевой продолжения
    /// (пример 26). Сетка полос там продолжается, а в узле она и есть сетка
    /// продолжения, по которой идёт колея соседки.
    pub fn lane_profile(self, body: LaneFrame, total: f32) -> Vec<(f32, LaneFrame)> {
        let asphalt = |along: f32| {
            self.frame_at(body, along).map(|mut frame| {
                if self.frame.high.abs() < self.frame.low.abs() {
                    frame.high = frame.high.max(body.high);
                } else {
                    frame.low = frame.low.min(body.low);
                }
                frame
            })
        };
        let steps = (self.length / RAMP_STEP).ceil().max(1.0) as usize;
        let mut profile: Vec<(f32, LaneFrame)> = (0..=steps)
            .filter_map(|step| {
                let along = self.along(self.length * step as f32 / steps as f32);
                (0.0..=total)
                    .contains(&along)
                    .then(|| (along, asphalt(along).unwrap_or(body)))
            })
            .collect();
        // клин, начатый на соседнем way или ушедший на следующий, — пары и на
        // торцах этого: за крайней парой профиль стоит
        for along in [0.0, total] {
            if let Some(frame) = asphalt(along) {
                profile.push((along, frame));
            }
        }
        profile.sort_by(|a, b| a.0.total_cmp(&b.0));
        profile
    }
}

/// Шаг вершин на клине слияния, м: раскладка по smoothstep — кривая.
const RAMP_STEP: f32 = 2.0;

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

/// Нахлёст разрыва крючка ([`hook_breaks`]) с найденным разрывом, м.
const HOOK_OVERLAP: f32 = 0.05;

/// Где осевая двусторонней дороги с `lanes` полосами: сдвиг от оси way в
/// раме раскладки ([`lane_frame`], плюс — влево по ходу точек) — граница
/// между потоками. Против хода точек — [`RoadLine::lanes_backward`] полос, а
/// без тега поровну; лишнюю полосу нечётной берёт поток по ходу точек. Так у
/// `lanes=5` осевая встаёт на границу полос, а не посреди средней, и её
/// не было вовсе: чётность решала, есть ли осевая (Ростов, витрина 03 —
/// Текучёва в пять полос одними пунктирами). `None` — осевой нет:
/// односторонняя или одна полоса на оба потока.
pub fn axis_offset(road: &RoadLine, lanes: u8, side: TrafficSide) -> Option<f32> {
    if road.oneway || lanes < 2 {
        return None;
    }
    let backward = road
        .lanes_backward
        .filter(|&backward| backward > 0 && backward < lanes)
        .unwrap_or(lanes / 2);
    let forward = f32::from(lanes - backward) * lane_width();
    let half = f32::from(lanes) * lane_width() / 2.0;
    // по ходу точек едут справа при правостороннем
    Some(match side {
        TrafficSide::Right => forward - half,
        TrafficSide::Left => half - forward,
    })
}

#[cfg(test)]
mod tests;
