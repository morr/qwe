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

use bevy::prelude::*;

use crate::map::footprint::{FENCE_BAND_WIDTH, distance_to_polyline};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, BuildingUse, FenceKind, Highway, MapData, PitchKind, PolyArea, RailKind,
    RoadAreaKind, RoadClass, RoadLine, RoadNodeKind, SidewalkSide, StructureKind, point_in_polygon,
};
use crate::settings::{
    Z_ALLEY, Z_BRIDGE, Z_BUILDING, Z_FENCE, Z_GRASS, Z_GROUND, Z_INDUSTRY, Z_LANDUSE,
    Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PIPE, Z_PITCH, Z_POND, Z_RAIL, Z_ROAD, Z_ROAD_ISLANDS,
    Z_ROAD_PAINT, Z_SAND, Z_SIDEWALK, Z_TRAM, Z_TREE, Z_UNPAVED_ROAD, Z_WALL, Z_WATERWAY, Z_WOOD,
};

/// Сторона ячейки индекса, м.
const CELL: f32 = 64.0;
/// Радиус, в котором дорожный узел («здесь зебра») считается накрывшим точку, м:
/// сам узел стоит на оси, и его почти всегда накрывает и своя дорога.
const NODE_REACH: f32 = 4.0;

/// Одна находка пробы: подпись признака и z слоя, где он рисуется.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub z: f32,
    pub label: String,
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

        let mut push_line = |src: LineSrc, points: &[Vec2], width: f32| {
            if points.len() < 2 {
                return;
            }
            for pair in points.windows(2) {
                index
                    .lines
                    .insert_segment(pair[0], pair[1], width / 2.0, src);
            }
        };
        for (i, road) in map.roads.iter().enumerate() {
            push_line(LineSrc::Road(i as u32), &road.points, road.width);
        }
        for (i, rail) in map.rails.iter().enumerate() {
            push_line(LineSrc::Rail(i as u32), &rail.points, rail.width);
        }
        // труб-кульверт не рисует никто — в ответе пробы ему не место
        for (i, line) in map
            .water_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.tunnel)
        {
            push_line(LineSrc::Waterway(i as u32), &line.points, line.width);
        }
        for (i, pipe) in map.pipes.iter().enumerate() {
            push_line(LineSrc::Pipe(i as u32), &pipe.points, pipe.width);
        }
        for (i, wall) in map.walls.iter().enumerate() {
            push_line(LineSrc::Wall(i as u32), &wall.points, wall.width);
        }
        // физическая толщина ограды, не экранная: зум-LOD индекс не видит
        for (i, fence) in map.fences.iter().enumerate() {
            push_line(LineSrc::Fence(i as u32), &fence.points, FENCE_BAND_WIDTH);
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
    /// его ранг меньше префикса плотности.
    pub fn hits(&self, map: &MapData, tree_density: f32, at: Vec2) -> Vec<Hit> {
        if !self.built {
            return Vec::new();
        }
        // земля — квад на всю карту: накрыт всегда
        let mut hits = vec![Hit {
            z: Z_GROUND,
            label: "ground".to_string(),
        }];
        let inside = |area: &PolyArea| {
            point_in_polygon(at, &area.outer)
                && !area.holes.iter().any(|hole| point_in_polygon(at, hole))
        };
        for &src in self.areas.at(at) {
            let (z, label) = match src {
                AreaSrc::Building(i) => match map.buildings.get(i as usize) {
                    Some(area) if inside(area) => (Z_BUILDING, building_label(area)),
                    _ => continue,
                },
                AreaSrc::Water(i) => match map.water.get(i as usize) {
                    Some(area) if inside(area) => (Z_POND, "water".to_string()),
                    _ => continue,
                },
                AreaSrc::Park(i) => match map.parks.get(i as usize) {
                    Some(area) if inside(area) => (Z_PARK, "park".to_string()),
                    _ => continue,
                },
                AreaSrc::Wood(i) => match map.woods.get(i as usize) {
                    Some(area) if inside(area) => (Z_WOOD, "wood".to_string()),
                    _ => continue,
                },
                AreaSrc::Grass(i) => match map.grass.get(i as usize) {
                    Some(area) if inside(area) => (Z_GRASS, "grass".to_string()),
                    _ => continue,
                },
                AreaSrc::Sand(i) => match map.sand.get(i as usize) {
                    Some(area) if inside(area) => (Z_SAND, "sand".to_string()),
                    _ => continue,
                },
                AreaSrc::Landuse(i) => match map.landuse.get(i as usize) {
                    Some(area) if inside(area) => (
                        match area.kind {
                            AreaKind::Industrial => Z_LANDUSE,
                            _ => Z_LANDUSE_YARD,
                        },
                        format!("landuse {}", lower(area.kind)),
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
                    ),
                    _ => continue,
                },
                AreaSrc::Parking(i) => match map.parking.get(i as usize) {
                    Some(area) if inside(area) => {
                        let AreaKind::Parking(kind) = area.kind else {
                            continue;
                        };
                        (Z_PARKING, format!("parking lot {}", lower(kind)))
                    }
                    _ => continue,
                },
                AreaSrc::Pitch(i) => match map.pitches.get(i as usize) {
                    Some(area) if inside(area) => (Z_PITCH, pitch_label(area.kind)),
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
                    ),
                    _ => continue,
                },
            };
            hits.push(Hit { z, label });
        }
        for &src in self.lines.at(at) {
            let hit = match src {
                LineSrc::Road(i) => map.roads.get(i as usize).and_then(|road| {
                    (distance_to_polyline(at, &road.points) <= road.width / 2.0).then(|| Hit {
                        z: road_z(road),
                        label: road_label(road),
                    })
                }),
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
                    })
                }),
                LineSrc::Waterway(i) => map.water_lines.get(i as usize).and_then(|line| {
                    (distance_to_polyline(at, &line.points) <= line.width / 2.0).then(|| Hit {
                        z: Z_WATERWAY,
                        label: format!("waterway {} {:.1} m", lower(line.kind), line.width),
                    })
                }),
                LineSrc::Pipe(i) => map.pipes.get(i as usize).and_then(|pipe| {
                    (distance_to_polyline(at, &pipe.points) <= pipe.width / 2.0).then(|| Hit {
                        z: Z_PIPE,
                        label: "heating pipe".to_string(),
                    })
                }),
                LineSrc::Wall(i) => map.walls.get(i as usize).and_then(|wall| {
                    (distance_to_polyline(at, &wall.points) <= wall.width / 2.0).then(|| Hit {
                        z: Z_WALL,
                        label: "city wall".to_string(),
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

/// Строки пробы для панели инспектора и отчёта `brp probe`: одна находка —
/// строка `z  label`, самый верхний слой первым. Строк больше `cap` — хвост
/// `+N more`.
pub fn format_hits(hits: &[Hit], cap: usize) -> String {
    let mut out = String::new();
    for hit in hits.iter().take(cap) {
        out.push_str(&format!("z {:6.3}  {}\n", hit.z, hit.label));
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
/// данных; мост и тротуары — флагами, точную полосу тротуара индекс не
/// считает (это геометрия сборки дорог).
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
    use crate::map::osm::model::{TreeSet, WaterKind, WaterLine};

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
        let hits = index.hits(&map, 0.0, Vec2::new(100.0, 102.0));
        let labels: Vec<&str> = hits.iter().map(|hit| hit.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["building", "road residential 8.0 m, sidewalks", "ground"]
        );
        assert!(hits[0].z > hits[1].z && hits[1].z > hits[2].z);

        // внутри здания, но вне полотна: дороги в ответе нет
        let hits = index.hits(&map, 0.0, Vec2::new(100.0, 107.0));
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
        let hits = index.hits(&map, 0.0, Vec2::new(200.0, 200.0));
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
        let hits = index.hits(&map, 0.0, Vec2::new(100.0, 100.0));
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
        let hits = index.hits(&map, 0.0, Vec2::new(30.0, 50.0));
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
            }],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = index.hits(&map, 0.0, Vec2::new(25.0, 0.0));
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
            }],
            ..default()
        };
        let index = InspectIndex::rebuild(&map);
        let hits = index.hits(&map, 0.0, Vec2::new(25.0, 0.5));
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
        let hits = index.hits(&map, 0.5, Vec2::new(120.0, 100.0));
        assert_eq!(
            hits,
            vec![Hit {
                z: Z_GROUND,
                label: "ground".into()
            }]
        );
        // при плотности 0.8 видно уже всё
        let hits = index.hits(&map, 0.8, Vec2::new(120.0, 100.0));
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
        let hits = index.hits(&map, 0.0, Vec2::new(100.0, 100.0));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].label, "grass");
    }

    #[test]
    fn an_unbuilt_index_answers_nothing() {
        let map = MapData::default();
        let hits = InspectIndex::default().hits(&map, 0.0, Vec2::ZERO);
        assert!(hits.is_empty());
    }

    #[test]
    fn the_cap_cuts_the_list_with_a_tail() {
        let hits: Vec<Hit> = (0..5)
            .map(|i| Hit {
                z: i as f32,
                label: format!("x{i}"),
            })
            .collect();
        assert_eq!(format_hits(&hits, 12).lines().count(), 5);
        let capped = format_hits(&hits, 3);
        assert_eq!(capped.lines().count(), 4);
        assert!(capped.ends_with("+2 more"));
    }
}
