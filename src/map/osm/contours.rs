//! **Контуры OSM** — геометрия карты такой, какой её прочёл элементный цикл
//! разбора, до единого доводочного прохода, и оверлей из неё поверх
//! обработанной карты.
//!
//! Зачем: на всякий кривой кусок карты первый вопрос разбора — «это данные
//! OSM или наш разбор?». Доводочные проходы (`parse::finish_parse`) двигают
//! геометрию: выпрямляют подходы к кольцам, отодвигают дома от тротуаров,
//! подтягивают кварталы и стоянки к дорогам, выпрямляют косые домики, сеют
//! карманы, а рендер достраивает поверх ещё больше. Снимок, сделанный до них,
//! отвечает на вопрос одним кадром: контур совпал с кривым местом — виноваты
//! данные, не совпал — наша обработка.
//!
//! Два читателя одного слоя: строка `OSM contours` вкладки Debug игры
//! (`ui/debug`) и строка `Contours` панели витрины `roads` (`ROADS_CONTOURS=1`
//! для автоснимка) — главный потребитель, где «данные против разбора»
//! сравниваются на одном перекрёстке.

use bevy::prelude::*;

use crate::map::MeshBuilder;
use crate::map::meshing::{RibbonCap, RibbonJoin};
use crate::map::osm::model::{MapData, PolyArea, RoadClass};
use crate::map::surface::{LayerMesh, MaterialSpec};

/// Над оверлеем сети (`roads/network/overlay.rs`, 29): когда горят оба,
/// тонкая линия данных читается поверх толстой линии улицы.
const Z_CONTOURS: f32 = 29.5;
/// Толщина оси улицы, м — тоньше самой узкой ленты разметки не нужно: линия
/// должна лечь по кромке и не закрыть её.
const STREET_LINE: f32 = 0.35;
/// Толщина оси дорожки и прочих линий (рельсы, водотоки, ограды), м.
const PATH_LINE: f32 = 0.25;
/// Толщина контура полигона, м.
const AREA_LINE: f32 = 0.25;
/// Сторона квадратика на вершине пути, м: вершина — это узел OSM, и по ней
/// видно, где маппер поставил точку, а где её дорисовал наш разбор.
const VERTEX_SIDE: f32 = 0.7;

/// Ось улицы — пурпур: на сером асфальте и на зелени его не спутать ни с
/// краской, ни с оверлеем сети.
const STREET_COLOR: Color = Color::srgb(1.0, 0.1, 0.8);
/// Ось дорожки — бирюза.
const PATH_COLOR: Color = Color::srgb(0.0, 0.85, 0.95);
/// Прочие линии (рельсы, водотоки, ограды, стены, аллеи, трубы) — синий.
const LINE_COLOR: Color = Color::srgb(0.25, 0.35, 1.0);
/// Контур полигона — жёлтый.
const AREA_COLOR: Color = Color::srgb(1.0, 0.9, 0.0);

/// Геометрия карты до доводочных проходов — снимок в конце элементного цикла
/// (`parse::read_elements`). Только точки: теги, классы и ширины уже лежат в
/// самой `MapData`, а оверлею нужна форма.
///
/// Цена — копия точек: на Туле это строка `osm parse: … contour points` в логе
/// разбора (вершин порядка сотен тысяч, мегабайты памяти и миллисекунды на
/// копию).
#[derive(Debug, Default, Clone)]
pub struct OsmContours {
    /// Оси улиц и проездов (`RoadClass::Street`), как их прочёл цикл: `oneway=-1`
    /// уже развёрнут, подземное уже выброшено — это решения чтения, не правки.
    pub streets: Vec<Vec<Vec2>>,
    /// Оси дорожек (`RoadClass::Alley`).
    pub paths: Vec<Vec<Vec2>>,
    /// Прочие линии: рельсы, водотоки, ограды, стены, аллеи, трубы.
    pub lines: Vec<Vec<Vec2>>,
    /// Кольца полигонов — наружные и дырки, открытые (без повтора первой
    /// точки), всех площадных слоёв и площадей дорог.
    pub rings: Vec<Vec<Vec2>>,
}

impl OsmContours {
    /// Снимок сырой карты. Зовётся **до** `finish_parse`, иначе снимать
    /// незачем.
    pub fn of(map: &MapData) -> Self {
        let mut contours = Self::default();
        for road in &map.roads {
            let target = match road.class {
                RoadClass::Street => &mut contours.streets,
                RoadClass::Alley => &mut contours.paths,
            };
            target.push(road.points.clone());
        }
        contours.lines.extend(
            map.rails
                .iter()
                .map(|line| line.points.clone())
                .chain(map.water_lines.iter().map(|line| line.points.clone()))
                .chain(map.fences.iter().map(|line| line.points.clone()))
                .chain(map.walls.iter().map(|line| line.points.clone()))
                .chain(map.tree_rows.iter().map(|line| line.points.clone()))
                .chain(map.pipes.iter().map(|line| line.points.clone())),
        );
        let areas: [&[PolyArea]; 9] = [
            &map.buildings,
            &map.water,
            &map.parks,
            &map.woods,
            &map.grass,
            &map.sand,
            &map.landuse,
            &map.parking,
            &map.pitches,
        ];
        for area in areas.into_iter().flatten() {
            contours.rings.push(area.outer.clone());
            contours.rings.extend(area.holes.iter().cloned());
        }
        contours
            .rings
            .extend(map.road_areas.iter().map(|area| area.outline.clone()));
        contours
    }

    /// Сколько вершин в снимке — для строки лога и оценки памяти.
    pub fn points(&self) -> usize {
        [&self.streets, &self.paths, &self.lines, &self.rings]
            .into_iter()
            .flatten()
            .map(Vec::len)
            .sum()
    }
}

/// Слой оверлея контуров OSM карты `map`: оси путей тонкими линиями с
/// квадратиками на вершинах, контуры полигонов замкнутыми линиями. Плоский
/// материал без фактуры, над зданиями и кронами.
pub fn mesh_osm_contours(contours: &OsmContours) -> LayerMesh {
    let mut builder = MeshBuilder::default();
    let mut line = |points: &[Vec2], closed: bool, width: f32, color: Color| {
        if points.len() < 2 {
            return;
        }
        builder.push_ribbon(
            points,
            closed,
            width,
            color.to_linear(),
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    };
    for ring in &contours.rings {
        line(ring, true, AREA_LINE, AREA_COLOR);
    }
    for points in &contours.lines {
        line(points, false, PATH_LINE, LINE_COLOR);
    }
    for points in &contours.paths {
        line(points, false, PATH_LINE, PATH_COLOR);
    }
    for points in &contours.streets {
        line(points, false, STREET_LINE, STREET_COLOR);
    }
    // вершины путей — поверх их линий
    let half = Vec2::splat(VERTEX_SIDE / 2.0);
    for (ways, color) in [
        (&contours.paths, PATH_COLOR),
        (&contours.streets, STREET_COLOR),
    ] {
        let color = color.to_linear();
        for &point in ways.iter().flatten() {
            builder.push_rect(point - half, point + half, color);
        }
    }
    LayerMesh::new(builder, Z_CONTOURS, "osm_contours", MaterialSpec::Flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{building, street};

    #[test]
    fn a_street_and_a_house_draw_their_contours() {
        let map = MapData {
            roads: vec![street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0)],
            buildings: vec![building(
                vec![
                    Vec2::new(10.0, 10.0),
                    Vec2::new(20.0, 10.0),
                    Vec2::new(20.0, 20.0),
                ],
                Vec::new(),
            )],
            ..Default::default()
        };
        let contours = OsmContours::of(&map);
        assert_eq!(contours.streets.len(), 1);
        assert_eq!(contours.rings.len(), 1);
        assert_eq!(contours.points(), 5);
        let layer = mesh_osm_contours(&contours);
        assert_eq!(layer.name, "osm_contours");
        assert!(layer.builder.vertex_count() > 0);
    }
}
