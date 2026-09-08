//! Тропинки: вытоптанные дорожки от подъездов к ближайшей дороге.
//!
//! На снимке двора это первое, что выдаёт, что здесь живут: газон разрезан
//! светлыми полосами там, где ходят, а не там, где положили асфальт. Дорожка
//! от подъезда к проезду срезает угол, и срезает его каждый, поэтому линия
//! получается прямая и голая.
//!
//! Геометрия ровно такая, какой её описывает само явление: **отрезок от двери
//! до ближайшей точки дороги**. Ни поиска пути, ни изгибов: тропа — это и есть
//! прямая, которую люди протоптали вместо крюка по асфальту.
//!
//! Пересечения с домами не проверяются, и это не недосмотр: слой лежит **под**
//! зданиями (`Z_WORN_PATH` 0.72 против 4.9 у фасада), так что кусок тропы,
//! попавший на соседний дом, закрыт этим домом. Ровно то же и с водой.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{PolyArea, RoadLine};

/// Ширина тропы, м. Уже дорожки в парке: её никто не мостил.
const PATH_WIDTH: f32 = 1.1;
/// Короче этого тропу не рисуем: она целиком уходит под полосу фасада.
const PATH_MIN: f32 = 7.0;
/// Длиннее — это уже не срезанный угол, а маршрут: такие тропы не
/// протаптываются одной дверью, и прямая линия там была бы выдумкой.
const PATH_MAX: f32 = 45.0;
/// Ячейка сетки, по которой ищется ближайшая дорога, м. Чуть больше
/// [`PATH_MAX`] / 2: тогда достаточно посмотреть три ячейки в каждую сторону.
const CELL: f32 = 24.0;

/// Тропы в меш; отдаёт, сколько дверей их получило. Порядок — по зданиям и
/// дверям, то есть детерминированный.
pub fn push_paths(
    builder: &mut MeshBuilder,
    buildings: &[PolyArea],
    roads: &[RoadLine],
    color: Color,
) -> usize {
    let grid = RoadGrid::of(roads);
    let color = color.to_linear();
    let mut worn = 0;
    for building in buildings {
        for &door in &building.entrances {
            let Some(at) = grid.nearest(door) else {
                continue;
            };
            let reach = at - door;
            let length = reach.length();
            if !(PATH_MIN..=PATH_MAX).contains(&length) {
                continue;
            }
            let Some(along) = reach.try_normalize() else {
                continue;
            };
            let half = Vec2::new(-along.y, along.x) * (PATH_WIDTH / 2.0);
            builder.push_quad([door - half, at - half, at + half, door + half], color);
            worn += 1;
        }
    }
    worn
}

/// Отрезки дорог, разложенные по ячейкам: дверей одиннадцать тысяч, дорог
/// четыре с половиной, и перебор пар был бы квадратичным на ровном месте.
struct RoadGrid {
    cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
}

impl RoadGrid {
    fn of(roads: &[RoadLine]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>> =
            std::collections::HashMap::new();
        for road in roads {
            for pair in road.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                // отрезок кладётся во все ячейки, которые задевает его
                // прямоугольник: длинный перегон иначе потерялся бы в середине
                let low = (a.min(b) / CELL).floor();
                let high = (a.max(b) / CELL).floor();
                for x in low.x as i32..=high.x as i32 {
                    for y in low.y as i32..=high.y as i32 {
                        cells.entry((x, y)).or_default().push((a, b));
                    }
                }
            }
        }
        Self { cells }
    }

    /// Ближайшая точка дороги не дальше [`PATH_MAX`], либо `None`.
    fn nearest(&self, from: Vec2) -> Option<Vec2> {
        let cell = (from / CELL).floor();
        let reach = (PATH_MAX / CELL).ceil() as i32;
        let mut best: Option<(f32, Vec2)> = None;
        for x in -reach..=reach {
            for y in -reach..=reach {
                let key = (cell.x as i32 + x, cell.y as i32 + y);
                let Some(segments) = self.cells.get(&key) else {
                    continue;
                };
                for &(a, b) in segments {
                    let at = closest_on_segment(from, a, b);
                    let distance = from.distance_squared(at);
                    if best.is_none_or(|(best, _)| distance < best) {
                        best = Some((distance, at));
                    }
                }
            }
        }
        best.filter(|(distance, _)| *distance <= PATH_MAX * PATH_MAX)
            .map(|(_, at)| at)
    }
}

fn closest_on_segment(from: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let span = b - a;
    let length = span.length_squared();
    if length <= f32::EPSILON {
        return a;
    }
    a + span * ((from - a).dot(span) / length).clamp(0.0, 1.0)
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
        assert_eq!(worn * 4, builder.vertex_count());
        builder.vertex_count()
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

    /// Тропа идёт к **ближайшей** точке, а не к концу отрезка.
    #[test]
    fn the_path_aims_at_the_nearest_point_of_the_road() {
        let grid = RoadGrid::of(&[road(Vec2::new(0.0, 120.0), Vec2::new(300.0, 120.0))]);
        let at = grid
            .nearest(Vec2::new(100.0, 100.0))
            .expect("road in reach");
        assert!((at - Vec2::new(100.0, 120.0)).length() < 1e-3, "{at}");
    }
}
