//! Тропинки: вытоптанные дорожки от подъездов к ближайшей дороге.
//!
//! На снимке двора это первое, что выдаёт, что здесь живут: газон разрезан
//! светлыми полосами там, где ходят, а не там, где положили асфальт. Дорожка
//! от подъезда к проезду срезает угол, и срезает его каждый, поэтому линия
//! получается прямая и голая.
//!
//! Геометрия ровно такая, какой её описывает само явление: **отрезок от двери
//! до кромки ближайшей дороги**. Ни поиска пути, ни изгибов: тропа — это и есть
//! прямая, которую люди протоптали вместо крюка по асфальту.
//!
//! **До кромки, а не до оси.** `RoadLine` — это осевая линия и ширина отдельным
//! полем, а лента кроет ось ±`width`/2 (у `primary` это ±8 м, и сверху тротуар).
//! Тропа лежит под дорожным слоем, так что её кусок под лентой не виден, —
//! значит и мерить его нечестно: дверь в восьми метрах от оси магистрали давала
//! «тропу» в восемь метров, которой на снимке не было ни пикселя, и всё равно
//! попадала в счётчик.
//!
//! Пересечения с домами не проверяются, и это не недосмотр: слой лежит **под**
//! зданиями (`Z_WORN_PATH` 0.72 против 4.9 у фасада), так что кусок тропы,
//! попавший на соседний дом, закрыт этим домом. Ровно то же и с водой.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{closest_on_segment, grid_cell, put_in_cells};
use crate::map::osm::{PolyArea, RoadLine};

/// Ширина тропы, м. Уже дорожки в парке: её никто не мостил.
const PATH_WIDTH: f32 = 1.1;
/// Короче этого тропу не рисуем: прямая длиной в шесть своих ширин читается
/// пятном, а не линией, и то немногое, что от неё осталось бы, доедают полоса
/// фасада и кайма дороги.
///
/// Меряется **видимая** часть — от двери до кромки ленты, а не до оси (см.
/// шапку модуля), поэтому порог отсекает и дверь у самой красной линии, и
/// дверь в восьми метрах от оси шестнадцатиметровой магистрали: у обеих
/// тропа целиком под асфальтом.
///
/// **Полоса фасада закрывает только южные подходы** — это контур, сдвинутый
/// вниз (`buildings/layers.rs`, `offset = (0, -facade_height)`), так что у
/// двери на северной, восточной или западной грани она не кроет ничего.
/// «Короткая тропа целиком уходит под фасад» было бы верно для четверти
/// дверей, и порог держится не на этом.
const PATH_MIN: f32 = 7.0;
/// Длиннее — это уже не срезанный угол, а маршрут: такие тропы не
/// протаптываются одной дверью, и прямая линия там была бы выдумкой.
///
/// В отличие от [`PATH_MIN`], меряется **до оси**: окно поиска в
/// `⌈PATH_MAX / CELL⌉` ячеек гарантирует полноту именно на этом расстоянии,
/// и предел обязан совпадать с тем, что окно умеет найти.
const PATH_MAX: f32 = 45.0;
/// Ячейка сетки, по которой ищется ближайшая дорога, м. Чуть больше
/// [`PATH_MAX`] / 2: тогда достаточно посмотреть две ячейки в каждую сторону
/// (`reach = ⌈PATH_MAX / CELL⌉`), то есть пять по каждой оси.
const CELL: f32 = 24.0;

/// Итог перебора дверей. Отсев разведён по причинам: «дверь выходит прямо на
/// асфальт» и «до дороги отсюда не ходят» — разные наблюдения о городе, а в
/// одной цифре «столько-то из стольких-то» неразличимы.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WornPaths {
    /// Дверей, получивших тропу.
    pub count: usize,
    /// Видимая тропа короче [`PATH_MIN`]: дверь стоит на красной линии или её
    /// подход целиком укрыт лентой дороги.
    pub too_short: usize,
    /// До оси ближайшей дороги дальше [`PATH_MAX`].
    pub too_far: usize,
    /// Дороги в окне поиска нет вовсе.
    pub no_road: usize,
}

/// Тропы в меш; отдаёт разложенный по причинам итог. Порядок — по зданиям и
/// дверям, то есть детерминированный.
pub fn push_paths(
    builder: &mut MeshBuilder,
    buildings: &[PolyArea],
    roads: &[RoadLine],
    color: Color,
) -> WornPaths {
    let grid = RoadGrid::of(roads);
    let color = color.to_linear();
    let mut worn = WornPaths::default();
    for building in buildings {
        for &door in &building.entrances {
            let Some((axis, half_width)) = grid.nearest(door) else {
                worn.no_road += 1;
                continue;
            };
            let reach = axis - door;
            let to_axis = reach.length();
            // предел меряется до оси: ровно на это расстояние окно поиска и
            // даёт гарантию полноты
            if to_axis > PATH_MAX {
                worn.too_far += 1;
                continue;
            }
            let Some(along) = reach.try_normalize() else {
                // дверь ровно на оси — тропе взяться неоткуда
                worn.too_short += 1;
                continue;
            };
            // тропа кончается на кромке ленты, а не на оси: то, что под
            // асфальтом, всё равно закрыто дорожным слоем
            let at = axis - along * half_width;
            if to_axis - half_width < PATH_MIN {
                worn.too_short += 1;
                continue;
            }
            let across = Vec2::new(-along.y, along.x) * (PATH_WIDTH / 2.0);
            builder.push_quad(
                [door - across, at - across, at + across, door + across],
                color,
            );
            worn.count += 1;
        }
    }
    worn
}

/// Отрезки дорог, разложенные по ячейкам: дверей одиннадцать тысяч, дорог
/// четыре с половиной, и перебор пар был бы квадратичным на ровном месте.
///
/// Сетка своя, а не `osm::entrances::index::RoadIndex`, и это осознанно:
/// у генератора дверей ячейка 60 м и кольцевой обход с ранним выходом на
/// радиус в 240 м, здесь — 24 м и фиксированное окно на [`PATH_MAX`]. Общий у
/// них ровно тот инвариант, который и вынесен в
/// [`put_in_cells`](crate::map::osm::model::put_in_cells): отрезок лежит во
/// всех ячейках своего AABB.
struct RoadGrid {
    /// Отрезок осевой и **полуширина** его ленты: до кромки от оси ровно
    /// столько, и без этого числа тропу не укоротить.
    cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2, f32)>>,
}

impl RoadGrid {
    fn of(roads: &[RoadLine]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2, f32)>> =
            std::collections::HashMap::new();
        for road in roads {
            let half_width = road.width / 2.0;
            for pair in road.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                // отрезок кладётся во все ячейки, которые задевает его
                // прямоугольник: длинный перегон иначе потерялся бы в середине
                put_in_cells(&mut cells, a.min(b), a.max(b), CELL, (a, b, half_width));
            }
        }
        Self { cells }
    }

    /// Ближайшая точка **осевой** линии в окне поиска и полуширина её ленты,
    /// либо `None`, если дороги в окне нет вовсе.
    ///
    /// Предел [`PATH_MAX`] здесь не применяется: отсев по нему живёт в
    /// [`push_paths`], где отличима «дороги в окне нет» от «дорога есть, но
    /// дальше предела». Ближайшая выбирается по расстоянию **до оси** — лента
    /// у́же не делает дорогу ближе, а шире не делает дальше.
    fn nearest(&self, from: Vec2) -> Option<(Vec2, f32)> {
        let (cx, cy) = (grid_cell(from.x, CELL), grid_cell(from.y, CELL));
        let reach = (PATH_MAX / CELL).ceil() as i32;
        let mut best: Option<(f32, Vec2, f32)> = None;
        for x in -reach..=reach {
            for y in -reach..=reach {
                let key = (cx + x, cy + y);
                let Some(segments) = self.cells.get(&key) else {
                    continue;
                };
                for &(a, b, half_width) in segments {
                    let at = closest_on_segment(from, a, b);
                    let distance = from.distance_squared(at);
                    if best.is_none_or(|(best, _, _)| distance < best) {
                        best = Some((distance, at, half_width));
                    }
                }
            }
        }
        best.map(|(_, at, half_width)| (at, half_width))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::{AreaKind, BuildingUse, RoadClass};

    fn road(a: Vec2, b: Vec2) -> RoadLine {
        RoadLine {
            points: vec![a, b],
            width: 8.0,
            class: RoadClass::Street,
            bridge: false,
            passage: false,
            oneway: false,
            roundabout: false,
            lanes: None,
        }
    }

    fn house(door: Vec2) -> PolyArea {
        PolyArea {
            outer: vec![
                door,
                door + Vec2::new(12.0, 0.0),
                door + Vec2::new(12.0, 8.0),
                door + Vec2::new(0.0, 8.0),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::Apartments,
            height: None,
            entrances: vec![door],
        }
    }

    fn paths(door: Vec2, road: RoadLine) -> usize {
        let mut builder = MeshBuilder::default();
        let worn = push_paths(&mut builder, &[house(door)], &[road], Color::WHITE);
        assert_eq!(worn.count * 4, builder.vertex_count());
        builder.vertex_count()
    }

    /// Одна дверь, одна дорога — итог по причинам.
    fn tally(door: Vec2, road: RoadLine) -> WornPaths {
        let mut builder = MeshBuilder::default();
        push_paths(&mut builder, &[house(door)], &[road], Color::WHITE)
    }

    /// Дверь в двадцати метрах от проезда — тропа есть.
    #[test]
    fn a_door_across_the_yard_wears_a_path() {
        let door = Vec2::new(100.0, 100.0);
        assert_eq!(
            paths(door, road(Vec2::new(0.0, 120.0), Vec2::new(300.0, 120.0))),
            4
        );
    }

    /// Дверь на красной линии — тропы нет: она вся под фасадом.
    #[test]
    fn a_door_on_the_street_wears_nothing() {
        let door = Vec2::new(100.0, 100.0);
        assert_eq!(
            paths(door, road(Vec2::new(0.0, 102.0), Vec2::new(300.0, 102.0))),
            0
        );
    }

    /// И дверь на отшибе — тоже: полкилометра по прямой никто не топчет.
    #[test]
    fn a_door_far_from_any_road_wears_nothing() {
        let door = Vec2::new(100.0, 100.0);
        assert_eq!(
            paths(door, road(Vec2::new(0.0, 600.0), Vec2::new(300.0, 600.0))),
            0
        );
    }

    /// Тропа идёт к **ближайшей** точке оси, а не к концу отрезка, и вместе с
    /// точкой приходит полуширина ленты.
    #[test]
    fn the_path_aims_at_the_nearest_point_of_the_road() {
        let grid = RoadGrid::of(&[road(Vec2::new(0.0, 120.0), Vec2::new(300.0, 120.0))]);
        let (at, half_width) = grid
            .nearest(Vec2::new(100.0, 100.0))
            .expect("road in reach");
        assert!((at - Vec2::new(100.0, 120.0)).length() < 1e-3, "{at}");
        assert!((half_width - 4.0).abs() < 1e-3, "{half_width}");
    }

    /// Дверь в восьми метрах от оси магистрали: лента в 16 м кроет подход
    /// целиком, и тропы нет — хотя до **оси** тут больше [`PATH_MIN`].
    #[test]
    fn a_door_beside_a_trunk_road_wears_nothing() {
        let door = Vec2::new(100.0, 100.0);
        let mut primary = road(Vec2::new(0.0, 108.0), Vec2::new(300.0, 108.0));
        primary.width = 16.0;
        assert_eq!(paths(door, primary), 0);
    }

    /// Тропа кончается на кромке ленты: до оси 20 м, полуширина 4, значит
    /// дальний край квада стоит в 16 м от двери.
    #[test]
    fn the_path_stops_at_the_kerb() {
        let door = Vec2::new(100.0, 100.0);
        let mut builder = MeshBuilder::default();
        let worn = push_paths(
            &mut builder,
            &[house(door)],
            &[road(Vec2::new(0.0, 120.0), Vec2::new(300.0, 120.0))],
            Color::WHITE,
        );
        assert_eq!(worn.count, 1);
        let far = builder
            .positions_for_test()
            .iter()
            .map(|point| point[1])
            .fold(f32::MIN, f32::max);
        assert!((far - 116.0).abs() < 1e-3, "{far}");
    }

    /// Отсев разведён по причинам: у самой дороги, слишком далеко, дороги нет.
    #[test]
    fn the_tally_tells_the_three_rejections_apart() {
        let door = Vec2::new(100.0, 100.0);
        let at_the_kerb = tally(door, road(Vec2::new(0.0, 102.0), Vec2::new(300.0, 102.0)));
        assert_eq!(at_the_kerb.too_short, 1, "{at_the_kerb:?}");

        // дорога в окне поиска (±48 м), но дальше PATH_MAX
        let too_far = tally(door, road(Vec2::new(0.0, 147.0), Vec2::new(300.0, 147.0)));
        assert_eq!(too_far.too_far, 1, "{too_far:?}");

        let no_road = tally(door, road(Vec2::new(0.0, 600.0), Vec2::new(300.0, 600.0)));
        assert_eq!(no_road.no_road, 1, "{no_road:?}");
    }
}
