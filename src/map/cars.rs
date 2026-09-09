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

use crate::map::meshing::{Break, MeshBuilder};
use crate::map::osm::{MapData, RoadLine};
use crate::map::roads::junctions::{self, MarkingBreaks};
use crate::map::roads::{RoadSmoothing, RoadStyle, is_carriageway, smooth_path};
use crate::map::seed::{Lcg, seed_from_point};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SHADOW_COLOR, SHADOW_DIR, shadow_length_scale};
use crate::settings::{CAR_MAX_ZOOM, CAR_OCCUPANCY_DEFAULT, Z_CAR};

/// Габарит легковой машины, м — «Логан» с точностью до сантиметров.
const CAR_LENGTH: f32 = 4.4;
const CAR_WIDTH: f32 = 1.8;
/// Высота машины, м: по ней считается длина её тени, тем же котангенсом
/// высоты солнца, что у домов.
const CAR_HEIGHT: f32 = 1.5;
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
/// Насколько ряд не доходит до перекрёстка, м, сверх полуширины самой широкой
/// из сошедшихся дорог (`Break::reach`): ближе пяти метров к перекрёстку не
/// паркуются. Тупик приходит разрывом нулевого `reach`, и клиренс даёт в нём
/// те же пять пустых метров, что и на настоящем узле.
const JUNCTION_CLEARANCE: f32 = 5.0;

/// Палитра кузовов, по долям близкая к тому, что видно на снимке русского
/// города. Слот выбирается равномерно, поэтому доля цвета — это счёт слотов:
/// светлого ахроматического (белый, серебро, серый, тёмно-серый) — две пятых,
/// чёрного — пятая часть, остальное цветное.
const CAR_COLORS: [Color; 10] = [
    Color::srgb(0.78, 0.78, 0.77),
    Color::srgb(0.72, 0.73, 0.74),
    Color::srgb(0.60, 0.61, 0.62),
    Color::srgb(0.46, 0.47, 0.48),
    Color::srgb(0.16, 0.16, 0.17),
    Color::srgb(0.20, 0.20, 0.21),
    Color::srgb(0.22, 0.27, 0.38),
    Color::srgb(0.45, 0.14, 0.13),
    Color::srgb(0.29, 0.33, 0.28),
    Color::srgb(0.55, 0.50, 0.42),
];

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

/// Ступени зума слоя машин: ближе порога — ряды на месте, дальше слоя нет
/// вовсе. Машина в 4.4 м на 0.8 м/px это пять с половиной пикселей; ещё
/// дальше ряд превращается в мерцающий пунктир вдоль улицы.
pub enum CarLods {}

impl ZoomLods for CarLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [CAR_MAX_ZOOM, f32::INFINITY].into_iter()
    }
}

pub type CarZoomBucket = ZoomBucket<CarLods>;

/// Одна машина: центр, направление вдоль кузова и цвет.
struct Car {
    at: Vec2,
    along: Vec2,
    color: Color,
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
    if bucket.index > 0 || !style.visible {
        return;
    }
    let started = std::time::Instant::now();
    // разрывы — по **всем** настоящим улицам, а не только по парковочным: ряд
    // обязан прерваться и там, где к жилой улице примыкает другая жилая.
    //
    // Считаются заново на каждую пересборку слоя, а не один раз на загрузку
    // мира: по Туле это 0.76 мс из 5.4 мс сборки всего слоя — на фоне 70 мс
    // зданиевого слоя кеш ради этого не окупается, и мерить надо было
    // прежде, чем его заводить
    let junctions = junctions::marking_breaks(&map.roads, is_carriageway);
    let breaks_took = started.elapsed();
    let cars = park_cars(&map.roads, &junctions, *style, road_style.smoothing);
    let builder = mesh_cars(&cars);
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
        "cars: {count} parked ({vertices} verts) in {elapsed:?} (junctions {}, {breaks_took:?})",
        junctions.junctions,
    );
}

/// Меш припаркованных рядов по готовому срезу улиц — единственная дверь
/// наружу, для витрины `examples/demos/car_gallery`.
///
/// Открыта затем, что клетки витрины обязаны строиться **теми же вызовами**,
/// что и город: про шаг, палитру, разрывы на перекрёстках и правило излома
/// витрина не знает ничего и знать не должна — иначе она показывает свою
/// геометрию, а не игровую.
///
/// `smoothing` — то же, с чем витрина кладёт под ряд асфальт: осевая у ленты и
/// у ряда обязана быть одна.
pub fn cars_mesh(roads: &[RoadLine], style: CarStyle, smoothing: RoadSmoothing) -> MeshBuilder {
    let junctions = junctions::marking_breaks(roads, is_carriageway);
    mesh_cars(&park_cars(roads, &junctions, style, smoothing))
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
        // ряд с каждой стороны: отступ от кромки внутрь проезжей части
        let offset = road.width / 2.0 - CURB_GAP - CAR_WIDTH / 2.0;
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
                side * offset,
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
/// сдвигом `offset` поперёк и с пропусками.
///
/// Шаг идёт по дуговой координате целой улицы, а не по каждому её звену
/// порознь: звено ломаной в городе сплошь и рядом короче двух отступов, и
/// пошаговый обход `windows(2)` выбрасывал их целиком (в кеше Тулы — половину
/// сегментов и треть длины), а на каждой вершине сбрасывал шаг, отчего ряд то
/// рвался, то удваивался.
fn park_along(
    cars: &mut Vec<Car>,
    points: &[Vec2],
    offset: f32,
    junctions: &[Break],
    occupancy: f32,
    rng: &mut Lcg,
) {
    let (along, total) = arclengths(points);
    if total <= 2.0 * END_MARGIN {
        return;
    }
    // последняя **поставленная** машина этой стороны: на изломе внутренний
    // ряд сжимается, и место, наехавшее на соседа, пропускается. Проверка по
    // мировому расстоянию, а не по дуговой координате, — она ловит и излом,
    // и любую другую кривизну
    let mut last: Option<Vec2> = None;
    let mut step = END_MARGIN;
    while step <= total - END_MARGIN {
        let at = step;
        step += CAR_PITCH;
        let Some((point, direction)) = place_on_path(points, &along, at) else {
            continue;
        };
        let place = point + direction.perp() * offset;
        if junctions
            .iter()
            .any(|junction| place.distance(junction.at) < junction.reach + JUNCTION_CLEARANCE)
        {
            continue;
        }
        if last.is_some_and(|previous| previous.distance(place) < CAR_LENGTH) {
            continue;
        }
        if rng.next_f32() >= occupancy {
            continue;
        }
        last = Some(place);
        cars.push(Car {
            at: place,
            along: direction,
            color: CAR_COLORS
                [(rng.next_f32() * CAR_COLORS.len() as f32) as usize % CAR_COLORS.len()],
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
fn mesh_cars(cars: &[Car]) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let shadow = SHADOW_COLOR.to_linear();
    let offset = SHADOW_DIR * (CAR_HEIGHT * shadow_length_scale());
    for car in cars {
        builder.push_quad(body(car, offset), shadow);
    }
    for car in cars {
        builder.push_quad(body(car, Vec2::ZERO), car.color.to_linear());
    }
    builder
}

/// Прямоугольник кузова, сдвинутый на `offset` (для тени — по свету).
fn body(car: &Car, offset: Vec2) -> [Vec2; 4] {
    let half_length = car.along * (CAR_LENGTH / 2.0);
    let half_width = car.along.perp() * (CAR_WIDTH / 2.0);
    let at = car.at + offset;
    [
        at - half_length - half_width,
        at + half_length - half_width,
        at + half_length + half_width,
        at - half_length + half_width,
    ]
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
        // ряды по обе стороны осевой, внутри проезжей части
        let offset = road.width / 2.0 - CURB_GAP - CAR_WIDTH / 2.0;
        assert!(cars.iter().any(|car| car.at.y > 0.0));
        assert!(cars.iter().any(|car| car.at.y < 0.0));
        for car in &cars {
            assert!((car.at.y.abs() - offset).abs() < 0.01, "{}", car.at.y);
            assert!(car.at.x >= END_MARGIN - 0.01 && car.at.x <= 200.0 - END_MARGIN + 0.01);
        }
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
                car.at.y.abs() - CAR_WIDTH / 2.0 > 0.5,
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
        for (index, car) in cars.iter().enumerate() {
            for other in &cars[index + 1..] {
                let gap = car.at.distance(other.at);
                assert!(
                    gap >= CAR_LENGTH - 0.01,
                    "{gap} м между {} и {}",
                    car.at,
                    other.at
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
                off <= road.width / 2.0 - CAR_WIDTH / 2.0 + 0.01,
                "кузов в {off} м от нарисованной осевой"
            );
        }
    }
}
