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
use crate::map::meshing::MeshBuilder;
use crate::map::osm::parse::{DrawBuildout, RawOsm};
use crate::map::osm::{MapData, RoadLine, TrafficSide};
use crate::map::parallel::{self, in_parallel};
use crate::map::parking::ParkingLayout;
use crate::map::roads::network::{RoadNetwork, RoadNodes};
use crate::map::roads::pockets::{self, KerbLots};
use crate::map::roads::shape::{RoadShape, RoadShapeOnMap};
use crate::map::roads::tapers::Tapers;
use crate::map::roads::{Drawn, axis};
use crate::map::shadow;
use crate::map::surface::{LayerCost, LayerMaterials, LayerMesh, MaterialSpec, swap_layers};
use crate::map::z::Z_CAR;
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use crate::settings::{CAR_DETAIL_MAX_ZOOM, CAR_MAX_ZOOM, CAR_SILHOUETTE_MAX_ZOOM};

pub mod body;
pub(crate) mod district;
mod rails;
mod street;
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
pub(crate) use street::place;
pub use street::{CarPlacement, ParkedCars, seed_parked_cars};
// Раскладка ряда и стоянок — в `street`; ре-экспорты держат прежние имена
// здесь, для детей модуля (`tests`, `yard`) и внешних путей `cars::…`.
pub use street::{
    CAR_PITCH, CURB_GAP, DISTRICT_STEP, END_MARGIN, JUNCTION_CLEARANCE, LOT_OCCUPANCY_LARGE,
    LOT_OCCUPANCY_SMALL, PARK_SKEW_DEGREES, PARK_SLOP, clear_of_break, lot_occupancy, park_all,
};
pub(in crate::map::cars) use street::{park_cars, park_on};
// Имена, которых в пространстве `cars` ждут не код модуля, а glob тестов
// (`tests.rs`: `use super::*`) и док-ссылка wagons.rs; сама раскладка зовёт
// их из `street`, поэтому для родителя они «неиспользуемые».
#[allow(unused_imports)]
use crate::map::osm::{PolyArea, model::distance_to_segment};
#[allow(unused_imports)]
use crate::map::roads::{Axis, paint};
#[allow(unused_imports)]
use crate::map::seed::Lcg;
#[allow(unused_imports)]
pub(crate) use street::park_along;

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
pub(crate) struct CarLayerTag;

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
/// `None` — слоя нет (тумблер снят, зум за последней ступенью или сырой OSM
/// второго уровня: машины — наша достройка, в данных их нет). Одно условие
/// на адаптер, дверь слоя и меш кузовов: снятый слой не расставляет ничего.
fn shown_detail(bucket: CarZoomBucket, style: CarStyle, raw: RawOsm) -> Option<CarDetail> {
    detail_for(bucket.index).filter(|_| style.visible && !raw.disables(DrawBuildout::ParkedCars))
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
    let layout = ParkingLayout::new(map);
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
        let (_, report) = mesh_parked(
            CarZoomBucket::at(step),
            CarStyle::default(),
            map.knobs.raw,
            &parked,
            false,
        );
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
pub(crate) fn rebuild_cars(
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
    // снятый слой не расставляет ничего — ни каркаса, ни разрывов
    let placed = if shown_detail(*bucket, *style, map.knobs.raw).is_some() {
        placement.refresh(*style, road_shape.0, &map, &layout)
    } else {
        false
    };
    let (layers, report) = mesh_parked(*bucket, *style, map.knobs.raw, &placement.parked, placed);
    swap_layers(
        &mut commands,
        &mut meshes,
        &materials,
        layers,
        CarLayerTag,
        &existing,
    );
    info!("{report}");
}

/// Что вышло из сборки слоя машин — значением, а не только строкой в логе.
///
/// `detail` — ступень подробности кузова, и `None` в ней значит «слоя нет»:
/// зум ушёл за последнюю ступень, выключен тумблер или включён сырой OSM
/// второго уровня. Остальные счётчики тогда нули, и это не заглушка: сборка
/// в таком случае действительно не идёт.
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
/// её зовёт и игра (через [`rebuild_cars`]), и тест. Выключенный тумблер,
/// ушедший за последнюю ступень зум и сырой OSM второго уровня — это пустой
/// список слоёв, а не ранний выход у вызывающего: деспавн в адаптере
/// безусловен, и второй дороги, на которой
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
    let raw = map.knobs.raw;
    if shown_detail(bucket, style, raw).is_none() {
        return mesh_parked(bucket, style, raw, &ParkedCars::default(), false);
    }
    let parked = park_on(style, drawn, map, layout, std::time::Instant::now());
    mesh_parked(bucket, style, raw, &parked, true)
}

/// Меш кузовов по готовой расстановке — то, что стоит пересечение порога
/// зума, когда расстановка взята из [`CarPlacement`]. `raw` — сырой OSM:
/// второй уровень снимает слой, как тумблер и дальняя ступень, — состоянием
/// отчёта (`detail: None`, строка `cars: hidden`), а не нулём в счётчике.
/// `placed` — расставлены ли машины этой же сборкой: тогда в отчёт идёт и её
/// время.
pub(crate) fn mesh_parked(
    bucket: CarZoomBucket,
    style: CarStyle,
    raw: RawOsm,
    parked: &ParkedCars,
    placed: bool,
) -> (Vec<LayerMesh>, CarReport) {
    let Some(detail) = shown_detail(bucket, style, raw) else {
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
///
/// `occupancy` — ползунок `Occupancy` игры (`None` — его умолчание): полная
/// занятость ставит машину на каждое разрешённое место, и правило, которое
/// место запрещает, видно на снимке, а не на броске кости.
pub fn mesh_map_cars(
    map: &MapData,
    shape: &RoadShape,
    layout: &ParkingLayout,
    occupancy: Option<f32>,
) -> (Vec<LayerMesh>, CarReport) {
    let drawn = Drawn::nodal(map, shape);
    let defaults = CarStyle::default();
    mesh_cars(
        CarZoomBucket::at(0),
        CarStyle {
            occupancy: occupancy.unwrap_or(defaults.occupancy),
            ..defaults
        },
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
