//! Полигональный navmesh (polyanya): CDT по векторной геометрии карты вместо
//! растеризации в тайлы. Строится лениво — по включению панели Polymesh
//! (`ui/polynav.rs`) — и асинхронно, по образцу постройки northstar-иерархии;
//! готовый меш и рисуется оверлеем, и водит пешек
//! ([`find_path_polymesh`]), пока панель включена.
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
use crate::settings::{
    MAP_SIZE, PASSAGE_MAX_WIDTH, POLYMESH_AGENT_RADIUS_MAX, POLYMESH_AGENT_RADIUS_MIN,
};

/// polyanya живёт на glam 0.30, bevy — на 0.32: типы не связаны ничем, и
/// конверсия возможна только по полям.
fn to_poly(point: Vec2) -> polyanya_glam::Vec2 {
    polyanya_glam::Vec2::new(point.x, point.y)
}

fn from_poly(point: polyanya_glam::Vec2) -> Vec2 {
    Vec2::new(point.x, point.y)
}

/// Близнец `navmesh.rs::JOIN_EPSILON`: примыкание — общий узел двух ways,
/// допуск покрывает лишь потерю точности проекции.
const JOIN_EPSILON: f32 = 0.5;

/// Упрощение контуров препятствий (Visvalingam–Whyatt внутри polyanya),
/// метры. Не косметика: boolean оставляет отрезки в доли миллиметра, CDT на
/// них вырождается, и поиск по такому мешу зацикливается — замерено на
/// `examples/polymesh_bench`, без упрощения прогон встаёт на 40-м запросе.
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

/// Шаг и число шагов посадки конца запроса на меш: polyanya пробует сам
/// запрошенный конец, затем окружности радиусом `delta`, `2·delta`, … по
/// `10 · step` точек. Итоговый допуск — `SEARCH_DELTA * SEARCH_STEPS`, метр;
/// половина навтайла, то есть ровно та неопределённость, с которой цель
/// приходит из сетки. Цена платится только за конец, оказавшийся вне меша, и
/// только точечными запросами к BVH.
const SEARCH_DELTA: f32 = 0.25;
const SEARCH_STEPS: u32 = 4;

/// Панель Polymesh: тумблер и радиус агента (инфляция препятствий).
/// Персистится, как остальные панели; восстановленный `enabled` означает
/// «кнопка уже нажата» — постройка стартует на входе в мир.
#[derive(Resource, Reflect, SettingsGroup)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "polymesh")]
pub struct PolymeshDebug {
    pub enabled: bool,
    pub agent_radius: f32,
    /// Рисовать ли границы чанков — верхний уровень иерархии. Отдельно от
    /// `enabled`, потому что это разные вопросы: чанки нужны, чтобы понять
    /// **как** выбирается коридор, а меш — чтобы видеть саму геометрию.
    pub chunks: bool,
}

impl Default for PolymeshDebug {
    fn default() -> Self {
        Self {
            enabled: false,
            agent_radius: POLYMESH_AGENT_RADIUS_MIN,
            chunks: false,
        }
    }
}

impl PolymeshDebug {
    /// Радиус агента, приведённый к диапазону ползунка. Читать надо через
    /// него: минимум подняли с нуля уже после того, как настройки начали
    /// сохраняться, и в файле у любого, кто трогал панель раньше, лежит 0.0.
    /// Клампить на чтении дешевле, чем чинить ресурс из системы, которая
    /// сама гейтится на `resource_changed` по нему же.
    pub fn radius(&self) -> f32 {
        self.agent_radius
            .clamp(POLYMESH_AGENT_RADIUS_MIN, POLYMESH_AGENT_RADIUS_MAX)
    }
}

/// Слоёв у polyanya не больше 256: индекс полигона — 8 бит слоя плюс 24 бита
/// номера (`instance.rs::U32Layer`). Превышение нигде не проверяется, лишний
/// слой молча берётся как `layer_index as u8` и портит меш, поэтому потолок
/// держим сами и с запасом (255-й ещё и конфликтует с сентинелом `u32::MAX`).
const MAX_CHUNKS: u32 = 240;

/// Желаемая сторона чанка. Не жёсткая: если карта такая, что чанков выйдет
/// больше потолка, сторона растёт, пока не уложится.
///
/// Полигонов во фронте поиска примерно `плотность × длина маршрута × сторона
/// чанка` — линейно по стороне. Но сторона зажата потолком снизу, и на нашей
/// карте весь доступный диапазон (295…467 м) меняет фронт всего в 1.6 раза,
/// тогда как любая точка диапазона даёт 18–24× против плоского поиска.
/// Поэтому целимся не в минимум: крупный чанк даёт более широкий коридор,
/// то есть путь ближе к оптимальному, и дешевле обходится при постройке.
///
/// **Сейчас больше карты — то есть один слой и иерархия выключена.** Сшивка
/// слоёв даёт свой, независимый от инфляции дефект: на 140 чанках прогон
/// `examples/polymesh_bench` виснет на первом же запросе, тогда как тот же
/// набор на одном слое проходит целиком (300 из 300, среднее 6.4 мс). Код
/// чанкования оставлен и работает — включается через
/// `QWE_POLYMESH_CHUNK_M=400`, — но по умолчанию не используется, пока
/// зависание на швах не найдено.
const CHUNK_TARGET_METERS: f32 = 99_000.0;

/// Сетка чанков под размер карты. Считается, а не задаётся константой в
/// метрах: фиксированная сторона на вдвое большей карте дала бы тысячи
/// слоёв — то есть не ошибку, а тихо неверную навигацию.
fn chunk_grid(map: Vec2) -> UVec2 {
    // переопределение стороны чанка из окружения — только для офлайн-прогонов
    // (`examples/polymesh_bench.rs`): выставить размер больше карты значит
    // получить один слой без сшивки, то есть развести «виновата иерархия» и
    // «виновата геометрия» без пересборки
    let mut side = std::env::var("QWE_POLYMESH_CHUNK_M")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(CHUNK_TARGET_METERS);
    loop {
        let grid = (map / side).ceil().as_uvec2().max(UVec2::ONE);
        if grid.element_product() <= MAX_CHUNKS {
            return grid;
        }
        side *= 1.05;
    }
}

/// Связные компоненты свободного места внутри одного чанка. Чанк, разрезанный
/// рекой, обязан быть двумя узлами графа, иначе коридор пообещает проход,
/// которого нет.
struct ChunkComponents {
    /// Номер компоненты для каждого полигона слоя.
    of_polygon: Vec<u32>,
    /// Центроид каждой компоненты, в локальных координатах слоя.
    centers: Vec<Vec2>,
}

/// Узел верхнего уровня — компонента конкретного чанка.
struct GraphNode {
    chunk: u8,
    center: Vec2,
}

/// Граф уровня 1: компоненты чанков и свободные смежности между ними через
/// швы. Он же отвечает за достижимость — у polyanya проверка островов
/// отключена, как только слоёв больше одного (`lib.rs:418`, там `TODO`).
struct ChunkGraph {
    /// Глобальный номер узла по (слой, компонента).
    node_of: Vec<Vec<u32>>,
    nodes: Vec<GraphNode>,
    /// Смежность: узел → (сосед, вес).
    edges: Vec<Vec<(u32, f32)>>,
    /// Сколько вершин удалось спарить на швах — для лога и диагностики.
    seam_vertices: usize,
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
    grid: UVec2,
    chunk_size: Vec2,
    components: Vec<ChunkComponents>,
    graph: ChunkGraph,
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

/// Постройка меша прямо из карты, без ECS и без таска — вход для офлайн-
/// прогонов (`examples/polymesh_bench.rs`). Игровой путь идёт через
/// `sync_polymesh_build` и зовёт ровно тот же конвейер.
pub fn build_polymesh_from_map(map: &MapData, agent_radius: f32) -> Option<PolymeshBuild> {
    build_polymesh(&PolymeshInput::from_map(map), agent_radius, None)
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

impl PolymeshInput {
    fn from_map(map: &MapData) -> Self {
        Self {
            buildings: map.buildings.clone(),
            water: map.water.clone(),
            water_lines: map.water_lines.clone(),
            walls: map.walls.clone(),
            roads: map.roads.clone(),
        }
    }
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
    let radius = debug.radius();
    if poly.build.is_some() && poly.built_radius.to_bits() == radius.to_bits() {
        return;
    }
    if let Some((in_flight, _)) = &poly.task
        && in_flight.to_bits() == radius.to_bits()
    {
        return;
    }
    poly.cancel_task();
    let input = PolymeshInput::from_map(&map);
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

    // Радиус агента отрабатывается ЗДЕСЬ, а не в polyanya.
    //
    // `Triangulation::set_agent_radius` раздувает каждое кольцо препятствия
    // независимо (`input/triangulation.rs::inflate_obstacles` — `.map()` по
    // interiors) и **не объединяет** результат. На карте города это всегда
    // невалидный вход: соседние дома стоят в 30–40 см, раздутые на 0.2 м
    // контуры пересекаются, и CDT получает самопересекающийся набор
    // ограничений. Смежность в таком меше перестаёт соответствовать
    // геометрии, и воронка поиска на нём зацикливается — замерено на
    // `examples/polymesh_bench`: с радиусом 0.2 запрос №131 висит вечно и
    // съедает всю память, с радиусом 0 те же 300 запросов проходят.
    //
    // Свой офсет плюс union даёт снова непересекающийся набор: union заодно
    // разрешает самопересечения, которые miter даёт на острых вогнутых углах.
    let inflated: Vec<Vec<[f32; 2]>> = shapes
        .iter()
        // контур 0 — внешний; дыры результата difference — карманы, см. выше
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let ring: Vec<Vec2> = contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect();
            if agent_radius > 0.0 {
                oriented(inflate_ring(&ring, agent_radius))
            } else {
                oriented(ring)
            }
        })
        .collect();
    let nothing: Vec<Vec<[f32; 2]>> = Vec::new();
    let shapes = inflated.overlay(&nothing, OverlayRule::Subject, FillRule::NonZero);
    if is_cancelled() {
        return None;
    }

    // контуры придержаны для оверлея: polyanya хранит только проходимые
    // полигоны, а закрасить надо непроходимое — и закрасить именно то, что
    // блокирует на самом деле, то есть уже раздутое
    let obstacles: Vec<Vec<Vec2>> = shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(|contour| contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect())
        .collect();

    let grid = chunk_grid(MAP_SIZE);
    let chunk_size = MAP_SIZE / grid.as_vec2();
    let chunking = Instant::now();
    // внешние контуры со своими bbox — по ним чанк отбирает то, что его
    // вообще задевает
    let contours: Vec<(Vec2, Vec2, Vec<[f32; 2]>)> = shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let (min, max) = contour.iter().fold(
                (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
                |(min, max), &[x, y]| (min.min(Vec2::new(x, y)), max.max(Vec2::new(x, y))),
            );
            (min, max, contour.clone())
        })
        .collect();
    let mut layers = Vec::with_capacity((grid.x * grid.y) as usize);
    for index in 0..grid.x * grid.y {
        if is_cancelled() {
            return None;
        }
        layers.push(chunk_layer(
            &contours,
            chunk_origin(grid, chunk_size, index),
            chunk_size,
        ));
    }
    let polygons: usize = layers.iter().map(|layer| layer.polygons.len()).sum();
    info!(
        "polymesh chunked into {}x{} layers ({polygons} polygons) in {:?}",
        grid.x,
        grid.y,
        chunking.elapsed()
    );
    if is_cancelled() {
        return None;
    }

    // компоненты считаются ДО сшивки: она метит индексы в `vertex.polygons`
    // номером слоя, и обход по ним после неё уедет за границы своего слоя
    let components: Vec<ChunkComponents> = layers.iter().map(components_of).collect();

    let mut mesh = polyanya::Mesh {
        layers,
        ..Default::default()
    };
    // строго после слияния и строго ДО сшивки: `Layer::bake` требует
    // несшитый слой, а `merge_polygons` начинается с `unbake`.
    //
    // Без bake меш только рисуется: поиску он даёт линейный скан по всем
    // полигонам на каждый конец запроса вместо BVH.
    let baking = Instant::now();
    mesh.bake();
    info!("polymesh baked in {:?}", baking.elapsed());
    if is_cancelled() {
        return None;
    }

    let stitching = Instant::now();
    let graph = stitch_chunks(&mut mesh, grid, chunk_size, &components);
    info!(
        "polymesh stitched: {} seam vertices, {} component nodes, {} edges, {:?}",
        graph.seam_vertices,
        graph.nodes.len(),
        graph.edges.iter().map(Vec::len).sum::<usize>(),
        stitching.elapsed()
    );

    // допуск посадки концов запроса на меш. Дефолт polyanya —
    // `search_delta 0.1 × search_steps 2` = 0.2 м, ровно вровень с радиусом
    // агента, и этого не хватает: цель прогулки — вершина контура здания
    // (`human::pick_building_ahead`), а сетка объявляет тайл проходимым, если
    // его центр вне полигона хоть на сантиметр. Раздутый на радиус контур
    // такой центр накрывает, и цель оказывается вне меша. Замерено на Туле:
    // с дефолтным допуском отказывало 96% запросов против 3.5% у сетки.
    mesh.set_search_delta(SEARCH_DELTA);
    mesh.set_search_steps(SEARCH_STEPS);
    Some(PolymeshBuild {
        mesh,
        obstacles,
        grid,
        chunk_size,
        components,
        graph,
    })
}

/// Шаг подразбиения кромки чанка, метры.
///
/// Без подразбиения на шве совпадают только углы чанка: CDT ставит вершины на
/// внешнем контуре лишь там, где его касается препятствие. Сшитые по двум
/// точкам полигоны становятся «соседями» через **точку**, а не через общее
/// ребро, и поиск идёт по мусорной смежности — замерено: 140 ГБ за считанные
/// секунды. Подразбиение даёт обеим сторонам совпадающие вершины (чанки
/// одинакового размера, точки кратны шагу от общего угла), то есть цепочку
/// настоящих общих рёбер. Интервал видимости переносится через ребро
/// целиком, поэтому шаг задаёт не точность пути, а лишь дробность цепочки;
/// геометрию проходов вдоль шва по-прежнему задают вершины препятствий.
/// `Triangulation::simplify` внешний контур не трогает (только `interiors`),
/// так что подразбиение переживает упрощение.
const SEAM_STEP_METERS: f32 = 20.0;

/// Контур чанка в локальных координатах, подразбитый с шагом
/// `SEAM_STEP_METERS`, обход против часовой.
fn chunk_outline(chunk_size: Vec2) -> Vec<polyanya_glam::Vec2> {
    let mut outline = Vec::new();
    let mut side = |from: Vec2, to: Vec2| {
        let steps = ((to - from).length() / SEAM_STEP_METERS).ceil().max(1.0);
        for step in 0..steps as u32 {
            outline.push(to_poly(from.lerp(to, step as f32 / steps)));
        }
    };
    let corners = [
        Vec2::ZERO,
        Vec2::new(chunk_size.x, 0.0),
        chunk_size,
        Vec2::new(0.0, chunk_size.y),
    ];
    for index in 0..4 {
        side(corners[index], corners[(index + 1) % 4]);
    }
    outline
}

/// Левый нижний угол чанка по его индексу.
fn chunk_origin(grid: UVec2, chunk_size: Vec2, index: u32) -> Vec2 {
    Vec2::new((index % grid.x) as f32, (index / grid.x) as f32) * chunk_size
}

/// Один чанк: препятствия, обрезанные его прямоугольником, триангулированные
/// в **локальных** координатах. Локальные, а не мировые, — потому что у слоя
/// есть собственный `offset`, и именно так устроен образец сшивки соседних
/// чанков в тестах самой polyanya.
fn chunk_layer(
    contours: &[(Vec2, Vec2, Vec<[f32; 2]>)],
    origin: Vec2,
    chunk_size: Vec2,
) -> polyanya::Layer {
    let far = origin + chunk_size;
    let rect = vec![vec![
        [origin.x, origin.y],
        [far.x, origin.y],
        [far.x, far.y],
        [origin.x, far.y],
    ]];
    // только те контуры, чей bbox задевает чанк. Без этого отбора каждый из
    // 140 чанков резал бы весь набор из 7178 контуров: работа квадратична, а
    // память i_overlay пропорциональна входу — на этом приложение съедало
    // десяток гигабайт ещё до первого поиска
    let nearby: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .filter(|(min, max, _)| {
            min.x <= far.x && max.x >= origin.x && min.y <= far.y && max.y >= origin.y
        })
        .map(|(_, _, contour)| contour.clone())
        .collect();
    let clipped = nearby.overlay(&rect, OverlayRule::Intersect, FillRule::NonZero);

    let local = |p: [f32; 2]| polyanya_glam::Vec2::new(p[0] - origin.x, p[1] - origin.y);
    // радиус агента уже вшит в контуры (см. `build_polymesh`), поэтому
    // `set_agent_radius` здесь не зовётся: он раздувает кольца по отдельности
    // и снова столкнул бы их.
    let mut triangulation = polyanya::Triangulation::from_outer_edges(&chunk_outline(chunk_size));
    for shape in &clipped {
        let Some(contour) = shape.first() else {
            continue;
        };
        triangulation.add_obstacle(contour.iter().map(|&point| local(point)));
    }
    // а вот упрощение обязательно, и это проверено: без него те же 300
    // запросов виснут на 40-м, с ним доезжают до конца. Boolean оставляет
    // микроотрезки в доли миллиметра, и CDT на них вырождается
    triangulation.simplify(SIMPLIFY_EPSILON);

    let mut layer = triangulation.as_layer();
    // до сходимости, а не один раз: `merge_polygons` возвращает «слил хоть
    // что-то», и каждый проход открывает следующие пары — слитый выпуклый
    // полигон становится соседом, которым не был
    while layer.merge_polygons() {}
    layer.remove_useless_vertices();
    layer.offset = to_poly(origin);
    layer
}

impl PolymeshBuild {
    /// Сетка чанков и размер одного — оверлею, чтобы нарисовать их границы.
    pub fn chunks(&self) -> (UVec2, Vec2) {
        (self.grid, self.chunk_size)
    }

    /// Чанк, в котором лежит мировая точка.
    fn chunk_at(&self, point: Vec2) -> usize {
        let cell = (point / self.chunk_size)
            .floor()
            .as_ivec2()
            .clamp(IVec2::ZERO, self.grid.as_ivec2() - IVec2::ONE);
        (cell.y as u32 * self.grid.x + cell.x as u32) as usize
    }

    /// Точка на меше и её узел графа. Слой подсказывается явно
    /// (`Coords::on_layer`): без подсказки локализация линейно перебирает все
    /// слои, а с ней идёт сразу в BVH нужного. Если в своём чанке точка не
    /// села — вторая попытка без подсказки: у самой кромки свободное место
    /// может оказаться уже за швом.
    fn locate(&self, point: Vec2, towards: Vec2) -> Option<(polyanya::Coords, u32)> {
        let chunk = self.chunk_at(point);
        let coords = self
            .mesh
            .get_closest_point(polyanya::Coords::on_layer(to_poly(point), chunk as u8))
            .or_else(|| self.mesh.get_closest_point(to_poly(point)))
            // круговой допуск (метр) не достал — идём от точки по прямой к
            // напарнику запроса: цель у стены здания оказывается внутри
            // раздутого контура, и ближайшее свободное место лежит именно в
            // ту сторону, откуда пешка придёт
            .or_else(|| {
                self.mesh
                    .get_closest_point_towards(to_poly(point), to_poly(towards))
            })?;
        let layer = coords.layer()? as usize;
        // индекс полигона упакован: старшие 8 бит — слой
        let polygon = (coords.polygon() & 0x00FF_FFFF) as usize;
        let component = *self.components.get(layer)?.of_polygon.get(polygon)?;
        let node = *self.graph.node_of.get(layer)?.get(component as usize)?;
        Some((coords, node))
    }
}

/// Путь по полигональному мешу от точки к точке, **включая стартовую** —
/// таков контракт `movement::listen_for_pathfinding_tasks`, унаследованный от
/// сеточного поиска (первый waypoint отбрасывается, единственный означает
/// «уже на месте»). У polyanya `Path::path` старта не содержит.
///
/// Двухуровневый, по образцу `NorthstarGrid`: сперва A* по графу компонент
/// чанков, затем один запрос polyanya, которому оставлены незаблокированными
/// только чанки коридора. Плоский поиск по всему мешу разворачивал фронт по
/// всему городу — 85 млн узлов и смерть процесса от OOM; коридор держит его в
/// пределах пары тысяч полигонов.
///
/// `None` — конец не сажается на меш либо цель в другой компоненте связности.
/// Достижимость отвечает именно граф: у polyanya проверка островов отключена,
/// как только слоёв больше одного.
pub fn find_path_polymesh(build: &PolymeshBuild, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
    let (start, start_node) = build.locate(from, to)?;
    let (goal, goal_node) = build.locate(to, from)?;

    let blocked = if start_node == goal_node {
        // один и тот же кусок одного чанка — верхний уровень не нужен
        build.blocked_outside(std::iter::once(build.graph.nodes[start_node as usize].chunk))
    } else {
        let target = build.graph.nodes[goal_node as usize].center;
        let (route, _) = pathfinding::directed::astar::astar(
            &start_node,
            |&node| {
                build.graph.edges[node as usize]
                    .iter()
                    .map(|&(next, weight)| (next, (weight * COST_SCALE) as u32))
            },
            |&node| {
                (build.graph.nodes[node as usize].center.distance(target) * COST_SCALE) as u32
            },
            |&node| node == goal_node,
        )?;
        build.blocked_outside(route.into_iter().map(|node| build.graph.nodes[node as usize].chunk))
    };

    let path = build.mesh.path_on_layers(start, goal, blocked)?;
    let mut points = Vec::with_capacity(path.path.len() + 1);
    points.push(from);
    points.extend(path.path.into_iter().map(from_poly));
    Some(points)
}

/// Веса графа целочисленные (у `astar` из `pathfinding` порядок на стоимости);
/// метры переводятся в сантиметры.
const COST_SCALE: f32 = 100.0;

impl PolymeshBuild {
    /// Набор слоёв, которые поиску закрыты: всё, кроме коридора.
    fn blocked_outside(&self, corridor: impl Iterator<Item = u8>) -> std::collections::HashSet<u8> {
        let corridor: std::collections::HashSet<u8> = corridor.collect();
        (0..self.mesh.layers.len() as u8)
            .filter(|layer| !corridor.contains(layer))
            .collect()
    }
}

/// Компоненты связности слоя: flood fill по соседству полигонов через общие
/// вершины. Считать можно только **до** сшивки — она метит индексы в
/// `vertex.polygons` номером слоя, и обход по ним уедет за границы массива.
/// Своё, а не `Layer::bake_islands_detection`: его результат лежит в
/// `pub(crate)` поле и наружу не отдаётся.
fn components_of(layer: &polyanya::Layer) -> ChunkComponents {
    let count = layer.polygons.len();
    let mut of_polygon = vec![u32::MAX; count];
    let mut centers: Vec<Vec2> = Vec::new();

    for root in 0..count {
        if of_polygon[root] != u32::MAX {
            continue;
        }
        let id = centers.len() as u32;
        of_polygon[root] = id;
        let mut sum = Vec2::ZERO;
        let mut members = 0.0;
        let mut stack = vec![root];

        while let Some(current) = stack.pop() {
            let polygon = &layer.polygons[current];
            let mut center = Vec2::ZERO;
            let mut corners = 0.0;
            let count = polygon.vertices.len();
            for index in 0..count {
                let first = polygon.vertices[index] as usize;
                let Some(vertex) = layer.vertices.get(first) else {
                    continue;
                };
                center += from_poly(vertex.coords);
                corners += 1.0;
                // сосед — только через РЕБРО: polyanya переносит поиск через
                // общее ребро, и два полигона, соприкоснувшиеся одной
                // вершиной (защемление на углу препятствия), для неё не
                // связаны. Заливка по вершинам склеивала бы их в одну
                // компоненту, граф обещал бы проход, поиск бы его не нашёл —
                // и выжег бы весь бюджет итераций
                let second = polygon.vertices[(index + 1) % count] as usize;
                let Some(neighbour) = shared_polygon_excluding(layer, first, second, current)
                else {
                    continue;
                };
                if of_polygon.get(neighbour) == Some(&u32::MAX) {
                    of_polygon[neighbour] = id;
                    stack.push(neighbour);
                }
            }
            if corners > 0.0 {
                sum += center / corners;
                members += 1.0;
            }
        }
        centers.push(if members > 0.0 {
            sum / members
        } else {
            Vec2::ZERO
        });
    }

    ChunkComponents {
        of_polygon,
        centers,
    }
}

/// Полигон, которому принадлежат обе вершины, — то есть их общее ребро.
/// Считать можно только до сшивки: она метит индексы номером слоя.
fn shared_polygon(layer: &polyanya::Layer, first: usize, second: usize) -> Option<usize> {
    shared_polygon_excluding(layer, first, second, usize::MAX)
}

/// То же, но мимо заданного полигона — для обхода соседей: у ребра их два, и
/// нужен тот, с которого не начинали.
fn shared_polygon_excluding(
    layer: &polyanya::Layer,
    first: usize,
    second: usize,
    skip: usize,
) -> Option<usize> {
    let others = &layer.vertices.get(second)?.polygons;
    layer
        .vertices
        .get(first)?
        .polygons
        .iter()
        .find(|&&polygon| {
            polygon != u32::MAX && polygon as usize != skip && others.contains(&polygon)
        })
        .map(|&polygon| polygon as usize)
}

/// Допуск совпадения вершин на шве, метры. Оба соседа режут одни и те же
/// глобальные контуры одной и той же прямой, так что координаты обязаны
/// совпадать; допуск покрывает только потерю точности f32 при переносе в
/// локальные координаты чанка.
const SEAM_EPSILON: f32 = 1e-3;

/// Сшивка соседних чанков и граф уровня 1 одним проходом.
///
/// Пары вершин ищутся **по совпадению мировых координат**, а не `zip`'ом
/// отсортированных списков, как в примере из тестов polyanya: zip парует по
/// порядку вдоль кромки и молча спарит не то, если с одной стороны вершин
/// окажется больше.
///
/// `stitch_at_vertices` зовётся ровно один раз на весь меш: он метит индексы
/// номером слоя через `+=`, и второй вызов пометил бы их повторно.
fn stitch_chunks(
    mesh: &mut polyanya::Mesh,
    grid: UVec2,
    chunk_size: Vec2,
    components: &[ChunkComponents],
) -> ChunkGraph {
    let mut node_of: Vec<Vec<u32>> = Vec::with_capacity(components.len());
    let mut nodes: Vec<GraphNode> = Vec::new();
    for (chunk, chunk_components) in components.iter().enumerate() {
        let offset = from_poly(mesh.layers[chunk].offset);
        let ids = chunk_components
            .centers
            .iter()
            .map(|&center| {
                nodes.push(GraphNode {
                    chunk: chunk as u8,
                    center: center + offset,
                });
                nodes.len() as u32 - 1
            })
            .collect();
        node_of.push(ids);
    }

    let mut edges: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nodes.len()];
    // сколько вершин шва связывает пару компонент. Одной мало: polyanya
    // переносит поиск через общее РЕБРО, а соприкосновение в точке она не
    // пройдёт — а граф на таком ребре пообещает проход, и запрос выжжет весь
    // бюджет итераций, разворачивая фронт по всему коридору
    let mut touching: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut stitches: Vec<((u8, u8), Vec<(usize, usize)>)> = Vec::new();
    let mut seam_vertices = 0;
    let mut weak_seams = 0;

    for y in 0..grid.y {
        for x in 0..grid.x {
            let chunk = (y * grid.x + x) as usize;
            // сосед справа и сосед сверху — каждый шов обходится один раз
            for (neighbour, along_y) in [
                (x + 1 < grid.x).then(|| (chunk + 1, true)),
                (y + 1 < grid.y).then(|| (chunk + grid.x as usize, false)),
            ]
            .into_iter()
            .flatten()
            {
                let (start, end) = if along_y {
                    (
                        Vec2::new(chunk_size.x, 0.0),
                        Vec2::new(chunk_size.x, chunk_size.y),
                    )
                } else {
                    (
                        Vec2::new(0.0, chunk_size.y),
                        Vec2::new(chunk_size.x, chunk_size.y),
                    )
                };
                let (other_start, other_end) = if along_y {
                    (Vec2::ZERO, Vec2::new(0.0, chunk_size.y))
                } else {
                    (Vec2::ZERO, Vec2::new(chunk_size.x, 0.0))
                };

                let here = mesh.layers[chunk].get_vertices_on_segment(to_poly(start), to_poly(end));
                let there = mesh.layers[neighbour]
                    .get_vertices_on_segment(to_poly(other_start), to_poly(other_end));
                if here.is_empty() || there.is_empty() {
                    continue;
                }

                let offset_here = from_poly(mesh.layers[chunk].offset);
                let offset_there = from_poly(mesh.layers[neighbour].offset);
                // концы шва — углы сетки чанков, общие сразу для четырёх
                // слоёв. Сшивать их нельзя: соседство ставится попарно
                // (правый сосед и верхний), в такой точке связи выходят
                // несимметричными, кольцо соседей после
                // `reorder_neighbors_ccw_and_fix_corners` — несогласованным, и
                // обход внутри `successors` из него не выходит вовсе: поиск
                // висит, а память течёт. Потеря — засечка в самом углу, мимо
                // которой воронка проходит по соседним отрезкам шва.
                let corners = [start + offset_here, end + offset_here];
                let mut pairs = Vec::new();
                for &vertex in &here {
                    let world = from_poly(mesh.layers[chunk].vertices[vertex].coords) + offset_here;
                    if corners
                        .iter()
                        .any(|corner| corner.distance_squared(world) <= SEAM_EPSILON * SEAM_EPSILON)
                    {
                        continue;
                    }
                    let matched = there.iter().find(|&&other| {
                        (from_poly(mesh.layers[neighbour].vertices[other].coords) + offset_there)
                            .distance_squared(world)
                            <= SEAM_EPSILON * SEAM_EPSILON
                    });
                    let Some(&other) = matched else {
                        continue;
                    };
                    pairs.push((vertex, other));
                }
                // ребро графа — не «вершины совпали», а **общий отрезок шва**:
                // polyanya переносит поиск через ребро, и пара компонент,
                // соприкоснувшихся в точке (или в двух точках по разные
                // стороны препятствия), проход не даёт. Граф, пообещавший
                // такой проход, стоит очень дорого: коридор построен, а цели
                // в нём не достичь, и запрос выжигает весь бюджет итераций,
                // разворачивая фронт на гигабайты.
                //
                // Вершины идут вдоль шва по порядку (`get_vertices_on_segment`
                // сортирует), так что соседние в списке и есть концы отрезка;
                // отрезок свободен ровно тогда, когда обе его вершины лежат в
                // одном полигоне — то есть у него есть общее ребро.
                for window in pairs.windows(2) {
                    let [(here_from, there_from), (here_to, there_to)] = window else {
                        continue;
                    };
                    let (Some(polygon_here), Some(polygon_there)) = (
                        shared_polygon(&mesh.layers[chunk], *here_from, *here_to),
                        shared_polygon(&mesh.layers[neighbour], *there_from, *there_to),
                    ) else {
                        continue;
                    };
                    let (Some(&from), Some(&to)) = (
                        components[chunk].of_polygon.get(polygon_here),
                        components[neighbour].of_polygon.get(polygon_there),
                    ) else {
                        continue;
                    };
                    *touching
                        .entry((node_of[chunk][from as usize], node_of[neighbour][to as usize]))
                        .or_insert(0) += 1;
                }
                if pairs.len() < 2 {
                    weak_seams += 1;
                }
                if pairs.is_empty() {
                    continue;
                }
                seam_vertices += pairs.len();
                stitches.push(((chunk as u8, neighbour as u8), pairs));
            }
        }
    }

    for ((from, to), segments) in touching {
        debug_assert!(segments > 0);
        let weight = nodes[from as usize]
            .center
            .distance(nodes[to as usize].center);
        edges[from as usize].push((to, weight));
        edges[to as usize].push((from, weight));
    }

    if weak_seams > 0 {
        warn!("polymesh: {weak_seams} seams stitched by fewer than two vertices");
    }
    if !stitches.is_empty() {
        // ровно один вызов на весь меш: он метит индексы номером слоя через
        // `+=`, и второй вызов пометил бы их повторно
        mesh.stitch_at_vertices(stitches, false);
    }

    ChunkGraph {
        node_of,
        nodes,
        edges,
        seam_vertices,
    }
}

/// Кольцо, смещённое наружу на `distance`, тем же miter-офсетом, которым
/// строятся ленты дорог. Сторона выбирается по площади: у смещённого наружу
/// кольца она по модулю больше, и это не зависит от исходной закрутки.
fn inflate_ring(ring: &[Vec2], distance: f32) -> Vec<Vec2> {
    let offsets = miter_offsets(ring, true, distance);
    let shift = |sign: f32| -> Vec<Vec2> {
        ring.iter()
            .zip(&offsets)
            .map(|(&point, &offset)| point + offset * sign)
            .collect()
    };
    let (outward, inward) = (shift(1.0), shift(-1.0));
    if signed_ring_area(&outward).abs() >= signed_ring_area(&inward).abs() {
        outward
    } else {
        inward
    }
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
        let from = Vec2::new(300.0, 450.0);
        let to = Vec2::new(300.0, 550.0);

        let severed = build_polymesh(&input_with(vec![]), 0.0, None).expect("not cancelled");
        assert!(
            find_path_polymesh(&severed, from, to).is_none(),
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
        let path = find_path_polymesh(&bridged, from, to)
            .expect("bridge deck must carry a path over the waterway");

        // контракт для `listen_for_pathfinding_tasks`: стартовая точка входит
        // в путь (её там срежут), последняя — цель
        assert_eq!(path.first(), Some(&from), "path must start at `from`");
        assert!(
            path.last().expect("path is non-empty").distance(to) < 0.5,
            "path must end at `to`, got {:?}",
            path.last()
        );
    }

    /// Маршрут длиной в несколько чанков через единственный проход в стене.
    /// Пиннит сшивку швов и коридор разом: без сшивки соседние чанки не
    /// связаны и пути нет вовсе, а без прохода в стене граф компонент обязан
    /// отказать, не запуская polyanya.
    #[test]
    fn a_route_crosses_chunks_through_the_only_gap_in_a_wall() {
        let wall = |gap: bool| {
            let x = 2000.0;
            let mut walls = vec![WallLine {
                points: vec![Vec2::new(x, 0.0), Vec2::new(x, 1500.0)],
                width: 6.0,
            }];
            // верхний кусок стены начинается выше прохода — или вплотную,
            // если прохода нет
            walls.push(WallLine {
                points: vec![
                    Vec2::new(x, if gap { 1560.0 } else { 1500.0 }),
                    Vec2::new(x, MAP_SIZE.y),
                ],
                width: 6.0,
            });
            PolymeshInput {
                buildings: vec![],
                water: vec![],
                water_lines: vec![],
                walls,
                roads: vec![],
            }
        };

        let from = Vec2::new(500.0, 1000.0);
        let to = Vec2::new(3500.0, 1000.0);

        let open = build_polymesh(&wall(true), 0.2, None).expect("not cancelled");
        let path = find_path_polymesh(&open, from, to).expect("the gap must carry a path");
        assert_eq!(path.first(), Some(&from));
        assert!(path.last().expect("non-empty").distance(to) < 1.0);
        // путь обязан свернуть к проходу, а не идти напрямик сквозь стену
        assert!(
            path.iter().any(|point| point.y > 1400.0),
            "path must detour to the gap at y=1530: {path:?}"
        );

        let closed = build_polymesh(&wall(false), 0.2, None).expect("not cancelled");
        assert!(
            find_path_polymesh(&closed, from, to).is_none(),
            "a wall without a gap must sever the map"
        );
    }
}
