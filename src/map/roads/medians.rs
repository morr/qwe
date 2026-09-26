//! Разделительная парных половин на картинке — то, что лежит между ними
//! (`roads/network/pairs.rs` находит пару и разводит оси).
//!
//! - **Асфальт** ([`push_paved`]) — у разделительной до
//!   [`RoadShape::median_gap`](super::shape::RoadShape::median_gap): полоса вдоль середины
//!   на всё расстояние между осями, **под** лентами половин. Под зазором в
//!   полметра иначе светился тротуар — нитка во всю длину проспекта. Колеи у
//!   этой полосы нет (раскладки полос у неё нет), а поверх неё колею кладут
//!   сами половины: полоса ложится раньше них. Двойную сплошную по середине
//!   кладёт слой краски (`Painter::paint_median`).
//! - **Газон с бордюром** ([`push_lawn`]) — шире: контур между внутренними
//!   кромками половин, скруглённый у концов в нос, в слое тротуаров — это
//!   бордюр, видный полосой [`MEDIAN_KERB`] у кромки, — и газон поверх него,
//!   ужатый на бордюр. Газон рвётся у перекрёстка: поперечная улица проходит
//!   разделительную насквозь, и нос встаёт за [`NOSE_CLEARANCE`] до края
//!   разрыва разметки.
//! - **Трамвайное полотно** ([`push_bed`], `Median::carries_tram`) —
//!   разделительная, по которой идёт трамвай: каждая половина расширяется до
//!   середины своей внутренней полосой, без разметки. Асфальт кладётся от
//!   внутренней кромки до внутренней кромки — с ровными торцами, а не
//!   круглыми: соседний газон кладёт нос у торца полотна, как у перекрёстка.
//!   Двойная сплошная — по середине, между путями; светлая полоса над
//!   рельсами — `roads/tram_band.rs`.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

use super::network::pairs::{Median, PAIR_MIN};
use super::{RoadJoin, push_ribbon};
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::polyline_length;
use crate::map::shapes::{ARC, Shape, contour_area, oriented, push_shape};

/// Насколько середина может не доходить до края разрыва перекрёстка, чтобы её
/// дотянули ([`reach_breaks`]), м.
const MEDIAN_EXTEND: f32 = 12.0;
/// Бордюр газона, м: светлая полоса между кромкой половины и травой.
pub const MEDIAN_KERB: f32 = 0.5;
/// Сколько асфальта нос газона оставляет до края разрыва перекрёстка, м.
const NOSE_CLEARANCE: f32 = 1.0;
/// Доля ширины газона — радиус скругления носа: почти полукруг.
const NOSE_SHARE: f32 = 0.45;
/// Кусочек газона мельче этого, м², не рисуется.
const MIN_LAWN_AREA: f32 = 4.0;

/// Разрывы, которые проходят разделительную насквозь: разрыв одной половины,
/// против которого есть разрыв другой. Улица, примыкающая только к ближней
/// половине, разделительную не открывает — ни газон, ни двойную сплошную:
/// дальняя половина идёт мимо, а налево через неё не повернуть. Открывают её
/// поперечная улица и разворот — у обеих половин по узлу напротив друг друга.
pub fn crossing_breaks(median: &Median, [first, second]: [&[Break]; 2]) -> Vec<Break> {
    let apart = median.apart();
    let facing = |gap: &Break, others: &[Break]| {
        others
            .iter()
            .any(|other| gap.at.distance(other.at) <= apart + gap.reach + other.reach)
    };
    first
        .iter()
        .filter(|gap| facing(gap, second))
        .chain(second.iter().filter(|gap| facing(gap, first)))
        .copied()
        .collect()
}

/// Дотянуть середину разделительной до разрыва перекрёстка впереди — не
/// дальше [`MEDIAN_EXTEND`] от её торца.
///
/// Пара ищется пробами: у торца way ближайшая точка соседа — его торец, она
/// смещена вдоль оси, и проба перестаёт считать соседа «рядом» за несколько
/// метров до узла. Середина кончалась там же, и двойная сплошная не доходила
/// до перекрёстка, где линии полос уже доходят (отчёт автора). Дотянутая до
/// центра разрыва, она гаснет у его края сама — как линии полос.
/// Кромки газона продлеваются на ту же длину, каждая по своему ходу.
pub fn reach_breaks(median: &mut Median, breaks: &[Break]) {
    fn lines(median: &mut Median) -> [&mut Vec<Vec2>; 3] {
        let Median { midline, inner, .. } = median;
        let [first, second] = inner;
        [midline, first, second]
    }
    for end in [false, true] {
        let count = median.midline.len();
        if count < 2 {
            return;
        }
        let Some((tip, heading)) = tip_of(&median.midline, end) else {
            continue;
        };
        // ближайший разрыв впереди, до края которого не дальше предела
        let ahead = breaks
            .iter()
            .filter_map(|gap| {
                let along = (gap.at - tip).dot(heading);
                let aside = (gap.at - tip - heading * along).length();
                (along > 0.0 && aside < gap.reach && along - gap.reach < MEDIAN_EXTEND)
                    .then_some(along)
            })
            .min_by(f32::total_cmp);
        let Some(along) = ahead else {
            continue;
        };
        for line in lines(median) {
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
}

/// Торец ломаной и направление её последнего звена наружу.
pub(super) fn tip_of(line: &[Vec2], end: bool) -> Option<(Vec2, Vec2)> {
    let count = line.len();
    if count < 2 {
        return None;
    }
    let (tip, before) = if end {
        (line[count - 1], line[count - 2])
    } else {
        (line[0], line[1])
    };
    Some((tip, (tip - before).try_normalize()?))
}

/// Асфальт узкой разделительной — полосой по середине шириной во всё
/// расстояние между осями, в слой улиц до лент половин.
pub fn push_paved(builder: &mut MeshBuilder, median: &Median, color: LinearRgba, join: RoadJoin) {
    if median.midline.len() < 2 {
        return;
    }
    builder.set_lanes(None);
    push_ribbon(builder, &median.midline, median.apart(), color, join);
}

/// Нахлёст асфальта полотна под ленты половин, м: край, совпадающий с
/// краем ленты, но не делящий с ней вершин, растеризуется с пропусками.
const BED_OVERLAP: f32 = 0.05;

/// Асфальт трамвайного полотна — внутренние полосы обеих половин: от
/// внутренней кромки одной до внутренней кромки другой, с нахлёстом
/// [`BED_OVERLAP`] под их ленты, в слой улиц до лент половин. Контуром, а не
/// лентой по середине: ширина идёт за кромками, где зазор гуляет, а торцы
/// ровные — круглый торец ленты ложился поверх носа соседнего газона.
/// Раскладки полос у него нет: колея — только на автомобильных полосах.
pub fn push_bed(builder: &mut MeshBuilder, median: &Median, color: LinearRgba) {
    let [first, second] = &median.inner;
    if median.midline.len() < 2 || first.len() != median.midline.len() {
        return;
    }
    let widened = |edge: &[Vec2]| -> Vec<Vec2> {
        edge.iter()
            .zip(&median.midline)
            .map(|(&point, &mid)| point + (point - mid).normalize_or_zero() * BED_OVERLAP)
            .collect()
    };
    let ring: Vec<Vec2> = widened(first)
        .into_iter()
        .chain(widened(second).into_iter().rev())
        .collect();
    builder.set_lanes(None);
    builder.push_polygon(&ring, &[], color);
}

/// Торцы трамвайного полотна — разрывами для соседнего газона: нос газона
/// встаёт за [`NOSE_CLEARANCE`] до торца, как у перекрёстка.
pub fn bed_ends(median: &Median) -> [Option<Break>; 2] {
    [false, true].map(|end| tip_of(&median.midline, end).map(|(at, _)| Break { at, reach: 0.0 }))
}

/// Газон разделительной: бордюр — в `kerbs` (слой тротуаров), трава — в
/// `grass`. `breaks` — разрывы разметки обеих половин. Возвращает контуры
/// бордюра — к ним подходит асфальт торца трамвайного полотна ([`bed_caps`]).
pub fn push_lawn(
    kerbs: &mut MeshBuilder,
    grass: &mut MeshBuilder,
    median: &Median,
    breaks: &[Break],
    kerb_color: LinearRgba,
    grass_color: LinearRgba,
) -> Vec<Shape> {
    let nose = (median.gap * NOSE_SHARE).max(ARC);
    let round = || LineJoin::Round(ARC);
    let mut drawn = Vec::new();
    for outline in lawn_outlines(median, breaks) {
        let kerb: Vec<Shape> = vec![vec![outline]]
            .outline(&OutlineStyle::new(-nose).line_join(round()))
            .outline(&OutlineStyle::new(nose).line_join(round()));
        let lawn: Vec<Shape> = kerb.outline(&OutlineStyle::new(-MEDIAN_KERB).line_join(round()));
        drawn.extend(kerb.iter().cloned());
        for (shapes, builder, color) in [
            (kerb, &mut *kerbs, kerb_color),
            (lawn, &mut *grass, grass_color),
        ] {
            for shape in shapes {
                if shape
                    .first()
                    .is_some_and(|outer| contour_area(outer) >= MIN_LAWN_AREA)
                {
                    push_shape(builder, shape, color);
                }
            }
        }
    }
    drawn
}

/// Насколько асфальт торца полотна тянется к носу соседнего газона, м:
/// отступ носа от торца и его скругление на самом широком полотне.
const BED_CAP: f32 = NOSE_CLEARANCE + NOSE_SHARE * 8.0;

/// Асфальт между торцом трамвайного полотна и носом газона той же пары.
/// Нос отступает от торца на [`NOSE_CLEARANCE`] и скруглён, а между ними
/// ничего не лежало — светлел тротуар половины (у клина он со стороны пары
/// есть) или земля. Торец продлевается на [`BED_CAP`] вперёд за вычетом
/// бордюра газона `kerbs`: трава лежит под асфальтом улиц, и продление
/// поверх съело бы нос. Только у торца, к которому подходит газон.
pub fn bed_caps(median: &Median, kerbs: &[Shape]) -> Vec<Shape> {
    let [first, second] = &median.inner;
    let mut caps = Vec::new();
    for end in [false, true] {
        let (Some((mid, heading)), Some(&a), Some(&b)) = (
            tip_of(&median.midline, end),
            if end { first.last() } else { first.first() },
            if end { second.last() } else { second.first() },
        ) else {
            continue;
        };
        let reach = mid + heading * BED_CAP;
        let near: Vec<Shape> = kerbs
            .iter()
            .filter(|shape| {
                shape.first().is_some_and(|outer| {
                    outer
                        .iter()
                        .any(|point| Vec2::from(*point).distance(reach) < BED_CAP + median.apart())
                })
            })
            .cloned()
            .collect();
        if near.is_empty() {
            continue;
        }
        let back = heading * BED_OVERLAP;
        let forward = heading * BED_CAP;
        let quad = oriented(&[a - back, b - back, b + forward, a + forward], true);
        caps.extend(vec![vec![quad]].overlay(&near, OverlayRule::Difference, FillRule::NonZero));
    }
    caps
}

/// Контуры газона между внутренними кромками половин — по кускам между
/// перекрёстками.
fn lawn_outlines(median: &Median, breaks: &[Break]) -> Vec<Vec<[f32; 2]>> {
    let clear = |at: Vec2| {
        breaks
            .iter()
            .all(|gap| at.distance(gap.at) - gap.reach > NOSE_CLEARANCE)
    };
    let [first, second] = &median.inner;
    let mut outlines = Vec::new();
    let mut start = None;
    for index in 0..=median.midline.len() {
        let open = index < median.midline.len() && clear(median.midline[index]);
        match (open, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                start = None;
                if index - from < 2 || polyline_length(&median.midline[from..index]) < PAIR_MIN {
                    continue;
                }
                let ring: Vec<Vec2> = first[from..index]
                    .iter()
                    .chain(second[from..index].iter().rev())
                    .copied()
                    .collect();
                outlines.push(oriented(&ring, true));
            }
            _ => {}
        }
    }
    outlines
}
