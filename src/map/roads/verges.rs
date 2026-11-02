//! Обочины дороги: плитка у бордюра и газон за ней, по профилю ширины
//! ([`RoadLine::verge_profile`]), со швами там, где одна переходит в другую.
//! Ступень зума — [`VergeZoomBucket`]: на средней и дальней широкая обочина
//! не рисуется вовсе.

use bevy::prelude::*;

use super::ROAD_JOIN;
use super::SIDEWALK_COLOR;
use super::drawn::Drawn;
use super::ribbons::push_ribbon_trimmed;
use super::scraps;
use super::walls::Meadows;
use super::{DrawnPiece, PieceKind};
use crate::map::meshing::{MeshBuilder, miter_offsets};
use crate::map::osm::RoadLine;
use crate::map::osm::model::polyline_length;
use crate::map::spawn::GRASS_COLOR;
use crate::map::spawn::RESIDENTIAL_COLOR;
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::settings::{Z_ROAD_VERGE, Z_ROAD_VERGE_LAWN, Z_ROAD_VERGE_YARD};

/// Шаг вершин обочины по месту ([`RoadLine::verge_profile`]), м: пробы
/// стоят через пять.
const VERGE_STEP: f32 = 2.5;

/// Обочина шире этого, м, — газон, а не плитка ([`paved_verge`]): у
/// многоэтажки между бордюром и тротуаром лежит газон, а сплошная плитка в
/// пятнадцать метров до дома «заливала улицу бетоном» (Фрунзе в Туле,
/// районный кадр d2). Уже — плитка до дорожки, как у углов центра.
pub(super) const VERGE_PAVED_MAX: f32 = 4.0;

/// Обочина шире этого, м, — на средней и дальней ступени зума
/// ([`VergeLods`]) не рисуется вовсе: ни плиткой у бордюра, ни своим
/// газоном (`road_verge_lawns`/`road_verge_yards`), — остаётся газон или
/// земля, лежащие под ними. Полоса в 5–11 м приглушённой травы двора
/// рядом с проезжей читалась как второе полотно улицы (Первомайская у
/// парка в Туле, NEW-1: «двойная Первомайская»). На ближней ступени
/// рисуется всё, как раньше. Порог между «узкой» и «широкой» здесь свой,
/// не [`VERGE_PAVED_MAX`]: до 4 м — плитка целиком и на среднем зуме
/// (две полосы плитки у кромки — тротуар, не полотно), 4–5 м — как на
/// ближней, полоса у бордюра с газоном.
pub(super) const VERGE_MID_MAX: f32 = 5.0;

/// Газон широкой обочины — приглушённая трава двора (`SurfaceKind::Yard`,
/// слой `road_verge_yards`), а не луг разделительной: обочина у многоэтажки —
/// край того же двора, и светлый луг (`GRASS_COLOR`) лежал вдоль улиц яркой
/// лентой со швом на кромке квартала (районный кадр d2). Фактура по мировой
/// позиции та же, что у двора, поэтому шва нет вовсе. Спрашивать двор было
/// мало: у площади, стоянки или голой земли за обочиной луг оставался
/// салатовой лентой (восточная сторона Фрунзе, d2). Лугом газон ложится
/// только у замапленного газона или сквера ([`Meadows`]) — как они сами
/// (Тула, 15: сквер с лугами на плитке).
pub(super) const VERGE_YARD_COLOR: Color = RESIDENTIAL_COLOR;
/// Доли длины дороги, на которых обочина спрашивает газон ([`Meadows::beside`]).
pub(super) const MEADOW_PROBES: [f32; 3] = [0.25, 0.5, 0.75];
/// Насколько за обочиной, м, ищется газон, который кончается у дорожки.
pub(super) const MEADOW_PROBE_BEYOND: f32 = 3.0;
/// На сколько обочина по месту заходит за торец своей улицы, м
/// ([`push_verges`]): внахлёст с обочиной продолжения.
pub(super) const VERGE_END_OVERLAP: f32 = 0.3;
/// Полоса плитки у бордюра перед газоном широкой обочины, м.
pub(super) const VERGE_KERB: f32 = 0.5;
/// На сколько обочина заходит за ось своей дорожки, м ([`push_verges`]).
/// Профиль обочины снят разбором по **сырым** осям улицы и дорожки, а лента
/// ляжет по сглаженным: у Орджоникидзе (Орёл, у Ляшко) сглаженная ось улицы
/// ушла от сырой на 1.7 м, и край плитки, стоявший на оси дорожки, отошёл от
/// её ленты (полуширина 1.5 м) — вдоль дорожки светилась нить земли в
/// десяток сантиметров (S6). Под дорожкой край не виден, а за кварталом
/// плитка лежит под ним (`Z_ROAD_VERGE` < `Z_LANDUSE`).
pub(super) const VERGE_UNDER_PATH: f32 = 0.5;
/// Самый короткий кусок обочины по месту одного рода — газон или плитка —
/// между кусками другого, м ([`verge_runs`]): короче — он того же рода, что
/// соседи. Профиль, колеблющийся у [`VERGE_PAVED_MAX`], резал плитку зубцами.
const VERGE_RUN_MIN: f32 = 10.0;
/// Полуширина шва между плиткой и газоном вдоль обочины, м: плитка кончается
/// поперёк улицы, а не косой.
pub(super) const VERGE_SEAM: f32 = 0.05;

/// Ступени зума обочин: до [`VERGE_ZOOM_MAX`] — ближняя, обочины рисуются
/// как рассчитаны; дальше — широкие ([`VERGE_MID_MAX`]) не рисуются. Порог
/// совпадает с ближней границей крон (`trees::TREE_LODS`): крона в 2–6 м на
/// этом зуме — 1–3 px, и обочинная полоса той же ширины читается уже не
/// деталью улицы, а полотном рядом с ней.
pub enum VergeLods {}

impl ZoomLods for VergeLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [VERGE_ZOOM_MAX, f32::INFINITY].into_iter()
    }
}

/// Верхняя (исключающая) граница ближней ступени обочин, м на логический
/// пиксель — как у крон ([`VergeLods`]).
pub(super) const VERGE_ZOOM_MAX: f32 = 2.0;

/// Текущая ступень [`VergeLods`]; её пересечение переключает видимость между
/// двумя заранее собранными наборами слоёв ([`super::show_road_steps`]), а не
/// пересобирает дорогу — в [`super::rebuilds_on`] ступень не входит.
pub type VergeZoomBucket = ZoomBucket<VergeLods>;

impl VergeZoomBucket {
    /// Рисуются ли на этой ступени широкие обочины ([`VERGE_MID_MAX`]) —
    /// вопрос одной таблицы, и лента с углами спрашивает его здесь, а не
    /// своим индексом.
    pub fn draws_wide_verges(self) -> bool {
        self.index == 0
    }
}

/// Сколько обочины шириной `verge` мостится плиткой от кромки: узкая —
/// целиком, широкая — полосой [`VERGE_KERB`] у бордюра, а газон под ней
/// ([`push_verges`]) — до дорожки. Её же вырезает из кварталов разбор
/// (`osm/parse/verges.rs`): плитка обочины — не двор.
///
/// **Ступенью, без перехода.** Плитка сходила на полосу у бордюра за 2 м
/// лишней ширины, и там, где дорожка медленно отходит от улицы, её кромка
/// шла косой через всю обочину — тонкий косой клин газона между плиткой и
/// дорожкой, а где профиль колебался у 4 м — зубцы (Орёл, витрина 03). Где
/// плитка кончается, решает [`verge_runs`] — швом поперёк улицы.
pub(crate) fn paved_verge(verge: f32) -> f32 {
    if verge <= VERGE_PAVED_MAX {
        verge
    } else {
        VERGE_KERB
    }
}

/// Обочина по месту — газон или плитка на каждой точке `dense` (с длинами
/// `along` и обочинами `widths`): газон, где обочина шире
/// [`VERGE_PAVED_MAX`], без кусков короче [`VERGE_RUN_MIN`] между кусками
/// другого рода (концевые куски остаются — у торца угол узла). На каждой
/// смене рода вставляется пара точек в [`VERGE_SEAM`] от места, где обочина
/// проходит 4 м, — плитка кончается там поперечным швом. Возвращает точки,
/// обочины и род каждой точки.
fn verge_runs(dense: &[Vec2], along: &[f32], widths: &[f32]) -> (Vec<Vec2>, Vec<f32>, Vec<bool>) {
    let count = dense.len();
    let mut lawn: Vec<bool> = widths
        .iter()
        .map(|&verge| verge > VERGE_PAVED_MAX)
        .collect();
    // куски одного рода: [начало, конец] по индексам точек
    let runs = |lawn: &[bool]| -> Vec<(usize, usize)> {
        let mut runs = Vec::new();
        let mut start = 0;
        for index in 1..=count {
            if index == count || lawn[index] != lawn[start] {
                runs.push((start, index - 1));
                start = index;
            }
        }
        runs
    };
    // длина куска — от середины шва до середины шва
    let seam = |index: usize| (along[index] + along[index + 1]) / 2.0;
    loop {
        let found = runs(&lawn);
        let shortest = found[1..found.len().saturating_sub(1).max(1)]
            .iter()
            .map(|&(start, end)| (seam(end) - seam(start - 1), start, end))
            .filter(|&(length, ..)| length < VERGE_RUN_MIN)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, start, end)) = shortest else {
            break;
        };
        for flag in &mut lawn[start..=end] {
            *flag = !*flag;
        }
    }
    let mut points = Vec::with_capacity(count + 4);
    let mut verges = Vec::with_capacity(count + 4);
    let mut kinds = Vec::with_capacity(count + 4);
    for index in 0..count {
        points.push(dense[index]);
        verges.push(widths[index]);
        kinds.push(lawn[index]);
        if index + 1 == count || lawn[index] == lawn[index + 1] {
            continue;
        }
        let [from, to] = [widths[index], widths[index + 1]];
        let length = (dense[index + 1] - dense[index]).length();
        // шов — там, где обочина проходит 4 м; у перекрашенного куска её
        // перехода нет, и шов — посредине
        let crossing = if from != to && (from - VERGE_PAVED_MAX) * (to - VERGE_PAVED_MAX) <= 0.0 {
            (VERGE_PAVED_MAX - from) / (to - from)
        } else {
            0.5
        };
        let margin = (VERGE_SEAM / length.max(f32::EPSILON)).min(0.25);
        let crossing = crossing.clamp(2.0 * margin, 1.0 - 2.0 * margin);
        for (t, kind) in [
            (crossing - margin, lawn[index]),
            (crossing + margin, lawn[index + 1]),
        ] {
            points.push(dense[index].lerp(dense[index + 1], t));
            verges.push(from + (to - from) * t);
            kinds.push(kind);
        }
    }
    (points, verges, kinds)
}

/// Обочины дороги `road`, нарисованной по `points`, — по ленте на сторону: от
/// оси до кромки плюс обочина, круглыми торцами. `verges` — какие стороны
/// рисуются (`Drawn::verges_drawn`). Газон и плитка уходят ещё и в `scraps`
/// (`roads/scraps.rs`); `ends` — какие торцы `[начало, конец]` кончаются в
/// узле (`KerbReturns::butt`): лоскуты ищутся только у них; `bites` — вершины
/// ([`scraps::Bites`]), где чужое мощение врезается в полосу газона в
/// середине улицы, — там окно своё. Обочина по месту
/// ([`RoadLine::verge_at`]) — полосой переменной ширины от оси до края, и у
/// каждого торца — круг торцевой ширины: торцом она доходит до угла узла, как
/// постоянная лента.
///
/// Плитка — в `tiles` на ширину [`paved_verge`], газон — на всю обочину, где
/// она шире [`VERGE_PAVED_MAX`]: в `meadows` лугом, если у этой стороны
/// замапленный газон ([`Meadows::beside`]), иначе в `yard_lawns` травой
/// двора. Слои газона лежат под плиткой, и голой земли между кромкой и
/// дорожкой не остаётся.
/// Обочина до отдельного тротуара — с какой стороны и сколько рисуется
/// (`prepared.verges_drawn`); у дуги кольца — только снаружи: внутри остров
/// и его газон, а снаружи между тротуаром кольца и дорожкой вдоль него
/// оставалась голая земля (Калуга, витрина 01, СВ)
pub(super) fn street_verged(prepared: &Drawn, index: usize, ring_ccw: Option<bool>) -> [f32; 2] {
    let mut verged = prepared.verges_drawn(index);
    if let Some(ccw) = ring_ccw {
        // остров — слева по ходу против часовой стрелки
        verged[usize::from(!ccw)] = 0.0;
    }
    verged
}

#[allow(clippy::too_many_arguments)]
pub(super) fn push_verges(
    [tiles, meadows, yard_lawns]: [&mut MeshBuilder; 3],
    mapped: &Meadows,
    road_index: u32,
    road: &RoadLine,
    points: &[Vec2],
    ends: [bool; 2],
    bites: &[Vec2],
    verges: [f32; 2],
    scraps: &mut scraps::Scraps,
    pieces: &mut Vec<DrawnPiece>,
) {
    let width = road.width;
    let [tile_color, grass_color, yard_color] =
        [SIDEWALK_COLOR, GRASS_COLOR, VERGE_YARD_COLOR].map(|color| color.to_linear());
    let raw = polyline_length(&road.points);
    for (side, verge) in verges.into_iter().enumerate() {
        if verge <= 0.0 {
            continue;
        }
        // газон рядом спрашивается, только когда газон обочины вообще будет
        let widest = road.verge_widest(side);
        let on_lawn = widest > VERGE_PAVED_MAX && mapped.beside(points, width, side, verge);
        let (lawns, lawn_color) = if on_lawn {
            (&mut *meadows, grass_color)
        } else {
            (&mut *yard_lawns, yard_color)
        };
        // находка следует за лентой: луг ложится в `road_verge_lawns`,
        // трава двора — в `road_verge_yards`
        let (lawn_kind, lawn_z) = if on_lawn {
            (PieceKind::VergeLawn, Z_ROAD_VERGE_LAWN)
        } else {
            (PieceKind::VergeYard, Z_ROAD_VERGE_YARD)
        };
        // `miter_offsets` плюсом сдвигает влево — сторона 0
        let sign = if side == 0 { 1.0 } else { -1.0 };
        let ribbon = |builder: &mut MeshBuilder, path: &[Vec2], verge: f32, color: LinearRgba| {
            let shifted: Vec<Vec2> = path
                .iter()
                .zip(miter_offsets(path, false, sign * verge / 2.0))
                .map(|(point, offset)| *point + offset)
                .collect();
            push_ribbon_trimmed(
                builder,
                &shifted,
                width + verge,
                color,
                ROAD_JOIN,
                [false; 2],
            );
        };
        // до дорожки — и под неё на [`VERGE_UNDER_PATH`]; полоса у бордюра
        // перед газоном — своей шириной
        let under = |verge: f32| {
            let paved = paved_verge(verge);
            if paved < verge {
                paved
            } else {
                verge + VERGE_UNDER_PATH
            }
        };
        if road.verge_profile[side].is_empty() || points.len() < 2 {
            ribbon(tiles, points, under(verge), tile_color);
            // лоскутам — те же полосы от оси за кромку, по точкам через шаг
            let dense = crate::map::along::densify(points, VERGE_STEP);
            let normals = miter_offsets(&dense, false, sign);
            let reach = |verge: f32| vec![width / 2.0 + verge; dense.len()];
            scraps.push_paving_band(&dense, &normals, &reach(under(verge)));
            // находке — видимая полоса: от кромки полотна до внешней кромки
            // плитки; под дорогой кусок не виден, и находке он не нужен
            let count = points.len();
            let point_normals = miter_offsets(points, false, sign);
            push_band_piece(
                pieces,
                points,
                &point_normals,
                &vec![width / 2.0; count],
                &vec![width / 2.0 + under(verge); count],
                PieceKind::VergePaving,
                road_index,
                Z_ROAD_VERGE,
            );
            if verge > VERGE_PAVED_MAX {
                let lawn = verge + VERGE_UNDER_PATH;
                ribbon(lawns, points, lawn, lawn_color);
                scraps.push_lawn_band(road_index, &dense, &normals, &reach(lawn), ends, bites);
                // видимый газон начинается за кромкой плитки
                push_band_piece(
                    pieces,
                    points,
                    &point_normals,
                    &vec![width / 2.0 + under(verge); count],
                    &vec![width / 2.0 + lawn; count],
                    lawn_kind,
                    road_index,
                    lawn_z,
                );
            }
            continue;
        }
        // торцы — продлённые на `VERGE_END_OVERLAP` по касательной: у стыка
        // двух way одной улицы край каждой обочины кончался своим перпендикуляром,
        // и между ними светилась нить (Орёл, витрина 04, север). Круга торцевой
        // ширины здесь нет и не было: огрызок в сантиметр, из которого его
        // строили, лента сливала в точку (`merge_ribbon_points`)
        let mut dense = crate::map::along::densify(points, VERGE_STEP);
        let count = dense.len();
        let [head, tail] = [
            (dense[0] - dense[1]).normalize_or_zero(),
            (dense[count - 1] - dense[count - 2]).normalize_or_zero(),
        ];
        dense.insert(0, dense[0] + head * VERGE_END_OVERLAP);
        dense.push(dense[count] + tail * VERGE_END_OVERLAP);
        let (along, total) = crate::map::along::arclengths(&dense);
        let scale = raw / total.max(f32::EPSILON);
        let widths: Vec<f32> = along
            .iter()
            .map(|&at| road.verge_at(side, at * scale))
            .collect();
        let (dense, widths, lawns_at) = verge_runs(&dense, &along, &widths);
        let normals = miter_offsets(&dense, false, sign);
        let band = |widths: &mut dyn Iterator<Item = f32>| -> Vec<Vec2> {
            let mut outline: Vec<Vec2> = dense
                .iter()
                .zip(&normals)
                .zip(widths)
                .map(|((&point, &normal), verge)| point + normal * (width / 2.0 + verge))
                .collect();
            outline.extend(dense.iter().rev());
            outline
        };
        let paved: Vec<f32> = widths
            .iter()
            .zip(&lawns_at)
            .map(|(&verge, &lawn)| {
                if lawn {
                    VERGE_KERB.min(verge)
                } else {
                    verge + VERGE_UNDER_PATH
                }
            })
            .collect();
        let widths: Vec<f32> = widths
            .iter()
            .map(|verge| verge + VERGE_UNDER_PATH)
            .collect();
        // кромки лент от оси — общие для меша, лоскутов и находки
        let edge = |verges: &[f32]| -> Vec<f32> {
            verges.iter().map(|verge| width / 2.0 + verge).collect()
        };
        let paved_edge = edge(&paved);
        tiles.push_polygon(&band(&mut paved.iter().copied()), &[], tile_color);
        push_band_piece(
            pieces,
            &dense,
            &normals,
            &vec![width / 2.0; dense.len()],
            &paved_edge,
            PieceKind::VergePaving,
            road_index,
            Z_ROAD_VERGE,
        );
        scraps.push_paving_band(&dense, &normals, &paved_edge);
        if lawns_at.iter().any(|&lawn| lawn) {
            lawns.push_polygon(&band(&mut widths.iter().copied()), &[], lawn_color);
            let lawn_edge = edge(&widths);
            scraps.push_lawn_band(road_index, &dense, &normals, &lawn_edge, ends, bites);
            // видимый газон — от внешней кромки плитки до внешней кромки
            // газона: между ними полоса целиком газонная
            push_band_piece(
                pieces,
                &dense,
                &normals,
                &paved_edge,
                &lawn_edge,
                lawn_kind,
                road_index,
                lawn_z,
            );
        }
    }
}

/// Видимая полоса куска обочины для находки: между внутренней и внешней
/// кромками по `normals` (единичные сдвиги в сторону обочины) вдоль `path`.
/// Рисуется лента от оси, под дорогой кусок не виден, — находке полоса
/// режется от кромки полотна.
#[allow(clippy::too_many_arguments)]
fn push_band_piece(
    pieces: &mut Vec<DrawnPiece>,
    path: &[Vec2],
    normals: &[Vec2],
    inner: &[f32],
    outer: &[f32],
    kind: PieceKind,
    road: u32,
    z: f32,
) {
    let mut contour: Vec<Vec2> = path
        .iter()
        .zip(normals)
        .zip(outer)
        .map(|((&point, &normal), &distance)| point + normal * distance)
        .collect();
    contour.extend(
        path.iter()
            .zip(normals)
            .zip(inner)
            .map(|((&point, &normal), &distance)| point + normal * distance)
            .rev(),
    );
    if contour.len() >= 3 {
        pieces.push(DrawnPiece {
            contour,
            holes: Vec::new(),
            kind,
            road: Some(road),
            z,
        });
    }
}

#[cfg(test)]
mod tests;
