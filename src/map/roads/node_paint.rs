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
//!   улиц в одном узле — рвёт равную, как и проходящая насквозь, а крестовина
//!   не ниже `tertiary` ([`CROSSING_CUTS_RANK`]) — и старшую; половина
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
//!   не висит посреди перекрёстка.
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

use super::drawn::{Axis, Drawn};
use super::junctions::{JUNCTION_MARGIN, SharedNode, Visit, node_key};
use super::network::pairs::TRAM_BED_MAX_GAP;
use super::paint::{LineBreaks, axis_offset};
use super::{is_carriageway, lane_count};
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::footprint::distance_to_polyline;
use crate::map::grid::Grid;
use crate::map::meshing::Break;
use crate::map::osm::model::{point_in_polygon, ring_bounds};
use crate::map::osm::{Highway, MapData, RoadLine, RoadNodeKind, TrafficSide};
use crate::map::shapes::is_ring;

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
const STOP_GAP: f32 = 1.0;
/// Сколько чистого асфальта линии полос оставляют вокруг зебры и
/// стоп-линии, м. Метр — столько же, сколько было видно, пока линия гасла у
/// разрыва за метр; теперь она обрывается резко на самом краю разрыва.
const PAINT_CLEAR: f32 = 1.0;
/// Сколько дороги должно остаться за краской плеча, м.
const ARM_TAIL: f32 = 8.0;
/// Сколько дороги нужно зебре по правилу от кромки узла до следующего узла
/// той же дороги, м. Короче — перемычка между двумя узлами (ветки
/// треугольника развилки в 22 и 31 м, Тула, витрина 06): зебры с обоих её
/// концов и стоп-линия между ними теснятся на пятнадцати метрах, и пешеход
/// переходит на внешних плечах.
const RULE_ZEBRA_ROOM: f32 = 30.0;
/// Размеченный переход OSM на той же улице ближе этого к точке узла — и
/// зебры по правилу на плечах этой улицы нет: переход узла уже есть по
/// данным, м. Та же досягаемость, на которой переход становится зеброй
/// самого плеча ([`ARM_CROSSING_REACH`]).
const RULE_ZEBRA_DATA_REACH: f32 = ARM_CROSSING_REACH;
/// Ранг ([`class_rank`]) улицы, без которой в кластере зебры по правилу нет,
/// если узел не под светофором: `tertiary`.
const RULE_ZEBRA_RANK: u8 = 2;
/// Ранг ([`class_rank`]) крестовины, которая рвёт линии и старшей дороги:
/// `tertiary`. Через поле настоящего перекрёстка линий полос не кладут ни
/// одной из дорог — в Орле (витрина 05) сплошная primary Московской шла
/// наискось через полотно secondary-пары Пушкина. Жилая крестовина главную
/// не рвёт: там её осевая через узел — обычное дело.
const CROSSING_CUTS_RANK: u8 = 2;
/// Кусок линий между двумя разрывами короче этого — не рисуется: одинокий
/// штрих между узлом и зеброй читается мусором.
const MIN_RUN: f32 = 6.0;
/// Насколько зебры могут зайти одна на другую краями, м.
const OVERLAP_SLACK: f32 = 0.2;
/// Зебры двух половин сливаются в одну планку ([`join_zebras`]), если они
/// параллельны (косинус), лежат на одной прямой с точностью до метра и между
/// ними не шире самой широкой асфальтовой разделительной с отступами
/// [`EDGE_INSET`] от обеих кромок. Самая широкая — трамвайное полотно, оно
/// мощёное до [`TRAM_BED_MAX_GAP`] (разделительная по ручке `Median gap` —
/// до 6 м), и [`OVERLAP_SLACK`] на то, что планки ложатся не по пробе.
const JOIN_PARALLEL: f32 = 0.95;
const JOIN_OFFSET: f32 = 1.0;
const JOIN_GAP: f32 = TRAM_BED_MAX_GAP + 2.0 * EDGE_INSET + OVERLAP_SLACK;
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
}

impl<'a> PaintBreaks<'a> {
    pub fn of(&self, road: usize) -> LineBreaks<'a> {
        LineBreaks {
            cut: &self.cut[road],
            solid: &self.solid[road],
        }
    }
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
    pub junctions: Vec<Junction>,
    /// Карманы у торцов дорог `[начало, конец]`.
    pub pockets: Vec<[Option<Pocket>; 2]>,
    pub zebras: Vec<Zebra>,
    pub stop_lines: Vec<StopLine>,
    /// Кластеры из двух узлов и больше.
    pub clusters: usize,
    /// Узлы, где главная прошла насквозь.
    pub through: usize,
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

/// Как далеко от узла кромка плеча ищется по асфальту кластера, м: дальше —
/// плечо целиком перемычка внутри узла ([`JunctionArm::link`]), и кромка
/// остаётся на полуширине соседа.
const EDGE_SEARCH: f32 = 25.0;
/// Шаг этого поиска, м.
const EDGE_STEP: f32 = 0.5;

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

/// Где сечение плеча выходит из асфальта кластера — чужих дорог и
/// замощённых островов `paved`: длина от узла `from` по ходу `dir`, не
/// меньше `reach`. Сечение — две точки в `half` по обе стороны оси; чужая
/// дорога — путь и полуширина. Плечо, что за [`EDGE_SEARCH`] или до конца
/// своей дороги так и не вышло, — `None`: перемычка.
fn clear_reach(
    walk: &Walk,
    from: f32,
    dir: f32,
    reach: f32,
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
    while ahead <= EDGE_SEARCH {
        if clear(ahead) {
            return Some(ahead);
        }
        ahead += EDGE_STEP;
    }
    None
}

/// Переход на дороге.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    along: f32,
    signals: bool,
    used: bool,
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
        }
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
            solid: vec![Vec::new(); drawn.len()],
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
        let junctions: Vec<&SharedNode> = nodes
            .iter()
            .filter(|node| node.is_junction() && merged(node).is_none())
            .collect();
        let marks: HashMap<(i32, i32), RoadNodeKind> = map
            .road_nodes
            .iter()
            .map(|node| (node_key(node.pos), node.kind))
            .collect();
        let mut near_marks: Grid<usize> = Grid::new(CLUSTER_CELL);
        for (index, node) in map.road_nodes.iter().enumerate() {
            near_marks.insert(node.pos, node.pos, index);
        }
        let street = |road: usize| {
            map.network
                .street_of(road)
                .map_or(usize::MAX - road, |(street, _)| street)
        };

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
                if node.is_junction() {
                    continue;
                }
                for visit in &node.visits {
                    // зебры по грунту не бывает: обрывки белых планок на
                    // щебне у тротуара читались мусором (Калуга, 06)
                    if drawn[visit.road].bridge || drawn[visit.road].is_unpaved_street() {
                        continue;
                    }
                    let walk = Walk::new(paths[visit.road].as_ref());
                    crossings[visit.road].push(Crossing {
                        along: walk.project(node.at),
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
        let mut nodes_along: Vec<Vec<(f32, Vec2)>> = vec![Vec::new(); drawn.len()];
        for node in &junctions {
            for visit in &node.visits {
                let walk = Walk::new(paths[visit.road].as_ref());
                nodes_along[visit.road].push((walk.project(node.at), node.at));
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
        let spills = std::mem::take(&mut paint.spills);
        spill_over_ends(drawn, paths, &junction_keys, &spills, &mut paint.breaks);
        let narrowing = narrowing_ends(drawn, paths);
        for (road, breaks) in paint.breaks.iter_mut().enumerate() {
            bridge_short_runs(paths[road].as_ref(), breaks, narrowing[road]);
        }
        paint.zebras = without_overlaps(std::mem::take(&mut paint.zebras));
        paint
    }

    fn paint_cluster(
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
        let rule_zebras = (signalized || major) && !at_ring;
        // дорога самого кольца: дуга (`roads/rings.rs`) или кольцо одним way
        let ring_road = |road: usize| on_ring(road) || is_closed(&drawn[road].points);

        let mut broken: BTreeMap<usize, f32> = BTreeMap::new();
        let mut reaches: BTreeMap<usize, f32> = BTreeMap::new();
        let mut leading: Vec<usize> = Vec::new();
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
            // ранг крестовины — младшей из двух её улиц; `None` — крестовины нет
            let crossing = foreign
                .iter()
                .enumerate()
                .flat_map(|(index, arm)| {
                    foreign[index + 1..]
                        .iter()
                        .filter(|other| other.at.distance(arm.at) < JUNCTION_MARGIN)
                        .map(|other| {
                            class_rank(drawn[arm.road].highway)
                                .min(class_rank(drawn[other.road].highway))
                        })
                })
                .max();
            let crossed = crossing.is_some();
            // крестовина не ниже `tertiary` рвёт и старшую дорогу
            let cut = crossing.is_some_and(|rank| rank >= CROSSING_CUTS_RANK);
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
                        && !cut
                        && !others.iter().any(|&other| {
                            let theirs = rank(other);
                            theirs > own || (theirs == own && (passes(other) || crossed))
                        }));
            let yields = signalized || !leads;
            let widest = others
                .iter()
                .map(|&other| drawn[other].width / 2.0)
                .fold(0.0_f32, f32::max);
            let reach = widest + JUNCTION_MARGIN;
            reaches.insert(road, reach);
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
            breaks.extend(here.iter().map(|&at| Break { at, reach }));
            // соседние узлы кластера на одной дороге — один разрыв
            for pair in here.windows(2) {
                breaks.push(Break {
                    at: (pair[0] + pair[1]) / 2.0,
                    reach: pair[0].distance(pair[1]) / 2.0,
                });
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
        let arm_edge = |arm: &Arm, walk: &Walk| -> (f32, bool) {
            let from = walk.project(arm.at);
            let reach = reach_of(arm.road);
            if at_ring {
                return (from + arm.dir * reach, false);
            }
            let foreign: Vec<(&[Vec2], f32)> = others(arm.road)
                .into_iter()
                .map(|other| (paths[other].as_ref(), drawn[other].width / 2.0))
                .collect();
            let half = drawn[arm.road].width / 2.0 - EDGE_INSET;
            match clear_reach(walk, from, arm.dir, reach, half, &foreign, paved) {
                Some(ahead) => (from + arm.dir * ahead, false),
                None => (from + arm.dir * reach, true),
            }
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
            // где точка сечения на `offset` вбок от оси вышла из кольца
            let exit = |offset: f32| -> Option<(f32, Vec2)> {
                let mut ahead = 0.0;
                while ahead <= EDGE_SEARCH {
                    let (point, tangent) = walk.at(from + arm.dir * ahead)?;
                    let side = point + tangent.perp() * offset;
                    if ring_roads
                        .iter()
                        .all(|&(path, half)| distance_to_polyline(side, path) >= half)
                    {
                        return Some((ahead, side));
                    }
                    ahead += EDGE_STEP;
                }
                None
            };
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
                let closed = path.len() > 2 && path[0] == path[path.len() - 1];
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

        // зебры и стоп-линии на плечах, что рвутся: сперва где встать зебре
        let first = ZEBRA_SETBACK + ZEBRA_LENGTH / 2.0;
        let mut plans: Vec<ArmPlan> = Vec::new();
        for arm in &arms {
            if !broken.contains_key(&arm.road) || drawn[arm.road].bridge {
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
            let room = nodes_along[arm.road]
                .iter()
                .filter(|(_, at)| cluster.iter().all(|node| node.at != *at))
                .map(|(along, _)| (along - edge) * dir)
                .filter(|ahead| *ahead > 0.0)
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
            .filter(|&at| ring_line.is_some() || !in_other(at));
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
                self.stop_lines.extend(stop_line_at(
                    &walk,
                    road,
                    at,
                    dir,
                    map.traffic_side,
                    yields,
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
        }
    }

    /// Переход не у узла: зебра с разрывом линий вокруг и стоп-линиями по
    /// обе стороны, если он регулируемый.
    fn paint_crossing(
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
        let mut reach = ZEBRA_LENGTH / 2.0;
        if style.stop_lines && crossing.signals {
            let offset = ZEBRA_LENGTH / 2.0 + STOP_GAP + STOP_WIDTH / 2.0;
            for dir in [-1.0, 1.0] {
                if incoming(road, dir) {
                    self.stop_lines.extend(stop_line_at(
                        &walk,
                        road,
                        center + dir * offset,
                        dir,
                        side,
                        false,
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

/// Плечо, которое рвётся, и где на нём встанет зебра: длина центра и по
/// данным ли она.
struct ArmPlan<'a> {
    arm: Arm,
    walk: Walk<'a>,
    /// Длина кромки узла на пути плеча.
    edge: f32,
    /// Переход OSM, которым стала зебра, — индекс в переходах дороги.
    osm: Option<usize>,
    zebra: Option<(f32, bool)>,
    /// Плечо — перемычка сложного узла ([`JunctionArm::link`]).
    link: bool,
    /// Въезд в кольцо: линия по его кромке ([`RingEntry::line`]).
    ring_line: Option<(Vec2, Vec2)>,
}

/// Въезд в кольцо: где подход выходит из асфальта самого кольца.
struct RingEntry {
    /// Длина на пути плеча, где из кольца вышло всё сечение.
    edge: f32,
    /// Отрезок линии уступи дорогу — от середины (у односторонней — от левой
    /// кромки) к бордюру, каждый конец там, где из кольца вышла его сторона.
    line: (Vec2, Vec2),
}

/// Замкнута ли ломаная — кольцо одним way.
fn is_closed(points: &[Vec2]) -> bool {
    points.len() > 2 && points[0] == points[points.len() - 1]
}

impl ArmPlan<'_> {
    /// Центр зебры и направление от узла наружу.
    fn zebra_frame(&self) -> Option<(Vec2, Vec2)> {
        let (center, _) = self.zebra?;
        let (point, direction) = self.walk.at(center)?;
        Some((point, direction * self.arm.dir))
    }
}

/// Зебры двух половин одной улицы — на одну линию поперёк неё: вторая
/// сдвигается к той, что по данным, а если обе по правилу — к дальней от
/// узла. Два перехода OSM — по узлу на каждой половине — маппер ставит не на
/// одну прямую (на 02 они разошлись на метр), и тогда обе встают посередине
/// между ними: сдвиг в метр остаётся внутри длины самой планки. Не ближе
/// кромки узла.
fn align_pair(plans: &mut [ArmPlan], a: usize, b: usize) {
    let (Some((point_a, out_a)), Some((point_b, out_b))) =
        (plans[a].zebra_frame(), plans[b].zebra_frame())
    else {
        return;
    };
    if out_a.dot(out_b) < 0.5 {
        return;
    }
    let out = (out_a + out_b).normalize();
    let osm = |plan: &ArmPlan| plan.zebra.is_some_and(|(_, osm)| osm);
    let (line, moved) = match (osm(&plans[a]), osm(&plans[b])) {
        (true, true) => (
            (point_a.dot(out) + point_b.dot(out)) / 2.0,
            vec![(a, point_a, out_a), (b, point_b, out_b)],
        ),
        (true, false) => (point_a.dot(out), vec![(b, point_b, out_b)]),
        (false, true) => (point_b.dot(out), vec![(a, point_a, out_a)]),
        (false, false) if point_a.dot(out) >= point_b.dot(out) => {
            (point_a.dot(out), vec![(b, point_b, out_b)])
        }
        (false, false) => (point_b.dot(out), vec![(a, point_a, out_a)]),
    };
    for (index, from, along) in moved {
        let shift = (line - from.dot(out)) / along.dot(out);
        let plan = &mut plans[index];
        let dir = plan.arm.dir;
        let Some((center, osm)) = plan.zebra else {
            continue;
        };
        let nearest = plan.edge + dir * (ZEBRA_SETBACK + ZEBRA_LENGTH / 2.0);
        let center = center + dir * shift;
        plan.zebra = Some((
            if (center - nearest) * dir < 0.0 {
                nearest
            } else {
                center
            },
            osm,
        ));
    }
}

/// То, что `paint_cluster` читает, одним аргументом.
struct Context<'a, P> {
    drawn: &'a [&'a RoadLine],
    paths: &'a [P],
    map: &'a MapData,
    marks: &'a HashMap<(i32, i32), RoadNodeKind>,
    near_marks: &'a Grid<usize>,
    style: NodePaintStyle,
    street: &'a dyn Fn(usize) -> usize,
    sidewalk: &'a dyn Fn(usize) -> bool,
    partners: &'a dyn Fn(usize) -> Vec<Partner>,
    on_ring: &'a dyn Fn(usize) -> bool,
    /// Узлы на каждой дороге: длина на её пути и сама точка.
    nodes_along: &'a [Vec<(f32, Vec2)>],
    /// Замощённые острова треугольников узлов.
    paved: &'a [PavedIsland],
}

impl<P> Clone for Context<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for Context<'_, P> {}

/// Зона узла: полуширина самой широкой его дороги и запас на скругление.
fn zone(drawn: &[&RoadLine], node: &SharedNode) -> f32 {
    node.visits
        .iter()
        .map(|visit| drawn[visit.road].width / 2.0)
        .fold(0.0_f32, f32::max)
        + CLUSTER_ZONE
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

/// Есть ли на плече, уходящем от узла в сторону `dir`, полосы к узлу:
/// односторонняя едет по ходу точек, к узлу — только с плеча к началу.
fn incoming(road: &RoadLine, dir: f32) -> bool {
    !road.oneway || dir < 0.0
}

/// Зебра поперёк `road` на длине `at`.
fn zebra_at(walk: &Walk, road: &RoadLine, at: f32, osm: bool) -> Option<Zebra> {
    let (point, direction) = walk.at(at)?;
    let across = direction.perp() * (road.width / 2.0 - EDGE_INSET);
    Some(Zebra {
        from: point - across,
        to: point + across,
        osm,
    })
}

/// Одна планка из зебр двух половин: от дальнего края одной до дальнего края
/// другой. Только если они легли на одну прямую ([`align_pair`]) и между ними
/// не больше [`JOIN_GAP`] — иначе это не пара через узкую разделительную.
fn join_zebras(a: &Zebra, b: &Zebra) -> Option<Zebra> {
    let across = (a.to - a.from).try_normalize()?;
    let theirs = (b.to - b.from).try_normalize()?;
    if across.dot(theirs).abs() < JOIN_PARALLEL
        || (b.from - a.from).perp_dot(across).abs() > JOIN_OFFSET
        || (b.to - a.from).perp_dot(across).abs() > JOIN_OFFSET
    {
        return None;
    }
    let ends = [a.from, a.to, b.from, b.to];
    let along = ends.map(|point| (point - a.from).dot(across));
    let (low, high) = along
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &at| {
            (low.min(at), high.max(at))
        });
    // зазор между половинами: вся длина минус обе планки
    let gap = high - low - a.from.distance(a.to) - b.from.distance(b.to);
    (gap <= JOIN_GAP).then(|| Zebra {
        from: a.from + across * low,
        to: a.from + across * high,
        osm: a.osm || b.osm,
    })
}

/// Стоп-линия на длине `at` поперёк полос, едущих к узлу с плеча `dir`:
/// у двусторонней — от оси до кромки по стороне движения, у односторонней —
/// во всю ширину.
fn stop_line_at(
    walk: &Walk,
    road: &RoadLine,
    at: f32,
    dir: f32,
    side: TrafficSide,
    yields: bool,
) -> Option<StopLine> {
    let (point, direction) = walk.at(at)?;
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

/// Зебры, что легли одна на другую — две ветки развилки у одного узла,
/// переход OSM рядом с правилом, — одной: по данным остаётся, из
/// сгенерированных — первая. Зебры двух половин одной улицы стоят бок о бок
/// и друг друга не задевают.
fn without_overlaps(zebras: Vec<Zebra>) -> Vec<Zebra> {
    let mut order: Vec<usize> = (0..zebras.len()).collect();
    order.sort_by_key(|&index| !zebras[index].osm);
    let bounds = |zebra: &Zebra| {
        let pad = ZEBRA_LENGTH / 2.0;
        (
            zebra.from.min(zebra.to) - pad,
            zebra.from.max(zebra.to) + pad,
        )
    };
    let mut kept: Vec<Zebra> = Vec::with_capacity(zebras.len());
    let mut near: Grid<usize> = Grid::new(CLUSTER_CELL);
    for index in order {
        let zebra = zebras[index];
        let (min, max) = bounds(&zebra);
        if !near
            .near_each(min, max)
            .any(|&other| overlaps(&zebra, &kept[other]) || overlaps(&kept[other], &zebra))
        {
            near.insert(min, max, kept.len());
            kept.push(zebra);
        }
    }
    kept
}

/// Задевает ли отрезок зебры `a` плашку зебры `b`.
fn overlaps(a: &Zebra, b: &Zebra) -> bool {
    let span = b.to - b.from;
    let Some(across) = span.try_normalize() else {
        return false;
    };
    let along = across.perp();
    let center = (b.from + b.to) / 2.0;
    let (half_span, half_length) = (span.length() / 2.0, ZEBRA_LENGTH / 2.0);
    (0..=8).any(|step| {
        let point = a.from.lerp(a.to, step as f32 / 8.0) - center;
        point.dot(across).abs() < half_span - OVERLAP_SLACK
            && point.dot(along).abs() < half_length - OVERLAP_SLACK
    })
}

/// Разрыв краски, что выходит за конец дороги, продолжается на дороге за ним:
/// OSM режет улицу на ways у светофора, у перехода, и зебра у самого конца
/// короткого way отрезала линии только на нём — двойная сплошная соседнего
/// начиналась вплотную к зебре (Первомайская в Туле, 3279, 2799). Продолжение —
/// единственная другая проезжая часть, чей конец в той же точке, и точка не
/// узел: на узле у каждой дороги свой разрыв, а ведущая линий не рвёт.
/// Остатки разрывов (`spills`) — концы и длины, что обрезал [`Walk::gap`].
fn spill_over_ends(
    drawn: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    junctions: &[(i32, i32)],
    spills: &[(Vec2, f32)],
    breaks: &mut [Vec<Break>],
) {
    if spills.is_empty() {
        return;
    }
    let ends = open_ends(drawn, paths);
    for &(at, reach) in spills {
        let key = node_key(at);
        if junctions.contains(&key) {
            continue;
        }
        let Some(roads) = ends.get(&key).filter(|roads| roads.len() == 2) else {
            continue;
        };
        // оба конца в точке — и та дорога, чей разрыв обрезан: продолжение —
        // вторая из двух
        for &road in roads {
            let path = paths[road].as_ref();
            let walk = Walk::new(path);
            let from = walk.project(at);
            let covered = breaks[road].iter().any(|found| {
                found.reach > 0.0 && (walk.project(found.at) - from).abs() <= found.reach + 1e-3
            });
            if !covered {
                breaks[road].push(Break { at, reach });
            }
        }
    }
}

/// Торцы дороги могут быть швом: проезжая часть, не кольцо и не точка.
fn has_open_ends(road: &RoadLine, path: &[Vec2]) -> bool {
    is_carriageway(road) && path.len() >= 2 && !is_ring(path)
}

/// Дороги с торцом в каждом узле — по [`has_open_ends`].
fn open_ends(drawn: &[&RoadLine], paths: &[impl AsRef<[Vec2]>]) -> HashMap<(i32, i32), Vec<usize>> {
    let mut ends: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (road, path) in paths.iter().enumerate() {
        let path = path.as_ref();
        if !has_open_ends(drawn[road], path) {
            continue;
        }
        for end in [path[0], path[path.len() - 1]] {
            ends.entry(node_key(end)).or_default().push(road);
        }
    }
    ends
}

/// Концы `[начало, конец]` каждой дороги, за которыми улица идёт дальше с
/// **меньшим** числом полос: чистый шов с одной проезжей частью, где линиям,
/// которых у соседа нет, продолжаться некуда — они гаснут в клине.
fn narrowing_ends(drawn: &[&RoadLine], paths: &[impl AsRef<[Vec2]>]) -> Vec<[bool; 2]> {
    let ends = open_ends(drawn, paths);
    paths
        .iter()
        .enumerate()
        .map(|(road, path)| {
            let path = path.as_ref();
            if !has_open_ends(drawn[road], path) {
                return [false; 2];
            }
            [path[0], path[path.len() - 1]].map(|end| {
                ends.get(&node_key(end)).is_some_and(|roads| {
                    roads.len() == 2
                        && roads
                            .iter()
                            .filter(|&&other| other != road)
                            .all(|&other| lane_count(drawn[other]) < lane_count(drawn[road]))
                })
            })
        })
        .collect()
}

/// Кусок линий между двумя разрывами короче [`MIN_RUN`] — тоже разрыв. Конец
/// пути, за которым улица сужается (`narrowing`, [`narrowing_ends`]), считается
/// таким же разрывом: у короткой улицы между узлом и клином оставался штрих в
/// пару метров у кромки (витрина 08).
fn bridge_short_runs(path: &[Vec2], breaks: &mut Vec<Break>, narrowing: [bool; 2]) {
    if path.len() < 2 {
        return;
    }
    let walk = Walk::new(path);
    let mut spans: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = walk.project(found.at);
            (at - found.reach, at + found.reach)
        })
        .collect();
    for (narrows, at) in narrowing.into_iter().zip([0.0, walk.total]) {
        if narrows {
            spans.push((at, at));
        }
    }
    if spans.len() < 2 {
        return;
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = spans[0].1;
    for span in &spans[1..] {
        if span.0 > reach && span.0 - reach < MIN_RUN {
            breaks.extend(walk.gap(reach, span.0));
        }
        reach = reach.max(span.1);
    }
}

#[cfg(test)]
mod tests;
