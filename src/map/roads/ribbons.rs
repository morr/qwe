//! Лента дороги и полоса тротуара при ней: обёртки над `MeshBuilder`, которыми
//! слой дорог кладёт свои ленты — стыком [`ROAD_JOIN`], с торцами, срезанными
//! клином (`roads/tapers.rs`), кусками полосы тротуара по сторонам пары и
//! клиньями на швах земля/мост (`roads/bridges.rs`). Подложка аллей
//! (`map::spawn`) зовёт [`push_ribbon`] — у неё те же три настройки, что у
//! дорог, и мапиться на `MeshBuilder` они обязаны одинаково.

use bevy::prelude::*;

use super::network::pairs::BandPiece;
use super::shape::{ROAD_JOIN, RoadJoin};
use super::tapers;
use crate::map::meshing::{MeshBuilder, RibbonCap, miter_offsets};
use crate::map::osm::model::polyline_length;
use crate::map::shapes::is_ring;

/// Лента выбранного стиля — `RoadJoin` через [`RoadJoin::ribbon_shape`]. Общая
/// с подложкой аллей (`map::spawn`): у неё те же три настройки, что у дорог, и
/// мапиться на `MeshBuilder` они обязаны одинаково.
///
/// Слои с **жёстко заданным** стыком (ограда, рельсы, трамвай) зовут
/// `MeshBuilder::push_ribbon` напрямую: обёртка им говорила бы только «переведи
/// `RoadJoin::Round` в `RibbonJoin::Round`», то есть выдавала бы стиль дорог за
/// их собственный.
///
/// **Замкнутый путь рисуется замкнутой лентой** — признак берётся из самого
/// пути ([`is_ring`]), а не аргументом: у кольца торцов нет, и решать это за
/// ленту нечем, кроме её же формы. Иначе на шве ложились два торцевых
/// полудиска поверх собственного асфальта кольца, и внутри такого диска
/// разметка с износом считались в **замороженной** раме торца — круглое пятно
/// со смещёнными штрихами (отчёт автора по кольцу ТРЦ «Макси»).
pub fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    push_ribbon_trimmed(builder, points, width, color, join, [false; 2]);
}

/// Лента дороги, у которой торец `[начало, конец]` может быть **срезан под
/// клин** (`roads/tapers.rs`): такой торец кончается ровно на своей точке.
/// Круглый торец полной ширины выпер бы из-под клина, который в этом месте
/// начинается с той же ширины и дальше только сужается.
pub(super) fn push_ribbon_trimmed(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
    trimmed: [bool; 2],
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    let caps = trimmed.map(|trimmed| if trimmed { RibbonCap::Butt } else { cap });
    builder.push_ribbon_capped(points, is_ring(points), width, color, join, caps);
}

/// Полуширины клина для `MeshBuilder::push_taper_sided` — `[[слева в начале,
/// слева в конце], [справа …]]` в раме **пути клина** (от шва к телу):
/// `half(сторона по ходу way)` — полуширина `[узкого соседа, своя]` с той
/// стороны; сужаемая сторона (`sides`, `roads/tapers.rs`) идёт от первой ко
/// второй, сохранённая — своя на всём клине. У торца конца путь идёт против
/// way, и стороны меняются местами.
pub(super) fn wedge_halves(
    sides: [bool; 2],
    end: bool,
    half: impl Fn(usize) -> [f32; 2],
) -> [[f32; 2]; 2] {
    let by_side = [0, 1].map(|side| {
        let [narrow, own] = half(side);
        if sides[side] { [narrow, own] } else { [own; 2] }
    });
    if end {
        [by_side[1], by_side[0]]
    } else {
        by_side
    }
}

/// Середина клина длиной `wedge` у начала (`end == false`) или конца пути
/// длиной `length`, м по оси ленты: по ней ищется пара клина половины.
pub(super) fn wedge_middle(length: f32, wedge: f32, end: bool) -> f32 {
    if end {
        length - wedge / 2.0
    } else {
        wedge / 2.0
    }
}

/// Тротуар дороги шириной `widths[0]` с полосой `widths[1]` — кусками
/// `pieces` (`Pairs::band_pieces`: стороны по тегу, без стороны пары), или,
/// при `None`, одной лентой с обеих сторон.
pub(super) fn push_sidewalk(
    builder: &mut MeshBuilder,
    body: &[Vec2],
    widths: [f32; 2],
    pieces: Option<&[BandPiece]>,
    color: LinearRgba,
    trimmed: [bool; 2],
) {
    push_sidewalk_to_seams(builder, body, widths, pieces, color, trimmed, [None; 2]);
}

/// Длина клина, которым полоса тротуара сходит в бортик настила на шве
/// земля/мост ([`push_sidewalk_to_seams`]), м.
pub(super) const BAND_SEAM_TAPER: f32 = 4.0;
/// На сколько полоса тротуара на шве земля/мост заходит под настил, м
/// ([`push_sidewalk_to_seams`]): закрывает излом шва.
pub(super) const BAND_SEAM_OVERLAP: f32 = 0.5;
/// Полудлина тихого разрыва линий полос у половины слияния в узле шва
/// земля/мост, м (S3).
pub(super) const SEAM_DASH_GAP: f32 = 1.0;
/// Насколько асфальтовый фартук под торцом настила-улицы заходит на подход,
/// м (S3): закрывает излом шва, где асфальт подхода срезан прямо.
pub(super) const DECK_SEAM_APRON: f32 = 0.5;

/// [`push_sidewalk`], у которого на торцах `seam_reach` (наружная кромка
/// бортика настила от оси, `bridges::seam_curb_reach`) полоса последние
/// [`BAND_SEAM_TAPER`] м сходит клином к этой кромке: полоса шире бортика по
/// построению, и её прямой торец у головы моста стоял уступом (S1, S5).
pub(super) fn push_sidewalk_to_seams(
    builder: &mut MeshBuilder,
    body: &[Vec2],
    [width, sidewalk]: [f32; 2],
    pieces: Option<&[BandPiece]>,
    color: LinearRgba,
    trimmed: [bool; 2],
    seam_reach: [Option<f32>; 2],
) {
    let band = width / 2.0 + sidewalk;
    let total = polyline_length(body);
    // клин — только где кромка бортика ближе кромки полосы, и не длиннее
    // половины дороги с каждого конца
    let tapers = seam_reach.map(|reach| {
        reach
            .filter(|&reach| reach < band && total > 0.0)
            .map(|reach| (reach.max(width / 2.0), BAND_SEAM_TAPER.min(total / 2.0)))
    });
    if pieces.is_none() && seam_reach == [None; 2] {
        return push_ribbon_trimmed(builder, body, 2.0 * band, color, ROAD_JOIN, trimmed);
    }
    let whole = [(0.0, total, [true, true])];
    let pieces = pieces.unwrap_or(&whole);
    let lead = tapers[0].map_or(0.0, |(_, length)| length);
    let tail = total - tapers[1].map_or(0.0, |(_, length)| length);
    // полуширина полосы с тротуаром в точке `at` клина
    let half_at = |at: f32| match (tapers[0], tapers[1]) {
        (Some((reach, length)), _) if at < lead => reach + (band - reach) * at / length,
        (_, Some((reach, length))) if at > tail => reach + (band - reach) * (total - at) / length,
        _ => band,
    };
    for &(from, to, sides) in pieces {
        for (start, end) in [(from, to.min(lead)), (from.max(tail), to)] {
            if end - start <= f32::EPSILON || sides == [false; 2] {
                continue;
            }
            // контуром, а не `push_taper_sided`: тот сливает точки ближе
            // полуширины, и клин в 4 м при полосе в 10 м схлопывался в точку
            let path = tapers::cut(body, start, end);
            let (along, _) = crate::map::along::arclengths(&path);
            let miters = miter_offsets(&path, false, 1.0);
            let side_half = |side: usize, at: f32| {
                if sides[side] {
                    half_at(start + at)
                } else {
                    width / 2.0
                }
            };
            let mut outline: Vec<Vec2> = path
                .iter()
                .zip(&miters)
                .zip(&along)
                .map(|((point, miter), &at)| *point + *miter * side_half(0, at))
                .collect();
            outline.extend(
                path.iter()
                    .zip(&miters)
                    .zip(&along)
                    .rev()
                    .map(|((point, miter), &at)| *point - *miter * side_half(1, at)),
            );
            builder.push_polygon(&outline, &[], color);
        }
        let (from, to) = (from.max(lead), to.min(tail));
        if to - from <= f32::EPSILON {
            continue;
        }
        push_band_piece(
            builder,
            body,
            [width, sidewalk],
            (from, to, sides),
            color,
            [from > 0.0 || trimmed[0], to < total || trimmed[1]],
        );
    }
    // губа под настил: прямой торец полосы стоит поперёк своей оси, а торец
    // настила — поперёк своей, и на изломе шва между ними светилась нитка
    // земли — наискось от кромки через крайнюю полосу (S3, Орёл, мост Р-119:
    // 1.2° на 8.7 м — 0.19 м). Губа лежит под бортиком: не шире его кромки
    let last = body.len().saturating_sub(1);
    if last == 0 {
        return;
    }
    for (end, reach) in seam_reach.into_iter().enumerate() {
        let Some(reach) = reach else { continue };
        let covering = pieces.iter().find(|&&(from, to, _)| {
            if end == 0 {
                from <= 1e-3
            } else {
                to >= total - 1e-3
            }
        });
        let Some(&(_, _, sides)) = covering else {
            continue;
        };
        let (tip, inner) = if end == 0 {
            (body[0], body[1])
        } else {
            (body[last], body[last - 1])
        };
        let Some(out) = (tip - inner).try_normalize() else {
            continue;
        };
        // «влево» по ходу точек полосы
        let left = if end == 0 { -out.perp() } else { out.perp() };
        let half = |side: usize| {
            if sides[side] {
                reach.min(band).max(width / 2.0)
            } else {
                width / 2.0
            }
        };
        let (a, b) = (tip + left * half(0), tip - left * half(1));
        let lip = out * BAND_SEAM_OVERLAP;
        builder.push_polygon(&[a, b, b + lip, a + lip], &[], color);
    }
}

/// Кусок `piece` полосы тротуара — с обеих сторон или со своей.
fn push_band_piece(
    builder: &mut MeshBuilder,
    body: &[Vec2],
    [width, sidewalk]: [f32; 2],
    (from, to, [left, right]): BandPiece,
    color: LinearRgba,
    trims: [bool; 2],
) {
    let points = tapers::cut(body, from, to);
    // полоса с одной стороны — лента на полтротуара в её сторону:
    // `miter_offsets` плюсом сдвигает влево
    let shift = match (left, right) {
        (true, true) => {
            return push_ribbon_trimmed(
                builder,
                &points,
                width + 2.0 * sidewalk,
                color,
                ROAD_JOIN,
                trims,
            );
        }
        (true, false) => sidewalk / 2.0,
        (false, true) => -sidewalk / 2.0,
        (false, false) => return,
    };
    let shifted: Vec<Vec2> = points
        .iter()
        .zip(miter_offsets(&points, false, shift))
        .map(|(point, offset)| *point + offset)
        .collect();
    push_ribbon_trimmed(builder, &shifted, width + sidewalk, color, ROAD_JOIN, trims);
}
