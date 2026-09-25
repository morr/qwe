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
//! - **Трамвайное полотно** ([`carries_tram`]) — разделительная любой ширины,
//!   по которой идёт трамвай: асфальт, как у узкой, но двойные сплошные — по
//!   обеим кромкам, а не по середине.

use bevy::prelude::*;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

use super::network::pairs::{Median, PAIR_MIN};
use super::{RoadJoin, push_ribbon};
use crate::map::footprint::distance_to_polyline;
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::{RailKind, RailLine, polyline_length};
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

/// Доля середины разделительной, на которой рядом лежит трамвай, начиная с
/// которой разделительная — трамвайное полотно ([`carries_tram`]).
const TRAM_SHARE_MIN: f32 = 0.5;
/// Шаг, с которым середина проверяется на трамвай, м.
const TRAM_PROBE_STEP: f32 = 5.0;
/// Насколько внутрь от кромки половины ложится двойная сплошная трамвайного
/// полотна, м: линия — на асфальте полотна, у самой кромки проезжей части.
pub const TRAM_EDGE_INSET: f32 = 0.3;

/// Трамвайное полотно между половинами: на большей части середины в пределах
/// самой разделительной лежит ось `railway=tram`. В OSM такой проспект — две
/// половины и пути между ними в зазоре шире `Median gap` (Советская в Туле,
/// 5 м), и по ширине он читался газоном — лужайкой посреди проспекта, пока на
/// месте трамвайные пути на общем с улицей полотне. Такое полотно мощёное,
/// какой бы ширины ни было, с двойными сплошными **по кромкам**, а не по
/// середине: рельсы — между ними (слой трамвая, выключен по умолчанию).
pub fn carries_tram(median: &Median, rails: &[RailLine]) -> bool {
    let reach = median.apart() / 2.0;
    let points = &median.midline;
    let Some((min, max)) = bounds(points) else {
        return false;
    };
    let near: Vec<&RailLine> = rails
        .iter()
        .filter(|rail| rail.kind == RailKind::Tram)
        .filter(|rail| {
            bounds(&rail.points).is_some_and(|(low, high)| {
                low.cmple(max + reach).all() && high.cmpge(min - reach).all()
            })
        })
        .collect();
    if near.is_empty() {
        return false;
    }
    let probes: Vec<Vec2> = points
        .windows(2)
        .flat_map(|link| {
            let parts = (link[0].distance(link[1]) / TRAM_PROBE_STEP)
                .ceil()
                .max(1.0) as usize;
            (0..parts).map(move |part| link[0].lerp(link[1], part as f32 / parts as f32))
        })
        .collect();
    let covered = probes
        .iter()
        .filter(|&&probe| {
            near.iter()
                .any(|rail| distance_to_polyline(probe, &rail.points) <= reach)
        })
        .count();
    !probes.is_empty() && covered as f32 >= TRAM_SHARE_MIN * probes.len() as f32
}

fn bounds(points: &[Vec2]) -> Option<(Vec2, Vec2)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(low, high), &point| {
        (low.min(point), high.max(point))
    }))
}

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

/// Газон разделительной: бордюр — в `kerbs` (слой тротуаров), трава — в
/// `grass`. `breaks` — разрывы разметки обеих половин.
pub fn push_lawn(
    kerbs: &mut MeshBuilder,
    grass: &mut MeshBuilder,
    median: &Median,
    breaks: &[Break],
    kerb_color: LinearRgba,
    grass_color: LinearRgba,
) {
    let nose = (median.gap * NOSE_SHARE).max(ARC);
    let round = || LineJoin::Round(ARC);
    for outline in lawn_outlines(median, breaks) {
        let kerb: Vec<Shape> = vec![vec![outline]]
            .outline(&OutlineStyle::new(-nose).line_join(round()))
            .outline(&OutlineStyle::new(nose).line_join(round()));
        let lawn: Vec<Shape> = kerb.outline(&OutlineStyle::new(-MEDIAN_KERB).line_join(round()));
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
