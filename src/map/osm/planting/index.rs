//! Индексы близости для посадки: «что уже занимает это место» — площадные
//! препятствия, отрезки дорог и стен, уже поставленные кроны. Всё через
//! [`NEARBY_CELL`]-сетку: полный скан по 7500 зданиям на каждую из десятков
//! тысяч попыток посадки квадратичен, и на Токио это минуты.
//!
//! Та же идиома, что у `entrances/index.rs`; политику посадки (сколько сажать,
//! какие зазоры держать) задаёт [`super`].

use std::collections::HashMap;

use bevy::math::{IVec2, Vec2};

use super::{
    TREE_CROWN_REACH, TREE_KERB_CLEARANCE, TREE_MAX_RADIUS, TREE_MIN_SPACING, TREE_SHORE_CLEARANCE,
    TREE_WALL_CLEARANCE,
};
use crate::map::osm::model::{MapData, PolyArea, distance_to_segment, point_in_area, ring_bounds};
use crate::map::roads::casing_width;

/// Сторона ячейки индексов близости, м. Того же порядка, что `FOOTPRINT_CELL`
/// (30) у генератора входов: в ячейке должно лежать несколько кандидатов, а не
/// полквартала и не одна стена.
const NEARBY_CELL: f32 = 32.0;
pub(super) fn nearby_cell(pos: Vec2) -> IVec2 {
    (pos / NEARBY_CELL).floor().as_ivec2()
}

/// Равномерная сетка «что может накрывать точку»: номера элементов, чей
/// **расширенный** AABB пересекает ячейку. Посадка перебирает десятки тысяч
/// точек, и линейный проход по семи тысячам домов и всем дорогам на каждую
/// попытку — почти всё время посадки. Идиома та же, что в
/// `entrances/index.rs`; сам AABB после выборки всё равно проверяется —
/// ячейка крупнее его.
pub(super) struct NearbyAreas(HashMap<IVec2, Vec<usize>>);

impl NearbyAreas {
    pub(super) fn build(bounds: &[(Vec2, Vec2)]) -> Self {
        let mut cells: HashMap<IVec2, Vec<usize>> = HashMap::new();
        for (index, &(min, max)) in bounds.iter().enumerate() {
            let (lo, hi) = (nearby_cell(min), nearby_cell(max));
            for x in lo.x..=hi.x {
                for y in lo.y..=hi.y {
                    cells.entry(IVec2::new(x, y)).or_default().push(index);
                }
            }
        }
        Self(cells)
    }

    fn near(&self, pos: Vec2) -> &[usize] {
        self.0.get(&nearby_cell(pos)).map_or(&[], Vec::as_slice)
    }
}

/// Та же сетка для линейных объектов, но по **отрезкам**, а не по объектам:
/// AABB реки или проспекта накрывает пол-карты, и индекс по объектам вернул бы
/// в кандидаты все её сотни отрезков. В ячейку кладётся
/// `(ширина ленты, начало, конец)` — ширина рядом с отрезком, а не номером в
/// исходном векторе, потому что индексов теперь два (дороги и русла) и
/// разыменовывать они бы стали разные вектора.
pub(super) struct NearbySegments(HashMap<IVec2, Vec<(f32, Vec2, Vec2)>>);

impl NearbySegments {
    /// `clearance` — наибольший зазор, который потребитель прибавит к
    /// полуширине; паддинг ячейки обязан его накрывать, иначе отрезок, до
    /// которого дереву не хватило зазора, не попадёт в кандидаты.
    fn build<'a>(lines: impl Iterator<Item = (&'a [Vec2], f32)>, clearance: f32) -> Self {
        let mut cells: HashMap<IVec2, Vec<(f32, Vec2, Vec2)>> = HashMap::new();
        for (points, width) in lines {
            let pad = width / 2.0 + TREE_MAX_RADIUS + clearance;
            for segment in points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                let lo = nearby_cell(from.min(to) - pad);
                let hi = nearby_cell(from.max(to) + pad);
                for x in lo.x..=hi.x {
                    for y in lo.y..=hi.y {
                        cells
                            .entry(IVec2::new(x, y))
                            .or_default()
                            .push((width, from, to));
                    }
                }
            }
        }
        Self(cells)
    }

    fn near(&self, pos: Vec2) -> &[(f32, Vec2, Vec2)] {
        self.0.get(&nearby_cell(pos)).map_or(&[], Vec::as_slice)
    }
}

/// Всё, что мешает дереву встать, вместе с индексами близости по этому всему.
///
/// Отдельной структурой, а не замыканием внутри посадки, потому что индексы
/// строятся по семи тысячам домов и всем дорогам карты, а посадок теперь три:
/// лес и по одной на каждую политику аллей ([`TreeRowPlacement`]). Строить одно
/// и то же трижды — почти всё время посадки.
pub(super) struct Obstacles<'a> {
    map: &'a MapData,
    /// Расширенные AABB зданий: паддинг по максимальной кроне, на конкретный
    /// радиус проверка ужимается уже в [`Obstacles::solid`].
    building_bounds: Vec<(Vec2, Vec2)>,
    water_bounds: Vec<(Vec2, Vec2)>,
    /// Луг и пляж деревьев не несут, но кроне свисать на них не запрещено.
    bare: Vec<&'a PolyArea>,
    bare_bounds: Vec<(Vec2, Vec2)>,
    building_index: NearbyAreas,
    road_index: NearbySegments,
    water_index: NearbyAreas,
    /// Линейные русла — тот же запрет, что у пруда, только по отрезку. Трубы
    /// сюда не попадают: над культвертом земля, и дерево на ней законно.
    water_line_index: NearbySegments,
    bare_index: NearbyAreas,
}

pub(super) fn in_bbox(pos: Vec2, min: Vec2, max: Vec2) -> bool {
    pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
}

impl<'a> Obstacles<'a> {
    pub(super) fn build(map: &'a MapData) -> Self {
        let building_bounds: Vec<(Vec2, Vec2)> = map
            .buildings
            .iter()
            .map(|building| {
                let (min, max) = ring_bounds(&building.outer);
                let pad = TREE_MAX_RADIUS * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
                (min - pad, max + pad)
            })
            .collect();
        let water_bounds: Vec<(Vec2, Vec2)> = map
            .water
            .iter()
            .map(|area| {
                let (min, max) = ring_bounds(&area.outer);
                (min - TREE_SHORE_CLEARANCE, max + TREE_SHORE_CLEARANCE)
            })
            .collect();
        let bare: Vec<&PolyArea> = map.grass.iter().chain(&map.sand).collect();
        let bare_bounds: Vec<(Vec2, Vec2)> =
            bare.iter().map(|area| ring_bounds(&area.outer)).collect();

        Self {
            building_index: NearbyAreas::build(&building_bounds),
            road_index: NearbySegments::build(
                map.roads
                    .iter()
                    .map(|road| (road.points.as_slice(), road.width)),
                TREE_KERB_CLEARANCE,
            ),
            water_index: NearbyAreas::build(&water_bounds),
            water_line_index: NearbySegments::build(
                map.water_lines
                    .iter()
                    .filter(|line| !line.tunnel)
                    .map(|line| (line.points.as_slice(), line.width)),
                TREE_SHORE_CLEARANCE,
            ),
            bare_index: NearbyAreas::build(&bare_bounds),
            map,
            building_bounds,
            water_bounds,
            bare,
            bare_bounds,
        }
    }

    /// Место, куда дерево не должно попасть **никогда**, откуда бы ни взялась
    /// его позиция: внутрь дома (или впритык к его стене) и в воду. Дерево на
    /// асфальте — спорная картинка, дерево из крыши или из пруда — баг рендера.
    pub(super) fn solid(&self, pos: Vec2, radius: f32) -> bool {
        // до стены — весь вылет кроны: крона, наползающая на дом, читается как
        // растущая из крыши, поэтому зазор меряется от края кроны
        let wall_gap = radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
        self.building_index.near(pos).iter().any(|&index| {
            let (min, max) = self.building_bounds[index];
            let building = &self.map.buildings[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, building) || near_area_edge(pos, building, wall_gap))
        }) || self.water_index.near(pos).iter().any(|&index| {
            let (min, max) = self.water_bounds[index];
            let area = &self.map.water[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, area) || near_area_edge(pos, area, TREE_SHORE_CLEARANCE))
        }) || self
            .water_line_index
            .near(pos)
            .iter()
            .any(|&(width, from, to)| {
                distance_to_segment(pos, from, to) <= width / 2.0 + TREE_SHORE_CLEARANCE
            })
    }

    /// Полная проверка: [`Obstacles::solid`] плюс дороги (парковые аллеи — тоже
    /// дороги) и голые полигоны — луга и песок. Ею отбирается лес, где позиция
    /// разыгрывается и промах ничего не стоит.
    pub(super) fn blocked(&self, pos: Vec2, radius: f32) -> bool {
        // до кромки дороги — только ствол с запасом: свисающая над дорожкой
        // ветка выглядит естественно, крона поверх стены — нет
        let kerb_gap = radius + TREE_KERB_CLEARANCE;
        self.solid(pos, radius)
            || self.road_index.near(pos).iter().any(|&(width, from, to)| {
                distance_to_segment(pos, from, to) <= width / 2.0 + kerb_gap
            })
            || self.bare_index.near(pos).iter().any(|&index| {
                let (min, max) = self.bare_bounds[index];
                in_bbox(pos, min, max) && point_in_area(pos, self.bare[index])
            })
    }

    /// Ствол стоит на полотне дороги или на её канте. Мягче дорожной части
    /// [`Obstacles::blocked`]: без зазора кроны — дерево у кромки с нависшей
    /// над полотном кроной легально, дерево, растущее из асфальта, нет.
    pub(super) fn on_road(&self, pos: Vec2) -> bool {
        self.road_index.near(pos).iter().any(|&(width, from, to)| {
            distance_to_segment(pos, from, to) <= width / 2.0 + casing_width(width)
        })
    }
}

/// Сетка занятых мест со стороной ячейки [`TREE_MIN_SPACING`]: единственная
/// проверка, растущая с числом посаженных деревьев, и линейным перебором она
/// делала посадку квадратичной (при потолке плотности деревьев уже десятки
/// тысяч). Результат не приблизительный: при такой стороне любое дерево ближе
/// минимума лежит в одной из девяти соседних ячеек.
#[derive(Clone, Default)]
pub(super) struct Occupied(HashMap<IVec2, Vec<Vec2>>);

impl Occupied {
    fn cell_of(pos: Vec2) -> IVec2 {
        (pos / TREE_MIN_SPACING).floor().as_ivec2()
    }

    pub(super) fn crowded(&self, pos: Vec2) -> bool {
        let spacing_sq = TREE_MIN_SPACING * TREE_MIN_SPACING;
        let cell = Self::cell_of(pos);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dy| {
                self.0
                    .get(&(cell + IVec2::new(dx, dy)))
                    .is_some_and(|others| {
                        others
                            .iter()
                            .any(|&other| pos.distance_squared(other) < spacing_sq)
                    })
            })
        })
    }

    pub(super) fn insert(&mut self, pos: Vec2) {
        self.0.entry(Self::cell_of(pos)).or_default().push(pos);
    }
}

/// Точка ближе `clearance` к любому ребру полигона — внешнему кольцу или
/// кольцу дырки (кольца замкнуты неявно, последнее ребро — от конца к началу).
pub(in crate::map::osm) fn near_area_edge(point: Vec2, area: &PolyArea, clearance: f32) -> bool {
    std::iter::once(&area.outer).chain(&area.holes).any(|ring| {
        (0..ring.len()).any(|index| {
            distance_to_segment(point, ring[index], ring[(index + 1) % ring.len()]) <= clearance
        })
    })
}
