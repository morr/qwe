//! **Парные половины** — разделённая улица, которую OSM рисует двумя
//! встречными односторонними ways бок о бок.
//!
//! Каждая половина рисовалась своей улицей: своим тротуаром с обеих сторон,
//! своими кромками, и между ними оставалась полоса того, что лежит ниже, —
//! светлая нитка тротуара под зазором в полметра или газон с двумя
//! тротуарами посреди проспекта. А там, где картограф свёл половины теснее
//! их ширины, полотна ложились одно на другое, и линии полос одной половины
//! резали линии другой. Здесь пара находится один раз, на нарисованных осях
//! (`roads/axis.rs`), и её читают все, кто рисует улицу: заливка
//! разделительной, двойная сплошная, газон с бордюром, тротуар только с
//! внешней стороны.
//!
//! **Поиск** ([`Pairs::new`]) — так же, как искала разделительную большая
//! стоянка: от каждой точки оси с шагом [`PROBE_STEP`] — ближайшая другая
//! половина, идущая **навстречу** и **рядом** ([`PAIR_PARALLEL`],
//! [`PAIR_SKEW`] — продолжение той же дороги торец в торец соседом не
//! считается), того же класса ([`Highway`](crate::map::osm::Highway)) и не
//! дальше [`PAIR_MAX_GAP`] между кромками. Кусок короче [`PAIR_MIN`] — не
//! пара: так сходятся два съезда; кроме короткого way, идущего рядом с парой
//! почти целиком ([`PAIR_COVER`]) и продолжающего половину, у которой пара
//! уже нашлась ([`RunKind::Short`]). Сосед держится от пробы к пробе, пока он
//! почти так же близок ([`PARTNER_SLACK`]): у шва встречной половины иначе
//! перескакивал с одного её way на другой.
//!
//! **Общая ось** ([`Pairs::align`]). Зазор между половинами OSM гуляет — в
//! Туле на одной паре от наложения в метр до зазора в полтора. Половины
//! разводятся от середины между ними на постоянное расстояние: зазор куска —
//! его медиана, у асфальтовой разделительной — не у́же [`PAVED_MIN_GAP`].
//! Разводка сходит на нет за [`ALIGN_TRANSITION`] до конца куска: сдвиг в
//! полметра у конца куска читался бы ступенькой. Узел с чужой дорогой —
//! выездом из двора, поперечной улицей — едет вместе с половиной, а его
//! дороги идут следом ([`Pairs::follow_moved_nodes`]); `RoadNodes` узнаёт его
//! и по новому месту (по нему находят друг друга скругления бордюров,
//! разрывы разметки и стежки). Закреплён только узел, за которым дорога
//! пойти не может, — с чужой разводимой половиной, кольцом или мостом; там
//! разводка сходит на нет, а ось ещё и прямая на [`PIN_STRAIGHT`]: на гнутом
//! крае скругление бордюра не помещается. Стык с продолжением той же
//! половины, у которого пара тоже есть, концом куска не считается — там
//! разводка идёт насквозь. Не считается им и стык двух кусков одной
//! половины через дыру до [`RUN_BRIDGE`] — шов **встречной** половины, где
//! пара сменила way: куски сводятся в участок, и зазор переходит от одного к
//! другому за [`ALIGN_TRANSITION`]. Прежде разводка сходила на нет у каждого
//! такого шва, ось возвращалась к OSM на 40 м, и край проспекта, сведённого
//! картографом внахлёст, «дышал» на метр–полтора через каждые 50–100 м
//! (Красноармейский, Советская — отчёт автора).
//! Сдвигается только нарисованная ось: точки `RoadLine` читает навмеш.
//!
//! **Трамвайное полотно** ([`Median::carries_tram`]). Пара, в зазоре которой
//! на большей части пути лежит `railway=tram` (Советская в Туле: две
//! половины по две полосы и пути между ними в 5 м), — не газон, а асфальт с
//! рельсами: каждая половина едет по нему своей внутренней полосой. Такая
//! разделительная мощёная при любой ширине до [`TRAM_BED_MAX_GAP`]; шире —
//! обособленное полотно на траве, газон. Зазор у полотна — свой у каждого
//! куска, как у любой пары: общий на всю улицу (медиана цепочки) стягивал
//! половины там, где картограф развёл их шире, — между перекрёстками
//! проспект сужался на пару метров, у закреплённых узлов возвращался.
//! Ступеньки на швах нет и так: зазор на стыке кусков переходит плавно.

use std::borrow::Cow;

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::{RoadNetwork, RoadNodes};
use crate::map::along::{nearest_on_path, simplify, tip_of};
use crate::map::grid::Grid;
use crate::map::osm::model::{RailKind, RailLine, distance_to_segment, polyline_length};
use crate::map::osm::{RoadClass, RoadLine};
use crate::map::roads::junctions::node_key;
use crate::map::roads::smoothstep;
use crate::map::roads::tapers::{self, Tapers};

/// Шаг, которым ось ощупывается на соседа, м.
pub const PROBE_STEP: f32 = 2.0;
/// Самая узкая асфальтовая разделительная после разводки, м: двойная
/// сплошная шириной 0.45 м ложится на неё, не заходя на полосы. Половины,
/// наложенные одна на другую, расходятся до неё.
pub const PAVED_MIN_GAP: f32 = 0.5;
/// Самый широкий газон между половинами, м, при котором они ещё одна улица.
pub const PAIR_MAX_GAP: f32 = 15.0;
/// На сколько кромки половин могут заходить одна на другую, м, — на полосу.
/// Встречные половины не сливаются, как попутные полосы, так что наложение —
/// небрежность картографа: у Красноармейского проспекта оси трёхполосных
/// половин стоят в 9.5 м вместо 11, и разводка их расставляет.
pub const PAIR_OVERLAP: f32 = 3.3;
/// Кусок пары короче этого, м, — не пара: так сходятся два съезда.
pub const PAIR_MIN: f32 = 8.0;
/// Доля своей длины, которую короткий way должен пройти рядом с парой,
/// чтобы кусок короче [`PAIR_MIN`] всё же был парой ([`is_pair_run`]).
const PAIR_COVER: f32 = 0.75;
/// Косинус угла между встречными осями, при котором половины ещё идут рядом.
const PAIR_PARALLEL: f32 = 0.9;
/// Доля расстояния, на которую сосед смещён вдоль оси: больше — это торец
/// продолжения, а не бок соседа.
const PAIR_SKEW: f32 = 0.35;
/// Насколько проба может выйти за торец звена соседа, чтобы он ещё шёл рядом,
/// м: полторы пробы — шов или узел, а не продолжение торец в торец.
const END_OVERHANG: f32 = 3.0;
/// Насколько сосед прошлой пробы может быть дальше ближайшего, м, чтобы
/// остаться соседом ([`beside`]). У шва встречной половины её два way идут
/// торец в торец, и в полосе [`END_OVERHANG`] ближайшим через пробу
/// оказывался то один, то другой — пара рвалась на куски короче
/// [`PAIR_MIN`] и не находилась вовсе (Рязань, Первомайский у моста —
/// тротуары половин легли плиткой на всю разделительную).
const PARTNER_SLACK: f32 = 0.5;
/// Торцы соседних разделительных ближе этого, м, сводятся в одну точку
/// ([`Pairs::join_ends`]).
const JOIN_GAP: f32 = 5.0;
/// За сколько метров до конца куска и до закреплённого узла разводка сходит
/// на нет.
pub const ALIGN_TRANSITION: f32 = 20.0;
/// Куски одной половины, между которыми дыра не длиннее этого, м, — один
/// участок разводки ([`Pairs::align`]): пара меняется на каждом шве
/// встречной половины, а у шва несколько проб соседа не находят (8 м на
/// Красноармейском). Сходи разводка на нет у каждого такого стыка, край
/// проспекта «дышал» бы через каждые 50–100 м.
const RUN_BRIDGE: f32 = 12.0;
/// Сколько метров оси у закреплённого узла разводка не трогает вовсе: там
/// ложится скругление бордюра к поперечной улице (`roads/corners.rs`), а оно
/// кладётся только на прямой край — полуширина поперечного проспекта и
/// касательная дуги в 10 м. Переход начинается за этим участком.
const PIN_STRAIGHT: f32 = 16.0;
/// На каком расстоянии от шва двух половин, сведённых в одну точку, м, их
/// оси встают на общую касательную ([`Pairs::align`]).
const SEAM_TAIL: f32 = 0.5;
/// Излом на шве двух половин круче этого, рад, — не продолжение, торцы не
/// выравниваются.
const SEAM_MAX_BEND: f32 = 10.0 * std::f32::consts::PI / 180.0;
/// Сдвиг узла короче этого, м, — не сдвиг: дорогам узла идти некуда.
const MOVE_EPSILON: f32 = 0.01;
/// На каком расстоянии от сдвинутого узла, м, дорога, идущая за ним
/// ([`Pairs::follow_moved_nodes`]), возвращается на своё место.
const FOLLOW_FADE: f32 = 12.0;
/// Шаг вершин разводимой оси, м: на длинном прямом звене сдвиг одних его
/// концов не держал бы зазор посередине.
const ALIGN_STEP: f32 = 4.0;
/// Допуск, с которым разведённая ось и середина разделительной прореживаются
/// обратно, м: сдвиг ведётся по вершинам через [`ALIGN_STEP`] и
/// [`PROBE_STEP`], а на прямой их столько не нужно.
const SIMPLIFY_TOLERANCE: f32 = 0.03;
/// Ячейка сетки звеньев, м.
const CELL: f32 = 32.0;
/// Доля проб куска, у середины которых лежит трамвай, начиная с которой
/// разделительная — трамвайное полотно.
const TRAM_SHARE_MIN: f32 = 0.5;
/// Самое широкое трамвайное полотно между кромками половин, м. Шире —
/// обособленное полотно на траве (Воздухофлотская в Туле, 10–24 м между
/// осями): там газон правдоподобен.
pub const TRAM_BED_MAX_GAP: f32 = 8.0;
/// Путь засчитан у середины, если он не дальше этого от неё, даже когда
/// зазор между кромками уже, м: два пути в 3–4 м друг от друга.
const TRAM_REACH_MIN: f32 = 2.0;
/// Самый длинный кусок поперечной улицы между половинами одной пары, м: две
/// половины и самый широкий газон между ними. Такой кусок лежит в проёме
/// разделительной ([`Pairs::across_median`]), и тротуара у него нет.
const MEDIAN_CROSSING_MAX: f32 = 40.0;
/// Кусок тротуара короче этого, м, не кладётся ([`Pairs::band_pieces`]):
/// между кусками пары остаются обрезки в сантиметры.
const SIDEWALK_PIECE_MIN: f32 = 0.5;
/// Щель короче этого, м, — между двумя кусками пары с одной стороны или
/// между куском и концом дороги — остаётся без тротуара со стороны пары
/// ([`Pairs::band_pieces`]). Пробы теряют соседа за несколько метров до
/// узла, где половины сходятся, и у шва асфальтовой разделительной с
/// газонной (восемь метров на Советской в Туле, пример 16), и в этих метрах
/// тротуар половины светился со стороны пары светлым языком. Столько же
/// дотягивается и середина (`MEDIAN_EXTEND` в `roads/medians.rs`).
const PAIR_SIDE_REACH: f32 = 12.0;

/// Кусок полосы тротуара половины: от и до, м по оси ленты, и с каких сторон
/// `[слева, справа]` по ходу точек он есть ([`Pairs::band_pieces`]).
pub type BandPiece = (f32, f32, [bool; 2]);

/// Вторая половина разделённой улицы: её дорога и асфальт ли между ними — по
/// асфальтовой разделительной зебра идёт одной планкой через обе половины
/// (`roads/node_paint.rs`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Partner {
    pub road: usize,
    pub paved: bool,
}

/// Кусок половины, на котором рядом идёт её пара.
///
/// Поля закрыты: что половина рядом с парой, спрашивают у [`Pairs`]
/// ([`Pairs::beside`], [`Pairs::partners`], [`Pairs::band_pieces`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairRun {
    /// Длина по нарисованной оси половины, м: начало и конец куска.
    pub(super) from: f32,
    pub(super) to: f32,
    /// Индекс второй половины.
    pub(super) partner: usize,
    /// Пара лежит слева по ходу половины.
    pub(super) left: bool,
    /// Асфальта или газона между кромками после разводки, м.
    pub(super) gap: f32,
    /// Асфальт между половинами, а не газон ([`Median::is_paved`]).
    pub(super) paved: bool,
    /// Между половинами — трамвайное полотно ([`Median::carries_tram`]).
    pub(super) tram: bool,
}

#[cfg(test)]
impl PairRun {
    /// Кусок `from..to` с парой `partner` слева или справа — без трамвая.
    pub fn for_test(from: f32, to: f32, partner: usize, left: bool, gap: f32, paved: bool) -> Self {
        Self {
            from,
            to,
            partner,
            left,
            gap,
            paved,
            tram: false,
        }
    }

    /// Начало и конец куска, м.
    pub fn span(&self) -> [f32; 2] {
        [self.from, self.to]
    }

    /// Пара слева по ходу половины.
    pub fn is_left(&self) -> bool {
        self.left
    }
}

/// Разделительная пары: то, что лежит между половинами.
///
/// Поля закрыты: рисует её `roads/medians.rs` по запросам ниже, а
/// дотягивает до перекрёстка — [`Median::extend`].
#[derive(Debug, Clone, PartialEq)]
pub struct Median {
    /// Половины: та, по чьей оси она меряется (с меньшим индексом), и её пара.
    pub(super) roads: [usize; 2],
    /// Кусок первой половины, м.
    pub(super) from: f32,
    pub(super) to: f32,
    /// Асфальта или газона между кромками после разводки, м.
    pub(super) gap: f32,
    /// Асфальт между половинами, а не газон: зазор не шире ручки `Median gap`
    /// или трамвайное полотно.
    paved: bool,
    /// Трамвайное полотно.
    tram: bool,
    /// Середина между осями — по ходу первой половины. До [`Pairs::align`]
    /// пуста.
    pub(super) midline: Vec<Vec2>,
    /// Внутренние кромки половин в тех же точках, что и середина.
    pub(super) inner: [Vec<Vec2>; 2],
}

impl Median {
    /// Половины пары: первая — та, по чьей оси она меряется.
    pub fn roads(&self) -> [usize; 2] {
        self.roads
    }

    /// Асфальта или газона между кромками после разводки, м.
    pub fn gap(&self) -> f32 {
        self.gap
    }

    /// Середина между осями — по ходу первой половины.
    pub fn midline(&self) -> &[Vec2] {
        &self.midline
    }

    /// Внутренние кромки половин в тех же точках, что и [`Self::midline`].
    pub fn inner(&self) -> &[Vec<Vec2>; 2] {
        &self.inner
    }

    /// Продлить середину и обе кромки у торца (`end` — у конца) на `along`
    /// метров — каждую по направлению своего крайнего звена
    /// (`medians::reach_breaks`).
    pub fn extend(&mut self, end: bool, along: f32) {
        let [first, second] = &mut self.inner;
        for line in [&mut self.midline, first, second] {
            let Some((tip, heading)) = tip_of(line, end) else {
                continue;
            };
            let point = tip + heading * along;
            if end {
                line.push(point);
            } else {
                line.insert(0, point);
            }
        }
    }

    /// Асфальт между половинами, а не газон.
    pub fn is_paved(&self) -> bool {
        self.paved
    }

    /// Трамвайное полотно: на большей части куска у середины между
    /// половинами лежит `railway=tram`, а зазор не шире
    /// [`TRAM_BED_MAX_GAP`]. Всегда мощёное.
    pub fn carries_tram(&self) -> bool {
        self.tram
    }

    /// Газонная разделительная по середине и внутренним кромкам — тестам
    /// тех, кто её читает.
    #[cfg(test)]
    pub fn lawn_for_test(midline: Vec<Vec2>, inner: [Vec<Vec2>; 2]) -> Self {
        Self {
            roads: [0, 1],
            from: 0.0,
            to: crate::map::osm::model::polyline_length(&midline),
            gap: 0.0,
            paved: false,
            tram: false,
            midline,
            inner,
        }
    }

    /// Наибольшая ширина разделительной, м, — зазор между **внутренними
    /// кромками** половин, не между их осями: ширина полосы вдоль середины,
    /// которая кроет всё между половинами. Оси дальше друг от друга на
    /// полуширины обеих половин (`medians::crossing_breaks`).
    pub fn width(&self) -> f32 {
        self.midline
            .iter()
            .zip(&self.inner[0])
            .map(|(mid, inner)| 2.0 * mid.distance(*inner))
            .fold(0.0, f32::max)
    }
}

/// Парные половины карты.
#[derive(Debug, Clone, Default)]
pub struct Pairs {
    /// По каждой дороге — её куски с парой, по ходу.
    pub(super) runs: Vec<Vec<PairRun>>,
    /// По разделительной на каждую пару кусков — одну, а не с обеих сторон:
    /// две двойные линии, посчитанные от каждой половины, ложились бы одна на
    /// другую со сдвигом в сантиметры.
    pub(super) medians: Vec<Median>,
}

/// Может ли дорога быть половиной разделённой улицы: одностороннее полотно,
/// не кольцо и не арка. Проезд (`service`) — тоже: бульвар у ТРЦ
/// «Макси» размечен именно так, двумя встречными проездами, а пара ищется
/// только в своём классе. Проезд ряда стоянки — нет: встречные ряды
/// разделяют места, а не разделительная.
///
/// Мост — половина, как любая: без пары его оси стояли там, где их положил
/// картограф, а подходы с обеих сторон сходились к ним — проспект сужался у
/// каждого моста (Красноармейский над каналом: 9.5 м между осями вместо
/// 11.4). Навмеш режет мост по точкам OSM, а разводка раздвигает половины
/// наружу — проходимое остаётся внутри нарисованного настила. Арка — нет:
/// её стены — стены дома.
pub fn pairable(road: &RoadLine) -> bool {
    road.class == RoadClass::Street
        && !road.parking_aisle
        && road.oneway
        && !road.is_roundabout()
        && !road.passage
        && road.points.len() >= 2
}

/// Зазор, до которого разводится кусок с медианным зазором `gap`;
/// `median_gap` — самая широкая асфальтовая разделительная.
fn target_gap(gap: f32, median_gap: f32) -> f32 {
    if gap <= median_gap {
        gap.max(PAVED_MIN_GAP)
    } else {
        gap
    }
}

/// Точка оси, ощупанная на соседа.
#[derive(Clone)]
struct Probe {
    along: f32,
    at: Vec2,
    /// Сосед: дорога, ближайшая точка его оси, расстояние между осями.
    beside: Option<(usize, Vec2, f32)>,
}

impl Pairs {
    /// Пары среди `roads`, нарисованных по `paths`; `rails` — пути карты,
    /// по трамвайным из них находится полотно. Середины разделительных ещё
    /// пусты — их кладёт [`Pairs::align`].
    pub fn new(
        roads: &[RoadLine],
        paths: &[impl AsRef<[Vec2]>],
        median_gap: f32,
        rails: &[RailLine],
    ) -> Self {
        let mut pairs = Self {
            runs: vec![Vec::new(); roads.len()],
            medians: Vec::new(),
        };
        let tracks = Tracks::new(rails);
        let candidates: Vec<usize> = (0..roads.len())
            .filter(|&index| pairable(&roads[index]) && paths[index].as_ref().len() >= 2)
            .collect();
        if candidates.len() < 2 {
            return pairs;
        }
        let widest = candidates
            .iter()
            .map(|&index| roads[index].width)
            .fold(0.0, f32::max);
        let mut segments: Grid<(usize, usize)> = Grid::new(CELL);
        for &index in &candidates {
            for (at, pair) in paths[index].as_ref().windows(2).enumerate() {
                segments.insert_segment(pair[0], pair[1], 0.0, (index, at));
            }
        }
        // куски короче `PAIR_MIN`, почти во весь свой way — до второго прохода
        let mut short: Vec<(usize, usize, Vec<Probe>)> = Vec::new();
        for &index in &candidates {
            let path = paths[index].as_ref();
            let reach = (roads[index].width + widest) / 2.0 + PAIR_MAX_GAP;
            // сосед прошлой пробы держится, пока он почти так же близок
            // ([`PARTNER_SLACK`]): у шва встречной половины два её way стоят
            // рядом, и ближайший перескакивал с одного на другой через пробу
            let mut previous = None;
            let probes: Vec<Probe> = samples(path)
                .into_iter()
                .map(|(along, at, heading)| {
                    let near = beside(index, at, heading, reach, roads, paths, &segments, previous);
                    previous = near.map(|(partner, ..)| partner);
                    Probe {
                        along,
                        at,
                        beside: near,
                    }
                })
                .collect();
            let total = probes.last().map_or(0.0, |probe| probe.along);
            let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
            let mut start = 0;
            while start < probes.len() {
                let Some((partner, ..)) = probes[start].beside else {
                    start += 1;
                    continue;
                };
                let end = probes[start..]
                    .iter()
                    .position(|probe| probe.beside.map(|beside| beside.0) != Some(partner))
                    .map_or(probes.len(), |offset| start + offset);
                pieces.push((start, end, partner));
                start = end;
            }
            // длина куска меряется по цепочке, в которой сосед сменился без
            // пропуска: на шве встречной половины короткий way видит пару
            // половиной длины с одним её way и половиной — с другим, и ни
            // один кусок не дотягивал до пары (Красноармейский у моста: way в
            // 16 м — по 7.4 м с каждым), а половина сходилась к мосту
            for chain in pieces.chunk_by(|a, b| a.1 == b.0) {
                let span = probes[chain[chain.len() - 1].1 - 1].along - probes[chain[0].0].along;
                for &(start, end, partner) in chain {
                    let run = &probes[start..end];
                    // осколок у шва, где обе половины меняют way, — не кусок
                    if chain.len() > 1 && run[run.len() - 1].along - run[0].along < PAIR_MIN / 2.0
                    {
                        continue;
                    }
                    match run_kind(span, total) {
                        RunKind::Pair => {
                            pairs.push_run(index, partner, run, roads, median_gap, &tracks)
                        }
                        RunKind::Short => short.push((index, partner, run.to_vec())),
                        RunKind::None => {}
                    }
                }
            }
        }
        // короткий way, целиком идущий рядом с парой, — пара, если он
        // продолжает половину, у которой пара уже есть: у моста или газона
        // улицу режут на куски в десяток метров, а два сходящихся съезда
        // продолжением пары не бывают
        let paired_ends: HashSet<(i32, i32)> = (0..roads.len())
            .filter(|&index| !pairs.runs[index].is_empty())
            .flat_map(|index| {
                let path = paths[index].as_ref();
                [path[0], path[path.len() - 1]].map(node_key)
            })
            .collect();
        for (index, partner, run) in short {
            let path = paths[index].as_ref();
            if [path[0], path[path.len() - 1]]
                .iter()
                .any(|&end| paired_ends.contains(&node_key(end)))
            {
                pairs.push_run(index, partner, &run, roads, median_gap, &tracks);
            }
        }
        pairs
    }

    fn push_run(
        &mut self,
        index: usize,
        partner: usize,
        probes: &[Probe],
        roads: &[RoadLine],
        median_gap: f32,
        tracks: &Tracks,
    ) {
        let (first, last) = (&probes[0], &probes[probes.len() - 1]);
        let asphalt = (roads[index].width + roads[partner].width) / 2.0;
        let mut gaps: Vec<f32> = probes
            .iter()
            .filter_map(|probe| probe.beside.map(|(_, _, apart)| apart - asphalt))
            .collect();
        gaps.sort_by(f32::total_cmp);
        let raw = gaps[gaps.len() / 2];
        // трамвай у середины между осями, в пределах самого зазора
        let railed = probes
            .iter()
            .filter(|probe| {
                probe.beside.is_some_and(|(_, near, apart)| {
                    let reach = ((apart - asphalt) / 2.0).max(TRAM_REACH_MIN);
                    tracks.near(probe.at.midpoint(near), reach)
                })
            })
            .count();
        let tram = raw <= TRAM_BED_MAX_GAP && railed as f32 >= TRAM_SHARE_MIN * probes.len() as f32;
        // полотно — асфальт той ширины, что есть: половины расширяются до
        // середины (`roads/medians.rs`), и разводить их не к чему
        let gap = if tram {
            raw.max(PAVED_MIN_GAP)
        } else {
            target_gap(raw, median_gap)
        };
        let paved = tram || gap <= median_gap;
        let (_, near, _) = first.beside.expect("кусок пары — из точек с соседом");
        let heading = probes[1].at - first.at;
        self.runs[index].push(PairRun {
            from: first.along,
            to: last.along,
            partner,
            left: heading.perp_dot(near - first.at) > 0.0,
            gap,
            paved,
            tram,
        });
        if index < partner {
            self.medians.push(Median {
                roads: [index, partner],
                from: first.along,
                to: last.along,
                gap,
                paved,
                tram,
                midline: Vec::new(),
                inner: [Vec::new(), Vec::new()],
            });
        }
    }

    /// Развести половины на постоянный зазор — сдвигом нарисованных осей
    /// `paths` — и положить середины разделительных по разведённым осям.
    ///
    /// `wedges` — клинья у швов половин (`roads/tapers.rs`): на клине, который
    /// сужает сторону пары, полуширина к паре берётся суженной
    /// ([`facing_half`]). Иначе ось широкого way на шве стояла дальше от
    /// середины, чем ось узкого, — на полразницы ширин: у шва ступенька на
    /// внешней кромке и дыра в землю у внутренней (Тула, витрина 16).
    ///
    /// Отдаёт узлы с чужими дорогами, которые уехали вместе с половиной:
    /// `(точка OSM, куда нарисован)` — их дорогам идти следом
    /// ([`Self::follow_moved_nodes`]).
    pub fn align(
        &mut self,
        paths: &mut [Cow<[Vec2]>],
        roads: &[RoadLine],
        network: &RoadNetwork,
        nodes: &RoadNodes,
        wedges: &Tapers,
    ) -> Vec<(Vec2, Vec2)> {
        let facing = |road: usize, at: f32, total: f32, left: bool| {
            facing_half(roads, wedges, road, at, total, left)
        };
        let mut original: Vec<Option<Vec<Vec2>>> = vec![None; paths.len()];
        for run in self.runs.iter().flatten() {
            original[run.partner].get_or_insert_with(|| paths[run.partner].to_vec());
        }
        let original_lengths: Vec<f32> = original
            .iter()
            .map(|path| path.as_deref().map_or(0.0, polyline_length))
            .collect();
        let street = |road: usize| network.street_of(road).map(|(street, _)| street);
        let lengths: Vec<f32> = (0..paths.len())
            .map(|road| {
                if self.runs[road].is_empty() {
                    0.0
                } else {
                    polyline_length(&paths[road])
                }
            })
            .collect();
        // стык с продолжением той же половины, у которого пара у этого же
        // узла тоже есть, — не конец куска. «У узла» — ближе [`RUN_BRIDGE`]:
        // шов своей половины бывает и швом встречной, и у него несколько проб
        // пары не находят. Отдаёт зазор куска продолжения: к нему зазор
        // подходит через шов, без ступеньки. Узел — точка OSM, а не конец
        // нарисованной оси: сглаженная ось режется на ways в ближайшей к шву
        // точке дуги (`roads/axis.rs`), и по концу оси узел не находился —
        // разводка сходила на нет у каждого шва своей половины. Продолжение —
        // way той же улицы или соосный way за торцом ([`RoadNodes::next_way`]):
        // у перекрёстка улица сети кончается (Красноармейский — tertiary до
        // узла и primary после), а половина идёт дальше, и разводка, сходившая
        // на нет с обеих сторон, сужала проспект у каждого перекрёстка
        let continues = |road: usize, other: usize| {
            other != road
                && ((street(other).is_some() && street(other) == street(road))
                    || [0, 1]
                        .into_iter()
                        .any(|end| nodes.next_way(road, end).is_some_and(|(next, _)| next == other)))
        };
        let continued = |road: usize, end: bool| {
            let points = &roads[road].points;
            let node = if end {
                points[points.len() - 1]
            } else {
                points[0]
            };
            nodes.roads_at(node).iter().find_map(|&other| {
                if !continues(road, other) {
                    return None;
                }
                let points = &roads[other].points;
                self.runs[other]
                    .iter()
                    .find(|run| {
                        (points[0] == node && run.from <= RUN_BRIDGE)
                            || (points[points.len() - 1] == node
                                && run.to >= lengths[other] - RUN_BRIDGE)
                    })
                    .map(|run| run.gap)
            })
        };
        let mut aligned: Vec<(usize, Vec<Vec2>)> = Vec::new();
        let mut moved: Vec<(Vec2, Vec2)> = Vec::new();
        for (road, runs) in self.runs.iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            let path = &paths[road];
            let ends = [continued(road, false), continued(road, true)];
            let (mut dense, along) = densify(path, ALIGN_STEP);
            let total = along[along.len() - 1];
            let foreign = |other: usize| other != road && !continues(road, other);
            // узел с чужой дорогой едет вместе с половиной, а дорога идёт
            // следом (`Pairs::follow_moved_nodes`). Держит ось только тот, за
            // которым дорога пойти не может: другая разводимая половина,
            // кольцо или мост — у них своё место. Прежде закреплён был каждый
            // узел с проездом, и у каждого выезда из двора ось возвращалась
            // к OSM: кромка Красноармейского гуляла на метр–полтора через
            // каждые 50–100 м
            let held = |point: Vec2| {
                nodes.roads_at(point).iter().any(|&other| {
                    foreign(other)
                        && roads[other].class == RoadClass::Street
                        && (!self.runs[other].is_empty()
                            || roads[other].is_roundabout()
                            || roads[other].carves_navmesh())
                })
            };
            // узел на конце оси — точка OSM: сглаженная ось режется на ways
            // рядом с узлом, а не в нём (`continued` выше)
            let osm = &roads[road].points;
            let last = dense.len() - 1;
            let node_of = |index: usize| match index {
                0 => osm[0],
                _ if index == last => osm[osm.len() - 1],
                _ => dense[index],
            };
            let pinned: Vec<f32> = (0..dense.len())
                .filter(|&index| held(node_of(index)))
                .map(|index| along[index])
                .collect();
            // узлы запоминаются до сдвига: после него по вершине их не найти
            let shared: Vec<bool> = (0..dense.len())
                .map(|index| nodes.is_shared(node_of(index)))
                .collect();
            // и шов с продолжением: обе половины сдвигают его каждая сама, и
            // их концы расходились на сантиметры — узел по ним не находился
            let movable: Vec<Option<Vec2>> = (0..dense.len())
                .map(|index| {
                    let node = node_of(index);
                    (shared[index] && !held(node)).then_some(node)
                })
                .collect();
            // участок, у конца которого ось продолжает та же половина с
            // парой, тянется до самого конца оси и там не сходит на нет
            let spans: Vec<(&[PairRun], f32, f32)> = spans(runs)
                .into_iter()
                .map(|span| {
                    let (first, last) = (&span[0], &span[span.len() - 1]);
                    let from = if ends[0].is_some() && first.from <= RUN_BRIDGE {
                        f32::NEG_INFINITY
                    } else {
                        first.from
                    };
                    let to = if ends[1].is_some() && last.to >= total - RUN_BRIDGE {
                        f32::INFINITY
                    } else {
                        last.to
                    };
                    (span, from, to)
                })
                .collect();
            for (point, &at) in dense.iter_mut().zip(&along) {
                let Some(&(span, from, to)) = spans
                    .iter()
                    .find(|(_, from, to)| from - 1e-3 <= at && at <= to + 1e-3)
                else {
                    continue;
                };
                let (from_start, to_end) = (at - from, to - at);
                let to_pin = pinned
                    .iter()
                    .map(|pin| (pin - at).abs())
                    .fold(f32::INFINITY, f32::min)
                    - PIN_STRAIGHT;
                let weight = ease(from_start.min(to_end)) * ease(to_pin);
                if weight <= 0.0 {
                    continue;
                }
                // у шва встречной половины ближайшей бывает любая из двух её
                // ways — берётся та, что ближе
                let Some((run, near, near_along)) = span
                    .iter()
                    .filter(|run| run.from - RUN_BRIDGE <= at && at <= run.to + RUN_BRIDGE)
                    .filter_map(|run| {
                        let path = original[run.partner]
                            .as_deref()
                            .expect("ось пары сохранена до разводки");
                        nearest_on_path(path, *point).map(|(near, along)| (run, near, along))
                    })
                    .min_by(|a, b| a.1.distance(*point).total_cmp(&b.1.distance(*point)))
                else {
                    continue;
                };
                let partner = run.partner;
                let Some(outward) = (*point - near).try_normalize() else {
                    continue;
                };
                // через шов своей половины — к зазору продолжения: у самого
                // узла обе стороны берут середину между своими зазорами
                let mut gap = span_gap(span, at);
                if let (Some(before), true) = (ends[0], from == f32::NEG_INFINITY) {
                    gap = blend(before, gap, at);
                }
                if let (Some(after), true) = (ends[1], to == f32::INFINITY) {
                    gap = blend(gap, after, at - total);
                }
                // полуширины половин, обращённые друг к другу, — с клиньями
                let partner_path = original[partner].as_deref().unwrap_or_default();
                let partner_left = heading_at(partner_path, near_along)
                    .is_some_and(|heading| heading.perp_dot(*point - near) > 0.0);
                let asphalt = facing(road, at, total, run.left)
                    + facing(partner, near_along, original_lengths[partner], partner_left);
                let wanted = point.midpoint(near) + outward * (asphalt + gap) / 2.0;
                *point += (wanted - *point) * weight;
            }
            // узлы остаются вершинами: по их точному месту их находят соседи
            let kept = simplify(&dense, false, SIMPLIFY_TOLERANCE, |index| shared[index]);
            moved.extend(
                movable
                    .iter()
                    .zip(&dense)
                    .filter_map(|(from, &to)| from.map(|from| (from, to)))
                    .filter(|(from, to)| from.distance(*to) > MOVE_EPSILON),
            );
            aligned.push((road, kept.into_iter().map(|index| dense[index]).collect()));
        }
        // узел, сдвинутый концом половины и началом её продолжения, — в одном
        // месте, середине сдвигов: скругления и стежки ищут его по вершине
        let mut meets: HashMap<(i32, i32), (Vec2, f32)> = HashMap::new();
        for (from, to) in &moved {
            let meet = meets.entry(node_key(*from)).or_insert((Vec2::ZERO, 0.0));
            meet.0 += *to;
            meet.1 += 1.0;
        }
        let place = |from: Vec2| {
            let (sum, count) = meets[&node_key(from)];
            sum / count
        };
        let bits = |point: Vec2| [point.x.to_bits(), point.y.to_bits()];
        let met: HashMap<[u32; 2], Vec2> = moved
            .iter()
            .filter(|(from, _)| meets[&node_key(*from)].1 > 1.0)
            .map(|&(from, to)| (bits(to), place(from)))
            .collect();
        if !met.is_empty() {
            for point in aligned.iter_mut().flat_map(|(_, path)| path.iter_mut()) {
                if let Some(&place) = met.get(&bits(*point)) {
                    *point = place;
                }
            }
            align_seam_ends(&mut aligned, &met);
        }
        for (from, to) in &mut moved {
            *to = place(*from);
        }
        for (road, path) in aligned {
            paths[road] = Cow::Owned(path);
        }
        for median in &mut self.medians {
            let [first, second] = median.roads;
            let (path, partner) = (paths[first].as_ref(), paths[second].as_ref());
            let totals = [polyline_length(path), polyline_length(partner)];
            median.midline.clear();
            median.inner = [Vec::new(), Vec::new()];
            for (along, at, heading) in samples(path)
                .into_iter()
                .filter(|(along, ..)| median.from <= *along && *along <= median.to)
            {
                let Some((near, near_along)) = nearest_on_path(partner, at) else {
                    continue;
                };
                let across = (near - at).normalize_or_zero();
                let half_first = facing(first, along, totals[0], heading.perp_dot(across) > 0.0);
                let second_left = heading_at(partner, near_along)
                    .is_some_and(|heading| heading.perp_dot(at - near) > 0.0);
                let half_second = facing(second, near_along, totals[1], second_left);
                median.midline.push(at.midpoint(near));
                median.inner[0].push(at + across * half_first);
                median.inner[1].push(near - across * half_second);
            }
            // вершина остаётся, если она нужна хоть одной из трёх линий
            let mut kept = vec![false; median.midline.len()];
            for line in [&median.midline, &median.inner[0], &median.inner[1]] {
                for index in simplify(line, false, SIMPLIFY_TOLERANCE, |_| false) {
                    kept[index] = true;
                }
            }
            let thin = |line: &mut Vec<Vec2>| {
                let mut index = 0;
                line.retain(|_| {
                    index += 1;
                    kept[index - 1]
                });
            };
            thin(&mut median.midline);
            thin(&mut median.inner[0]);
            thin(&mut median.inner[1]);
        }
        self.join_ends();
        moved
    }

    /// Дороги узлов, уехавших с половиной ([`Self::align`]), идут следом:
    /// вершина узла — туда же, куда половина, соседние вершины — с затуханием
    /// на [`FOLLOW_FADE`], до следующего узла. Разводимые половины
    /// сдвинуты своей разводкой и не трогаются.
    pub fn follow_moved_nodes(
        &self,
        paths: &mut [Cow<[Vec2]>],
        nodes: &RoadNodes,
        moved: &[(Vec2, Vec2)],
    ) {
        let mut seen = HashSet::new();
        for &(from, to) in moved {
            let key = node_key(from);
            if !seen.insert(key) {
                continue;
            }
            let shift = to - from;
            for &road in nodes.roads_at(from) {
                if !self.runs[road].is_empty() {
                    continue;
                }
                let Some(vertex) = paths[road].iter().position(|point| node_key(*point) == key)
                else {
                    continue;
                };
                let original = paths[road].to_vec();
                let points = paths[road].to_mut();
                points[vertex] = to;
                let after = (vertex + 1..original.len()).collect::<Vec<_>>();
                let before = (0..vertex).rev().collect::<Vec<_>>();
                for side in [after, before] {
                    let (mut at, mut walked) = (vertex, 0.0);
                    for next in side {
                        walked += original[next].distance(original[at]);
                        if walked >= FOLLOW_FADE || nodes.is_shared(original[next]) {
                            break;
                        }
                        points[next] += shift * (1.0 - smoothstep(walked / FOLLOW_FADE));
                        at = next;
                    }
                }
            }
        }
    }

    /// Свести торцы соседних разделительных, лежащие ближе [`JOIN_GAP`], в
    /// одну точку.
    ///
    /// Половина из двух ways — две пары кусков и две разделительные: одна
    /// кончается последней пробой до шва, другая начинается у шва с другой
    /// стороны, и между ними оставалась пара метров — дыра в двойной сплошной
    /// и островок бордюра стоянки посреди бульвара «Макси».
    fn join_ends(&mut self) {
        let tip = |median: &Median, end: bool| {
            let line = &median.midline;
            (line.len() >= 2).then(|| if end { line[line.len() - 1] } else { line[0] })
        };
        let mut joins: Vec<(usize, bool, Vec2, [Vec2; 2])> = Vec::new();
        for (index, median) in self.medians.iter().enumerate() {
            for end in [false, true] {
                let Some(at) = tip(median, end) else {
                    continue;
                };
                let nearest = self
                    .medians
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .flat_map(|(other, median)| {
                        [false, true].map(|other_end| (other, other_end, tip(median, other_end)))
                    })
                    .filter_map(|(other, other_end, point)| Some((other, other_end, point?)))
                    .filter(|(.., point)| point.distance(at) < JOIN_GAP)
                    .min_by(|a, b| a.2.distance(at).total_cmp(&b.2.distance(at)));
                let Some((other, other_end, point)) = nearest else {
                    continue;
                };
                let pick = |line: &[Vec2]| {
                    if other_end {
                        line[line.len() - 1]
                    } else {
                        line[0]
                    }
                };
                let edge = |own: &[Vec2]| {
                    let own = if end { own[own.len() - 1] } else { own[0] };
                    // кромки у разделительных, посчитанных с разных половин,
                    // идут в разном порядке — берётся ближайшая
                    let theirs = [&self.medians[other].inner[0], &self.medians[other].inner[1]]
                        .map(|line| pick(line))
                        .into_iter()
                        .min_by(|a, b| a.distance(own).total_cmp(&b.distance(own)))
                        .unwrap_or(own);
                    own.midpoint(theirs)
                };
                joins.push((
                    index,
                    end,
                    at.midpoint(point),
                    [edge(&median.inner[0]), edge(&median.inner[1])],
                ));
            }
        }
        for (index, end, mid, [first, second]) in joins {
            let Median { midline, inner, .. } = &mut self.medians[index];
            let [inner_first, inner_second] = inner;
            for (line, point) in [(midline, mid), (inner_first, first), (inner_second, second)] {
                if end {
                    line.push(point);
                } else {
                    line.insert(0, point);
                }
            }
        }
    }

    /// Разделительных с асфальтом, с газоном и из них трамвайных полотен.
    pub fn count(&self) -> [usize; 3] {
        let count = |test: fn(&Median) -> bool| self.medians.iter().filter(|m| test(m)).count();
        let paved = count(Median::is_paved);
        [
            paved,
            self.medians.len() - paved,
            count(Median::carries_tram),
        ]
    }

    /// Разделительные пар — по одной на пару кусков.
    pub fn medians(&self) -> &[Median] {
        &self.medians
    }

    /// Есть ли у дороги хоть один кусок пары — половина ли она где-нибудь.
    pub fn has_runs(&self, road: usize) -> bool {
        !self.runs[road].is_empty()
    }

    /// Пары без разделительных: куски `runs` по каждой дороге, как их
    /// нашёл бы [`Self::new`].
    #[cfg(test)]
    pub fn of_runs(runs: Vec<Vec<PairRun>>) -> Self {
        Self {
            runs,
            medians: Vec::new(),
        }
    }

    /// Куски пары на дороге `road` — вместо найденных.
    #[cfg(test)]
    pub fn set_runs(&mut self, road: usize, runs: Vec<PairRun>) {
        self.runs[road] = runs;
    }

    /// Куски пары на дороге `road`, по ходу.
    #[cfg(test)]
    pub fn runs(&self, road: usize) -> &[PairRun] {
        &self.runs[road]
    }

    /// Вторые половины разделённой улицы у дороги — по её кускам пары.
    pub fn partners(&self, road: usize) -> impl Iterator<Item = Partner> + '_ {
        self.runs[road].iter().map(|run| Partner {
            road: run.partner,
            paved: run.paved,
        })
    }

    /// Есть ли у половины `half` кусок пары с дорогой `other`.
    pub fn is_paired(&self, half: usize, other: usize) -> bool {
        self.runs[half].iter().any(|run| run.partner == other)
    }

    /// Лежит ли на длине `at` по узловой оси дороги рядом вторая половина и
    /// слева ли она: с её стороны у половины нет ни тротуара, ни кромки.
    /// `slack` — на сколько метров `at` может выйти за кусок: кусок кончается
    /// там, где пробы перестали находить пару, а скругление у узла стоит
    /// дальше. Первый кусок, в который `at` попал, — ответ.
    pub fn beside(&self, road: usize, at: f32, slack: f32) -> Option<bool> {
        self.runs[road]
            .iter()
            .find(|run| run.from - slack <= at && at <= run.to + slack)
            .map(|run| run.left)
    }

    /// Дорога — кусок поперечной улицы в проёме разделительной: короче
    /// [`MEDIAN_CROSSING_MAX`] и соединяет узлом `path[0]` одну половину
    /// пары, а узлом конца — её пару. `path` — узловая ось дороги; её торцы —
    /// точки OSM, и ось их не двигает.
    pub fn across_median(&self, road: usize, path: &[Vec2], nodes: &RoadNodes) -> bool {
        let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
            return false;
        };
        polyline_length(path) < MEDIAN_CROSSING_MAX
            && nodes.roads_at(start).iter().any(|&half| {
                half != road
                    && self.runs[half].iter().any(|run| {
                        run.partner != road && nodes.roads_at(end).contains(&run.partner)
                    })
            })
    }

    /// Куски полосы тротуара дороги длиной `total` по оси ленты: с тех
    /// сторон, где он есть по тегу (`sides`, `RoadLine::sidewalks`), и **без
    /// стороны пары** на её кусках. Куски пары меряны по узловой оси;
    /// `stitch` — длина стежка перед её началом на ленте (со знаком: у
    /// клиновой половины тело начинается за клином). В щели короче
    /// [`PAIR_SIDE_REACH`] между двумя кусками с одной стороны — любыми,
    /// полотном и газоном тоже, — и между крайним куском и концом дороги
    /// тротуара с той стороны тоже нет: светлое пятно лежало между ними.
    /// Обрезки короче [`SIDEWALK_PIECE_MIN`] и куски без
    /// сторон пропущены. `None` — полоса целиком, с обеих сторон, резать
    /// нечего.
    pub fn band_pieces(
        &self,
        road: usize,
        sides: [bool; 2],
        stitch: f32,
        total: f32,
    ) -> Option<Vec<BandPiece>> {
        band_pieces(&self.runs[road], sides, stitch, total)
    }
}

/// [`Pairs::band_pieces`] по кускам пары `runs` одной дороги.
fn band_pieces(
    runs: &[PairRun],
    sides: [bool; 2],
    stitch: f32,
    total: f32,
) -> Option<Vec<BandPiece>> {
    if runs.is_empty() && sides == [true; 2] {
        return None;
    }
    let mut pieces = Vec::new();
    let mut piece = |from: f32, to: f32, sides: [bool; 2]| {
        if to - from >= SIDEWALK_PIECE_MIN && sides != [false; 2] {
            pieces.push((from, to, sides));
        }
    };
    let mut cursor = 0.0;
    let mut previous: Option<bool> = None;
    for run in runs {
        let from = (run.from + stitch).clamp(cursor, total);
        let to = (run.to + stitch).clamp(from, total);
        // со стороны пары тротуара нет
        let mut paired = sides;
        paired[usize::from(!run.left)] = false;
        // и в щели до предыдущего куска с той же стороны, и от начала
        // дороги, если кусок начался у самого узла
        let bridged = match previous {
            Some(left) => left == run.left && from - cursor < PAIR_SIDE_REACH,
            None => from < PAIR_SIDE_REACH,
        };
        piece(cursor, from, if bridged { paired } else { sides });
        piece(from, to, paired);
        cursor = to;
        previous = Some(run.left);
    }
    // и до конца дороги, если последний кусок кончился у самого узла
    let tail = match (previous, runs.last()) {
        (Some(left), Some(_)) if total - cursor < PAIR_SIDE_REACH => {
            let mut paired = sides;
            paired[usize::from(!left)] = false;
            paired
        }
        _ => sides,
    };
    piece(cursor, total, tail);
    Some(pieces)
}

/// Звенья трамвайных путей карты — сеткой, чтобы пробы пар не перебирали
/// все пути города.
struct Tracks<'a> {
    rails: &'a [RailLine],
    links: Grid<(usize, usize)>,
}

impl<'a> Tracks<'a> {
    fn new(rails: &'a [RailLine]) -> Self {
        let mut links = Grid::new(CELL);
        for (index, rail) in rails.iter().enumerate() {
            if rail.kind != RailKind::Tram {
                continue;
            }
            for (at, pair) in rail.points.windows(2).enumerate() {
                links.insert_segment(pair[0], pair[1], 0.0, (index, at));
            }
        }
        Self { rails, links }
    }

    /// Лежит ли трамвайный путь не дальше `reach` от точки.
    fn near(&self, at: Vec2, reach: f32) -> bool {
        self.links
            .near_each(at - reach, at + reach)
            .any(|&(rail, link)| {
                let points = &self.rails[rail].points;
                distance_to_segment(at, points[link], points[link + 1]) <= reach
            })
    }
}

/// Торцы двух половин, сведённых в одну точку `met`, — на общую касательную:
/// у каждой вершина в [`SEAM_TAIL`] от шва по биссектрисе их направлений.
/// Ленты кончаются торцом поперёк своей оси, и при изломе в пару градусов
/// (картограф свёл мост и подход не по прямой — Красноармейский над каналом)
/// угол бордюра настила заходил на асфальт подхода зубцом; излом уходит
/// внутрь лент, где его кроют их соединения.
fn align_seam_ends(aligned: &mut [(usize, Vec<Vec2>)], met: &HashMap<[u32; 2], Vec2>) {
    let bits = |point: Vec2| [point.x.to_bits(), point.y.to_bits()];
    let places: HashSet<[u32; 2]> = met.values().map(|place| bits(*place)).collect();
    let mut at_place: HashMap<[u32; 2], Vec<(usize, bool)>> = HashMap::new();
    for (slot, (_, path)) in aligned.iter().enumerate() {
        if path.len() < 2 {
            continue;
        }
        for end in [false, true] {
            let point = if end { path[path.len() - 1] } else { path[0] };
            if places.contains(&bits(point)) {
                at_place.entry(bits(point)).or_default().push((slot, end));
            }
        }
    }
    // направление от шва вдоль оси
    let away = |path: &[Vec2], end: bool| {
        let (node, next) = if end {
            (path[path.len() - 1], path[path.len() - 2])
        } else {
            (path[0], path[1])
        };
        (next - node).normalize_or_zero()
    };
    for ends in at_place.into_values() {
        let [(first, first_end), (second, second_end)] = ends[..] else {
            continue;
        };
        let (a, b) = (
            away(&aligned[first].1, first_end),
            away(&aligned[second].1, second_end),
        );
        if a.dot(-b) < SEAM_MAX_BEND.cos() {
            continue;
        }
        let Some(tangent) = (a - b).try_normalize() else {
            continue;
        };
        for (slot, end, direction) in [(first, first_end, tangent), (second, second_end, -tangent)] {
            let path = &mut aligned[slot].1;
            let (node, neighbour, at) = if end {
                (path[path.len() - 1], path[path.len() - 2], path.len() - 1)
            } else {
                (path[0], path[1], 1)
            };
            // соседняя вершина может быть узлом — не трогается, новая встаёт
            // между ней и швом
            let reach = SEAM_TAIL.min(neighbour.distance(node) / 2.0);
            path.insert(at, node + direction * reach);
        }
    }
}

/// Плавный переход разводки: 0 у конца куска или у узла, 1 за
/// [`ALIGN_TRANSITION`] от них.
fn ease(distance: f32) -> f32 {
    smoothstep(distance / ALIGN_TRANSITION)
}

/// Куски половины (по ходу), сведённые в участки разводки: соседние, между
/// которыми не больше [`RUN_BRIDGE`], — один участок.
fn spans(runs: &[PairRun]) -> Vec<&[PairRun]> {
    let mut spans = Vec::new();
    let mut start = 0;
    for index in 1..=runs.len() {
        if index == runs.len() || runs[index].from - runs[index - 1].to > RUN_BRIDGE {
            spans.push(&runs[start..index]);
            start = index;
        }
    }
    spans
}

/// Зазор участка в точке `at`: у каждого куска — свой, а на стыке двух он
/// переходит от одного к другому за [`ALIGN_TRANSITION`], без ступеньки.
fn span_gap(span: &[PairRun], at: f32) -> f32 {
    span.windows(2).fold(span[0].gap, |gap, pair| {
        blend(gap, pair[1].gap, at - (pair[0].to + pair[1].from) / 2.0)
    })
}

/// Зазор `before` до шва и `after` за ним в `beyond` м от шва (за ним — со
/// знаком плюс): переход за [`ALIGN_TRANSITION`], на самом шве — середина.
fn blend(before: f32, after: f32, beyond: f32) -> f32 {
    before + (after - before) * smoothstep(beyond / ALIGN_TRANSITION + 0.5)
}

/// Что за кусок проб, чья цепочка длиной `span`, у половины длиной `total`.
enum RunKind {
    /// Не короче [`PAIR_MIN`] — пара.
    Pair,
    /// Короче, но почти весь way ([`PAIR_COVER`]) и не короче половины
    /// [`PAIR_MIN`]: пара, если way продолжает половину, у которой пара уже
    /// есть. Way в девять метров между газоном и мостом (Рязань,
    /// Первомайский) идёт рядом с парой целиком — и без пары клал тротуар
    /// плиткой в разделительную.
    Short,
    /// Два съезда сходятся — не пара.
    None,
}

fn run_kind(span: f32, total: f32) -> RunKind {
    if span >= PAIR_MIN {
        RunKind::Pair
    } else if span >= PAIR_MIN / 2.0 && span >= PAIR_COVER * total {
        RunKind::Short
    } else {
        RunKind::None
    }
}

/// Ближайшая половина рядом с точкой `at` оси дороги `index`, идущей по
/// `heading`, — или никакой. Сосед прошлой пробы `prefer` остаётся, если он
/// дальше ближайшего не больше чем на [`PARTNER_SLACK`].
#[allow(clippy::too_many_arguments)]
fn beside(
    index: usize,
    at: Vec2,
    heading: Vec2,
    reach: f32,
    roads: &[RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    segments: &Grid<(usize, usize)>,
    prefer: Option<usize>,
) -> Option<(usize, Vec2, f32)> {
    let own = &roads[index];
    let mut best: Option<(usize, Vec2, f32)> = None;
    let mut preferred: Option<(usize, Vec2, f32)> = None;
    for &(other, segment) in segments.near_each(at - reach, at + reach) {
        if other == index || roads[other].highway != own.highway {
            continue;
        }
        let path = paths[other].as_ref();
        let (from, to) = (path[segment], path[segment + 1]);
        let Some(other_heading) = (to - from).try_normalize() else {
            continue;
        };
        if heading.dot(other_heading) > -PAIR_PARALLEL {
            continue;
        }
        // основание перпендикуляра на прямой звена, а не ближайшая точка
        // отрезка: у торца соседа ближайшей была бы сама его точка, смещённая
        // вдоль оси, и пара терялась за несколько метров до узла или шва —
        // двойная сплошная не доходила до перекрёстка (отчёт автора). За торец
        // звена основание уходит не дальше [`END_OVERHANG`]
        let length = from.distance(to);
        let along = (at - from).dot(other_heading);
        if along < -END_OVERHANG || along > length + END_OVERHANG {
            continue;
        }
        let near = from + other_heading * along;
        let apart = near - at;
        let distance = apart.length();
        let gap = distance - (own.width + roads[other].width) / 2.0;
        if !(-PAIR_OVERLAP..=PAIR_MAX_GAP).contains(&gap)
            || apart.dot(heading).abs() > PAIR_SKEW * distance
        {
            continue;
        }
        if Some(other) == prefer && preferred.is_none_or(|(_, _, closest)| distance < closest) {
            preferred = Some((other, near, distance));
        }
        if best.is_some_and(|(_, _, closest)| closest <= distance) {
            continue;
        }
        best = Some((other, near, distance));
    }
    match (preferred, best) {
        (Some(kept), Some((_, _, closest))) if kept.2 <= closest + PARTNER_SLACK => Some(kept),
        _ => best,
    }
}

/// Точки оси с шагом [`PROBE_STEP`]: длина от начала, точка, направление.
/// Полуширина половины `road` в `at` метрах от начала её оси длиной `total`
/// со стороны пары (`left` — пара слева по ходу way): у клина, сужающего эту
/// сторону, — суженная, как её рисует клин (от узкого соседа у шва к своей
/// ширине на длине клина, `tapers::fit`); иначе — половина ширины.
fn facing_half(
    roads: &[RoadLine],
    wedges: &Tapers,
    road: usize,
    at: f32,
    total: f32,
    left: bool,
) -> f32 {
    let width = roads[road].width;
    let ends = wedges.at(road);
    let lengths = tapers::fit(total, ends.map(|end| end.map(|taper| taper.length)));
    let mut half = width / 2.0;
    for (end, (taper, length)) in ends.iter().zip(lengths).enumerate() {
        let (Some(taper), Some(length)) = (taper, length) else {
            continue;
        };
        if !taper.sides[usize::from(!left)] {
            continue;
        }
        let from_node = if end == 0 { at } else { total - at };
        if from_node < length {
            let narrow = roads[taper.narrow].width.min(width);
            let share = (from_node / length).max(0.0);
            half = half.min((narrow + (width - narrow) * share) / 2.0);
        }
    }
    half
}

/// Направление звена ломаной на дуговой координате `at`.
fn heading_at(path: &[Vec2], at: f32) -> Option<Vec2> {
    let mut run = 0.0;
    let mut last = None;
    for link in path.windows(2) {
        let length = link[0].distance(link[1]);
        let heading = (link[1] - link[0]).try_normalize();
        if heading.is_some() {
            last = heading;
        }
        run += length;
        if run >= at && last.is_some() {
            return last;
        }
    }
    last
}

pub(in crate::map::roads) fn samples(path: &[Vec2]) -> Vec<(f32, Vec2, Vec2)> {
    let mut points = Vec::new();
    let mut start = 0.0;
    for pair in path.windows(2) {
        let length = pair[0].distance(pair[1]);
        let Some(heading) = (pair[1] - pair[0]).try_normalize() else {
            continue;
        };
        let steps = (length / PROBE_STEP).ceil().max(1.0) as usize;
        let from = usize::from(!points.is_empty());
        for step in from..=steps {
            let t = step as f32 / steps as f32;
            points.push((start + length * t, pair[0].lerp(pair[1], t), heading));
        }
        start += length;
    }
    points
}

/// Ломаная с вершинами не реже `step` и длина дуги в каждой вершине.
/// Исходные вершины остаются на месте.
///
/// Точки — те же, что у `along::densify`, но **длины — нет**, и потому это
/// своя функция, а не `along::densify` + `along::arclengths`. Здесь длина —
/// `start + length·t` по звену OSM, там — сумма расстояний между соседними
/// догущёнными точками; разница в последних битах, но выравнивание половин
/// на неё отзывается: замена сдвинула асфальт, тротуары и газоны
/// разделительных на всех четырёх проверенных городах (Берлин — +38
/// вершин дорожных слоёв).
fn densify(path: &[Vec2], step: f32) -> (Vec<Vec2>, Vec<f32>) {
    let mut points = vec![path[0]];
    let mut along = vec![0.0];
    let mut start = 0.0;
    for pair in path.windows(2) {
        let length = pair[0].distance(pair[1]);
        let steps = (length / step).ceil().max(1.0) as usize;
        for index in 1..=steps {
            let t = index as f32 / steps as f32;
            // исходная вершина — как есть, а не `lerp`: по её точному месту
            // узел находят его соседи
            points.push(if index == steps {
                pair[1]
            } else {
                pair[0].lerp(pair[1], t)
            });
            along.push(start + length * t);
        }
        start += length;
    }
    (points, along)
}

#[cfg(test)]
mod tests;
