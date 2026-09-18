//! Пространственные индексы генератора: равномерные сетки дорог и контуров.
//! Париж — 27 000 зданий против 20 000 дорог, и линейный проход «для каждой
//! грани найти ближайшую улицу» по времени не проходит.

use bevy::math::{IVec2, Vec2};

use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, PolyArea, RoadLine, closest_on_segment, distance_to_segment, point_in_area,
    ring_bounds, signed_ring_area,
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
    segments: Grid<(Vec2, Vec2)>,
}

impl RoadIndex {
    pub(super) fn build(roads: &[RoadLine]) -> Self {
        let mut segments = Grid::new(ROAD_CELL);
        for road in roads {
            for segment in road.points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                // радиус здесь знает запрос (`nearest` обходит кольцами), а не
                // сам отрезок — рамка не раздувается
                segments.insert_segment(from, to, 0.0, (from, to));
            }
        }
        Self { segments }
    }

    /// Ближайшая точка дорожной сети и расстояние до неё. Кольца
    /// просматриваются от центра наружу; поиск останавливается, как только
    /// найденное расстояние заведомо меньше, чем всё, что может лежать в
    /// следующем кольце.
    pub(super) fn nearest(&self, point: Vec2) -> Option<(Vec2, f32)> {
        // единственное место на карте, которое обходит ячейки **кольцами**, —
        // поэтому оно и спрашивает у сетки координату ячейки, а не «что рядом»
        let centre = self.segments.cell_of(point);
        let mut best: Option<(Vec2, f32)> = None;

        for ring in 0..=ROAD_SEARCH_RINGS {
            for x in centre.x - ring..=centre.x + ring {
                for y in centre.y - ring..=centre.y + ring {
                    // только периметр кольца — внутренние ячейки уже пройдены
                    if (x - centre.x).abs() != ring && (y - centre.y).abs() != ring {
                        continue;
                    }
                    for &(from, to) in self.segments.cell(IVec2::new(x, y)) {
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

/// Та же сетка, но из **арок** — дорог с флагом `passage`, проложенных сквозь
/// дом. Проезд выедает кусок стены на всю её высоту
/// (`buildings::arches`), и подъезда в этом куске не бывает: снаружи там
/// дыра, а изнутри — проезжая часть.
///
/// Каждый отрезок кладётся в ячейки своего AABB, **раздутого на запрет**
/// ([`super::ENTRANCE_ARCH_CLEARANCE`] плюс полуширина дороги), поэтому
/// спрашивать хватает одну ячейку точки.
pub(super) struct PassageIndex {
    passages: Grid<(Vec2, Vec2, f32)>,
}

impl PassageIndex {
    pub(super) fn build(roads: &[RoadLine]) -> Self {
        let mut passages = Grid::new(ROAD_CELL);
        for road in roads.iter().filter(|road| road.passage) {
            let reach = road.width / 2.0 + super::ENTRANCE_ARCH_CLEARANCE;
            for segment in road.points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                passages.insert_segment(from, to, reach, (from, to, reach));
            }
        }
        Self { passages }
    }

    /// Стоит ли эта точка в арке или вплотную к ней.
    pub(super) fn blocks(&self, point: Vec2) -> bool {
        self.passages
            .at(point)
            .iter()
            .any(|&(from, to, reach)| distance_to_segment(point, from, to) < reach)
    }
}

/// Равномерная сетка контуров зданий. Нужна, чтобы ответить на вопрос «есть ли
/// перед этой стеной свободное место»: в плотной застройке дома в OSM стоят
/// вплотную и даже перекрываются, и дверь, поставленная на общую стену,
/// оказывается внутри соседа — снаружи её не видно, а изнутри к ней не пройти.
pub(super) struct FootprintIndex<'a> {
    /// Номера домов из [`Self::buildings`] по ячейкам их пятен — имя `buildings`
    /// занято самим срезом, в который эти номера индексируют.
    buildings_by_cell: Grid<usize>,
    buildings: &'a [PolyArea],
}

impl<'a> FootprintIndex<'a> {
    pub(super) fn build(buildings: &'a [PolyArea]) -> Self {
        let mut buildings_by_cell = Grid::new(FOOTPRINT_CELL);
        for (index, building) in buildings.iter().enumerate() {
            // загораживает дверь только дом; вода и парк — не преграда
            if building.kind != AreaKind::Building || building.outer.len() < 3 {
                continue;
            }
            let (min, max) = ring_bounds(&building.outer);
            buildings_by_cell.insert(min, max, index);
        }
        Self {
            buildings_by_cell,
            buildings,
        }
    }

    /// Точка занята чужим домом? Свой дом (`owner`) не в счёт — дверь стоит на
    /// его собственной стене.
    pub(super) fn is_covered(&self, point: Vec2, owner: usize) -> bool {
        self.buildings_by_cell
            .at(point)
            .iter()
            .any(|&index| index != owner && point_in_area(point, &self.buildings[index]))
    }
}

/// Обход контура против часовой стрелки? От этого зависит, в какую сторону
/// смотрит внешняя нормаль грани.
pub(super) fn ring_is_ccw(ring: &[Vec2]) -> bool {
    signed_ring_area(ring) > 0.0
}
