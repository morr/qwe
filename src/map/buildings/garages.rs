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
//! ([`garage_runs`]), и весь прогон получает одну ось, один посев и **свою
//! сетку ячеек** — целое число боксов вдоль и рядов поперёк. Дальше рисует
//! шейдер ([`super::material::RoofKind::GarageRow`] и `GarageBlock`): шов на
//! каждой границе бокса, а у кооператива ещё и тёмный проезд на границе ряда.
//! Метры остаются здесь: в вершину едут ячейки, и шейдеру достаточно их
//! дробной части. Ни одной новой вершины — только другие значения в том же
//! `ATTRIBUTE_ROOF`.

use std::collections::HashMap;

use bevy::prelude::*;

use super::material::building_seed;
use super::roofs::min_area_rect;
use crate::map::osm::{BuildingUse, PolyArea};

/// Зазор, ближе которого два гаража считаются одним прогоном, м. Боксы обычно
/// стоят стена к стене, но контуры в OSM разведены на десятки сантиметров.
///
/// Зазор меряется **между контурами**, а не между их осевыми рамками, и это
/// не педантизм: лента ГСК идёт под произвольным углом, её осевая рамка втрое
/// шире её самой, и рамки соседних лент перекрываются насквозь через проезд.
/// На туламском ГСК у Косой Горы по рамкам в один прогон слипались десять
/// параллельных лент — прогон 350 × 72 м, чья ось расходилась с осями самих
/// лент на 7°, и швы вставали наискось к их стенам.
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
/// Шаг бокса, м. Ворота гаража 2.5–3 м, бокс с простенком — 3.2–3.6. Это
/// **цель, а не делитель**: сколько боксов встанет на ленту, решает
/// округление, поэтому на своей ленте бокс чуть шире или уже — зато их целое
/// число и на торцах нет огрызка.
pub(super) const BAY: f32 = 3.4;
/// Шаг рядов в кооперативе, м, той же природы: два ряда боксов спинами
/// (6 + 6) и проезд между парами (6).
const ROW_PITCH: f32 = 18.0;
/// Ячейка пространственного хеша при поиске соседей, м.
const CELL: f32 = 32.0;

/// Прогон гаражей глазами отрисовки: общая ось, общий посев и **своя сетка
/// ячеек** — целое число боксов вдоль прогона и целое число рядов поперёк.
///
/// Ячейки, а не метры, — то же лекарство, что у стены (`layers::wall_frame`):
/// сетка в метрах не знает, где лента кончается, и на торце остаётся
/// отрезанный бокс. Здесь шаг подогнан под саму ленту, поэтому шов приходится
/// ровно на оба её торца, а проезд — на край пятна.
#[derive(Clone, Copy, Debug)]
pub(super) struct GarageRun {
    /// Угол прогона, от которого считаются ячейки: минимум по обеим осям.
    pub(super) origin: Vec2,
    /// Вдоль прогона — длинная ось его общего прямоугольника.
    pub(super) axis: Vec2,
    /// Шаг бокса на **этой** ленте, м: длина, делённая на целое число боксов.
    pub(super) bay: f32,
    /// Шаг ряда поперёк, м: у кооператива ширина, делённая на целое число
    /// пар рядов; у ленты — вся её ширина, один ряд на ленту.
    pub(super) row: f32,
    /// Посев прогона (минимум по боксам, поэтому не зависит от их порядка) —
    /// им выбирается цвет и разнобой тона.
    pub(super) seed: u32,
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
                // рамка — только отсев: она у диагональной ленты втрое шире
                // самой ленты, и через проезд задевает соседнюю. Решает
                // расстояние между контурами
                if boxes[a].near(&boxes[b])
                    && rings_near(&buildings[members[a]].outer, &buildings[members[b]].outer)
                {
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
        let Some(shape) = run_of(&points, cooperative) else {
            continue;
        };
        let seed = group
            .iter()
            .map(|&slot| building_seed(&buildings[members[slot]]))
            .min()
            .unwrap_or(0);
        for &slot in group {
            runs.insert(members[slot], GarageRun { seed, ..shape });
        }
    }
    runs
}

/// Сетка прогона: угол отсчёта, ось, шаг бокса и ряда, «это кооператив» —
/// либо `None`, если пятно не тянет ни на ленту, ни на кооператив. Посев
/// проставляет вызывающий, он общий на группу.
fn run_of(points: &[Vec2], cooperative: bool) -> Option<GarageRun> {
    let rect = min_area_rect(points)?;
    // `min_area_rect` кладёт длинную сторону первой
    let axis = (rect[1] - rect[0]).try_normalize()?;
    let across = Vec2::new(-axis.y, axis.x);
    let length = (rect[1] - rect[0]).length();
    let width = (rect[2] - rect[1]).length();
    // угол прямоугольника: минимум по обеим осям, от него и считаются ячейки
    let low = |direction: Vec2| {
        rect.iter()
            .map(|at| at.dot(direction))
            .fold(f32::MAX, f32::min)
    };
    let origin = axis * low(axis) + across * low(across);
    // целое число боксов на длину: шаг подгоняется под ленту, а не лента под
    // шаг, и на торцах не остаётся огрызка
    let bay = length / (length / BAY).round().max(1.0);

    // Широкое пятно кооператива — ряды с проездами, и рядов тоже целое число:
    // крайний ряд не разрезан проездом посередине
    if cooperative && width >= BLOCK_MIN_WIDTH && length * width >= BLOCK_MIN_AREA {
        return Some(GarageRun {
            origin,
            axis,
            bay,
            row: width / (width / ROW_PITCH).round().max(1.0),
            seed: 0,
            block: true,
        });
    }
    if length < ROW_MIN_LENGTH || length < width * ROW_MIN_ASPECT {
        return None;
    }
    // у ленты ряд один на всю ширину: поперечной координате остаётся сказать
    // только «внутри», и проездов на ней не бывает
    Some(GarageRun {
        origin,
        axis,
        bay,
        row: width.max(1e-3),
        seed: 0,
        block: false,
    })
}

/// Расстояние между двумя кольцами не больше [`JOIN_GAP`]: пары рёбер, без
/// проверки на вложенность — гаражи друг в друга не вкладываются.
fn rings_near(a: &[Vec2], b: &[Vec2]) -> bool {
    a.iter().enumerate().any(|(at, &a0)| {
        let a1 = a[(at + 1) % a.len()];
        b.iter().enumerate().any(|(to, &b0)| {
            let b1 = b[(to + 1) % b.len()];
            segments_distance(a0, a1, b0, b1) <= JOIN_GAP
        })
    })
}

/// Расстояние между отрезками. Пересечение считать не нужно: контуры соседних
/// гаражей не пересекаются, а касание даёт ноль и так — через концы.
fn segments_distance(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f32 {
    point_to_segment(a0, b0, b1)
        .min(point_to_segment(a1, b0, b1))
        .min(point_to_segment(b0, a0, a1))
        .min(point_to_segment(b1, a0, a1))
}

fn point_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let edge = b - a;
    let length2 = edge.length_squared();
    if length2 < 1e-12 {
        return point.distance(a);
    }
    let at = ((point - a).dot(edge) / length2).clamp(0.0, 1.0);
    point.distance(a + edge * at)
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

    /// Координаты точки в ячейках прогона — то же, что кладёт в вершину
    /// `layers::garage_frame`.
    fn cell(run: &GarageRun, point: Vec2) -> Vec2 {
        let offset = point - run.origin;
        let across = Vec2::new(-run.axis.y, run.axis.x);
        Vec2::new(offset.dot(run.axis) / run.bay, offset.dot(across) / run.row)
    }

    fn whole(value: f32) -> bool {
        (value - value.round()).abs() < 1e-3
    }

    /// На ленте целое число боксов: оба её торца приходятся на шов, поэтому
    /// обрезанного бокса на конце не бывает. Шаг для этого подгоняется под
    /// ленту и лежит около [`BAY`], а не равен ему.
    #[test]
    fn a_run_holds_a_whole_number_of_bays() {
        let start = Vec2::new(137.4, -58.1);
        let runs = garage_runs(&[garage(start, Vec2::new(40.0, 6.0))]);
        let run = &runs[&0];
        assert!(whole(cell(run, start).x), "{}", cell(run, start).x);
        let far = cell(run, start + Vec2::new(40.0, 0.0)).x;
        assert!(whole(far), "{far}");
        assert_eq!(far.round(), 12.0, "40 м это 12 боксов по 3.33");
        assert!((run.bay - BAY).abs() < 0.4, "шаг {}", run.bay);
    }

    /// А у кооператива целое число рядов: крайний ряд не разрезан проездом
    /// посередине.
    #[test]
    fn a_block_holds_a_whole_number_of_rows() {
        let start = Vec2::new(-311.7, 92.3);
        let runs = garage_runs(&[garage_of(
            start,
            Vec2::new(120.0, 54.0),
            BuildingUse::GarageBlock,
        )]);
        let run = &runs[&0];
        assert!(run.block);
        assert!(whole(cell(run, start).y));
        let far = cell(run, start + Vec2::new(0.0, 54.0)).y;
        assert!(whole(far), "{far}");
        assert_eq!(far.round(), 3.0, "54 м это три пары рядов по 18");
    }

    /// Две параллельные ленты через проезд — **разные** прогоны, у каждой своя
    /// ось. Осевые рамки диагональной ленты перекрываются через проезд
    /// насквозь, и по ним они слипались в один прогон 350 × 72 м, чья ось
    /// расходилась с их собственными на 7°: швы вставали наискось к стенам, а
    /// пятно вдобавок проходило по ширине в кооператив и получало проезды
    /// поперёк всей пачки.
    #[test]
    fn parallel_ribbons_across_a_drive_stay_apart() {
        let along = Vec2::new(1.0, 1.0).normalize();
        let across = Vec2::new(-along.y, along.x);
        let ribbon = |offset: Vec2| PolyArea {
            outer: vec![
                offset,
                offset + along * 60.0,
                offset + along * 60.0 + across * 7.0,
                offset + across * 7.0,
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use: BuildingUse::GarageBlock,
            height: None,
            entrances: Vec::new(),
        };
        // вторая лента сдвинута поперёк на проезд и вдоль на половину длины —
        // так их осевые рамки и перекрываются
        let runs = garage_runs(&[ribbon(Vec2::ZERO), ribbon(across * 19.0 + along * 30.0)]);
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[&0].seed, runs[&1].seed);
        for run in runs.values() {
            assert!(!run.block, "лента шириной 7 м не кооператив");
            assert!(
                run.axis.perp_dot(along).abs() < 1e-3,
                "ось {} не вдоль ленты",
                run.axis
            );
        }
    }
}
