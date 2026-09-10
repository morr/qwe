//! Гаражные кооперативы: ГСК рисуется рядами боксов, а не одной коробкой.
//!
//! На снимке русского города ГСК ни с чем не спутать: сетка мелких боксов,
//! разрезанная проездами. В OSM он приезжает двумя разными способами, и до
//! сих пор оба рисовались неправильно:
//!
//! * **боксы по одному** (`building=garage`) — каждый со своим посевом, своим
//!   материалом кровли и своей фазой фактуры, так что ряд из двадцати боксов
//!   выходил конфетти из черепицы, битума и профлиста;
//! * **кооператив целиком** (`building=garages`) — одним контуром на всю
//!   территорию, и в Туле это пятна до 255 × 51 м, нарисованные как один
//!   гигантский ангар с зенитными фонарями.
//!
//! Здесь оба случая сводятся к одному: контуры сшиваются в **прогон**
//! ([`garage_runs`]), и весь прогон получает одну ось, один посев и одну
//! фазу. Дальше рисует шейдер ([`super::material::RoofKind::GarageRow`] и
//! `GarageBlock`): шов через [`BAY`] по всей ленте, а у кооператива ещё и
//! тёмные проезды через [`ROW_PITCH`]. Ни одной новой вершины — только другие
//! значения в том же `ATTRIBUTE_ROOF`.

use std::collections::HashMap;

use bevy::prelude::*;

use super::material::building_seed;
use super::roofs::min_area_rect;
use crate::map::osm::{BuildingUse, PolyArea};

/// Зазор, ближе которого два гаража считаются одним прогоном, м. Боксы обычно
/// стоят стена к стене, но контуры в OSM разведены на десятки сантиметров.
const JOIN_GAP: f32 = 2.0;
/// Лента короче этого прогоном не считается: гребёнка начинает читаться
/// примерно с четырёх боксов, а до того это просто пара сараев.
const ROW_MIN_LENGTH: f32 = 12.0;
/// И она обязана быть **лентой**, а не пятном: у квадратного пятна нет оси,
/// и поперечный шов на нём пошёл бы наугад.
const ROW_MIN_ASPECT: f32 = 2.2;
/// Пятно шириной от этого держит уже не ленту, а **ряды с проездом** между
/// ними — 12 м боксов спинами плюс проезд.
const BLOCK_MIN_WIDTH: f32 = 14.0;
/// И оно обязано быть кооперативом по площади, а не гаражом на две машины.
const BLOCK_MIN_AREA: f32 = 400.0;
/// Шаг бокса, м — **зеркало `BAY` в `assets/shaders/roof.wgsl`**. Ворота
/// гаража 2.5–3 м, бокс с простенком — 3.2–3.6.
pub(super) const BAY: f32 = 3.4;
/// Шаг рядов в кооперативе, м — **зеркало `ROW_PITCH` там же**: два ряда
/// боксов спинами (6 + 6) и проезд между парами (6).
const ROW_PITCH: f32 = 18.0;
/// Множители фазы в шейдере (`uv = vec2(dot(p, axis), dot(p, across)) + seed *
/// (U_SCALE, V_SCALE)`): чтобы решить, какая фаза кладёт первый шов на край
/// прогона, обе стороны обязаны знать эти числа.
const U_SCALE: f32 = 37.0;
const V_SCALE: f32 = 23.0;
/// Ячейка пространственного хеша при поиске соседей, м.
const CELL: f32 = 32.0;

/// Прогон гаражей глазами отрисовки: общая ось, общий посев и фаза, кладущая
/// первый шов ровно на край прогона.
pub(super) struct GarageRun {
    /// Вдоль прогона — длинная ось его общего прямоугольника.
    pub(super) axis: Vec2,
    /// Посев прогона (минимум по боксам, поэтому не зависит от их порядка) —
    /// им выбирается цвет.
    pub(super) seed: u32,
    /// Что уходит в `ATTRIBUTE_ROOF` как посев: фаза, при которой первый шов
    /// приходится на край прогона.
    pub(super) phase: f32,
    /// Кооператив целиком (`building=garages`, широкое пятно) — рисуется
    /// рядами с проездами; иначе одна лента боксов.
    pub(super) block: bool,
}

/// Прогоны по контурам: индекс здания → прогон, в который оно вошло. Здания
/// вне прогонов (одиночный гараж, сарай, пара боксов) в карту не попадают и
/// рисуются по-старому.
pub(super) fn garage_runs(buildings: &[PolyArea]) -> HashMap<usize, GarageRun> {
    let members: Vec<usize> = buildings
        .iter()
        .enumerate()
        .filter(|(_, building)| {
            matches!(
                building.building_use,
                BuildingUse::Garage | BuildingUse::GarageBlock
            )
        })
        .map(|(index, _)| index)
        .collect();
    let mut runs = HashMap::new();
    if members.is_empty() {
        return runs;
    }
    let boxes: Vec<Aabb> = members
        .iter()
        .map(|&at| aabb(&buildings[at].outer))
        .collect();

    // Пространственный хеш: гараж мал, поэтому раскладывается в одну-две
    // ячейки, и пары ищутся внутри ячейки, а не по всему городу. Раздутый
    // на зазор контур попадает в каждую задетую ячейку, поэтому два
    // достаточно близких бокса гарантированно встретятся хотя бы в одной.
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (slot, box_) in boxes.iter().enumerate() {
        let low = ((box_.min - JOIN_GAP) / CELL).floor();
        let high = ((box_.max + JOIN_GAP) / CELL).floor();
        for x in low.x as i32..=high.x as i32 {
            for y in low.y as i32..=high.y as i32 {
                cells.entry((x, y)).or_default().push(slot);
            }
        }
    }
    let mut union = Union::new(members.len());
    for slots in cells.values() {
        for (at, &a) in slots.iter().enumerate() {
            for &b in &slots[at + 1..] {
                if boxes[a].near(&boxes[b]) {
                    union.join(a, b);
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for slot in 0..members.len() {
        groups.entry(union.root(slot)).or_default().push(slot);
    }
    for group in groups.values() {
        let points: Vec<Vec2> = group
            .iter()
            .flat_map(|&slot| buildings[members[slot]].outer.iter().copied())
            .collect();
        // проезды рисуются только там, где картограф сказал «кооператив»:
        // на большом сарае они были бы выдумкой
        let cooperative = group
            .iter()
            .all(|&slot| buildings[members[slot]].building_use == BuildingUse::GarageBlock);
        let Some((axis, phase, block)) = run_of(&points, cooperative) else {
            continue;
        };
        let seed = group
            .iter()
            .map(|&slot| building_seed(&buildings[members[slot]]))
            .min()
            .unwrap_or(0);
        for &slot in group {
            runs.insert(
                members[slot],
                GarageRun {
                    axis,
                    seed,
                    phase,
                    block,
                },
            );
        }
    }
    runs
}

/// Ось прогона, фаза первого шва и «это кооператив» — либо `None`, если
/// пятно не тянет ни на ленту, ни на кооператив.
fn run_of(points: &[Vec2], cooperative: bool) -> Option<(Vec2, f32, bool)> {
    let rect = min_area_rect(points)?;
    // `min_area_rect` кладёт длинную сторону первой
    let axis = (rect[1] - rect[0]).try_normalize()?;
    let length = (rect[1] - rect[0]).length();
    let width = (rect[2] - rect[1]).length();

    // Широкое пятно кооператива — ряды с проездами; фаза кладёт край первого
    // ряда на край пятна, поэтому проезд не начинается посреди крайнего ряда
    if cooperative && width >= BLOCK_MIN_WIDTH && length * width >= BLOCK_MIN_AREA {
        let across = Vec2::new(-axis.y, axis.x);
        let edge = rect
            .iter()
            .map(|at| at.dot(across))
            .fold(f32::MAX, f32::min);
        return Some(((axis), (-edge).rem_euclid(ROW_PITCH) / V_SCALE, true));
    }
    if length < ROW_MIN_LENGTH || length < width * ROW_MIN_ASPECT {
        return None;
    }
    // у ленты фаза кладёт первый поперечный шов на её торец
    let edge = rect.iter().map(|at| at.dot(axis)).fold(f32::MAX, f32::min);
    Some((axis, (-edge).rem_euclid(BAY) / U_SCALE, false))
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec2,
    max: Vec2,
}

impl Aabb {
    /// Зазор между коробками не больше [`JOIN_GAP`] по обеим осям.
    fn near(&self, other: &Aabb) -> bool {
        self.min.x - JOIN_GAP <= other.max.x
            && other.min.x - JOIN_GAP <= self.max.x
            && self.min.y - JOIN_GAP <= other.max.y
            && other.min.y - JOIN_GAP <= self.max.y
    }
}

fn aabb(ring: &[Vec2]) -> Aabb {
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for point in ring {
        min = min.min(*point);
        max = max.max(*point);
    }
    Aabb { min, max }
}

/// Система непересекающихся множеств со сжатием путей — боксы сшиваются
/// попарно, а прогон нужен целиком.
struct Union {
    parent: Vec<usize>,
}

impl Union {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn root(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.parent[a] = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::AreaKind;

    fn garage_of(at: Vec2, size: Vec2, use_: BuildingUse) -> PolyArea {
        PolyArea {
            outer: vec![
                at,
                at + Vec2::new(size.x, 0.0),
                at + size,
                at + Vec2::new(0.0, size.y),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: use_,
            height: None,
            entrances: Vec::new(),
        }
    }

    fn garage(at: Vec2, size: Vec2) -> PolyArea {
        garage_of(at, size, BuildingUse::Garage)
    }

    /// Шесть боксов стена к стене — один прогон с осью вдоль ленты.
    #[test]
    fn adjacent_boxes_become_one_run() {
        let boxes: Vec<PolyArea> = (0..6)
            .map(|at| garage(Vec2::new(at as f32 * 3.3, 0.0), Vec2::new(3.2, 6.0)))
            .collect();
        let runs = garage_runs(&boxes);
        assert_eq!(runs.len(), 6);
        let first = &runs[&0];
        for at in 1..6 {
            assert_eq!(runs[&at].seed, first.seed);
            assert!((runs[&at].axis - first.axis).length() < 1e-5);
        }
        assert!(first.axis.x.abs() > 0.99, "{}", first.axis);
        assert!(!first.block);
    }

    /// Цельный `building=garages` лентой — тот же прогон, просто уже сшитый
    /// картографом; проездов на нём не рисуем, он их шириной не держит.
    #[test]
    fn a_single_long_outline_is_a_run_by_itself() {
        let runs = garage_runs(&[garage_of(
            Vec2::ZERO,
            Vec2::new(40.0, 6.0),
            BuildingUse::GarageBlock,
        )]);
        assert_eq!(runs.len(), 1);
        assert!(!runs[&0].block);
    }

    /// Пятно кооператива — ряды с проездами.
    #[test]
    fn a_wide_cooperative_outline_is_a_block() {
        let runs = garage_runs(&[garage_of(
            Vec2::ZERO,
            Vec2::new(120.0, 50.0),
            BuildingUse::GarageBlock,
        )]);
        assert!(runs[&0].block);
    }

    /// Такое же пятно, но это сарай: проезды поперёк сарая были бы выдумкой.
    #[test]
    fn a_big_shed_is_not_a_cooperative() {
        let runs = garage_runs(&[garage(Vec2::ZERO, Vec2::new(120.0, 50.0))]);
        assert!(runs.get(&0).is_none_or(|run| !run.block));
    }

    /// Одинокий гараж и пара сараев рядом рисуются по-старому.
    #[test]
    fn a_lone_garage_is_not_a_run() {
        let two = vec![
            garage(Vec2::ZERO, Vec2::new(3.2, 6.0)),
            garage(Vec2::new(3.3, 0.0), Vec2::new(3.2, 6.0)),
        ];
        assert!(garage_runs(&two).is_empty());
    }

    /// Два кооператива в разных концах города не сшиваются в один.
    #[test]
    fn distant_runs_stay_apart() {
        let mut boxes: Vec<PolyArea> = (0..6)
            .map(|at| garage(Vec2::new(at as f32 * 3.3, 0.0), Vec2::new(3.2, 6.0)))
            .collect();
        boxes.extend(
            (0..6).map(|at| garage(Vec2::new(at as f32 * 3.3, 400.0), Vec2::new(3.2, 6.0))),
        );
        let runs = garage_runs(&boxes);
        assert_eq!(runs.len(), 12);
        assert_ne!(runs[&0].seed, runs[&6].seed);
    }

    /// Фаза кладёт первый шов на торец ленты: `u` на её краю кратен боксу.
    #[test]
    fn the_first_seam_lands_on_the_end_of_the_run() {
        let start = Vec2::new(137.4, -58.1);
        let runs = garage_runs(&[garage(start, Vec2::new(40.0, 6.0))]);
        let run = &runs[&0];
        let u = start
            .dot(run.axis)
            .min((start + Vec2::new(40.0, 0.0)).dot(run.axis))
            + run.phase * U_SCALE;
        assert!((u / BAY - (u / BAY).round()).abs() < 1e-3, "u {u}");
    }

    /// А у кооператива — на край пятна, поперёк: проезд не начинается
    /// посреди крайнего ряда.
    #[test]
    fn the_first_aisle_lands_on_the_edge_of_a_block() {
        let start = Vec2::new(-311.7, 92.3);
        let runs = garage_runs(&[garage_of(
            start,
            Vec2::new(120.0, 50.0),
            BuildingUse::GarageBlock,
        )]);
        let run = &runs[&0];
        let across = Vec2::new(-run.axis.y, run.axis.x);
        let edge = [start, start + Vec2::new(0.0, 50.0)]
            .iter()
            .map(|at| at.dot(across))
            .fold(f32::MAX, f32::min);
        let v = edge + run.phase * V_SCALE;
        assert!(
            (v / ROW_PITCH - (v / ROW_PITCH).round()).abs() < 1e-3,
            "v {v}"
        );
    }
}
