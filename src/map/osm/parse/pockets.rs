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
//! вливающимся в ту зелень ([`merge_into_green`]: сеяться рядом — значит
//! лечь на её кайму полосой и оставить видимый шов по краю посева), а без
//! них — травой двора. Это правило газона широкой обочины
//! (`roads.rs::Meadows`): луг — только у замапленной зелени, по умолчанию —
//! двор. Без квартала рядом карман оставался землёй — бежевый параллелограмм
//! между газонами обочин у кольца Калуги 01.
//!
//! Что должно остаться землёй, остаётся ею: карман у грунтовой тропы — не
//! двор (пустырь с тропинками, как юго-запад Орла, 03), большой — это уже
//! площадка сама по себе (пустырь, стройка), а у дырки в самом квартале
//! (мультиполигон вырезал её нарочно) нет дороги на краю.
//!
//! **Фартук зелени** — вторая половина вопроса, для полос, которые дыркой
//! не бывают: голая земля между внешним краем замапленной зелени и кромкой
//! мощёной дорожки (`RoadClass::Alley`) шириной до [`GREEN_GAP_MAX`]
//! засевается лугом от края зелени до самой кромки (`green_aprons`).
//! Велодорожка вдоль Центрального парка идёт параллельно его краю в
//! полутора-двух метрах, и между ними светилась полоса голой земли —
//! тёмной линией вдоль кромки (Оружейная × Энгельса, R37-02): на концах
//! такая полоса протекает, и «что замкнуто нарисованным» её не видит.
//! Улице зелень подшивается к тротуарной ленте — но только там, где её
//! собственный газон обочины полосы не закрывает (по месту,
//! [`RoadLine::verge_at`]): полоса начинается у края того, что улица
//! рисует сама — ленты или газона обочины, кто дальше, — и фартук не
//! ложится на чужой цвет (Тула, сквер у Сойфера: полоса земли 1 × 7 м у
//! тротуарной ленты улицы Фёдора Смирнова, N8b). Кромку зелени фартук
//! спрашивает после вливания карманов: влитый растит зелень к самой ленте,
//! и полоса открыта именно от выращенной кромки. Грунтовой тропе — потому
//! что земля у неё пустырь, как и у кармана.
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
use super::verges::{Cover, verge_rings};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    AreaKind, BuildingUse, MapData, PolyArea, RoadClass, RoadLine, closest_on_segment,
    distance_to_segment, ring_area,
};
use crate::map::parallel::in_parallel;
use crate::map::roads::rings::islands as ring_islands;
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
/// Ширина полосы голой земли между краем замапленной зелени и кромкой
/// дорожной ленты, м, — фартук зелени (R37-02): велодорожка вдоль
/// Центрального парка идёт параллельно его краю в полутора-двух метрах, и
/// между ними светилась полоса голой земли — тёмной линией вдоль кромки.
/// Замкнутым карманом такая полоса не бывает: на концах она протекает, и
/// вопрос «что замкнуто нарисованным» её не видит. Шире — уже не щель, а
/// настоящая граница, и землю там видно нарочно.
const GREEN_GAP_MAX: f32 = 2.5;
/// Шаг опроса края зелени для фартука, м.
const APRON_STEP: f32 = 1.0;
/// Скачок кромки ленты между соседними пробами фартука, м: проба сменила
/// ближайшую дорогу — полоса рвётся, а не тянется через чужую ленту.
const APRON_BREAK: f32 = 3.0;
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

/// Звено дороги для вопроса «касается ли карман полотна».
#[derive(Clone, Copy)]
struct Link {
    from: Vec2,
    to: Vec2,
    reach: f32,
    /// Грунтовая тропа или улица: карман у неё — не двор.
    rough: bool,
    /// Дорожка (`RoadClass::Alley`): фартук зелени подшивается к краю её
    /// полотна всегда.
    alley: bool,
    /// Улица ([`RoadLine::is_carriageway`]): фартук зелени подшивается и к её
    /// тротуарной ленте — сама улица (копия: фартук шьётся, когда `MapData`
    /// уже снова меняется) в [`Scene::streets`], а обочина читается по месту
    /// (`RoadLine::verge_at`). Здесь — индекс улицы и метров по её точкам от
    /// начала до `from`.
    street: Option<(u32, f32)>,
}

/// Всё, что нужно проходу, собранное один раз на карту. Сцена владеет своими
/// данными целиком, поэтому живёт и дальше вливания карманов — фартук зелени
/// спрашивает кромку, которую влитые карманы уже вырастили.
struct Scene {
    /// Улицы для фартука зелени ([`Link::street`]): обочины по месту.
    streets: Vec<RoadLine>,
    covers: Vec<Cover>,
    cover_grid: Grid<usize>,
    links: Vec<Link>,
    link_grid: Grid<usize>,
    /// Те же звенья с запасом [`GREEN_GAP_MAX`]: фартук зелени спрашивает
    /// кромку ленты издалека, а не с краю.
    apron_grid: Grid<usize>,
    /// Рёбра колец кварталов: `(от, до, вид квартала)`.
    blocks: Rim<AreaKind>,
    /// Рёбра колец замапленной зелени (`parks`, `grass`) с номером зелени:
    /// у газона и у сквера карман вливается в неё одним лугом, как газон
    /// обочины рядом с ними.
    meadows: Rim<Green>,
}

/// Рёбра колец площадей одного рода с индексом по [`Grid`]: у какого кольца
/// карман и что с ним тогда делать.
struct Rim<T> {
    edges: Vec<(Vec2, Vec2, T)>,
    grid: Grid<usize>,
}

/// Замапленная зелень (`parks`, `grass`), в которую вливается карман-луг.
#[derive(Clone, Copy)]
struct Green {
    park: bool,
    index: usize,
}

/// Карман, найденный в плитке: кольцо, уже заведённое под соседей, и вид
/// травы, которой он засевается. `absorb` — зелень, в которую карман вливается
/// вместо отдельного посева ([`merge_into_green`]).
struct Pocket {
    outer: Vec<Vec2>,
    kind: AreaKind,
    absorb: Option<Green>,
}

/// Засеять карманы земли травой ([`Scene::grass_near`]): каждый — площадью
/// в `MapData::pockets` (не в `landuse`: карман — не квартал, и обочина
/// рядом с ним двор не спрашивает). Возвращает, сколько.
///
/// Карман-луг, касающийся своей зелени, в неё вливается ([`merge_into_green`])
/// — по порядку плиток, так что и это не зависит от числа потоков. Фартук
/// зелени (`green_aprons`) шьётся после вливания, по кромке, которую влитые
/// карманы уже вырастили: полоса у ленты улицы открыта именно от неё
/// (сквер Сойфера, N8b), и спрашивать прежнюю — значит оставить полосу.
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
    let sown = pockets.len();
    for Pocket {
        outer,
        kind,
        absorb,
    } in pockets
    {
        if absorb.is_some_and(|green| merge_into_green(map, green, &outer)) {
            continue;
        }
        map.pockets.push(sown_area(outer, kind));
    }
    // кромка зелени, выращенной вливанием, — по ней шьётся фартук
    let edges: Vec<(Vec2, Vec2)> = map
        .parks
        .iter()
        .chain(&map.grass)
        .flat_map(|area| {
            let ring = &area.outer;
            (0..ring.len()).map(move |index| (ring[index], ring[(index + 1) % ring.len()]))
        })
        .collect();
    let aprons = scene.green_aprons(&edges);
    let grown = aprons.len();
    for Pocket { outer, kind, .. } in aprons {
        map.pockets.push(sown_area(outer, kind));
    }
    sown + grown
}

/// Посев площадкой в `MapData::pockets`: без дырок, дверей и высоты — кольцо
/// и вид травы, ничего лишнего у посева нет.
fn sown_area(outer: Vec<Vec2>, kind: AreaKind) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use: BuildingUse::Other,
        height: None,
        storeys: None,
        entrances: Vec::new(),
        colours: Default::default(),
    }
}

/// Влить карман `pocket` (уже заведённый под соседей) в замапленную зелень:
/// их объединение становится её внешним кольцом. `false` — карман зелени не
/// касался (`Rim::near` ответил про неё по вершине в четырёх метрах, а расти
/// до неё карману не хватило своих полметра): сеется отдельной площадкой,
/// как раньше.
///
/// Карман-луг, посеянный рядом с газоном, лёг бы на его кайму полосой в
/// полметра ([`LANDUSE_OVERLAP`]), и там, где посев кончался, срезанная кайма
/// читалась швом — две светлые линии сходящимся клином (Тула, сквер у
/// Сойфера, N8). Влившийся карман растит зелень целиком, и кайма остаётся
/// только по внешнему краю — против квартала и дороги.
fn merge_into_green(map: &mut MapData, green: Green, pocket: &[Vec2]) -> bool {
    let areas = if green.park {
        &mut map.parks
    } else {
        &mut map.grass
    };
    let Some(area) = areas.get_mut(green.index) else {
        return false;
    };
    let joined: Vec<Shape> = vec![
        vec![oriented(&area.outer, true)],
        vec![oriented(pocket, true)],
    ]
    .simplify_shape(FillRule::NonZero);
    let [shape] = joined.as_slice() else {
        return false;
    };
    let Some(outer) = shape.first() else {
        return false;
    };
    area.outer = super::collapse_slivers(&ring_of(outer));
    area.holes = shape
        .iter()
        .skip(1)
        .map(|hole| super::collapse_slivers(&ring_of(hole)))
        .collect();
    area.holes.retain(|hole| hole.len() >= 3);
    true
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
        // улицы для фартука зелени — копии: сцена живёт дальше вливания
        // карманов, когда `map` снова меняется
        let mut streets: Vec<RoadLine> = Vec::new();
        for road in roads.iter() {
            let reach = road.sidewalk().mapped_edge(road.width / 2.0);
            let rough = road.is_unpaved_street()
                || (road.class == RoadClass::Alley && !road.is_paved_path());
            let alley = road.class == RoadClass::Alley;
            let street = (!rough && road.is_carriageway()).then(|| {
                streets.push((**road).clone());
                (streets.len() - 1) as u32
            });
            let mut at = 0.0;
            for pair in road.points.windows(2) {
                links.push(Link {
                    from: pair[0],
                    to: pair[1],
                    reach,
                    rough,
                    alley,
                    street: street.map(|street| (street, at)),
                });
                at += pair[0].distance(pair[1]);
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
        // остров кольца кроется газоном целиком (`roads.rs::ring_island_lawns`)
        for island in ring_islands(&map.roads) {
            covers.push(Cover::of(vec![oriented(&island, true)]));
        }
        covers.retain(|cover| !cover.contours.is_empty());

        let mut cover_grid = Grid::new(TILE);
        for (index, cover) in covers.iter().enumerate() {
            cover_grid.insert(cover.low, cover.high, index);
        }
        let mut link_grid = Grid::new(MARGIN);
        let mut apron_grid = Grid::new(MARGIN);
        for (index, link) in links.iter().enumerate() {
            link_grid.insert_segment(link.from, link.to, link.reach + TOUCH, index);
            apron_grid.insert_segment(link.from, link.to, link.reach + GREEN_GAP_MAX, index);
        }
        Self {
            streets,
            covers,
            cover_grid,
            links,
            link_grid,
            apron_grid,
            blocks: Rim::of(map.landuse.iter().map(|area| (area, area.kind))),
            meadows: Rim::of(
                map.parks
                    .iter()
                    .enumerate()
                    .map(|(index, area)| (area, Green { park: true, index }))
                    .chain(
                        map.grass
                            .iter()
                            .enumerate()
                            .map(|(index, area)| (area, Green { park: false, index })),
                    ),
            ),
        }
    }

    /// Какой травой засевается карман `ring` площадью `area` и куда он
    /// вливается: квартала рядом, иначе лугом у замапленного газона или
    /// сквера — в ту зелень, к чьему кольцу ближе всего подошёл, — иначе
    /// травой двора (`Residential`), как газон обочины (`roads.rs::Meadows`).
    /// Карман без соседа меньше [`LONE_POCKET_MIN`] не засевается вовсе.
    fn grass_near(&self, ring: &[Vec2], area: f32) -> Option<(AreaKind, Option<Green>)> {
        self.blocks
            .near(ring)
            .map(|kind| (kind, None))
            .or_else(|| {
                self.meadows
                    .near(ring)
                    .map(|green| (AreaKind::Grass, Some(green)))
            })
            .or_else(|| (area >= LONE_POCKET_MIN).then_some((AreaKind::Residential, None)))
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
            .filter(|cover| cover.overlaps(low, high))
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
            let Some((kind, absorb)) = self.grass_near(&ring, area) else {
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
                absorb,
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

    /// Фартук зелени: полосы голой земли не шире [`GREEN_GAP_MAX`] между
    /// внешним краем замапленной зелени и кромкой мощёной ленты — дорожки
    /// или улицы ([`band_edge`]) — лугом, от края зелени до самой кромки.
    /// Край — тот, что зелень имеет **после вливания карманов**: влитый
    /// карман растит её до самой ленты, и полоса улицы открыта именно от
    /// выращенной кромки (сквер Сойфера, N8b). Пробы идут по краю зелени
    /// каждые [`APRON_STEP`] м; смена ближайшей дороги рвёт полосу
    /// ([`APRON_BREAK`]). Грунтовой тропе зелень не подшивается: земля у
    /// неё — пустырь, как у кармана.
    fn green_aprons(&self, edges: &[(Vec2, Vec2)]) -> Vec<Pocket> {
        let mut aprons = Vec::new();
        let mut run: Vec<(Vec2, Vec2)> = Vec::new();
        for &(from, to) in edges {
            let length = from.distance(to);
            let steps = (length / APRON_STEP).ceil() as usize;
            for step in 0..=steps {
                let point = if step == steps {
                    to
                } else {
                    from.lerp(to, step as f32 / (steps.max(1)) as f32)
                };
                let Some(edge) = self.band_edge(point) else {
                    close_apron(&mut run, &mut aprons);
                    continue;
                };
                if run
                    .last()
                    .is_some_and(|&(_, previous)| previous.distance(edge) > APRON_BREAK)
                {
                    close_apron(&mut run, &mut aprons);
                }
                run.push((point, edge));
            }
            close_apron(&mut run, &mut aprons);
        }
        aprons
    }

    /// Кромка мощёной ленты для пробы края зелени `point`: у дорожки
    /// (`RoadClass::Alley`) — внешний край полотна, у улицы — тротуарная
    /// лента, а дальше её газон обочины, если он шире. Полоса голой земли
    /// начинается у края того, что улица рисует сама, и вопрос фартука —
    /// не «есть ли у неё обочина», а «доходит ли этот край до кромки
    /// зелени» ([`GREEN_GAP_MAX`]): недошедшее — залив. Точка на этом краю
    /// по направлению к пробе; `None` — проба на ленте, вплотную к ней или
    /// дальше предела; у проезда без тротуарной ленты края нет. Грунтовой
    /// тропе зелень не подшивается: земля у неё — пустырь, как у кармана.
    fn band_edge(&self, point: Vec2) -> Option<Vec2> {
        let (index, _, reach) = self
            .apron_grid
            .at(point)
            .iter()
            .filter_map(|&index| {
                let link = self.links[index];
                if link.rough {
                    return None;
                }
                let distance = distance_to_segment(point, link.from, link.to);
                // что граничит с полосой: лента или газон обочины, кто дальше
                let reach = match link.street {
                    Some((road, at)) => {
                        let road = &self.streets[road as usize];
                        let nearest = closest_on_segment(point, link.from, link.to);
                        // проба за торцом улицы: квадратный торец её покрытия —
                        // фикция охвата (лента у узла подрезана рисунком), краем
                        // для фартука быть не может; полосу там закрывает лента
                        // той дороги, вдоль которой идёт зелень
                        if nearest == link.from || nearest == link.to {
                            return None;
                        }
                        let heading = (link.to - link.from).normalize_or_zero();
                        // `perp` — влево по ходу точек, сторона 0
                        let side = usize::from((point - nearest).dot(heading.perp()) <= 0.0);
                        let along = at + nearest.distance(link.from);
                        link.reach
                            .max(road.width / 2.0 + road.verge_at(side, along))
                    }
                    None if link.alley => link.reach,
                    None => return None,
                };
                let gap = distance - reach;
                ((TOUCH..=GREEN_GAP_MAX).contains(&gap)).then_some((index, gap, reach))
            })
            // чей край ближе подошёл к зелени, тот и закрывает полосу —
            // дальше его края голой земли не остаётся
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let link = self.links[index];
        let nearest = closest_on_segment(point, link.from, link.to);
        let towards = (point - nearest).try_normalize().unwrap_or_default();
        Some(nearest + towards * reach)
    }
}

/// Закрыть начатый фартук: полоса от края зелени (пробы) до кромки ленты
/// (их проекции), одной площадкой луга. Короче двух проб — не полоса.
fn close_apron(run: &mut Vec<(Vec2, Vec2)>, aprons: &mut Vec<Pocket>) {
    if run.len() >= 2 {
        let mut outer: Vec<Vec2> = run.iter().map(|&(point, _)| point).collect();
        outer.extend(run.iter().rev().map(|&(_, edge)| edge));
        aprons.push(Pocket {
            outer,
            kind: AreaKind::Grass,
            absorb: None,
        });
    }
    run.clear();
}

impl<T: Copy> Rim<T> {
    fn of<'a>(areas: impl Iterator<Item = (&'a PolyArea, T)>) -> Self {
        let mut edges = Vec::new();
        let mut grid = Grid::new(MARGIN);
        for (area, tag) in areas {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                for (index, &from) in ring.iter().enumerate() {
                    let to = ring[(index + 1) % ring.len()];
                    grid.insert_segment(from, to, POCKET_NEAR, edges.len());
                    edges.push((from, to, tag));
                }
            }
        }
        Self { edges, grid }
    }

    /// Тег площади, к кольцу которой ближе всего подходит карман, — если
    /// ближе [`POCKET_NEAR`]; при равенстве — раньше заведённой.
    fn near(&self, ring: &[Vec2]) -> Option<T> {
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
