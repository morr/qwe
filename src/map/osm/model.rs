//! Геометрическая модель карты в мировых метрах (юго-западный угол — (0,0)).

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    Building,
    Kremlin,
    Water,
    /// Парк — светлая подложка без деревьев (`leisure=park|garden`).
    Park,
    /// Лес внутри парка (`natural=wood` / `landuse=forest`) — единственное, что
    /// засаживается деревьями; рисуется темнее парка.
    Wood,
    /// Луг/газон: светлее парка и без деревьев (`landuse=grass|meadow`).
    Grass,
    /// Пляж или песчаная отмель (`natural=sand|beach`), тоже без деревьев.
    Sand,
}

/// Полигон с дырками. Кольца открытые: последняя точка не повторяет первую.
#[derive(Debug, Clone)]
pub struct PolyArea {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
    pub kind: AreaKind,
    /// Высота здания в метрах из OSM (`parse::building_height`). `None` —
    /// тегов нет либо это не здание: у воды, парков и лугов высоты не бывает.
    /// Покрытие сильно зависит от города (Берлин 80%, Токио 5%), поэтому
    /// потребитель обязан иметь свой дефолт, а не считать `None` ошибкой.
    pub height: Option<f32>,
    /// Входы (`entrance=*`) на контуре этого здания. Пусто у подавляющего
    /// большинства домов и у всего, что не здание — потребитель обязан уметь
    /// работать без них. См. `parse::attach_entrances`.
    pub entrances: Vec<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadClass {
    Street,
    Alley,
}

/// Дорога: осевая полилиния и ширина по классу highway.
#[derive(Debug, Clone)]
pub struct RoadLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub class: RoadClass,
    /// `bridge=yes` — по такой дороге прорезается проходимый коридор через воду.
    pub bridge: bool,
    /// Арка: проезд/проход сквозь здание (`tunnel=building_passage`,
    /// `covered=building_passage|yes`). По такой дороге прорезается проходимый
    /// коридор сквозь уже заблокированное здание.
    pub passage: bool,
}

/// Состояние ж/д пути: действующий или заброшенный (рисуется тусклее).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    Active,
    Disused,
}

/// Ж/д путь: осевая полилиния и ширина по значению `railway`.
///
/// Навмеша не касается — слой чисто визуальный, люди ходят через пути как по
/// земле. Непрерывная линия, режущая город пополам, иначе отрезала бы половину
/// карты, и `prune_unreachable` её бы ампутировал.
#[derive(Debug, Clone)]
pub struct RailLine {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub kind: RailKind,
}

/// Стена (Кремль): полилиния фиксированной ширины, непроходима.
#[derive(Debug, Clone)]
pub struct WallLine {
    pub points: Vec<Vec2>,
    pub width: f32,
}

/// Аллея из OSM (`natural=tree_row`): осевая полилиния и то, что данные знают о
/// самой посадке. Деревья по ней расставляет `planting::plant_rows`.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub points: Vec<Vec2>,
    /// Шаг посадки из тегов, м (`spacing`, либо длина / (`count` − 1)).
    /// `None` — в данных шага нет, и он берётся из ползунка плотности.
    ///
    /// Теги эти в OSM редки и полустандартны, так что почти каждый ряд —
    /// `None`; ветка «шаг из данных» проверяется тестом, а не городом.
    pub spacing: Option<f32>,
    /// Радиус кроны из `diameter_crown`, м. `None` — разыгрывается, как в лесу.
    pub radius: Option<f32>,
}

/// Одиночное дерево из OSM (`node natural=tree`): позиция и радиус кроны из
/// `diameter_crown`. `None` — радиус разыгрывается, как в лесу. Сажает
/// `planting::plant_standalone`; ноды в лесу и у аллей там же и отсеиваются.
#[derive(Debug, Clone, Copy)]
pub struct TreeNode {
    pub pos: Vec2,
    pub radius: Option<f32>,
}

/// Посаженное дерево: центр, радиус кроны и плотность, на которой оно
/// появляется (см. [`MapData::tree_appears_at`]).
pub type PlantedTree = (Vec2, f32, f32);

/// Что делать с деревом аллеи, попавшим на занятое место.
///
/// Живёт здесь, а не в `planting`, потому что по нему собирается
/// [`MapData::trees`]: политика — часть состояния модели, а не только аргумент
/// посадки. Переключается на лету из панели Trees (`TreeStyle::row_placement`),
/// поэтому оба варианта считаются на загрузке разом (см. `planting::plant_rows`).
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TreeRowPlacement {
    /// Позиция из OSM как есть: выбрасываются только деревья в домах и в воде.
    ///
    /// По умолчанию именно так, потому что ширина дороги у нас **синтезирована**
    /// по классу highway (8–16 м) и о настоящих кромках ничего не знает.
    /// Аллея вдоль бульвара сплошь и рядом лежит внутри этой ширины, и полная
    /// проверка `blocked` стёрла бы ровно те ряды, ради которых всё и делалось.
    #[default]
    Keep,
    /// Занятое место — сдвиг вперёд по ряду до свободного; не нашлось на длине
    /// одного шага — дерева нет. Дороги и газоны при этом уважаются полностью.
    Slide,
}

impl TreeRowPlacement {
    pub const ALL: [Self; 2] = [Self::Keep, Self::Slide];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Slide => "slide",
        }
    }
}

/// Что решает, **где именно** встанут деревья аллеи: политика размещения и то,
/// слушаем ли мы шаг из тегов OSM. Обе оси меняют позиции, а не вид, поэтому
/// каждое сочетание раскладывается на загрузке и переключение в UI остаётся
/// пересборкой массива, а не пересадкой (см. [`RowTrees`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRowLayout {
    pub placement: TreeRowPlacement,
    /// `true` — шаг берётся из `spacing` / `count`, и ползунок плотности такой
    /// ряд не прореживает. `false` — теги игнорируются, ряд живёт по ползунку
    /// наравне с лесом.
    pub osm_spacing: bool,
}

impl Default for TreeRowLayout {
    fn default() -> Self {
        // по умолчанию данные важнее ползунка: если картограф проставил шаг,
        // он знает про эту аллею больше, чем наша формула
        Self {
            placement: TreeRowPlacement::default(),
            osm_spacing: true,
        }
    }
}

impl TreeRowLayout {
    pub const ALL: [Self; 4] = [
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Keep,
            osm_spacing: false,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: true,
        },
        Self {
            placement: TreeRowPlacement::Slide,
            osm_spacing: false,
        },
    ];
}

/// Аллеи, разложенные под каждое сочетание [`TreeRowLayout`]. Четыре варианта
/// вместо одного стоят копейки — рядов на карте сотни против десятков тысяч
/// лесных деревьев, — а взамен переключение любой из двух ручек не трогает
/// индексы близости, которые строятся по всем домам и дорогам карты.
#[derive(Debug, Default)]
pub struct RowTrees([Vec<PlantedTree>; 4]);

impl RowTrees {
    fn slot(layout: TreeRowLayout) -> usize {
        TreeRowLayout::ALL
            .iter()
            .position(|&known| known == layout)
            .expect("TreeRowLayout::ALL перечисляет все сочетания")
    }

    pub fn get(&self, layout: TreeRowLayout) -> &[PlantedTree] {
        &self.0[Self::slot(layout)]
    }

    pub fn set(&mut self, layout: TreeRowLayout, trees: Vec<PlantedTree>) {
        self.0[Self::slot(layout)] = trees;
    }
}

/// Распарсенная карта; остаётся ресурсом после спавна — для отладки.
#[derive(Resource, Debug, Default)]
pub struct MapData {
    pub buildings: Vec<PolyArea>,
    pub water: Vec<PolyArea>,
    pub parks: Vec<PolyArea>,
    /// Лесные массивы — обычно внутри парков; только здесь растут деревья.
    pub woods: Vec<PolyArea>,
    /// Луга/газоны — лежат поверх парков и не засаживаются деревьями.
    pub grass: Vec<PolyArea>,
    /// Песчаные пляжи — тоже поверх парков и без деревьев.
    pub sand: Vec<PolyArea>,
    pub roads: Vec<RoadLine>,
    /// Ж/д пути — только для отрисовки, в навмеш не попадают.
    pub rails: Vec<RailLine>,
    pub walls: Vec<WallLine>,
    /// Аллеи (`natural=tree_row`) — исходная геометрия, для отладки; деревья по
    /// ним уже разложены в [`MapData::row_trees_kept`] / [`MapData::row_trees_slid`].
    pub tree_rows: Vec<TreeRow>,
    /// Одиночные деревья (`node natural=tree`) — сырые ноды, для отладки;
    /// посаженные уже влиты в [`MapData::wood_trees`] с порогом 0.
    pub tree_nodes: Vec<TreeNode>,
    /// Деревья лесных полигонов по возрастанию порога появления, с одиночными
    /// деревьями из OSM впереди (их порог 0 — дерево из данных видно всегда).
    /// Сырьё для [`MapData::compose_trees`], а не то, что читает рендер.
    pub wood_trees: Vec<PlantedTree>,
    /// Деревья аллей под каждую раскладку, по возрастанию порога.
    pub row_trees: RowTrees,
    /// Под какую раскладку собран [`MapData::trees`]; `None` — ещё не собран.
    ///
    /// Признак живёт в `MapData`, а не в `Local` системы, именно потому, что при
    /// смене города ресурс заменяется целиком: `Local` пережил бы замену и
    /// решил, что для нового города всё уже собрано.
    pub composed_for: Option<TreeRowLayout>,
    /// Деревья карты: (центр, радиус). Детерминированы данными карты.
    /// Отсортированы по [`MapData::tree_appears_at`] — по возрастанию плотности,
    /// на которой дерево появляется. Собираются
    /// [`MapData::compose_trees`] из леса и аллей выбранной политики; всё
    /// остальное (рендер, тени, поле хвои, ползунок плотности) видит только этот
    /// массив и про аллеи не знает.
    pub trees: Vec<(Vec2, f32)>,
    /// На какой плотности (`TreeStyle::density`) появляется каждое дерево —
    /// массив **той же длины и того же порядка**, что [`MapData::trees`].
    ///
    /// Лес засаживается по потолку плотности, а ползунок показывает префикс:
    /// `trees[..partition_point(|d| d <= density)]`. Порог считается по номеру
    /// дерева внутри своего массива (`(номер + 1) · TREE_AREA_PER_TREE / площадь`),
    /// поэтому каждый лес отдаёт ровно свою долю, даже если он засаживался до
    /// упора и не добрал запрошенного (см. `planting.rs`).
    pub tree_appears_at: Vec<f32>,
}

impl MapData {
    /// Собрать [`MapData::trees`] из леса и аллей выбранной раскладки.
    ///
    /// Оба слагаемых уже отсортированы по порогу появления, так что это слияние,
    /// а не сортировка: префикс по плотности (`trees::visible_count`) обязан
    /// оставаться монотонным, иначе шаг ползунка вверх убирал бы деревья.
    pub fn compose_trees(&mut self, layout: TreeRowLayout) {
        // разбор по полям, а не `self.…`: аллеи читаются, пока выход пишется
        let MapData {
            row_trees,
            wood_trees,
            trees,
            tree_appears_at,
            ..
        } = self;
        let rows = row_trees.get(layout);

        trees.clear();
        tree_appears_at.clear();
        trees.reserve(wood_trees.len() + rows.len());
        tree_appears_at.reserve(wood_trees.len() + rows.len());

        let (mut wood, mut row) = (0, 0);
        while wood < wood_trees.len() || row < rows.len() {
            // при равных порогах первым идёт лес — порядок должен быть
            // детерминированным, иначе поле хвои и оттенки крон разъезжаются
            let take_wood = match (wood_trees.get(wood), rows.get(row)) {
                (Some(&(.., wood_at)), Some(&(.., row_at))) => wood_at <= row_at,
                (Some(_), None) => true,
                _ => false,
            };
            let &(position, radius, at) = if take_wood {
                wood += 1;
                &wood_trees[wood - 1]
            } else {
                row += 1;
                &rows[row - 1]
            };
            trees.push((position, radius));
            tree_appears_at.push(at);
        }

        self.composed_for = Some(layout);
    }
}

/// Точка внутри кольца (even-odd raycast). Кольцо открытое.
pub fn point_in_polygon(point: Vec2, ring: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Внутри внешнего кольца и вне всех дырок.
pub fn point_in_area(point: Vec2, area: &PolyArea) -> bool {
    point_in_polygon(point, &area.outer)
        && !area.holes.iter().any(|hole| point_in_polygon(point, hole))
}

/// Ближайшая точка отрезка. Проекция, зажатая концами: за пределами отрезка
/// ближайшая точка — его конец, а не точка на прямой.
pub fn closest_on_segment(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return from;
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    from + segment * t
}

/// Расстояние от точки до отрезка.
pub fn distance_to_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    point.distance(closest_on_segment(point, from, to))
}

/// Знаковая площадь кольца по формуле шнурования: положительная — обход
/// против часовой стрелки. Знак нужен тени (свипы силуэта обязаны быть
/// одинаково закручены) и генератору входов (от обхода зависит, куда смотрит
/// внешняя нормаль грани), поэтому базовая формула — знаковая, а абсолютная
/// [`ring_area`] получается из неё.
pub fn signed_ring_area(ring: &[Vec2]) -> f32 {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += ring[j].perp_dot(ring[i]);
        j = i;
    }
    doubled / 2.0
}

/// Площадь кольца, абсолютная.
pub fn ring_area(ring: &[Vec2]) -> f32 {
    signed_ring_area(ring).abs()
}

/// AABB кольца: (min, max).
pub fn ring_bounds(ring: &[Vec2]) -> (Vec2, Vec2) {
    let mut min = Vec2::INFINITY;
    let mut max = Vec2::NEG_INFINITY;
    for &point in ring {
        min = min.min(point);
        max = max.max(point);
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ]
    }

    #[test]
    fn point_in_polygon_square() {
        let ring = square();
        assert!(point_in_polygon(Vec2::new(5.0, 5.0), &ring));
        assert!(!point_in_polygon(Vec2::new(15.0, 5.0), &ring));
        assert!(!point_in_polygon(Vec2::new(-1.0, 5.0), &ring));
    }

    #[test]
    fn point_in_area_respects_holes() {
        let area = PolyArea {
            outer: square(),
            holes: vec![vec![
                Vec2::new(4.0, 4.0),
                Vec2::new(6.0, 4.0),
                Vec2::new(6.0, 6.0),
                Vec2::new(4.0, 6.0),
            ]],
            kind: AreaKind::Building,
            height: None,
            entrances: Vec::new(),
        };
        assert!(point_in_area(Vec2::new(2.0, 2.0), &area));
        assert!(!point_in_area(Vec2::new(5.0, 5.0), &area));
    }

    #[test]
    fn ring_area_square() {
        assert!((ring_area(&square()) - 100.0).abs() < 1e-3);
    }
}
