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
//! Расстановка детерминирована (ГПСЧ Лемера, засеянный первой точкой улицы),
//! так что от запуска к запуску ряд стоит одинаково. Виден он только вблизи:
//! [`CarZoomBucket`] снимает слой целиком, когда машина становится мельче
//! шести пикселей.

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::map::buildings::LayerCost;
use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::{MapData, RoadLine};
use crate::map::roads::junctions::{self, MarkingBreaks};
use crate::map::roads::{RoadSmoothing, RoadStyle, is_carriageway, smooth_path};
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{shadow_dir, shadow_length_scale};
use crate::settings::{
    CAR_DETAIL_MAX_ZOOM, CAR_MAX_ZOOM, CAR_OCCUPANCY_DEFAULT, CAR_SILHOUETTE_MAX_ZOOM, Z_CAR,
};

pub mod body;

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
    /// Доля занятых мест, 0..1 — печатается процентом.
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
#[derive(Component)]
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
/// Кузов меряется на **каждой** ступени подробности, своей строкой: разница
/// между ними и есть то, ради чего заведён [`CarLods`], и она должна быть
/// видна в тех же числах, что и цена зданиевых слоёв.
pub fn measure_cars(roads: &[RoadLine]) -> (usize, Vec<LayerCost>) {
    let started = std::time::Instant::now();
    let junctions = junctions::marking_breaks(roads, is_carriageway);
    let breaks_took = started.elapsed();
    let started = std::time::Instant::now();
    let cars = park_cars(
        roads,
        &junctions,
        CarStyle::default(),
        RoadStyle::default().smoothing,
    );
    let parking_took = started.elapsed();
    let mut costs = vec![
        LayerCost {
            name: "breaks",
            vertices: 0,
            elapsed: breaks_took,
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
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<CarZoomBucket>,
    style: Res<CarStyle>,
    // сглаживание осевой: ряд стоит по той же ломаной, по которой `map::roads`
    // кладёт ленту асфальта
    road_style: Res<RoadStyle>,
    map: Res<MapData>,
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
    let cars = park_cars(&map.roads, &junctions, *style, road_style.smoothing);
    let builder = mesh_cars(&cars, detail);
    let count = cars.len();
    let vertices = builder.vertex_count();
    let elapsed = started.elapsed();
    if builder.is_empty() {
        return;
    }
    // слой с блендингом: тень машины полупрозрачна, кузов — нет
    let material = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    surface::spawn_layer(
        &mut commands,
        &mut meshes,
        builder,
        Z_CAR,
        "cars",
        LayerMaterial::Flat(material),
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
pub fn cars_mesh(
    roads: &[RoadLine],
    style: CarStyle,
    smoothing: RoadSmoothing,
    detail: CarDetail,
) -> MeshBuilder {
    let junctions = junctions::marking_breaks(roads, is_carriageway);
    mesh_cars(&park_cars(roads, &junctions, style, smoothing), detail)
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
) -> Vec<Car> {
    let mut cars = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !parkable(road) {
            continue;
        }
        // осевая та же, по которой `map::roads` строит ленту: по сырым точкам
        // OSM ряд на изломе съезжает с асфальта на тротуар, потому что Chaikin
        // срезает вершину на метры. Арок здесь не бывает — `is_carriageway` их
        // отсеял, — поэтому `smooth_path`, а не `centerline`
        let centre = smooth_path(&road.points, road.width, smoothing);
        let mut rng = Lcg::new(seed_from_point(
            road.points.first().copied().unwrap_or(Vec2::ZERO),
        ));
        // односторонняя — один ряд, справа по ходу: движение правостороннее, и
        // у половины разделённого проспекта справа бордюр, а слева
        // разделительная. Порядок точек way совпадает с направлением потока
        // (`oneway=-1` развёрнут при разборе), а поперечная в [`park_along`]
        // (`direction.perp()`) смотрит влево, поэтому правая сторона — `-1`
        let sides: &[f32] = if road.oneway { &[-1.0] } else { &[-1.0, 1.0] };
        for &side in sides {
            park_along(
                &mut cars,
                &centre,
                road.width / 2.0,
                side,
                &junctions.breaks[index],
                style.occupancy,
                &mut rng,
            );
        }
    }
    cars
}

/// Улица, вдоль которой паркуются: настоящая проезжая часть — то же
/// [`is_carriageway`], которым отбирает тротуары и разметку `map::roads`, —
/// но не мост (на мосту не стоят) и не кольцо (по кольцу едут, а не
/// паркуются). Разметке мост и кольцо нужны, машинам нет, поэтому оба
/// условия здесь, а не внутри предиката.
fn parkable(road: &RoadLine) -> bool {
    is_carriageway(road) && !road.bridge && !road.roundabout
}

/// Ряд вдоль одной стороны: шагом [`CAR_PITCH`] по **всей** ломаной улицы, со
/// сдвигом от кромки поперёк и с пропусками. `side` — знак поперечной
/// (`-1` — правая сторона по ходу), `half_road` — полуширина проезжей части,
/// от которой ряд и отступает: отступ считается по габариту **этой** машины,
/// так что фургон стоит к бордюру так же вплотную, как седан.
///
/// Шаг идёт по дуговой координате целой улицы, а не по каждому её звену
/// порознь: звено ломаной в городе сплошь и рядом короче двух отступов, и
/// пошаговый обход `windows(2)` выбрасывал их целиком (в кеше Тулы — половину
/// сегментов и треть длины), а на каждой вершине сбрасывал шаг, отчего ряд то
/// рвался, то удваивался.
fn park_along(
    cars: &mut Vec<Car>,
    points: &[Vec2],
    half_road: f32,
    side: f32,
    junctions: &[Break],
    occupancy: f32,
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
        let across = direction.perp() * side;
        let offset = half_road - CURB_GAP - shape.width() / 2.0;
        let place = point + across * offset;
        if junctions
            .iter()
            .any(|junction| place.distance(junction.at) < junction.reach + JUNCTION_CLEARANCE)
        {
            continue;
        }
        if last.is_some_and(|(previous, previous_length)| {
            previous.distance(place) < (previous_length + shape.length()) / 2.0
        }) {
            continue;
        }
        if rng.next_f32() >= occupancy {
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
            along: skew * direction,
            color: body::color_from_share(rng.next_f32()),
            shape,
        });
    }
}

/// Накопленные длины по точкам ломаной и её полная длина — та же форма, что
/// у лент в `map::meshing`, но своя: обобщать ради одного вызова нечего.
fn arclengths(points: &[Vec2]) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (index, &point) in points.iter().enumerate() {
        if index > 0 {
            total += point.distance(points[index - 1]);
        }
        along.push(total);
    }
    (along, total)
}

/// Точка ломаной на дуговой координате `at` и направление звена, на которое
/// она попала: звено ищется бинарным поиском по `along`, позиция внутри него —
/// интерполяцией.
fn place_on_path(points: &[Vec2], along: &[f32], at: f32) -> Option<(Vec2, Vec2)> {
    let last = points.len().checked_sub(2)?;
    let index = match along.binary_search_by(|value| value.total_cmp(&at)) {
        Ok(index) => index.min(last),
        Err(index) => index.saturating_sub(1).min(last),
    };
    let direction = (points[index + 1] - points[index]).try_normalize()?;
    Some((points[index] + direction * (at - along[index]), direction))
}

/// Меш слоя: сначала **все** тени, потом **все** кузова — тень соседней
/// машины иначе легла бы поверх кузова той, что нарисована раньше.
///
/// Длина тени — по высоте **этой** машины (у фургона она заметно длиннее) и
/// по тому же котангенсу высоты солнца, которым меряются тени домов.
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
    use crate::map::osm::fixture::street;
    use crate::map::roads::junctions::JUNCTION_MARGIN;

    /// Расстановка по срезу целиком — так же, как её зовёт пересборка слоя.
    fn park(roads: &[RoadLine]) -> Vec<Car> {
        park_with(roads, CarStyle::default())
    }

    fn park_with(roads: &[RoadLine], style: CarStyle) -> Vec<Car> {
        park_cars(
            roads,
            &junctions::marking_breaks(roads, is_carriageway),
            style,
            RoadSmoothing::Off,
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
