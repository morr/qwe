//! **Drawn** — дороги карты так, как они рисуются: переезды через тротуар
//! асфальтом проезда и дуги колец сечением всего кольца, общие узлы, оси улиц.
//! Строится один раз на [`mesh_roads`](super::mesh_roads) и отдаёт
//! подмодулям то, что раньше жило локалами в его начале.
//!
//! Все векторы — по индексу в `map.roads`, длины равны. Ни одна правка
//! `RoadLine::points` не трогает: навмеш, двери и деревья видят OSM как есть.

use bevy::prelude::*;

use super::axis::{self, Axes};
use super::network::{self, RoadNodes};
use super::ring_arcs;
use super::shape::RoadShape;
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
}

impl<'m> Drawn<'m> {
    pub fn new(map: &'m MapData, shape: &RoadShape) -> Self {
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
        Self {
            osm: roads,
            nodes,
            axes,
            crossings,
        }
    }

    /// Дороги как рисуются, по индексу `map.roads`: подмены на своих местах.
    pub fn roads(&self) -> Vec<&RoadLine> {
        let mut drawn: Vec<&RoadLine> = self.osm.iter().collect();
        for (index, crossing) in &self.crossings {
            drawn[*index] = crossing;
        }
        drawn
    }
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
        let drawn = Drawn::new(&map, &RoadShape::default());
        assert_eq!(drawn.osm.len(), 2);
        assert_eq!(drawn.roads().len(), 2);
        assert_eq!(drawn.axes.paths.len(), 2);
        assert!(drawn.crossings.is_empty());
    }
}
