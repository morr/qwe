//! Машины, стоящие вдоль улиц.
//!
//! На снимке города с воздуха машины — второй по узнаваемости признак после
//! самих крыш: улица без единого автомобиля читается как чертёж, чем бы её ни
//! красили. Их и не хватало карте больше всего после того, как дома получили
//! материал кровли, оборудование на ней и настоящие тени.
//!
//! Всё, что здесь есть, — **декорация**: машины не попадают ни в навмеш, ни в
//! симуляцию, пешки ходят сквозь них. Это сознательно: припаркованный ряд
//! вдоль каждой улицы съел бы тротуары, по которым идёт вся толпа.
//!
//! Паркуются вдоль **всякой** проезжей части, а не только вдоль магистралей:
//! отбор идёт тем же [`is_carriageway`](crate::map::roads::is_carriageway), которым `map::roads` решает, где
//! рисовать тротуар и разметку. Ширина в `RoadLine` — рисовальная константа
//! класса, а не измеренная ширина улицы, так что порог по ней читается не как
//! «узкая улица», а как «не магистраль»; на снимке города плотнее всего
//! запаркованы как раз жилые кварталы.
//!
//! **А насколько плотно — решает застройка вокруг** ([`district`]): в квартале
//! частных домов машин вчетверо меньше обычного, в микрорайоне — на 15 %
//! больше. Множитель один на оба источника машин, и ряд у бордюра, и
//! размеченную стоянку: наблюдение про частный сектор — одно, а то, что во
//! дворе он выражается стоянкой, а вдоль улицы рядом, — деталь укладки.
//!
//! Расстановка детерминирована (ГПСЧ Лемера, засеянный первой точкой улицы),
//! так что от запуска к запуску ряд стоит одинаково. Виден он только вблизи:
//! [`CarZoomBucket`] снимает слой целиком, когда машина становится мельче
//! шести пикселей.

use std::borrow::Cow;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::map::SunOnMap;
use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::{distance_to_segment, ring_vertex_mean};
use crate::map::osm::{MapData, PolyArea, RoadLine, TrafficSide};
use crate::map::parallel::{self, in_parallel};
use crate::map::parking::{ParkingLayout, Stall};
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::pockets::{self, KerbLots, Kerbside, POCKET_WIDTH, RowBreaks};
use crate::map::roads::shape::{RoadShape, RoadShapeOnMap};
use crate::map::roads::tapers::Tapers;
use crate::map::roads::{Axis, Drawn, axis};
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::shadow;
use crate::map::surface::{LayerCost, LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use crate::settings::{CAR_DETAIL_MAX_ZOOM, CAR_MAX_ZOOM, CAR_SILHOUETTE_MAX_ZOOM, Z_CAR};

pub mod body;
pub(crate) mod district;
mod rails;
mod yard;

/// Дефолт, границы и шаг ползунка занятости мест ([`CarStyle::occupancy`]) —
/// какая доля парковочных мест улицы занята. Сплошной ряд от перекрёстка до
/// перекрёстка выглядит как автосалон; у настоящей улицы ряд рваный.
pub const CAR_OCCUPANCY_DEFAULT: f32 = 0.45;
pub const CAR_OCCUPANCY_MIN: f32 = 0.0;
pub const CAR_OCCUPANCY_MAX: f32 = 1.0;
pub const CAR_OCCUPANCY_STEP: f32 = 0.05;

// Умолчание ползунка — внутри его же диапазона.
const _: () = {
    assert!(
        CAR_OCCUPANCY_DEFAULT >= CAR_OCCUPANCY_MIN && CAR_OCCUPANCY_DEFAULT <= CAR_OCCUPANCY_MAX
    );
};

use self::district::Districts;
pub use body::{Car, CarDetail, CarShape};

/// Шаг парковочного места вдоль улицы, м: машина плюс просвет. Одно число с
/// местом вдоль бордюра в кармане-стоянке — живёт у раскладки, машины его читают.
const CAR_PITCH: f32 = crate::map::parking::PARALLEL_LENGTH;
/// Насколько край машины отступает от кромки проезжей части, м. Ноль —
/// колесо на кромке; полметра оставляют полосу асфальта между рядом и
/// разметкой, как на настоящей улице.
const CURB_GAP: f32 = 0.5;
/// Отступ от торца нарисованной ленты, м: машина, поставленная вплотную к
/// концу улицы, свисала бы с него. Про перекрёстки этот отступ ничего не
/// знает — way кончается где угодно, а перекрёсток восстанавливается по
/// общим нодам (`map::roads::junctions`).
const END_MARGIN: f32 = 2.0;
/// Небрежность парковки: разброс угла, градусы, и поперечного отступа, м.
/// Ряд, выровненный по линейке, читается как разметка склада, а не как двор;
/// оба числа малы нарочно — машина не должна вылезти на разметку или на
/// тротуар (полметра `CURB_GAP` держит и перекос).
const PARK_SKEW_DEGREES: f32 = 2.5;
const PARK_SLOP: f32 = 0.12;
/// Насколько **кузов** не доходит до разрыва ряда, м, сверх его `Break::reach`
/// (у перекрёстка — полуширина самой широкой из сошедшихся дорог, у перехода
/// — полдлины зебры, `roads::pockets::row_breaks`): ближе пяти метров к
/// перекрёстку и переходу не паркуются, и ещё метр — зебра по правилу стоит
/// за кромкой узла на метр и тянется на четыре (`roads::node_paint`), а
/// машина, отмеренная центром в пяти метрах, вставала на неё носом. Тупик
/// приходит разрывом нулевого `reach`, и клиренс даёт в нём те же метры.
const JUNCTION_CLEARANCE: f32 = 6.0;
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
const DISTRICT_STEP: f32 = 48.0;
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
const LOT_OCCUPANCY_SMALL: f32 = 0.5;
const LOT_OCCUPANCY_LARGE: f32 = 0.12;
/// Мест на стоянке, до которых заполненность ещё [`LOT_OCCUPANCY_SMALL`], и от
/// которых уже [`LOT_OCCUPANCY_LARGE`]; между ними — по логарифму числа мест.
const LOT_SMALL_STALLS: f32 = 20.0;
const LOT_LARGE_STALLS: f32 = 400.0;

/// Ручки слоя машин: строки `Cars` и `Occupancy` секции Roads
/// (`ui/roads.rs`) — путь и машины стоят на одной проезжей части, так что
/// читаются вместе с дорогами. Пишется и по BRP, сохраняется между запусками;
/// правка пересобирает **только** слой машин ([`rebuild_cars`]), держать её в
/// [`RoadStyle`](crate::map::RoadStyle) значило бы гнать полную пересборку
/// дорожных слоёв на каждый шаг ползунка.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "cars")]
pub struct CarStyle {
    pub visible: bool,
    /// Доля занятых мест у бордюра, 0..1 — печатается процентом. **База**, а
    /// не итог: её домножает застройка вокруг ([`Districts`]), так что в
    /// частном секторе ряд вчетверо реже неё, а в микрорайоне на 15 % плотнее.
    /// Произведение прижато к единице, поэтому «100 %» на ползунке по-прежнему
    /// значит «все места заняты».
    pub occupancy: f32,
}

impl Default for CarStyle {
    fn default() -> Self {
        Self {
            visible: true,
            occupancy: CAR_OCCUPANCY_DEFAULT,
        }
    }
}

/// Слой машин — чтобы пересборка по зуму знала, что деспавнить.
///
/// `Copy` — метку получает каждый слой модуля, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct CarLayerTag;

/// Ступени зума слоя машин: не размер машины, а **подробность кузова** —
/// вблизи стёкла и зеркала ([`CarDetail::Full`]), дальше силуэт со
/// скруглениями, ещё дальше габаритный прямоугольник, за `CAR_MAX_ZOOM`
/// слоя нет вовсе. Машина в 4.4 м на 0.8 м/px это пять с половиной
/// пикселей; ещё дальше ряд превращается в мерцающий пунктир вдоль улицы.
///
/// Ступени идут только на убывание вершин, и слой строится на **весь**
/// город, а не на кадр: подробный кузов на всех двадцати двух тысячах машин
/// — это разовый хитч на пересечении порога, той же природы, что у рельсов
/// (`RAIL_LODS`), и порог `CAR_DETAIL_MAX_ZOOM` выбран так, чтобы платить за
/// него только там, где деталь видно.
pub enum CarLods {}

impl ZoomLods for CarLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [
            CAR_DETAIL_MAX_ZOOM,
            CAR_SILHOUETTE_MAX_ZOOM,
            CAR_MAX_ZOOM,
            f32::INFINITY,
        ]
        .into_iter()
    }
}

pub type CarZoomBucket = ZoomBucket<CarLods>;

/// Подробность кузова на ступени зума; последняя ступень — слоя нет.
///
/// Публична ради витрины (`examples/demos/car_gallery`): её ручка `Detail`
/// показывает те же ступени, что выбирает зум, и вторая копия этой таблицы
/// разошлась бы с игрой на первой же новой ступени.
pub fn detail_for(bucket: usize) -> Option<CarDetail> {
    match bucket {
        0 => Some(CarDetail::Full),
        1 => Some(CarDetail::Silhouette),
        2 => Some(CarDetail::Block),
        _ => None,
    }
}

/// Подробность, с которой слой рисуется при этой ступени и этих настройках;
/// `None` — слоя нет (тумблер снят или зум за последней ступенью). Одно
/// условие на адаптер, дверь слоя и меш кузовов: снятый слой не расставляет
/// ничего.
fn shown_detail(bucket: CarZoomBucket, style: CarStyle) -> Option<CarDetail> {
    detail_for(bucket.index).filter(|_| style.visible)
}

/// Замер слоя машин без мира — для офлайн-бенча, по той же причине, что и
/// `buildings::measure_layers`. Ручки берутся игровые: бенч меряет тот слой,
/// который город строит на дефолтных настройках, а не произвольный.
///
/// Отдаёт число машин и цену теми же [`LayerCost`], что зданиевые слои:
/// миллисекунды — ровно то, ради чего замер и выносили из живого приложения, а
/// `drawn`, `breaks` и `placement` отдельными строками: каркас дорог и
/// расстановка платятся один раз на город и на правку занятости или формы
/// дорог ([`CarPlacement`]), а не на пересечение порога зума.
///
/// Кузов меряется на **каждой** ступени подробности, своей строкой, и эти
/// строки — ровно цена пересечения порога: разница между ними и есть то, ради
/// чего заведён [`CarLods`], и она должна быть видна в тех же числах, что и
/// цена зданиевых слоёв.
pub fn measure_cars(map: &MapData) -> (usize, Vec<LayerCost>) {
    // каркас — тот же, что строит расстановка ([`park_all`]), своей строкой:
    // узлы, оси, клинья и стоянки — цена, которую ряд платит до первой машины
    let shape = RoadShape::default();
    let started = std::time::Instant::now();
    let drawn = Drawn::nodal(map, &shape);
    let drawn_took = started.elapsed();
    // раскладка стоянок — вход слоя, не его цена: в игре она считается на
    // загрузку мира (`map::spawn`), и без неё бенч не считал бы машины на
    // стоянках, которые игра рисует
    let layout = ParkingLayout::new(&map.parking, &map.roads, map.traffic_side);
    let mut costs = vec![LayerCost {
        name: "drawn",
        vertices: 0,
        elapsed: drawn_took,
    }];
    // расстановка — один раз, как в игре ([`CarPlacement`]); разрывы — её
    // долей своей строкой
    let parked = park_on(
        CarStyle::default(),
        &drawn,
        map,
        &layout,
        std::time::Instant::now(),
    );
    costs.push(LayerCost {
        name: "breaks",
        vertices: 0,
        elapsed: parked.breaks_took,
    });
    costs.push(LayerCost {
        name: "placement",
        vertices: 0,
        elapsed: parked.took,
    });
    // меш кузовов на каждой ступени — ровно то, что стоит пересборка слоя на
    // пересечении порога зума
    for (name, step) in [("cars full", 0), ("cars silhouette", 1), ("cars block", 2)] {
        let (_, report) = mesh_parked(CarZoomBucket::at(step), CarStyle::default(), &parked, false);
        costs.push(LayerCost {
            name,
            vertices: report.vertices,
            elapsed: report.elapsed,
        });
    }
    (parked.cars.len(), costs)
}

/// Когда пересобирать слой припаркованных машин: своя ступень зума, тумблер и
/// ручка занятости, форма дорог и осевшее солнце.
///
/// Форма дорог (`RoadShapeOnMap`) здесь потому, что ряд стоит по **той же**
/// осевой, по которой рисуется асфальт, и рвётся на тех же клиньях: допуск
/// оси и длина клина двигают машины вместе с лентой. `RoadStyle` — нет: его
/// тумблеры кладут или снимают слои ленты, а карман решает тег `sidewalk=*`
/// на самой дороге (`pockets::kerb_parking`), не тумблер тротуаров.
///
/// **Условие одно, регистрация одна** (см. `crate::map::roads::rebuilds_on`).
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<CarZoomBucket>
        .or_else(retuned::<CarStyle>)
        .or_else(retuned::<RoadShapeOnMap>)
        .or_else(retuned::<SunOnMap>)
}

/// Пересборка слоя машин: на входе в мир и на пересечении порога зума.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_cars(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<CarZoomBucket>,
    style: Res<CarStyle>,
    // форма дорог: ряд стоит по той же ломаной, по которой `map::roads`
    // кладёт ленту асфальта
    road_shape: Res<RoadShapeOnMap>,
    map: Res<MapData>,
    layout: Res<ParkingLayout>,
    mut placement: ResMut<CarPlacement>,
    existing: Query<Entity, With<CarLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    // снятый слой не расставляет ничего — ни каркаса, ни разрывов
    let placed = if shown_detail(*bucket, *style).is_some() {
        placement.refresh(*style, road_shape.0, &map, &layout)
    } else {
        false
    };
    let (layers, report) = mesh_parked(*bucket, *style, &placement.parked, placed);
    spawn_layers(&mut commands, &mut meshes, &materials, layers, CarLayerTag);
    info!("{report}");
}

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
fn park_all(
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
fn park_on(
    style: CarStyle,
    drawn: &Drawn,
    map: &MapData,
    layout: &ParkingLayout,
    started: std::time::Instant,
) -> ParkedCars {
    // сырой OSM, второй уровень: машины — наша достройка, их нет вовсе
    if map.knobs.raw.draws_raw() {
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
/// кеш сбрасывает [`forget_parked_cars`].
#[derive(Resource, Default)]
pub struct CarPlacement {
    key: Option<(f32, RoadShape)>,
    parked: ParkedCars,
}

impl CarPlacement {
    /// Расставить заново, если ключ поехал; `true` — если расставили.
    fn refresh(
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

/// Сброс расстановки при входе в мир: у нового города свои улицы, а ключ
/// кеша про город не знает.
pub fn forget_parked_cars(mut placement: ResMut<CarPlacement>) {
    *placement = CarPlacement::default();
}

/// Что вышло из сборки слоя машин — значением, а не только строкой в логе.
///
/// `detail` — ступень подробности кузова, и `None` в ней значит «слоя нет»:
/// зум ушёл за последнюю ступень или выключен тумблер. Остальные счётчики тогда
/// нули, и это не заглушка: сборка в таком случае действительно не идёт.
///
/// `elapsed` меряется внутри сборки, потому что время тратится там; печатает его
/// адаптер — `info!` на macOS ещё и меряет не то, потому что App Nap решает, как
/// быстро идёт сборка. `breaks_took` — доля того же времени, ушедшая на поиск
/// перекрёстков. Когда расстановка взята из [`CarPlacement`] (пересечение
/// порога зума), `placed` ложно, а `elapsed` и `breaks_took` — только меш и
/// ноль: расстановка в этой сборке не шла.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CarReport {
    /// Сколько машин расставлено — и вдоль бордюров, и на размеченных стоянках.
    pub cars: usize,
    /// Подробность кузова на этой ступени зума; `None` — слой не строился.
    pub detail: Option<CarDetail>,
    /// Сколько найдено перекрёстков, по которым рвутся ряды.
    pub junctions: usize,
    pub vertices: usize,
    /// Расставлены ли машины этой сборкой (а не взяты из кеша).
    pub placed: bool,
    pub elapsed: std::time::Duration,
    pub breaks_took: std::time::Duration,
}

impl std::fmt::Display for CarReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            cars,
            detail,
            junctions,
            vertices,
            placed,
            elapsed,
            breaks_took,
        } = self;
        match detail {
            Some(detail) if *placed => write!(
                f,
                "cars: {cars} parked, {detail:?} ({vertices} verts) in {elapsed:?} \
                 (junctions {junctions}, {breaks_took:?})"
            ),
            Some(detail) => write!(
                f,
                "cars: {cars} parked (placement reused), {detail:?} ({vertices} verts) \
                 in {elapsed:?}"
            ),
            None => write!(f, "cars: hidden"),
        }
    }
}

/// Слой машин целиком: разрывы на перекрёстках, застройка вокруг, расстановка
/// вдоль улиц, заполнение стоянок и меш кузовов с тенями.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_cars`]), и тест. Выключенный тумблер и
/// ушедший за последнюю ступень зум — это пустой список слоёв, а не ранний выход
/// у вызывающего: деспавн в адаптере безусловен, и второй дороги, на которой
/// можно его забыть, нет. Сборка при этом не идёт вовсе — ни разрывов, ни
/// расстановки: снятый слой не должен стоить дороже, чем стоил ранний возврат.
///
/// `drawn` — подготовленные дороги (`roads::Drawn::nodal`): ряд стоит по той
/// же узловой оси (`Axis::Nodal`), по которой `map::roads` кладёт ленту
/// асфальта, рвётся на тех же клиньях и обходит те же стоянки.
pub fn mesh_cars(
    bucket: CarZoomBucket,
    style: CarStyle,
    drawn: &Drawn,
    map: &MapData,
    layout: &ParkingLayout,
) -> (Vec<LayerMesh>, CarReport) {
    if shown_detail(bucket, style).is_none() {
        return mesh_parked(bucket, style, &ParkedCars::default(), false);
    }
    let parked = park_on(style, drawn, map, layout, std::time::Instant::now());
    mesh_parked(bucket, style, &parked, true)
}

/// Меш кузовов по готовой расстановке — то, что стоит пересечение порога
/// зума, когда расстановка взята из [`CarPlacement`]. `placed` — расставлены
/// ли машины этой же сборкой: тогда в отчёт идёт и её время.
pub fn mesh_parked(
    bucket: CarZoomBucket,
    style: CarStyle,
    parked: &ParkedCars,
    placed: bool,
) -> (Vec<LayerMesh>, CarReport) {
    let Some(detail) = shown_detail(bucket, style) else {
        return (
            Vec::new(),
            CarReport {
                cars: 0,
                detail: None,
                junctions: 0,
                vertices: 0,
                placed: false,
                elapsed: std::time::Duration::ZERO,
                breaks_took: std::time::Duration::ZERO,
            },
        );
    };
    let started = std::time::Instant::now();
    let builder = mesh_bodies(&parked.cars, detail);
    let meshed = started.elapsed();
    let report = CarReport {
        cars: parked.cars.len(),
        detail: Some(detail),
        junctions: parked.junctions,
        vertices: builder.vertex_count(),
        placed,
        elapsed: if placed { parked.took + meshed } else { meshed },
        breaks_took: if placed {
            parked.breaks_took
        } else {
            std::time::Duration::ZERO
        },
    };
    // слой с блендингом: тень машины полупрозрачна, кузов — нет
    (
        vec![LayerMesh::new(builder, Z_CAR, "cars", MaterialSpec::Blend)],
        report,
    )
}

/// Слой машин по готовой карте на ближней ступени зума — дверь для витрины
/// `examples/demos/roads` (`ROADS_CARS=1`): у неё своя карта на пример, и
/// подготовленные дороги ([`Drawn`]) она строить не умеет. Тот же
/// [`mesh_cars`], что зовёт игра, с той же формой дорог, что у ленты.
pub fn mesh_map_cars(
    map: &MapData,
    shape: &RoadShape,
    layout: &ParkingLayout,
) -> (Vec<LayerMesh>, CarReport) {
    let drawn = Drawn::nodal(map, shape);
    mesh_cars(
        CarZoomBucket::at(0),
        CarStyle::default(),
        &drawn,
        map,
        layout,
    )
}

/// Меш припаркованных рядов по готовому срезу улиц — дверь наружу для витрины
/// `examples/demos/car_gallery` (геометрию наружу отдают она и [`drawn_axes`] —
/// оси, по которым витрина кладёт асфальт под ряд; второй выход,
/// [`measure_cars`], отдаёт не меш, а его цену).
///
/// Открыта затем, что клетки витрины обязаны строиться **теми же вызовами**,
/// что и город: про шаг, палитру, разрывы на перекрёстках и правило излома
/// витрина не знает ничего и знать не должна — иначе она показывает свою
/// геометрию, а не игровую.
///
/// `shape` — форма дорог, та же, с которой витрина берёт [`drawn_axes`] под
/// асфальт: осевая у ленты и у ряда обязана быть одна; `detail` — ступень
/// подробности, которую в игре выдаёт зум, а витрина показывает все три рядом.
///
/// Домов у витрины нет вовсе, и пустой [`Districts`] здесь не заглушка, а
/// честное «квартала вокруг не прочесть»: множитель тогда ровно 1, и клетки
/// показывают правило укладки, не смешанное с правилом плотности.
pub fn cars_mesh(
    roads: &[RoadLine],
    style: CarStyle,
    shape: RoadShape,
    traffic: TrafficSide,
    detail: CarDetail,
) -> MeshBuilder {
    // разрывы — игровые (`row_breaks`: проезды тоже рвут ряд); сети и точек
    // дорог у витрины нет, как нет их и у её осей (`drawn_axes`), а соседство
    // way'ев по узлу `RoadNodes` выводит сам
    let tapers = Tapers::of_map(roads, &RoadNetwork::default(), shape.taper());
    let shared = RoadNodes::new(roads);
    let junctions = pockets::row_breaks(roads, &shared, &tapers, &[]);
    let districts = Districts::new(&[]);
    mesh_bodies(
        &park_cars(
            roads,
            &shared,
            &junctions,
            style,
            &drawn_axes(roads, &shape),
            traffic,
            &districts,
            &KerbLots::new(&[]),
        ),
        detail,
    )
}

/// Оси дорог так, как их рисует `map::roads`, — для среза без собранной сети
/// (замер, витрина, тесты): улицы склеиваются здесь же. Открыта ради витрины:
/// её асфальт лежит по этим же осям, а не по своему сглаживанию, — иначе ряд
/// стоял бы не на своей ленте.
pub fn drawn_axes<'a>(roads: &'a [RoadLine], shape: &RoadShape) -> Vec<Cow<'a, [Vec2]>> {
    let mut nodes = RoadNodes::new(roads);
    axis::street_axes(roads, &[], &RoadNetwork::default(), &mut nodes, shape).paths
}

/// Ряды вдоль всех улиц, годных под парковку.
///
/// `junctions` — разрывы ряда ([`pockets::row_breaks`]), индексированы по
/// номеру дороги **во входном срезе**, поэтому `roads` — весь срез карты, а не
/// отфильтрованный список парковочных; `shared` — их узлы (карман через торец
/// way идёт на продолжение улицы), `axes` — по тому же индексу, нарисованные
/// оси дорог.
#[allow(clippy::too_many_arguments)]
fn park_cars(
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
fn lot_occupancy(stalls: usize) -> f32 {
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
struct Density<'a> {
    base: f32,
    districts: &'a Districts,
}

/// Где ряду стоять нельзя: перекрёстки этой улицы и мосты рядом с ней.
struct Clearings<'a> {
    junctions: &'a [Break],
    decks: &'a [&'a BridgeDeck<'a>],
}

/// Бордюр, вдоль которого стоит ряд.
#[derive(Clone, Copy)]
struct Kerb<'a> {
    /// Знак поперечной `direction.perp()`: `-1` — правая сторона по ходу way.
    side: f32,
    /// Куда смотрит нос: `1` — по ходу way, `-1` — против.
    heading: f32,
    /// Где на этой стороне стоят: у бордюра на полосе и в карманах
    /// (`roads::pockets`).
    stand: &'a Kerbside,
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
fn park_along(
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
        // до разрыва — вдоль улицы и от кузова, а не от центра машины: поперёк
        // место отнесено к бордюру, и по прямой до узла выходило больше, чем
        // вдоль
        let clear = |junction: &Break| {
            (place - junction.at).dot(direction).abs() - shape.length() / 2.0
                >= junction.reach + JUNCTION_CLEARANCE
                || place.distance(junction.at)
                    > junction.reach + JUNCTION_CLEARANCE + half_road * BREAK_BEND_SLACK_HALF_WIDTHS
        };
        if !(pocket || kerb.stand.lane)
            || !clearings.junctions.iter().all(clear)
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

/// Меш слоя: сначала **все** тени, потом **все** кузова — тень соседней
/// машины иначе легла бы поверх кузова той, что нарисована раньше.
///
/// Длина тени — по высоте **этой** машины (у фургона она заметно длиннее) и
/// по тому же котангенсу высоты солнца, которым меряются тени домов. Сам
/// сдвиг — не место тени, а то, на сколько заметается силуэт: тень лежит под
/// машиной и тянется из-под неё ([`body::push_shadow`]).
///
/// Тени соседних машин здесь не объединяются, в отличие от зданиевых: при
/// дефолтном солнце сдвиг — метр с небольшим, а шаг ряда `CAR_PITCH` — шесть
/// метров, так что накладываться им негде; булев union на двадцать две тысячи
/// машин стоил бы дороже всего слоя. При низком солнце тени вдоль ряда
/// перекрываются и складываются в пятна двойной темноты — известная плата за
/// это решение.
///
/// **Собирается по потокам**: машины друг от друга не зависят, и ряд режется
/// на куски по числу ядер, каждый из которых кладёт свои тени и свои кузова в
/// два своих сборщика. Склейка ([`MeshBuilder::concat`]) идёт в прежнем
/// порядке — тени всех кусков, потом кузова всех кусков, — так что меш тот же
/// байт в байт, что при сборке в один поток. Это и есть цена перехода порога
/// зума машин: расстановка между порогами не пересчитывается, и на пороге
/// собираются только кузова.
///
/// Потоки — свои, общие с разбором карты ([`in_parallel`]), а не
/// `ComputeTaskPool`: переход порога держит кадр целиком, и ждать ему некого.
fn mesh_bodies(cars: &[Car], detail: CarDetail) -> MeshBuilder {
    // сдвиг на метр высоты — общий множитель слоя, а высоту прикладывает
    // каждая машина своей (`CarShape::height`)
    let stretch = shadow::offset(1.0);
    let workers = parallel::workers();
    let chunk = cars.len().div_ceil(workers).max(BODY_CHUNK_MIN);
    let (shadow_size, body_size) = body::vertices_per_car(detail);
    let build = |chunk: &[Car]| {
        // место — сразу под весь кусок: иначе вектор вершин удваивается с
        // десяток раз, и каждый раз копирует всё, что уже положено. Индексов —
        // полтора на вершину (квад), веер контура дешевле, так что с запасом
        let mut shadows = MeshBuilder::default();
        shadows.reserve(chunk.len() * shadow_size, chunk.len() * shadow_size * 3 / 2);
        let mut bodies = MeshBuilder::default();
        bodies.reserve(chunk.len() * body_size, chunk.len() * body_size * 3 / 2);
        for car in chunk {
            body::push_shadow(&mut shadows, car, stretch * car.shape.height(), detail);
            body::push_body(&mut bodies, car, detail);
        }
        (shadows, bodies)
    };
    let chunks: Vec<&[Car]> = cars.chunks(chunk).collect();
    let parts: Vec<(MeshBuilder, MeshBuilder)> = in_parallel(&chunks, |&chunk| build(chunk));
    let (shadows, bodies): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
    let ordered: Vec<MeshBuilder> = shadows.into_iter().chain(bodies).collect();
    MeshBuilder::concat(&ordered, workers)
}

/// Меньше этого машин на кусок [`mesh_bodies`] не режет: витрине и маленькому
/// городу поток на горстку машин стоит дороже, чем они сами.
const BODY_CHUNK_MIN: usize = 1024;

#[cfg(test)]
mod tests;
