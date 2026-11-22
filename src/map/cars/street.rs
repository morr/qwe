//! Расстановка машин: ряды вдоль улиц и заполнение размеченных стоянок.
//!
//! Здесь раскладка — что считается местом, что место запрещает (перекрёстки,
//! сплошные, мосты) и какая доля мест занята (ползунок плюс застройка вокруг).
//! Стиль, ступени зума и сборка меша — в родителе; кузова рисует
//! [`super::body`], плотность квартала читает [`super::district`], дворовые
//! проезды расставляет [`super::yard`].

use std::borrow::Cow;

use bevy::prelude::*;

use super::CarStyle;
use super::body::{self, Car, CarShape};
use super::district::Districts;
use super::rails;
use super::yard;
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::Break;
use crate::map::osm::model::{distance_to_segment, ring_vertex_mean};
use crate::map::osm::parse::DrawBuildout;
use crate::map::osm::{MapData, PolyArea, RoadLine, TrafficSide};
use crate::map::parking::{ParkingLayout, Stall};
use crate::map::prepare::PreparedMeshes;
use crate::map::roads::junctions::{self, JUNCTION_MARGIN};
use crate::map::roads::network::RoadNodes;
use crate::map::roads::network::sections::EDGE_WIDTH;
use crate::map::roads::pockets::{self, KerbLots, Kerbside, POCKET_WIDTH, RowBreaks};
use crate::map::roads::shape::RoadShape;
use crate::map::roads::{Axis, Drawn};
use crate::map::roads::{is_carriageway, lane_count, paint};
use crate::map::seed::{Lcg, seed_from_point};

/// Шаг парковочного места вдоль улицы, м: машина плюс просвет. Одно число с
/// местом вдоль бордюра в кармане-стоянке — живёт у раскладки, машины его читают.
pub const CAR_PITCH: f32 = crate::map::parking::PARALLEL_LENGTH;
/// Насколько край машины отступает от кромки проезжей части, м. Ноль —
/// колесо на кромке; полметра оставляют полосу асфальта между рядом и
/// разметкой, как на настоящей улице.
pub const CURB_GAP: f32 = 0.5;
/// Отступ от торца нарисованной ленты, м: машина, поставленная вплотную к
/// концу улицы, свисала бы с него. Про перекрёстки этот отступ ничего не
/// знает — way кончается где угодно, а перекрёсток восстанавливается по
/// общим нодам (`map::roads::junctions`).
pub const END_MARGIN: f32 = 2.0;
/// Небрежность парковки: разброс угла, градусы, и поперечного отступа, м.
/// Ряд, выровненный по линейке, читается как разметка склада, а не как двор;
/// оба числа малы нарочно — машина не должна вылезти на разметку или на
/// тротуар (полметра `CURB_GAP` держит и перекос).
pub const PARK_SKEW_DEGREES: f32 = 2.5;
pub const PARK_SLOP: f32 = 0.12;
/// Насколько **кузов** не доходит до разрыва ряда, м, сверх его `Break::reach`
/// (у перекрёстка — полуширина самой широкой из сошедшихся дорог, у перехода
/// — полдлины зебры, `roads::pockets::row_breaks`): ближе пяти метров к
/// перекрёстку и переходу не паркуются, и ещё метр — зебра по правилу стоит
/// за кромкой узла на метр и тянется на четыре (`roads::node_paint`), а
/// машина, отмеренная центром в пяти метрах, вставала на неё носом. Тупик
/// приходит разрывом нулевого `reach`, и клиренс даёт в нём те же метры.
pub const JUNCTION_CLEARANCE: f32 = 6.0;
/// Насколько кузов не доходит до кромки **пересекаемой проезжей части**, м
/// (R31). ПДД 12.4 требует пять метров, но у настоящего перекрёстка машина
/// стоит дальше: ближе десяти метров она стоит у стоп-линии, в самом устье, и
/// читалась помехой на выезде. Кромка — вылет разрыва без
/// `junctions::JUNCTION_MARGIN`: вылет — полуширина самой широкой из
/// сошедшихся плюс этот запас.
const CROSSED_EDGE_CLEARANCE: f32 = 10.0;
/// Ближе этого, м, к сплошной линии разметки не паркуются (ПДД 12.4): вдоль
/// сплошной подхода ([`paint::APPROACH`] до перекрёстка) на улице в полосу
/// в каждую сторону ряда нет вовсе, — машина у бордюра там в полутора метрах
/// от осевой. Карман отодвигает ряд на свою ширину, и в нём стоять можно.
const SOLID_LINE_CLEARANCE: f32 = 3.0;
/// Запас по прямой, в полуширинах дороги, за которым разрыв заведомо не
/// мешает месту, как бы ни легла проекция на звено: на изломе узел за
/// поворотом проецируется на текущее звено коротко, и без этого запаса он
/// вычёркивал бы места на всей длине прямой до него.
const BREAK_BEND_SLACK_HALF_WIDTHS: f32 = 4.0;
/// Через сколько метров улицы застройка вокруг перечитывается заново, м.
/// Квартал не меняется от места к месту, а запрос к [`Districts`] на каждое из
/// двадцати двух тысяч мест стоил бы больше, чем весь слой; полсотни метров —
/// это пара участков частного сектора и торец секции, то есть тот масштаб, на
/// котором застройка и в самом деле успевает смениться. Длинная улица,
/// выходящая из частного сектора в микрорайон, при этом меняет плотность там,
/// где меняется город, а не там, где кончается way.
pub const DISTRICT_STEP: f32 = 48.0;
/// Какая доля мест занята на **размеченной стоянке**, от малой к большой
/// ([`lot_occupancy`]). Двор на пару десятков мест заставлен наполовину, а
/// стоянка торгового центра на сотни мест почти пуста: забитым её видно только
/// в час пик, и сплошное поле машин читалось автосалоном. Свои константы, а не
/// ползунок `CarStyle::occupancy`: тот про рваный ряд у бордюра, а полупустая
/// стоянка — это другое наблюдение, и крутить их вместе нечем.
///
/// Сверх этой доли стоянка домножается на множитель квартала ([`Districts`]),
/// как и ряд у бордюра: размер стоянки и застройка вокруг — два независимых
/// наблюдения, поэтому они перемножаются, а не спорят за одно число.
pub const LOT_OCCUPANCY_SMALL: f32 = 0.5;
pub const LOT_OCCUPANCY_LARGE: f32 = 0.12;
/// Мест на стоянке, до которых заполненность ещё [`LOT_OCCUPANCY_SMALL`], и от
/// которых уже [`LOT_OCCUPANCY_LARGE`]; между ними — по логарифму числа мест.
const LOT_SMALL_STALLS: f32 = 20.0;
const LOT_LARGE_STALLS: f32 = 400.0;

/// Расстановка машин без меша: всё, что слой знает до первой вершины, и
/// ничто в ней не зависит ни от ступени зума, ни от солнца.
#[derive(Default)]
pub struct ParkedCars {
    pub cars: Vec<Car>,
    /// Перекрёстков, по которым рвутся ряды.
    pub junctions: usize,
    /// Время расстановки целиком — каркас дорог, разрывы, квартала, ряды.
    pub took: std::time::Duration,
    /// Доля `took`, ушедшая на поиск перекрёстков.
    pub breaks_took: std::time::Duration,
}

/// Расставить машины: каркас дорог (`Drawn::nodal` — та же узловая ось, по
/// которой `map::roads` кладёт ленту), разрывы на перекрёстках, застройка
/// вокруг, ряды вдоль улиц и стоянки.
pub fn park_all(
    style: CarStyle,
    shape: &RoadShape,
    map: &MapData,
    layout: &ParkingLayout,
) -> ParkedCars {
    let started = std::time::Instant::now();
    let drawn = Drawn::nodal(map, shape);
    park_on(style, &drawn, map, layout, started)
}

/// Расстановка по готовому каркасу; `started` — откуда мерить `took`.
pub(in crate::map::cars) fn park_on(
    style: CarStyle,
    drawn: &Drawn,
    map: &MapData,
    layout: &ParkingLayout,
    started: std::time::Instant,
) -> ParkedCars {
    // сырой OSM, второй уровень: машины — наша достройка, их нет вовсе
    if map.knobs.raw.disables(DrawBuildout::ParkedCars) {
        return ParkedCars {
            took: started.elapsed(),
            ..Default::default()
        };
    }
    // разрывы — по **всем** настоящим улицам, а не только по парковочным: ряд
    // обязан прерваться и там, где к жилой улице примыкает другая жилая, — и
    // на клиньях между сечениями улицы: бордюр там ближе к оси; те же
    // разрывы режут карманы ленты (`roads::pockets`)
    let breaks_started = std::time::Instant::now();
    let junctions = pockets::row_breaks(&map.roads, drawn.nodes(), drawn.tapers(), &map.road_nodes);
    let breaks_took = breaks_started.elapsed();
    // застройка вокруг — тем же проходом и с тем же сроком жизни, что и
    // разрывы: индекс на 7.6 тысячи домов дешевле, чем повод его кешировать
    let districts = Districts::new(&map.buildings);
    let axes = drawn.axes(Axis::Nodal);
    let mut cars = park_cars(
        &map.roads,
        drawn.nodes(),
        &junctions,
        style,
        &axes,
        map.traffic_side,
        &districts,
        drawn.lots(),
    );
    // дворовые проезды — своим проходом: у них нет бордюра, и место каждой
    // машины проверяется по домам, стоянкам и чужим лентам (`yard.rs`)
    cars.extend(yard::park_yards(
        &map.roads,
        drawn.nodes(),
        &junctions,
        &axes,
        style.occupancy,
        &districts,
        &yard::Blocked::new(map),
    ));
    cars.extend(fill_lots(&map.parking, &layout.0, &districts));
    // последний шаг всех трёх расстановок: на путях и у самого балласта
    // машина не стоит (`rails.rs`)
    let keepout = rails::RailKeepout::new(&map.rails);
    cars.retain(|car| !keepout.blocks(car));
    ParkedCars {
        cars,
        junctions: junctions.junctions,
        took: started.elapsed(),
        breaks_took,
    }
}

/// Расставленные машины текущего мира — **между пересечениями порога зума**.
///
/// Ступень зума меняет только подробность кузова, а расстановка от неё не
/// зависит; до кеша каждое пересечение порога заново строило каркас дорог
/// (`Drawn::nodal`, 25 мс Тула / 49 Калуга) и расставляло машины. Теперь
/// пересечение платит один меш кузовов.
///
/// Ключ — то, от чего расстановка зависит из настроек: занятость (`CarStyle`
/// без тумблера) и форма дорог. Город ключом не является: при входе в мир
/// кеш заменяется расстановкой из потока загрузки (`seed_parked_cars`).
#[derive(Resource, Default)]
pub struct CarPlacement {
    key: Option<(f32, RoadShape)>,
    pub(in crate::map::cars) parked: ParkedCars,
}

impl CarPlacement {
    /// Расставить заново, если ключ поехал; `true` — если расставили.
    pub(in crate::map::cars) fn refresh(
        &mut self,
        style: CarStyle,
        shape: RoadShape,
        map: &MapData,
        layout: &ParkingLayout,
    ) -> bool {
        let key = (style.occupancy, shape);
        if self.key == Some(key) {
            return false;
        }
        self.parked = park_all(style, &shape, map, layout);
        self.key = Some(key);
        true
    }
}

/// Расстановка для потока загрузки (`map/prepare.rs`): тот же пересчёт, что
/// раньше шёл на входе в мир, — без мира. Сырому OSM машины не нужны вовсе:
/// расстановка остаётся пустой, ключ не ставится, и `rebuild_cars` на входе
/// не пересчитывает.
pub(crate) fn place(
    style: CarStyle,
    shape: RoadShape,
    map: &MapData,
    layout: &ParkingLayout,
) -> CarPlacement {
    let mut placement = CarPlacement::default();
    if !map.knobs.raw.disables(DrawBuildout::ParkedCars) {
        placement.refresh(style, shape, map, layout);
    }
    placement
}

/// Расстановка из потока загрузки — в ресурс мира: ключ уже совпадает с
/// текущими ручками, и `rebuild_cars` на входе платит только меш кузовов.
/// Замена целым ресурсом — и сброс прошлого города заодно.
pub fn seed_parked_cars(mut placement: ResMut<CarPlacement>, mut prepared: ResMut<PreparedMeshes>) {
    *placement = std::mem::take(&mut prepared.0.parked);
}

/// Ряды вдоль всех улиц, годных под парковку.
///
/// `junctions` — разрывы ряда ([`pockets::row_breaks`]), индексированы по
/// номеру дороги **во входном срезе**, поэтому `roads` — весь срез карты, а не
/// отфильтрованный список парковочных; `shared` — их узлы (карман через торец
/// way идёт на продолжение улицы), `axes` — по тому же индексу, нарисованные
/// оси дорог.
#[allow(clippy::too_many_arguments)]
pub(in crate::map::cars) fn park_cars(
    roads: &[RoadLine],
    shared: &RoadNodes,
    junctions: &RowBreaks,
    style: CarStyle,
    axes: &[Cow<[Vec2]>],
    traffic: TrafficSide,
    districts: &Districts,
    lots: &KerbLots,
) -> Vec<Car> {
    let mut cars = Vec::new();
    let kerb = traffic.kerb();
    let density = Density {
        base: style.occupancy,
        districts,
    };
    let decks: Vec<BridgeDeck> = roads.iter().filter_map(BridgeDeck::of).collect();
    let kerbsides = pockets::all_kerbsides(roads, shared, axes, junctions, traffic, lots);
    // перекрёстки проезжих частей — те, у которых краска кладёт стоп-линию и
    // сплошную подхода (`roads/paint.rs`): от них ряд держится дальше
    // ([`CROSSED_EDGE_CLEARANCE`], [`SolidLine`])
    let crossed = junctions::marking_breaks(roads, is_carriageway, &[]);
    let mut near = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !pockets::parkable(road) {
            continue;
        }
        near.clear();
        near.extend(
            decks
                .iter()
                .filter(|deck| deck.near(&road.points, road.width)),
        );
        // осевая та же, по которой `map::roads` строит ленту (`roads/axis.rs`):
        // по сырым точкам OSM ряд на изломе съезжает с асфальта на тротуар,
        // потому что дуга уводит ось от вершины на метры
        let centre = &axes[index];
        let mut rng = Lcg::new(seed_from_point(
            road.points.first().copied().unwrap_or(Vec2::ZERO),
        ));
        // односторонняя — один ряд, у бордюра своей стороны движения: у
        // половины разделённого проспекта там бордюр, а с другой стороны
        // разделительная. Порядок точек way совпадает с направлением потока
        // (`oneway=-1` развёрнут при разборе), а поперечная в [`park_along`]
        // (`direction.perp()`) смотрит влево, поэтому сторона — знак
        // `TrafficSide::kerb`.
        //
        // Нос машины смотрит по потоку своей полосы: у бордюра стороны
        // движения — по ходу way, у противоположного — против. Порядок сторон
        // двусторонней улицы не зависит от `traffic`: он решает поток ГПСЧ, и
        // от стороны движения ряд не должен переставляться, только
        // разворачиваться
        //
        // Стороны и карманы на них — от `roads::pockets`, того же ответа, по
        // которому лента кладёт асфальт кармана
        for kerbside in &kerbsides[index] {
            if !kerbside.lane && kerbside.pockets.is_empty() {
                continue;
            }
            let side = kerbside.side;
            park_along(
                &mut cars,
                centre,
                road.width / 2.0,
                Kerb {
                    side,
                    heading: if side == kerb { 1.0 } else { -1.0 },
                    stand: kerbside,
                },
                &Clearings {
                    junctions: junctions.of(index),
                    crossed: &crossed.breaks[index],
                    line: SolidLine::of(road),
                    decks: &near,
                },
                &density,
                &mut rng,
            );
        }
    }
    cars
}

/// Машины на размеченных стоянках: то же место, что и у разметки
/// (`map::parking::ParkingLayout`), — иначе машина встала бы мимо своей полосы.
/// Занята доля мест по размеру стоянки ([`lot_occupancy`]) и по застройке
/// вокруг неё ([`Districts`]): полная стоянка выглядит как автосалон, пустая —
/// как чертёж, а забитая стоянка посреди частного сектора — как чужой двор.
fn fill_lots(lots: &[PolyArea], layout: &[Vec<Stall>], districts: &Districts) -> Vec<Car> {
    let mut cars = Vec::new();
    for (lot, stalls) in lots.iter().zip(layout) {
        let mut rng = Lcg::new(lot_seed(lot));
        // квартал читается по центру пятна, а не по первой вершине контура,
        // которой стоянка засеяна: у вытянутой вдоль квартала стоянки угол и
        // середина стоят в разной застройке
        let around = ring_vertex_mean(&lot.outer).map_or(1.0, |at| districts.fill_at(at));
        let occupancy = (lot_occupancy(stalls.len()) * around).clamp(0.0, 1.0);
        for stall in stalls {
            if rng.next_f32() >= occupancy {
                continue;
            }
            cars.push(Car {
                at: stall.at,
                along: stall.along,
                color: body::color_from_share(rng.next_f32()),
                shape: CarShape::from_share(rng.next_f32()),
            });
        }
    }
    cars
}

/// Доля занятых мест на стоянке из `stalls` мест: чем стоянка больше, тем она
/// пустее. По логарифму — разница между двором на 20 мест и на 40 заметна, а
/// между стоянками на 400 и 800 уже нет.
pub fn lot_occupancy(stalls: usize) -> f32 {
    let t = ((stalls as f32).max(1.0) / LOT_SMALL_STALLS).ln()
        / (LOT_LARGE_STALLS / LOT_SMALL_STALLS).ln();
    LOT_OCCUPANCY_SMALL + (LOT_OCCUPANCY_LARGE - LOT_OCCUPANCY_SMALL) * t.clamp(0.0, 1.0)
}

/// Посев стоянки — от её первой вершины, тем же [`seed_from_point`], что у
/// улиц, домов и крон.
fn lot_seed(lot: &PolyArea) -> u32 {
    seed_from_point(lot.outer.first().copied().unwrap_or(Vec2::ZERO))
}

/// Полотно моста, от которого ряд держится на [`JUNCTION_CLEARANCE`] — улица
/// под мостом или упёршаяся в его бок. Общей ноды с мостом у такой улицы нет
/// (`junctions` про неё не знает), а слой машин (`Z_CAR`) лежит **над**
/// мостом, так что машина посреди перекрёстка с мостом рисовалась бы прямо на
/// его асфальте.
struct BridgeDeck<'a> {
    points: &'a [Vec2],
    /// Полуширина полотна с бортиком (внешняя кромка нарисованного) плюс
    /// клиренс.
    reach: f32,
    min: Vec2,
    max: Vec2,
}

impl<'a> BridgeDeck<'a> {
    fn of(road: &'a RoadLine) -> Option<Self> {
        if !road.bridge || road.points.is_empty() {
            return None;
        }
        let reach = road.curb_reach() + JUNCTION_CLEARANCE;
        let (min, max) = road.points.iter().fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(min, max), &point| (min.min(point), max.max(point)),
        );
        Some(Self {
            points: &road.points,
            reach,
            min: min - reach,
            max: max + reach,
        })
    }

    /// Может ли полотно задеть ряд вдоль этой ломаной, — префильтр по рамкам,
    /// чтобы на каждое место не проверять все мосты города.
    fn near(&self, points: &[Vec2], margin: f32) -> bool {
        points.windows(2).any(|pair| {
            let (lo, hi) = (pair[0].min(pair[1]), pair[0].max(pair[1]));
            lo.x <= self.max.x + margin
                && hi.x >= self.min.x - margin
                && lo.y <= self.max.y + margin
                && hi.y >= self.min.y - margin
        })
    }

    fn covers(&self, place: Vec2) -> bool {
        match self.points {
            [single] => place.distance(*single) < self.reach,
            points => points
                .windows(2)
                .any(|pair| distance_to_segment(place, pair[0], pair[1]) < self.reach),
        }
    }
}

/// Насколько густо занимать места вдоль улицы: базовая доля (ползунок
/// `CarStyle::occupancy`, один на весь город) и застройка вокруг, которая её
/// домножает. Два числа, но одно решение, поэтому и один аргумент.
pub struct Density<'a> {
    base: f32,
    districts: &'a Districts,
}

/// Где ряду стоять нельзя: перекрёстки этой улицы и мосты рядом с ней.
pub struct Clearings<'a> {
    junctions: &'a [Break],
    /// Перекрёстки с проезжими частями: разрывы разметки этой улицы у узлов
    /// проезжих частей (`junctions::marking_breaks` по `is_carriageway`),
    /// тупики — с нулевым вылетом, их правило не касается.
    crossed: &'a [Break],
    /// Ближайшая к ряду линия разметки, сплошная у перекрёстка.
    line: Option<SolidLine>,
    decks: &'a [&'a BridgeDeck<'a>],
}

/// Линия разметки, ближайшая к ряду у бордюра, — та, что у перекрёстка
/// становится сплошной (`roads/paint.rs`, [`paint::APPROACH`]).
#[derive(Clone, Copy)]
struct SolidLine {
    /// От кромки проезжей части до линии, м: кромка и крайняя полоса.
    from_kerb: f32,
    /// Осевая двусторонней улицы — сплошная по обе стороны узла; линия
    /// полос — только на подходе к нему по ходу своих полос.
    both: bool,
}

impl SolidLine {
    /// Линия у бордюра улицы `road`: у улицы в одну полосу линий нет, как и у
    /// грунтовой и у `lane_markings=no`. У двусторонней в две-три полосы
    /// ближняя к бордюру — осевая, у широкой и у односторонней — линия полос.
    fn of(road: &RoadLine) -> Option<Self> {
        if !road.lane_markings || road.is_unpaved_street() {
            return None;
        }
        let lanes = lane_count(road);
        if lanes < 2 {
            return None;
        }
        let lane = (road.width - 2.0 * EDGE_WIDTH) / f32::from(lanes);
        Some(Self {
            from_kerb: EDGE_WIDTH + lane,
            both: !road.oneway && lanes <= 3,
        })
    }
}

/// Бордюр, вдоль которого стоит ряд.
#[derive(Clone, Copy)]
pub struct Kerb<'a> {
    /// Знак поперечной `direction.perp()`: `-1` — правая сторона по ходу way.
    side: f32,
    /// Куда смотрит нос: `1` — по ходу way, `-1` — против.
    heading: f32,
    /// Где на этой стороне стоят: у бордюра на полосе и в карманах
    /// (`roads::pockets`).
    stand: &'a Kerbside,
}

/// Место `place` чисто от разрыва ряда `junction` — один тест на ряд улицы
/// ([`park_along`]) и ряд дворового проезда (`yard::park_drive`), которым
/// [`RowBreaks::of`] назначает одни и те же разрывы. Расстояние — вдоль
/// улицы и от кузова, а не от центра машины: поперёк место отнесено к
/// бордюру, и по прямой до узла выходило больше, чем вдоль. Вторая
/// дизъюнкция — запас по прямой [`BREAK_BEND_SLACK_HALF_WIDTHS`]: на изломе
/// узел за поворотом проецируется на звено коротко, и без запаса он
/// вычёркивал бы места на всей длине прямой до него.
pub fn clear_of_break(
    junction: &Break,
    place: Vec2,
    direction: Vec2,
    shape: CarShape,
    half_road: f32,
) -> bool {
    (place - junction.at).dot(direction).abs() - shape.length() / 2.0
        >= junction.reach + JUNCTION_CLEARANCE
        || place.distance(junction.at)
            > junction.reach + JUNCTION_CLEARANCE + half_road * BREAK_BEND_SLACK_HALF_WIDTHS
}

/// Ряд вдоль одной стороны: шагом [`CAR_PITCH`] по **всей** ломаной улицы, со
/// сдвигом от кромки поперёк и с пропусками. `half_road` — полуширина
/// проезжей части, от которой ряд и отступает: отступ считается по габариту
/// **этой** машины, так что фургон стоит к бордюру так же вплотную, как седан.
///
/// Шаг идёт по дуговой координате целой улицы, а не по каждому её звену
/// порознь: звено ломаной в городе сплошь и рядом короче двух отступов, и
/// пошаговый обход `windows(2)` выбрасывал их целиком (в кеше Тулы — половину
/// сегментов и треть длины), а на каждой вершине сбрасывал шаг, отчего ряд то
/// рвался, то удваивался.
///
/// [`Density`] — базовая доля занятых мест (ползунок `CarStyle`) и застройка,
/// которая её домножает; квартал перечитывается раз в [`DISTRICT_STEP`]
/// метров, потому что одна улица может выйти из частного сектора в микрорайон,
/// и плотность обязана смениться там же, где меняется город.
pub(crate) fn park_along(
    cars: &mut Vec<Car>,
    points: &[Vec2],
    half_road: f32,
    kerb: Kerb,
    clearings: &Clearings,
    density: &Density,
    rng: &mut Lcg,
) {
    let (along, total) = arclengths(points);
    if total <= 2.0 * END_MARGIN {
        return;
    }
    // последняя **поставленная** машина этой стороны — точка и её длина: на
    // изломе внутренний ряд сжимается, и место, наехавшее на соседа,
    // пропускается. Проверка по мировому расстоянию, а не по дуговой
    // координате, — она ловит и излом, и любую другую кривизну. Длина нужна
    // потому, что два кузова длиной `a` и `b`, стоящие носом к корме,
    // перекрываются ближе полусуммы; пока габарит был один, полусумма и была
    // этой единственной длиной, а с пятью типами хэтчбек за фургоном проезжал
    // проверку с наложением до 0.7 м
    let mut last: Option<(Vec2, f32)> = None;
    // застройка вокруг, перечитываемая раз в [`DISTRICT_STEP`] метров улицы, —
    // дуговая координата прошлого чтения и его доля занятости
    let mut around: Option<(f32, f32)> = None;
    let mut step = END_MARGIN;
    while step <= total - END_MARGIN {
        let at = step;
        step += CAR_PITCH;
        let Some((point, direction)) = place_on_path(points, &along, at) else {
            continue;
        };
        // тип кузова выбирается до места, а не после: отступ от кромки идёт от
        // габарита именно этой машины, и у фургона он свой
        let shape = CarShape::from_share(rng.next_f32());
        let across = direction.perp() * kerb.side;
        // в кармане бордюр отодвинут на его ширину; мимо кармана, где у
        // бордюра не стоят, места нет
        let pocket = kerb.stand.pocket_at(at).is_some();
        let offset =
            half_road + if pocket { POCKET_WIDTH } else { 0.0 } - CURB_GAP - shape.width() / 2.0;
        let place = point + across * offset;
        // перекрёсток проезжих частей: кузов — не ближе
        // [`CROSSED_EDGE_CLEARANCE`] к кромке пересекаемой, и не вдоль
        // сплошной подхода, если до неё меньше [`SOLID_LINE_CLEARANCE`]
        // (ПДД 12.4). Карман отодвигает машину от линии на свою ширину
        let line_gap = clearings.line.map(|line| {
            let gap =
                line.from_kerb - CURB_GAP - shape.width() + if pocket { POCKET_WIDTH } else { 0.0 };
            (line, gap)
        });
        let clear_of_crossing = |junction: &Break| {
            if junction.reach <= 0.0 {
                return true;
            }
            let ahead = (junction.at - place).dot(direction);
            let mut keep = junction.reach - JUNCTION_MARGIN + CROSSED_EDGE_CLEARANCE;
            if let Some((line, gap)) = line_gap
                && gap < SOLID_LINE_CLEARANCE
                && (line.both || ahead * kerb.heading > 0.0)
            {
                keep = keep.max(junction.reach + paint::APPROACH);
            }
            ahead.abs() - shape.length() / 2.0 >= keep
                || place.distance(junction.at) > keep + half_road * BREAK_BEND_SLACK_HALF_WIDTHS
        };
        if !(pocket || kerb.stand.lane)
            || !clearings
                .junctions
                .iter()
                .all(|junction| clear_of_break(junction, place, direction, shape, half_road))
            || !clearings.crossed.iter().all(clear_of_crossing)
            || clearings.decks.iter().any(|deck| deck.covers(place))
        {
            continue;
        }
        if last.is_some_and(|(previous, previous_length)| {
            previous.distance(place) < (previous_length + shape.length()) / 2.0
        }) {
            continue;
        }
        // квартал читается по осевой, а не по месту у бордюра: полтора метра
        // поперёк улицы застройку не меняют. Бросок кости от множителя не
        // зависит и остаётся на месте — поток ГПСЧ у ряда тот же, что и был,
        // меняется только то, какие из мест выживают
        let fill = match around {
            Some((read_at, fill)) if at - read_at < DISTRICT_STEP => fill,
            _ => {
                let fill = density.districts.fill_at(point);
                around = Some((at, fill));
                fill
            }
        };
        if rng.next_f32() >= (density.base * fill).clamp(0.0, 1.0) {
            continue;
        }
        // запоминается место **до** поперечной небрежности ниже, и это
        // сознательно: `PARK_SLOP` разыгрывается после проверки, а перенести
        // его бросок выше — сдвинуть поток ГПСЧ и переставить весь город.
        // Сдвиг идёт поперёк ряда и не более чем на 0.12 м, так что вдоль
        // улицы он почти ничего не значит; тест на изломе держит этот допуск
        last = Some((place, shape.length()));
        // машину ставят руками, и на снимке города ни один ряд не выровнен по
        // линейке: колокол `bell4` даёт мелкую небрежность у большинства и
        // заметный перекос у единиц
        let skew = Rot2::degrees(rng.bell4() * PARK_SKEW_DEGREES);
        cars.push(Car {
            at: place + across * (rng.bell4() * PARK_SLOP),
            along: skew * (direction * kerb.heading),
            color: body::color_from_share(rng.next_f32()),
            shape,
        });
    }
}

// Сам обход по дуговой координате (`arclengths` + `place_on_path`) живёт в
// `map::along`: тот же шаг понадобился вагонам, а второй его копии — ровно
// того, от чего этот слой уходил, — здесь быть не должно.
