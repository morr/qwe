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
//! отбор идёт тем же [`is_carriageway`], которым `map::roads` решает, где
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

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::map::along::{arclengths, place_on_path};
use crate::map::buildings::LayerCost;
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::model::{distance_to_segment, ring_vertex_mean};
use crate::map::osm::{MapData, PolyArea, RoadLine, TrafficSide};
use crate::map::parking::{ParkingLayout, Stall};
use crate::map::roads::junctions::{self, MarkingBreaks};
use crate::map::roads::{RoadSmoothing, RoadStyle, is_carriageway, smooth_path};
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{shadow_dir, shadow_length_scale};
use crate::settings::{
    CAR_DETAIL_MAX_ZOOM, CAR_MAX_ZOOM, CAR_OCCUPANCY_DEFAULT, CAR_SILHOUETTE_MAX_ZOOM, Z_CAR,
};

pub mod body;
mod district;

use self::district::Districts;
pub use body::{Car, CarDetail, CarShape};

/// Шаг парковочного места вдоль улицы, м: машина плюс просвет.
const CAR_PITCH: f32 = 6.0;
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
/// Насколько ряд не доходит до перекрёстка, м, сверх полуширины самой широкой
/// из сошедшихся дорог (`Break::reach`): ближе пяти метров к перекрёстку не
/// паркуются. Тупик приходит разрывом нулевого `reach`, и клиренс даёт в нём
/// те же пять пустых метров, что и на настоящем узле.
const JUNCTION_CLEARANCE: f32 = 5.0;
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

/// Замер слоя машин без мира — для офлайн-бенча, по той же причине, что и
/// `buildings::measure_layers`. Ручки берутся игровые: бенч меряет тот слой,
/// который город строит на дефолтных настройках, а не произвольный.
///
/// Отдаёт число машин и цену теми же [`LayerCost`], что зданиевые слои:
/// миллисекунды — ровно то, ради чего замер и выносили из живого приложения, а
/// `breaks` и `parking` отдельными строками — та же разбивка, что печатает
/// `rebuild_cars` (разрывы считаются на каждую пересборку, и это решение
/// перемеряется здесь).
///
/// Расстановка стоит своей строки, а не молчания: `park_cars` — это проход по
/// всем улицам города с бинарным поиском по дуговой координате и несколькими
/// бросками ГПСЧ на место, и от ступени подробности она не зависит, поэтому
/// меряется один раз. Без неё строка `cars *` мерила бы одну укладку меша, и
/// её миллисекунды нельзя было бы сравнить ни с логом `rebuild_cars`, ни с
/// прежним замером, где расстановка входила в общее время.
///
/// Индекс застройки ([`Districts`]) — тоже своей строкой, и по той же причине:
/// он строится на все семь с половиной тысяч домов и от ступени подробности не
/// зависит, а решение не кешировать его между пересборками держится ровно на
/// этом числе.
///
/// Кузов меряется на **каждой** ступени подробности, своей строкой: разница
/// между ними и есть то, ради чего заведён [`CarLods`], и она должна быть
/// видна в тех же числах, что и цена зданиевых слоёв.
pub fn measure_cars(
    buildings: &[PolyArea],
    roads: &[RoadLine],
    traffic: TrafficSide,
) -> (usize, Vec<LayerCost>) {
    let started = std::time::Instant::now();
    let junctions = junctions::marking_breaks(roads, is_carriageway);
    let breaks_took = started.elapsed();
    let started = std::time::Instant::now();
    let districts = Districts::new(buildings);
    let districts_took = started.elapsed();
    let started = std::time::Instant::now();
    let cars = park_cars(
        roads,
        &junctions,
        CarStyle::default(),
        RoadStyle::default().smoothing,
        traffic,
        &districts,
    );
    let parking_took = started.elapsed();
    let mut costs = vec![
        LayerCost {
            name: "breaks",
            vertices: 0,
            elapsed: breaks_took,
        },
        LayerCost {
            name: "districts",
            vertices: 0,
            elapsed: districts_took,
        },
        LayerCost {
            name: "parking",
            vertices: 0,
            elapsed: parking_took,
        },
    ];
    for (name, detail) in [
        ("cars full", CarDetail::Full),
        ("cars silhouette", CarDetail::Silhouette),
        ("cars block", CarDetail::Block),
    ] {
        let started = std::time::Instant::now();
        let builder = mesh_cars(&cars, detail);
        costs.push(LayerCost {
            name,
            vertices: builder.vertex_count(),
            elapsed: started.elapsed(),
        });
    }
    (cars.len(), costs)
}

/// Пересборка слоя машин: на входе в мир и на пересечении порога зума.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_cars(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<CarZoomBucket>,
    style: Res<CarStyle>,
    // сглаживание осевой: ряд стоит по той же ломаной, по которой `map::roads`
    // кладёт ленту асфальта
    road_style: Res<RoadStyle>,
    map: Res<MapData>,
    layout: Res<ParkingLayout>,
    existing: Query<Entity, With<CarLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    // выключенный слой проходит тем же путём, что и снятый зумом: деспавн
    // старого и никакой сборки нового — второй ветки, которая могла бы забыть
    // деспавн, нет
    let Some(detail) = detail_for(bucket.index).filter(|_| style.visible) else {
        return;
    };
    let started = std::time::Instant::now();
    // разрывы — по **всем** настоящим улицам, а не только по парковочным: ряд
    // обязан прерваться и там, где к жилой улице примыкает другая жилая.
    //
    // Считаются заново на каждую пересборку слоя, а не один раз на загрузку
    // мира: по Туле это около четверти сборки слоя машин, а весь слой —
    // проценты от зданиевого, так что кеш ради этого не окупается, и мерить
    // надо было прежде, чем его заводить. Доли, а не миллисекунды: абсолютное
    // время зависит от App Nap, перемеряет его `measure_cars` из
    // `examples/bench/map_meshing` (он печатает обе строки — `breaks` и `cars`)
    let junctions = junctions::marking_breaks(&map.roads, is_carriageway);
    let breaks_took = started.elapsed();
    // застройка вокруг — тем же проходом и с тем же сроком жизни, что и
    // разрывы: индекс на 7.6 тысячи домов дешевле, чем повод его кешировать
    let districts = Districts::new(&map.buildings);
    let mut cars = park_cars(
        &map.roads,
        &junctions,
        *style,
        road_style.smoothing,
        map.traffic_side,
        &districts,
    );
    cars.extend(fill_lots(&map.parking, &layout.0, &districts));
    let builder = mesh_cars(&cars, detail);
    let count = cars.len();
    let vertices = builder.vertex_count();
    let elapsed = started.elapsed();
    // слой с блендингом: тень машины полупрозрачна, кузов — нет
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials,
        [LayerMesh::new(builder, Z_CAR, "cars", MaterialSpec::Blend)],
        CarLayerTag,
    );
    info!(
        "cars: {count} parked, {detail:?} ({vertices} verts) in {elapsed:?} (junctions {}, {breaks_took:?})",
        junctions.junctions,
    );
}

/// Меш припаркованных рядов по готовому срезу улиц — дверь наружу для витрины
/// `examples/demos/car_gallery` (геометрию наружу отдаёт только она; второй
/// выход, [`measure_cars`], отдаёт не меш, а его цену).
///
/// Открыта затем, что клетки витрины обязаны строиться **теми же вызовами**,
/// что и город: про шаг, палитру, разрывы на перекрёстках и правило излома
/// витрина не знает ничего и знать не должна — иначе она показывает свою
/// геометрию, а не игровую.
///
/// `smoothing` — то же, с чем витрина кладёт под ряд асфальт: осевая у ленты и
/// у ряда обязана быть одна; `detail` — ступень подробности, которую в игре
/// выдаёт зум, а витрина показывает все три рядом.
///
/// Домов у витрины нет вовсе, и пустой [`Districts`] здесь не заглушка, а
/// честное «квартала вокруг не прочесть»: множитель тогда ровно 1, и клетки
/// показывают правило укладки, не смешанное с правилом плотности.
pub fn cars_mesh(
    roads: &[RoadLine],
    style: CarStyle,
    smoothing: RoadSmoothing,
    traffic: TrafficSide,
    detail: CarDetail,
) -> MeshBuilder {
    let junctions = junctions::marking_breaks(roads, is_carriageway);
    let districts = Districts::new(&[]);
    mesh_cars(
        &park_cars(roads, &junctions, style, smoothing, traffic, &districts),
        detail,
    )
}

/// Ряды вдоль всех улиц, годных под парковку.
///
/// `junctions.breaks` индексирован по номеру дороги **во входном срезе**,
/// поэтому `roads` — весь срез карты, а не отфильтрованный список
/// парковочных.
fn park_cars(
    roads: &[RoadLine],
    junctions: &MarkingBreaks,
    style: CarStyle,
    smoothing: RoadSmoothing,
    traffic: TrafficSide,
    districts: &Districts,
) -> Vec<Car> {
    let mut cars = Vec::new();
    let kerb = traffic.kerb();
    let density = Density {
        base: style.occupancy,
        districts,
    };
    let decks: Vec<BridgeDeck> = roads.iter().filter_map(BridgeDeck::of).collect();
    let mut near = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !parkable(road) {
            continue;
        }
        near.clear();
        near.extend(
            decks
                .iter()
                .filter(|deck| deck.near(&road.points, road.width)),
        );
        // осевая та же, по которой `map::roads` строит ленту: по сырым точкам
        // OSM ряд на изломе съезжает с асфальта на тротуар, потому что Chaikin
        // срезает вершину на метры. Арок здесь не бывает — `is_carriageway` их
        // отсеял, — поэтому `smooth_path`, а не `centerline`
        let centre = smooth_path(&road.points, road.width, smoothing);
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
        let sides: &[f32] = if road.oneway { &[kerb] } else { &[-1.0, 1.0] };
        for &side in sides {
            park_along(
                &mut cars,
                &centre,
                road.width / 2.0,
                Kerb {
                    side,
                    heading: if side == kerb { 1.0 } else { -1.0 },
                },
                &Clearings {
                    junctions: &junctions.breaks[index],
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

/// Улица, вдоль которой паркуются: настоящая проезжая часть — то же
/// [`is_carriageway`], которым отбирает тротуары и разметку `map::roads`, —
/// но не мост (на мосту не стоят) и не кольцо (по кольцу едут, а не
/// паркуются). Разметке мост и кольцо нужны, машинам нет, поэтому оба
/// условия здесь, а не внутри предиката.
fn parkable(road: &RoadLine) -> bool {
    is_carriageway(road) && !road.bridge && !road.roundabout
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
struct Kerb {
    /// Знак поперечной `direction.perp()`: `-1` — правая сторона по ходу way.
    side: f32,
    /// Куда смотрит нос: `1` — по ходу way, `-1` — против.
    heading: f32,
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
        let offset = half_road - CURB_GAP - shape.width() / 2.0;
        let place = point + across * offset;
        if clearings
            .junctions
            .iter()
            .any(|junction| place.distance(junction.at) < junction.reach + JUNCTION_CLEARANCE)
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
fn mesh_cars(cars: &[Car], detail: CarDetail) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let stretch = shadow_dir() * shadow_length_scale();
    for car in cars {
        body::push_shadow(&mut builder, car, stretch * car.shape.height(), detail);
    }
    for car in cars {
        body::push_body(&mut builder, car, detail);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{self, street};
    use crate::map::roads::junctions::JUNCTION_MARGIN;

    /// Большая стоянка пустее малой, и доля не выходит за свои края.
    #[test]
    fn a_bigger_lot_is_emptier() {
        assert_eq!(lot_occupancy(5), LOT_OCCUPANCY_SMALL);
        assert!((lot_occupancy(5000) - LOT_OCCUPANCY_LARGE).abs() < 1e-6);
        let shares: Vec<f32> = [20, 50, 100, 200, 400].map(lot_occupancy).to_vec();
        assert!(
            shares.windows(2).all(|pair| pair[1] < pair[0]),
            "{shares:?}"
        );
    }

    /// Расстановка по срезу целиком — так же, как её зовёт пересборка слоя.
    fn park(roads: &[RoadLine]) -> Vec<Car> {
        park_with(roads, CarStyle::default())
    }

    fn park_with(roads: &[RoadLine], style: CarStyle) -> Vec<Car> {
        park_driving(roads, style, TrafficSide::Right)
    }

    /// Домов в сценах этих тестов нет: пустой индекс даёт множитель 1, и они
    /// проверяют укладку ряда, не смешанную с плотностью квартала — та
    /// проверяется своими тестами в [`district`].
    fn park_driving(roads: &[RoadLine], style: CarStyle, traffic: TrafficSide) -> Vec<Car> {
        park_cars(
            roads,
            &junctions::marking_breaks(roads, is_carriageway),
            style,
            RoadSmoothing::Off,
            traffic,
            &Districts::new(&[]),
        )
    }

    #[test]
    fn cars_line_a_street_on_both_sides() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let cars = park(std::slice::from_ref(&road));
        assert!(!cars.is_empty());
        // ряды по обе стороны осевой, внутри проезжей части: отступ считается
        // по габариту этой машины, плюс небрежность парковки
        assert!(cars.iter().any(|car| car.at.y > 0.0));
        assert!(cars.iter().any(|car| car.at.y < 0.0));
        for car in &cars {
            let offset = road.width / 2.0 - CURB_GAP - car.shape.width() / 2.0;
            assert!(
                (car.at.y.abs() - offset).abs() <= PARK_SLOP + 0.01,
                "{}",
                car.at.y
            );
            assert!(car.at.x >= END_MARGIN - 0.01 && car.at.x <= 200.0 - END_MARGIN + 0.01);
        }
    }

    /// Таблица зума и лестница подробности — об одном и том же: у каждой
    /// ступени `CarLods` своя подробность, и только у последней её нет
    /// (слоя там не строят вовсе).
    #[test]
    fn the_zoom_table_and_the_detail_ladder_agree() {
        let steps = CarLods::max_zooms().count();
        for bucket in 0..steps - 1 {
            assert!(
                detail_for(bucket).is_some(),
                "ступень {bucket} без подробности"
            );
        }
        assert!(detail_for(steps - 1).is_none(), "последняя ступень рисует");
    }

    #[test]
    fn a_narrow_lane_and_a_bridge_stay_empty() {
        let narrow = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
        assert!(park(std::slice::from_ref(&narrow)).is_empty());

        let mut bridge = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 14.0);
        bridge.bridge = true;
        assert!(park(std::slice::from_ref(&bridge)).is_empty());
    }

    #[test]
    fn a_street_crossing_a_bridge_clears_the_row_under_the_deck() {
        // улица проходит под мостом: общей ноды нет, перекрёстка тоже
        let through = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let mut bridge = street(vec![Vec2::new(100.0, -80.0), Vec2::new(100.0, 80.0)], 16.0);
        bridge.bridge = true;
        let cars = park(&[through.clone(), bridge.clone()]);

        let cleared = bridge.curb_reach() + JUNCTION_CLEARANCE;
        for car in &cars {
            assert!(
                (car.at.x - 100.0).abs() >= cleared - 0.01,
                "машина на мосту: {}",
                car.at
            );
        }
        assert!(cars.iter().any(|car| car.at.x > 130.0));
        assert!(cars.iter().any(|car| car.at.x < 70.0));

        // первый ряд до моста стоит ровно как без него (после пропуска поток
        // ГПСЧ уже другой — так же, как за перекрёстком, и второй ряд идёт
        // по тому же потоку следом)
        let alone = park(std::slice::from_ref(&through));
        let far = |car: &&Car| car.at.x <= 100.0 - cleared - 10.0 && car.at.y < 0.0;
        let with: Vec<_> = cars.iter().filter(far).map(|car| car.at).collect();
        let without: Vec<_> = alone.iter().filter(far).map(|car| car.at).collect();
        assert_eq!(with, without);
    }

    #[test]
    fn a_residential_street_gets_a_row() {
        // 8 м — `residential`/`unclassified`: основная масса улиц города
        let residential = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 8.0);
        let cars = park(std::slice::from_ref(&residential));
        assert!(!cars.is_empty());
        // и ряды на ней не наезжают на осевую: между ними остаётся проезд
        for car in &cars {
            assert!(
                car.at.y.abs() - car.shape.width() / 2.0 > 0.5,
                "ряд на осевой: {}",
                car.at.y
            );
        }

        // 5 м — `service`, проезд: там не паркуются
        let service = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 5.0);
        assert!(park(std::slice::from_ref(&service)).is_empty());
    }

    /// Та же улица в частном секторе запаркована много реже, чем в
    /// микрорайоне, и ползунок занятости остаётся за обоими: он задаёт базу, а
    /// квартал — множитель к ней.
    #[test]
    fn the_same_street_parks_thinner_in_a_private_sector() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(600.0, 0.0)], 8.0);
        let roads = std::slice::from_ref(&road);
        let breaks = junctions::marking_breaks(roads, is_carriageway);
        let rows = |buildings: &[PolyArea]| {
            park_cars(
                roads,
                &breaks,
                CarStyle::default(),
                RoadSmoothing::Off,
                TrafficSide::Right,
                &Districts::new(buildings),
            )
            .len()
        };
        // частный сектор: одноэтажные дома по обе стороны улицы
        let houses: Vec<PolyArea> = (0..30)
            .map(|i| PolyArea {
                height: Some(3.2),
                ..fixture::building(
                    fixture::square(
                        Vec2::new(i as f32 % 15.0 * 40.0, i as f32 % 2.0 * 60.0 - 30.0),
                        5.0,
                    ),
                    Vec::new(),
                )
            })
            .collect();
        // микрорайон: те же места, но девятиэтажные секции
        let slabs: Vec<PolyArea> = (0..8)
            .map(|i| PolyArea {
                height: Some(27.0),
                ..fixture::building(
                    fixture::square(
                        Vec2::new(i as f32 % 4.0 * 150.0, i as f32 % 2.0 * 60.0 - 30.0),
                        15.0,
                    ),
                    Vec::new(),
                )
            })
            .collect();
        let (low, high) = (rows(&houses), rows(&slabs));
        assert!(low * 3 < high, "частный сектор {low}, микрорайон {high}");
        // и без домов вокруг остаётся ровно прежнее правило
        assert!(rows(&[]) > low && rows(&[]) < high);
    }

    #[test]
    fn the_row_crosses_the_bends_of_a_polyline() {
        // десять звеньев по 10 м: каждое короче прежних двух отступов, и
        // посегментный обход не ставил на них ни одной машины
        let points = (0..=10).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let with_vertices = park(std::slice::from_ref(&street(points, 12.0)));
        let straight = street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 12.0);
        let one_span = park(std::slice::from_ref(&straight));

        // вершины на прямой не меняют ничего: шаг идёт по длине улицы
        assert!(!with_vertices.is_empty());
        assert_eq!(with_vertices.len(), one_span.len());
        for (car, twin) in with_vertices.iter().zip(&one_span) {
            assert!(car.at.distance(twin.at) < 0.01, "{} / {}", car.at, twin.at);
        }
    }

    #[test]
    fn cars_never_overlap_on_a_sharp_bend() {
        // излом в 90°: с внутренней стороны ряд сжимается, и место, наехавшее
        // на соседа, обязано быть пропущено
        let road = street(
            vec![
                Vec2::new(0.0, 100.0),
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
            ],
            12.0,
        );
        let cars = park(std::slice::from_ref(&road));
        assert!(cars.len() > 10, "{}", cars.len());
        // просвет — по **полусумме** длин пары: именно на ней два кузова,
        // стоящие носом к корме, и перестают перекрываться. Небрежность
        // парковки (`PARK_SLOP`, поперёк ряда) разыгрывается после проверки и
        // может сдвинуть навстречу обе машины, отсюда допуск в два слопа
        for (index, car) in cars.iter().enumerate() {
            for other in &cars[index + 1..] {
                let gap = car.at.distance(other.at);
                let apart = (car.shape.length() + other.shape.length()) / 2.0 - 2.0 * PARK_SLOP;
                assert!(
                    gap >= apart - 0.01,
                    "{gap} м между {} ({:?}) и {} ({:?})",
                    car.at,
                    car.shape,
                    other.at,
                    other.shape
                );
            }
        }
    }

    #[test]
    fn a_junction_clears_the_row_and_the_row_resumes() {
        // Т-образный: сквозная улица с нодой в узле (перекрёсток и держится
        // на общей ноде) и примыкающая к ней в середине
        let through = street(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(200.0, 0.0),
            ],
            12.0,
        );
        let joining = street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 10.0);
        let cars = park(&[through, joining]);

        // полуширина самой широкой из сошедшихся плюс запас разметки и клиренс
        let node = Vec2::new(100.0, 0.0);
        let cleared = 10.0 / 2.0 + JUNCTION_MARGIN + JUNCTION_CLEARANCE;
        for car in &cars {
            assert!(
                car.at.distance(node) >= cleared - 0.01,
                "машина на перекрёстке: {}",
                car.at
            );
        }
        // и ряд идёт дальше по обе стороны от узла
        assert!(
            cars.iter()
                .any(|car| car.at.x > 130.0 && car.at.y.abs() < 6.0)
        );
        assert!(
            cars.iter()
                .any(|car| car.at.x < 70.0 && car.at.y.abs() < 6.0)
        );
    }

    #[test]
    fn a_way_split_in_the_middle_keeps_the_row() {
        // одна прямая улица, разрезанная на два way в общей ноде: стык двух
        // торцов — не перекрёсток, и ряд идёт сквозь него
        let first = street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 12.0);
        let second = street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let cars = park(&[first, second]);
        assert!(
            cars.iter().any(|car| (92.0..=108.0).contains(&car.at.x)),
            "ряд разорван на стыке двух way одной улицы"
        );
    }

    #[test]
    fn a_street_shorter_than_its_end_margins_stays_empty() {
        let stub = street(vec![Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0)], 12.0);
        assert!(park(std::slice::from_ref(&stub)).is_empty());
    }

    #[test]
    fn a_one_way_carriageway_parks_on_its_right() {
        // улица на восток: справа по ходу — юг
        let points = vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)];
        let mut oneway = street(points.clone(), 12.0);
        oneway.oneway = true;
        let cars = park(std::slice::from_ref(&oneway));
        assert!(!cars.is_empty());
        for car in &cars {
            assert!(car.at.y < 0.0, "ряд не с той стороны: {}", car.at);
        }

        // та же улица без `oneway` — с обеих сторон
        let both = park(std::slice::from_ref(&street(points, 12.0)));
        assert!(both.iter().any(|car| car.at.y > 0.0));
        assert!(both.iter().any(|car| car.at.y < 0.0));
    }

    /// Нос машины — по потоку её полосы: при правостороннем движении южный
    /// ряд улицы на восток смотрит на восток, северный — на запад, при
    /// левостороннем наоборот. Перекос парковки — градусы, так что знак
    /// проекции на ось улицы он не меняет.
    #[test]
    fn cars_face_the_traffic_of_their_own_kerb() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        for (traffic, south_heading) in [(TrafficSide::Right, 1.0), (TrafficSide::Left, -1.0)] {
            let cars = park_driving(std::slice::from_ref(&road), CarStyle::default(), traffic);
            assert!(cars.iter().any(|car| car.at.y > 0.0));
            assert!(cars.iter().any(|car| car.at.y < 0.0));
            for car in &cars {
                let expected = if car.at.y < 0.0 {
                    south_heading
                } else {
                    -south_heading
                };
                assert!(
                    car.along.x * expected > 0.9,
                    "{traffic:?}: машина в {} смотрит {}",
                    car.at,
                    car.along
                );
            }
        }
    }

    /// Сторона движения переворачивает машины, но не переставляет их: поток
    /// ГПСЧ от неё не зависит, и двусторонняя улица Лондона стоит теми же
    /// местами, что и такая же в Туле.
    #[test]
    fn the_traffic_side_turns_the_row_without_moving_it() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let right = park_driving(
            std::slice::from_ref(&road),
            CarStyle::default(),
            TrafficSide::Right,
        );
        let left = park_driving(
            std::slice::from_ref(&road),
            CarStyle::default(),
            TrafficSide::Left,
        );
        assert_eq!(right.len(), left.len());
        for (a, b) in right.iter().zip(&left) {
            assert_eq!(a.at, b.at);
            assert!(
                (a.along + b.along).length() < 1e-5,
                "{} {}",
                a.along,
                b.along
            );
        }
    }

    #[test]
    fn a_left_hand_one_way_carriageway_parks_on_its_left() {
        // улица на восток: слева по ходу — север, и носы по ходу
        let mut oneway = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        oneway.oneway = true;
        let cars = park_driving(
            std::slice::from_ref(&oneway),
            CarStyle::default(),
            TrafficSide::Left,
        );
        assert!(!cars.is_empty());
        for car in &cars {
            assert!(car.at.y > 0.0, "ряд не с той стороны: {}", car.at);
            assert!(
                car.along.x > 0.9,
                "машина смотрит против потока: {}",
                car.along
            );
        }
    }

    #[test]
    fn occupancy_zero_parks_nothing() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        let empty = CarStyle {
            occupancy: 0.0,
            ..default()
        };
        assert!(park_with(std::slice::from_ref(&road), empty).is_empty());
    }

    #[test]
    fn a_roundabout_stays_empty() {
        let mut ring = street(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)], 12.0);
        ring.roundabout = true;
        assert!(park(std::slice::from_ref(&ring)).is_empty());
    }

    #[test]
    fn the_row_is_ragged_and_stable() {
        let road = street(vec![Vec2::new(0.0, 0.0), Vec2::new(400.0, 0.0)], 12.0);
        let cars = park(std::slice::from_ref(&road));
        // мест на 400 м вдвое больше, чем машин: ряд рваный, а не сплошной
        let places = ((400.0 - 2.0 * END_MARGIN) / CAR_PITCH) as usize * 2;
        assert!(cars.len() < places, "{} of {places}", cars.len());
        assert!(cars.len() > places / 5, "{} of {places}", cars.len());
        // и он тот же самый при повторной сборке
        let again = park(std::slice::from_ref(&road));
        assert_eq!(cars.len(), again.len());
        for (car, twin) in cars.iter().zip(&again) {
            assert_eq!(car.at, twin.at);
        }
    }

    /// Наименьшее расстояние от точки до ломаной — тем же способом, каким
    /// глаз проверяет, лежит ли машина на асфальте.
    fn distance_to_path(points: &[Vec2], at: Vec2) -> f32 {
        points
            .windows(2)
            .map(|link| {
                let span = link[1] - link[0];
                let t = (at - link[0]).dot(span) / span.length_squared().max(f32::EPSILON);
                at.distance(link[0] + span * t.clamp(0.0, 1.0))
            })
            .fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn the_row_stays_on_the_drawn_asphalt_through_a_bend() {
        // излом 30° на звеньях по 40 м: Chaikin срезает вершину на два метра,
        // и ряд по сырым точкам вставал бы за кромкой
        let points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0 + 40.0 * 0.866, 20.0),
        ];
        let road = street(points, 8.0);
        let style = CarStyle {
            visible: true,
            occupancy: 1.0,
        };
        let cars = park_cars(
            std::slice::from_ref(&road),
            &junctions::marking_breaks(std::slice::from_ref(&road), is_carriageway),
            style,
            RoadSmoothing::Light,
            TrafficSide::Right,
            &Districts::new(&[]),
        );
        assert!(!cars.is_empty());
        let drawn = smooth_path(&road.points, road.width, RoadSmoothing::Light);
        for car in &cars {
            let off = distance_to_path(&drawn, car.at);
            assert!(
                off <= road.width / 2.0 - car.shape.width() / 2.0 + 0.01,
                "кузов в {off} м от нарисованной осевой"
            );
        }
    }
}
