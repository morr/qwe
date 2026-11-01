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
//! (`road_verges`), который лежит над слоями газона. **Зелень земли** —
//! карманы разбора и замапленный газон — спрашивается своим вопросом
//! ([`Scraps::push_green`], [`Scraps::find_green`]) и кладётся в слой
//! тротуаров; у неё два своих порога — клин ([`GREEN_OPENING_MAX`]) и
//! клин-хвост ([`GREEN_TAIL_AREA_MAX`], кончик островка под горкой).
//!
//! Ошибка в «мощёном» безопасна в одну сторону: мощение, которое здесь
//! недосчитано, только оставляет лоскут открытым к краю газона — и он
//! остаётся газоном, как был. Поэтому полотно берётся по сторонам
//! (`Drawn::band_half`), а не по большей из них.

use std::borrow::Cow;

use bevy::math::Vec2;
use bevy::platform::collections::HashMap;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};

use super::SIDEWALK_COLOR;
use super::corners::KerbReturns;
use super::drawn::Drawn;
use super::gores::Gores;
use super::islands::RoadIslands;
use super::junctions::node_key;
use super::{DrawnPiece, PieceKind};
use crate::map::grid::Grid;
use crate::map::meshing::{MeshBuilder, miter_offsets, ring_perimeter};
use crate::map::osm::model::{RoadLine, distance_to_segment, ring_bounds};
use crate::map::osm::{MapData, RoadClass};
use crate::map::parallel::in_parallel;
use crate::map::roads::network::RoadNodes;
use crate::map::shapes::{
    Contour, Shape, contour_area, contour_bounds, oriented, push_shape, ring_of,
};
use crate::settings::{Z_ROAD_VERGE, Z_SIDEWALK};

/// Не больше этого, м², замкнутый кусок газона — лоскут: карман у угла
/// Халтурина и Гоголевской — около девяти.
const SCRAP_AREA_MAX: f32 = 10.0;
/// Уже этого в среднем (две площади на периметр), м, замкнутый кусок —
/// щепка, какой бы длины ни был: Орёл — меньше метра на десять.
const SCRAP_WIDTH_MAX: f32 = 1.5;
/// Не больше этого, м², замкнутый кусок — клин: он берётся не средней
/// шириной, а остриём, и бывает шире щепки при той же площади (серп между
/// срезом съезда и проезжей, витрина 15, — около двадцати).
const SCRAP_WEDGE_AREA_MAX: f32 = 25.0;
/// Угол острия, °, ниже которого кусок — клин: длинные стороны сходятся, и
/// ширина падает к нулю не в среднем, а у самого острия.
const SCRAP_TIP_ANGLE: f32 = 20.0;
/// Ребро острия не короче этого, м: углы наломанной обводки — не остриё.
const SCRAP_TIP_EDGE: f32 = 1.0;
/// Кусок зелени земли — карман разбора или замапленный газон — кандидат,
/// только если он уже этого, м²: большему клину взяться неоткуда, а вопрос
/// к мощению стоит одной разностью на кусок.
const GREEN_AREA_MAX: f32 = 500.0;
/// Кусок зелени земли кандидат, только если он уже этого в среднем (две
/// площади на периметр), м, — узкая полоса: островок вилки (витрина 14) —
/// около восьми, двор целиком — далеко за ней.
const GREEN_WIDTH_MAX: f32 = 10.0;
/// И только если мощение в него врезается: какая-то вершина куска ближе
/// этого к нарисованной плитке или бордюру, м. Точечный опрос вместо
/// сплошного: у внутренней зелени мощения рядом нет вовсе.
const GREEN_BITE: f32 = 2.0;
/// Открытость клинового куска зелени своему кольцу, м: край кармана — не
/// продолжение газона наружу (под мощением его и не видно), но и не повод
/// мостить длинный двор; клину до [`SCRAP_WEDGE_AREA_MAX`] хватает.
const GREEN_OPENING_MAX: f32 = 8.0;
/// Не больше этого, м², зелёный остаток-«клин-хвост» — сужающийся к острию
/// вдоль всей своей длины: кончик островка под штрихованной горкой вилки
/// (витрина 14, R37-01) — замапленный газон 37 м от среза основания к острию
/// в 13°, полторы сотни квадратов. Хвост узнаётся формой — сужение
/// ([`tapers_to_tip`]) и узость в среднем ([`GREEN_WIDTH_MAX`]) — и обязан
/// быть срезан мощением поперёк ([`TAIL_BASE_MIN`]): двор без острия им не
/// бывает при любой площади, газон, которого мощение лишь коснулось, — тоже.
const GREEN_TAIL_AREA_MAX: f32 = 160.0;
/// Мощения на границе хвоста, м, не меньше: горка и полотна **прилегают** к
/// кончику островка, не пересекая его, — вычитание ничего не отрезает, и
/// хвост обязан показать, что отрезан от тела не одним лишь касанием
/// ([`GREEN_BITE`] у вершины). Срез основания вилки — шесть с лишним.
const TAIL_BASE_MIN: f32 = 2.0;
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
/// Звеньев в обе стороны от вершины, где чужое мощение врезается в полосу
/// газона ([`Scraps::push_lawn_band`]): те же 2.5 м на звено — пятнадцать
/// метров от устья; клин у Сойфера лежит в двенадцати от него (R37-01).
const WEDGE_REACH: usize = 6;

/// Газоны обочин, зелень земли и всё мощёное рядом с ними — то, из чего
/// ищутся лоскуты ([`Scraps::find`], [`Scraps::find_green`]). Копится по ходу
/// укладки лент; мощение — раньше зелени: ей надо видеть врезание.
#[derive(Default)]
pub(super) struct Scraps {
    lawns: Vec<Lawn>,
    /// Зелень земли — карманы разбора и замапленный газон — кольцами
    /// ([`Scraps::push_green`]); срезов у неё нет: кусок сам себе полоса.
    green: Vec<Vec<Vec2>>,
    /// Мощёные контуры `i_overlay` с габаритами и сеткой по ним.
    contours: Vec<(Contour, Vec2, Vec2)>,
    grid: Option<Grid<usize>>,
    /// Газоны по габаритам — какое мощение вообще нужно ([`Scraps::near_lawn`]).
    lawn_grid: Option<Grid<usize>>,
}

/// Газон — или кусок полосы газона ([`Scraps::push_lawn_band`]): контур,
/// **срезы** — рёбра, по которым он отрезан от остальной полосы, — и дорога-
/// владелец, чей OSM id ведёт хвост находки лоскута. За срезом газон
/// продолжается, и кусок, дошедший до среза, лоскутом не бывает.
struct Lawn {
    ring: Vec<Vec2>,
    cuts: Vec<[Vec2; 2]>,
    road: Option<u32>,
}

impl Scraps {
    /// Контур газона.
    pub(super) fn push_lawn(&mut self, ring: Vec<Vec2>) {
        if ring.len() >= 3 {
            self.lawns.push(Lawn {
                ring,
                cuts: Vec::new(),
                road: None,
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
    /// вдоль себя, а кусков в тысячи раз больше, чем лоскутов. Второе
    /// исключение — **укусы** (`bites`, вершины из [`Bites`]): там чужое
    /// мощение врезается в полосу и отгораживает клин, — окно на
    /// [`WEDGE_REACH`] точек в обе стороны, перекрывающиеся слитно (R37-01).
    pub(super) fn push_lawn_band(
        &mut self,
        road: u32,
        path: &[Vec2],
        normals: &[Vec2],
        reach: &[f32],
        ends: [bool; 2],
        bites: &[Vec2],
    ) {
        let last = path.len().saturating_sub(1);
        let edge = |index: usize| [path[index] + normals[index] * reach[index], path[index]];
        let mut windows: Vec<(usize, usize)> = match ends {
            [false, false] => Vec::new(),
            _ if last <= 2 * END_REACH => vec![(0, last)],
            [head, tail] => [(head, (0, END_REACH)), (tail, (last - END_REACH, last))]
                .into_iter()
                .filter_map(|(end, window)| end.then_some(window))
                .collect(),
        };
        let mut run: Option<(usize, usize)> = None;
        for index in (1..last).filter(|index| bites.contains(&path[*index])) {
            let window = (
                index.saturating_sub(WEDGE_REACH),
                (index + WEDGE_REACH).min(last),
            );
            match &mut run {
                Some((_, to)) if window.0 <= *to + 1 => *to = window.1,
                _ => {
                    if let Some(merged) = run.take() {
                        windows.push(merged);
                    }
                    run = Some(window);
                }
            }
        }
        windows.extend(run);
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
                road: Some(road),
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

    /// Кольца и срезы всех газонов — окно спрашивания каждого; диагнозу
    /// (`tests.rs::diag_window`).
    #[cfg(test)]
    pub(super) fn lawns(&self) -> impl Iterator<Item = (&[Vec2], &[[Vec2; 2]])> {
        self.lawns
            .iter()
            .map(|lawn| (&lawn.ring[..], &lawn.cuts[..]))
    }

    /// Лоскуты всех газонов — фигуры, заведённые под мощение на
    /// [`SCRAP_OVERLAP`], с дорогой-владельцем газона, в порядке газонов.
    /// Газоны друг от друга не зависят и считаются по потокам.
    pub(super) fn find(&self) -> Vec<(Shape, Option<u32>)> {
        in_parallel(&self.lawns, |lawn| self.scraps_of(lawn))
            .into_iter()
            .flatten()
            .collect()
    }

    /// Кусок зелени земли — карман разбора (`MapData::pockets`) или
    /// замапленный газон (`MapData::grass`). Клин между полосами мощения
    /// лежит не только в газоне обочины (R37-01: клин у Сойфера — карман,
    /// кончик островка на вилке — замапленный газон, серп витрины 15 —
    /// карман), и спрашивается он у зелени так же, как у газона. Сплошной
    /// опрос всей зелени города стоил бы тех же ~80 мс, что опрос каждые
    /// 40 м ([`END_REACH`]), поэтому кусок — кандидат, только если он форма
    /// клина: узкая полоса ([`GREEN_AREA_MAX`], [`GREEN_WIDTH_MAX`]), в
    /// которую мощение врезается ([`GREEN_BITE`]).
    pub(super) fn push_green(&mut self, ring: Vec<Vec2>) {
        if ring.len() < 3 {
            return;
        }
        let area = contour_area(&oriented(&ring, true));
        let perimeter = ring_perimeter(&ring);
        if area > GREEN_AREA_MAX
            || perimeter <= 0.0
            || 2.0 * area / perimeter > GREEN_WIDTH_MAX
            || !self.bitten(&ring)
        {
            return;
        }
        self.green.push(ring);
    }

    /// Лоскуты зелени земли — тот же вопрос к нарисованному, что у газонов
    /// обочин ([`Scraps::find`]); владельца у зелени земли нет — карман или
    /// замапленный газон не дорога.
    pub(super) fn find_green(&self) -> Vec<(Shape, Option<u32>)> {
        in_parallel(&self.green, |ring| {
            let subject = vec![oriented(ring, true)];
            let (low, high) = contour_bounds(&subject[0]);
            let clip = self.near(low, high);
            if clip.is_empty() {
                return Vec::new();
            }
            scraps_among(ring, &[], true, subject, clip)
                .into_iter()
                .map(|shape| (shape, None))
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()
    }

    /// DIAG-TEMP: все остатки всех газонов с метриками фильтра — (кольцо,
    /// м², средняя ширина, открытость, клин) — диагнозу (`tests.rs`).
    #[cfg(test)]
    pub(super) fn debug_pieces(&self) -> Vec<(Vec<Vec2>, f32, f32, f32, bool)> {
        let lawns: Vec<(Vec<Vec2>, Vec<[Vec2; 2]>)> = self
            .lawns
            .iter()
            .map(|lawn| (lawn.ring.clone(), lawn.cuts.clone()))
            .chain(self.green.iter().map(|ring| (ring.clone(), Vec::new())))
            .collect();
        in_parallel(&lawns, |(ring, cuts)| {
            let subject = vec![oriented(ring, true)];
            let (low, high) = contour_bounds(&subject[0]);
            let clip = self.near(low, high);
            if clip.is_empty() {
                return Vec::new();
            }
            let outline = ring.clone();
            let cuts = cuts.clone();
            subject
                .overlay(&clip, OverlayRule::Difference, FillRule::NonZero)
                .into_iter()
                .filter_map(move |shape| {
                    let outer = shape.first()?;
                    let ring = ring_of(outer);
                    let area =
                        contour_area(outer) - shape[1..].iter().map(contour_area).sum::<f32>();
                    let perimeter = ring_perimeter(&ring);
                    let avg = if perimeter > 0.0 {
                        2.0 * area / perimeter
                    } else {
                        0.0
                    };
                    let on = |point: Vec2| {
                        (0..outline.len()).any(|index| {
                            distance_to_segment(
                                point,
                                outline[index],
                                outline[(index + 1) % outline.len()],
                            ) < ON_EDGE
                        })
                    };
                    let on_cut = |point: Vec2| {
                        cuts.iter()
                            .any(|&[from, to]| distance_to_segment(point, from, to) < ON_EDGE)
                    };
                    let open: f32 = (0..ring.len())
                        .map(|index| (ring[index], ring[(index + 1) % ring.len()]))
                        .filter(|&(from, to)| on((from + to) / 2.0) && on(from) && on(to))
                        .map(|(from, to)| from.distance(to))
                        .sum();
                    let cut = (0..ring.len())
                        .map(|index| (ring[index], ring[(index + 1) % ring.len()]))
                        .any(|(from, to)| on_cut((from + to) / 2.0));
                    let tip = tapers_to_tip(&ring) && !cut;
                    Some((ring, area, avg, open, tip))
                })
                .collect()
        })
        .into_iter()
        .flatten()
        .collect()
    }

    /// Врезается ли мощение в кусок зелени: какая-то его вершина лежит ближе
    /// [`GREEN_BITE`] к нарисованному мощению.
    fn bitten(&self, ring: &[Vec2]) -> bool {
        let Some(grid) = &self.grid else {
            return false;
        };
        ring.iter().any(|&vertex| {
            grid.near_each(vertex - GREEN_BITE, vertex + GREEN_BITE)
                .any(|&index| {
                    let (contour, ..) = &self.contours[index];
                    (0..contour.len()).any(|edge| {
                        distance_to_segment(
                            vertex,
                            contour[edge].into(),
                            contour[(edge + 1) % contour.len()].into(),
                        ) < GREEN_BITE
                    })
                })
        })
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

    /// Лоскуты одного газона: его остаток за мощением, замкнутый и малый,
    /// с владельцем газона.
    fn scraps_of(&self, lawn: &Lawn) -> Vec<(Shape, Option<u32>)> {
        let subject = vec![oriented(&lawn.ring, true)];
        let (low, high) = contour_bounds(&subject[0]);
        let clip = self.near(low, high);
        if clip.is_empty() {
            return Vec::new();
        }
        scraps_among(&lawn.ring, &lawn.cuts, false, subject, clip)
            .into_iter()
            .map(|shape| (shape, lawn.road))
            .collect()
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

/// Сужается ли контур к острию: есть вершина, у которой оба ребра длинные
/// ([`SCRAP_TIP_EDGE`]) и сходятся под углом не шире [`SCRAP_TIP_ANGLE`] —
/// ширина куска падает там к нулю, а не в среднем по куску (R37-01).
fn tapers_to_tip(ring: &[Vec2]) -> bool {
    (0..ring.len()).any(|index| {
        let (prev, at, next) = (
            ring[(index + ring.len() - 1) % ring.len()],
            ring[index],
            ring[(index + 1) % ring.len()],
        );
        let (into, outof) = (prev - at, next - at);
        into.length() >= SCRAP_TIP_EDGE
            && outof.length() >= SCRAP_TIP_EDGE
            && into.perp_dot(outof).abs().atan2(into.dot(outof)) <= SCRAP_TIP_ANGLE.to_radians()
    })
}

/// Окна `[от, до]` по `count` точкам встык, по [`CHUNK`] звеньев.
fn chunks(count: usize) -> Vec<(usize, usize)> {
    let last = count.saturating_sub(1);
    (0..last)
        .step_by(CHUNK)
        .map(|from| (from, (from + CHUNK).min(last)))
        .collect()
}

/// Общие узлы, в которых чужая дорога кончается, — места, где её мощение
/// врезается в полосу газона дороги, прошедшей через узел насквозь
/// (R37-01: устье проезда, срез съезда). Узел, в котором кончается
/// продолжение самой этой улицы, — шов двух way: там газон идёт дальше, и
/// окно не нужно, как и у торцов ([`Scraps::push_lawn_band`], `ends`).
pub(super) struct Bites {
    ends: HashMap<(i32, i32), Vec<Option<usize>>>,
}

impl Bites {
    pub(super) fn of(nodes: &RoadNodes, roads: &[&RoadLine]) -> Self {
        let mut ends: HashMap<(i32, i32), Vec<Option<usize>>> = HashMap::new();
        for (index, road) in roads.iter().enumerate() {
            for (end, point) in [(0, road.points.first()), (1, road.points.last())] {
                let Some(&point) = point else {
                    continue;
                };
                if !nodes.is_shared(point) {
                    continue;
                }
                let continues = nodes.next_way(index, end).map(|(road, _)| road);
                ends.entry(node_key(point)).or_default().push(continues);
            }
        }
        Self { ends }
    }

    /// Врезается ли чужое мощение в полосу дороги `road` в узле `point`.
    pub(super) fn bite(&self, road: usize, point: Vec2) -> bool {
        self.ends
            .get(&node_key(point))
            .is_some_and(|list| list.iter().any(|&continues| continues != Some(road)))
    }

    /// Укусы на внутренних вершинах полосы `points` дороги `index` — но
    /// только там, где обочина вообще будет (`wanted`): вопрос к общим
    /// узлам на каждое звено каждой дороги не нужен
    pub(super) fn on_points(&self, index: usize, points: &[Vec2], wanted: bool) -> Vec<Vec2> {
        if !wanted {
            return Vec::new();
        }
        points[1..points.len().saturating_sub(1)]
            .iter()
            .filter(|&point| self.bite(index, *point))
            .copied()
            .collect()
    }
}

fn scraps_among(
    outline: &[Vec2],
    cuts: &[[Vec2; 2]],
    green: bool,
    subject: Vec<Contour>,
    clip: Vec<Contour>,
) -> Vec<Shape> {
    let on_lawn = |point: Vec2| {
        (0..outline.len()).any(|index| {
            let next = outline[(index + 1) % outline.len()];
            distance_to_segment(point, outline[index], next) < ON_EDGE
        })
    };
    let on_cut = |point: Vec2| {
        cuts.iter()
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
            // клин-хвост зелени: сужается к острию вдоль всей длины и срезан
            // мощением поперёк — кончик островка под горкой вилки, горка и
            // полотна прилегают к нему, не пересекая (R37-01, витрина 14).
            // Хвост мостится целиком: его открытые стороны — его собственные
            // сходящиеся бока, а не рот в продолжение газона
            let tail = green
                && area <= GREEN_TAIL_AREA_MAX
                && perimeter > 0.0
                && 2.0 * area / perimeter <= GREEN_WIDTH_MAX
                && tapers_to_tip(&ring)
                && perimeter - open >= TAIL_BASE_MIN;
            let small = area <= SCRAP_AREA_MAX
                || (perimeter > 0.0 && 2.0 * area / perimeter <= SCRAP_WIDTH_MAX)
                // клин: тонкий у острия, а не в среднем (R37-01)
                || (area <= SCRAP_WEDGE_AREA_MAX && tapers_to_tip(&ring))
                || tail;
            // у клинового куска зелени открытость — собственный край кармана,
            // под мощением его не видно: для куска до [`SCRAP_WEDGE_AREA_MAX`]
            // она не длиннее [`GREEN_OPENING_MAX`]; открытость хвоста и есть
            // его бока — форма уже сказала своё, и она не ограничена
            let open_max = if tail {
                f32::INFINITY
            } else if green && area <= SCRAP_WEDGE_AREA_MAX {
                GREEN_OPENING_MAX
            } else {
                SCRAP_OPENING
            };
            small && open <= open_max && !cut
        })
        .flat_map(|shape| {
            vec![shape].outline(&OutlineStyle::new(SCRAP_OVERLAP).line_join(LineJoin::Bevel))
        })
        .collect()
}

/// Лоскуты газона обочин, замкнутые мощением, — плиткой поверх газона:
/// слой обочин лежит над слоями газона. Мощение, которым
/// они замкнуты, собирается только у газонов, которые спрашиваются: улиц и
/// углов в городе десятки тысяч, а концов газона у узлов — тысяча.
/// Мощение узлов — представлением `KerbReturns::paving`: скругления
/// улиц, тротуары, обочины и мощёные носы с дугами; наезды плитки — нет.
/// и горки: асфальт или штриховка направляющего островка — не газон,
/// и кончик островка под горкой оставался клином травы (R37-01, витрина 14)
pub(super) fn pave_scraps(
    kerb_returns: &KerbReturns,
    gores: &Gores,
    drawn: &[&RoadLine],
    prepared: &Drawn,
    ribbon: &[Cow<'_, [Vec2]>],
    road_islands: &RoadIslands,
    scraps: &mut Scraps,
) {
    let gore_outlines: Vec<Vec<Vec2>> = gores
        .outlines()
        .map(|contour| contour.iter().map(|&point| point.into()).collect())
        .collect();
    for outline in kerb_returns
        .paving()
        .flat()
        .chain(gore_outlines.iter().map(Vec::as_slice))
        .chain(kerb_returns.paving().curved())
    {
        if scraps.near_lawn(outline, 0.0) {
            scraps.push_paving(outline);
        }
    }
    for (index, road) in drawn.iter().enumerate() {
        let paved =
            !road.is_unpaved_street() && (road.class == RoadClass::Street || road.is_paved_path());
        if !paved || road.bridge || road_islands.outlines[index] {
            continue;
        }
        let halves = [0, 1].map(|side| prepared.band_half(index, side));
        if scraps.near_lawn(&ribbon[index], halves[0].max(halves[1])) {
            scraps.push_band(&ribbon[index], halves);
        }
    }
}

/// Зелень земли — карманы разбора и замапленный газон: у клиньев два
/// владельца. Спрашивается после мощения — врезание в
/// неё видно по нарисованному; кладётся в слой тротуаров: слой обочин
/// лежит ПОД зеленью (Z_ROAD_VERGE < Z_GRASS), и плитка клина осталась бы
/// под травой, а тротуары — над (Z_SIDEWALK), и перекрытие с соседней
/// плиткой того же материала не видно
pub(super) fn lay_green_scraps(
    map: &MapData,
    scraps: &mut Scraps,
    verges: &mut MeshBuilder,
    sidewalks: &mut MeshBuilder,
    pieces: &mut Vec<DrawnPiece>,
) -> usize {
    for area in map.pockets.iter().chain(&map.grass) {
        scraps.push_green(area.outer.clone());
    }
    let green_scraps = scraps.find_green();
    let lawn_scraps = scraps.find();
    verges.set_lanes(None);
    for (shape, road) in &lawn_scraps {
        push_shape(verges, shape.clone(), SIDEWALK_COLOR.to_linear());
        // замостенный лоскут газона — кусок слоя обочин
        pieces.push(shape_piece(
            shape,
            PieceKind::VergePaving,
            *road,
            Z_ROAD_VERGE,
        ));
    }
    sidewalks.set_lanes(None);
    for (shape, road) in &green_scraps {
        push_shape(sidewalks, shape.clone(), SIDEWALK_COLOR.to_linear());
        // замостенный клин зелени земли — кусок слоя тротуаров
        pieces.push(shape_piece(
            shape,
            PieceKind::PavingScrap,
            *road,
            Z_SIDEWALK,
        ));
    }
    lawn_scraps.len() + green_scraps.len()
}

/// Отрисованный кусок из фигуры `i_overlay`: внешнее кольцо и дырки как есть.
fn shape_piece(shape: &Shape, kind: PieceKind, road: Option<u32>, z: f32) -> DrawnPiece {
    let to_vec2 = |contour: &Contour| {
        contour
            .iter()
            .map(|point| Vec2::from_array(*point))
            .collect::<Vec<Vec2>>()
    };
    DrawnPiece {
        contour: to_vec2(shape.first().expect("лоскут не бывает без внешнего кольца")),
        holes: shape[1..].iter().map(to_vec2).collect(),
        kind,
        road,
        z,
    }
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
        scraps.find().into_iter().map(|(shape, _)| shape).collect()
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

    /// Газон 40 × 8 м, закрытый мощением целиком, кроме треугольной дырки
    /// `(10, 2), (22.25, 2), (10, 6)` — клин, зажатый плиткой с трёх сторон.
    fn scraps_with_wedge_hole() -> Vec<Shape> {
        let mut scraps = Scraps::default();
        scraps.push_lawn(rect(Vec2::ZERO, Vec2::new(40.0, 8.0)));
        // три полосы мощения вокруг клина: низ, лево и наискось по гипотенузе
        scraps.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 2.0)));
        scraps.push_paving(&rect(Vec2::new(-1.0, 2.0), Vec2::new(10.0, 9.0)));
        scraps.push_paving(&[
            Vec2::new(10.0, 6.0),
            Vec2::new(22.25, 2.0),
            Vec2::new(41.0, -4.2),
            Vec2::new(41.0, 9.0),
            Vec2::new(9.0, 9.0),
        ]);
        scraps.find().into_iter().map(|(shape, _)| shape).collect()
    }

    #[test]
    fn a_remainder_tapering_to_a_tip_is_a_scrap_beyond_the_average() {
        // клин 24.5 м²: площадь и средняя ширина (1.68) за порогами, но
        // длинные стороны сходятся под углом в 18° — остриё, и мощению такой
        // кусок оставлять не на что (R37-01)
        assert_eq!(scraps_with_wedge_hole().len(), 1);
    }

    /// Кусок зелени земли — полоса 40 × 3 м между плиткой снизу и сверху, с
    /// наискось врезавшейся полосой мощения у `x = 18` и закрытым левым
    /// торцом. Мощение кладётся раньше зелени: ей видно врезание.
    fn green_with_bite() -> Scraps {
        let mut scraps = Scraps::default();
        scraps.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 0.5)));
        scraps.push_paving(&rect(Vec2::new(-1.0, 2.5), Vec2::new(41.0, 4.0)));
        scraps.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(6.0, 4.0)));
        scraps.push_paving(&[
            Vec2::new(18.0, -1.0),
            Vec2::new(19.5, -1.0),
            Vec2::new(8.0, 4.0),
            Vec2::new(6.5, 4.0),
        ]);
        scraps.push_green(rect(Vec2::ZERO, Vec2::new(40.0, 3.0)));
        scraps
    }

    #[test]
    fn a_ground_green_wedge_bitten_by_paving_is_a_scrap() {
        // клин у Сойфера (N3): карман земли между полосами мощения, сужающийся
        // к устью, — мостится, хоть и не газон обочины (R37-01); кусок справа
        // от вреза шире порога клина (41 м²) и остаётся травой
        let found = green_with_bite().find_green();
        assert_eq!(found.len(), 1, "мостится клин слева от вреза");
        let (low, high) = contour_bounds(&found[0].0[0]);
        assert!(high.x < 20.0, "клин — левее вреза: {low} {high}");
    }

    #[test]
    fn a_green_piece_open_to_the_lawn_stays_grass() {
        // клык у устья (витрина 12): длинная сторона куска открыта в большой
        // газон — замкнутым клином он не бывает, что за форма у остатка ни будь
        let mut scraps = Scraps::default();
        scraps.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 0.5)));
        scraps.push_green(rect(Vec2::ZERO, Vec2::new(40.0, 3.0)));
        assert!(scraps.find_green().is_empty());
    }

    #[test]
    fn a_fat_green_remainder_is_not_asked_into_a_scrap() {
        // двор целиком — не узкая полоса: и кандидатом клина не бывает
        let mut scraps = Scraps::default();
        scraps.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 0.5)));
        scraps.push_green(rect(Vec2::ZERO, Vec2::new(40.0, 10.0)));
        assert!(scraps.find_green().is_empty());
    }

    #[test]
    fn a_wedge_bitten_mid_strip_is_asked_at_the_bite() {
        // проезд врезается в полосу газона в середине улицы: клин между его
        // срезом и полосой у бордюра мостится, хоть правило и спрашивало
        // раньше одни концы полос у узлов (R37-01)
        let path: Vec<Vec2> = (0..=16)
            .map(|index| Vec2::new(index as f32 * 2.5, 0.0))
            .collect();
        let normals = vec![Vec2::Y; path.len()];
        let reach = vec![6.0; path.len()];
        let mut with_bite = Scraps::default();
        with_bite.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 2.0)));
        with_bite.push_paving(&rect(Vec2::new(-1.0, 5.0), Vec2::new(41.0, 7.0)));
        // врез наискось и его завод вдоль полосы, справа — закрытый торец
        with_bite.push_paving(&[
            Vec2::new(24.0, -1.0),
            Vec2::new(25.5, -1.0),
            Vec2::new(21.5, 7.0),
            Vec2::new(20.0, 7.0),
        ]);
        with_bite.push_paving(&rect(Vec2::new(20.0, -1.0), Vec2::new(34.0, 3.5)));
        with_bite.push_paving(&rect(Vec2::new(33.0, -1.0), Vec2::new(35.0, 7.0)));
        with_bite.push_lawn_band(0, &path, &normals, &reach, [false; 2], &[path[10]]);
        assert_eq!(with_bite.find().len(), 1, "клин у вреза мостится");
        // а без укуса то же место не спрашивается вовсе — окно даёт укус
        let mut without = Scraps::default();
        without.push_paving(&rect(Vec2::new(-1.0, -1.0), Vec2::new(41.0, 2.0)));
        without.push_paving(&rect(Vec2::new(-1.0, 5.0), Vec2::new(41.0, 7.0)));
        without.push_lawn_band(0, &path, &normals, &reach, [false; 2], &[]);
        assert!(without.find().is_empty());
    }

    /// Кончик островка под штрихованной горкой вилки (витрина 14, R37-01):
    /// замапленный газон — клин 37 м от основания 8.6 м к острию в 13°.
    /// Полотна вилки режут основание, горка **прилегает** к западному боку,
    /// не пересекая, — вычитание ничего не отрезает, и весь остаток (~146 м²,
    /// открытость 71 м — его собственные бока) оставался травой: не клин
    /// (25 м²), не щепка (1.5 м в среднем). Хвост мостится целиком.
    #[test]
    fn a_green_island_tail_tapering_along_is_paved_whole() {
        let island = vec![
            Vec2::new(0.0, 36.9),
            Vec2::new(-4.3, 0.0),
            Vec2::new(4.3, 0.0),
        ];
        let mut scraps = Scraps::default();
        // полотна вилки срезают основание клина
        scraps.push_paving(&rect(Vec2::new(-40.0, -40.0), Vec2::new(40.0, 1.5)));
        // штрихованная горка: полоса вдоль западного бока, зазором 0.3 м —
        // прилегает, не пересекая
        scraps.push_paving(&[
            Vec2::new(-0.3, 36.9),
            Vec2::new(-4.6, 0.0),
            Vec2::new(-6.1, 0.0),
            Vec2::new(-2.1, 36.9),
        ]);
        scraps.push_green(island.clone());
        let found = scraps.find_green();
        assert_eq!(found.len(), 1, "клин-хвост мостится целиком");
        let (low, high) = contour_bounds(&found[0].0[0]);
        assert!(
            low.y > 1.0 && (high.y - 36.9).abs() < 1.0,
            "хвост — от среза основания до острия: {low} {high}"
        );
        // тупой кусок той же площади (клык у устья, витрина 12) — без
        // острия хвостом не бывает и остаётся травой
        let mut blunt = Scraps::default();
        blunt.push_paving(&rect(Vec2::new(-40.0, -40.0), Vec2::new(40.0, 1.5)));
        blunt.push_green(rect(Vec2::new(-20.0, 1.6), Vec2::new(20.0, 5.0)));
        assert!(blunt.find_green().is_empty(), "без острия — трава");
        // мощение, лишь коснувшееся угла газона (ближе GREEN_BITE), не
        // отрезав хвоста от тела, — трава: хвост обязан быть срезан
        let mut grazed = Scraps::default();
        grazed.push_paving(&rect(Vec2::new(6.0, -10.0), Vec2::new(20.0, 1.0)));
        grazed.push_green(island);
        assert!(grazed.find_green().is_empty(), "не отрезан — трава");
    }
}
