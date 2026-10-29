//! **Полоса над рельсами** — светлый асфальт вдоль трамвайного пути там, где
//! путь лежит под асфальтом улицы.
//!
//! Рельсы на карте не рисуются (слой трамвая выключен), а полоса, по которой
//! идёт трамвай, всё равно видна: на Яндексе она залита чуть более светлым
//! асфальтом по всей линии — и между половинами проспекта (трамвайное
//! полотно, `roads/network/pairs.rs`), и по оси одиночной улицы. Разметки у
//! неё нет: трамвайная полоса читается цветом, а не линией.
//!
//! Путь ощупывается каждые [`PROBE_STEP`]; проба под асфальтом, если рядом
//! идёт улица **вдоль** пути ([`ALONG_MIN`]) и полоса целиком ложится на её
//! ленту — или проба на мощёной разделительной пары. Трамвай, пересекающий
//! улицу поперёк, полосы не даёт: это переезд, а не полоса. Куски вне
//! асфальта (своё полотно, парки) не рисуются, кусок короче [`MIN_RUN`] —
//! тоже.
//!
//! **Настил улицы — тоже асфальт** (S1, S2): покрытие считается одно через
//! шов земля/мост и режется настилами, что несут полосу ([`split_cover`]), —
//! наземная часть ложится в слой улиц, настильная в слой настилов. Пока
//! полосы на настиле и на земле строились порознь, их покрытия на шве
//! наезжали друг на друга со сдвигом (Красный мост в Орле), а у моста без
//! разделительной полоса обрывалась у голов косым светлым параллелограммом
//! (1-я Курская). Линия разреза одна на обе части — ни щели, ни нахлёста.

use std::borrow::Cow;

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, OutlineStyle};

use super::TRAM_BAND_COLOR;
use super::bridges::Bridges;
use super::medians::MedianDrawing;
use super::network::pairs::Median;
use super::tram_lay::{TramTracks, lay_tracks};
use crate::map::along::simplify;
use crate::map::grid::Grid;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{
    RailKind, RailLine, distance_to_segment, polyline_length, ring_bounds,
};
use crate::map::osm::{MapData, RoadClass, RoadLine};
use crate::map::shapes::{ARC, Contour, Shape, push_shape, stroke};

/// На сколько контур разделительной двух настилов шире её самой, когда он
/// режет полосу над рельсами ([`deck_cuts`]), м: кромки половин
/// лежат ровно на её краю, и без запаса вдоль них оставались нитки.
const DECK_CUT_SLACK: f32 = 0.2;

/// Ширина полосы над одним путём, м: полоса движения. Два пути в 3–4 м друг
/// от друга сливаются в одну полосу — нахлёст в одном слое не виден.
pub const TRAM_BAND_WIDTH: f32 = 3.3;
/// Шаг, с которым путь ощупывается на асфальт, м.
const PROBE_STEP: f32 = 2.0;
/// Насколько полоса может выйти за кромку ленты улицы, м: путь в OSM лежит
/// не точно по оси полосы.
const EDGE_SLACK: f32 = 0.6;
/// Косинус угла между путём и улицей, при котором путь идёт вдоль неё.
const ALONG_MIN: f32 = 0.8;
/// Насколько основание пробы может выйти за торец звена улицы, м: на изломе
/// оси снаружи угла основание не попадает ни на одно из звеньев.
const LINK_SLACK: f32 = 1.0;
/// Кусок полосы короче этого, м, не рисуется.
const MIN_RUN: f32 = 12.0;
/// Допуск, с которым выборка пути прореживается обратно, м: проба через
/// [`PROBE_STEP`] — вершина ленты, а на прямой их столько не нужно (без
/// прореживания полоса стоила 65 тысяч вершин на Тулу).
const SIMPLIFY_TOLERANCE: f32 = 0.05;
/// Ячейка сетки звеньев улиц, м.
const CELL: f32 = 32.0;
/// Непокрытый участок пути между двумя кусками полосы короче этого, м,
/// зарастает, если весь лежит у асфальта: трамвай между половинами
/// проспекта проходит узел, где разделительная кончилась, а до лент половин
/// от путей больше полуширины — без моста полоса рвалась посреди перекрёстка.
const BRIDGE_MAX: f32 = 40.0;
/// Насколько проба моста может отойти от ленты улицы, м, оставаясь у
/// асфальта: пути посреди узла разделённой улицы — в 3 м от кромок половин.
const BRIDGE_SLACK: f32 = 4.0;

/// Куски трамвайных путей, лежащие под асфальтом: по улицам `roads`,
/// нарисованным по `paths`, и по мощёным разделительным `medians`.
pub fn tram_bands(
    rails: &[RailLine],
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    medians: &[Median],
) -> Vec<Vec<Vec2>> {
    if !rails.iter().any(|rail| rail.kind == RailKind::Tram) {
        return Vec::new();
    }
    // ячейки, по которым идёт трамвай: сетку звеньев улиц незачем строить
    // по всему городу ради шестидесяти километров путей
    let mut tracks: Grid<()> = Grid::new(CELL);
    for rail in rails.iter().filter(|rail| rail.kind == RailKind::Tram) {
        for pair in rail.points.windows(2) {
            tracks.insert_segment(pair[0], pair[1], 0.0, ());
        }
    }
    // звено и полуширина асфальта вокруг него
    let mut links: Grid<(Vec2, Vec2, f32)> = Grid::new(CELL);
    for (road, path) in roads.iter().zip(paths) {
        let path = path.as_ref();
        if road.class != RoadClass::Street || road.passage || path.is_empty() {
            continue;
        }
        let half = road.width / 2.0;
        let (low, high) = ring_bounds(path);
        if tracks.near_each(low - half, high + half).next().is_none() {
            continue;
        }
        for pair in path.windows(2) {
            links.insert_segment(
                pair[0],
                pair[1],
                half + BRIDGE_SLACK,
                (pair[0], pair[1], half),
            );
        }
    }
    // мощёная разделительная — вместе с внутренними полосами половин по
    // бокам: полоса над путём у самой кромки заходит на них, а они асфальт
    for median in medians.iter().filter(|median| median.is_paved()) {
        let half = median.width() / 2.0 + TRAM_BAND_WIDTH;
        for pair in median.midline().windows(2) {
            links.insert_segment(
                pair[0],
                pair[1],
                half + BRIDGE_SLACK,
                (pair[0], pair[1], half),
            );
        }
    }
    let covered = |at: Vec2, heading: Vec2| {
        links.at(at).iter().any(|&(from, to, half)| {
            let Some(direction) = (to - from).try_normalize() else {
                return false;
            };
            if direction.dot(heading).abs() < ALONG_MIN {
                return false;
            }
            // основание — на звене, а не за его торцом: за торцом улицы путь
            // уже на своём полотне
            let t = (at - from).dot(direction);
            if !(-LINK_SLACK..=from.distance(to) + LINK_SLACK).contains(&t) {
                return false;
            }
            (from + direction * t).distance(at) + TRAM_BAND_WIDTH / 2.0 <= half + EDGE_SLACK
        })
    };
    // у асфальта — в любую сторону от звена, для моста через узел
    let near_asphalt = |at: Vec2| {
        links
            .at(at)
            .iter()
            .any(|&(from, to, half)| distance_to_segment(at, from, to) <= half + BRIDGE_SLACK)
    };
    let mut bands = Vec::new();
    for rail in rails.iter().filter(|rail| rail.kind == RailKind::Tram) {
        let mut run: Vec<Vec2> = Vec::new();
        // непокрытые пробы после куска: мост, пока он может зарасти
        let mut gap: Vec<Vec2> = Vec::new();
        let mut bridgeable = true;
        let mut flush = |run: &mut Vec<Vec2>| {
            if run.len() >= 2 && polyline_length(run) >= MIN_RUN {
                let kept = simplify(run, false, SIMPLIFY_TOLERANCE, |_| false);
                bands.push(kept.into_iter().map(|index| run[index]).collect());
            }
            run.clear();
        };
        for pair in rail.points.windows(2) {
            let Some(heading) = (pair[1] - pair[0]).try_normalize() else {
                continue;
            };
            let steps = (pair[0].distance(pair[1]) / PROBE_STEP).ceil().max(1.0) as usize;
            let from = usize::from(!run.is_empty());
            for step in from..=steps {
                let at = pair[0].lerp(pair[1], step as f32 / steps as f32);
                if covered(at, heading) {
                    if !gap.is_empty() && bridgeable {
                        run.append(&mut gap);
                    }
                    gap.clear();
                    bridgeable = true;
                    run.push(at);
                } else if !run.is_empty() {
                    gap.push(at);
                    bridgeable &= near_asphalt(at) && polyline_length(&gap) < BRIDGE_MAX;
                    if !bridgeable {
                        flush(&mut run);
                        gap.clear();
                    }
                }
            }
        }
        flush(&mut run);
    }
    bands
}

/// Щель между полосами уже этого, м, зарастает ([`band_cover`]).
const CLOSE_GAP: f32 = 3.0;

/// Покрытие полосами `bands` одной фигурой, со щелями между ними заросшими:
/// у поворота, где путь отходит от прямого, у стыка кусков соседних путей,
/// оборванных на разной высоте, между полосами оставался клин обычного
/// асфальта — на светлой полосе он читался дырой (пример 5). Замыкание:
/// каждая полоса обводится шире на полщели с каждой стороны и длиннее на
/// столько же с каждого торца, обводы сливаются и сжимаются обратно — края и
/// торцы там, где были, а щели уже [`CLOSE_GAP`] нет.
pub fn band_cover(bands: &[Vec<Vec2>]) -> Vec<Shape> {
    let reach = CLOSE_GAP / 2.0;
    let contours: Vec<Contour> = bands
        .iter()
        .filter(|band| band.len() >= 2)
        .flat_map(|band| {
            let mut band = band.clone();
            let last = band.len() - 1;
            if let Some(back) = (band[0] - band[1]).try_normalize() {
                band[0] += back * reach;
            }
            if let Some(ahead) = (band[last] - band[last - 1]).try_normalize() {
                band[last] += ahead * reach;
            }
            stroke(&band, TRAM_BAND_WIDTH + CLOSE_GAP, LineCap::Butt, false)
        })
        .collect();
    if contours.is_empty() {
        return Vec::new();
    }
    let merged: Vec<Shape> = contours.simplify_shape(FillRule::NonZero);
    merged.outline(&OutlineStyle::new(-reach).line_join(LineJoin::Round(ARC)))
}

/// Контуры настилов `decks` (ось, как она нарисована, и ширина обводки, м),
/// по которым **вдоль** идёт полоса `bands`, — с прямыми торцами на их
/// головах: линия разреза покрытия ([`split_cover`]). Настил путепровода над
/// трамваем, идущим поперёк, полосы не несёт — и не режет её: иначе кусок
/// наземной полосы под ним лёг бы в слой настилов поверх путепровода.
pub fn deck_cuts<'a>(
    bands: &[Vec<Vec2>],
    decks: impl IntoIterator<Item = (&'a [Vec2], f32)>,
) -> Vec<Contour> {
    let links: Vec<(Vec2, Vec2)> = bands
        .iter()
        .flat_map(|band| band.windows(2).map(|pair| (pair[0], pair[1])))
        .collect();
    if links.is_empty() {
        return Vec::new();
    }
    let mut cuts = Vec::new();
    for (path, width) in decks {
        if path.len() < 2 {
            continue;
        }
        let half = width / 2.0;
        let (low, high) = ring_bounds(path);
        let (low, high) = (low - half, high + half);
        let carries = links.iter().any(|&(from, to)| {
            let middle = (from + to) / 2.0;
            let Some(heading) = (to - from).try_normalize() else {
                return false;
            };
            middle.cmpge(low).all()
                && middle.cmple(high).all()
                && path.windows(2).any(|pair| {
                    (pair[1] - pair[0]).try_normalize().is_some_and(|along| {
                        along.dot(heading).abs() >= ALONG_MIN
                            && distance_to_segment(middle, pair[0], pair[1]) <= half
                    })
                })
        });
        if carries {
            cuts.extend(stroke(path, width, LineCap::Butt, false));
        }
    }
    cuts
}

/// Покрытие `cover`, разрезанное контурами настилов `cuts` ([`deck_cuts`]):
/// `[на земле, на настилах]`. Обе части режет одна и та же линия — торец
/// настила, — так что на шве они сходятся встык.
pub fn split_cover(cover: Vec<Shape>, cuts: &[Contour]) -> [Vec<Shape>; 2] {
    if cuts.is_empty() || cover.is_empty() {
        return [cover, Vec::new()];
    }
    let clip: Vec<Shape> = cuts.iter().map(|cut| vec![cut.clone()]).collect();
    let on_decks = cover.overlay(&clip, OverlayRule::Intersect, FillRule::NonZero);
    let on_ground = cover.overlay(&clip, OverlayRule::Difference, FillRule::NonZero);
    [on_ground, on_decks]
}

/// Светлая полоса над рельсами — поверх всего
/// асфальта улиц: порядок пуша в слое — порядок отрисовки, а краска лежит
/// своим слоем выше. По путям, уложенным по нарисованной улице
/// (`roads/tram_lay.rs`), а не по OSM: полоса обязана лежать под тем же
/// рельсом, что рисует трамвай. Одним покрытием через шов земля/мост, со
/// щелями между полосами соседних путей заросшими, — и разрезанным
/// торцами настилов, что несут полосу: на настиле она лежит в его слое
/// (S1, S2: два покрытия порознь на шве наезжали друг на друга со
/// сдвигом, у моста без разделительной полоса обрывалась у голов косым
/// параллелограммом)
pub(super) fn lay_tram_bands(
    map: &MapData,
    drawn: &[&RoadLine],
    nodal: &[Cow<'_, [Vec2]>],
    ribbon: &[Cow<'_, [Vec2]>],
    medians: &MedianDrawing,
    bridges: &mut Bridges,
    streets: &mut MeshBuilder,
) -> (TramTracks, usize) {
    let tram_tracks = lay_tracks(&map.rails, drawn, nodal, &medians.paved);
    let tram_bands = tram_bands(&tram_tracks.0, drawn, nodal, &medians.paved);
    let street_decks = drawn
        .iter()
        .enumerate()
        .filter(|(_, road)| road.bridge && road.class == RoadClass::Street && !road.passage)
        .map(|(index, road)| (ribbon[index].as_ref(), 2.0 * road.curb_reach()));
    // разделительная двух настилов лежит в слое настилов (R30) — и полоса
    // над ней; ровно по её ширине: внутренние полосы половин режут их настилы
    let median_decks = medians
        .paved
        .iter()
        .filter(|median| median.roads().iter().all(|&road| drawn[road].bridge))
        .map(|median| (median.midline(), median.width() + DECK_CUT_SLACK));
    let cuts = deck_cuts(&tram_bands, street_decks.chain(median_decks));
    let [on_ground, on_decks] = split_cover(band_cover(&tram_bands), &cuts);
    streets.set_lanes(None);
    for shape in on_ground {
        push_shape(streets, shape, TRAM_BAND_COLOR.to_linear());
    }
    let fills = bridges.fills();
    fills.set_lanes(None);
    for shape in on_decks {
        push_shape(fills, shape, TRAM_BAND_COLOR.to_linear());
    }
    (tram_tracks, tram_bands.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    fn tram(points: Vec<Vec2>) -> RailLine {
        RailLine {
            kind: RailKind::Tram,
            ..fixture::rail(points, 1.2)
        }
    }

    #[test]
    fn a_tram_along_a_street_is_banded_and_one_across_it_is_not() {
        let road = fixture::street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 14.0);
        let paths = [road.points.clone()];
        let along = tram(vec![Vec2::new(-50.0, 1.5), Vec2::new(250.0, 1.5)]);
        let across = tram(vec![Vec2::new(100.0, -60.0), Vec2::new(100.0, 60.0)]);
        let bands = tram_bands(&[along, across], &[&road], &paths, &[]);
        assert_eq!(bands.len(), 1, "полоса только вдоль улицы: {bands:?}");
        let band = &bands[0];
        assert!(band.iter().all(|at| (at.y - 1.5).abs() < 1e-3));
        // и только над асфальтом: путь дальше торцов улицы — своё полотно
        assert!(
            band[0].x >= -1.0 && band[band.len() - 1].x <= 201.0,
            "{band:?}"
        );
        assert!(band[band.len() - 1].x - band[0].x > 190.0);
    }

    /// Щель между полосами соседних путей зарастает, а края и торцы стоят,
    /// где стояли; далёкие полосы остаются порознь.
    #[test]
    fn the_cover_closes_a_narrow_gap_and_keeps_the_ends() {
        use crate::map::shapes::contour_bounds;
        let band = |y: f32| vec![Vec2::new(0.0, y), Vec2::new(50.0, y)];
        let cover = band_cover(&[band(0.0), band(5.0)]);
        assert_eq!(cover.len(), 1, "щель 1.7 м заросла: {cover:?}");
        assert_eq!(cover[0].len(), 1, "без дырок: {cover:?}");
        let (low, high) = contour_bounds(&cover[0][0]);
        let half = TRAM_BAND_WIDTH / 2.0;
        assert!(low.abs_diff_eq(Vec2::new(0.0, -half), 0.05), "{low}");
        assert!(
            high.abs_diff_eq(Vec2::new(50.0, 5.0 + half), 0.05),
            "{high}"
        );
        assert_eq!(band_cover(&[band(0.0), band(10.0)]).len(), 2);
    }

    /// Путь проходит узел, где лента вдоль него прервалась: короткий участок
    /// у асфальта зарастает мостом, длинный вне асфальта рвёт полосу.
    #[test]
    fn a_band_bridges_a_junction_but_not_open_ground() {
        let street = |from: f32, to: f32| {
            fixture::street(vec![Vec2::new(from, 0.0), Vec2::new(to, 0.0)], 14.0)
        };
        let track = tram(vec![Vec2::new(0.0, 1.5), Vec2::new(200.0, 1.5)]);
        // узел: улица вдоль прервана на 25 м, поперёк идёт другая
        let (west, east) = (street(0.0, 80.0), street(105.0, 200.0));
        let across = fixture::street(vec![Vec2::new(92.0, -60.0), Vec2::new(92.0, 60.0)], 14.0);
        let roads = [&west, &east, &across];
        let paths: Vec<_> = roads.iter().map(|road| road.points.clone()).collect();
        let bands = tram_bands(std::slice::from_ref(&track), &roads, &paths, &[]);
        assert_eq!(bands.len(), 1, "узел зарос: {bands:?}");
        assert!(polyline_length(&bands[0]) > 190.0);
        // пустырь в 60 м между кусками улицы — не узел
        let (west, east) = (street(0.0, 70.0), street(130.0, 200.0));
        let roads = [&west, &east];
        let paths: Vec<_> = roads.iter().map(|road| road.points.clone()).collect();
        assert_eq!(tram_bands(&[track], &roads, &paths, &[]).len(), 2);
    }

    /// Путь идёт с улицы на её мост: покрытие одно через шов, и разрезано
    /// торцом настила — наземная часть кончается, а настильная начинается
    /// ровно на голове, и вместе они — всё покрытие, без щели и нахлёста
    /// (S1, Красный мост в Орле; S2, мост 1-й Курской).
    #[test]
    fn the_band_cover_is_one_shape_cut_at_the_deck_head() {
        use crate::map::shapes::{contour_bounds, shape_area};
        let head = Vec2::new(100.0, 0.0);
        let street = fixture::street(vec![Vec2::ZERO, head], 14.0);
        let deck = fixture::bridge(vec![head, Vec2::new(200.0, 0.0)], 14.0);
        let paths = [street.points.clone(), deck.points.clone()];
        let track = tram(vec![Vec2::new(-50.0, 1.5), Vec2::new(250.0, 1.5)]);
        let bands = tram_bands(&[track], &[&street, &deck], &paths, &[]);
        assert_eq!(bands.len(), 1, "one band across the seam: {bands:?}");
        let cover = band_cover(&bands);
        let whole: f32 = cover.iter().map(shape_area).sum();
        let cuts = deck_cuts(&bands, [(paths[1].as_slice(), 18.0)]);
        let [ground, decked] = split_cover(cover, &cuts);
        let area = |shapes: &[Shape]| shapes.iter().map(shape_area).sum::<f32>();
        assert!(
            (area(&ground) + area(&decked) - whole).abs() < 0.05,
            "{} + {} != {whole}",
            area(&ground),
            area(&decked)
        );
        let bounds = |shapes: &[Shape]| {
            shapes.iter().map(|shape| contour_bounds(&shape[0])).fold(
                (Vec2::INFINITY, Vec2::NEG_INFINITY),
                |(low, high), (a, b)| (low.min(a), high.max(b)),
            )
        };
        let (_, ground_high) = bounds(&ground);
        let (decked_low, _) = bounds(&decked);
        assert!((ground_high.x - head.x).abs() < 0.01, "{ground_high}");
        assert!((decked_low.x - head.x).abs() < 0.01, "{decked_low}");
    }

    /// Настил путепровода над трамваем, идущим поперёк, полосу не режет:
    /// кусок наземной полосы под ним лёг бы в слой настилов поверх него.
    #[test]
    fn an_overpass_across_the_band_does_not_cut_it() {
        let band = vec![vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)]];
        let across = [Vec2::new(50.0, -40.0), Vec2::new(50.0, 40.0)];
        assert!(deck_cuts(&band, [(across.as_slice(), 18.0)]).is_empty());
    }

    #[test]
    fn a_track_beyond_the_kerb_is_not_banded() {
        let road = fixture::street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 7.6);
        let paths = [road.points.clone()];
        let beside = tram(vec![Vec2::new(0.0, 5.0), Vec2::new(200.0, 5.0)]);
        assert!(tram_bands(&[beside], &[&road], &paths, &[]).is_empty());
    }
}
