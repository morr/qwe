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
//! Полоса тротуара, дорисованная дорогой, мерится от OSM-осевой по закону
//! ширины модели — и потому отвечает за **модель**, а не за рисунок:
//! нарисованный тротуар сглажен и у узлов срезан скруглениями; переезды через
//! тротуар (driveway crossings), кусок в проёме пары половин (`across_median`)
//! и стежки индекс не знает. Состав обочины за тротуаром, наоборот, —
//! **рисунком**: отрисованные куски (`roads::DrawnPieces`, `FeatureRef::Piece`)
//! собирает проход сборки дорог — точный контур каждого логического куска
//! (полоса обочины на ран, лоскут, кусок узла) с его слоем, — и находка с
//! подсветкой ведут ровно то, что лежит на карте. Зум-LOD обочин
//! (`VergeZoomBucket`) индексу по-прежнему не виден — как и LOD оград;
//! точный арбитр по нарисованным слоям — та же проба слоёв.
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
    AreaLayer, FenceKind, MapData, PolyArea, RailKind, RoadAreaKind, RoadLine, lower,
    point_in_polygon, ring_bounds,
};
use crate::map::roads::pockets::kerb_pocket_outlines;
use crate::map::roads::{DrawnPiece, DrawnPieces, PieceKind};
use crate::settings::{
    Z_FENCE, Z_INDUSTRY, Z_PIPE, Z_RAIL, Z_ROAD, Z_ROAD_ISLANDS, Z_ROAD_PAINT, Z_SIDEWALK, Z_TRAM,
    Z_TREE, Z_WALL, Z_WATERWAY,
};

#[cfg(test)]
use crate::map::osm::model::{Highway, SidewalkSide};
#[cfg(test)]
use crate::settings::Z_BRIDGE;

mod labels;
mod shapes;

use self::labels::{node_label, piece_label, road_hit, structure_label};
pub use self::labels::{format_hits, hit_line, pinned_line};
pub use self::shapes::{
    FeatureShape, PinnedFill, feature_shape, mesh_feature, mesh_pins, pin_stripe,
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
/// Площадная находка (вид слоя [`AreaLayer`], позиция в его поле) ведёт в
/// то же место, что источник в индексе: вектор-слой и позиция в нём.
/// Полосы-генераты ссылаются на свою дорогу и сторону (сторона 0 — слева по
/// ходу точек, как у [`RoadLine::sidewalk`]); дерево — рангом в наборе
/// (`MapData::trees`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureRef {
    /// Площадная находка: вид слоя и позиция в его поле [`MapData`].
    Area(AreaLayer, u32),
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
    /// Отрисованный кусок дороги — по номеру в индексе
    /// ([`InspectIndex::pieces`]): состав обочины, лоскуты и куски узлов в
    /// [`MapData`] не живут, их рисует проход сборки дорог.
    Piece(u32),
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
    /// Тротуар по правдоподобию (`parse::infer_sidewalks`). Отрисованные
    /// куски обочины (`FeatureRef::Piece`) несут его же без различения
    /// причины: тега у обочины нет, и подпись куска (`piece_label`) короткая —
    /// `verge paving (auto)` и родственные, без хвоста причины.
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

/// Откуда площадная находка — по вектору [`MapData`]: вид слоя и позиция в
/// его поле; вектор даёт слой, а `kind` полигона — члена слоя (здание или
/// крепость, карман-двор или луг). Площади дорог — не полигоны [`PolyArea`],
/// в перечне видов им места нет, поэтому отдельный рукав.
#[derive(Clone, Copy)]
enum AreaSrc {
    Area(AreaLayer, u32),
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
    /// Отрисованные куски дорог (`roads::DrawnPiece`): состав обочины,
    /// лоскуты, куски узлов; сетка хранит номер в векторе.
    pieces: Vec<DrawnPiece>,
    piece_grid: Grid<u32>,
    built: bool,
    /// База (`prebuild`) ждёт куски мира: `Some` — форма, под которую
    /// собраны базовые проходы; пока форма та же, их догоняет
    /// `apply_pieces` (пятый проход — дешёвый). Дрейф формы — полная
    /// пересборка.
    pending_shape: Option<RoadShape>,
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
            pieces: Vec::new(),
            piece_grid: Grid::new(CELL),
            built: false,
            pending_shape: None,
        }
    }

    /// Индекс ещё не построен (карты нет или она пустая).
    pub fn is_built(&self) -> bool {
        self.built
    }

    /// Индексировать карту целиком: базовые проходы ([`InspectIndex::prebuild`])
    /// плюс отрисованные куски ([`InspectIndex::apply_pieces`]). Полный путь —
    /// галерея, дрейф формы, пересборка дорог при игре; на входе в мир база
    /// едет из лоад-потока готовой, и куски догоняются отдельно.
    pub fn rebuild(map: &MapData, shape: &RoadShape, pieces: &[DrawnPiece]) -> InspectIndex {
        let mut index = InspectIndex::prebuild(map, shape);
        index.apply_pieces(pieces);
        index
    }

    /// База индекса — четыре прохода по карте и карманы у бордюра: площади —
    /// габаритом контура, линии — каждым отрезком с полушириной
    /// (`Grid::insert_segment`), точки — кругом. Полная, а не приблизительная,
    /// выборка одной ячейкой держится на инварианте сетки: значение лежит в
    /// каждой ячейке, которой касается его бокс, раздутый ровно на досягаемость
    /// запроса.
    ///
    /// Карманы у бордюра — четвёртый проход: их в [`MapData`] нет, полигоны
    /// выводит закон стоянки (`roads::pockets::kerb_pocket_outlines`) на
    /// узловой оси — той же, по которой карман рисуется; форма влияет на ось,
    /// поэтому база собирается под форму. Дороже прочих проходов — подъём
    /// каркаса дорог, у машин тот же.
    ///
    /// Лоад-поток собирает базу параллельно лейнам (`finish_world`): на входе
    /// в мир остаётся догнать куски — [`InspectIndex::apply_pieces`] — вместо
    /// полной пересборки в warmup-окне.
    pub fn prebuild(map: &MapData, shape: &RoadShape) -> InspectIndex {
        let mut index = InspectIndex::empty();
        let mut push_area = |src: AreaSrc, area: &PolyArea| {
            if area.outer.is_empty() {
                return;
            }
            let (min, max) = ring_bounds(&area.outer);
            index.areas.insert(min, max, src);
        };
        // площади — по перечню видов: члены перечня с `index` отвечают пробе,
        // поля берутся у дескриптора
        for desc in AreaLayer::ALL {
            if !desc.index {
                continue;
            }
            for (i, area) in (desc.field)(map).iter().enumerate() {
                push_area(AreaSrc::Area(desc.layer, i as u32), area);
            }
        }
        for (i, area) in map.road_areas.iter().enumerate() {
            if area.outline.is_empty() {
                continue;
            }
            let (min, max) = ring_bounds(&area.outline);
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
            let (min, max) = ring_bounds(&outline.contour);
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
        index.pending_shape = Some(*shape);
        index
    }

    /// Пятый проход — отрисованные куски дорог: их рисует проход сборки
    /// (`roads::build_roads`), в [`MapData`] они не живут. Ресурс
    /// `DrawnPieces` меняется при каждой пересборке дорог — стиль, форма,
    /// ступень обочины, — и индекс догоняет его этим же методом; габарит и
    /// вставка в сетку — дешёвые, на входе в мир они проходят вместо полной
    /// пересборки.
    pub fn apply_pieces(&mut self, pieces: &[DrawnPiece]) {
        self.piece_grid = Grid::new(CELL);
        for (i, piece) in pieces.iter().enumerate() {
            if piece.contour.is_empty() {
                continue;
            }
            let (min, max) = ring_bounds(&piece.contour);
            self.piece_grid.insert(min, max, i as u32);
        }
        self.pieces = pieces.to_vec();
        self.pending_shape = None;
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
            let hit = match src {
                // площадь дорог — не полигоны `PolyArea`, в перечне видов её
                // нет: свой слой и своя подпись по виду площади
                AreaSrc::RoadArea(i) => match map.road_areas.get(i as usize) {
                    Some(area) if point_in_polygon(at, &area.outline) => {
                        let (z, label) = match area.kind {
                            RoadAreaKind::Carriageway => (Z_ROAD, "carriageway area".to_string()),
                            RoadAreaKind::Walkway => (Z_SIDEWALK, "walkway".to_string()),
                            RoadAreaKind::Island => (Z_ROAD_ISLANDS, "traffic island".to_string()),
                        };
                        Hit {
                            z,
                            label,
                            src: HitSource::Osm,
                            id: area.osm_id,
                            feature: FeatureRef::RoadArea(i),
                        }
                    }
                    _ => continue,
                },
                // прочие площади — по перечню видов: слой находки и подпись
                // даёт дескриптор, у кварталов и карманов их решает вид
                // полигона внутри его `hit`
                AreaSrc::Area(layer, i) => {
                    let Some(area) = layer.field(map).get(i as usize) else {
                        continue;
                    };
                    if !inside(area) {
                        continue;
                    }
                    let Some((z, label)) = (layer.desc().hit)(area) else {
                        continue;
                    };
                    Hit {
                        z,
                        label,
                        src: HitSource::Osm,
                        id: area.osm_id,
                        feature: FeatureRef::Area(layer, i),
                    }
                }
            };
            hits.push(hit);
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
        // отрисованные куски: ручка «Sidewalks» прячет состав обочины так же,
        // как у рисунка (`Drawn::verges_drawn` без ручки возвращает нули);
        // лоскуты и куски узлов от неё не зависят
        for &i in self.piece_grid.at(at) {
            let Some(piece) = self.pieces.get(i as usize) else {
                continue;
            };
            if matches!(
                piece.kind,
                PieceKind::VergePaving | PieceKind::VergeLawn | PieceKind::VergeYard
            ) && !roads.sidewalks
            {
                continue;
            }
            if !point_in_polygon(at, &piece.contour) {
                continue;
            }
            if piece.holes.iter().any(|hole| point_in_polygon(at, hole)) {
                continue;
            }
            hits.push(Hit {
                z: piece.z,
                label: piece_label(piece.kind),
                src: HitSource::Generated(HitReason::Inferred),
                id: piece
                    .road
                    .and_then(|road| map.roads.get(road as usize))
                    .and_then(|road| road.osm_id),
                feature: FeatureRef::Piece(i),
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
            // смену ресурсов судит сама система (см. `rebuild_index`), условие
            // только гейтит отсутствие карты до первой загрузки
            .add_systems(Update, rebuild_index.run_if(resource_exists::<MapData>));
    }
}

/// Пересобрать индекс по [`MapData`] и отрисованным кускам. Ресурс карты
/// меняется только при вставке разобранной карты и при пересборке набора
/// деревьев (`compose_trees`). Форма дорог — второй повод: узловая ось, по
/// которой стоят карманы у бордюра, зависит от неё. Куски — третий: их
/// записывает каждая пересборка дорог (стиль, форма, ступень обочины).
///
/// Смену судит сама система, а не `resource_changed` в run-condition: их
/// окна change-detection расходятся, когда пишущая система идёт в том же
/// кадре параллельно с этой (проверено на живом Париже — условие зажигало
/// две лишние пересборки на входе в мир при `is_changed() == false` внутри,
/// по четверти секунды каждая, и всё это в окне warmup-холда).
pub fn rebuild_index(
    map: Res<MapData>,
    shape: Res<RoadShapeOnMap>,
    pieces: Res<DrawnPieces>,
    mut index: ResMut<InspectIndex>,
) {
    if !map.is_changed() && !shape.is_changed() && !pieces.is_changed() {
        return;
    }
    // База из лоад-потока ждёт куски под той же формой: догнать их — вместо
    // полной пересборки в первом кадре мира. Порядок кадра гарантирует
    // свежесть кусков: вставки `poll_job` применяются в конце кадра Done, а
    // спавн-цепочка (`StateTransition`) кладёт новые `DrawnPieces` до первого
    // Update следующего кадра — между ними эта система не запустится.
    if index.pending_shape == Some(shape.0) {
        let started = std::time::Instant::now();
        index.apply_pieces(&pieces.0);
        info!("inspect index: pieces applied in {:.1?}", started.elapsed());
        return;
    }
    let started = std::time::Instant::now();
    *index = InspectIndex::rebuild(&map, &shape.0, &pieces.0);
    info!("inspect index: rebuilt in {:.1?}", started.elapsed());
}

/// Досягаемость дороги в индексе: полотно плюс полоса тротуара, которую она
/// дорисовывает (обе стороны сразу: индекс строится один раз, ручка
/// «Sidewalks» может быть перевёрнута позже). Состав обочины за тротуаром
/// отвечают отрисованные куски со своей сеткой, дорога туда не дотягивается.
/// Pad сетки обязан накрыть самый дальний ответ, иначе внешний край полос у
/// соседней ячейки отвечал бы пусто.
fn road_reach(road: &RoadLine) -> f32 {
    road.width / 2.0 + road.sidewalk().band().unwrap_or(0.0)
}

#[cfg(test)]
mod tests;
