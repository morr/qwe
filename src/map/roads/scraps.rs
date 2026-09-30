//! **Лоскуты газона** (lawn scraps) — куски газона широкой обочины, со всех
//! сторон закрытые мощением, мостятся плиткой тротуара.
//!
//! Газон обочины кладётся полосой от оси улицы до оси дорожки за ней
//! (`push_verges`), под плиткой и асфальтом: видно из него то, что между
//! кромкой (полосой тротуара или плитки у бордюра) и лентой дорожки. Обычно
//! это полоса газона вдоль улицы, но у угла, где её режут площадка плитки у
//! бордюра (`corners::kerb_pad`), переход и дорожка, поворачивающая к нему,
//! и там, где дорожка почти прижалась к тротуару, от неё остаётся **лоскут**:
//! зелёный карман метра в три посреди плитки (Тула, угол Халтурина и
//! Гоголевской) или щепка в десять метров длиной и меньше метра шириной
//! между тротуаром и дорожкой (Орёл, Орджоникидзе у Ляшко). На месте такого
//! газона не бывает — там плитка.
//!
//! Правило — вопрос к нарисованному: газон минус всё мощёное рядом (полотна
//! с тротуарами по сторонам, мощёные дорожки, плитка обочин, скругления и
//! площадки узлов). Кусок остатка — лоскут, если он **замкнут** мощением (на
//! краю самого газона лежит не больше [`SCRAP_OPENING`] его границы: за краем
//! газон продолжается — газоном соседней улицы, угла, или уходит под квартал)
//! и **мал**: не больше [`SCRAP_AREA_MAX`] или в среднем уже
//! [`SCRAP_WIDTH_MAX`]. Лоскут кладётся плиткой в слой обочин
//! (`road_verges`), который лежит над слоями газона. Замапленная
//! зелень (`parks`, `grass`) сюда не попадает вовсе — спрашиваются только
//! газоны обочин, которые рисуем мы.
//!
//! Ошибка в «мощёном» безопасна в одну сторону: мощение, которое здесь
//! недосчитано, только оставляет лоскут открытым к краю газона — и он
//! остаётся газоном, как был. Поэтому полотно берётся по сторонам
//! (`Drawn::band_half`), а не по большей из них.

use bevy::math::Vec2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

use crate::map::grid::Grid;
use crate::map::meshing::{miter_offsets, ring_perimeter};
use crate::map::osm::model::{distance_to_segment, ring_bounds};
use crate::map::parallel::in_parallel;
use crate::map::shapes::{Contour, Shape, contour_area, contour_bounds, oriented, ring_of};

/// Не больше этого, м², замкнутый кусок газона — лоскут: карман у угла
/// Халтурина и Гоголевской — около девяти.
const SCRAP_AREA_MAX: f32 = 10.0;
/// Уже этого в среднем (две площади на периметр), м, замкнутый кусок —
/// щепка, какой бы длины ни был: Орёл — меньше метра на десять.
const SCRAP_WIDTH_MAX: f32 = 1.5;
/// На сколько лоскут заводится под мощение вокруг, м: остаток считался по
/// осям без сглаживания лент, и шов не должен светиться нитью газона.
const SCRAP_OVERLAP: f32 = 0.2;
/// Столько края самого газона, м, лоскут может иметь и остаться замкнутым:
/// щепка Орла остриём выходит к краю газона на семь десятков сантиметров, за
/// краем там квартал. Лоскут, открытый шире, — продолжение газона.
const SCRAP_OPENING: f32 = 1.0;
/// Ребро куска ближе этого к краю газона, м, лежит на нём.
const ON_EDGE: f32 = 0.02;
/// Ячейка сетки мощения, м.
const CELL: f32 = 32.0;
/// Звеньев в куске полосы мощения ([`Scraps::push_band`],
/// [`Scraps::push_paving_band`]): газон спрашивает мощение по габариту, и
/// улица целиком тянула бы к нему все свои сотни метров.
const CHUNK: usize = 16;
/// Звеньев у каждого конца полосы газона, где ищутся лоскуты
/// ([`Scraps::push_lawn_band`]): точки обочины через 2.5 м — двадцать пять
/// метров от угла. Лоскут Орла — щепка в десять метров, кончающаяся у
/// перехода в пятнадцати от узла.
const END_REACH: usize = 10;

/// Газоны обочин и всё мощёное рядом с ними — то, из чего ищутся лоскуты
/// ([`Scraps::find`]). Копится по ходу укладки лент.
#[derive(Default)]
pub(super) struct Scraps {
    lawns: Vec<Lawn>,
    /// Мощёные контуры `i_overlay` с габаритами и сеткой по ним.
    contours: Vec<(Contour, Vec2, Vec2)>,
    grid: Option<Grid<usize>>,
    /// Газоны по габаритам — какое мощение вообще нужно ([`Scraps::near_lawn`]).
    lawn_grid: Option<Grid<usize>>,
}

/// Газон — или кусок полосы газона ([`Scraps::push_lawn_band`]): контур и
/// **срезы** — рёбра, по которым он отрезан от остальной полосы. За срезом
/// газон продолжается, и кусок, дошедший до среза, лоскутом не бывает.
struct Lawn {
    ring: Vec<Vec2>,
    cuts: Vec<[Vec2; 2]>,
}

impl Scraps {
    /// Контур газона.
    pub(super) fn push_lawn(&mut self, ring: Vec<Vec2>) {
        if ring.len() >= 3 {
            self.lawns.push(Lawn {
                ring,
                cuts: Vec::new(),
            });
        }
    }

    /// Лежит ли ломаная `path` с полосой `reach` по сторонам у какого-нибудь
    /// газона — по габаритам. Сетка газонов собирается при первом вопросе:
    /// спрашивают, когда газоны уже все.
    pub(super) fn near_lawn(&mut self, path: &[Vec2], reach: f32) -> bool {
        let lawns = &self.lawns;
        let grid = self.lawn_grid.get_or_insert_with(|| {
            let mut grid = Grid::new(CELL);
            for (index, lawn) in lawns.iter().enumerate() {
                let (low, high) = ring_bounds(&lawn.ring);
                grid.insert(low, high, index);
            }
            grid
        });
        let (low, high) = ring_bounds(path);
        let (low, high) = (low - reach, high + reach);
        grid.near_each(low, high).any(|&index| {
            let (from, to) = ring_bounds(&lawns[index].ring);
            from.cmple(high).all() && to.cmpge(low).all()
        })
    }

    /// Мощёный контур.
    pub(super) fn push_paving(&mut self, ring: &[Vec2]) {
        if ring.len() < 3 {
            return;
        }
        let contour = oriented(ring, true);
        let (low, high) = contour_bounds(&contour);
        let grid = self.grid.get_or_insert_with(|| Grid::new(CELL));
        grid.insert(low, high, self.contours.len());
        self.contours.push((contour, low, high));
    }

    /// Газон полосой вдоль ломаной `path` от оси до `reach` в сторону
    /// `normals` (сдвиг точки на метр — `meshing::miter_offsets`). Лоскуты
    /// бывают **у концов** полосы — у угла, где её режут площадка у бордюра,
    /// переход и дорожка к нему; в середине улицы газон — полоса, открытая
    /// вдоль. Поэтому спрашиваются только концы, по [`END_REACH`] точек, со
    /// срезом поперёк, и только те, что кончаются в узле (`ends`, `[начало,
    /// конец]`): у шва двух way одной улицы газон идёт дальше газоном
    /// продолжения. Длинная полоса целиком тянула бы в разность всё мощение
    /// вдоль себя, а кусков в тысячи раз больше, чем лоскутов.
    pub(super) fn push_lawn_band(
        &mut self,
        path: &[Vec2],
        normals: &[Vec2],
        reach: &[f32],
        ends: [bool; 2],
    ) {
        let last = path.len().saturating_sub(1);
        let edge = |index: usize| [path[index] + normals[index] * reach[index], path[index]];
        let windows = match ends {
            [false, false] => Vec::new(),
            _ if last <= 2 * END_REACH => vec![(0, last)],
            [head, tail] => [(head, (0, END_REACH)), (tail, (last - END_REACH, last))]
                .into_iter()
                .filter_map(|(end, window)| end.then_some(window))
                .collect(),
        };
        for (from, to) in windows {
            let ring = band_ring(path, normals, reach, from, to);
            if ring.len() < 3 {
                continue;
            }
            let cuts = [
                (from > 0).then(|| edge(from)),
                (to < last).then(|| edge(to)),
            ];
            self.lawns.push(Lawn {
                ring,
                cuts: cuts.into_iter().flatten().collect(),
            });
        }
    }

    /// Мощёная полоса того же вида — плитка обочины: кусками встык.
    pub(super) fn push_paving_band(&mut self, path: &[Vec2], normals: &[Vec2], reach: &[f32]) {
        for (from, to) in chunks(path.len()) {
            self.push_paving(&band_ring(path, normals, reach, from, to));
        }
    }

    /// Мощёная полоса вдоль ломаной `path`: `halves` — от оси влево и вправо
    /// по ходу точек. Кусками по [`CHUNK`] точек (стык в общей точке, сдвиги
    /// — по всей ломаной): газон спрашивает мощение по габариту.
    pub(super) fn push_band(&mut self, path: &[Vec2], halves: [f32; 2]) {
        if path.len() < 2 {
            return;
        }
        let [left, right] = [halves[0], -halves[1]].map(|half| miter_offsets(path, false, half));
        for (from, to) in chunks(path.len()) {
            let mut ring: Vec<Vec2> = (from..=to).map(|index| path[index] + left[index]).collect();
            ring.extend((from..=to).rev().map(|index| path[index] + right[index]));
            self.push_paving(&ring);
        }
    }

    /// Лоскуты всех газонов — фигуры, заведённые под мощение на
    /// [`SCRAP_OVERLAP`], в порядке газонов. Газоны друг от друга не зависят и
    /// считаются по потокам.
    pub(super) fn find(&self) -> Vec<Shape> {
        in_parallel(&self.lawns, |lawn| self.scraps_of(lawn))
            .into_iter()
            .flatten()
            .collect()
    }

    fn near(&self, low: Vec2, high: Vec2) -> Vec<Contour> {
        let Some(grid) = &self.grid else {
            return Vec::new();
        };
        grid.near(low, high)
            .into_iter()
            .map(|index| &self.contours[index])
            .filter(|(_, from, to)| from.cmple(high).all() && to.cmpge(low).all())
            .map(|(contour, ..)| contour.clone())
            .collect()
    }

    /// Лоскуты одного газона: его остаток за мощением, замкнутый и малый.
    fn scraps_of(&self, lawn: &Lawn) -> Vec<Shape> {
        let subject = vec![oriented(&lawn.ring, true)];
        let (low, high) = contour_bounds(&subject[0]);
        let clip = self.near(low, high);
        if clip.is_empty() {
            return Vec::new();
        }
        scraps_among(lawn, subject, clip)
    }
}

/// Контур полосы вдоль точек `from..=to` ломаной `path`: наружная кромка на
/// `reach` по `normals` туда, ось — обратно.
fn band_ring(path: &[Vec2], normals: &[Vec2], reach: &[f32], from: usize, to: usize) -> Vec<Vec2> {
    let mut ring: Vec<Vec2> = (from..=to)
        .map(|index| path[index] + normals[index] * reach[index])
        .collect();
    ring.extend(path[from..=to].iter().rev());
    ring
}

/// Окна `[от, до]` по `count` точкам встык, по [`CHUNK`] звеньев.
fn chunks(count: usize) -> Vec<(usize, usize)> {
    let last = count.saturating_sub(1);
    (0..last)
        .step_by(CHUNK)
        .map(|from| (from, (from + CHUNK).min(last)))
        .collect()
}

fn scraps_among(lawn: &Lawn, subject: Vec<Contour>, clip: Vec<Contour>) -> Vec<Shape> {
    let outline = &lawn.ring;
    let on_lawn = |point: Vec2| {
        (0..outline.len()).any(|index| {
            let next = outline[(index + 1) % outline.len()];
            distance_to_segment(point, outline[index], next) < ON_EDGE
        })
    };
    let on_cut = |point: Vec2| {
        lawn.cuts
            .iter()
            .any(|&[from, to]| distance_to_segment(point, from, to) < ON_EDGE)
    };
    subject
        .overlay(&clip, OverlayRule::Difference, FillRule::NonZero)
        .into_iter()
        .filter(|shape| {
            let Some(outer) = shape.first() else {
                return false;
            };
            let ring = ring_of(outer);
            let area = contour_area(outer) - shape[1..].iter().map(contour_area).sum::<f32>();
            let perimeter = ring_perimeter(&ring);
            let small = area <= SCRAP_AREA_MAX
                || (perimeter > 0.0 && 2.0 * area / perimeter <= SCRAP_WIDTH_MAX);
            // замкнут: на краю газона (ребро серединой и обоими концами на
            // нём) — не больше щели [`SCRAP_OPENING`]
            let edges =
                || (0..ring.len()).map(|index| (ring[index], ring[(index + 1) % ring.len()]));
            let open: f32 = edges()
                .filter(|&(from, to)| on_lawn((from + to) / 2.0) && on_lawn(from) && on_lawn(to))
                .map(|(from, to)| from.distance(to))
                .sum();
            // до среза куска полосы — газон идёт дальше
            let cut = edges().any(|(from, to)| on_cut((from + to) / 2.0));
            small && open <= SCRAP_OPENING && !cut
        })
        .flat_map(|shape| {
            vec![shape].outline(&OutlineStyle::new(SCRAP_OVERLAP).line_join(LineJoin::Bevel))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(low: Vec2, high: Vec2) -> Vec<Vec2> {
        vec![
            low,
            Vec2::new(high.x, low.y),
            high,
            Vec2::new(low.x, high.y),
        ]
    }

    /// Газон 40 × 6 м, закрытый мощением целиком, кроме окна `hole`.
    fn scraps_with_hole(hole: (Vec2, Vec2)) -> Vec<Shape> {
        let lawn = rect(Vec2::ZERO, Vec2::new(40.0, 6.0));
        let mut scraps = Scraps::default();
        scraps.push_lawn(lawn);
        let (low, high) = hole;
        let outer = (Vec2::splat(-1.0), Vec2::new(41.0, 7.0));
        // четыре полосы мощения вокруг окна
        for (from, to) in [
            (outer.0, Vec2::new(low.x, outer.1.y)),
            (Vec2::new(high.x, outer.0.y), outer.1),
            (Vec2::new(low.x, outer.0.y), Vec2::new(high.x, low.y)),
            (Vec2::new(low.x, high.y), Vec2::new(high.x, outer.1.y)),
        ] {
            if from.x < to.x && from.y < to.y {
                scraps.push_paving(&rect(from, to));
            }
        }
        scraps.find()
    }

    #[test]
    fn an_enclosed_small_remainder_is_a_scrap() {
        let found = scraps_with_hole((Vec2::new(10.0, 2.0), Vec2::new(12.0, 4.0)));
        assert_eq!(found.len(), 1);
        // заведён под мощение
        let (low, high) = contour_bounds(&found[0][0]);
        assert!(low.x < 10.0 - 0.1 && high.x > 12.0 + 0.1, "{low} {high}");
    }

    #[test]
    fn a_long_thin_enclosed_remainder_is_a_scrap() {
        // метр на двадцать — 20 м², но средняя ширина под метр
        let found = scraps_with_hole((Vec2::new(10.0, 2.0), Vec2::new(30.0, 3.0)));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_large_or_open_remainder_stays_lawn() {
        // 5 × 5 — площадь и ширина больше порогов
        assert!(scraps_with_hole((Vec2::new(10.0, 0.5), Vec2::new(15.0, 5.5))).is_empty());
        // малый, но выходит к краю газона на два метра: там газон продолжается
        assert!(scraps_with_hole((Vec2::new(10.0, 4.0), Vec2::new(12.0, 8.0))).is_empty());
    }
}
