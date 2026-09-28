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

use super::network::pairs::{Median, TRAM_BED_MAX_GAP};
use super::node_paint::{ZEBRA_LENGTH, Zebra};
use super::{RoadJoin, push_ribbon};
use crate::map::along::{arclengths, nearest_on_path, place_on_path, tip_of};
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::{closest_on_segment, polyline_length};
use crate::map::shapes::{ARC, Shape, contour_area, oriented, point_in_shape, push_shape};

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
/// Кусок середины между разрывами короче этого, м, газоном не становится.
/// Не длина пары (`PAIR_MIN`): кусок в пять метров между зеброй и узлом —
/// обычный островок газона, а слишком узкое для носа стирает само
/// скругление ([`nosed`]) и площадь ([`MIN_LAWN_AREA`]).
const MIN_LAWN_RUN: f32 = 3.0;
/// Срезанное носом мельче этого, м², асфальтом не кладётся: крошки
/// булевой разности вдоль кромки.
const MIN_CUT_AREA: f32 = 0.05;
/// На сколько бордюр газона раздут, прежде чем его вычесть из асфальта
/// между кромками, м ([`uncovered`]).
const CUT_MARGIN: f32 = 0.05;

/// Разрывы, которые проходят разделительную насквозь: разрыв одной половины,
/// против которого есть разрыв другой. Улица, примыкающая только к ближней
/// половине, разделительную не открывает — ни газон, ни двойную сплошную:
/// дальняя половина идёт мимо, а налево через неё не повернуть. Открывают её
/// поперечная улица и разворот — у обеих половин по узлу напротив друг друга.
///
/// Разрывы лежат на **осях** половин, а оси разнесены на саму разделительную
/// ([`Median::width`]) и полуширины обеих половин `halves`: с одной шириной
/// разделительной разрывы трёхполосных половин напротив друг друга друг
/// друга не видели, и разделительная не открывалась там, где её пересекают.
pub fn crossing_breaks(
    median: &Median,
    halves: [f32; 2],
    [first, second]: [&[Break]; 2],
) -> Vec<Break> {
    let axes = median.width() + halves[0] + halves[1];
    let facing = |gap: &Break, others: &[Break]| {
        others
            .iter()
            .any(|other| gap.at.distance(other.at) <= axes + gap.reach + other.reach)
    };
    first
        .iter()
        .filter(|gap| facing(gap, second))
        .chain(second.iter().filter(|gap| facing(gap, first)))
        .copied()
        .collect()
}

/// Разрывы краски половины `cut`, разделённые на зебры и остальное:
/// `(переходы, прочее)`. Разрыв, через который проходит зебра `zebras`, —
/// переходом шириной в зебру у точки, где она пересекает ось (вместе с самой
/// зеброй); разрыв без зебры (стоп-линия у узла) — в прочее как есть.
///
/// Газон зебра не рвёт, а **прорезает** на свою ширину — проходом через
/// островок ([`push_lawn`]). Разрыв краски у зебры — полдлины зебры,
/// стоп-линия у светофора и запас краски, до девяти-одиннадцати метров, — и
/// нос газона перед ним: две зебры `crossing:island=yes` со сдвигом вдоль оси
/// (Рязань, витрина 03, Вокзальная) с разрывом узла не оставляли газону ни
/// одного куска, и газон разделительной пропадал целиком.
pub fn split_zebras(cut: &[Break], zebras: &[Zebra]) -> (Vec<(Break, Zebra)>, Vec<Break>) {
    let (mut crossings, mut rest) = (Vec::new(), Vec::new());
    for gap in cut {
        let before = crossings.len();
        for zebra in zebras {
            let at = closest_on_segment(gap.at, zebra.from, zebra.to);
            if at.distance(gap.at) < gap.reach {
                let reach = ZEBRA_LENGTH / 2.0;
                crossings.push((Break { at, reach }, *zebra));
            }
        }
        if crossings.len() == before {
            rest.push(*gap);
        }
    }
    (crossings, rest)
}

/// Переходы через разделительную насквозь: зебра одной половины, против
/// которой есть зебра другой — ближе ширины разделительной и двух зебр со
/// сдвигом вдоль оси, — или та же планка через обе. Меряются сами зебры, а не
/// разрывы на осях ([`crossing_breaks`]): у разрыва шириной в зебру ось
/// половины дальше от оси соседки, чем допуск по разрывам, и сдвинутые зебры
/// Вокзальной друг друга не видели.
pub fn facing_zebras(median: &Median, [first, second]: [&[(Break, Zebra)]; 2]) -> Vec<Break> {
    let reach = median.width() + 2.0 * ZEBRA_LENGTH;
    let apart = |a: &Zebra, b: &Zebra| {
        [
            closest_on_segment(a.from, b.from, b.to).distance(a.from),
            closest_on_segment(a.to, b.from, b.to).distance(a.to),
            closest_on_segment(b.from, a.from, a.to).distance(b.from),
            closest_on_segment(b.to, a.from, a.to).distance(b.to),
        ]
        .into_iter()
        .fold(f32::INFINITY, f32::min)
    };
    let facing = |zebra: &Zebra, others: &[(Break, Zebra)]| {
        others.iter().any(|(_, other)| apart(zebra, other) <= reach)
    };
    first
        .iter()
        .filter(|(_, zebra)| facing(zebra, second))
        .chain(second.iter().filter(|(_, zebra)| facing(zebra, first)))
        .map(|&(gap, _)| gap)
        .collect()
}

/// Кусок двойной сплошной между разрывом и торцом (или другим разрывом)
/// короче этого, м, не рисуется — тот же порог, что у линий полос
/// (`node_paint::MIN_RUN`).
const MEDIAN_MIN_RUN: f32 = 6.0;

/// Закрыть разрывом каждый кусок осевой `midline` короче [`MEDIAN_MIN_RUN`]
/// между двумя разрывами `breaks` или между разрывом и торцом. Разрывы узла
/// лежат на осях половин, а середина — в стороне от них: длинный разрыв
/// плеча на пологой крестовине (Орёл, витрина 05) покрывал её не до конца, и
/// посреди поля перекрёстка оставался обрывок двойной сплошной в метр-пять.
/// Осевая, которой не касается ни один разрыв, остаётся как есть.
pub fn bridge_short_pieces(midline: &[Vec2], breaks: &mut Vec<Break>) {
    let (along, total) = arclengths(midline);
    if total <= 0.0 {
        return;
    }
    let mut spans: Vec<(f32, f32)> = breaks
        .iter()
        .filter_map(|gap| {
            let (_, at) = nearest_on_path(midline, gap.at)?;
            Some((at - gap.reach, at + gap.reach))
        })
        .filter(|&(low, high)| high > 0.0 && low < total)
        .collect();
    if spans.is_empty() {
        return;
    }
    spans.extend([(0.0, 0.0), (total, total)]);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = spans[0].1;
    for span in &spans[1..] {
        if span.0 > reach && span.0 - reach < MEDIAN_MIN_RUN {
            let middle = (reach + span.0) / 2.0;
            if let Some((at, _)) = place_on_path(midline, &along, middle) {
                breaks.push(Break {
                    at,
                    reach: (span.0 - reach) / 2.0 + CUT_MARGIN,
                });
            }
        }
        reach = reach.max(span.1);
    }
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
    for end in [false, true] {
        if median.midline().len() < 2 {
            return;
        }
        let Some((tip, heading)) = tip_of(median.midline(), end) else {
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
        if let Some(along) = ahead {
            median.extend(end, along);
        }
    }
}

/// Звено у торца середины короче этого, м, — огрызок шва: у стыка
/// асфальтовой разделительной с газонной той же пары обе начинаются в общей
/// точке, и последнее звено асфальтовой к ней заворачивает.
const SEAM_STUB: f32 = 1.0;
/// Насколько далеко впереди торца двойной сплошной ищется нос газона той же
/// пары, м.
const NOSE_REACH: f32 = 12.0;
/// Шаг, с которым осевая щупает бордюр носа впереди, м.
const NOSE_PROBE: f32 = 0.25;
/// Сколько асфальта двойная сплошная оставляет до бордюра носа, м.
const PAINT_NOSE_CLEARANCE: f32 = 0.5;

/// Двойная сплошная асфальтовой разделительной — к торцу газона той же пары.
///
/// Там, где асфальтовая разделительная переходит в газонную, её середина
/// кончалась в общей точке шва огрызком звена, повёрнутым к ней, и двойная
/// сплошная у носа загибалась крюком; а нос газона стоит на метры дальше
/// шва — асфальт между ними кладёт [`push_lawn`], и линия до носа не
/// доходила. Здесь у торца, впереди которого в [`NOSE_REACH`] лежит бордюр
/// газона `kerbs`, огрызок короче [`SEAM_STUB`] снимается, а линия идёт
/// прямо по ходу до бордюра без [`PAINT_NOSE_CLEARANCE`]. Торец без газона
/// впереди — у перекрёстка — остаётся как был.
pub fn reach_nose(line: &mut Vec<Vec2>, kerbs: &[Shape]) {
    let inside = |at: Vec2| kerbs.iter().any(|shape| point_in_shape(at, shape));
    for end in [false, true] {
        let mut trimmed = line.clone();
        while trimmed.len() > 2 {
            let (tip, before) = if end {
                (trimmed[trimmed.len() - 1], trimmed[trimmed.len() - 2])
            } else {
                (trimmed[0], trimmed[1])
            };
            if tip.distance(before) >= SEAM_STUB {
                break;
            }
            if end {
                trimmed.pop();
            } else {
                trimmed.remove(0);
            }
        }
        let Some((tip, heading)) = tip_of(&trimmed, end) else {
            continue;
        };
        let steps = (NOSE_REACH / NOSE_PROBE) as usize;
        let Some(hit) = (0..=steps)
            .map(|step| step as f32 * NOSE_PROBE)
            .find(|&along| inside(tip + heading * along))
        else {
            continue;
        };
        let reach = hit - PAINT_NOSE_CLEARANCE;
        if reach > 0.0 {
            let point = tip + heading * reach;
            if end {
                trimmed.push(point);
            } else {
                trimmed.insert(0, point);
            }
        }
        *line = trimmed;
    }
}

/// Асфальт узкой разделительной — полосой по середине шириной в саму
/// разделительную ([`Median::width`]), в слой улиц до лент половин, и контуром между
/// внутренними кромками ([`between_edges`]): середина меряется между осями,
/// и у половин разной ширины она ближе к узкой, так что полоса по ней не
/// доставала до кромки широкой там, где зазор разводится (пример 16 Тулы —
/// светлый язык вдоль двойной сплошной).
pub fn push_paved(builder: &mut MeshBuilder, median: &Median, color: LinearRgba, join: RoadJoin) {
    if median.midline().len() < 2 {
        return;
    }
    builder.set_lanes(None);
    push_ribbon(builder, median.midline(), median.width(), color, join);
    if let Some(ring) = between_edges(median, FILL_OVERLAP) {
        builder.push_polygon(&ring, &[], color);
    }
}

/// Нахлёст асфальта разделительной под ленты половин у асфальтовой и
/// газонной, м: кромки середины сняты с оси пробами и прорежены, а лента
/// половины у шва двух разделительных виляет по разводке, и в полуметре от
/// кромки светилась земля или тротуар. Под лентой лишний асфальт не виден.
/// У узла, где половина круто гнётся, нарисованная лента скругляет излом, а
/// кромка середины идёт хордой: при метре нахлёста между ними светился шип
/// тротуара в полметра (Рязань, Вокзальная у Первомайского, витрина 03).
const FILL_OVERLAP: f32 = 2.5;

/// Контур между внутренними кромками половин, с нахлёстом `overlap` под их
/// ленты; `None`, если кромок нет.
///
/// Кромка уходит под **свою** ленту — прочь от кромки напротив, а не от
/// середины: середина лежит посередине между осями, и у половин разной
/// ширины кромка широкой заходит за неё. Уводимая от середины, эта кромка
/// шла под чужую ленту, контур ложился целиком под узкую половину, и зазор
/// между половинами светился землёй — иглой при нахлёсте в метр, плашкой в
/// два метра при нынешнем (Орёл, витрина 05: Московская, 10.9 и 7.6 м).
fn between_edges(median: &Median, overlap: f32) -> Option<Vec<Vec2>> {
    edges_ring(median.midline(), median.inner(), overlap)
}

/// [`between_edges`] по середине `midline` и кромкам `inner` в тех же точках.
fn edges_ring(midline: &[Vec2], inner: &[Vec<Vec2>; 2], overlap: f32) -> Option<Vec<Vec2>> {
    let [first, second] = inner;
    let count = midline.len();
    if count < 2 || first.len() != count || second.len() != count {
        return None;
    }
    // куда от кромки её лента: прочь от кромки напротив; кромки сошлись в
    // точку — прочь от середины
    let widened = |edge: &[Vec2], facing: &[Vec2]| -> Vec<Vec2> {
        edge.iter()
            .zip(facing)
            .zip(midline)
            .map(|((&point, &other), &mid)| {
                let away = (point - other)
                    .try_normalize()
                    .unwrap_or_else(|| (point - mid).normalize_or_zero());
                point + away * overlap
            })
            .collect()
    };
    Some(
        widened(first, second)
            .into_iter()
            .chain(widened(second, first).into_iter().rev())
            .collect(),
    )
}

/// Нахлёст асфальта полотна под ленты половин, м: край, совпадающий с
/// краем ленты, но не делящий с ней вершин, растеризуется с пропусками.
const BED_OVERLAP: f32 = 0.05;

/// Асфальт трамвайного полотна — внутренние полосы обеих половин: от
/// внутренней кромки одной до внутренней кромки другой, с нахлёстом
/// [`FILL_OVERLAP`] под их ленты, в слой улиц до лент половин. Контуром, а не
/// лентой по середине: ширина идёт за кромками, где зазор гуляет, а торцы
/// ровные — круглый торец ленты ложился поверх носа соседнего газона.
/// Раскладки полос у него нет: колея — только на автомобильных полосах.
pub fn push_bed(builder: &mut MeshBuilder, median: &Median, color: LinearRgba) {
    let Some(ring) = between_edges(median, FILL_OVERLAP) else {
        return;
    };
    builder.set_lanes(None);
    builder.push_polygon(&ring, &[], color);
}

/// Торцы трамвайного полотна — разрывами для соседнего газона: нос газона
/// встаёт за [`NOSE_CLEARANCE`] до торца, как у перекрёстка.
pub fn bed_ends(median: &Median) -> [Option<Break>; 2] {
    [false, true].map(|end| tip_of(median.midline(), end).map(|(at, _)| Break { at, reach: 0.0 }))
}

/// Газон разделительной: бордюр — в `kerbs` (слой тротуаров), трава — в
/// `grass`, а всё между внутренними кромками половин, что не газон, —
/// асфальтом в `streets` (слой улиц, под лентами половин). `breaks` —
/// разрывы обеих половин, у которых газон кончается носом; `crossings` —
/// зебры через обе половины ([`split_zebras`]): газон ими не рвётся, а
/// прорезается проходом ([`passages`]). Возвращает контуры бордюра — к ним
/// подходит асфальт торца трамвайного полотна ([`bed_caps`]).
///
/// **Не газон — асфальт.** Газон рвётся у перекрёстка, а нос — морфологическое
/// открытие контура: всё, что у́же двух радиусов носа, стирается целиком, а
/// у конца куска, где зазор между половинами только разводится до ширины
/// газона (`Pairs::align`) или половины сходятся к узлу, это метры клина. Под
/// ним не лежало ничего — светился тротуар половины или земля: светлый язык
/// за носом (пример 16 Тулы), бледный клин на поле перекрёстка (Калуга,
/// Кирова × Плеханова).
pub fn push_lawn(
    kerbs: &mut MeshBuilder,
    grass: &mut MeshBuilder,
    streets: &mut MeshBuilder,
    median: &Median,
    [breaks, crossings]: [&[Break]; 2],
    [kerb_color, grass_color, road_color]: [LinearRgba; 3],
) -> Vec<Shape> {
    let round = || LineJoin::Round(ARC);
    let mut drawn = Vec::new();
    let mut visible = Vec::new();
    let passages = passages(median, crossings);
    for (outline, length) in lawn_outlines(median, breaks) {
        // нос — почти полукруг во всю ширину, а у куска короче ширины — по
        // его длине: открытие стирает всё у́же двух радиусов, и островок
        // между зеброй и узлом пропадал целиком (Рязань, витрина 03)
        let nose = (median.gap().min(length) * NOSE_SHARE).max(ARC);
        let kerb = nosed(outline, nose);
        let lawn: Vec<Shape> = kerb
            .outline(&OutlineStyle::new(-MEDIAN_KERB).line_join(round()))
            .into_iter()
            .filter(is_drawn)
            .collect();
        // бордюр без травы внутри — бледный обрубок на поле перекрёстка
        // (Орёл, витрина 04): не рисуется, и под ним ляжет асфальт
        let kerb: Vec<Shape> = kerb
            .into_iter()
            .filter(is_drawn)
            .filter(|shape| {
                lawn.iter().any(|grass| {
                    grass
                        .first()
                        .and_then(|outer| outer.first())
                        .is_some_and(|point| point_in_shape(Vec2::from(*point), shape))
                })
            })
            .collect();
        // проход по зебре — плиткой бордюра поперёк: трава вырезана
        let lawn: Vec<Shape> = if passages.is_empty() {
            lawn
        } else {
            lawn.overlay(&passages, OverlayRule::Difference, FillRule::NonZero)
                .into_iter()
                .filter(is_drawn)
                .collect()
        };
        visible.extend(kerb.iter().cloned());
        drawn.extend(kerb.iter().cloned());
        for (shapes, builder, color) in [
            (kerb, &mut *kerbs, kerb_color),
            (lawn, &mut *grass, grass_color),
        ] {
            for shape in shapes {
                push_shape(builder, shape, color);
            }
        }
    }
    if let Some(ring) = between_edges(median, FILL_OVERLAP) {
        let ring = oriented(&ring, true);
        streets.set_lanes(None);
        for cut in uncovered(ring, visible) {
            push_shape(streets, cut, road_color);
        }
    }
    drawn
}

/// Проходы через газон по зебрам `crossings` (разрывы шириной в зебру на
/// осях половин): прямоугольник поперёк разделительной во всю её ширину, на
/// длину зебры вдоль середины. Из травы вычитается, бордюр остаётся — проход
/// лежит плиткой островка между двумя кусками газона.
fn passages(median: &Median, crossings: &[Break]) -> Vec<Shape> {
    let midline = median.midline();
    let (along, _) = arclengths(midline);
    let across = median.width();
    crossings
        .iter()
        .filter_map(|gap| {
            let (at, length) = nearest_on_path(midline, gap.at)?;
            let (_, heading) = place_on_path(midline, &along, length)?;
            let [ahead, aside] = [heading * gap.reach, heading.perp() * across];
            let quad = [
                at - ahead - aside,
                at + ahead - aside,
                at + ahead + aside,
                at - ahead + aside,
            ];
            Some(vec![oriented(&quad, true)])
        })
        .collect()
}

/// Кусочек газона или бордюра, который рисуется: не мельче [`MIN_LAWN_AREA`].
fn is_drawn(shape: &Shape) -> bool {
    shape
        .first()
        .is_some_and(|outer| contour_area(outer) >= MIN_LAWN_AREA)
}

/// Контур газона `outline`, скруглённый в нос радиуса `nose`: открытие —
/// внутрь и обратно.
fn nosed(outline: Vec<[f32; 2]>, nose: f32) -> Vec<Shape> {
    let round = || LineJoin::Round(ARC);
    vec![vec![outline]]
        .outline(&OutlineStyle::new(-nose).line_join(round()))
        .outline(&OutlineStyle::new(nose).line_join(round()))
}

/// Что от контура `ring` между кромками остаётся за вычетом бордюров газона
/// `kerbs`, — без крошек мельче [`MIN_CUT_AREA`].
fn uncovered(ring: Vec<[f32; 2]>, kerbs: Vec<Shape>) -> Vec<Shape> {
    let whole: Vec<Shape> = vec![vec![ring]];
    let cuts = if kerbs.is_empty() {
        whole
    } else {
        // бордюр чуть шире себя: вдоль газона кромки кольца и бордюра
        // совпадают, и разность оставляла нитки асфальта — слой улиц лежит
        // выше газона, и нитки читались штрихами по бордюру
        let kerbs = kerbs.outline(&OutlineStyle::new(CUT_MARGIN).line_join(LineJoin::Round(ARC)));
        whole.overlay(&kerbs, OverlayRule::Difference, FillRule::NonZero)
    };
    cuts.into_iter()
        .filter(|cut| {
            cut.first()
                .is_some_and(|outer| contour_area(outer) >= MIN_CUT_AREA)
        })
        .collect()
}

/// Насколько асфальт торца полотна тянется к носу соседнего газона, м:
/// отступ носа от торца и его скругление на самом широком полотне
/// ([`TRAM_BED_MAX_GAP`]).
const BED_CAP: f32 = NOSE_CLEARANCE + NOSE_SHARE * TRAM_BED_MAX_GAP;
/// Насколько продление торца полотна без газона впереди шире кромок, м.
const BED_WIDER: f32 = 1.0;

/// Асфальт между торцом трамвайного полотна и носом газона той же пары.
/// Нос отступает от торца на [`NOSE_CLEARANCE`] и скруглён, а между ними
/// ничего не лежало — светлел тротуар половины (у клина он со стороны пары
/// есть) или земля. Торец продлевается на [`BED_CAP`] вперёд за вычетом
/// бордюра газона `kerbs`: трава лежит под асфальтом улиц, и продление
/// поверх съело бы нос. У торца без газона рядом — продление целиком.
pub fn bed_caps(median: &Median, kerbs: &[Shape]) -> Vec<Shape> {
    let [first, second] = median.inner();
    let mut caps = Vec::new();
    for end in [false, true] {
        let (Some((mid, heading)), Some(&a), Some(&b)) = (
            tip_of(median.midline(), end),
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
                        .any(|point| Vec2::from(*point).distance(reach) < BED_CAP + median.width())
                })
            })
            .cloned()
            .collect();
        let back = heading * BED_OVERLAP;
        if near.is_empty() {
            // газона впереди нет — его срезали разрывы узла, и между торцом
            // полотна и узлом светилась земля (Орёл, витрина 04, квадрат у
            // торца полотна): продление — на ширину полотна дальше, до лент
            // узла, и шире кромок: за торцом половины расходятся
            let forward = heading * (BED_CAP + median.width());
            let wider = (a - b).normalize_or_zero() * BED_WIDER;
            let [a, b] = [a + wider, b - wider];
            caps.push(vec![oriented(
                &[a - back, b - back, b + forward, a + forward],
                true,
            )]);
            continue;
        }
        let forward = heading * BED_CAP;
        let quad = oriented(&[a - back, b - back, b + forward, a + forward], true);
        caps.extend(vec![vec![quad]].overlay(&near, OverlayRule::Difference, FillRule::NonZero));
    }
    caps
}

/// Шаг, до которого догущаются середина и кромки перед поиском кусков газона,
/// м ([`lawn_outlines`]).
const LAWN_STEP: f32 = 1.0;
/// Насколько ось половины может отстоять от кромки газона, м, — полуширина
/// шестиполосной половины с запасом: разрыв дальше сбоку газон не режет.
const BREAK_ASIDE: f32 = 12.0;

/// Середина и обе кромки разделительной, догущённые до шага [`LAWN_STEP`] —
/// каждое звено делится на одно и то же число частей во всех трёх, так что
/// точки остаются друг против друга. И по точке — своя ли это вершина, а не
/// вставленная: в контур газона идут только свои и концы куска.
fn densified(median: &Median) -> ([Vec<Vec2>; 3], Vec<bool>) {
    let [first, second] = median.inner();
    let midline = median.midline();
    let lines = [midline, first.as_slice(), second.as_slice()];
    let mut dense: [Vec<Vec2>; 3] = Default::default();
    let mut own = Vec::new();
    let count = lines.iter().map(|line| line.len()).min().unwrap_or(0);
    for index in 0..count {
        for (line, out) in lines.iter().zip(dense.iter_mut()) {
            out.push(line[index]);
        }
        own.push(true);
        if index + 1 == count {
            break;
        }
        let parts = (midline[index].distance(midline[index + 1]) / LAWN_STEP)
            .ceil()
            .max(1.0);
        for part in 1..parts as usize {
            let t = part as f32 / parts;
            for (line, out) in lines.iter().zip(dense.iter_mut()) {
                out.push(line[index].lerp(line[index + 1], t));
            }
            own.push(false);
        }
    }
    (dense, own)
}

/// Контуры газона между внутренними кромками половин — по кускам между
/// перекрёстками.
///
/// Куски ищутся по точкам середины, **догущённой** ([`densified`]): вершины
/// OSM на прямом проспекте стоят в десятках метров друг от друга, и когда
/// первая за узлом попадала в разрыв перекрёстка, а следующая была в
/// шестидесяти метрах, газона на этом пролёте не было вовсе — между
/// половинами лежал голый асфальт без осевой (Калуга, витрина 02, восточный
/// луч Кирова).
///
/// Каждый контур — с длиной своего куска по середине: нос короткого куска
/// скругляется по длине, а не по ширине ([`push_lawn`]).
fn lawn_outlines(median: &Median, breaks: &[Break]) -> Vec<(Vec<[f32; 2]>, f32)> {
    let ([midline, first, second], own) = densified(median);
    // Разрыв лежит на оси половины, в стороне от середины, и меряется **вдоль**
    // неё: по прямой до его центра круг разрыва накрывал середину на пару
    // метров короче, чем ось, и нос газона въезжал между зебрами (Калуга, 02).
    // Сбоку — не дальше оси половины: разрыв своей пары, а не колена
    // разделительной за поворотом.
    let aside_max = median.width() / 2.0 + BREAK_ASIDE;
    let clear = |index: usize| {
        let at = midline[index];
        let ahead = midline[(index + 1).min(midline.len() - 1)];
        let behind = midline[index.saturating_sub(1)];
        let heading = (ahead - behind).normalize_or_zero();
        breaks.iter().all(|gap| {
            let offset = at - gap.at;
            if heading == Vec2::ZERO {
                return offset.length() - gap.reach > NOSE_CLEARANCE;
            }
            offset.dot(heading.perp()).abs() > aside_max
                || offset.dot(heading).abs() - gap.reach > NOSE_CLEARANCE
        })
    };
    let mut outlines = Vec::new();
    let mut start = None;
    for index in 0..=midline.len() {
        let open = index < midline.len() && clear(index);
        match (open, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                start = None;
                let length = polyline_length(&midline[from..index]);
                if index - from < 2 || length < MIN_LAWN_RUN {
                    continue;
                }
                // вставленные точки на прямом звене контуру не нужны
                let kept: Vec<usize> = (from..index)
                    .filter(|&at| at == from || at + 1 == index || own[at])
                    .collect();
                let ring: Vec<Vec2> = kept
                    .iter()
                    .map(|&at| first[at])
                    .chain(kept.iter().rev().map(|&at| second[at]))
                    .collect();
                outlines.push((oriented(&ring, true), length));
            }
            _ => {}
        }
    }
    outlines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::shapes::shape_area;

    fn rect(min: Vec2, max: Vec2) -> Shape {
        let ring = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        vec![oriented(&ring, true)]
    }

    /// Шов асфальтовой разделительной с газонной (Советская в Туле, пример
    /// 16): огрызок звена у торца, повёрнутый к общей точке, снимается, и
    /// двойная сплошная идёт прямо до бордюра носа — без крюка.
    #[test]
    fn the_median_line_drops_the_seam_stub_and_reaches_the_nose() {
        let kerb = rect(Vec2::new(24.0, -2.0), Vec2::new(40.0, 2.0));
        let mut line = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.5, 0.3)];
        reach_nose(&mut line, &[kerb]);
        assert!(line.iter().all(|at| at.y.abs() < 1e-3), "крюк: {line:?}");
        let tip = *line.last().unwrap();
        assert!(
            (tip.x - (24.0 - PAINT_NOSE_CLEARANCE)).abs() <= NOSE_PROBE + 1e-3,
            "линия кончается не у носа: {tip}"
        );
        // начало, перед которым газона нет, — как было
        assert_eq!(line[0], Vec2::ZERO);
    }

    #[test]
    fn a_median_line_without_a_lawn_ahead_stays_as_it_was() {
        let far = rect(Vec2::new(40.0, -2.0), Vec2::new(60.0, 2.0));
        let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.5, 0.3)];
        let mut line = original.clone();
        reach_nose(&mut line, &[far]);
        assert_eq!(line, original);
    }

    /// Между кромками половин всё, что не газон, — асфальт: срезанный носом
    /// клин и кусок у перекрёстка, до которого газон не доходит.
    #[test]
    fn what_the_lawn_leaves_between_the_edges_is_asphalt() {
        let ring = oriented(
            &[
                Vec2::ZERO,
                Vec2::new(40.0, 0.0),
                Vec2::new(40.0, 4.0),
                Vec2::new(0.0, 4.0),
            ],
            true,
        );
        let kerb = rect(Vec2::new(10.0, 0.0), Vec2::new(40.0, 4.0));
        let cuts = uncovered(ring, vec![kerb]);
        let area: f32 = cuts.iter().map(shape_area).sum();
        // бордюр раздут на `CUT_MARGIN`
        assert!(
            (area - (10.0 - CUT_MARGIN) * 4.0).abs() < 0.1,
            "асфальта {area} м²"
        );
    }

    /// Разделительная в 4 м между трёхполосными половинами (10.9 м): оси
    /// разнесены на 14.9 м. Поперечная двухполосная улица рвёт обе оси на
    /// 4.8 м — друг против друга, и разделительную она открывает; с одной
    /// шириной разделительной (4 + 4.8 + 4.8 < 14.9) разрывы друг друга не
    /// видели. Улица, примыкающая к одной половине в 40 м от перекрёстка, —
    /// нет.
    #[test]
    fn breaks_face_each_other_across_the_axes_of_wide_halves() {
        let at = |y: f32| vec![Vec2::new(0.0, y), Vec2::new(100.0, y)];
        let median = Median::lawn_for_test(at(0.0), [at(-2.0), at(2.0)]);
        let halves = [10.9 / 2.0; 2];
        let cross = |y: f32| Break {
            at: Vec2::new(50.0, y),
            reach: 4.8,
        };
        let aside = Break {
            at: Vec2::new(90.0, -7.45),
            reach: 4.8,
        };
        let found = crossing_breaks(&median, halves, [&[cross(-7.45), aside], &[cross(7.45)]]);
        assert_eq!(found, vec![cross(-7.45), cross(7.45)]);
    }

    /// Половины 10.9 и 7.6 м, оси в 10.8 м: середина между осями в 5.4 от
    /// широкой, и её кромка (5.45) зашла за середину. Контур асфальта кроет
    /// зазор между кромками целиком и заходит под обе ленты; уводимая от
    /// середины, кромка широкой шла под узкую, и зазор светился землёй
    /// (Орёл, витрина 05).
    #[test]
    fn the_paved_fill_covers_the_gap_when_the_wide_edge_crosses_the_midline() {
        let xs = [0.0, 20.0];
        let at = |y: f32| xs.map(|x| Vec2::new(x, y)).to_vec();
        let midline = at(5.4);
        let inner = [at(5.45), at(7.0)];
        let ring = edges_ring(&midline, &inner, FILL_OVERLAP).expect("кромки есть");
        let (low, high) = ring
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), p| {
                (low.min(p.y), high.max(p.y))
            });
        assert!(
            low <= 5.45 - FILL_OVERLAP + 1e-3 && high >= 7.0 + FILL_OVERLAP - 1e-3,
            "контур {low}..{high} не кроет зазор 5.45..7.0 с нахлёстом"
        );
    }

    /// Разрыв краски со стоп-линией и зеброй — переходом шириной в зебру там,
    /// где она пересекает ось; разрыв без зебры остаётся как был.
    #[test]
    fn a_paint_break_with_a_zebra_becomes_a_zebra_wide_crossing() {
        let zebra = Zebra {
            from: Vec2::new(12.0, -6.0),
            to: Vec2::new(12.0, 6.0),
            osm: true,
        };
        let cut = [
            Break {
                at: Vec2::new(10.0, 0.0),
                reach: 5.5,
            },
            Break {
                at: Vec2::new(60.0, 0.0),
                reach: 3.0,
            },
        ];
        let (crossings, rest) = split_zebras(&cut, &[zebra]);
        assert_eq!(
            crossings,
            vec![(
                Break {
                    at: Vec2::new(12.0, 0.0),
                    reach: ZEBRA_LENGTH / 2.0
                },
                zebra
            )]
        );
        assert_eq!(rest, vec![cut[1]]);
    }

    /// Обрывок осевой между разрывом узла (он лежит на оси половины, в трёх
    /// метрах сбоку) и торцом — тоже разрыв; длинный кусок по другую сторону
    /// остаётся, осевая без разрывов — тоже.
    #[test]
    fn a_median_stub_past_a_break_is_bridged() {
        let midline = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
        let mut breaks = vec![Break {
            at: Vec2::new(14.0, 3.0),
            reach: 5.0,
        }];
        bridge_short_pieces(&midline, &mut breaks);
        assert_eq!(breaks.len(), 2, "{breaks:?}");
        let stub = breaks[1];
        assert!(
            stub.at.x - stub.reach <= 19.0 && stub.at.x + stub.reach >= 20.0,
            "{stub:?}"
        );
        let mut none = Vec::new();
        bridge_short_pieces(&[Vec2::ZERO, Vec2::new(4.0, 0.0)], &mut none);
        assert!(none.is_empty());
    }
}
