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
//!   второстепенной улицы линий главной не рвёт; крестовина — два плеча чужих
//!   улиц в одном узле — рвёт всех, и старшую тоже, какого бы класса ни была
//!   поперечная (осевая насквозь — запрет левого поворота); половина
//!   разделённой улицы своей второй половине не соперник. Светофор в кластере
//!   рвёт всех;
//! - **зебра и стоп-линия на плече**, которое рвётся: зебра — по узлу
//!   `highway=crossing` на плече (до [`ARM_CROSSING_REACH`]), иначе
//!   ([`CrossingMode::Generated`]) — в [`ZEBRA_SETBACK`] от кромки узла, если
//!   в кластере сошлись две улицы с тротуарами (на регулируемом узле — если
//!   тротуары по обе стороны у самого плеча), одна из них не ниже
//!   `tertiary` (или узел под светофором) и ни одна не дуга кольца.
//!   Стоп-линия — за зеброй, на встречных узлу полосах; у `give_way`
//!   прерывистая. Дворовых проездов тут
//!   нет вовсе: они не проезжая часть и узлов не образуют;
//! - **зебра посреди квартала** — по любому размеченному переходу на улице,
//!   со стоп-линиями с обеих сторон, если переход регулируемый;
//! - **карман**: если у сквозной пары разное число полос, линия широкой
//!   дороги, которой на узкой места нет, кончается у кромки узла сплошной, а
//!   не висит посреди перекрёстка;
//! - **горло съезда** ([`Throat`]): от узла съезда с кольца до места, где
//!   съезд вышел из асфальта кольца, линии кольца со стороны съезда рвутся.
//!
//! **Кромка узла** на плече — где его сечение выходит из асфальта чужих дорог
//! кластера ([`clear_reach`]), не ближе полуширины самой широкой из них плюс
//! метр; у кольца — только это. Плечо, что из асфальта узла (с замощёнными
//! островами его треугольников) так и не вышло, — перемычка сложного узла: ни
//! зебры по правилу, ни стоп-линии, ни стрелок.
//!
//! Всё — точками на **нарисованной** оси улицы (`paths`): зебра и линия
//! полос лежат на одной кривой.

use std::collections::BTreeMap;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::corners::{MAX_ANGLE, MIN_ANGLE};
use super::drawn::{Axis, Drawn};
#[cfg(test)]
use super::is_carriageway;
use super::junctions::{JUNCTION_MARGIN, SharedNode, Visit, node_key};
use super::paint::{ArrowMarks, LineBreaks, PaintGround, Painter, trim_stop_line, whole_bars};
use super::ruts::RutLines;
use super::shape::RoadStyle;
use super::turns::Turns;
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::{point_in_polygon, ring_bounds};

mod cluster;
mod ends;
mod zebras;

use self::cluster::{Context, PASS_CHORD};
pub(in crate::map::roads) use self::ends::short_runs;
use self::ends::{bridge_short_runs, narrowing_ends, spill_over_ends};
use self::zebras::{breaks_across_zebras, join_touching, without_overlaps};
use crate::map::osm::{MapData, RoadLine, RoadNodeKind};

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

/// Сколько чистого асфальта линии полос оставляют вокруг зебры и
/// стоп-линии, м. Метр — столько же, сколько было видно, пока линия гасла у
/// разрыва за метр; теперь она обрывается резко на самом краю разрыва.
const PAINT_CLEAR: f32 = 1.0;

/// Косинус, круче которого два плеча своей улицы (хорды на [`PASS_CHORD`]
/// по нарисованному пути) не лежат на одной прямой: 15° —
/// двусторонняя улица узел насквозь не проходит, её линии рвутся (R22,
/// Путейская); односторонней нечем ломаться углом — осевой нет. Эхо
/// `SHARED_MAX_BEND` оси (`axis.rs`): мельче — дрожание оси OSM, круче —
/// поворот, который закреплённый узел дугой не скругляет, и осевая сплошной
/// легла бы через узел углом.
pub(in crate::map::roads) const PASS_ALIGN: f32 = 0.966;

/// Кусок линий между двумя разрывами короче этого — не рисуется: одинокий
/// штрих между узлом и зеброй читается мусором.
pub(super) const MIN_RUN: f32 = 6.0;

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

pub use super::network::pairs::Partner;

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

/// Горло съезда на дуге кольца: от узла съезда до места, где сечение съезда
/// вышло из асфальта кольца, линии кольца со стороны съезда (`side` — знак
/// левой нормали пути кольца: `+1` — слева по ходу пути, `−1` — справа)
/// рвутся в разрыве `gap`. Наружная полоса кольца там уходит в съезд, и её
/// пунктир шёл поперёк горла вразнобой со сплошной съезда (Тула, витрина 04,
/// юг). Линии по другую сторону оси кольца идут дальше.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Throat {
    pub gap: Break,
    pub side: f32,
}

/// Плечо узла так, как его видят траектории манёвров (`roads/turns.rs`):
/// дорога уходит от кромки узла — длины `edge` на её нарисованной оси — в
/// сторону `dir` (+1 — к концу). `link` — плечо это перемычка: его сечение
/// так и не вышло из асфальта узла ([`clear_reach`]), и стрелок на нём нет —
/// это горловина сложного узла, а не подход к нему.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct JunctionArm {
    pub road: usize,
    pub edge: f32,
    pub dir: f32,
    pub link: bool,
}

/// Узел (кластер) для траекторий: его плечи и дороги, что его **ведут** —
/// проходят насквозь, и уступать им некому. Колея ведущей идёт через узел
/// асфальтом ([`NodePaint::asphalt`]), и прямо по ней траектория не нужна.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Junction {
    pub arms: Vec<JunctionArm>,
    pub leading: Vec<usize>,
}

/// Разрывы **асфальта** по дорогам — заливке улиц и её колее: база без
/// разрывов ведущей узла, плюс островки по правилу
/// (`Junctions::add_splitters`). Свой тип, а не `&[Vec<Break>]`: заливка
/// берёт [`NodePaint::asphalt`] и не может взять разрывы краски или ряда,
/// у которых другие участники и другой вылет.
#[derive(Clone, Copy)]
pub struct AsphaltBreaks<'a>(&'a [Vec<Break>]);

impl<'a> AsphaltBreaks<'a> {
    pub fn of(&self, road: usize) -> &'a [Break] {
        &self.0[road]
    }
}

/// Разрывы **краски** по дорогам: где линии рвутся (`cut`) и какие узлы дорога
/// проходит насквозь (`solid` — осевая там сплошная). Одной дороге —
/// [`LineBreaks`] через [`Self::of`]; `Painter::paint` берёт его и ничего
/// другого.
#[derive(Clone, Copy)]
pub struct PaintBreaks<'a> {
    cut: &'a [Vec<Break>],
    solid: &'a [Vec<Break>],
    throats: &'a [Vec<Throat>],
    quiet: &'a [Vec<Break>],
}

impl<'a> PaintBreaks<'a> {
    pub fn of(&self, road: usize) -> LineBreaks<'a> {
        LineBreaks {
            cut: &self.cut[road],
            solid: &self.solid[road],
            throats: &self.throats[road],
            quiet: &self.quiet[road],
        }
    }
}

/// Узлы-кластеры для траекторий манёвров (`roads/turns.rs`) и порядка
/// заливки (`roads::fill_order`): у кластера — его плечи и ведущие.
/// Представление [`NodePaint`], наружу — из [`NodePaint::junctions`].
#[derive(Clone, Copy)]
pub struct PaintJunctions<'a>(&'a [Junction]);

impl<'a> PaintJunctions<'a> {
    /// Кластеры, как их обходит укладка.
    pub fn iter(self) -> impl Iterator<Item = &'a Junction> {
        self.0.iter()
    }

    /// Пуст ли список — узлов-кластеров нет.
    #[cfg(test)]
    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }

    /// Узлы теста — готовым списком кластеров.
    #[cfg(test)]
    pub(super) fn of(junctions: &'a [Junction]) -> Self {
        Self(junctions)
    }
}

/// Карманы краски у торцов дорог: у торца, за которым продолжение улицы
/// уже́, линиям широкой дороги, которым у узкой места нет, кончаться в узле
/// некуда — они гаснут в разрыве кармана, а не висят посреди перекрёстка.
/// Представление [`NodePaint`], наружу — из [`NodePaint::pockets`].
#[derive(Clone, Copy)]
pub struct PaintPockets<'a>(&'a [[Option<Pocket>; 2]]);

impl<'a> PaintPockets<'a> {
    /// Карманы дороги `road` — `[начало, конец]`.
    pub fn of(&self, road: usize) -> [Option<Pocket>; 2] {
        self.0[road]
    }
}

/// Поперечная краска узлов — зебры и стоп-линии: то, что кладётся поперёк
/// улицы, а не вдоль её линий. Зебра ложится целыми планками по асфальту
/// улиц, стоп-линия режется по нему же (`paint::PaintGround`), стрелки
/// встают за ними (`paint::ArrowMarks`), разделительные пропускают через
/// газон переходы (`medians::MedianInputs`). Представление [`NodePaint`],
/// наружу — из [`NodePaint::marks`].
#[derive(Clone, Copy)]
pub struct PaintMarks<'a> {
    zebras: &'a [Zebra],
    stop_lines: &'a [StopLine],
}

impl<'a> PaintMarks<'a> {
    /// Зебры — на плечах узлов и переходах посреди квартала.
    pub fn zebras(self) -> &'a [Zebra] {
        self.zebras
    }

    /// Стоп-линии — на встречных узлу полосах.
    pub fn stop_lines(self) -> &'a [StopLine] {
        self.stop_lines
    }
}

/// Что краска узлов насчитала — в `JunctionCounts` (`roads/junctions.rs`):
/// кластеры, проходы насквозь, зебры (все и из них по OSM), стоп-линии,
/// карманы.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct PaintCounts {
    /// Кластеров сближенных узлов — из двух узлов и больше.
    pub(super) clusters: usize,
    /// Узлов, где главная прошла насквозь.
    pub(super) through: usize,
    /// Зебры: все и из них по OSM.
    pub(super) zebras: [usize; 2],
    /// Стоп-линии — на встречных узлу полосах.
    pub(super) stop_lines: usize,
    /// Карманы краски у торцов дорог.
    pub(super) pockets: usize,
}

/// Краска узлов карты.
#[derive(Default)]
pub struct NodePaint {
    /// Разрывы краски по дорогам — вместо разрывов асфальта. Наружу — только
    /// как [`PaintBreaks`] ([`Self::lines`]), вместе с `solid`.
    breaks: Vec<Vec<Break>>,
    /// Разрывы асфальта — колеи и разделительных: базовые без тех, что лежали
    /// на ведущей дороге узла. Её колея идёт сквозь. Наружу — только как
    /// [`AsphaltBreaks`] ([`Self::asphalt`]).
    asphalt: Vec<Vec<Break>>,
    /// Узлы, которые дорога проходит насквозь, — там, где её разрыв был бы,
    /// уступай она. Осевая у такого узла сплошная, как и перед разрывом
    /// (`Painter::paint`): через примыкание не обгоняют.
    solid: Vec<Vec<Break>>,
    /// Тихие разрывы из `breaks` — у угла двух улиц ([`is_corner`]): линии
    /// рвутся, но перед ними не сплошные. Наружу — в [`PaintBreaks`].
    quiet: Vec<Vec<Break>>,
    /// Узлы-кластеры для траекторий и порядка заливки. Наружу — только как
    /// [`PaintJunctions`] ([`Self::junctions`]).
    junctions: Vec<Junction>,
    /// Карманы у торцов дорог `[начало, конец]`. Наружу — только как
    /// [`PaintPockets`] ([`Self::pockets`]).
    pockets: Vec<[Option<Pocket>; 2]>,
    /// Горла съездов на дорогах колец. Наружу — в [`PaintBreaks`].
    throats: Vec<Vec<Throat>>,
    /// Зебры. Наружу — только как [`PaintMarks`] ([`Self::marks`]).
    zebras: Vec<Zebra>,
    /// Дорога каждой зебры, пока они собираются: по ней [`join_touching`]
    /// узнаёт половины на газоне.
    zebra_roads: Vec<usize>,
    /// Стоп-линии. Наружу — в [`PaintMarks`].
    stop_lines: Vec<StopLine>,
    /// Кластеры из двух узлов и больше — в [`Self::counts`].
    clusters: usize,
    /// Узлы, где главная прошла насквозь — в [`Self::counts`].
    through: usize,
    /// Разрывы краски, обрезанные концом пути: конец и остаток — его
    /// получает продолжение улицы за ним ([`spill_over_ends`]).
    spills: Vec<(Vec2, f32)>,
}

/// Что рисовать на узлах.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NodePaintStyle {
    pub crossings: CrossingMode,
    pub stop_lines: bool,
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
        nearest_on_path(self.path, point).map_or(0.0, |(_, along)| along)
    }

    /// Точка и направление пути на длине `at`, если она на пути.
    fn at(&self, at: f32) -> Option<(Vec2, Vec2)> {
        (0.0..=self.total)
            .contains(&at)
            .then(|| place_on_path(self.path, &self.along, at))
            .flatten()
    }

    /// Направление хорды пути от длины `from` к длине `to`; `None` — одна из
    /// них не на пути или хорда вырождена.
    fn chord(&self, from: f32, to: f32) -> Option<Vec2> {
        let (start, _) = self.at(from)?;
        let (end, _) = self.at(to)?;
        (end - start).try_normalize()
    }

    /// Что от отрезка длин `[a, b]` выходит за концы пути: конец и на
    /// сколько. [`Self::gap`] это обрезает — продолжение улицы за концом
    /// получает остаток ([`spill_over_ends`]).
    fn spills(&self, a: f32, b: f32) -> impl Iterator<Item = (Vec2, f32)> + use<> {
        let (low, high) = (a.min(b), a.max(b));
        let first = self.path.first().copied();
        let last = self.path.last().copied();
        [
            first.filter(|_| low < 0.0).map(|at| (at, -low)),
            last.filter(|_| high > self.total)
                .map(|at| (at, high - self.total)),
        ]
        .into_iter()
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

/// Замощённый остров треугольника узлов (`corners::small_islands`) — тоже
/// асфальт узла: ветки развилки идут по нему от узла до узла.
struct PavedIsland {
    low: Vec2,
    high: Vec2,
    ring: Vec<Vec2>,
}

impl PavedIsland {
    fn new(ring: &[Vec2]) -> Self {
        let (low, high) = ring_bounds(ring);
        Self {
            low,
            high,
            ring: ring.to_vec(),
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.low).all()
            && point.cmple(self.high).all()
            && point_in_polygon(point, &self.ring)
    }
}

/// Переход на дороге.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    along: f32,
    signals: bool,
    used: bool,
}

/// Узел на пути дороги: длина на её пути, сама точка и где по ней кончается
/// асфальт других дорог узла — полуширина самой широкой из них, м.
#[derive(Clone, Copy)]
struct NodeAlong {
    along: f32,
    at: Vec2,
    reach: f32,
}

impl NodePaint {
    /// Разрывы асфальта — заливке улиц (`roads::push_street_fill`) и клиньям.
    pub fn asphalt(&self) -> AsphaltBreaks<'_> {
        AsphaltBreaks(&self.asphalt)
    }

    /// Разрывы краски — линиям (`Painter::paint`), второму ряду стрелок и
    /// зебрам поперёк разделительной (`medians::crossing_breaks`).
    pub fn lines(&self) -> PaintBreaks<'_> {
        PaintBreaks {
            cut: &self.breaks,
            solid: &self.solid,
            throats: &self.throats,
            quiet: &self.quiet,
        }
    }

    /// Узлы-кластеры — траекториям манёвров (`roads/turns.rs`) и порядку
    /// заливки (`roads::fill_order`).
    pub fn junctions(&self) -> PaintJunctions<'_> {
        PaintJunctions(&self.junctions)
    }

    /// Карманы краски у торцов дорог — линиям (`Painter::paint`).
    pub fn pockets(&self) -> PaintPockets<'_> {
        PaintPockets(&self.pockets)
    }

    /// Поперечная краска узлов — зебры и стоп-линии.
    pub fn marks(&self) -> PaintMarks<'_> {
        PaintMarks {
            zebras: &self.zebras,
            stop_lines: &self.stop_lines,
        }
    }

    /// Что краска узлов насчитала — в `JunctionCounts`
    /// (`roads/junctions.rs`).
    pub(super) fn counts(&self) -> PaintCounts {
        PaintCounts {
            clusters: self.clusters,
            through: self.through,
            zebras: [
                self.zebras.len(),
                self.zebras.iter().filter(|zebra| zebra.osm).count(),
            ],
            stop_lines: self.stop_lines.len(),
            pockets: self.pockets.iter().flatten().flatten().count(),
        }
    }

    /// Горла съездов на дороге `road` ([`Throat`]) — тестам; краска берёт их
    /// из [`Self::lines`].
    #[cfg(test)]
    pub(super) fn throats(&self, road: usize) -> &[Throat] {
        &self.throats[road]
    }

    /// Островок по правилу на подходе к кольцу (`gores::splitters`): подход
    /// рвётся на его длину и краской, и колеей — единственная правка снаружи
    /// (`Junctions::add_splitters`).
    pub(super) fn add_splitter(&mut self, road: usize, gap: Break) {
        self.breaks[road].push(gap);
        self.asphalt[road].push(gap);
    }

    /// Краска узлов теста: узлы проезжих частей со стежками `prepared` — те,
    /// что `Junctions::new` передал бы [`Self::new`], — и готовая база.
    #[cfg(test)]
    pub(super) fn for_test(
        prepared: &Drawn,
        base: &[Vec<Break>],
        map: &MapData,
        paved: &[Vec<Vec2>],
        style: NodePaintStyle,
    ) -> Self {
        let nodes = super::junctions::with_stitches(
            &prepared.roads(),
            is_carriageway,
            &prepared.stitches().targets,
        );
        Self::new(prepared, &nodes, base, map, paved, style)
    }

    /// Краска узлов по подготовленным дорогам `prepared` (`roads/drawn.rs`),
    /// по оси ленты (`Axis::Ribbon` — стежки тоже узлы). `base` — разрывы
    /// асфальта (`junctions::marking_breaks`), `paved` — замощённые острова
    /// треугольников узлов (`corners::small_islands`). Из `Drawn` берутся:
    /// тротуар по тегу (`sidewalk_mapped` — `sidewalk=*` независимо от
    /// `RoadStyle::sidewalks`: ручка прячет ленту, а зебра по правилу —
    /// вопрос модели), вторые половины разделённой улицы (`partners`), дуги
    /// колец (`on_ring`, `roads/rings.rs` — узел кольца зебры по правилу не
    /// получает) и слияния (`merges`, `roads/merges.rs`): узел слияния без
    /// других проезжих частей — не перекрёсток, улица его проходит, линии не
    /// рвутся, осевая продолжения у него сплошная.
    ///
    /// `nodes` — узлы проезжих частей со стежками, те же, по которым считана
    /// база (`junctions::Junctions::new` обходит их один раз).
    pub(super) fn new(
        prepared: &Drawn,
        nodes: &[SharedNode],
        base: &[Vec<Break>],
        map: &MapData,
        paved: &[Vec<Vec2>],
        style: NodePaintStyle,
    ) -> Self {
        let drawn = prepared.roads();
        let paths = prepared.axes(Axis::Ribbon);
        let (drawn, paths) = (drawn.as_slice(), paths.as_slice());
        let merges = &prepared.merges().list;
        let sidewalk = |road: usize| prepared.sidewalk_mapped(road).is_some();
        let partners = |road: usize| prepared.pairs().partners(road).collect::<Vec<Partner>>();
        let on_ring = |road: usize| prepared.on_ring(road);
        assert_eq!(base.len(), drawn.len(), "разрывы — на каждую дорогу карты");
        let mut paint = Self {
            breaks: base.to_vec(),
            asphalt: base.to_vec(),
            pockets: vec![[None; 2]; drawn.len()],
            throats: vec![Vec::new(); drawn.len()],
            solid: vec![Vec::new(); drawn.len()],
            quiet: vec![Vec::new(); drawn.len()],
            ..Self::default()
        };
        let merged = |node: &SharedNode| {
            merges.iter().find(|merge| {
                merge.pure
                    && node_key(merge.node) == node_key(node.at)
                    && node
                        .visits
                        .iter()
                        .all(|visit| merge.roads().contains(&visit.road))
            })
        };
        for node in nodes {
            let Some(merge) = merged(node) else {
                continue;
            };
            let key = node_key(node.at);
            for road in merge.roads() {
                // и колея не гаснет: раскладка половин сведена к продолжению
                paint.breaks[road].retain(|found| node_key(found.at) != key);
                paint.asphalt[road].retain(|found| node_key(found.at) != key);
                let widest = merge
                    .roads()
                    .into_iter()
                    .filter(|&other| other != road)
                    .map(|other| drawn[other].width / 2.0)
                    .fold(0.0_f32, f32::max);
                paint.solid[road].push(Break {
                    at: node.at,
                    reach: widest + JUNCTION_MARGIN,
                });
            }
        }
        let street = |road: usize| {
            map.network
                .street_of(road)
                .map_or(usize::MAX - road, |(street, _)| street)
        };
        // узел краски: перекрёсток или угол двух улиц, не чистое слияние
        let painted = |node: &SharedNode| {
            (node.is_junction() || is_corner(node, paths, &street)) && merged(node).is_none()
        };
        let junctions: Vec<&SharedNode> = nodes.iter().filter(|node| painted(node)).collect();
        let marks: HashMap<(i32, i32), RoadNodeKind> = map
            .road_nodes
            .iter()
            .map(|node| (node_key(node.pos), node.kind))
            .collect();
        let mut near_marks: Grid<usize> = Grid::new(CLUSTER_CELL);
        for (index, node) in map.road_nodes.iter().enumerate() {
            near_marks.insert(node.pos, node.pos, index);
        }

        // переходы по дорогам — на нарисованной оси
        let mut crossings: Vec<Vec<Crossing>> = vec![Vec::new(); drawn.len()];
        if style.crossings != CrossingMode::Off {
            for node in nodes {
                let Some(RoadNodeKind::Crossing {
                    signals,
                    marked: true,
                    ..
                }) = marks.get(&node_key(node.at))
                else {
                    continue;
                };
                // чистый узел слияния — не перекрёсток: переход OSM в нём
                // ложится поперёк продолжения, как посреди улицы (Болдина у
                // кольца, R16/R17)
                let merge = merged(node);
                if painted(node) {
                    continue;
                }
                for visit in &node.visits {
                    if merge.is_some_and(|merge| visit.road != merge.street) {
                        continue;
                    }
                    // зебры по грунту не бывает: обрывки белых планок на
                    // щебне у тротуара читались мусором (Калуга, 06)
                    if drawn[visit.road].bridge || drawn[visit.road].is_unpaved_street() {
                        continue;
                    }
                    let walk = Walk::new(paths[visit.road].as_ref());
                    let mut along = walk.project(node.at);
                    // у слияния — целиком на продолжении, до линий веток
                    if merge.is_some() {
                        let reach = ZEBRA_LENGTH / 2.0 + PAINT_CLEAR;
                        along = along.clamp(reach, (walk.total - reach).max(reach));
                    }
                    crossings[visit.road].push(Crossing {
                        along,
                        signals: *signals,
                        used: false,
                    });
                }
            }
        }

        let paved: Vec<PavedIsland> = paved
            .iter()
            .filter(|ring| ring.len() >= 3)
            .map(|ring| PavedIsland::new(ring))
            .collect();
        let mut nodes_along: Vec<Vec<NodeAlong>> = vec![Vec::new(); drawn.len()];
        for node in &junctions {
            for visit in &node.visits {
                let walk = Walk::new(paths[visit.road].as_ref());
                let reach = node
                    .visits
                    .iter()
                    .filter(|other| other.road != visit.road)
                    .map(|other| drawn[other.road].width / 2.0)
                    .fold(0.0, f32::max);
                nodes_along[visit.road].push(NodeAlong {
                    along: walk.project(node.at),
                    at: node.at,
                    reach,
                });
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
                    on_ring: &on_ring,
                    nodes_along: &nodes_along,
                    paved: &paved,
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
                    style,
                    map.traffic_side,
                );
            }
        }
        let junction_keys: Vec<(i32, i32)> =
            junctions.iter().map(|node| node_key(node.at)).collect();
        // планка зебры — поперёк всех, чьё полотно через неё идёт
        breaks_across_zebras(
            drawn,
            paths,
            &paint.zebras,
            &paint.zebra_roads,
            &mut paint.breaks,
        );
        let spills = std::mem::take(&mut paint.spills);
        spill_over_ends(drawn, paths, &junction_keys, &spills, &mut paint.breaks);
        let narrowing = narrowing_ends(drawn, paths);
        for (road, breaks) in paint.breaks.iter_mut().enumerate() {
            bridge_short_runs(paths[road].as_ref(), breaks, narrowing[road]);
        }
        let lawn = |a: usize, b: usize| {
            partners(a)
                .iter()
                .any(|partner| partner.road == b && !partner.paved)
        };
        let roads = std::mem::take(&mut paint.zebra_roads);
        let zebras = join_touching(std::mem::take(&mut paint.zebras), &roads, lawn);
        paint.zebras = without_overlaps(zebras);
        paint
    }
}

/// Зона узла: полуширина самой широкой его дороги и запас на скругление.
fn zone(drawn: &[&RoadLine], node: &SharedNode) -> f32 {
    node.visits
        .iter()
        .map(|visit| drawn[visit.road].width / 2.0)
        .fold(0.0_f32, f32::max)
        + CLUSTER_ZONE
}

/// Угол двух улиц: в узле кончаются торцами ровно две дороги разных улиц, и
/// их нарисованные оси (хорды на [`PASS_CHORD`] — угол линий у самого узла)
/// сходятся под углом от `corners::MIN_ANGLE` до `MAX_ANGLE` — тем же, под
/// которым `corners.rs` скругляет там бордюр. [`SharedNode::is_junction`]
/// такой узел считает швом, и линии обеих улиц сходились в нём углом
/// (Белгород, Пушкина и Народный бульвар, 3171, 3553 — R22). Соосный шов и
/// шов посреди плавного поворота (хорда на 10 м читала и его, Орёл, 4537,
/// 1208) — не угол.
fn is_corner(
    node: &SharedNode,
    paths: &[impl AsRef<[Vec2]>],
    street: &impl Fn(usize) -> usize,
) -> bool {
    let [a, b] = node.visits.as_slice() else {
        return false;
    };
    if a.road == b.road
        || !a.end
        || !b.end
        || a.inner
        || b.inner
        || street(a.road) == street(b.road)
    {
        return false;
    }
    let away = |visit: &Visit| {
        let walk = Walk::new(paths[visit.road].as_ref());
        let from = walk.project(node.at);
        let dir = if visit.vertex == 0 { 1.0 } else { -1.0 };
        walk.chord(from, (from + dir * PASS_CHORD).clamp(0.0, walk.total))
    };
    let (Some(first), Some(second)) = (away(a), away(b)) else {
        return false;
    };
    (MIN_ANGLE..=MAX_ANGLE).contains(&first.angle_to(second).abs())
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

/// Краска узлов и колея траекторий: зебра — из целых звеньев на асфальте
/// улиц, каким он уже лёг; стоп-линия обрезается по тому же асфальту: у
/// устья съезда её конец ложился на плитку обочины мимо наискось стоящей
/// кромки (S7)
pub(super) fn paint_junction_marks(
    marks: PaintMarks<'_>,
    streets: &MeshBuilder,
    painter: &mut Painter,
    turns: &Turns,
    ruts: &mut RutLines,
    paint_breaks: PaintBreaks<'_>,
    style: RoadStyle,
) {
    let ground = PaintGround::new(marks.zebras(), marks.stop_lines(), streets.triangles());
    // колея режется по асфальту под ней — узлы без зебр и стоп-линий
    // колею тоже несут, и `ground` (асфальт только у краски) её не видит
    let asphalt = PaintGround::under_wear(&turns.wear, streets.triangles());
    for zebra in marks.zebras() {
        if let Some(zebra) = whole_bars(zebra, |point| ground.contains(point)) {
            painter.paint_zebra(&zebra);
        }
    }
    for line in marks.stop_lines() {
        if let Some(line) = trim_stop_line(line, |point| ground.contains(point)) {
            painter.paint_stop_line(&line);
        }
    }
    // колея траекторий — всегда, как колея полос; только по асфальту:
    // у кромки поворота её полоса перелезала бордюрную дугу на плитку
    painter.paint_turn_wear(&turns.wear, |point| asphalt.contains(point));
    // те же кривые и хвосты — оверлею колеи, без пересчёта
    for junction in &turns.wear {
        ruts.curves.extend(junction.curves.iter().cloned());
        ruts.tails.extend(junction.tails.iter().copied());
    }
    // стрелки на полосах подходов — краска, своим тумблером
    if style.arrows {
        let arrow_marks = ArrowMarks::new(marks.zebras(), marks.stop_lines());
        for arrow in &turns.arrows {
            let setback = Painter::arrow_setback(arrow, &arrow_marks);
            painter.paint_arrow(arrow, setback);
            // и второй ряд дальше от узла, где полоса это позволяет
            let breaks = paint_breaks.of(arrow.road).cut;
            if let Some(repeat) = Painter::repeat_setback(arrow, setback, &arrow_marks, breaks) {
                painter.paint_arrow(arrow, repeat);
            }
        }
    }
}

#[cfg(test)]
mod tests;
