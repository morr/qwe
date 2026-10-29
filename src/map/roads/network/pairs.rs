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
//! (Красноармейский, Советская — отчёт автора). Но переход зазора не должен
//! переезжать перекрёсток: на стыке кусков у узла с чужой проезжей улицей
//! ось держится за OSM ([`PIN_STRAIGHT`]), и куски сходят на нет каждый на
//! своей стороне, не доехав до него, — иначе переход тянул ось через
//! перекрёсток, улица шла зигзагом мимо зебр, а узел уезжал с места (Тула,
//! Октябрьская у Максима Горького).
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

use super::RoadNodes;
use bevy::prelude::*;

use crate::map::along::{arclengths, nearest_on_path, place_on_path, simplify, tip_of};
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;
use crate::map::roads::tapers::{self, Tapers};

mod align;
mod search;

pub use self::search::pairable;

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

/// Торцы соседних разделительных ближе этого, м, сводятся в одну точку
/// ([`Pairs::join_ends`]).
const JOIN_GAP: f32 = 5.0;
/// У чистого шва половины (в узле только два её way) торцы сводятся и
/// дальше — до стольких метров друг от друга и от шва, м. Газон, меряный по
/// встречной половине, кончается в паре метров до шва своей, и у шва с клином,
/// где зазор держится до узла ([`seam_blend`]), между его торцом и асфальтовой
/// разделительной за швом выходило 5.07 м — дыра в землю (Тула, витрина 16).
/// Поднять сам [`JOIN_GAP`] нельзя: на шестилучевом узле Орла (витрина 04)
/// сошлись торцы разделительных через перекрёсток.
const SEAM_JOIN_GAP: f32 = 8.0;
/// За сколько метров до конца куска и до закреплённого узла разводка сходит
/// на нет.
pub const ALIGN_TRANSITION: f32 = 20.0;
/// Куски одной половины, между которыми дыра не длиннее этого, м, — один
/// участок разводки ([`Pairs::align`]): пара меняется на каждом шве
/// встречной половины, а у шва несколько проб соседа не находят (8 м на
/// Красноармейском). Сходи разводка на нет у каждого такого стыка, край
/// проспекта «дышал» бы через каждые 50–100 м.
const RUN_BRIDGE: f32 = 12.0;

/// Допуск, с которым разведённая ось и середина разделительной прореживаются
/// обратно, м: сдвиг ведётся по вершинам через [`ALIGN_STEP`] и
/// [`PROBE_STEP`], а на прямой их столько не нужно.
const SIMPLIFY_TOLERANCE: f32 = 0.03;

/// Самое широкое трамвайное полотно между кромками половин, м. Шире —
/// обособленное полотно на траве (Воздухофлотская в Туле, 10–24 м между
/// осями): там газон правдоподобен.
pub const TRAM_BED_MAX_GAP: f32 = 8.0;

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

impl Pairs {
    /// Разделительные половины `half` — заново по нарисованным осям `paths`:
    /// ось половины сдвинута после разводки (устье пары на перекрёстке,
    /// `merges::split_mouths`), и середина, положенная по старой оси, шла
    /// двойной сплошной поперёк полос.
    pub fn relay_medians(
        &mut self,
        half: usize,
        paths: &[Cow<[Vec2]>],
        roads: &[RoadLine],
        wedges: &Tapers,
    ) {
        for median in self
            .medians
            .iter_mut()
            .filter(|median| median.roads.contains(&half))
        {
            lay_median(median, paths, roads, wedges);
        }
    }

    /// Устье пары на перекрёстке (`merges::split_mouths`): половины `halves`
    /// идут рядом последние `lengths` метров до узла — у торца `at_end`
    /// (конец пути, иначе начало), пара слева, если `left`. Кусок пары там —
    /// тот же, что нашли бы пробы, если бы OSM не сводил оси в узел: без
    /// тротуара между половинами, зебры обеих в одну линию.
    pub fn join_mouth(
        &mut self,
        halves: [usize; 2],
        lengths: [f32; 2],
        totals: [f32; 2],
        at_end: [bool; 2],
        left: [bool; 2],
    ) {
        for side in 0..2 {
            let (road, partner) = (halves[side], halves[1 - side]);
            if self.runs[road].iter().any(|run| {
                run.partner == partner && {
                    let near = if at_end[side] {
                        totals[side] - run.to
                    } else {
                        run.from
                    };
                    near <= RUN_BRIDGE
                }
            }) {
                continue;
            }
            let length = lengths[side].min(totals[side]);
            let (from, to) = if at_end[side] {
                (totals[side] - length, totals[side])
            } else {
                (0.0, length)
            };
            let run = PairRun {
                from,
                to,
                partner,
                left: left[side],
                gap: 0.0,
                paved: true,
                tram: false,
            };
            let runs = &mut self.runs[road];
            let at = runs
                .iter()
                .position(|other| other.from > from)
                .unwrap_or(runs.len());
            runs.insert(at, run);
        }
    }
}

impl Pairs {
    /// Свести торцы соседних разделительных, лежащие ближе [`JOIN_GAP`], в
    /// одну точку.
    ///
    /// Половина из двух ways — две пары кусков и две разделительные: одна
    /// кончается последней пробой до шва, другая начинается у шва с другой
    /// стороны, и между ними оставалась пара метров — дыра в двойной сплошной
    /// и островок бордюра стоянки посреди бульвара «Макси». У чистого шва
    /// половины (`seams` — где он нарисован) — до [`SEAM_JOIN_GAP`].
    fn join_ends(&mut self, seams: &[Vec2]) {
        let tip = |median: &Median, end: bool| {
            let line = &median.midline;
            (line.len() >= 2).then(|| if end { line[line.len() - 1] } else { line[0] })
        };
        let joins_at = |at: Vec2, point: Vec2| {
            let apart = point.distance(at);
            apart < JOIN_GAP
                || (apart < SEAM_JOIN_GAP
                    && seams
                        .iter()
                        .any(|seam| seam.distance(at.midpoint(point)) < SEAM_JOIN_GAP))
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
                    .filter(|(.., point)| joins_at(at, *point))
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

    /// Куски бортика настила-половины: как [`Self::band_pieces`] с обеих
    /// сторон, но сторону пары снимают только куски с **мощёной**
    /// разделительной — она лежит на настиле (`medians::draw`). Между
    /// настилами над газоном (половины развязки в Туле, витрина 20) бортик
    /// остаётся: там его краем кончается мост.
    pub fn curb_pieces(&self, road: usize, stitch: f32, total: f32) -> Option<Vec<BandPiece>> {
        let paved: Vec<PairRun> = self.runs[road]
            .iter()
            .filter(|run| run.paved)
            .copied()
            .collect();
        band_pieces(&paved, [true; 2], stitch, total)
    }

    /// [`Self::beside`] по кускам пары с мощёной разделительной.
    pub fn beside_paved(&self, road: usize, at: f32, slack: f32) -> Option<bool> {
        self.runs[road]
            .iter()
            .filter(|run| run.paved)
            .find(|run| run.from - slack <= at && at <= run.to + slack)
            .map(|run| run.left)
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

/// Ключ точки по её точным битам: сведённый узел находят по вершине,
/// совпадающей до последнего бита.
fn point_bits(point: Vec2) -> [u32; 2] {
    [point.x.to_bits(), point.y.to_bits()]
}

/// Середина и внутренние кромки разделительной `median` по нарисованным осям
/// её половин `paths` — с клиньями `wedges` ([`facing_half`]).
fn lay_median(median: &mut Median, paths: &[Cow<[Vec2]>], roads: &[RoadLine], wedges: &Tapers) {
    let facing = |road: usize, at: f32, total: f32, left: bool| {
        facing_half(roads, wedges, road, at, total, left)
    };
    let [first, second] = median.roads;
    let (path, partner) = (paths[first].as_ref(), paths[second].as_ref());
    let totals = [polyline_length(path), polyline_length(partner)];
    median.midline.clear();
    median.inner = [Vec::new(), Vec::new()];
    let partner_along = arclengths(partner).0;
    for (along, at, heading) in samples(path)
        .into_iter()
        .filter(|(along, ..)| median.from <= *along && *along <= median.to)
    {
        let Some((near, near_along)) = nearest_on_path(partner, at) else {
            continue;
        };
        let across = (near - at).normalize_or_zero();
        let half_first = facing(first, along, totals[0], heading.perp_dot(across) > 0.0);
        let second_left = place_on_path(partner, &partner_along, near_along)
            .is_some_and(|(_, heading)| heading.perp_dot(at - near) > 0.0);
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
            // сечение шва у сужающего клина уже своего — встречный (шире
            // своего) здесь не учитывается: минимум его и не заметит
            let seam = roads[taper.seam].width.min(width);
            let share = (from_node / length).max(0.0);
            half = half.min((seam + (width - seam) * share) / 2.0);
        }
    }
    half
}

/// Точки оси с шагом [`PROBE_STEP`]: длина от начала, точка, направление.
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

#[cfg(test)]
mod tests;
