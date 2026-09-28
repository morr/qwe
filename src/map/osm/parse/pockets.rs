//! **Карманы земли** (ground pockets) между кварталом и дорожками — травой
//! двора.
//!
//! Квартал (`landuse`) в OSM не всегда доходит до дорожек вокруг: у поворота
//! тротуара к переходу, между диагональной дорожкой и дорожкой вдоль улицы,
//! за концом обочины ([`RoadLine::verges`]) остаётся треугольник, куда не
//! ложится ничего, — и он светится голой землёй, обрамлённый плиткой и
//! двором с трёх сторон (Тула, витрина 02: квартал 164045103 не доходит до
//! дорожки десяти метров; Орёл, 03: полоса у квартала 141157692 и клин у
//! обочины). Сдвиг вершин ([`super::pull_areas_to_roads`]) такой клин не
//! закрывает: у треугольника между двумя дорожками вершины, которую надо
//! сдвинуть, нет вовсе — угол клина стоит на перекрёстке дорожек.
//!
//! Здесь недостаток данных закрывается **одним вопросом к картине целиком**:
//! что из земли **замкнуто** нарисованным — полотнами дорог с тротуарами,
//! обочинами, кварталами, зеленью, водой, стоянками, площадками. Дырка их
//! объединения — это и есть карман: вокруг него всё покрыто, и ни один тег
//! его не описал. Карман, который касается квартала и дороги и мал
//! ([`POCKET_AREA_MAX`]), засевается травой того же квартала
//! (`MapData::pockets`, рисуется слоем кварталов).
//!
//! Что должно остаться землёй, остаётся ею: карман у грунтовой тропы — не
//! двор (пустырь с тропинками, как юго-запад Орла, 03), большой — это уже
//! площадка сама по себе (пустырь, стройка), а у дырки в самом квартале
//! (мультиполигон вырезал её нарочно) нет дороги на краю.
//!
//! Ошибиться в «покрытом» не страшно в одну сторону: квартал лежит **ниже**
//! всего, что на нём нарисовано (`Z_LANDUSE`), так что трава, подложенная
//! под асфальт или под плитку, не видна. Поэтому покрытие берётся с запасом
//! там, где рисунок сложнее разбора (тротуар — с обеих сторон, если он есть
//! хоть с одной), а дома в него не входят вовсе: карман, накрывший дом, —
//! трава под домом.

use bevy::math::{IVec2, Vec2};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};

use super::LANDUSE_OVERLAP;
use crate::map::along::{arclengths, densify};
use crate::map::grid::Grid;
use crate::map::meshing::miter_offsets;
use crate::map::osm::model::{
    BuildingUse, MapData, PolyArea, RoadClass, RoadLine, distance_to_segment, ring_area,
    ring_bounds,
};
use crate::map::shapes::{Contour, Shape, area_contours, contour_bounds, oriented, ring_of};

/// Больше этого, м², дырка покрытия — уже не карман, а площадка: пустырь,
/// стройка, двор без тега. Клин Тулы 02 — около 150 м², параллелограмм у
/// кольца Калуги 01 — 8 × 20 м.
const POCKET_AREA_MAX: f32 = 400.0;
/// Меньше этого, м², — шум обводки, а не земля.
const POCKET_AREA_MIN: f32 = 0.05;
/// Как близко к кольцу квартала, м, должна подходить хоть одна вершина
/// кармана: край квартала сплошь и рядом лежит **под** дорожкой, и клин за
/// ней касается уже её полотна, а не квартала. Ширина дорожки с запасом.
const POCKET_NEAR: f32 = 4.0;
/// Насколько близко к краю полотна вершина кармана, м, чтобы он считался
/// касающимся дороги.
const TOUCH: f32 = 0.25;
/// Сторона плитки, по которой считается объединение, м: весь город одной
/// булевой операцией — секунды, плитками по потокам — десятки миллисекунд.
const TILE: f32 = 400.0;
/// Запас окна плитки, м: карман принимается, только если весь лежит в окне, —
/// тогда в окне есть всё покрытие, которое его касается. Карман серединой
/// у края плитки вылезает за него на полдлины, и клин в
/// [`POCKET_AREA_MAX`] длиннее шестидесяти метров почти не бывает.
const MARGIN: f32 = 30.0;
/// Звеньев дороги в одном куске обводки: кусок берётся в плитку по габариту,
/// и длинная улица не тащит в каждую свои сотни метров.
const RUN: usize = 16;
/// Шаг точек обочины, м: вдвое реже рисунка (`roads.rs`, 2.5 м) — профиль
/// обочины ([`RoadLine::verge_profile`]) и так снят пробами через пять
/// метров, а точки — цена объединения.
const VERGE_STEP: f32 = 5.0;
/// Точек обочины в одном куске.
const VERGE_RUN: usize = 16;

/// Кусок покрытия: контуры `i_overlay` и габарит.
struct Cover {
    contours: Vec<Contour>,
    low: Vec2,
    high: Vec2,
}

/// Звено дороги для вопроса «касается ли карман полотна».
#[derive(Clone, Copy)]
struct Link {
    from: Vec2,
    to: Vec2,
    reach: f32,
    /// Грунтовая тропа или улица: карман у неё — не двор.
    rough: bool,
}

/// Всё, что нужно проходу, собранное один раз на карту.
struct Scene {
    covers: Vec<Cover>,
    cover_grid: Grid<usize>,
    links: Vec<Link>,
    link_grid: Grid<usize>,
    /// Рёбра колец кварталов: `(от, до, индекс квартала)`.
    edges: Vec<(Vec2, Vec2, usize)>,
    edge_grid: Grid<usize>,
}

/// Карман, найденный в плитке: кольцо, уже заведённое под соседей, и
/// квартал, чью траву он берёт.
struct Pocket {
    outer: Vec<Vec2>,
    block: usize,
}

/// Засеять карманы земли у кварталов травой их двора: каждый — площадью
/// того же вида, что ближний квартал, в `MapData::pockets` (не в `landuse`:
/// карман — не квартал, и обочина рядом с ним двор не спрашивает).
/// Возвращает, сколько.
///
/// Плитки друг от друга не зависят и считаются по потокам, как стоянки
/// (`lots::pave_lots`); порядок результата — порядок плиток, так что он не
/// зависит от числа потоков.
pub(super) fn fill_ground_pockets(map: &mut MapData) -> usize {
    if map.landuse.is_empty() {
        return 0;
    }
    let scene = Scene::of(map);
    let tiles = tiles_of(&map.landuse);
    let pockets: Vec<Pocket> = in_parallel(&tiles, |&tile| scene.pockets(tile))
        .into_iter()
        .flatten()
        .collect();
    drop(scene);
    let sown = pockets.len();
    for Pocket { outer, block } in pockets {
        let kind = map.landuse[block].kind;
        map.pockets.push(PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            building_use: BuildingUse::Other,
            height: None,
            storeys: None,
            entrances: Vec::new(),
            colours: Default::default(),
        });
    }
    sown
}

/// `work` над каждым из `items` по потокам — результаты в порядке `items`,
/// так что от числа потоков и их гонки ничего не зависит. Задания разбираются
/// по одному со счётчика, а не кусками поровну: плитка центра дороже плитки
/// окраины в десятки раз, и поток с кучей центральных плиток держал бы всех.
fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.min(items.len()))
            .map(|_| {
                let (next, work) = (&next, &work);
                scope.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            break done;
                        };
                        done.push((index, work(item)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("поток прохода карманов упал"))
            .collect()
    });
    done.sort_by_key(|&(index, _)| index);
    done.into_iter().map(|(_, result)| result).collect()
}

/// Плитки, которых касается хоть один квартал (с запасом [`POCKET_NEAR`]), —
/// по возрастанию: карман без квартала рядом не засевается, и считать там
/// нечего.
fn tiles_of(blocks: &[PolyArea]) -> Vec<IVec2> {
    let cell = |point: Vec2| (point / TILE).floor().as_ivec2();
    let mut tiles: Vec<IVec2> = Vec::new();
    for block in blocks {
        let (low, high) = ring_bounds(&block.outer);
        let (low, high) = (cell(low - POCKET_NEAR), cell(high + POCKET_NEAR));
        for x in low.x..=high.x {
            for y in low.y..=high.y {
                tiles.push(IVec2::new(x, y));
            }
        }
    }
    tiles.sort_by_key(|tile| (tile.x, tile.y));
    tiles.dedup();
    tiles
}

impl Scene {
    fn of(map: &MapData) -> Self {
        // мост висит над землёй, арка идёт сквозь дом
        let roads: Vec<&RoadLine> = map
            .roads
            .iter()
            .filter(|road| !road.bridge && !road.passage && road.points.len() >= 2)
            .collect();
        let mut covers: Vec<Cover> = in_parallel(&roads, |road| road_covers(road))
            .into_iter()
            .flatten()
            .collect();
        let mut links: Vec<Link> = Vec::new();
        for road in roads {
            let reach = road.sidewalk().mapped_edge(road.width / 2.0);
            let rough = road.is_unpaved_street()
                || (road.class == RoadClass::Alley && !road.is_paved_path());
            for pair in road.points.windows(2) {
                links.push(Link {
                    from: pair[0],
                    to: pair[1],
                    reach,
                    rough,
                });
            }
        }
        let areas = [
            &map.landuse,
            &map.parks,
            &map.woods,
            &map.grass,
            &map.sand,
            &map.water,
            &map.parking,
            &map.pitches,
        ];
        for area in areas.into_iter().flatten() {
            if area.outer.len() >= 3 {
                covers.push(Cover::of(area_contours(area)));
            }
        }
        covers.retain(|cover| !cover.contours.is_empty());

        let mut cover_grid = Grid::new(TILE);
        for (index, cover) in covers.iter().enumerate() {
            cover_grid.insert(cover.low, cover.high, index);
        }
        let mut link_grid = Grid::new(MARGIN);
        for (index, link) in links.iter().enumerate() {
            link_grid.insert_segment(link.from, link.to, link.reach + TOUCH, index);
        }
        let mut edges = Vec::new();
        let mut edge_grid = Grid::new(MARGIN);
        for (block, area) in map.landuse.iter().enumerate() {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                for (index, &from) in ring.iter().enumerate() {
                    let to = ring[(index + 1) % ring.len()];
                    edge_grid.insert_segment(from, to, POCKET_NEAR, edges.len());
                    edges.push((from, to, block));
                }
            }
        }
        Self {
            covers,
            cover_grid,
            links,
            link_grid,
            edges,
            edge_grid,
        }
    }

    /// Карманы, чей габарит серединой в плитке `tile`.
    fn pockets(&self, tile: IVec2) -> Vec<Pocket> {
        let core_low = tile.as_vec2() * TILE;
        let core_high = core_low + TILE;
        let (low, high) = (core_low - MARGIN, core_high + MARGIN);
        let contours: Vec<Contour> = self
            .cover_grid
            .near(low, high)
            .into_iter()
            .map(|index| &self.covers[index])
            .filter(|cover| cover.low.cmple(high).all() && cover.high.cmpge(low).all())
            .flat_map(|cover| cover.contours.iter().cloned())
            .collect();
        if contours.is_empty() {
            return Vec::new();
        }
        let union: Vec<Shape> = contours.simplify_shape(FillRule::NonZero);
        let mut pockets = Vec::new();
        for hole in union.iter().flat_map(|shape| shape.iter().skip(1)) {
            let (hole_low, hole_high) = contour_bounds(hole);
            let middle = (hole_low + hole_high) / 2.0;
            // в окне целиком — значит, всё, что его касается, в окне есть;
            // серединой в плитке — значит, соседняя плитка его не возьмёт
            if hole_low.cmplt(low).any()
                || hole_high.cmpgt(high).any()
                || middle.cmplt(core_low).any()
                || middle.cmpge(core_high).any()
            {
                continue;
            }
            let ring = ring_of(hole);
            let area = ring_area(&ring);
            if !(POCKET_AREA_MIN..=POCKET_AREA_MAX).contains(&area) {
                continue;
            }
            let Some(block) = self.block_near(&ring) else {
                continue;
            };
            if !self.on_road(&ring, false) || self.on_road(&ring, true) {
                continue;
            }
            // край заводится под соседей: лента рисуется по сглаженной оси, а
            // покрытие считалось по сырым точкам
            let grown: Vec<Shape> = vec![vec![oriented(&ring, true)]]
                .outline(&OutlineStyle::new(LANDUSE_OVERLAP).line_join(LineJoin::Bevel));
            let Some(outer) = grown.first().and_then(|shape| shape.first()) else {
                continue;
            };
            pockets.push(Pocket {
                outer: ring_of(outer),
                block,
            });
        }
        pockets
    }

    /// Квартал, к кольцу которого ближе всего подходит карман, — если ближе
    /// [`POCKET_NEAR`].
    fn block_near(&self, ring: &[Vec2]) -> Option<usize> {
        ring.iter()
            .flat_map(|&point| {
                self.edge_grid.at(point).iter().map(move |&index| {
                    let (from, to, block) = self.edges[index];
                    (distance_to_segment(point, from, to), block)
                })
            })
            .filter(|&(distance, _)| distance <= POCKET_NEAR)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, block)| block)
    }

    /// Касается ли карман полотна дороги — грунтовой (`rough`) или мощёной.
    fn on_road(&self, ring: &[Vec2], rough: bool) -> bool {
        ring.iter().any(|&point| {
            self.link_grid.at(point).iter().any(|&index| {
                let link = self.links[index];
                link.rough == rough
                    && distance_to_segment(point, link.from, link.to) <= link.reach + TOUCH
            })
        })
    }
}

impl Cover {
    fn of(contours: Vec<Contour>) -> Self {
        let (low, high) = contours.iter().map(contour_bounds).fold(
            (Vec2::INFINITY, Vec2::NEG_INFINITY),
            |(low, high), (from, to)| (low.min(from), high.max(to)),
        );
        Self {
            contours,
            low,
            high,
        }
    }
}

/// Покрытие одной дороги: полотно с тротуаром ([`band`]) кусками по [`RUN`]
/// звеньев и обочины ([`verge_covers`]).
fn road_covers(road: &RoadLine) -> Vec<Cover> {
    let reach = road.sidewalk().mapped_edge(road.width / 2.0);
    let last = road.points.len() - 1;
    let mut covers: Vec<Cover> = (0..last)
        .step_by(RUN)
        .map(|start| {
            Cover::of(band(
                &road.points[start..=(start + RUN).min(last)],
                2.0 * reach,
            ))
        })
        .collect();
    verge_covers(road, &mut covers);
    covers
}

/// Полоса шириной `width` вокруг куска оси — **со срезанными углами и
/// квадратными торцами**, а не скруглёнными, как рисунок (`shapes::stroke`):
/// дуга — десяток точек на каждом изломе и торце, а цена объединения —
/// точки. Срез на изломе недокрывает наружный угол на сантиметры (карман
/// там не замкнут — его замыкает соседнее), квадратный торец перекрывает
/// торец дороги на полширины — ошибка в безопасную сторону (см. модуль).
fn band(path: &[Vec2], width: f32) -> Vec<Contour> {
    let contour: Contour = path.iter().map(Vec2::to_array).collect();
    let style = StrokeStyle::new(width)
        .line_join(LineJoin::Bevel)
        .start_cap(LineCap::Square)
        .end_cap(LineCap::Square);
    contour.stroke(style, false).into_iter().flatten().collect()
}

/// Обочины дороги ([`RoadLine::verge_at`]) — полосами от оси до кромки плюс
/// обочина, по кускам в [`VERGE_RUN`] точек. Та же постройка, что у рисунка
/// (`roads.rs::push_verges`), только по сырым точкам и без плитки с газоном:
/// покрытию важно, что земля закрыта, а не чем.
fn verge_covers(road: &RoadLine, covers: &mut Vec<Cover>) {
    let half = road.width / 2.0;
    let raw = arclengths(&road.points).1;
    for side in 0..2 {
        if road.verges[side] <= 0.0 {
            continue;
        }
        let dense = densify(&road.points, VERGE_STEP);
        if dense.len() < 2 {
            continue;
        }
        let (along, total) = arclengths(&dense);
        let scale = raw / total.max(f32::EPSILON);
        // `miter_offsets` плюсом сдвигает влево — сторона 0
        let sign = if side == 0 { 1.0 } else { -1.0 };
        let normals = miter_offsets(&dense, false, sign);
        let outer: Vec<Vec2> = dense
            .iter()
            .zip(&normals)
            .zip(&along)
            .map(|((&point, &normal), &at)| {
                point + normal * (half + road.verge_at(side, at * scale))
            })
            .collect();
        let last = dense.len() - 1;
        for start in (0..last).step_by(VERGE_RUN - 1) {
            let end = (start + VERGE_RUN - 1).min(last);
            let ring: Vec<Vec2> = outer[start..=end]
                .iter()
                .chain(dense[start..=end].iter().rev())
                .copied()
                .collect();
            if ring_area(&ring) > 0.0 {
                covers.push(Cover::of(vec![oriented(&ring, true)]));
            }
        }
    }
}
