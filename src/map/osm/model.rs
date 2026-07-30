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
    /// Деревья в парках: (центр, радиус). Детерминированы данными карты.
    /// Отсортированы по [`MapData::tree_appears_at`] — по возрастанию плотности,
    /// на которой дерево появляется.
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
