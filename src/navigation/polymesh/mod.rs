//! Полигональный navmesh (polyanya): CDT по векторной геометрии карты вместо
//! растеризации в тайлы. `PolymeshDebug::enabled` включён по умолчанию, поэтому
//! обычный путь — постройка на входе в мир (`OnEnter(AppState::Playing)`,
//! `WorldInitSet::Spawn`), асинхронная, по образцу постройки
//! northstar-иерархии; тумблер, радиус и чанки в панели Polymesh
//! (`ui/navigation/`) её только перезапускают из `Update`. Готовый меш и
//! рисуется оверлеем, и водит пешек ([`find_path_polymesh`]), пока панель
//! включена; пока он строится или его выключили руками, пешек водит сетка.
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

use crate::map::osm::model::{MapData, PolyArea, RoadLine, WallLine, WaterLine};
use crate::settings::{
    POLYMESH_AGENT_RADIUS_MAX, POLYMESH_AGENT_RADIUS_MIN, POLYMESH_CHUNK_TARGET_METERS,
    POLYMESH_FLAT_CHUNK_METERS,
};

mod build;
mod path;
mod seams;
mod stitch;

// Приватные реэкспорты, а не `pub` в подмодулях: снаружи модуль виден ровно
// тем же набором имён, что и до разрезания, а `use super::*` в `tests.rs`
// продолжает доставать `build_polymesh` и соседей.
use self::build::build_polymesh;
pub use self::path::find_path_polymesh;
// Пороги шва — наружу ради аудита (`examples/audit/polymesh_seam_audit.rs`):
// он проверяет ровно ту сшивку, что делает сборка, и своей копией проверял бы
// не её.
pub use self::seams::SEAM_QUANTUM;
pub use self::stitch::SEAM_EPSILON;

/// polyanya живёт на glam 0.30, bevy — на 0.32: типы не связаны ничем, и
/// конверсия возможна только по полям.
fn to_poly(point: Vec2) -> polyanya_glam::Vec2 {
    polyanya_glam::Vec2::new(point.x, point.y)
}

fn from_poly(point: polyanya_glam::Vec2) -> Vec2 {
    Vec2::new(point.x, point.y)
}

/// Панель Polymesh: тумблер и радиус агента (инфляция препятствий).
/// Персистится, как остальные панели; восстановленный `enabled` означает
/// «кнопка уже нажата» — постройка стартует на входе в мир.
#[derive(Resource, Reflect, SettingsGroup)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "polymesh")]
pub struct PolymeshDebug {
    /// Строить меш и **ходить по нему**: это бэкенд поиска пути, а не картинка.
    ///
    /// Включено по умолчанию: полигональный поиск — основная навигация мира, а
    /// сетка осталась запасным бэкендом (пока меш строится — 0.3–20 с в
    /// зависимости от радиуса агента — и когда его выключили руками из панели
    /// Navmesh). От этого же тумблера зависит расталкивание
    /// (`movement::separation::separation_runs`): на сетке путь идёт центрами
    /// навтайлов, и разводить с них пешки некуда.
    pub enabled: bool,
    /// Рисовать ли оверлей построенного меша. Отдельно от `enabled`, потому
    /// что это разные вопросы: по мешу можно ходить, не заливая им карту, —
    /// а рёбра 24 тысяч полигонов поверх города мешают смотреть на всё
    /// остальное. Меш от этого тумблера не перестраивается.
    ///
    /// Выключено по умолчанию — ровно поэтому: с `enabled` по умолчанию
    /// включённым «рисовать» по умолчанию означало бы город под сеткой рёбер
    /// на первом же запуске.
    pub show: bool,
    pub agent_radius: f32,
    /// Иерархия чанков: и **строить** ли меш слоями (иначе один плоский слой,
    /// см. [`POLYMESH_FLAT_CHUNK_METERS`]), и рисовать ли их границы. Один тумблер, а
    /// не два, потому что рисовать сетку, по которой поиск не идёт, значит
    /// показывать неправду; переключение поэтому запускает перестройку меша.
    /// По умолчанию включено — иерархия быстрее по обоим числам
    /// (см. [`POLYMESH_CHUNK_TARGET_METERS`]).
    pub chunks: bool,
}

impl Default for PolymeshDebug {
    fn default() -> Self {
        Self {
            enabled: true,
            show: false,
            agent_radius: POLYMESH_AGENT_RADIUS_MIN,
            chunks: true,
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

/// Слой и номер полигона из упакованного индекса (см. [`MAX_CHUNKS`]): старшие
/// 8 бит — слой, младшие 24 — номер полигона внутри слоя. Имена — как у самой
/// polyanya (`instance.rs::U32Layer`), но её трейт `pub(crate)`, наружу не
/// виден, так что распаковка всё равно своя — и пусть будет одна.
fn layer_of(packed: u32) -> usize {
    (packed >> 24) as usize
}

fn polygon_of(packed: u32) -> usize {
    (packed & 0x00FF_FFFF) as usize
}

/// Пол стороны чанка. Не вкус, а предел арифметики [`chunk_grid`]: сторона
/// входит в деление `map / side`, и всё, что меньше, переполняет произведение
/// сторон сетки раньше, чем потолок `MAX_CHUNKS` успевает сработать. Метр —
/// заведомо ниже любого осмысленного чанка (чанк мельче навтайла бессмыслен) и
/// заведомо выше опасного: даже на карте 65×65 км метровая сторона ещё лезет в
/// `u32`, а цикл ниже поднимет её до сотен метров за ~120 шагов.
const CHUNK_MIN_METERS: f32 = 1.0;

/// Сетка чанков под размер карты. Считается, а не задаётся константой в
/// метрах: фиксированная сторона на вдвое большей карте дала бы тысячи
/// слоёв — то есть не ошибку, а тихо неверную навигацию.
fn chunk_grid(map: Vec2, requested: Option<f32>) -> UVec2 {
    // переопределение стороны чанка — окружением для офлайн-прогонов
    // (`examples/bench/polymesh_bench.rs`) и параметром для тестов: выставить размер
    // больше карты значит получить один слой без сшивки, то есть развести
    // «виновата иерархия» и «виновата геометрия» без пересборки
    let mut side = match requested.or_else(|| {
        std::env::var("QWE_POLYMESH_CHUNK_M")
            .ok()
            .and_then(|value| value.parse().ok())
    }) {
        // мельче пола — всё-таки просьба «мельче», её поднимаем
        Some(side) if side.is_finite() && side > 0.0 => side.max(CHUNK_MIN_METERS),
        // 0, отрицательное, NaN, inf — не длина, а отсутствие ответа: `map / 0`
        // даёт inf, `as_uvec2` насыщается в `u32::MAX`, произведение
        // заворачивается ровно под потолок, и постройка получала сетку
        // `u32::MAX × u32::MAX` (в dev-профиле — панику переполнения в
        // async-пуле). `nan` и `-1` при этом парсятся как f32 без ошибки.
        Some(bad) => {
            warn!(
                "polymesh chunk side {bad} is not a length, using {POLYMESH_CHUNK_TARGET_METERS} m"
            );
            POLYMESH_CHUNK_TARGET_METERS
        }
        None => POLYMESH_CHUNK_TARGET_METERS,
    };
    loop {
        let grid = (map / side).ceil().as_uvec2().max(UVec2::ONE);
        // произведение — в `u64`: у стороны есть пол, но потолок слоёв обязан
        // держаться и на карте, где метровая сторона даёт больше 4 млрд чанков,
        // а `u32`-умножение там завернётся и отрапортует «уложились»
        if u64::from(grid.x) * u64::from(grid.y) <= u64::from(MAX_CHUNKS) {
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
    /// Внешние контуры препятствий после boolean и **после** инфляции радиусом
    /// агента (`build.rs`: `inflate_ring` плюс union) — ровно то, что блокирует
    /// на самом деле, поэтому заливка оверлея ложится вплотную к рёбрам меша,
    /// без зазора. В `add_obstacle` уходят они же, но клипнутые по чанку и
    /// упрощённые.
    pub obstacles: Vec<Vec<Vec2>>,
    grid: UVec2,
    chunk_size: Vec2,
    components: Vec<ChunkComponents>,
    graph: ChunkGraph,
}

/// Под что построен меш: всё, что меняет геометрию и потому требует
/// перестройки. Радиус — по битам, а не по `f32`: ключ сравнивается на
/// равенство, а не на близость.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct BuildKey {
    radius_bits: u32,
    chunked: bool,
}

impl BuildKey {
    fn new(debug: &PolymeshDebug) -> Self {
        Self {
            radius_bits: debug.radius().to_bits(),
            chunked: debug.chunks,
        }
    }

    fn radius(&self) -> f32 {
        f32::from_bits(self.radius_bits)
    }

    /// Сторона чанка для конвейера: `None` — «как решит [`chunk_grid`]», то
    /// есть [`POLYMESH_CHUNK_TARGET_METERS`] с возможностью переопределить
    /// окружением.
    fn chunk_meters(&self) -> Option<f32> {
        (!self.chunked).then_some(POLYMESH_FLAT_CHUNK_METERS)
    }
}

/// Построенный полигональный меш; `None`, пока идёт первая постройка — при
/// включённом по умолчанию `PolymeshDebug::enabled` она стартует на входе в
/// мир — или если панель выключили руками до того, как та закончилась.
#[derive(Resource, Default)]
pub struct PolyNavmesh {
    build: Option<Arc<PolymeshBuild>>,
    /// Счётчик завершённых построек — ключ кеша оверлея.
    generation: u32,
    /// Параметры, под которые построен текущий меш.
    built_for: BuildKey,
    /// Параметры летящей постройки и её таск.
    task: Option<(BuildKey, Task<Option<PolymeshBuild>>)>,
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
        self.built_for.radius()
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
/// прогонов (`examples/bench/polymesh_bench.rs`). Игровой путь идёт через
/// `sync_polymesh_build` и зовёт ровно тот же конвейер.
pub fn build_polymesh_from_map(map: &MapData, agent_radius: f32) -> Option<PolymeshBuild> {
    build_polymesh(&PolymeshInput::from_map(map), agent_radius, None, None)
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
    let key = BuildKey::new(&debug);
    if poly.build.is_some() && poly.built_for == key {
        return;
    }
    if let Some((in_flight, _)) = &poly.task
        && *in_flight == key
    {
        return;
    }
    poly.cancel_task();
    let input = PolymeshInput::from_map(&map);
    let started = Instant::now();
    let cancelled = Arc::new(AtomicBool::new(false));
    poly.cancelled = Some(cancelled.clone());
    let radius = key.radius();
    let chunk_meters = key.chunk_meters();
    poly.task = Some((
        key,
        AsyncComputeTaskPool::get().spawn(async move {
            let built = build_polymesh(&input, radius, Some(&cancelled), chunk_meters);
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
    let Some((key, task)) = silent.task.as_mut() else {
        return;
    };
    let key = *key;
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
    poly.built_for = key;
    poly.generation = poly.generation.wrapping_add(1);
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

    /// Стоит ли точка на меше — то есть свободна ли она для агента с его
    /// радиусом. Строже сеточной проходимости: контуры раздуты на радиус, и
    /// свободный тайл у стены на меше уже внутри препятствия.
    ///
    /// Слой подсказывается чанком, иначе локализация перебирает все ~140;
    /// отрицательный ответ перепроверяется без подсказки — точка ровно на шве
    /// принадлежит соседнему чанку по `floor`, и без второй попытки стоящий на
    /// границе считался бы замурованным.
    pub fn contains(&self, point: Vec2) -> bool {
        let chunk = self.chunk_at(point);
        self.mesh
            .point_in_mesh(polyanya::Coords::on_layer(to_poly(point), chunk as u8))
            || self.mesh.point_in_mesh(to_poly(point))
    }

    /// Ближайшая точка на меше в пределах допуска локализации (0.75 м) — тем же
    /// снапом, которым садятся на меш концы запроса. `None` — свободного места
    /// в допуске нет, то есть точка не «чуть внутри раздутого контура», а
    /// глубоко в препятствии.
    ///
    /// Спасению это дешёвая дорога: пешка, оказавшаяся внутри инфляции после
    /// постройки меша с новым радиусом, стоит от свободного места в сантиметрах,
    /// и кольцевой перебор тайлов с запросом в BVH на каждый был бы за неё
    /// на три порядка дороже одного снапа.
    pub fn nearest_free_point(&self, point: Vec2) -> Option<Vec2> {
        let chunk = self.chunk_at(point);
        self.mesh
            .get_closest_point(polyanya::Coords::on_layer(to_poly(point), chunk as u8))
            .or_else(|| self.mesh.get_closest_point(to_poly(point)))
            .map(|coords| from_poly(coords.position()))
    }

    /// Точка на меше и её узел графа. Слой подсказывается явно
    /// (`Coords::on_layer`): без подсказки локализация линейно перебирает все
    /// слои, а с ней идёт сразу в BVH нужного. Если в своём чанке точка не
    /// села — вторая попытка без подсказки: у самой кромки свободное место
    /// может оказаться уже за швом.
    ///
    /// Возвращаются **координаты посадки**, а не запрошенная точка: воронка
    /// считает от них, поэтому и полилиния обязана их содержать
    /// (см. [`find_path_polymesh`]).
    fn locate(&self, point: Vec2, towards: Vec2) -> Option<(polyanya::Coords, u32)> {
        let chunk = self.chunk_at(point);
        let coords = self
            .mesh
            .get_closest_point(polyanya::Coords::on_layer(to_poly(point), chunk as u8))
            .or_else(|| self.mesh.get_closest_point(to_poly(point)))
            // круговой допуск (0.75 м) не достал — идём от точки по прямой к
            // напарнику запроса шагами по `SEARCH_DELTA`: цель у стены здания
            // оказывается внутри раздутого контура, и ближайшее свободное место
            // лежит именно в ту сторону, откуда пешка придёт. Промежуток
            // проходкой не проверяется — для старта это значит «шагнуть к цели
            // сквозь то, что между нами», и держится только тем, что дальше
            // посадка входит в путь своей точкой
            .or_else(|| {
                self.mesh
                    .get_closest_point_towards(to_poly(point), to_poly(towards))
            })?;
        let layer = coords.layer()? as usize;
        let polygon = polygon_of(coords.polygon());
        let component = *self.components.get(layer)?.of_polygon.get(polygon)?;
        let node = *self.graph.node_of.get(layer)?.get(component as usize)?;
        Some((coords, node))
    }
}

#[cfg(test)]
mod tests;
