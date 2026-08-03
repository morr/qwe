//! Прототип полигонального navmesh (polyanya): CDT по векторной геометрии
//! карты вместо растеризации в тайлы. Строится лениво — по включению панели
//! Polymesh (`ui/polynav.rs`) — и асинхронно, по образцу постройки
//! northstar-иерархии; результат пока только рисуется оверлеем, поиск пути
//! по нему не ходит.
//!
//! Препятствия — те же источники и та же семантика порядка, что у
//! `Navmesh::fill_from_mapdata`, сведённые в один boolean:
//! union(вода ∪ водотоки ∪ бордюры мостов ∪ здания ∪ стены) −
//! union(настилы ∪ примыкающие дороги ∪ арки). Чисто растровые компенсации
//! заливки аналога не имеют и опущены: диагональный seal pass (латает
//! касание углами, которого у полигонов не бывает) и поправки ширин на
//! `±tile·√2` (компенсируют блуждание центров тайлов).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::map::osm::model::{
    MapData, PolyArea, RoadLine, WallLine, WaterLine, distance_to_segment, signed_ring_area,
};
use crate::map::{bridge_curb_width, miter_offsets};
use crate::settings::{MAP_SIZE, PASSAGE_MAX_WIDTH};

/// Близнец `navmesh.rs::JOIN_EPSILON`: примыкание — общий узел двух ways,
/// допуск покрывает лишь потерю точности проекции.
const JOIN_EPSILON: f32 = 0.5;

/// Упрощение контуров (Visvalingam–Whyatt внутри polyanya) и допуск
/// инфляции радиусом агента, метры. Добивает вырожденные точки после клипа.
const SIMPLIFY_EPSILON: f32 = 0.05;

/// Насколько прямоугольник клипа ШИРЕ границы карты. Препятствия обрезаются
/// снаружи от внешней границы триангуляции, а не внутрь: втянутый клип
/// оставлял бы вдоль кромки карты проходимую щель шириной в отступ, и путь
/// обходил бы по ней реку, упирающуюся в границу. Пересечение препятствием
/// внешней границы polyanya переваривает: проходимость треугольника — точечный
/// `contains` центра (внутри exterior и вне колец препятствий), так что
/// треугольники за границей непроходимы сами по себе, а клип лишь не даёт
/// хвостам OSM-геометрии за bbox раздувать триангуляцию
/// (`polyanya::input/triangulation.rs::as_layer`).
const MAP_EDGE_MARGIN: f32 = 10.0;

/// Панель Polymesh: тумблер и радиус агента (инфляция препятствий).
/// Персистится, как остальные панели; восстановленный `enabled` означает
/// «кнопка уже нажата» — постройка стартует на входе в мир.
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "polymesh")]
pub struct PolymeshDebug {
    pub enabled: bool,
    pub agent_radius: f32,
}

/// Результат одной постройки: сам меш и контуры препятствий, которые в него
/// вошли. Контуры нужны оверлею — polyanya хранит только **проходимые**
/// полигоны, а закрасить надо ровно то, что было объявлено непроходимым.
pub struct PolymeshBuild {
    pub mesh: polyanya::Mesh,
    /// Внешние контуры препятствий после boolean — то же, что ушло в
    /// `add_obstacle`, до инфляции радиусом агента (её видно как зазор между
    /// заливкой и рёбрами меша).
    pub obstacles: Vec<Vec<Vec2>>,
}

/// Построенный полигональный меш; `None`, пока панель ни разу не включали
/// (ленивость) или постройка ещё идёт.
#[derive(Resource, Default)]
pub struct PolyNavmesh {
    build: Option<Arc<PolymeshBuild>>,
    /// Счётчик завершённых построек — ключ кеша оверлея.
    generation: u32,
    /// Радиус агента, под который построен текущий меш.
    built_radius: f32,
    /// Радиус летящей постройки и её таск.
    task: Option<(f32, Task<Option<PolymeshBuild>>)>,
    /// Флаг отмены текущей постройки — та же машинерия, что у
    /// [`NorthstarGrid`](super::NorthstarGrid), и по той же причине: тело
    /// таска синхронно, await-точек, на которых пул мог бы его выбросить,
    /// в нём нет, так что уронить `Task` — значит выбросить результат, но
    /// не работу. Протяжка ползунка радиуса ставит по постройке на шаг, а с
    /// ненулевым радиусом одна стоит ~20 с (инфляция препятствий), — без
    /// флага пять перекрытых построек доедали все ядра до конца.
    cancelled: Option<Arc<AtomicBool>>,
}

impl PolyNavmesh {
    pub fn build(&self) -> Option<Arc<PolymeshBuild>> {
        self.build.clone()
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn built_radius(&self) -> f32 {
        self.built_radius
    }

    pub fn is_building(&self) -> bool {
        self.task.is_some()
    }

    /// Сброс перед сменой карты: меш старого города описывает не ту
    /// геометрию, летящий таск — тем более.
    pub fn clear(&mut self) {
        self.build = None;
        self.cancel_task();
    }

    /// Отпустить летящую постройку и попросить её выйти: без флага она
    /// доедает ядра ещё десятки секунд ради результата, который выбросят.
    fn cancel_task(&mut self) {
        self.task = None;
        if let Some(cancelled) = self.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
    }
}

/// Снапшот векторной геометрии для таска — клон, без локов и без ссылок в
/// ECS (образец — `northstar::start_northstar_build`).
struct PolymeshInput {
    buildings: Vec<PolyArea>,
    water: Vec<PolyArea>,
    water_lines: Vec<WaterLine>,
    walls: Vec<WallLine>,
    roads: Vec<RoadLine>,
}

/// Запуск постройки, когда включённой панели нечего показать: меша под
/// текущий радиус нет и таск под него не летит. Смена радиуса при летящем
/// таске отменяет старый — результат он бы всё равно отдал в мусор.
pub fn sync_polymesh_build(
    map: Res<MapData>,
    debug: Res<PolymeshDebug>,
    mut poly: ResMut<PolyNavmesh>,
) {
    if !debug.enabled {
        // выключение не стирает готовый меш — оверлей просто прячется, а
        // повторное включение с тем же радиусом бесплатно
        poly.cancel_task();
        return;
    }
    let radius = debug.agent_radius;
    if poly.build.is_some() && poly.built_radius.to_bits() == radius.to_bits() {
        return;
    }
    if let Some((in_flight, _)) = &poly.task
        && in_flight.to_bits() == radius.to_bits()
    {
        return;
    }
    poly.cancel_task();
    let input = PolymeshInput {
        buildings: map.buildings.clone(),
        water: map.water.clone(),
        water_lines: map.water_lines.clone(),
        walls: map.walls.clone(),
        roads: map.roads.clone(),
    };
    let started = Instant::now();
    let cancelled = Arc::new(AtomicBool::new(false));
    poly.cancelled = Some(cancelled.clone());
    poly.task = Some((
        radius,
        AsyncComputeTaskPool::get().spawn(async move {
            let built = build_polymesh(&input, radius, Some(&cancelled));
            match &built {
                Some(built) => {
                    let polygons: usize = built
                        .mesh
                        .layers
                        .iter()
                        .map(|layer| layer.polygons.len())
                        .sum();
                    info!(
                        "polymesh built in {:?} ({polygons} polygons, {} obstacles, agent radius {radius:.1})",
                        started.elapsed(),
                        built.obstacles.len()
                    );
                }
                None => info!(
                    "polymesh build cancelled after {:?} (agent radius {radius:.1})",
                    started.elapsed()
                ),
            }
            built
        }),
    ));
}

/// Снятие готового меша с таска; `generation` растёт только здесь, и только
/// эта запись (не опрос) даёт `resource_changed` оверлею.
pub fn poll_polymesh_build(mut poly: ResMut<PolyNavmesh>) {
    let silent = poly.bypass_change_detection();
    let Some((radius, task)) = silent.task.as_mut() else {
        return;
    };
    let radius = *radius;
    let Some(built) = check_ready(task) else {
        return;
    };
    poly.task = None;
    poly.cancelled = None;
    // `None` — постройку отменили; меша у нас нет, и это не ошибка
    let Some(built) = built else {
        return;
    };
    poly.build = Some(Arc::new(built));
    poly.built_radius = radius;
    poly.generation = poly.generation.wrapping_add(1);
}

/// Весь конвейер: контуры препятствий → boolean → CDT polyanya. `None` —
/// постройку отменили; проверки стоят перед каждым долгим шагом, внутрь
/// i_overlay и spade не заглянуть.
fn build_polymesh(
    input: &PolymeshInput,
    agent_radius: f32,
    cancelled: Option<&AtomicBool>,
) -> Option<PolymeshBuild> {
    let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));

    let mut blockers: Vec<Vec<[f32; 2]>> = Vec::new();
    // дыры колец отброшены сознательно: карман внутри препятствия (двор без
    // арки, остров в пруду) недостижим снаружи — сеточный prune_unreachable
    // убивает ровно такие полости
    for area in input.buildings.iter().chain(&input.water) {
        push_contour(&mut blockers, area.outer.clone());
    }
    // трубы не блокируют — над культвертом земля (как в заливке сетки)
    for line in input.water_lines.iter().filter(|line| !line.tunnel) {
        if let Some(ring) = ribbon_outline(&line.points, line.width) {
            push_contour(&mut blockers, ring);
        }
    }
    for wall in &input.walls {
        if let Some(ring) = ribbon_outline(&wall.points, wall.width) {
            push_contour(&mut blockers, ring);
        }
    }
    // бордюры мостов — те же две полосы, что рисует рендер: от `width/2` до
    // `width/2 + curb` по каждому борту (осевая полосы на `(width+curb)/2`,
    // ширина `curb`), общие с заливкой сетки по `miter_offsets`.
    //
    // Блокирует не всякая полоса. OSM режет один физический мост на несколько
    // ways (проезжая часть и тротуар — параллельные ленты), и бордюр на
    // внутреннем шве такой пары запер бы мост поперёк. Сетка решает это щупом
    // «что снаружи» по тайлам; вектору доступна прямая формулировка того же
    // намерения — **полоса минус ленты всех ОСТАЛЬНЫХ bridge-ways**: то, что
    // накрыто соседней лентой, и есть внутренний шов, остальное — внешняя
    // граница composite-моста. Мостов на карте десятки, так что N разностей
    // дешевле одного union зданий.
    let bridges: Vec<&RoadLine> = input.roads.iter().filter(|road| road.bridge).collect();
    let bands: Vec<Vec<[f32; 2]>> = bridges
        .iter()
        .filter_map(|road| {
            let curb = bridge_curb_width(road.width);
            ribbon_outline(&road.points, road.width + 2.0 * curb)
        })
        .map(oriented)
        .collect();
    for (index, road) in bridges.iter().enumerate() {
        let curb = bridge_curb_width(road.width);
        let offsets = miter_offsets(&road.points, false, (road.width + curb) / 2.0);
        let mut sides: Vec<Vec<[f32; 2]>> = Vec::with_capacity(2);
        for side in [-1.0, 1.0] {
            let edge: Vec<Vec2> = road
                .points
                .iter()
                .zip(&offsets)
                .map(|(&point, &offset)| point + side * offset)
                .collect();
            if let Some(ring) = ribbon_outline(&edge, curb) {
                sides.push(oriented(ring));
            }
        }
        let others: Vec<Vec<[f32; 2]>> = bands
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, band)| band.clone())
            .collect();
        for shape in sides.overlay(&others, OverlayRule::Difference, FillRule::NonZero) {
            blockers.extend(shape);
        }
    }

    let mut carves: Vec<Vec<[f32; 2]>> = Vec::new();
    let bridge_ways: Vec<&[Vec2]> = bridges.iter().map(|road| road.points.as_slice()).collect();
    for road in &bridges {
        // настил — ровно проезжая часть, как её рисует рендер (`road.width`,
        // бордюрная подложка шире на `curb` с каждой стороны). Сеточное
        // `+curb − tile·√2` сюда не переносится: обе поправки компенсируют
        // блуждание центров тайлов, а полная ширина вместе с бордюром съела бы
        // половину полосы, которая обязана остаться барьером.
        if let Some(ring) = ribbon_outline(&road.points, road.width) {
            push_contour(&mut carves, ring);
        }
    }
    // примыкающая дорога открывает бордюр, который накрывает её панель;
    // береговая тропа в паре метров ПОД пролётом узла не делит и не
    // открывает ничего (тот же тест, что в заливке сетки)
    for road in input.roads.iter().filter(|road| !road.bridge) {
        let joins = bridge_ways.iter().any(|way| {
            road.points
                .iter()
                .any(|&point| distance_to_polyline(point, way) < JOIN_EPSILON)
                || way
                    .iter()
                    .any(|&point| distance_to_polyline(point, &road.points) < JOIN_EPSILON)
        });
        if joins && let Some(ring) = ribbon_outline(&road.points, road.width) {
            push_contour(&mut carves, ring);
        }
    }
    for road in input.roads.iter().filter(|road| road.passage) {
        if let Some(ring) = ribbon_outline(&road.points, road.width.min(PASSAGE_MAX_WIDTH)) {
            push_contour(&mut carves, ring);
        }
    }

    if is_cancelled() {
        return None;
    }
    // union(blockers) − union(carves) одним difference: NonZero при единой
    // закрутке контуров объединяет внутри subject и внутри clip сам
    let shapes = blockers.overlay(&carves, OverlayRule::Difference, FillRule::NonZero);
    let margin = MAP_EDGE_MARGIN;
    let rect = vec![vec![
        [-margin, -margin],
        [MAP_SIZE.x + margin, -margin],
        [MAP_SIZE.x + margin, MAP_SIZE.y + margin],
        [-margin, MAP_SIZE.y + margin],
    ]];
    let shapes = shapes.overlay(&rect, OverlayRule::Intersect, FillRule::NonZero);
    if is_cancelled() {
        return None;
    }

    // polyanya живёт на glam 0.30 (bevy — на 0.32), конверсия по полям
    let pv = |p: [f32; 2]| polyanya_glam::Vec2::new(p[0], p[1]);
    let outer = [
        pv([0.0, 0.0]),
        pv([MAP_SIZE.x, 0.0]),
        pv([MAP_SIZE.x, MAP_SIZE.y]),
        pv([0.0, MAP_SIZE.y]),
    ];
    let mut triangulation = polyanya::Triangulation::from_outer_edges(&outer);
    triangulation.set_agent_radius(agent_radius);
    triangulation.set_agent_radius_simplification(SIMPLIFY_EPSILON);
    let mut obstacles: Vec<Vec<Vec2>> = Vec::with_capacity(shapes.len());
    for shape in &shapes {
        // контур 0 — внешний; дыры результата difference — карманы, см. выше
        let Some(contour) = shape.first() else {
            continue;
        };
        triangulation.add_obstacle(contour.iter().map(|&point| pv(point)));
        // тот же контур придержан для оверлея: polyanya хранит только
        // проходимые полигоны, а закрасить надо непроходимое
        obstacles.push(contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect());
    }
    triangulation.simplify(SIMPLIFY_EPSILON);
    // самый длинный шаг: инфляция препятствий радиусом агента плюс CDT — на
    // Туле 5 с при нулевом радиусе и ~20 с при ненулевом
    if is_cancelled() {
        return None;
    }
    let mut mesh = triangulation.as_navmesh();
    if is_cancelled() {
        return None;
    }
    mesh.merge_polygons();
    Some(PolymeshBuild { mesh, obstacles })
}

/// Кольцо → контур i_overlay, нормализованный CCW: NonZero гасит контуры
/// противоположного обхода, а обход source-колец OSM произволен (тот же
/// приём, что у `buildings::layers::shadow_builder`).
fn oriented(mut ring: Vec<Vec2>) -> Vec<[f32; 2]> {
    if signed_ring_area(&ring) < 0.0 {
        ring.reverse();
    }
    ring.into_iter().map(|point| [point.x, point.y]).collect()
}

/// То же с отбраковкой вырожденных колец, прямо в накопитель.
fn push_contour(target: &mut Vec<Vec<[f32; 2]>>, ring: Vec<Vec2>) {
    if ring.len() < 3 {
        return;
    }
    target.push(oriented(ring));
}

/// Замкнутый контур ленты постоянной ширины вдоль открытой ломаной:
/// `p + o` вперёд, `p − o` в обратном порядке. Та же кромка, что у
/// `RoadJoin::Miter`-отрисовки и у бордюров в заливке сетки.
fn ribbon_outline(path: &[Vec2], width: f32) -> Option<Vec<Vec2>> {
    if path.len() < 2 {
        return None;
    }
    let offsets = miter_offsets(path, false, width / 2.0);
    let mut ring: Vec<Vec2> = path
        .iter()
        .zip(&offsets)
        .map(|(&point, &offset)| point + offset)
        .collect();
    ring.extend(
        path.iter()
            .zip(&offsets)
            .rev()
            .map(|(&point, &offset)| point - offset),
    );
    Some(ring)
}

/// Минимальное расстояние от точки до ломаной — близнец приватного хелпера
/// заливки (`navmesh.rs::distance_to_polyline`).
fn distance_to_polyline(point: Vec2, points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| distance_to_segment(point, segment[0], segment[1]))
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::model::{RoadClass, WaterKind};

    /// Водоток через всю ширину карты — без моста он обязан рассечь её
    /// надвое даже у самой кромки (клип расширен наружу, втянутый оставлял
    /// бы обходную щель вдоль границы).
    fn input_with(roads: Vec<RoadLine>) -> PolymeshInput {
        PolymeshInput {
            buildings: vec![],
            water: vec![],
            water_lines: vec![WaterLine {
                points: vec![Vec2::new(0.0, 500.0), Vec2::new(MAP_SIZE.x, 500.0)],
                width: 4.0,
                kind: WaterKind::Stream,
                tunnel: false,
            }],
            walls: vec![],
            roads,
        }
    }

    /// Пиннит порядок boolean: бордюры и русло блокируют, настил вычитается
    /// после них — иначе моста либо нет, либо он перегорожен своим бордюром.
    #[test]
    fn bridge_deck_opens_a_crossing_over_a_waterway() {
        let from = polyanya_glam::Vec2::new(300.0, 450.0);
        let to = polyanya_glam::Vec2::new(300.0, 550.0);

        let severed = build_polymesh(&input_with(vec![]), 0.0, None).expect("not cancelled");
        assert!(
            severed.mesh.path(from, to).is_none(),
            "waterway without a bridge must sever the banks"
        );
        assert!(
            !severed.obstacles.is_empty(),
            "the waterway must survive as a filled obstacle for the overlay"
        );

        let bridge = RoadLine {
            points: vec![Vec2::new(300.0, 460.0), Vec2::new(300.0, 540.0)],
            width: 5.0,
            class: RoadClass::Street,
            bridge: true,
            passage: false,
        };
        let bridged = build_polymesh(&input_with(vec![bridge]), 0.0, None).expect("not cancelled");
        assert!(
            bridged.mesh.path(from, to).is_some(),
            "bridge deck must carry a path over the waterway"
        );
    }
}
