//! **Пути трамвая по нарисованной улице** — где лежат трамвайные пути,
//! которые идут по проезжей части, на картинке.
//!
//! Пути OSM лежат там, где их провёл картограф, а улица рисуется не по OSM:
//! ось сглажена (`roads/axis.rs`), половины разделённого проспекта разведены
//! от середины на свой зазор (`network/pairs.rs`). Путь, взятый как есть,
//! уходил от своей улицы: на Советской в Туле пути лежали на северной
//! половине, заходили на её полосы, а на каждом узле OSM смещение менялось.
//! Здесь путь кладётся по тому, что нарисовано, и этот ответ читают все, кто
//! рисует трамвай: светлая полоса над рельсами (`roads/tram_band.rs`) и слой
//! самого трамвая (`map/tram.rs`, через ресурс [`TramTracks`]).
//!
//! - **На трамвайном полотне** пары половин (`Median::carries_tram`) путь
//!   встаёт на середину разделительной — ту же, по которой идёт двойная
//!   сплошная, — со сдвигом в полшага [`TRACK_SPACING`] в свою сторону.
//!   Сторону решает второй путь рядом: кто из двух южнее в OSM, тот южнее и
//!   на картинке. Путь без пары на полотне ложится на саму середину.
//! - **По одиночной улице** путь сдвигается ровно на столько, на сколько
//!   нарисованная ось ушла от оси OSM в этом месте: отступ пути от оси, каким
//!   его провёл картограф, остаётся.
//! - **Вне асфальта** (своё полотно, парк) путь остаётся как в OSM, а сдвиг
//!   сходит на нет за [`TRANSITION`] от последней пробы на асфальте; дыра
//!   между двумя кусками не длиннее [`BRIDGE_MAX`] (перекрёсток, где
//!   разделительная прервана) проходится сдвигом, плавно перетекающим от
//!   одного края к другому.
//!
//! Точки `RailLine` из карты не меняются: сдвиг — только отрисовка, как у
//! осей улиц.

use bevy::prelude::*;

use super::network::pairs::Median;
use super::smoothstep;
use crate::map::along::{arclengths, densify, nearest_on_path, simplify};
use crate::map::grid::Grid;
use crate::map::osm::model::{RailKind, RailLine, closest_on_segment};
use crate::map::osm::{RoadClass, RoadLine};

/// Расстояние между осями двух путей на полотне, м: вагон 2.6 м и зазор
/// между встречными вагонами.
pub const TRACK_SPACING: f32 = 3.2;
/// Шаг, с которым путь ощупывается, м.
const PROBE_STEP: f32 = 2.0;
/// Косинус угла между путём и улицей, при котором путь идёт вдоль неё.
const ALONG_MIN: f32 = 0.8;
/// Насколько путь может выйти за край полотна (половину его ширины), м, и
/// ещё лежать на нём: путь OSM — не точно между кромками. Целый шаг путей:
/// на Советской полотно между разведёнными половинами — 3.8 м, северный путь
/// OSM лежит на его середине, а южный — в 3.1 м от неё, на полосах южной
/// половины; с запасом в метр он полотна не находил и оставался там.
const BED_SLACK: f32 = TRACK_SPACING;
/// Насколько путь может выйти за кромку ленты улицы, м, и ещё идти по ней.
const EDGE_SLACK: f32 = 0.6;
/// Насколько основание пробы может выйти за торец звена, м.
const LINK_SLACK: f32 = 1.0;
/// Два пути ближе этого поперёк, м, — один и тот же путь, а не пара.
const PARTNER_MIN: f32 = 0.5;
/// За сколько метров от последней пробы на асфальте сдвиг сходит на нет.
const TRANSITION: f32 = 20.0;
/// Дыра между двумя кусками на асфальте не длиннее этого, м, проходится
/// сдвигом насквозь: пути пересекают узел, где разделительная прервана.
const BRIDGE_MAX: f32 = 40.0;
/// Допуск, с которым пробы прореживаются обратно в вершины, м.
const SIMPLIFY_TOLERANCE: f32 = 0.05;
/// Ячейка сеток звеньев, м.
const CELL: f32 = 32.0;

/// Трамвайные пути карты так, как они рисуются: по одному на каждый путь
/// `RailKind::Tram` карты, в её порядке, с точками по нарисованной улице
/// ([`lay_tracks`]). Пишет его сборка дорог (`roads::rebuild_roads`,
/// `spawn::spawn_map`), читает слой трамвая (`tram::rebuild_tram`).
#[derive(Resource, Clone, Debug, Default)]
pub struct TramTracks(pub Vec<RailLine>);

impl TramTracks {
    /// Пути как в OSM, без укладки: сырой OSM и замеры слоя трамвая.
    pub fn as_mapped(rails: &[RailLine]) -> Self {
        Self(
            rails
                .iter()
                .filter(|rail| rail.kind == RailKind::Tram)
                .cloned()
                .collect(),
        )
    }
}

/// Звено полотна: номер разделительной и звена середины.
type BedLink = (usize, usize);

/// Трамвайные пути `rails`, уложенные по нарисованным улицам: `roads` — как
/// рисуются, `paths` — их узловые оси по тому же индексу, `medians` —
/// разделительные пар (читаются только трамвайные полотна).
pub fn lay_tracks(
    rails: &[RailLine],
    roads: &[&RoadLine],
    paths: &[impl AsRef<[Vec2]>],
    medians: &[Median],
) -> TramTracks {
    let trams: Vec<&RailLine> = rails
        .iter()
        .filter(|rail| rail.kind == RailKind::Tram)
        .collect();
    if trams.is_empty() {
        return TramTracks::default();
    }
    let mut tracks: Grid<(usize, usize)> = Grid::new(CELL);
    for (index, rail) in trams.iter().enumerate() {
        for (link, pair) in rail.points.windows(2).enumerate() {
            tracks.insert_segment(pair[0], pair[1], 0.0, (index, link));
        }
    }
    let beds: Vec<&Median> = medians
        .iter()
        .filter(|median| median.carries_tram())
        .collect();
    let mut bed_links: Grid<BedLink> = Grid::new(CELL);
    for (index, bed) in beds.iter().enumerate() {
        let reach = bed.width() / 2.0 + BED_SLACK;
        for (link, pair) in bed.midline().windows(2).enumerate() {
            bed_links.insert_segment(pair[0], pair[1], reach, (index, link));
        }
    }
    // звенья нарисованных осей улиц, по которым идёт трамвай
    let mut street_links: Grid<(usize, usize)> = Grid::new(CELL);
    for (index, (road, path)) in roads.iter().zip(paths).enumerate() {
        let path = path.as_ref();
        if road.class != RoadClass::Street || road.passage || path.len() < 2 {
            continue;
        }
        let half = road.width / 2.0;
        let (low, high) = path.iter().fold((path[0], path[0]), |(low, high), &point| {
            (low.min(point), high.max(point))
        });
        if tracks.near_each(low - half, high + half).next().is_none() {
            continue;
        }
        for (link, pair) in path.windows(2).enumerate() {
            street_links.insert_segment(pair[0], pair[1], half + EDGE_SLACK, (index, link));
        }
    }
    let layer = Layer {
        trams: &trams,
        tracks: &tracks,
        beds: &beds,
        bed_links: &bed_links,
        roads,
        paths,
        street_links: &street_links,
    };
    TramTracks(
        trams
            .iter()
            .enumerate()
            .map(|(index, rail)| RailLine {
                points: layer.lay(index),
                ..(*rail).clone()
            })
            .collect(),
    )
}

/// Всё, по чему кладётся один путь.
struct Layer<'a, P> {
    trams: &'a [&'a RailLine],
    tracks: &'a Grid<(usize, usize)>,
    beds: &'a [&'a Median],
    bed_links: &'a Grid<BedLink>,
    roads: &'a [&'a RoadLine],
    paths: &'a [P],
    street_links: &'a Grid<(usize, usize)>,
}

impl<P: AsRef<[Vec2]>> Layer<'_, P> {
    /// Точки пути `index`, уложенные по улице.
    fn lay(&self, index: usize) -> Vec<Vec2> {
        let rail = self.trams[index];
        let dense = densify(&rail.points, PROBE_STEP);
        if dense.len() < 2 {
            return rail.points.clone();
        }
        let shifts: Vec<Option<Vec2>> = (0..dense.len())
            .map(|probe| {
                let heading = heading_at(&dense, probe)?;
                let at = dense[probe];
                self.on_bed(index, at, heading)
                    .or_else(|| self.on_street(at, heading))
            })
            .collect();
        if shifts.iter().all(Option::is_none) {
            return rail.points.clone();
        }
        let (along, _) = arclengths(&dense);
        let shifts = fill_gaps(&shifts, &along);
        let laid: Vec<Vec2> = dense
            .iter()
            .zip(&shifts)
            .map(|(point, shift)| *point + *shift)
            .collect();
        simplify(&laid, false, SIMPLIFY_TOLERANCE, |_| false)
            .into_iter()
            .map(|kept| laid[kept])
            .collect()
    }

    /// Сдвиг пробы на полотне: до середины разделительной и полшага в
    /// сторону пути от второго.
    fn on_bed(&self, index: usize, at: Vec2, heading: Vec2) -> Option<Vec2> {
        let (bed, foot, direction) = self
            .bed_links
            .at(at)
            .iter()
            .filter_map(|&(bed, link)| {
                let line = self.beds[bed].midline();
                let reach = self.beds[bed].width() / 2.0 + BED_SLACK;
                let (foot, direction) = foot_on(line[link], line[link + 1], at, heading)?;
                let distance = foot.distance(at);
                (distance <= reach).then_some((distance, bed, foot, direction))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, bed, foot, direction)| (bed, foot, direction))?;
        let normal = direction.perp();
        let offset = (at - foot).dot(normal);
        let reach = self.beds[bed].width() / 2.0 + BED_SLACK;
        // второй путь на том же полотне — ближайший к пробе
        let partner = self
            .tracks
            .near_each(at - 2.0 * reach, at + 2.0 * reach)
            .filter(|&&(other, _)| other != index)
            .filter_map(|&(other, link)| {
                let points = &self.trams[other].points;
                let (from, to) = (points[link], points[link + 1]);
                let along = (to - from).try_normalize()?;
                if along.dot(direction).abs() < ALONG_MIN {
                    return None;
                }
                let near = closest_on_segment(at, from, to);
                let across = (near - foot).dot(normal);
                (across.abs() <= reach && (offset - across).abs() > PARTNER_MIN)
                    .then_some((near.distance(at), across))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let target = match partner {
            Some((_, across)) => foot + normal * (offset - across).signum() * TRACK_SPACING / 2.0,
            None => foot,
        };
        Some(target - at)
    }

    /// Сдвиг пробы на одиночной улице: на столько, на сколько её нарисованная
    /// ось ушла здесь от оси OSM.
    fn on_street(&self, at: Vec2, heading: Vec2) -> Option<Vec2> {
        let (road, foot) = self
            .street_links
            .at(at)
            .iter()
            .filter_map(|&(road, link)| {
                let path = self.paths[road].as_ref();
                let (foot, _) = foot_on(path[link], path[link + 1], at, heading)?;
                let distance = foot.distance(at);
                (distance <= self.roads[road].width / 2.0 + EDGE_SLACK)
                    .then_some((distance, road, foot))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, road, foot)| (road, foot))?;
        let (mapped, _) = nearest_on_path(&self.roads[road].points, at)?;
        Some(foot - mapped)
    }
}

/// Основание пробы `at` на звене `from..to`, идущем вдоль `heading`, и
/// направление звена. `None` — звено поперёк или основание за его торцом.
fn foot_on(from: Vec2, to: Vec2, at: Vec2, heading: Vec2) -> Option<(Vec2, Vec2)> {
    let direction = (to - from).try_normalize()?;
    if direction.dot(heading).abs() < ALONG_MIN {
        return None;
    }
    let t = (at - from).dot(direction);
    if !(-LINK_SLACK..=from.distance(to) + LINK_SLACK).contains(&t) {
        return None;
    }
    Some((from + direction * t, direction))
}

/// Направление пути у пробы `index` — по звену за ней, у последней — перед.
fn heading_at(points: &[Vec2], index: usize) -> Option<Vec2> {
    let (from, to) = if index + 1 < points.len() {
        (points[index], points[index + 1])
    } else {
        (points[index - 1], points[index])
    };
    (to - from).try_normalize()
}

/// Сдвиги всех проб: у пробы вне асфальта — сошедший на нет сдвиг соседних
/// кусков ([`TRANSITION`]), в дыре не длиннее [`BRIDGE_MAX`] — перетекающий
/// от края к краю.
fn fill_gaps(shifts: &[Option<Vec2>], along: &[f32]) -> Vec<Vec2> {
    let count = shifts.len();
    let mut filled = vec![Vec2::ZERO; count];
    let mut previous: Option<usize> = None;
    let mut index = 0;
    while index < count {
        if let Some(shift) = shifts[index] {
            filled[index] = shift;
            previous = Some(index);
            index += 1;
            continue;
        }
        let next = (index..count).find(|&probe| shifts[probe].is_some());
        let end = next.unwrap_or(count);
        let bridged =
            matches!((previous, next), (Some(a), Some(b)) if along[b] - along[a] <= BRIDGE_MAX);
        for probe in index..end {
            filled[probe] = match (previous, next) {
                (Some(a), Some(b)) if bridged => {
                    let (from, to) = (shifts[a].unwrap_or_default(), shifts[b].unwrap_or_default());
                    let share = (along[probe] - along[a]) / (along[b] - along[a]);
                    from.lerp(to, smoothstep(share))
                }
                _ => {
                    let fade = |edge: Option<usize>| {
                        edge.and_then(|edge| {
                            let distance = (along[probe] - along[edge]).abs();
                            shifts[edge]
                                .map(|shift| shift * (1.0 - smoothstep(distance / TRANSITION)))
                        })
                        .unwrap_or_default()
                    };
                    fade(previous) + fade(next)
                }
            };
        }
        index = end;
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Дыра между двумя сдвигами проходится насквозь, а за краем сдвиг сходит
    /// на нет за [`TRANSITION`].
    #[test]
    fn a_short_gap_is_bridged_and_a_loose_end_fades() {
        let shift = Vec2::new(0.0, 2.0);
        let mut shifts = vec![Some(shift); 3];
        shifts.extend([None; 4]);
        shifts.extend([Some(shift); 3]);
        shifts.extend([None; 30]);
        let along: Vec<f32> = (0..shifts.len()).map(|at| at as f32 * 2.0).collect();
        let filled = fill_gaps(&shifts, &along);
        assert!(
            filled[..10].iter().all(|at| at.abs_diff_eq(shift, 1e-4)),
            "{filled:?}"
        );
        assert!(filled[12].y < 2.0 && filled[12].y > 0.0);
        assert_eq!(filled[filled.len() - 1], Vec2::ZERO);
    }
}
