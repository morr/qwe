//! Геометрическая модель карты в мировых метрах (юго-западный угол — (0,0)).
//!
//! Субъекты модели живут в каталоге `model/`: `features` (площадные зоны,
//! здания и линейные сооружения), `roads` (дорожный словарь), `trees`
//! (посадка и её политика), `geometry` (примитивы поверх контуров); здесь
//! остаются сам ресурс `MapData` и общие типы. Все имена субъектов
//! реэкспортируются ниже — наружу путь `osm::model::X` неизменен.

use bevy::prelude::*;

mod features;
mod geometry;
mod roads;
mod trees;

pub use features::{
    AreaKind, BuildingUse, Colours, Faith, FenceKind, FenceLine, LotKind, PipeLine, PitchKind,
    PolyArea, RailKind, RailLine, Rgb, Sacred, SacredForm, ServiceTrack, Structure, StructureKind,
    WallLine, WaterKind, WaterLine, srgba_of, water_line_caps,
};
pub use geometry::{
    BIG_BOX_AREA_MIN, BIG_BOX_MAX_HEIGHT, BIG_BOX_MAX_LEVELS, closest_on_segment, distance_to_area,
    distance_to_outline, distance_to_segment, is_big_box, is_big_box_shape, is_fortress_tower,
    point_at_arc_length, point_in_area, point_in_polygon, polyline_length, ring_area, ring_bounds,
    ring_vertex_mean, signed_ring_area,
};
pub use roads::{
    Highway, KerbParking, LaneTurn, Pavement, RoadArea, RoadAreaKind, RoadClass, RoadLine,
    RoadNode, RoadNodeKind, SIDEWALK_WIDTH_RANGE, SidewalkProfile, SidewalkSide, sidewalk_band,
};
pub use trees::{
    PlantedTree, RowTrees, TreeCompose, TreeNode, TreeRow, TreeRowLayout, TreeRowPlacement, TreeSet,
};

/// По какой стороне дороги идёт поток — тег `driving_side` на границе страны
/// (`parse::driving_side`). Решает, у какого бордюра стоит ряд односторонней
/// улицы и куда смотрят носы припаркованных машин (`map::cars`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrafficSide {
    #[default]
    Right,
    Left,
}

impl TrafficSide {
    /// Знак поперечной `direction.perp()` (она смотрит влево по ходу), на
    /// которой лежит бордюр своей полосы: при правостороннем движении это
    /// правая сторона, `-1`.
    pub fn kerb(self) -> f32 {
        match self {
            Self::Right => -1.0,
            Self::Left => 1.0,
        }
    }
}

/// Распарсенная карта; остаётся ресурсом после спавна — для отладки.
#[derive(Resource, Debug, Default)]
pub struct MapData {
    /// Сторона движения страны, в которой лежит карта. Без тега в ответе
    /// (зеркало без областей, `is_in` пуст) — правостороннее, с предупреждением
    /// при разборе.
    pub traffic_side: TrafficSide,
    /// С какими входами разобрана карта — снимок [`ParseKnobs`](super::parse::ParseKnobs),
    /// которые получил разбор. Его ширину полосы сравнивает с осевшей ручкой
    /// `city::lane_width_moved` (разошлись — перезагрузка), и из него же на
    /// входе в мир пишется глобаль краски (`roads::shape::adopt_lane_width`).
    /// У карты, собранной тестом руками, — входы по умолчанию.
    pub knobs: super::parse::ParseKnobs,
    /// **Контуры OSM** — геометрия, какой её прочёл элементный цикл, до
    /// доводочных проходов (`super::contours`): по ней рисуется оверлей
    /// `OSM contours`, который отвечает «данные или наш разбор». У карты,
    /// собранной тестом руками, пуст.
    pub osm_contours: super::contours::OsmContours,
    pub buildings: Vec<PolyArea>,
    pub water: Vec<PolyArea>,
    pub parks: Vec<PolyArea>,
    /// Лесные массивы — обычно внутри парков; только здесь растут деревья.
    pub woods: Vec<PolyArea>,
    /// Луга/газоны — лежат поверх парков и не засаживаются деревьями.
    pub grass: Vec<PolyArea>,
    /// Песчаные пляжи — тоже поверх парков и без деревьев.
    pub sand: Vec<PolyArea>,
    /// Кварталы `landuse` (жильё, промзона) — самая нижняя заливка, под
    /// парками; на навмеш и посадку деревьев не влияют.
    pub landuse: Vec<PolyArea>,
    /// **Карманы земли** — замкнутые нарисованным клочки у дорог, которые
    /// никакой тег не описал, засеянные травой (`parse/pockets.rs`): вид —
    /// вид соседнего квартала, `Grass` у замапленного газона или сквера
    /// (рисуется лугом в слое газонов), иначе `Residential` — трава двора.
    /// Рисуются слоями кварталов, но ни кварталом, ни газоном не считаются:
    /// луг обочины спрашивает замапленную зелень (`roads.rs::Meadows`), а
    /// карман рядом — не повод красить её лугом.
    pub pockets: Vec<PolyArea>,
    /// Стоянки (`amenity=parking`) — асфальт с разметкой мест; по ним же
    /// расставляются машины. Навмеш не трогают: по стоянке ходят. Вид каждой
    /// ([`LotKind`]) в её `kind`.
    pub parking: Vec<PolyArea>,
    /// Спортивные и детские площадки (`leisure=*`) — покрытие своего цвета и
    /// разметка (`map::pitch`). Навмеш не трогают: по площадке ходят.
    pub pitches: Vec<PolyArea>,
    pub roads: Vec<RoadLine>,
    /// Дорожные узлы (`highway=crossing|traffic_signals|…`) — см. [`RoadNode`].
    pub road_nodes: Vec<RoadNode>,
    /// Площади дорог, нарисованные контуром, — см. [`RoadArea`].
    pub road_areas: Vec<RoadArea>,
    /// Дороги, склеенные в улицы, — собирается проходом сечений при разборе
    /// (`map::roads::network::sections`) по окончательному [`MapData::roads`].
    /// Пуста у карты, собранной тестом руками: тогда клиньев между сечениями
    /// нет, а всё остальное рисуется как прежде.
    pub network: crate::map::roads::network::RoadNetwork,
    /// Ж/д пути — только для отрисовки, в навмеш не попадают.
    pub rails: Vec<RailLine>,
    pub walls: Vec<WallLine>,
    /// Ограды участков (`barrier=fence|wall|retaining_wall|hedge`) — в навмеш
    /// попадают с проёмами дорог и калитками: см. [`FenceLine`].
    pub fences: Vec<FenceLine>,
    /// Резервуары, силосы, трубы, башни (`man_made=*`) — только рисуются,
    /// навмеш не трогают: см. [`Structure`].
    pub structures: Vec<Structure>,
    /// Надземные трубопроводы (`man_made=pipeline`): см. [`PipeLine`].
    pub pipes: Vec<PipeLine>,
    /// Линейные водотоки — реки, ручьи, каналы, канавы. В навмеш попадают
    /// (кроме труб), в отличие от рельсов: см. [`WaterLine`].
    pub water_lines: Vec<WaterLine>,
    /// Аллеи (`natural=tree_row`) — исходная геометрия, для отладки; деревья по
    /// ним уже разложены в [`MapData::row_trees_kept`] / [`MapData::row_trees_slid`].
    pub tree_rows: Vec<TreeRow>,
    /// Одиночные деревья (`node natural=tree`) — сырые ноды, для отладки;
    /// посаженные лежат в [`MapData::standalone_trees`].
    pub tree_nodes: Vec<TreeNode>,
    /// Посаженные одиночные деревья, все с порогом 0 — дерево из данных видно
    /// всегда. Сырьё для [`MapData::compose_trees`], как и лес с аллеями.
    pub standalone_trees: Vec<PlantedTree>,
    /// Деревья лесных полигонов, по возрастанию порога появления. Сырьё для
    /// [`MapData::compose_trees`], а не то, что читает рендер.
    pub wood_trees: Vec<PlantedTree>,
    /// Деревья аллей под каждую раскладку, по возрастанию порога.
    pub row_trees: RowTrees,
    /// Из чего собран [`MapData::trees`]; `None` — ещё не собран.
    ///
    /// Признак живёт в `MapData`, а не в `Local` системы, именно потому, что при
    /// смене города ресурс заменяется целиком: `Local` пережил бы замену и
    /// решил, что для нового города всё уже собрано.
    pub composed_for: Option<TreeCompose>,
    /// Деревья карты. Детерминированы данными карты, собираются
    /// [`MapData::compose_trees`] из леса и аллей выбранной политики; всё
    /// остальное (рендер, тени, поле хвои, ползунок плотности) видит только
    /// этот набор и про аллеи не знает.
    pub trees: TreeSet,
}

impl MapData {
    /// Собрать [`MapData::trees`] из включённых источников: одиночные деревья,
    /// лес и аллеи выбранной раскладки.
    ///
    /// Все слагаемые уже отсортированы по порогу появления, так что это слияние,
    /// а не сортировка: префикс по плотности ([`TreeSet::visible_count`]) обязан
    /// оставаться монотонным, иначе шаг ползунка вверх убирал бы деревья.
    pub fn compose_trees(&mut self, compose: TreeCompose) {
        // разбор по полям, а не `self.…`: аллеи читаются, пока выход пишется
        let MapData {
            row_trees,
            wood_trees,
            standalone_trees,
            trees,
            ..
        } = self;
        let empty: &[PlantedTree] = &[];
        let rows = if compose.rows {
            row_trees.get(compose.layout)
        } else {
            empty
        };
        let wood_trees: &[PlantedTree] = if compose.woods { wood_trees } else { empty };
        let standalone: &[PlantedTree] = if compose.standalone {
            standalone_trees
        } else {
            empty
        };

        trees.clear();
        trees.reserve(standalone.len() + wood_trees.len() + rows.len());

        // одиночные первыми: у всех порог 0, ниже любого лесного, и на равных
        // порогах с OSM-аллеями они и раньше стояли впереди
        for &tree in standalone {
            trees.push(tree);
        }

        let (mut wood, mut row) = (0, 0);
        while wood < wood_trees.len() || row < rows.len() {
            // при равных порогах первым идёт лес — порядок должен быть
            // детерминированным, иначе поле хвои и оттенки крон разъезжаются
            let take_wood = match (wood_trees.get(wood), rows.get(row)) {
                (Some(&(.., wood_at)), Some(&(.., row_at))) => wood_at <= row_at,
                (Some(_), None) => true,
                _ => false,
            };
            let &tree = if take_wood {
                wood += 1;
                &wood_trees[wood - 1]
            } else {
                row += 1;
                &rows[row - 1]
            };
            trees.push(tree);
        }

        self.composed_for = Some(compose);
    }
}

#[cfg(test)]
mod tests;
