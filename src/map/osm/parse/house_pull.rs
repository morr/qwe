//! Дома, наехавшие на нарисованный тротуар, отодвигаются от улицы — рядом,
//! а не по одному ([`pull_houses_off_sidewalks`]).

use std::collections::HashMap;

use bevy::math::Vec2;

use super::{Link, SIDEWALK_CELL, vertex_key, vertex_uses};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, BuildingUse, MapData, closest_on_segment, point_in_polygon, ring_bounds,
};

/// Зазор между стеной и внешним краем нарисованного тротуара, м. Треть метра
/// оставляла дом вплотную к тротуару, а в 2.5D крыша ещё и наклоняется к
/// дороге на полметра-метр.
pub(super) const SIDEWALK_CLEARANCE: f32 = 2.0;
/// Дальше этого, м, дом от улицы не отодвигается: дом, которому нужно больше,
/// сдвигается на этот предел и остаётся краем на тротуаре. 4 м оставляли на
/// месте целые ряды Тулы (улица Громова: дому 17 не хватило сантиметра).
pub(super) const SIDEWALK_SHIFT_MAX: f32 = 6.0;
/// Ближе этого, м, сдвинутый дом не подходит к другому объекту — зданию,
/// линии (от её края), водоёму, цилиндру, — если до сдвига стоял дальше.
const SHIFT_CLEARANCE: f32 = 0.5;
/// Доли сдвига по очереди: полный упёрся — дом сдвигается на меньший.
const SHIFT_FRACTIONS: [f32; 4] = [1.0, 0.75, 0.5, 0.25];
/// Разница отступов от улицы, м, при которой соседи ещё стоят «на одной линии»
/// и сдвигаются вместе.
const ROW_SETBACK_TOLERANCE: f32 = 2.0;
/// Промежуток между габаритами соседей по ряду, м.
const ROW_GAP: f32 = 20.0;
/// Раундов сдвига: угловой дом у двух улиц отталкивается от каждой по очереди.
const SIDEWALK_SHIFT_ROUNDS: usize = 4;
/// Остаток наезда, м, который после раундов считается нулём.
const SIDEWALK_SHIFT_TOLERANCE: f32 = 0.05;

/// Дома, контур которых **наезжает на нарисованный тротуар** улицы, сдвигаются
/// от неё целиком — внутрь квартала. Возвращает, сколько домов сдвинуто и
/// сколько наезжающих оставлено как есть.
///
/// Ширина улицы в модели — константа класса, а тротуар добавляет рендер
/// (полоса по классу, [`crate::map::osm::model::RoadLine::sidewalk`]), и в старой застройке дом, стоящий в OSM у самой
/// кромки, выходит стеной на тротуар, а в 2.5D крышей — на асфальт (Тула,
/// way 179102449 у улицы Бундурина: стена в 4.7 м от оси при 5.76 м полосы).
/// Точность до метра игре не нужна, а дом на тротуаре читается как баг.
///
/// Сдвиг — перенос всего контура вместе с входами, по кратчайшему направлению
/// от оси улицы. **Двигается ряд, а не дом**: соседи по одной стороне одной
/// улицы (габариты ближе [`ROW_GAP`]), чей отступ не больше чем на
/// [`ROW_SETBACK_TOLERANCE`] отличается от отступа самого наезжающего дома
/// ряда, отодвигаются на тот же сдвиг — иначе линия фасадов ломается
/// ступенькой. Дом, которому нужно больше [`SIDEWALK_SHIFT_MAX`], в ряд не
/// встаёт — иначе он оставил бы на месте всех соседей — и сдвигается на
/// предел. Потом каждый дом добирает остаток наезда от других улиц раундами до
/// [`SIDEWALK_SHIFT_ROUNDS`], не дальше того же предела. Сдвиг, после
/// которого дом подходит к другому объекту ближе [`SHIFT_CLEARANCE`]
/// ([`Obstacles`]), укорачивается по [`SHIFT_FRACTIONS`]; не помогла и
/// четверть — дом остаётся. Дом остаётся на месте и тогда, когда улица
/// проходит сквозь контур. Не трогаются, как и в [`super::square_skewed_houses`],
/// дома с общей вершиной — сплошная застройка разошлась бы щелью, арка
/// потеряла бы проход, — крепость и храмы (части храма стоят друг на друге).
/// Сдвиг первой вершины меняет дому посев: материал и этажность выпадут заново.
///
/// Место в конвейере — **шаг 5** [`super::finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
pub(super) fn pull_houses_off_sidewalks(map: &mut MapData) -> PulledHouses {
    // `reach` — полуширина улицы с тротуаром и зазором; рядом индекс улицы
    let mut segments: Vec<Link> = Vec::new();
    let mut segment_road: Vec<usize> = Vec::new();
    for (index, road) in map.roads.iter().enumerate() {
        if road.bridge || !road.is_carriageway() {
            continue;
        }
        // край обочины: полоса по классу, тег не смотрит — там, где тег снял
        // тротуар, дом всё равно не встаёт вплотную к бордюру
        let Some(reach) = road
            .sidewalk()
            .verge_edge(road.width / 2.0, SIDEWALK_CLEARANCE)
        else {
            continue;
        };
        for link in road.points.windows(2) {
            segments.push(Link {
                from: link[0],
                to: link[1],
                reach,
            });
            segment_road.push(index);
        }
    }
    let widest = segments.iter().map(|link| link.reach).fold(0.0, f32::max);
    let mut lines: Grid<usize> = Grid::new(SIDEWALK_CELL);
    for (index, link) in segments.iter().enumerate() {
        // радиус здесь знает запрос (`widest` ниже), а не звено
        lines.insert_segment(link.from, link.to, 0.0, index);
    }

    let uses = vertex_uses(map);
    let obstacles = Obstacles::new(map);
    let mut fronts: Vec<Front> = Vec::new();
    let mut left = 0;
    for (index, building) in map.buildings.iter().enumerate() {
        if building.kind != AreaKind::Building
            || matches!(building.building_use, BuildingUse::Church(_))
            || building
                .outer
                .iter()
                .any(|vertex| uses[&vertex_key(*vertex)] > 1)
        {
            continue;
        }
        let (min, max) = ring_bounds(&building.outer);
        let margin = Vec2::splat(widest + SIDEWALK_SHIFT_MAX);
        let nearby = lines.near(min - margin, max + margin);
        if nearby.is_empty() {
            continue;
        }
        match Front::of(index, &building.outer, &nearby, &segments, &segment_road) {
            // улица сквозь дом: сдвигать некуда, и в ряд он не встаёт
            None => left += 1,
            Some(front) if front.need > -ROW_SETBACK_TOLERANCE => fronts.push(front),
            Some(_) => {}
        }
    }

    // ряды: соседи по одной стороне одной улицы, union-find
    let mut parent: Vec<usize> = (0..fronts.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut by_street: HashMap<(usize, bool), Vec<usize>> = HashMap::new();
    for (index, front) in fronts.iter().enumerate() {
        // дом, которому мало предела, — ряд из одного себя
        if front.need <= SIDEWALK_SHIFT_MAX {
            by_street
                .entry((front.road, front.left))
                .or_default()
                .push(index);
        }
    }
    for members in by_street.values() {
        for (position, &a) in members.iter().enumerate() {
            for &b in &members[position + 1..] {
                let (first, second) = (&fronts[a], &fronts[b]);
                let gap = (first.min - second.max)
                    .max(second.min - first.max)
                    .max(Vec2::ZERO)
                    .length();
                if gap <= ROW_GAP {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
    }
    // сдвиг ряда — наезд самого глубокого дома в нём
    let mut row_need: HashMap<usize, f32> = HashMap::new();
    for (index, front) in fronts.iter().enumerate() {
        let row = root(&mut parent, index);
        let need = row_need.entry(row).or_insert(f32::NEG_INFINITY);
        *need = need.max(front.need);
    }

    let mut moved = 0;
    let mut partly = 0;
    for (index, front) in fronts.iter().enumerate() {
        let need = row_need[&root(&mut parent, index)];
        if need <= SIDEWALK_SHIFT_TOLERANCE || front.need < need - ROW_SETBACK_TOLERANCE {
            continue;
        }
        let intruding = front.need > SIDEWALK_SHIFT_TOLERANCE;
        let building = &map.buildings[front.building];
        let nearby = local(&front.nearby, &segments);
        // сдвиг ряда, потом остаток наезда от других улиц — всё в пределе
        let mut shift = front.away * need.min(SIDEWALK_SHIFT_MAX);
        let Some(mut push) = sidewalk_push(&building.outer, shift, &nearby) else {
            left += usize::from(intruding);
            continue;
        };
        for _ in 0..SIDEWALK_SHIFT_ROUNDS {
            if push.length() <= SIDEWALK_SHIFT_TOLERANCE {
                break;
            }
            let next = (shift + push).clamp_length_max(SIDEWALK_SHIFT_MAX);
            // ось улицы вошла в контур или предел не пускает дальше
            let Some(next_push) = sidewalk_push(&building.outer, next, &nearby) else {
                break;
            };
            if next.distance(shift) <= SIDEWALK_SHIFT_TOLERANCE {
                break;
            }
            (shift, push) = (next, next_push);
        }
        let Some(shift) = SHIFT_FRACTIONS
            .iter()
            .map(|fraction| shift * fraction)
            .find(|&shift| !obstacles.blocks(map, front.building, shift))
        else {
            left += usize::from(intruding);
            continue;
        };
        let remains = sidewalk_push(&building.outer, shift, &nearby)
            .is_none_or(|push| push.length() > SIDEWALK_SHIFT_TOLERANCE);
        partly += usize::from(intruding && remains);
        let building = &mut map.buildings[front.building];
        building
            .outer
            .iter_mut()
            .for_each(|vertex| *vertex += shift);
        building
            .holes
            .iter_mut()
            .flatten()
            .for_each(|vertex| *vertex += shift);
        building
            .entrances
            .iter_mut()
            .for_each(|entrance| *entrance += shift);
        moved += 1;
    }
    PulledHouses {
        moved,
        partly,
        left,
    }
}

/// Итог [`pull_houses_off_sidewalks`].
#[derive(Default)]
pub(super) struct PulledHouses {
    /// Сдвинуто домов, соседи по ряду включительно.
    pub(super) moved: usize,
    /// Из них наезжавших, что после сдвига ещё задевают тротуар: упёрлись в
    /// предел или в соседний объект.
    pub(super) partly: usize,
    /// Наезжающих, оставленных на месте.
    pub(super) left: usize,
}

/// Во что сдвигаемый дом не должен упереться: другие здания и отрезки всего
/// прочего — дорог и рельсов (от края полотна), стен, заборов, труб, открытых
/// водотоков, берегов водоёмов, цилиндров (отрезок нулевой длины радиусом).
///
/// Правило относительное: мешает только объект, к которому дом **подошёл**
/// ближе [`SHIFT_CLEARANCE`], — стоявший вплотную по данным (тропа у стены,
/// забор по участку) сдвиг от себя не запрещает.
struct Obstacles {
    /// Контуры зданий до сдвигов: «было» меряется по ним, «стало» — по
    /// текущей карте, где соседи могли уже сдвинуться.
    original: Vec<Vec<Vec2>>,
    /// Здания по ячейкам — габарит, раздутый на [`SIDEWALK_SHIFT_MAX`], так
    /// что сдвинутое здание не выходит из своих ячеек.
    buildings: Grid<usize>,
    /// Звенья всего прочего; `reach` — радиус запрета, полуширина плюс зазор.
    segments: Vec<Link>,
    lines: Grid<usize>,
}

impl Obstacles {
    fn new(map: &MapData) -> Self {
        let original: Vec<Vec<Vec2>> = map
            .buildings
            .iter()
            .map(|building| building.outer.clone())
            .collect();
        let mut buildings = Grid::new(SIDEWALK_CELL);
        let grow = Vec2::splat(SIDEWALK_SHIFT_MAX + SHIFT_CLEARANCE);
        for (index, ring) in original.iter().enumerate() {
            let (min, max) = ring_bounds(ring);
            buildings.insert(min - grow, max + grow, index);
        }

        let mut segments = Vec::new();
        let mut add = |points: &[Vec2], half_width: f32| {
            let reach = half_width + SHIFT_CLEARANCE;
            points.windows(2).for_each(|link| {
                segments.push(Link {
                    from: link[0],
                    to: link[1],
                    reach,
                })
            });
        };
        map.roads
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.rails
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.walls
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.fences.iter().for_each(|line| add(&line.points, 0.0));
        map.pipes
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.water_lines
            .iter()
            .filter(|line| !line.tunnel)
            .for_each(|line| add(&line.points, line.width / 2.0));
        for area in &map.water {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                let closed: Vec<Vec2> = ring.iter().chain(ring.first()).copied().collect();
                add(&closed, 0.0);
            }
        }
        for structure in &map.structures {
            add(&[structure.at, structure.at], structure.radius);
        }
        let mut lines = Grid::new(SIDEWALK_CELL);
        for (index, link) in segments.iter().enumerate() {
            lines.insert_segment(link.from, link.to, link.reach, index);
        }
        Self {
            original,
            buildings,
            segments,
            lines,
        }
    }

    /// Упрётся ли здание `house`, перенесённое из исходного места на `shift`.
    fn blocks(&self, map: &MapData, house: usize, shift: Vec2) -> bool {
        let before = &self.original[house];
        let after: Vec<Vec2> = before.iter().map(|vertex| *vertex + shift).collect();
        let (min, max) = ring_bounds(&after);
        // стало ближе запрета и ближе, чем было
        let closer = |now: f32, reach: f32, was: f32| now < reach && now < was - 0.01;

        let buildings = self.buildings.near(min, max);
        let neighbours = buildings.iter().filter(|&&other| other != house);
        for &other in neighbours {
            let current = &map.buildings[other].outer;
            let (other_min, other_max) = ring_bounds(current);
            let apart = (min - other_max).max(other_min - max).max(Vec2::ZERO);
            if apart.length() >= SHIFT_CLEARANCE {
                continue;
            }
            let now = ring_distance(&after, current);
            if closer(
                now,
                SHIFT_CLEARANCE,
                ring_distance(before, &self.original[other]),
            ) {
                return true;
            }
        }
        self.lines.near(min, max).into_iter().any(|index| {
            let Link { from, to, reach } = self.segments[index];
            let now = ring_segment_distance(&after, from, to);
            now < reach && closer(now, reach, ring_segment_distance(before, from, to))
        })
    }
}

/// Расстояние от контура до отрезка: ноль, если отрезок пересекает контур или
/// лежит внутри.
fn ring_segment_distance(ring: &[Vec2], from: Vec2, to: Vec2) -> f32 {
    if point_in_polygon(from, ring) {
        return 0.0;
    }
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            closest_between_segments(a, b, from, to)
                .map_or(0.0, |(on_ring, on_segment)| on_ring.distance(on_segment))
        })
        .fold(f32::INFINITY, f32::min)
}

/// Расстояние между двумя контурами: ноль, если они пересекаются или один
/// внутри другого.
fn ring_distance(a: &[Vec2], b: &[Vec2]) -> f32 {
    if a.first().is_some_and(|vertex| point_in_polygon(*vertex, b)) {
        return 0.0;
    }
    (0..b.len())
        .map(|index| ring_segment_distance(a, b[index], b[(index + 1) % b.len()]))
        .fold(f32::INFINITY, f32::min)
}

/// Отрезки улиц по индексам.
fn local(indices: &[usize], segments: &[Link]) -> Vec<Link> {
    indices.iter().map(|&index| segments[index]).collect()
}

/// Дом, выходящий фасадом на улицу: ближайшая к контуру ось.
struct Front {
    building: usize,
    /// Индексы отрезков улиц рядом с домом.
    nearby: Vec<usize>,
    road: usize,
    /// С какой стороны оси улицы стоит дом (по порядку её точек).
    left: bool,
    /// Единичное направление от оси к стене.
    away: Vec2,
    /// Насколько стена заходит в полосу улицы, м; отрицательное — запас.
    need: f32,
    min: Vec2,
    max: Vec2,
}

impl Front {
    /// `None` — ось улицы проходит сквозь контур или кончается внутри него.
    fn of(
        building: usize,
        ring: &[Vec2],
        nearby: &[usize],
        segments: &[Link],
        segment_road: &[usize],
    ) -> Option<Self> {
        let mut best: Option<(f32, usize, Vec2, Vec2)> = None;
        for &segment in nearby {
            let Link { from, to, reach } = segments[segment];
            if point_in_polygon(from, ring) || point_in_polygon(to, ring) {
                return None;
            }
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                let (on_wall, on_axis) = closest_between_segments(a, b, from, to)?;
                let need = reach - on_wall.distance(on_axis);
                if best.is_none_or(|(deepest, ..)| need > deepest) {
                    best = Some((need, segment, on_wall, on_axis));
                }
            }
        }
        let (need, segment, on_wall, on_axis) = best?;
        let Link { from, to, .. } = segments[segment];
        let (min, max) = ring_bounds(ring);
        Some(Self {
            building,
            nearby: nearby.to_vec(),
            road: segment_road[segment],
            left: (to - from).perp_dot(on_wall - on_axis) > 0.0,
            away: (on_wall - on_axis).normalize_or_zero(),
            need,
            min,
            max,
        })
    }
}

/// Сдвиг, выводящий контур `ring`, перенесённый на `shift`, из самой глубоко
/// задетой полосы улиц `segments`: `Some(ZERO)` — не наезжает, `None` — ось
/// улицы проходит сквозь контур или кончается внутри него, и сдвигать некуда.
fn sidewalk_push(ring: &[Vec2], shift: Vec2, segments: &[Link]) -> Option<Vec2> {
    let inside = |point: Vec2| point_in_polygon(point - shift, ring);
    if segments
        .iter()
        .any(|link| inside(link.from) || inside(link.to))
    {
        return None;
    }
    let mut push = Vec2::ZERO;
    for index in 0..ring.len() {
        let (a, b) = (ring[index] + shift, ring[(index + 1) % ring.len()] + shift);
        for &Link { from, to, reach } in segments {
            let (on_wall, on_axis) = closest_between_segments(a, b, from, to)?;
            let away = on_wall - on_axis;
            let depth = reach - away.length();
            if depth > push.length() {
                push = away.normalize_or_zero() * depth;
            }
        }
    }
    Some(push)
}

/// Ближайшие точки двух отрезков `(на первом, на втором)`; `None` — отрезки
/// пересекаются.
pub(super) fn closest_between_segments(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<(Vec2, Vec2)> {
    let (ab, cd, ac) = (b - a, d - c, c - a);
    let denominator = ab.perp_dot(cd);
    if denominator != 0.0 {
        let t = ac.perp_dot(cd) / denominator;
        let u = ac.perp_dot(ab) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return None;
        }
    }
    [
        (a, closest_on_segment(a, c, d)),
        (b, closest_on_segment(b, c, d)),
        (closest_on_segment(c, a, b), c),
        (closest_on_segment(d, a, b), d),
    ]
    .into_iter()
    .min_by(|x, y| {
        x.0.distance_squared(x.1)
            .total_cmp(&y.0.distance_squared(y.1))
    })
}
