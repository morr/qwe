//! Геометрическая модель карты в мировых метрах (юго-западный угол — (0,0)).

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    Building,
    Kremlin,
    Water,
    Park,
}

/// Полигон с дырками. Кольца открытые: последняя точка не повторяет первую.
#[derive(Debug, Clone)]
pub struct PolyArea {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
    pub kind: AreaKind,
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
    pub roads: Vec<RoadLine>,
    pub walls: Vec<WallLine>,
    /// Деревья в парках: (центр, радиус). Детерминированы данными карты.
    pub trees: Vec<(Vec2, f32)>,
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

/// Расстояние от точки до отрезка.
pub fn distance_to_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return point.distance(from);
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(from + segment * t)
}

/// Площадь кольца по формуле шнурования, абсолютная.
pub fn ring_area(ring: &[Vec2]) -> f32 {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += (ring[j].x - ring[i].x) * (ring[j].y + ring[i].y);
        j = i;
    }
    (doubled / 2.0).abs()
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
        };
        assert!(point_in_area(Vec2::new(2.0, 2.0), &area));
        assert!(!point_in_area(Vec2::new(5.0, 5.0), &area));
    }

    #[test]
    fn ring_area_square() {
        assert!((ring_area(&square()) - 100.0).abs() < 1e-3);
    }
}
