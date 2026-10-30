//! Семантическая проба карты: что нарисовано в точке — по признакам
//! [`MapData`], без пересборки мешей.
//!
//! Индекс ([`InspectIndex`]) строится один раз на загрузку мира и отвечает
//! «какие объекты карты накрывают точку» — здание с назначением, дорога с
//! классом `highway`, дерево, рельсы, русло — с z того слоя, где каждый
//! рисуется. Это дешёвый ответ (одна ячейка сетки [`Grid`] плюс десяток
//! точных проверок), и его слушают двое: панель инспектора
//! (`ui/inspector.rs`) на каждом движении курсора и раздел `features at the
//! point` в отчёте пробы слоёв (`dev.rs::ProbeLayersEvent`, `brp probe`).
//!
//! Точную геометрию *нарисованных* слоёв даёт только проба слоёв — она
//! пересобирает меши и стоит сотни миллисекунд. Индекс же говорит про
//! **признаки**: то, что выводится из `MapData`, — без краски, теней и
//! зум-LOD; дерево видно по порогу плотности ([`TreeSet::visible_count`]),
//! а труб-кульвертов, которые не рисуются, в ответе нет вовсе.
//!
//! Полосы, дорисованные дорогой (тротуар, обочина), индекс мерит от OSM-осевой
//! по закону ширины модели — и потому отвечает за **модель**, а не за рисунок.
//! Нарисованный тротуар сглажен и у узлов срезан скруглениями; переезды через
//! тротуар (driveway crossings), кусок в проёме пары половин (`across_median`)
//! и стежки индекс не знает; зум-LOD обочин (`VergeZoomBucket`) ему тоже не
//! виден — как и LOD оград. Точный арбитр по нарисованному — та же проба
//! слоёв; привязать полосы к геометрии строителей дорог
//! (`map::roads::Drawn`) — следующий шаг, этой задачей сознательно не
//! сделанный: граница «индекс отвечает моделью, проба — рисунком».
//!
//! Единственное исключение — карманы у бордюра: их в `MapData` нет вовсе, и
//! закон стоянки живёт на узловой оси (`roads::pockets::kerb_pocket_outlines`,
//! тот же подъём каркаса, что у машин) — индекс считает полигоны на ней же и
//! хранит их сам. Это по-прежнему ответ **моделью** — о кармане как признаке,
//! а не о нарисованном асфальте, — но построенный на оси рисунка; потому
//! индекс перестраивается и по смене формы дорог.

use bevy::prelude::*;

use crate::map::RoadShape;
use crate::map::RoadShapeOnMap;
use crate::map::RoadStyle;
use crate::map::footprint::{FENCE_BAND_WIDTH, distance_to_polyline};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, FenceKind, MapData, PolyArea, RailKind, RoadAreaKind, RoadLine, point_in_polygon,
};
use crate::map::roads::pockets::kerb_pocket_outlines;
use crate::settings::{
    Z_BUILDING, Z_FENCE, Z_GRASS, Z_INDUSTRY, Z_LANDUSE, Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PIPE,
    Z_PITCH, Z_POND, Z_RAIL, Z_ROAD, Z_ROAD_ISLANDS, Z_ROAD_PAINT, Z_SAND, Z_SIDEWALK, Z_TRAM,
    Z_TREE, Z_WALL, Z_WATERWAY, Z_WOOD,
};

#[cfg(test)]
use crate::map::osm::model::{Highway, SidewalkSide};
#[cfg(test)]
use crate::settings::{Z_BRIDGE, Z_ROAD_VERGE};

mod labels;
mod shapes;

use self::labels::{building_label, lower, node_label, pitch_label, road_hit, structure_label};
pub use self::labels::{format_hits, hit_line};
pub use self::shapes::{FeatureShape, feature_shape, mesh_feature};

/// Сторона ячейки индекса, м.
const CELL: f32 = 64.0;
/// Радиус, в котором дорожный узел («здесь зебра») считается накрывшим точку, м:
/// сам узел стоит на оси, и его почти всегда накрывает и своя дорога.
const NODE_REACH: f32 = 4.0;

/// Одна находка пробы: подпись признака и z слоя, где он рисуется.
///
/// `src` — откуда находка: признак данных или полоса, дорисованная дорогой
/// (у генератов подпись несёт суффикс `(auto, …)`). `id` — OSM id
/// элемента-источника, печать ведёт его хвостом `#…`; `None` — находка без
/// своего элемента (посаженное дерево, карта, собранная тестом).
/// `feature` — адрес элемента в [`MapData`] для подсветки формы
/// ([`feature_shape`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub z: f32,
    pub label: String,
    pub src: HitSource,
    pub id: Option<u64>,
    pub feature: FeatureRef,
}

/// Ссылка находки на свой элемент карты — индекс в векторе [`MapData`].
/// Потребитель один: подсветка элемента под курсором (`ui/inspector.rs`)
/// строит по ней форму ([`feature_shape`]) и отличает выбранную находку от
/// той, что подсвечивалась мгновение назад.
///
/// Полосы-генераты ссылаются на свою дорогу и сторону (сторона 0 — слева по
/// ходу точек, как у [`RoadLine::sidewalk`]); дерево — рангом в наборе
/// (`MapData::trees`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureRef {
    Building(u32),
    Water(u32),
    Park(u32),
    Wood(u32),
    Grass(u32),
    Sand(u32),
    Landuse(u32),
    Pocket(u32),
    Parking(u32),
    Pitch(u32),
    RoadArea(u32),
    Road(u32),
    Rail(u32),
    Waterway(u32),
    Pipe(u32),
    Wall(u32),
    Fence(u32),
    RoadNode(u32),
    Structure(u32),
    Tree(u32),
    /// Карман у бордюра — по номеру в индексе ([`InspectIndex::kerb_pockets`]):
    /// в [`MapData`] кармана нет, его выводит закон стоянки
    /// (`roads::pockets::kerb_pocket_outlines`).
    KerbPocket(u32),
    SidewalkBand {
        road: u32,
        side: u8,
    },
    VergeBand {
        road: u32,
        side: u8,
    },
}

/// Откуда находка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitSource {
    /// Признак из данных выгрузки.
    Osm,
    /// Полоса, дорисованная дорогой разбором, — с причиной постановки.
    Generated(HitReason),
}

/// Причина, по которой разбор дорисовал полосу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitReason {
    /// Тротуар по тегу `sidewalk=*`.
    Tagged,
    /// Тротуар по правдоподобию (`parse::infer_sidewalks`). Обочина тоже
    /// несёт его: тега у обочины нет, её меряет разбор (`parse/verges.rs`),
    /// — но в подписи обочины причина не различается, там короткое
    /// `verge (auto)`.
    Inferred,
}

impl HitReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tagged => "tagged",
            Self::Inferred => "inferred",
        }
    }
}

/// Откуда площадная находка — по вектору [`MapData`]; вектор даёт слой, а
/// `kind` полигона — члена слоя (здание или крепость, карман-двор или луг).
#[derive(Clone, Copy)]
enum AreaSrc {
    Building(u32),
    Water(u32),
    Park(u32),
    Wood(u32),
    Grass(u32),
    Sand(u32),
    Landuse(u32),
    Pocket(u32),
    Parking(u32),
    Pitch(u32),
    RoadArea(u32),
}

/// Линейная находка — отрезок с полушириной.
#[derive(Clone, Copy)]
enum LineSrc {
    Road(u32),
    Rail(u32),
    Waterway(u32),
    Pipe(u32),
    Wall(u32),
    Fence(u32),
}

/// Точечная находка — круг.
#[derive(Clone, Copy)]
enum PointSrc {
    RoadNode(u32),
    Structure(u32),
}

/// Карман у бордюра, выведенный законом стоянки (`roads::pockets`): контура в
/// [`MapData`] нет — полигон хранит индекс, посчитав его на той же узловой
/// оси, по которой карман рисуется.
struct KerbPocket {
    outline: Vec<Vec2>,
    /// Чей карман: OSM id находки — дороги под ним.
    road: u32,
    /// Карман по тегу `parking:<side>` (иначе — по правилу).
    tagged: bool,
}

/// Индекс признаков карты для пробы точки. Пуст до первой [`InspectIndex::rebuild`].
#[derive(Resource)]
pub struct InspectIndex {
    areas: Grid<AreaSrc>,
    lines: Grid<LineSrc>,
    points: Grid<PointSrc>,
    /// Деревья по рангу в наборе — он же порог появления; позиция и радиус
    /// читаются из [`MapData::trees`] по рангу.
    trees: Grid<u32>,
    /// Карманы у бордюра со своими полигонами; сетка хранит номер в векторе.
    kerb_pockets: Vec<KerbPocket>,
    kerb_pocket_grid: Grid<u32>,
    built: bool,
}

impl Default for InspectIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl InspectIndex {
    fn empty() -> Self {
        Self {
            areas: Grid::new(CELL),
            lines: Grid::new(CELL),
            points: Grid::new(CELL),
            trees: Grid::new(CELL),
            kerb_pockets: Vec::new(),
            kerb_pocket_grid: Grid::new(CELL),
            built: false,
        }
    }

    /// Индекс ещё не построен (карты нет или она пустая).
    pub fn is_built(&self) -> bool {
        self.built
    }

    /// Индексировать карту: площади — габаритом контура, линии — каждым
    /// отрезком с полушириной (`Grid::insert_segment`), точки — кругом.
    /// Полная, а не приблизительная, выборка одной ячейкой держится на
    /// инварианте сетки: значение лежит в каждой ячейке, которой касается
    /// его бокс, раздутый ровно на досягаемость запроса.
    ///
    /// Карманы у бордюра — четвёртый проход: их в [`MapData`] нет, полигоны
    /// выводит закон стоянки (`roads::pockets::kerb_pocket_outlines`) на
    /// узловой оси — той же, по которой карман рисуется; форма влияет на ось,
    /// поэтому индекс перестраивается и по ней. Дороже прочих проходов —
    /// подъём каркаса дорог, у машин тот же.
    pub fn rebuild(map: &MapData, shape: &RoadShape) -> InspectIndex {
        let mut index = InspectIndex::empty();
        let mut push_area = |src: AreaSrc, area: &PolyArea| {
            if area.outer.is_empty() {
                return;
            }
            let (min, max) = bounds(&area.outer);
            index.areas.insert(min, max, src);
        };
        for (i, area) in map.buildings.iter().enumerate() {
            push_area(AreaSrc::Building(i as u32), area);
        }
        for (i, area) in map.water.iter().enumerate() {
            push_area(AreaSrc::Water(i as u32), area);
        }
        for (i, area) in map.parks.iter().enumerate() {
            push_area(AreaSrc::Park(i as u32), area);
        }
        for (i, area) in map.woods.iter().enumerate() {
            push_area(AreaSrc::Wood(i as u32), area);
        }
        for (i, area) in map.grass.iter().enumerate() {
            push_area(AreaSrc::Grass(i as u32), area);
        }
        for (i, area) in map.sand.iter().enumerate() {
            push_area(AreaSrc::Sand(i as u32), area);
        }
        for (i, area) in map.landuse.iter().enumerate() {
            push_area(AreaSrc::Landuse(i as u32), area);
        }
        for (i, area) in map.pockets.iter().enumerate() {
            push_area(AreaSrc::Pocket(i as u32), area);
        }
        for (i, area) in map.parking.iter().enumerate() {
            push_area(AreaSrc::Parking(i as u32), area);
        }
        for (i, area) in map.pitches.iter().enumerate() {
            push_area(AreaSrc::Pitch(i as u32), area);
        }
        for (i, area) in map.road_areas.iter().enumerate() {
            if area.outline.is_empty() {
                continue;
            }
            let (min, max) = bounds(&area.outline);
            index.areas.insert(min, max, AreaSrc::RoadArea(i as u32));
        }

        let mut push_line = |src: LineSrc, points: &[Vec2], pad: f32| {
            if points.len() < 2 {
                return;
            }
            for pair in points.windows(2) {
                index.lines.insert_segment(pair[0], pair[1], pad, src);
            }
        };
        for (i, road) in map.roads.iter().enumerate() {
            push_line(LineSrc::Road(i as u32), &road.points, road_reach(road));
        }
        for (i, rail) in map.rails.iter().enumerate() {
            push_line(LineSrc::Rail(i as u32), &rail.points, rail.width / 2.0);
        }
        // труб-кульверт не рисует никто — в ответе пробы ему не место
        for (i, line) in map
            .water_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.tunnel)
        {
            push_line(LineSrc::Waterway(i as u32), &line.points, line.width / 2.0);
        }
        for (i, pipe) in map.pipes.iter().enumerate() {
            push_line(LineSrc::Pipe(i as u32), &pipe.points, pipe.width / 2.0);
        }
        for (i, wall) in map.walls.iter().enumerate() {
            push_line(LineSrc::Wall(i as u32), &wall.points, wall.width / 2.0);
        }
        // физическая толщина ограды, не экранная: зум-LOD индекс не видит
        for (i, fence) in map.fences.iter().enumerate() {
            push_line(
                LineSrc::Fence(i as u32),
                &fence.points,
                FENCE_BAND_WIDTH / 2.0,
            );
        }

        let mut push_point = |src: PointSrc, at: Vec2, radius: f32| {
            let pad = Vec2::splat(radius.max(0.0));
            index.points.insert(at - pad, at + pad, src);
        };
        for (i, node) in map.road_nodes.iter().enumerate() {
            push_point(PointSrc::RoadNode(i as u32), node.pos, NODE_REACH);
        }
        for (i, structure) in map.structures.iter().enumerate() {
            push_point(
                PointSrc::Structure(i as u32),
                structure.at,
                structure.radius,
            );
        }
        for (rank, (at, radius)) in map.trees.positions().iter().enumerate() {
            let pad = Vec2::splat(radius.max(0.0));
            index.trees.insert(*at - pad, *at + pad, rank as u32);
        }
        let pocket_started = std::time::Instant::now();
        for outline in kerb_pocket_outlines(map, shape) {
            if outline.contour.is_empty() {
                continue;
            }
            let (min, max) = bounds(&outline.contour);
            let i = index.kerb_pockets.len() as u32;
            index.kerb_pockets.push(KerbPocket {
                outline: outline.contour,
                road: outline.road as u32,
                tagged: outline.tagged,
            });
            index.kerb_pocket_grid.insert(min, max, i);
        }
        info!(
            "inspect index: {} kerb pockets in {:.1?}",
            index.kerb_pockets.len(),
            pocket_started.elapsed()
        );
        index.built = true;
        index
    }

    /// Что нарисовано в точке: находки сверху вниз по слою, одинаковые
    /// схлопнуты. `tree_density` — [`TreeStyle::density`]: дерево видно, пока
    /// его ранг меньше префикса плотности. `roads` — [`RoadStyle`]: ручка
    /// «Sidewalks» прячет выведенные полосы так же, как у рисунка
    /// (`Drawn::verges_drawn` без ручки возвращает нули).
    pub fn hits(&self, map: &MapData, tree_density: f32, roads: &RoadStyle, at: Vec2) -> Vec<Hit> {
        if !self.built {
            return Vec::new();
        }
        // пустого фона в ответе нет: земля под всем — не находка
        let mut hits = Vec::new();
        let inside = |area: &PolyArea| {
            point_in_polygon(at, &area.outer)
                && !area.holes.iter().any(|hole| point_in_polygon(at, hole))
        };
        for &src in self.areas.at(at) {
            let (z, label, id) = match src {
                AreaSrc::Building(i) => match map.buildings.get(i as usize) {
                    Some(area) if inside(area) => (Z_BUILDING, building_label(area), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Water(i) => match map.water.get(i as usize) {
                    Some(area) if inside(area) => (Z_POND, "water".to_string(), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Park(i) => match map.parks.get(i as usize) {
                    Some(area) if inside(area) => (Z_PARK, "park".to_string(), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Wood(i) => match map.woods.get(i as usize) {
                    Some(area) if inside(area) => (Z_WOOD, "wood".to_string(), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Grass(i) => match map.grass.get(i as usize) {
                    Some(area) if inside(area) => (Z_GRASS, "grass".to_string(), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Sand(i) => match map.sand.get(i as usize) {
                    Some(area) if inside(area) => (Z_SAND, "sand".to_string(), area.osm_id),
                    _ => continue,
                },
                AreaSrc::Landuse(i) => match map.landuse.get(i as usize) {
                    Some(area) if inside(area) => (
                        match area.kind {
                            AreaKind::Industrial => Z_LANDUSE,
                            _ => Z_LANDUSE_YARD,
                        },
                        format!("landuse {}", lower(area.kind)),
                        area.osm_id,
                    ),
                    _ => continue,
                },
                AreaSrc::Pocket(i) => match map.pockets.get(i as usize) {
                    Some(area) if inside(area) => (
                        match area.kind {
                            AreaKind::Grass => Z_GRASS,
                            _ => Z_LANDUSE_YARD,
                        },
                        format!("pocket {}", lower(area.kind)),
                        area.osm_id,
                    ),
                    _ => continue,
                },
                AreaSrc::Parking(i) => match map.parking.get(i as usize) {
                    Some(area) if inside(area) => {
                        let AreaKind::Parking(kind) = area.kind else {
                            continue;
                        };
                        (
                            Z_PARKING,
                            format!("parking lot {}", lower(kind)),
                            area.osm_id,
                        )
                    }
                    _ => continue,
                },
                AreaSrc::Pitch(i) => match map.pitches.get(i as usize) {
                    Some(area) if inside(area) => (Z_PITCH, pitch_label(area.kind), area.osm_id),
                    _ => continue,
                },
                AreaSrc::RoadArea(i) => match map.road_areas.get(i as usize) {
                    Some(area) if point_in_polygon(at, &area.outline) => (
                        match area.kind {
                            RoadAreaKind::Carriageway => Z_ROAD,
                            RoadAreaKind::Walkway => Z_SIDEWALK,
                            RoadAreaKind::Island => Z_ROAD_ISLANDS,
                        },
                        match area.kind {
                            RoadAreaKind::Carriageway => "carriageway area".to_string(),
                            RoadAreaKind::Walkway => "walkway".to_string(),
                            RoadAreaKind::Island => "traffic island".to_string(),
                        },
                        area.osm_id,
                    ),
                    _ => continue,
                },
            };
            hits.push(Hit {
                z,
                label,
                src: HitSource::Osm,
                id,
                feature: area_feature(src),
            });
        }
        for &src in self.lines.at(at) {
            let hit = match src {
                LineSrc::Road(i) => map
                    .roads
                    .get(i as usize)
                    .and_then(|road| road_hit(i, road, roads, at)),
                LineSrc::Rail(i) => map.rails.get(i as usize).and_then(|rail| {
                    (distance_to_polyline(at, &rail.points) <= rail.width / 2.0).then(|| Hit {
                        z: match rail.kind {
                            RailKind::Tram => Z_TRAM,
                            _ => Z_RAIL,
                        },
                        label: match rail.kind {
                            RailKind::Tram => "tram".to_string(),
                            RailKind::Disused => "disused rail".to_string(),
                            RailKind::Active => "rail".to_string(),
                        },
                        src: HitSource::Osm,
                        id: rail.osm_id,
                        feature: FeatureRef::Rail(i),
                    })
                }),
                LineSrc::Waterway(i) => map.water_lines.get(i as usize).and_then(|line| {
                    (distance_to_polyline(at, &line.points) <= line.width / 2.0).then(|| Hit {
                        z: Z_WATERWAY,
                        label: format!("waterway {} {:.1} m", lower(line.kind), line.width),
                        src: HitSource::Osm,
                        id: line.osm_id,
                        feature: FeatureRef::Waterway(i),
                    })
                }),
                LineSrc::Pipe(i) => map.pipes.get(i as usize).and_then(|pipe| {
                    (distance_to_polyline(at, &pipe.points) <= pipe.width / 2.0).then(|| Hit {
                        z: Z_PIPE,
                        label: "heating pipe".to_string(),
                        src: HitSource::Osm,
                        id: pipe.osm_id,
                        feature: FeatureRef::Pipe(i),
                    })
                }),
                LineSrc::Wall(i) => map.walls.get(i as usize).and_then(|wall| {
                    (distance_to_polyline(at, &wall.points) <= wall.width / 2.0).then(|| Hit {
                        z: Z_WALL,
                        label: "city wall".to_string(),
                        src: HitSource::Osm,
                        id: wall.osm_id,
                        feature: FeatureRef::Wall(i),
                    })
                }),
                LineSrc::Fence(i) => map.fences.get(i as usize).and_then(|fence| {
                    (distance_to_polyline(at, &fence.points) <= FENCE_BAND_WIDTH / 2.0).then(|| {
                        Hit {
                            z: Z_FENCE,
                            label: match fence.kind {
                                FenceKind::Fence => "fence".to_string(),
                                FenceKind::Wall => "fence wall".to_string(),
                                FenceKind::Hedge => "hedge".to_string(),
                                FenceKind::Railing => "railing".to_string(),
                            },
                            src: HitSource::Osm,
                            id: fence.osm_id,
                            feature: FeatureRef::Fence(i),
                        }
                    })
                }),
            };
            if let Some(hit) = hit {
                hits.push(hit);
            }
        }
        for &src in self.points.at(at) {
            let hit = match src {
                PointSrc::RoadNode(i) => map.road_nodes.get(i as usize).and_then(|node| {
                    (at.distance(node.pos) <= NODE_REACH).then(|| Hit {
                        z: Z_ROAD_PAINT,
                        label: node_label(node.kind),
                        src: HitSource::Osm,
                        id: node.osm_id,
                        feature: FeatureRef::RoadNode(i),
                    })
                }),
                PointSrc::Structure(i) => map.structures.get(i as usize).and_then(|structure| {
                    (at.distance(structure.at) <= structure.radius).then(|| Hit {
                        z: Z_INDUSTRY,
                        label: format!(
                            "{}, {:.0} m tall",
                            structure_label(structure.kind),
                            structure.height
                        ),
                        src: HitSource::Osm,
                        id: structure.osm_id,
                        feature: FeatureRef::Structure(i),
                    })
                }),
            };
            if let Some(hit) = hit {
                hits.push(hit);
            }
        }
        let visible = map.trees.visible_count(tree_density);
        for &rank in self.trees.at(at) {
            if rank as usize >= visible {
                continue;
            }
            let Some((pos, radius)) = map.trees.positions().get(rank as usize) else {
                continue;
            };
            if at.distance(*pos) <= *radius {
                hits.push(Hit {
                    z: Z_TREE,
                    label: "tree".to_string(),
                    src: HitSource::Osm,
                    id: None,
                    feature: FeatureRef::Tree(rank),
                });
            }
        }
        for &i in self.kerb_pocket_grid.at(at) {
            let Some(pocket) = self.kerb_pockets.get(i as usize) else {
                continue;
            };
            if !point_in_polygon(at, &pocket.outline) {
                continue;
            }
            let reason = if pocket.tagged {
                HitReason::Tagged
            } else {
                HitReason::Inferred
            };
            hits.push(Hit {
                z: Z_ROAD,
                label: format!("parking pocket (auto, {})", reason.as_str()),
                src: HitSource::Generated(reason),
                id: map
                    .roads
                    .get(pocket.road as usize)
                    .and_then(|road| road.osm_id),
                feature: FeatureRef::KerbPocket(i),
            });
        }
        // сверху вниз; одинаковые подписи на одном слое — одна находка
        hits.sort_by(|a, b| b.z.total_cmp(&a.z).then_with(|| a.label.cmp(&b.label)));
        hits.dedup_by(|a, b| a.z == b.z && a.label == b.label);
        hits
    }
}

/// Сдвиг мира относительно карты: `world = map + origin`. У игры ноль; витрина
/// `roads` каждый кадр ставит сдвиг **текущего** примера — того, чей viewport
/// под курсором, а когда курсор между окнами, — показанного (`frame_sample`
/// витрины): панель инспектора, слой подсветки и проба говорят в метрах карты
/// одного и того же среза — тем же языком, что `tools/osm_near`.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct InspectOrigin(pub Vec2);

/// Окно просмотра курсора — viewport примера витрины метрами карты. Игра
/// рассечённых окон не знает: вся карта и есть окно ([`InspectViewport::Whole`]).
///
/// Пишет витрина каждый кадр (по курсору и колонке примеров), читают панель
/// инспектора (видимость, гейт попаданий) и слой подсветки (клип по окну —
/// подсветка не выходит за пределы viewport, даже если сам объект больше).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub enum InspectViewport {
    /// Курсор внутри окна примера: окно метрами карты.
    Inside(Rect),
    /// Курсор между окнами (или над подписью): панели нет, подсветки нет.
    Outside,
    /// Игра: окон нет, вся карта — одно окно. Дефолт: игра ресурс не пишет
    /// никогда, и «снаружи» по умолчанию гасил бы ей попадания и панель, а
    /// «внутри» требовал бы окна, которого у игры нет.
    #[default]
    Whole,
}

impl InspectViewport {
    /// Точка карты внутри окна просмотра.
    pub fn contains(&self, map_point: Vec2) -> bool {
        match self {
            Self::Inside(window) => window.contains(map_point),
            Self::Whole => true,
            Self::Outside => false,
        }
    }

    /// Чем резать подсветку (`MeshBuilder::clip_to_rect`): окно просмотра,
    /// метрами карты. `None` — не резать: у игры подсветке резать нечего.
    pub fn clip(&self) -> Option<Rect> {
        match self {
            Self::Inside(window) => Some(*window),
            Self::Whole | Self::Outside => None,
        }
    }
}

/// Проводка индекса приложению: сдвиг, сам индекс и пересборка по смене
/// `MapData`. Игра берёт её из `MapPlugin`; витрина `roads`, у которой своего
/// `MapPlugin` нет, поднимает плагин сама.
pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InspectOrigin>()
            .init_resource::<InspectViewport>()
            .init_resource::<InspectIndex>()
            .add_systems(
                Update,
                rebuild_index.run_if(resource_exists::<MapData>.and_then(
                    resource_changed::<MapData>.or_else(resource_changed::<RoadShapeOnMap>),
                )),
            );
    }
}

/// Пересобрать индекс по [`MapData`]. Ресурс меняется только при вставке
/// разобранной карты и при пересборке набора деревьев (`compose_trees`).
/// Форма дорог — второй повод: узловая ось, по которой стоят карманы у
/// бордюра, зависит от неё.
pub fn rebuild_index(
    map: Res<MapData>,
    shape: Res<RoadShapeOnMap>,
    mut index: ResMut<InspectIndex>,
) {
    let started = std::time::Instant::now();
    *index = InspectIndex::rebuild(&map, &shape.0);
    info!("inspect index: rebuilt in {:.1?}", started.elapsed());
}

/// Габарит кольца.
fn bounds(points: &[Vec2]) -> (Vec2, Vec2) {
    let (mut min, mut max) = (points[0], points[0]);
    for point in points {
        min = min.min(*point);
        max = max.max(*point);
    }
    (min, max)
}

/// Ссылка площадной находки на свой элемент — тот же индекс, что у источника
/// в индексе: вектор-слой и позиция в нём.
fn area_feature(src: AreaSrc) -> FeatureRef {
    match src {
        AreaSrc::Building(i) => FeatureRef::Building(i),
        AreaSrc::Water(i) => FeatureRef::Water(i),
        AreaSrc::Park(i) => FeatureRef::Park(i),
        AreaSrc::Wood(i) => FeatureRef::Wood(i),
        AreaSrc::Grass(i) => FeatureRef::Grass(i),
        AreaSrc::Sand(i) => FeatureRef::Sand(i),
        AreaSrc::Landuse(i) => FeatureRef::Landuse(i),
        AreaSrc::Pocket(i) => FeatureRef::Pocket(i),
        AreaSrc::Parking(i) => FeatureRef::Parking(i),
        AreaSrc::Pitch(i) => FeatureRef::Pitch(i),
        AreaSrc::RoadArea(i) => FeatureRef::RoadArea(i),
    }
}

/// Досягаемость дороги в индексе: полотно плюс полосы, которые она
/// дорисовывает, — тротуар и обочина (обе стороны сразу: индекс строится
/// один раз, ручка «Sidewalks» может быть перевёрнута позже). Pad сетки
/// обязан накрыть самый дальний ответ, иначе внешний край полос у соседней
/// ячейки отвечал бы пусто.
fn road_reach(road: &RoadLine) -> f32 {
    road.width / 2.0
        + road
            .sidewalk()
            .band()
            .map(|band| band + road.verges[0].max(road.verges[1]))
            .unwrap_or(0.0)
}

#[cfg(test)]
mod tests;
