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
//! его не описал. Карман, который касается мощёной дороги и мал
//! ([`POCKET_AREA_MAX`]), засевается травой (`MapData::pockets`) — какой,
//! решает сосед ([`Scene::grass_near`]): у квартала — травой того же
//! квартала (рисуется его слоем), у замапленного газона или сквера — лугом,
//! а без них — травой двора. Это правило газона широкой обочины
//! (`roads.rs::Meadows`): луг — только у замапленной зелени, по умолчанию —
//! двор. Без квартала рядом карман оставался землёй — бежевый параллелограмм
//! между газонами обочин у кольца Калуги 01.
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
use super::verges::verge_rings;
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, BuildingUse, MapData, PolyArea, RoadClass, RoadLine, distance_to_segment, ring_area,
};
use crate::map::parallel::in_parallel;
use crate::map::shapes::{Contour, Shape, area_contours, contour_bounds, oriented, ring_of};

/// Больше этого, м², дырка покрытия — уже не карман, а площадка: пустырь,
/// стройка, двор без тега. Клин Тулы 02 — около 150 м², параллелограмм у
/// кольца Калуги 01 — 8 × 20 м.
const POCKET_AREA_MAX: f32 = 400.0;
/// Меньше этого, м², — шум обводки, а не земля.
const POCKET_AREA_MIN: f32 = 0.05;
/// Меньше этого, м², карман без квартала и газона рядом — не земля, а щель
/// между полосами плитки (угол тротуара у перехода, Орёл 03): травой двора
/// он ложился на плитку тёмным пятном, землёй почти сливается с ней.
const LONE_POCKET_MIN: f32 = 10.0;
/// Как близко к кольцу квартала (или газона), м, должна подходить хоть одна
/// вершина кармана, чтобы взять его траву: край квартала сплошь и рядом
/// лежит **под** дорожкой, и клин за ней касается уже её полотна, а не
/// квартала. Ширина дорожки с запасом.
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
    /// Рёбра колец кварталов: `(от, до, вид квартала)`.
    blocks: Rim,
    /// Рёбра колец замапленной зелени (`parks`, `grass`) — вид всегда
    /// `Grass`: у газона и у сквера карман засевается одним лугом, как
    /// газон обочины рядом с ними.
    meadows: Rim,
}

/// Рёбра колец площадей одного рода с индексом по [`Grid`]: у какого кольца
/// карман и какой травой он тогда засевается.
struct Rim {
    edges: Vec<(Vec2, Vec2, AreaKind)>,
    grid: Grid<usize>,
}

/// Карман, найденный в плитке: кольцо, уже заведённое под соседей, и вид
/// травы, которой он засевается.
struct Pocket {
    outer: Vec<Vec2>,
    kind: AreaKind,
}

/// Засеять карманы земли травой ([`Scene::grass_near`]): каждый — площадью
/// в `MapData::pockets` (не в `landuse`: карман — не квартал, и обочина
/// рядом с ним двор не спрашивает). Возвращает, сколько.
///
/// Плитки друг от друга не зависят и считаются по потокам, как стоянки
/// (`lots::pave_lots`); порядок результата — порядок плиток, так что он не
/// зависит от числа потоков.
pub(super) fn fill_ground_pockets(map: &mut MapData) -> usize {
    let scene = Scene::of(map);
    let tiles = tiles_of(&map.roads);
    let pockets: Vec<Pocket> = in_parallel(&tiles, |&tile| scene.pockets(tile))
        .into_iter()
        .flatten()
        .collect();
    drop(scene);
    let sown = pockets.len();
    for Pocket { outer, kind } in pockets {
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

/// Плитки, которых касается хоть одно звено дороги (с запасом [`MARGIN`]:
/// середина кармана у дороги — не дальше), — по возрастанию: карман без
/// дороги на краю не засевается, и считать там нечего.
fn tiles_of(roads: &[RoadLine]) -> Vec<IVec2> {
    let cell = |point: Vec2| (point / TILE).floor().as_ivec2();
    let mut tiles: Vec<IVec2> = Vec::new();
    for pair in roads.iter().flat_map(|road| road.points.windows(2)) {
        let (low, high) = (
            cell(pair[0].min(pair[1]) - MARGIN),
            cell(pair[0].max(pair[1]) + MARGIN),
        );
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
        Self {
            covers,
            cover_grid,
            links,
            link_grid,
            blocks: Rim::of(map.landuse.iter().map(|area| (area, area.kind))),
            meadows: Rim::of(
                map.parks
                    .iter()
                    .chain(&map.grass)
                    .map(|area| (area, AreaKind::Grass)),
            ),
        }
    }

    /// Какой травой засевается карман `ring` площадью `area`: квартала рядом,
    /// иначе лугом у замапленного газона или сквера, иначе — травой двора
    /// (`Residential`), как газон обочины (`roads.rs::Meadows`). Карман без
    /// соседа меньше [`LONE_POCKET_MIN`] не засевается вовсе.
    fn grass_near(&self, ring: &[Vec2], area: f32) -> Option<AreaKind> {
        self.blocks
            .near(ring)
            .or_else(|| self.meadows.near(ring))
            .or((area >= LONE_POCKET_MIN).then_some(AreaKind::Residential))
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
            if !self.on_road(&ring, false) || self.on_road(&ring, true) {
                continue;
            }
            let Some(kind) = self.grass_near(&ring, area) else {
                continue;
            };
            // край заводится под соседей: лента рисуется по сглаженной оси, а
            // покрытие считалось по сырым точкам
            let grown: Vec<Shape> = vec![vec![oriented(&ring, true)]]
                .outline(&OutlineStyle::new(LANDUSE_OVERLAP).line_join(LineJoin::Bevel));
            let Some(outer) = grown.first().and_then(|shape| shape.first()) else {
                continue;
            };
            pockets.push(Pocket {
                outer: ring_of(outer),
                kind,
            });
        }
        pockets
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

impl Rim {
    fn of<'a>(areas: impl Iterator<Item = (&'a PolyArea, AreaKind)>) -> Self {
        let mut edges = Vec::new();
        let mut grid = Grid::new(MARGIN);
        for (area, kind) in areas {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                for (index, &from) in ring.iter().enumerate() {
                    let to = ring[(index + 1) % ring.len()];
                    grid.insert_segment(from, to, POCKET_NEAR, edges.len());
                    edges.push((from, to, kind));
                }
            }
        }
        Self { edges, grid }
    }

    /// Вид площади, к кольцу которой ближе всего подходит карман, — если
    /// ближе [`POCKET_NEAR`]; при равенстве — раньше заведённой.
    fn near(&self, ring: &[Vec2]) -> Option<AreaKind> {
        ring.iter()
            .flat_map(|&point| {
                self.grid.at(point).iter().map(move |&index| {
                    let (from, to, _) = self.edges[index];
                    (distance_to_segment(point, from, to), index)
                })
            })
            .filter(|&(distance, _)| distance <= POCKET_NEAR)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, index)| self.edges[index].2)
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
/// обочина ([`verge_rings`], во всю ширину): покрытию важно, что земля
/// закрыта, а не чем.
fn verge_covers(road: &RoadLine, covers: &mut Vec<Cover>) {
    for ring in verge_rings(road, |verge| verge) {
        covers.push(Cover::of(vec![oriented(&ring, true)]));
    }
}
