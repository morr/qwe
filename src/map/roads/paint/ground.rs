//! Поперечная краска, подогнанная по асфальту улиц: зебра из целых звеньев
//! и стоп-линия по кромке.

use bevy::prelude::*;

use super::{RUT_OFFSET, ZEBRA_FILL, ZEBRA_PERIOD};
use crate::map::grid::Grid;
use crate::map::roads::node_paint::{STOP_WIDTH, StopLine, ZEBRA_LENGTH, Zebra};
use crate::map::roads::turns::JunctionWear;

/// Звено зебры поперёк дороги, м: полоса краски периода [`ZEBRA_PERIOD`].
pub(super) const ZEBRA_BAR: f32 = ZEBRA_PERIOD * ZEBRA_FILL;

/// Шаг, которым торец поперечной краски отступает от кромки внутрь, пока
/// сечение не ляжет на асфальт целиком, м.
pub(super) const FIT_STEP: f32 = 0.05;
/// Насколько торец зебры может вырасти наружу от своей номинальной ширины,
/// м: у устья кромка — веер угла — шире полуширины дороги, и планка
/// дорастает до настоящей кромки (R37-03). Дальше — уже поле перекрёстка, а
/// не проезжая часть перехода.
const ZEBRA_REACH: f32 = 3.0;

/// Сечение поперечной краски на длине `at` от её начала вдоль `across`:
/// середина и оба края — по `half` поперёк — лежат на асфальте
/// ([`whole_bars`], [`trim_stop_line`]).
fn cross_section_fits(
    from: Vec2,
    across: Vec2,
    half: f32,
    on_asphalt: impl Fn(Vec2) -> bool,
) -> impl Fn(f32) -> bool {
    let along = across.perp() * half;
    move |at| {
        let point = from + across * at;
        [point - along, point, point + along]
            .into_iter()
            .all(&on_asphalt)
    }
}

/// Зебра из **целых звеньев** (roads list R2): звено рисуется только
/// целиком, лишнее не рисуется, а оставшиеся стоят по середине проезжей
/// части с равными полями. Планка по ширине дороги кончалась на дробной доле
/// периода, и крайнее звено выходило тонким клином у кромки (Тула, ул.
/// Фёдора Смирнова, 3396 3839); у развилки, где кромка идёт наискось через
/// четыре метра зебры, звено вылезало на тротуар (Болдина, 2527 2470).
///
/// Торцы планки сначала дорастают наружу — у устья и на клине ширины асфальт
/// шире номинальной полуширины дороги, и до починки планка не доставала до
/// кромки по звену и больше (R37-03: Фёдора Смирнова × Красноармейский,
/// кольцо Болдина, 1–3 м) — потом отступают внутрь, пока сечение поперёк
/// неё — оба края зебры вдоль дороги и середина — не ляжет на асфальт
/// (`on_asphalt`) целиком, так что скос кромки учтён; в получившийся отрезок
/// входит целое число звеньев, и он садится по его середине. Середину
/// планки не проверяет: разделительную пары или островок она проходит, как
/// проходила. `None` — не влезло ни одно звено.
pub(in crate::map::roads) fn whole_bars(
    zebra: &Zebra,
    on_asphalt: impl Fn(Vec2) -> bool,
) -> Option<Zebra> {
    let width = zebra.from.distance(zebra.to);
    let across = (zebra.to - zebra.from).try_normalize()?;
    let fits = cross_section_fits(zebra.from, across, ZEBRA_LENGTH / 2.0, on_asphalt);
    let middle = width / 2.0;
    let mut low = 0.0;
    while low > -ZEBRA_REACH && fits(low - FIT_STEP) {
        low -= FIT_STEP;
    }
    while low < middle && !fits(low) {
        low += FIT_STEP;
    }
    let mut high = width;
    while high < width + ZEBRA_REACH && fits(high + FIT_STEP) {
        high += FIT_STEP;
    }
    while high > middle && !fits(high) {
        high -= FIT_STEP;
    }
    let span = high - low;
    if span < ZEBRA_BAR {
        return None;
    }
    let bars = ((span - ZEBRA_BAR) / ZEBRA_PERIOD).floor() + 1.0;
    let used = (bars - 1.0) * ZEBRA_PERIOD + ZEBRA_BAR;
    let from = zebra.from + across * (low + (span - used) / 2.0);
    Some(Zebra {
        from,
        to: from + across * used,
        ..*zebra
    })
}

/// Проезжая часть под поперечной краской для [`whole_bars`] и
/// [`trim_stop_line`]: лежит ли точка на асфальте улиц — в треугольниках уже
/// собранного слоя `roads`. Не ленты по оси с полушириной: у торца под клин,
/// у рампы слияния и у сведённого торца лента уже, чем полуширина дороги, и
/// у развилки Болдиной звено по такой оценке ложилось на тротуар. Собирается
/// только около мест краски: треугольников в городе сотни тысяч, зебр и
/// стоп-линий — сотни.
pub(in crate::map::roads) struct PaintGround {
    triangles: Vec<[Vec2; 3]>,
    near: Grid<usize>,
}

impl PaintGround {
    pub(in crate::map::roads) fn new(
        zebras: &[Zebra],
        stops: &[StopLine],
        asphalt: impl IntoIterator<Item = [Vec2; 3]>,
    ) -> Self {
        let mut at_paint: Grid<()> = Grid::new(GROUND_CELL);
        for zebra in zebras {
            at_paint.insert_segment(zebra.from, zebra.to, ZEBRA_LENGTH, ());
        }
        for line in stops {
            at_paint.insert_segment(line.from, line.to, STOP_WIDTH, ());
        }
        Self::filtered(at_paint, asphalt)
    }

    /// Асфальт под колеёй траекторий: не весь (полный — это весь меш улиц,
    /// дорого), а треугольники у звеньев кривых и хвостов, с запасом на
    /// боковые колеи ([`RUT_OFFSET`]) — тот же отбор, что у зебр и
    /// стоп-линий, только «краска» здесь сама колея. Узел без зебры колею
    /// всё равно несёт, и её клип обязан видеть асфальт под ней.
    pub(in crate::map::roads) fn under_wear(
        junctions: &[JunctionWear],
        asphalt: impl IntoIterator<Item = [Vec2; 3]>,
    ) -> Self {
        // ячейка мельче `GROUND_CELL`: у колеи ячейку делят сотни
        // треугольников развязки, и каждый из шести запросов звена
        // перебирает их все — на Туле это были десятки миллисекунд
        let cell = 8.0;
        let reach = RUT_OFFSET + 1.0;
        let mut at_wear: Grid<()> = Grid::new(cell);
        for junction in junctions {
            // бокс кривой целиком, а не по звену: запросы клипа — точки у
            // звеньев, бокс с запасом накрывает их все, а вставок в сеть
            // в двадцать раз меньше
            for line in &junction.curves {
                let mut min = line[0];
                let mut max = line[0];
                for point in &line[1..] {
                    min = min.min(*point);
                    max = max.max(*point);
                }
                at_wear.insert(min - reach, max + reach, ());
            }
            for &[from, to] in &junction.tails {
                at_wear.insert(from.min(to) - reach, from.max(to) + reach, ());
            }
        }
        Self::filtered_in(cell, at_wear, asphalt)
    }

    fn filtered(at_paint: Grid<()>, asphalt: impl IntoIterator<Item = [Vec2; 3]>) -> Self {
        Self::filtered_in(GROUND_CELL, at_paint, asphalt)
    }

    fn filtered_in(
        cell: f32,
        at_paint: Grid<()>,
        asphalt: impl IntoIterator<Item = [Vec2; 3]>,
    ) -> Self {
        let mut triangles = Vec::new();
        let mut near = Grid::new(cell);
        for triangle in asphalt {
            let [a, b, c] = triangle;
            let (min, max) = (a.min(b).min(c), a.max(b).max(c));
            if at_paint.near_each(min, max).next().is_some() {
                near.insert(min, max, triangles.len());
                triangles.push(triangle);
            }
        }
        Self { triangles, near }
    }

    pub(in crate::map::roads) fn contains(&self, point: Vec2) -> bool {
        self.near.at(point).iter().any(|&index| {
            let [a, b, c] = self.triangles[index];
            let sides =
                [(a, b), (b, c), (c, a)].map(|(from, to)| (to - from).perp_dot(point - from));
            sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0)
        })
    }
}

/// Ячейка сетки [`PaintGround`], м.
const GROUND_CELL: f32 = 24.0;

/// Стоп-линия, обрезанная по асфальту улиц: торцы отступают внутрь, пока
/// сечение — край полосы и её середина — не ляжет на асфальт целиком. У устья
/// съезда кромка главной улицы стоит наискось, и линия до полуширины дороги
/// ложилась на плитку обочины, мимо кромки (Тула, S7). Линия, чья середина
/// не на асфальте улиц, не трогается: настил моста — асфальт другого слоя,
/// и на нём линия своя. `None` — короче полосы не осталось.
pub(in crate::map::roads) fn trim_stop_line(
    line: &StopLine,
    on_asphalt: impl Fn(Vec2) -> bool,
) -> Option<StopLine> {
    let span = line.to - line.from;
    let across = span.try_normalize()?;
    let fits = cross_section_fits(line.from, across, STOP_WIDTH / 2.0, on_asphalt);
    let width = span.length();
    let middle = width / 2.0;
    if !fits(middle) {
        return Some(*line);
    }
    let mut low = 0.0;
    while low < middle && !fits(low) {
        low += FIT_STEP;
    }
    let mut high = width;
    while high > middle && !fits(high) {
        high -= FIT_STEP;
    }
    if high - low < STOP_WIDTH {
        return None;
    }
    Some(StopLine {
        from: line.from + across * low,
        to: line.from + across * high,
        ..*line
    })
}
