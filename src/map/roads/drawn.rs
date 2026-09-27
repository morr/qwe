//! **Drawn** — дороги карты так, как они рисуются: переезды через тротуар
//! асфальтом проезда и дуги колец сечением всего кольца, общие узлы, оси улиц.
//! Строится один раз на [`mesh_roads`](super::mesh_roads) и отдаёт
//! подмодулям то, что раньше жило локалами в его начале.
//!
//! Все векторы — по индексу в `map.roads`, длины равны. Ни одна правка
//! `RoadLine::points` не трогает: навмеш, двери и деревья видят OSM как есть.

use std::borrow::Cow;

use bevy::prelude::*;

use super::axis::{self, Axes};
use super::merges::{self, Merges};
use super::network::{self, RoadNodes, Stitches};
use super::shape::RoadShape;
use super::tapers::Tapers;
use super::{MEDIAN_CROSSING_MAX, RoadStyle, drawn_sidewalk, ring_arcs};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{MapData, RoadClass, RoadLine};

/// Подготовленные дороги. Поля пока открыты подмодулям `roads` — сигнатуры
/// подмодулей не меняются, они берут `&drawn.roads()`, `&drawn.axes.paths`.
pub struct Drawn<'m> {
    /// Дороги карты как есть — по их точкам ищут узел по ключу
    /// (`junctions::node_key`), и от первой точки сеется карман.
    pub osm: &'m [RoadLine],
    /// Общие узлы дорог любого класса.
    pub nodes: RoadNodes,
    /// Оси улиц (`roads/axis.rs`), пары половин и кольца.
    pub axes: Axes<'m>,
    /// Подмены: переезд через тротуар — улицей шириной у́жего проезда
    /// (`network::driveway_crossings`), дуга кольца — сечением всего кольца
    /// ([`ring_arcs`]). Порядок — порядок подмены: поздняя побеждает.
    pub crossings: Vec<(usize, RoadLine)>,
    /// Стежки висячих торцов (`network::stitches`) — по дорогам как рисуются.
    pub stitches: Stitches,
    /// Ось со стежками — только у дорог, которых стежок коснулся; прочие
    /// рисуются по своей оси ([`Self::stitched`]).
    stitched: Vec<Option<Vec<Vec2>>>,
    /// Клинья между сечениями улиц (`roads/tapers.rs`) — **единственный**
    /// расчёт на `mesh_roads`: карманы и ряд машин берут его же.
    pub tapers: Tapers,
    /// Слияния разделённой улицы в обычную (`roads/merges.rs`).
    pub merges: Merges,
    /// Кусок поперечной улицы в проёме разделительной — между половинами
    /// одной пары: тротуара он не несёт, его полоса светлым пятном лежала
    /// посреди перекрёстка.
    pub across_median: Vec<bool>,
}

/// Что подготовка нашла — вложенное поле `RoadReport::drawn`; строку лога
/// печатает `RoadReport`, как и прежде.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct DrawnStats {
    /// Подмены: переезды через тротуар и дуги колец.
    pub crossings: usize,
    pub stitches: usize,
    /// Швы ways, пройденные осью улицы одной кривой (`roads/axis.rs`).
    pub seams: usize,
    /// Изломы, на которые звеньев не хватило для радиуса в полуширину.
    pub tight: usize,
    /// Клинья между сечениями улиц (`roads/tapers.rs`).
    pub tapers: usize,
    /// Слияния разделённой улицы в обычную (`roads/merges.rs`).
    pub merges: usize,
    /// Разделительные парных половин (`roads/network/pairs.rs`): асфальтом,
    /// газоном и из асфальтовых — трамвайных полотен.
    pub medians: [usize; 3],
    /// Кольца, нарисованные гладкой фигурой (`roads/rings.rs`), и щели
    /// между подходом и кольцом, залитые асфальтом.
    pub rings: [usize; 2],
}

impl<'m> Drawn<'m> {
    pub fn new(map: &'m MapData, style: &RoadStyle, shape: &RoadShape) -> Self {
        let roads = map.roads.as_slice();
        let nodes = RoadNodes::new(roads);
        // ось по улице целиком, не по way (`roads/axis.rs`); у переезда та же
        // ось, что у его дороги, — он отличается шириной и классом
        let axes = axis::street_axes(roads, &map.rails, &map.network, &nodes, shape);
        let crossings: Vec<(usize, RoadLine)> = network::driveway_crossings(roads, &nodes)
            .into_iter()
            .map(|(index, width)| {
                let crossing = RoadLine {
                    class: RoadClass::Street,
                    width,
                    ..roads[index].clone()
                };
                (index, crossing)
            })
            .chain(ring_arcs(roads, &axes.rings))
            .collect();
        let drawn = substituted(roads, &crossings);
        let paths = &axes.paths;
        let stitches = network::stitches(&drawn, map, &nodes, |road| drawn_sidewalk(style, road));
        let tapers = Tapers::new(&drawn, &map.network, &nodes, shape.taper());
        // Торцы узлов — точки OSM, и ось их не двигает.
        let across_median: Vec<bool> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
                    return false;
                };
                polyline_length(path) < MEDIAN_CROSSING_MAX
                    && nodes.roads_at(start).iter().any(|&half| {
                        half != index
                            && axes.pairs.runs[half].iter().any(|run| {
                                run.partner != index && nodes.roads_at(end).contains(&run.partner)
                            })
                    })
            })
            .collect();
        // разделённая улица, сходящаяся в обычную: узел не перекрёсток
        let merges = merges::merges(&drawn, paths, &nodes, &axes.pairs.runs, &map.network);
        // стежок до дороги, до которой OSM торец не довёл
        // (`roads/network/mod.rs`)
        let stitched = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                stitches.touches(index).then(|| {
                    let mut points = path.to_vec();
                    stitches.apply(index, &mut points);
                    points
                })
            })
            .collect();
        Self {
            osm: roads,
            nodes,
            axes,
            crossings,
            stitches,
            stitched,
            tapers,
            merges,
            across_median,
        }
    }

    /// Дороги как рисуются, по индексу `map.roads`: подмены на своих местах.
    pub fn roads(&self) -> Vec<&RoadLine> {
        substituted(self.osm, &self.crossings)
    }

    /// Счётчики подготовки для строки `road meshing:`.
    pub fn stats(&self) -> DrawnStats {
        DrawnStats {
            crossings: self.crossings.len(),
            stitches: self.stitches.count,
            seams: self.axes.seams,
            tight: self.axes.tight,
            tapers: self.tapers.count,
            merges: self.merges.list.len(),
            medians: self.axes.pairs.count(),
            rings: [self.axes.rings.list.len(), self.axes.rings.webs.len()],
        }
    }

    /// Оси со стежками — лента, краска, траектории: у тронутой стежком дороги
    /// своя копия, у прочих — их ось из [`Axes::paths`].
    pub fn stitched(&self) -> Vec<Cow<'_, [Vec2]>> {
        self.stitched
            .iter()
            .zip(&self.axes.paths)
            .map(|(stitched, path)| Cow::Borrowed(stitched.as_deref().unwrap_or(path)))
            .collect()
    }
}

/// Дороги карты с подменами на своих местах.
fn substituted<'a>(roads: &'a [RoadLine], crossings: &'a [(usize, RoadLine)]) -> Vec<&'a RoadLine> {
    let mut drawn: Vec<&RoadLine> = roads.iter().collect();
    for (index, crossing) in crossings {
        drawn[*index] = crossing;
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    #[test]
    fn every_vector_is_indexed_by_the_map_roads() {
        let mut map = MapData::default();
        map.roads.push(fixture::street(
            vec![Vec2::ZERO, Vec2::new(100.0, 0.0)],
            12.0,
        ));
        map.roads.push(fixture::street(
            vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 80.0)],
            8.0,
        ));
        let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
        assert_eq!(drawn.osm.len(), 2);
        assert_eq!(drawn.roads().len(), 2);
        assert_eq!(drawn.axes.paths.len(), 2);
        assert!(drawn.crossings.is_empty());
    }
}
