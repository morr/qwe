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

use bevy::prelude::*;

use crate::map::RoadStyle;
use crate::map::footprint::{FENCE_BAND_WIDTH, distance_to_polyline};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, BuildingUse, FenceKind, Highway, MapData, PitchKind, PolyArea, RailKind,
    RoadAreaKind, RoadClass, RoadLine, RoadNodeKind, SidewalkSide, StructureKind,
    closest_on_segment, point_in_polygon,
};
use crate::settings::{
    Z_ALLEY, Z_BRIDGE, Z_BUILDING, Z_FENCE, Z_GRASS, Z_GROUND, Z_INDUSTRY, Z_LANDUSE,
    Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PIPE, Z_PITCH, Z_POND, Z_RAIL, Z_ROAD, Z_ROAD_ISLANDS,
    Z_ROAD_PAINT, Z_ROAD_VERGE, Z_SAND, Z_SIDEWALK, Z_TRAM, Z_TREE, Z_UNPAVED_ROAD, Z_WALL,
    Z_WATERWAY, Z_WOOD,
};

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
/// своего элемента (земля, посаженное дерево, карта, собранная тестом).
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
/// строит по ней форму ([`feature_shape`]) и отличает верхнюю находку от
/// той, что была под курсором мгновение назад.
///
/// Полосы-генераты ссылаются на свою дорогу и сторону (сторона 0 — слева по
/// ходу точек, как у [`RoadLine::sidewalk`]); дерево — рангом в наборе
/// (`MapData::trees`). Земля подсветки не имеет.
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
    SidewalkBand { road: u32, side: u8 },
    VergeBand { road: u32, side: u8 },
    Ground,
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

/// Индекс признаков карты для пробы точки. Пуст до первой [`InspectIndex::rebuild`].
#[derive(Resource)]
pub struct InspectIndex {
    areas: Grid<AreaSrc>,
    lines: Grid<LineSrc>,
    points: Grid<PointSrc>,
    /// Деревья по рангу в наборе — он же порог появления; позиция и радиус
    /// читаются из [`MapData::trees`] по рангу.
    trees: Grid<u32>,
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
    pub fn rebuild(map: &MapData) -> InspectIndex {
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
        // земля — квад на всю карту: накрыт всегда
        let mut hits = vec![Hit {
            z: Z_GROUND,
            label: "ground".to_string(),
            src: HitSource::Osm,
            id: None,
            feature: FeatureRef::Ground,
        }];
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
        // сверху вниз; одинаковые подписи на одном слое — одна находка
        hits.sort_by(|a, b| b.z.total_cmp(&a.z).then_with(|| a.label.cmp(&b.label)));
        hits.dedup_by(|a, b| a.z == b.z && a.label == b.label);
        hits
    }
}

/// Сдвиг мира относительно карты: `world = map + origin`. У игры ноль; витрина
/// `roads` ставит сдвиг показанного примера (`slot - at`), чтобы панель
/// инспектора и проба говорили в метрах карты — тем же языком, что
/// `tools/osm_near`.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct InspectOrigin(pub Vec2);

/// Проводка индекса приложению: сдвиг, сам индекс и пересборка по смене
/// `MapData`. Игра берёт её из `MapPlugin`; витрина `roads`, у которой своего
/// `MapPlugin` нет, поднимает плагин сама.
pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InspectOrigin>()
            .init_resource::<InspectIndex>()
            .add_systems(
                Update,
                rebuild_index
                    .run_if(resource_exists::<MapData>.and_then(resource_changed::<MapData>)),
            );
    }
}

/// Одна находка строкой `z  label` — колонка z и отступ общие у обоих
/// выходов индекса: панели ([`format_hits`]) и отчёта пробы (`dev.rs`),
/// чтобы один и тот же признак читался в них одинаково. У находки с OSM id
/// элемента-источника подпись ведёт хвостом `#id` — теги элемента показывает
/// `tools/osm_near --id <id>`.
pub fn hit_line(hit: &Hit) -> String {
    match hit.id {
        Some(id) => format!("z {:6.3}  {} #{id}", hit.z, hit.label),
        None => format!("z {:6.3}  {}", hit.z, hit.label),
    }
}

/// Строки пробы для панели инспектора и отчёта `brp probe`: одна находка —
/// строка `z  label`, самый верхний слой первым. У находки с OSM id
/// элемента-источника подпись ведёт хвостом `#id` — теги элемента показывает
/// `tools/osm_near --id <id>`. Строк больше `cap` — хвост `+N more`.
pub fn format_hits(hits: &[Hit], cap: usize) -> String {
    let mut out = String::new();
    for hit in hits.iter().take(cap) {
        out.push_str(&hit_line(hit));
        out.push('\n');
    }
    if hits.len() > cap {
        out.push_str(&format!("+{} more\n", hits.len() - cap));
    }
    out.trim_end().to_string()
}

/// Пересобрать индекс по [`MapData`]. Ресурс меняется только при вставке
/// разобранной карты и при пересборке набора деревьев (`compose_trees`).
pub fn rebuild_index(map: Res<MapData>, mut index: ResMut<InspectIndex>) {
    let started = std::time::Instant::now();
    *index = InspectIndex::rebuild(&map);
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

/// Имя варианта в нижнем регистре — для подписей (`Residential` →
/// `residential`).
fn lower<T: std::fmt::Debug>(value: T) -> String {
    format!("{value:?}").to_lowercase()
}

/// Находка дороги в точке: полотно, а за ним полосы, которые дорога
/// дорисовывает, — тротуар и обочина. Полосы мерятся от OSM-осевой по закону
/// ширины модели ([`RoadLine::sidewalk`], [`RoadLine::verges`]); сторона —
/// по знаку к ближайшему отрезку осевой. Ручка [`RoadStyle::sidewalks`]
/// убирает обе полосы, как и у рисунка (`Drawn::verges_drawn`).
fn road_hit(road_index: u32, road: &RoadLine, style: &RoadStyle, at: Vec2) -> Option<Hit> {
    let (dist, left) = distance_and_side(at, &road.points);
    let half = road.width / 2.0;
    if dist <= half {
        return Some(Hit {
            z: road_z(road),
            label: road_label(road),
            src: HitSource::Osm,
            id: road.osm_id,
            feature: FeatureRef::Road(road_index),
        });
    }
    if !style.sidewalks {
        return None;
    }
    let profile = road.sidewalk();
    let band = profile.band()?;
    let side = usize::from(!left);
    if profile.on(side).is_some() && dist <= half + band {
        // on(side) — Some, значит сторона либо Tagged, либо Inferred
        let reason = if road.sidewalks[side] == SidewalkSide::Tagged {
            HitReason::Tagged
        } else {
            HitReason::Inferred
        };
        return Some(Hit {
            z: Z_SIDEWALK,
            label: format!("sidewalk (auto, {})", reason.as_str()),
            src: HitSource::Generated(reason),
            id: road.osm_id,
            feature: FeatureRef::SidewalkBand {
                road: road_index,
                side: side as u8,
            },
        });
    }
    if road.verges[side] > 0.0 && dist <= half + band + road.verges[side] {
        return Some(Hit {
            z: Z_ROAD_VERGE,
            label: "verge (auto)".to_string(),
            src: HitSource::Generated(HitReason::Inferred),
            id: road.osm_id,
            feature: FeatureRef::VergeBand {
                road: road_index,
                side: side as u8,
            },
        });
    }
    None
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

/// Расстояние от точки до ломаной и сторона ближайшего отрезка: `false` —
/// слева по ходу точек (сторона 0 модели), `true` — справа. Один проход
/// вместо двух: расстояние — то же, что у `distance_to_polyline`, сторона —
/// у отрезка, который его дал. Смещение меряется от отрезка к точке, и
/// `perp` смотрит влево по ходу — точка слева даёт плюс (та же сторона 0,
/// что у `parse/verges.rs`, там смещение развёрнуто). Равные отрезки —
/// к первому.
fn distance_and_side(point: Vec2, points: &[Vec2]) -> (f32, bool) {
    let mut best = (f32::INFINITY, false);
    for segment in points.windows(2) {
        let closest = closest_on_segment(point, segment[0], segment[1]);
        let dist = closest.distance(point);
        if dist < best.0 {
            let direction = (segment[1] - segment[0]).normalize_or_zero();
            let left = (point - closest).dot(direction.perp()) >= 0.0;
            best = (dist, left);
        }
    }
    best
}

/// Слой дороги: настил улицы, аллеи или грунтовой улицы, у моста — мостовой.
fn road_z(road: &RoadLine) -> f32 {
    if road.bridge {
        return Z_BRIDGE;
    }
    match road.class {
        RoadClass::Alley => Z_ALLEY,
        RoadClass::Street if road.is_unpaved_street() => Z_UNPAVED_ROAD,
        RoadClass::Street => Z_ROAD,
    }
}

/// Подпись дороги: класс `highway` и ширина — то, по чему признак ищут в
/// данных; мост и тротуары — флагами. Полоса тротуара под точкой отвечает
/// отдельной находкой (`sidewalk (auto, …)`) — по закону ширины модели;
/// точная нарисованная геометрия остаётся делом сборки дорог.
fn road_label(road: &RoadLine) -> String {
    let mut label = match road.class {
        RoadClass::Street => {
            format!("road {} {:.1} m", highway_label(road.highway), road.width)
        }
        RoadClass::Alley => format!("path {} {:.1} m", highway_label(road.highway), road.width),
    };
    if road.bridge {
        label.push_str(", bridge");
    }
    if road
        .sidewalks
        .iter()
        .any(|side| *side != SidewalkSide::None)
    {
        label.push_str(", sidewalks");
    }
    label
}

/// Имя класса `highway` для подписи: связки — через пробел, иначе Debug
/// слепил бы `motorwaylink`.
fn highway_label(highway: Highway) -> &'static str {
    match highway {
        Highway::Motorway => "motorway",
        Highway::Trunk => "trunk",
        Highway::Primary => "primary",
        Highway::Secondary => "secondary",
        Highway::Tertiary => "tertiary",
        Highway::MotorwayLink => "motorway link",
        Highway::TrunkLink => "trunk link",
        Highway::PrimaryLink => "primary link",
        Highway::SecondaryLink => "secondary link",
        Highway::TertiaryLink => "tertiary link",
        Highway::Residential => "residential",
        Highway::Unclassified => "unclassified",
        Highway::LivingStreet => "living street",
        Highway::Service => "service",
        Highway::Path => "path",
    }
}

/// Назначение здания для подписи: составные имена читаемее слитного Debug
/// (`garageblock` → `garage block`); у храма в значении вера — в подпись она
/// не нужна.
fn building_use_label(use_: BuildingUse) -> &'static str {
    match use_ {
        BuildingUse::House => "house",
        BuildingUse::Apartments => "apartments",
        BuildingUse::Commercial => "commercial",
        BuildingUse::Retail => "retail",
        BuildingUse::Industrial => "industrial",
        BuildingUse::Garage => "garage",
        BuildingUse::GarageBlock => "garage block",
        BuildingUse::Church(_) => "church",
        BuildingUse::Public => "public",
        BuildingUse::Other => "other",
    }
}

/// Подпись здания: назначение по `building=*` и высота, если тег был.
fn building_label(area: &PolyArea) -> String {
    let mut label = match area.kind {
        AreaKind::Kremlin => "kremlin building".to_string(),
        _ => match area.building_use {
            BuildingUse::Other => "building".to_string(),
            use_ => format!("building {}", building_use_label(use_)),
        },
    };
    if let Some(height) = area.height {
        label.push_str(&format!(", {:.0} m", height));
    }
    label
}

/// Подпись площадки: вид спорта читаемее номера варианта.
fn pitch_label(kind: AreaKind) -> String {
    let AreaKind::Pitch(kind) = kind else {
        return format!("pitch {}", lower(kind));
    };
    match kind {
        PitchKind::Soccer => "soccer field".to_string(),
        PitchKind::Hard => "hard court".to_string(),
        PitchKind::Track => "running track".to_string(),
        PitchKind::Playground => "playground".to_string(),
        PitchKind::Ground => "sports ground".to_string(),
    }
}

/// Подпись дорожного узла — краска, которую он кладёт на асфальт.
fn node_label(kind: RoadNodeKind) -> String {
    match kind {
        RoadNodeKind::Crossing {
            signals, island, ..
        } => {
            let mut label = String::from("crossing");
            if signals {
                label.push_str(" (signals)");
            }
            if island {
                label.push_str(" (island)");
            }
            label
        }
        RoadNodeKind::TrafficSignals => "traffic signals".to_string(),
        RoadNodeKind::Stop => "stop sign".to_string(),
        RoadNodeKind::GiveWay => "give-way sign".to_string(),
        RoadNodeKind::MiniRoundabout => "mini roundabout".to_string(),
        RoadNodeKind::TurningCircle => "turning circle".to_string(),
        RoadNodeKind::Island => "traffic island".to_string(),
    }
}

/// Подпись промышленного цилиндра.
fn structure_label(kind: StructureKind) -> String {
    match kind {
        StructureKind::Tank => "storage tank".to_string(),
        StructureKind::Silo => "silo".to_string(),
        StructureKind::Chimney => "chimney".to_string(),
        StructureKind::WaterTower => "water tower".to_string(),
        StructureKind::Gasometer => "gasometer".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;
    use crate::map::osm::model::{RoadArea, RoadAreaKind, TreeSet, WaterKind, WaterLine};

    /// Запрос с дефолтным стилем дорог — все выведенные полосы отвечают.
    fn query(index: &InspectIndex, map: &MapData, tree_density: f32, at: Vec2) -> Vec<Hit> {
        index.hits(map, tree_density, &RoadStyle::default(), at)
    }

    #[test]
    fn a_point_inside_the_feature_hits_it() {
        let map = MapData {
            buildings: vec![fixture::building(
                fixture::square(Vec2::new(100.0, 100.0), 10.0),
                vec![],
            )],
            roads: vec![fixture::street(
                vec![Vec2::new(80.0, 100.0), Vec2::new(120.0, 100.0)],
                8.0,
            )],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);

        // внутри здания и внутри полотна дороги: здание сверху, земля снизу
        let hits = query(&index, &map, 0.0, Vec2::new(100.0, 102.0));
        let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["building", "road residential 8.0 m, sidewalks", "ground"]
        );
        assert!(hits[0].z > hits[1].z && hits[1].z > hits[2].z);

        // внутри здания, но за краем полос тротуара: дороги в ответе нет
        let (_, sidewalk_edge, _) = band_edges(0.0);
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(100.0, 100.0 + sidewalk_edge + 0.1),
        );
        let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
        assert_eq!(labels, vec!["building", "ground"]);
    }

    #[test]
    fn a_point_outside_every_feature_is_ground() {
        let map = MapData {
            buildings: vec![fixture::building(
                fixture::square(Vec2::new(100.0, 100.0), 10.0),
                vec![],
            )],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(200.0, 200.0));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "ground");
    }

    #[test]
    fn a_hole_punches_the_answer_through() {
        let map = MapData {
            buildings: vec![fixture::building(
                fixture::square(Vec2::new(100.0, 100.0), 10.0),
                vec![fixture::square(Vec2::new(100.0, 100.0), 4.0)],
            )],
            water: vec![fixture::water_area(
                fixture::square(Vec2::new(100.0, 100.0), 3.0),
                vec![],
            )],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(100.0, 100.0));
        let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
        // сквозь дыру здания видна вода под ним
        assert_eq!(labels, vec!["water", "ground"]);
    }

    #[test]
    fn a_bridge_road_answers_as_a_bridge_layer() {
        let map = MapData {
            roads: vec![fixture::bridge(
                vec![Vec2::new(0.0, 50.0), Vec2::new(60.0, 50.0)],
                8.0,
            )],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(30.0, 50.0));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].label, "road residential 8.0 m, bridge, sidewalks");
        assert_eq!(hits[0].z, Z_BRIDGE);
    }

    #[test]
    fn a_culvert_is_not_drawn_so_it_is_not_a_hit() {
        // труба (`tunnel=culvert`) не рисуется вовсе — в ответе пробы ей не место
        let map = MapData {
            water_lines: vec![WaterLine {
                points: vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)],
                width: 2.5,
                kind: WaterKind::Stream,
                tunnel: true,
                osm_id: None,
            }],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(25.0, 0.0));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "ground");
    }

    #[test]
    fn an_open_waterway_is_a_hit() {
        let map = MapData {
            water_lines: vec![WaterLine {
                points: vec![Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)],
                width: 2.5,
                kind: WaterKind::Stream,
                tunnel: false,
                osm_id: None,
            }],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(25.0, 0.5));
        assert_eq!(hits[0].label, "waterway stream 2.5 m");
    }

    #[test]
    fn trees_answer_by_the_density_prefix() {
        let map = MapData {
            trees: TreeSet::of([
                (Vec2::new(100.0, 100.0), 4.0, 0.0),
                (Vec2::new(120.0, 100.0), 4.0, 0.7),
            ]),
            ..default()
        };
        let index = InspectIndex::rebuild(&map);

        // порог 0.7 выше плотности 0.5 — дерева в ответе нет
        let hits = query(&index, &map, 0.5, Vec2::new(120.0, 100.0));
        assert_eq!(
            hits,
            vec![Hit {
                z: Z_GROUND,
                label: "ground".into(),
                src: HitSource::Osm,
                id: None,
                feature: FeatureRef::Ground,
            }]
        );
        // при плотности 0.8 видно уже всё
        let hits = query(&index, &map, 0.8, Vec2::new(120.0, 100.0));
        assert_eq!(hits[0].label, "tree");
    }

    #[test]
    fn duplicates_of_one_kind_collapse() {
        let map = MapData {
            grass: vec![
                fixture::area(
                    AreaKind::Grass,
                    fixture::square(Vec2::new(100.0, 100.0), 20.0),
                ),
                fixture::area(
                    AreaKind::Grass,
                    fixture::square(Vec2::new(100.0, 100.0), 10.0),
                ),
            ],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = query(&index, &map, 0.0, Vec2::new(100.0, 100.0));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].label, "grass");
    }

    /// Улица вдоль x на оси `y`: ширина 8, тротуар и обочина по аргументам.
    fn banded_street(y: f32, verges: [f32; 2], sidewalks: [SidewalkSide; 2]) -> RoadLine {
        RoadLine {
            verges,
            sidewalks,
            ..fixture::street(vec![Vec2::new(0.0, y), Vec2::new(1000.0, y)], 8.0)
        }
    }

    /// Кромки полос такой улицы — расстояния от оси, м: полотно, край
    /// тротуара, край обочины.
    fn band_edges(verge: f32) -> (f32, f32, f32) {
        let half = 4.0;
        let band = crate::map::osm::model::sidewalk_band(8.0);
        (half, half + band, half + band + verge)
    }

    #[test]
    fn a_sidewalk_band_answers_inside_itself_and_not_beyond() {
        let map = MapData {
            roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, sidewalk_edge, _) = band_edges(0.0);

        // внутри полосы — тротуар дорисован дорогой, слой тротуаров
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
        );
        let sidewalks: Vec<&Hit> = hits
            .iter()
            .filter(|hit| hit.label.starts_with("sidewalk"))
            .collect();
        assert_eq!(sidewalks.len(), 1);
        assert_eq!(sidewalks[0].label, "sidewalk (auto, inferred)");
        assert_eq!(sidewalks[0].z, Z_SIDEWALK);

        // за краем полосы ответа нет — дороги под точкой тоже
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 70.0 + sidewalk_edge + 0.1),
        );
        assert!(hits.iter().all(|hit| hit.label == "ground"));
    }

    #[test]
    fn a_one_sided_sidewalk_answers_only_on_its_side() {
        // тротуар по тегу слева (по ходу точек — +y), справа снят разбором:
        // сторона None находки не даёт вовсе
        let map = MapData {
            roads: vec![banded_street(
                70.0,
                [0.0; 2],
                [SidewalkSide::Tagged, SidewalkSide::None],
            )],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, sidewalk_edge, _) = band_edges(0.0);
        let offset = sidewalk_edge - 0.1;

        let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + offset));
        assert_eq!(hits[0].label, "sidewalk (auto, tagged)");
        let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 - offset));
        assert!(hits.iter().all(|hit| hit.label == "ground"));
    }

    #[test]
    fn tagged_and_inferred_sides_label_their_reason() {
        let map = MapData {
            roads: vec![
                banded_street(70.0, [0.0; 2], [SidewalkSide::Tagged; 2]),
                banded_street(500.0, [0.0; 2], [SidewalkSide::Inferred; 2]),
            ],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, sidewalk_edge, _) = band_edges(0.0);

        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
        );
        assert!(
            hits.iter()
                .any(|hit| hit.label == "sidewalk (auto, tagged)")
        );
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 500.0 + sidewalk_edge - 0.1),
        );
        assert!(
            hits.iter()
                .any(|hit| hit.label == "sidewalk (auto, inferred)")
        );
    }

    #[test]
    fn the_sidewalks_knob_hides_the_generated_bands() {
        let map = MapData {
            roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, sidewalk_edge, _) = band_edges(0.0);
        let style = RoadStyle {
            sidewalks: false,
            ..RoadStyle::default()
        };

        let hits = index.hits(
            &map,
            0.0,
            &style,
            Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1),
        );
        assert!(hits.iter().all(|hit| hit.label == "ground"));
    }

    #[test]
    fn a_verge_band_answers_up_to_its_outer_edge() {
        let map = MapData {
            roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Inferred; 2])],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, _, verge_edge) = band_edges(2.0);

        // за краем тротуара — обочина, подпись без причины
        let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + verge_edge - 0.1));
        assert_eq!(hits[0].label, "verge (auto)");
        assert_eq!(hits[0].z, Z_ROAD_VERGE);
        assert_eq!(hits[0].src, HitSource::Generated(HitReason::Inferred));

        // за внешним краем обочины ответа нет
        let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0 + verge_edge + 0.1));
        assert!(hits.iter().all(|hit| hit.label == "ground"));
    }

    #[test]
    fn the_outer_edge_of_a_long_verge_is_found_across_the_cell_line() {
        // полнота сетки после роста pad: обочина шириной 2 м у дороги на
        // y = 70 дотягивается до y = 62.3 — за границу ячейки 64, которую
        // прежний pad (width/2 = 4) не задевал вовсе
        let map = MapData {
            roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Inferred; 2])],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, _, verge_edge) = band_edges(2.0);

        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 70.0 - verge_edge + 0.01),
        );
        assert!(hits.iter().any(|hit| hit.label == "verge (auto)"));
    }

    #[test]
    fn an_osm_walkway_and_a_generated_sidewalk_are_two_rows() {
        let map = MapData {
            roads: vec![banded_street(70.0, [0.0; 2], [SidewalkSide::Inferred; 2])],
            road_areas: vec![RoadArea {
                outline: fixture::rect(Vec2::new(495.0, 72.0), Vec2::new(510.0, 80.0)),
                kind: RoadAreaKind::Walkway,
                osm_id: None,
            }],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (_, sidewalk_edge, _) = band_edges(0.0);

        // OSM-площадь дорожки и дорисованный тротуар — разные находки,
        // пусть и на одном слое
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(500.0, 70.0 + sidewalk_edge - 0.5),
        );
        let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
        assert!(labels.contains(&"walkway"));
        assert!(labels.contains(&"sidewalk (auto, inferred)"));
    }

    #[test]
    fn identical_bands_of_two_roads_collapse() {
        // две улицы в девяти метрах: точка посередине лежит в полосе тротуара
        // обеих — одинаковые находки на одном слое схлопываются в одну
        let map = MapData {
            roads: vec![
                banded_street(65.5, [0.0; 2], [SidewalkSide::Inferred; 2]),
                banded_street(74.5, [0.0; 2], [SidewalkSide::Inferred; 2]),
            ],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);

        let hits = query(&index, &map, 0.0, Vec2::new(500.0, 70.0));
        let sidewalks = hits
            .iter()
            .filter(|hit| hit.label.starts_with("sidewalk"))
            .count();
        assert_eq!(sidewalks, 1);
    }

    #[test]
    fn the_osm_id_prints_as_a_tail() {
        let hits = vec![
            Hit {
                z: 1.0,
                label: "road residential 8.0 m".into(),
                src: HitSource::Osm,
                id: Some(179102449),
                feature: FeatureRef::Road(0),
            },
            Hit {
                z: 0.0,
                label: "ground".into(),
                src: HitSource::Osm,
                id: None,
                feature: FeatureRef::Ground,
            },
        ];
        let text = format_hits(&hits, 12);
        assert!(text.contains("road residential 8.0 m #179102449"));
        // у находки без элемента хвоста нет
        assert!(text.ends_with("ground"));
    }

    #[test]
    fn a_parsed_feature_carries_its_osm_id() {
        // улица с тротуаром по тегу: и находка полотна, и дорисованная полоса
        // наследуют way id своей дороги
        let overpass = fixture::Overpass::new(crate::city::City::Tula).way(
            &[("highway", "residential"), ("sidewalk", "both")],
            vec![Vec2::new(1000.0, 1400.0), Vec2::new(1400.0, 1400.0)],
        );
        let map = overpass.parse();
        let index = InspectIndex::rebuild(&map);

        let road = &map.roads[0];
        assert_eq!(road.osm_id, Some(1));

        let hits = query(&index, &map, 0.0, Vec2::new(1200.0, 1400.0));
        let road_hit = hits
            .iter()
            .find(|hit| hit.label.starts_with("road "))
            .expect("the way is a road hit");
        assert_eq!(road_hit.id, Some(1));

        let half = road.width / 2.0;
        let band = road.sidewalk().band().expect("tagged sidewalk");
        let hits = query(
            &index,
            &map,
            0.0,
            Vec2::new(1200.0, 1400.0 + half + band - 0.1),
        );
        let sidewalk = hits
            .iter()
            .find(|hit| hit.label.starts_with("sidewalk"))
            .expect("the band is a hit");
        assert_eq!(sidewalk.id, Some(1));
        assert_eq!(sidewalk.src, HitSource::Generated(HitReason::Tagged));
    }

    #[test]
    fn every_hit_carries_its_feature_reference() {
        let map = MapData {
            buildings: vec![fixture::building(
                fixture::square(Vec2::new(100.0, 100.0), 10.0),
                vec![],
            )],
            roads: vec![banded_street(70.0, [2.0; 2], [SidewalkSide::Tagged; 2])],
            water: vec![fixture::water_area(
                fixture::square(Vec2::new(200.0, 200.0), 10.0),
                vec![],
            )],
            trees: TreeSet::of([(Vec2::new(300.0, 100.0), 4.0, 0.0)]),
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let (half, sidewalk_edge, verge_edge) = band_edges(2.0);
        let feature = |at: Vec2| {
            query(&index, &map, 0.5, at)
                .into_iter()
                .next()
                .map(|hit| hit.feature)
        };

        // полосы-генераты ссылаются на свою дорогу и сторону (0 — слева, +y)
        assert_eq!(
            feature(Vec2::new(500.0, 70.0 + sidewalk_edge - 0.1)),
            Some(FeatureRef::SidewalkBand { road: 0, side: 0 })
        );
        assert_eq!(
            feature(Vec2::new(500.0, 70.0 + verge_edge - 0.1)),
            Some(FeatureRef::VergeBand { road: 0, side: 0 })
        );
        // полотно, здание, вода, дерево, земля — адреса своих элементов
        assert_eq!(
            feature(Vec2::new(500.0, 70.0 + half - 0.1)),
            Some(FeatureRef::Road(0))
        );
        assert_eq!(
            feature(Vec2::new(100.0, 102.0)),
            Some(FeatureRef::Building(0))
        );
        assert_eq!(feature(Vec2::new(200.0, 200.0)), Some(FeatureRef::Water(0)));
        assert_eq!(feature(Vec2::new(300.0, 100.0)), Some(FeatureRef::Tree(0)));
        assert_eq!(feature(Vec2::new(1000.0, 1000.0)), Some(FeatureRef::Ground));
    }

    #[test]
    fn an_unbuilt_index_answers_nothing() {
        let map = MapData::default();
        let hits = query(&InspectIndex::default(), &map, 0.0, Vec2::ZERO);
        assert!(hits.is_empty());
    }

    #[test]
    fn the_cap_cuts_the_list_with_a_tail() {
        let hits: Vec<Hit> = (0..5)
            .map(|i| Hit {
                z: i as f32,
                label: format!("x{i}"),
                src: HitSource::Osm,
                id: None,
                feature: FeatureRef::Ground,
            })
            .collect();
        assert_eq!(format_hits(&hits, 12).lines().count(), 5);
        let capped = format_hits(&hits, 3);
        assert_eq!(capped.lines().count(), 4);
        assert!(capped.ends_with("+2 more"));
    }
}
