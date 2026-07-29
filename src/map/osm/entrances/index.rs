//! Пространственные индексы генератора: равномерные сетки дорог и контуров.
//! Париж — 27 000 зданий против 20 000 дорог, и линейный проход «для каждой
//! грани найти ближайшую улицу» по времени не проходит.

use bevy::math::Vec2;

use crate::map::osm::model::{
    AreaKind, PolyArea, RoadLine, distance_to_segment, point_in_area, ring_bounds,
};

/// Сторона ячейки индекса дорог, м.
const ROAD_CELL: f32 = 60.0;
/// Сторона ячейки индекса контуров, м. Мельче дорожной: здание — пятно в
/// десятки метров, и в ячейку должно попадать несколько кандидатов, а не
/// полквартала.
const FOOTPRINT_CELL: f32 = 30.0;
/// Насколько колец ячеек вокруг точки просматривает индекс, прежде чем
/// сдаться. 4 кольца — 240 м; дальше от любой дороги в городе не бывает, а
/// здание в глубине квартала всё равно получит дверь по лучшей из граней.
const ROAD_SEARCH_RINGS: i32 = 4;

/// Равномерная сетка отрезков дорог. Генератору нужен ближайший участок улицы
/// для каждой грани каждого контура — в Париже это 27 000 зданий против
/// 20 000 дорог, и линейный проход тут не проходит по времени.
pub(super) struct RoadIndex {
    cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
}

impl RoadIndex {
    pub(super) fn build(roads: &[RoadLine]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>> =
            std::collections::HashMap::new();
        for road in roads {
            for segment in road.points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                let min = from.min(to);
                let max = from.max(to);
                // отрезок кладётся во все ячейки, которые пересекает его AABB
                for x in cell(min.x, ROAD_CELL)..=cell(max.x, ROAD_CELL) {
                    for y in cell(min.y, ROAD_CELL)..=cell(max.y, ROAD_CELL) {
                        cells.entry((x, y)).or_default().push((from, to));
                    }
                }
            }
        }
        Self { cells }
    }

    /// Ближайшая точка дорожной сети и расстояние до неё. Кольца
    /// просматриваются от центра наружу; поиск останавливается, как только
    /// найденное расстояние заведомо меньше, чем всё, что может лежать в
    /// следующем кольце.
    pub(super) fn nearest(&self, point: Vec2) -> Option<(Vec2, f32)> {
        let (cx, cy) = (cell(point.x, ROAD_CELL), cell(point.y, ROAD_CELL));
        let mut best: Option<(Vec2, f32)> = None;

        for ring in 0..=ROAD_SEARCH_RINGS {
            for x in cx - ring..=cx + ring {
                for y in cy - ring..=cy + ring {
                    // только периметр кольца — внутренние ячейки уже пройдены
                    if (x - cx).abs() != ring && (y - cy).abs() != ring {
                        continue;
                    }
                    for &(from, to) in self.cells.get(&(x, y)).into_iter().flatten() {
                        let distance = distance_to_segment(point, from, to);
                        if best.is_none_or(|(_, best_distance)| distance < best_distance) {
                            best = Some((closest_on_segment(point, from, to), distance));
                        }
                    }
                }
            }
            // всё, что осталось снаружи, дальше этого рубежа
            if best.is_some_and(|(_, distance)| distance <= ring as f32 * ROAD_CELL) {
                break;
            }
        }
        best
    }
}

fn cell(value: f32, size: f32) -> i32 {
    (value / size).floor() as i32
}

/// Равномерная сетка контуров зданий. Нужна, чтобы ответить на вопрос «есть ли
/// перед этой стеной свободное место»: в плотной застройке дома в OSM стоят
/// вплотную и даже перекрываются, и дверь, поставленная на общую стену,
/// оказывается внутри соседа — снаружи её не видно, а изнутри к ней не пройти.
pub(super) struct FootprintIndex<'a> {
    cells: std::collections::HashMap<(i32, i32), Vec<usize>>,
    buildings: &'a [PolyArea],
}

impl<'a> FootprintIndex<'a> {
    pub(super) fn build(buildings: &'a [PolyArea]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> =
            std::collections::HashMap::new();
        for (index, building) in buildings.iter().enumerate() {
            // загораживает дверь только дом; вода и парк — не преграда
            if building.kind != AreaKind::Building || building.outer.len() < 3 {
                continue;
            }
            let (min, max) = ring_bounds(&building.outer);
            for x in cell(min.x, FOOTPRINT_CELL)..=cell(max.x, FOOTPRINT_CELL) {
                for y in cell(min.y, FOOTPRINT_CELL)..=cell(max.y, FOOTPRINT_CELL) {
                    cells.entry((x, y)).or_default().push(index);
                }
            }
        }
        Self { cells, buildings }
    }

    /// Точка занята чужим домом? Свой дом (`owner`) не в счёт — дверь стоит на
    /// его собственной стене.
    pub(super) fn is_covered(&self, point: Vec2, owner: usize) -> bool {
        let key = (cell(point.x, FOOTPRINT_CELL), cell(point.y, FOOTPRINT_CELL));
        self.cells
            .get(&key)
            .into_iter()
            .flatten()
            .any(|&index| index != owner && point_in_area(point, &self.buildings[index]))
    }
}

/// Ближайшая точка отрезка — как `distance_to_segment`, но нужна сама точка.
fn closest_on_segment(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return from;
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    from + segment * t
}

/// Обход контура против часовой стрелки? От этого зависит, в какую сторону
/// смотрит внешняя нормаль грани. Формула та же, что в `ring_area`, но со
/// знаком.
pub(super) fn ring_is_ccw(ring: &[Vec2]) -> bool {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += (ring[j].x - ring[i].x) * (ring[j].y + ring[i].y);
        j = i;
    }
    doubled > 0.0
}
