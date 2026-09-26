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
//! асфальта (своё полотно, парки, мосты) не рисуются, кусок короче
//! [`MIN_RUN`] — тоже.

use bevy::prelude::*;

use super::network::pairs::Median;
use crate::map::along::simplify;
use crate::map::grid::Grid;
use crate::map::osm::model::{RailKind, RailLine};
use crate::map::osm::{RoadClass, RoadLine};

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
        if road.class != RoadClass::Street || road.bridge || road.passage || path.is_empty() {
            continue;
        }
        let half = road.width / 2.0;
        let (low, high) = path.iter().fold((path[0], path[0]), |(low, high), &point| {
            (low.min(point), high.max(point))
        });
        if tracks.near_each(low - half, high + half).next().is_none() {
            continue;
        }
        for pair in path.windows(2) {
            links.insert_segment(pair[0], pair[1], half, (pair[0], pair[1], half));
        }
    }
    // мощёная разделительная — вместе с внутренними полосами половин по
    // бокам: полоса над путём у самой кромки заходит на них, а они асфальт
    for median in medians.iter().filter(|median| median.is_paved()) {
        let half = median.apart() / 2.0 + TRAM_BAND_WIDTH;
        for pair in median.midline.windows(2) {
            links.insert_segment(pair[0], pair[1], half, (pair[0], pair[1], half));
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
    let mut bands = Vec::new();
    for rail in rails.iter().filter(|rail| rail.kind == RailKind::Tram) {
        let mut run: Vec<Vec2> = Vec::new();
        let mut flush = |run: &mut Vec<Vec2>| {
            if run.len() >= 2 && length(run) >= MIN_RUN {
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
                    run.push(at);
                } else {
                    flush(&mut run);
                }
            }
        }
        flush(&mut run);
    }
    bands
}

fn length(points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum()
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

    #[test]
    fn a_track_beyond_the_kerb_is_not_banded() {
        let road = fixture::street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 7.6);
        let paths = [road.points.clone()];
        let beside = tram(vec![Vec2::new(0.0, 5.0), Vec2::new(200.0, 5.0)]);
        assert!(tram_bands(&[beside], &[&road], &paths, &[]).is_empty());
    }
}
