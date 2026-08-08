//! Конвейер поиска пути: заявка → очередь диспетчера с приоритетом и бюджетом
//! → асинхронный таск → снятый ответ, подрезанный под уже пройденное.
//! Ходьбу по готовому пути ведёт [`super::systems`].

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::window::PrimaryWindow;

use super::VIEW_MARGIN;
use super::systems::{Walkable, rescue_from_impassable};
use crate::determinism::{DeterministicRun, SimTick};
use crate::grid::tile_center;
use crate::movement::components::{
    Movable, MovableState, PathfindingRequest, PathfindingTask, PreviousSimPosition, RequestedAt,
    RetireAt, SimPosition,
};
use crate::navigation::{
    Pathfinder, PathfindingAlgorithm, PathfindingResult, find_path, find_path_northstar,
    find_path_polymesh,
};

/// Лимит одновременных pathfinding-тасков; остальные запросы ждут в очереди
/// и запускаются диспетчером по приоритету близости к камере.
///
/// Размер — под спрос на 30x: ~1000 бегущих перепрокладываются каждые
/// 0.7–1.2 виртуальных секунды, то есть ~27k заявок в реальную секунду.
/// Диспетчер выдаёт до лимита за кадр; 512 при ~55 fps давали ~14k/с — и
/// URGENT-заявки часами стояли в очереди, а пул (8 потоков × ~0.4 мс на
/// поиск) при этом скучал.
const MAX_PATHFINDING_IN_FLIGHT: usize = 1024;

/// То же для поиска по полигональному мешу — вчетверо меньше.
///
/// Меньше сеточного, потому что незавершённый полигональный поиск держит
/// фронт (см. `polymesh/path.rs::SEARCH_POPS_PER_POLYGON`), а сеточный —
/// ничего заметного. Но и не 32, как было в первой попытке удержать память
/// лимитом: сам по себе лимит от разрастания не спасает (взрывается один
/// запрос, а не их число), зато низкий намертво останавливает мир — пара
/// тяжёлых поисков занимает почти все слоты, и диспетчер перестаёт выдавать
/// заявки всем остальным. Память ограничена бюджетом шагов, слоты — здесь.
const MAX_POLYMESH_PATHFINDING_IN_FLIGHT: usize = MAX_PATHFINDING_IN_FLIGHT;

/// Приоритет заявки в очереди диспетчера, по возрастанию: чем меньше, тем
/// раньше считается путь.
mod priority {
    /// Демоны и паникующие люди: без пути они стоят на месте, и стоят рядом
    /// с демоном. Считаются вперёд всех мирных, в кадре или нет.
    pub const URGENT: u8 = 0;
    /// Мирно гуляющие в кадре — их видно, но ждать они могут.
    pub const WANDER_ON_SCREEN: u8 = 1;
}

/// Берёт ли диспетчер мирных гуляющих на таком зуме. На сильном отдалении
/// пешка — точка, её простой не виден, а «в кадре» — полкарты: без отсечки
/// полный зум-аут разом делает диспатчабельными все ~17k мирных, топит пул
/// тасков (URGENT встают за ними на кадры) и заставляет сортировать 17k заявок
/// каждый кадр.
///
/// Публична, потому что ждать таких заявок нельзя никому: прогрев
/// (`loading.rs::poll_warmup`) держал бы экран загрузки до таймаута с
/// неподвижным счётчиком — заявки, которых диспетчер не берёт, не закроются.
pub fn wanderers_dispatched_at_zoom(camera_scale: f32) -> bool {
    camera_scale < crate::settings::WANDER_DISPATCH_MAX_ZOOM
}

/// Запуск тасков поиска пути из очереди запросов. МИРНО гуляющие люди вне
/// экрана путь НЕ получают вовсе — их заявки ждут, пока камера не приедет;
/// демоны и убегающие люди обсчитываются всегда (иначе инвазия и паника за
/// кадром встанут) и первыми. Внутри приоритета — по удалённости от центра
/// кадра.
pub fn dispatch_pathfinding_requests(
    mut commands: Commands,
    pathfinder: Pathfinder,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    requests: Query<(
        Entity,
        &SimPosition,
        &PathfindingRequest,
        Has<crate::human::Human>,
        Has<crate::human::HumanFleeTag>,
    )>,
    tasks: Query<(), With<PathfindingTask>>,
) {
    // включённая панель Polymesh перекрывает выбор алгоритма — но только
    // когда меш уже построен; пока он строится (5–20 с), здесь `None`, и
    // запросы обслуживает сетка
    let polymesh = pathfinder.polymesh_build();
    let limit = if polymesh.is_some() {
        MAX_POLYMESH_PATHFINDING_IN_FLIGHT
    } else {
        MAX_PATHFINDING_IN_FLIGHT
    };
    let budget = limit.saturating_sub(tasks.iter().count());
    if budget == 0 || requests.is_empty() {
        return;
    }

    let camera_position = camera.translation.truncate();
    // масштаб камеры = мировых метров на логический пиксель
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * VIEW_MARGIN;
    let wanderers_visible_at_this_zoom = wanderers_dispatched_at_zoom(camera.scale.x);

    let mut queue: Vec<(u8, f32, Entity, Vec2, IVec2, IVec2)> = requests
        .iter()
        .filter_map(|(entity, sim_position, request, is_human, is_fleeing)| {
            let offset = (sim_position.0 - camera_position).abs();
            let on_screen = wanderers_visible_at_this_zoom
                && offset.x <= half_view.x
                && offset.y <= half_view.y;
            let priority = match (is_human && !is_fleeing, on_screen) {
                (false, _) => priority::URGENT,
                (true, true) => priority::WANDER_ON_SCREEN,
                // мирный за кадром — заявка ждёт камеру
                (true, false) => return None,
            };
            (
                priority,
                offset.length_squared(),
                entity,
                // полигональный поиск стартует из реальной позиции пешки, а
                // не из центра её тайла: снап старта к центру — ровно та
                // ступенька, ради избавления от которой всё и делается
                sim_position.0,
                request.start_tile,
                request.end_tile,
            )
                .into()
        })
        .collect();

    if queue.len() > budget {
        queue.sort_unstable_by(|a, b| {
            (a.0, a.1)
                .partial_cmp(&(b.0, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        queue.truncate(budget);
    }

    let algorithm = *pathfinder.algorithm;
    for (_, _, entity, start_world, start_tile, end_tile) in queue {
        let task = spawn_path_task(
            pathfinder.navmesh.0.clone(),
            pathfinder.northstar.get(),
            polymesh.clone(),
            algorithm,
            start_world,
            start_tile,
            end_tile,
        );
        commands
            .entity(entity)
            .remove::<PathfindingRequest>()
            .insert(PathfindingTask(task));
    }
}

/// Метка тика на новых заявках — ключ FIFO детерминированного диспетчера.
/// Отдельной системой, а не в `Movable::to_pathfinding`: тот вызывается из
/// поведения, которое о номере тика знать не обязано.
pub fn stamp_pathfinding_requests(
    mut commands: Commands,
    tick: Res<SimTick>,
    fresh: Query<Entity, Added<PathfindingRequest>>,
) {
    for entity in &fresh {
        commands.entity(entity).insert(RequestedAt(tick.0));
    }
}

/// Заявка в очереди детерминированного диспетчера.
///
/// Ключ отбора — `(requested_at, species, pawn_id)`, и он **уникален**.
///
/// Вид в ключе обязателен: `PawnId` — порядковый номер спавна **в пределах
/// вида**, поэтому демон №5 и человек №5 существуют одновременно, а срочная
/// очередь смешивает демонов с убегающими людьми. Без вида их ключи совпали
/// бы, и порядок между ними задавала бы нестабильная сортировка, то есть
/// порядок обхода запроса — ровно то, от чего режим обязан не зависеть.
/// `u32::MAX` достаётся одному только тестовому ходоку из `dev.rs`, демонов с
/// таким номером не бывает.
///
/// На уникальности держится детерминизм частичной выборки (см.
/// `take_within_budget`), поэтому она проверяется `debug_assert`.
pub struct QueuedRequest {
    requested_at: u64,
    species: u8,
    pawn_id: u32,
    units: u32,
    entity: Entity,
    start_world: Vec2,
    start_tile: IVec2,
    end_tile: IVec2,
}

impl QueuedRequest {
    fn key(&self) -> (u64, u8, u32) {
        (self.requested_at, self.species, self.pawn_id)
    }
}

/// Предсказанная цена поиска в единицах бюджета — целая, по расстоянию между
/// тайлами старта и цели (метрика Чебышёва).
///
/// Целочисленная намеренно: сумма float зависела бы от порядка слагаемых, а
/// порядок обхода запроса меняется от спавнов и смертей. См.
/// [`crate::settings::PATHFINDING_UNIT_TILES`] — там измеренные цены, из
/// которых выведен делитель.
fn request_units(start_tile: IVec2, end_tile: IVec2) -> u32 {
    let offset = (end_tile - start_tile).abs();
    let tiles = offset.x.max(offset.y).max(0);
    1 + (tiles / crate::settings::PATHFINDING_UNIT_TILES) as u32
}

/// Добавить заявку в буфер кандидатов, не давая ему разрастаться.
///
/// Заявок в очереди этого режима — вся неохваченная популяция, до двадцати
/// тысяч, и держать их все в буфере незачем: каждая стоит хотя бы одну
/// единицу, значит уехать может не больше `budget` штук. Буфер поэтому
/// подрезается O(n)-выборкой, как только перерастает вдвое, и в
/// установившемся ходе занимает пару сотен записей вместо двадцати тысяч.
///
/// Это не микрооптимизация: очередь длиной в популяцию — **штатное**
/// состояние режима (см. док-комментарий диспетчера), поэтому её обход идёт
/// каждый тик, а полная копия означала бы ~1 МБ записи в память на тик, то
/// есть 70 МБ/с на скорости 1× и вдвое больше на любой мелкой правке темпа.
fn push_within_budget(queue: &mut Vec<QueuedRequest>, request: QueuedRequest, budget: u32) {
    queue.push(request);
    let cap = budget as usize;
    if queue.len() > cap * 2 {
        queue.select_nth_unstable_by_key(cap, QueuedRequest::key);
        queue.truncate(cap);
    }
}

/// Отобрать из буфера то, что влезает в бюджет: первые по ключу, пока хватает
/// единиц.
///
/// Порядок работы, а не просто сортировка с обрезкой: единиц у заявок разное
/// число, поэтому сколько их пройдёт — заранее неизвестно. Но каждая стоит
/// хотя бы одну, значит пройдёт не больше `budget` штук.
///
/// Заявка, которая не влезла, обрывает набор, а не пропускается: иначе
/// дорогое поручение вечно уступало бы дешёвым прогулкам, приходящим следом,
/// и не уехало бы никогда. Оборвавшись, она остаётся первой в очереди
/// следующего тика.
fn take_within_budget(queue: &mut Vec<QueuedRequest>, budget: u32) {
    let cap = budget as usize;
    if queue.len() > cap {
        queue.select_nth_unstable_by_key(cap, QueuedRequest::key);
        queue.truncate(cap);
    }
    queue.sort_unstable_by_key(QueuedRequest::key);
    debug_assert!(
        queue.windows(2).all(|pair| pair[0].key() < pair[1].key()),
        "ключ очереди обязан быть уникален, иначе частичная выборка недетерминирована"
    );

    let mut spent = 0;
    let taken = queue
        .iter()
        .take_while(|request| {
            spent += request.units;
            spent <= budget
        })
        .count();
    queue.truncate(taken);
}

/// Диспетчер детерминированного режима: **камера не участвует**.
///
/// Обычный диспетчер сортирует по удалённости от центра кадра и мирных вне
/// экрана не берёт вовсе — то есть симуляция зависит от того, куда смотрит
/// игрок, и повтор прогона с другим положением камеры разошёлся бы. Здесь
/// вместо этого честная очередь по `(тик заявки, PawnId)`: кто подал раньше,
/// тот и уедет раньше. Это и есть детерминированная замена камерному гейту —
/// дальние ждут дольше, но воспроизводимо, а не потому, что игрок отвернулся.
///
/// Сколько уезжает за тик, задают **темпы выдачи** — отдельно срочным
/// (демоны, убегающие) и мирным, оба в единицах предсказанной цены поиска
/// (`crate::settings::PATHFINDING_*_UNITS_PER_TICK`). Темп отмерен по тому,
/// сколько пул успевает сжевать за тик, а не по тому, как быстро хочется
/// опустошить очередь, и это принципиально: 16 000 поручений через город —
/// 85 с процессорного времени, и выданные разом они не считаются быстрее, они
/// просто останавливают кадр в `apply_pathfinding_results`, который их ждёт.
///
/// `MAX_*_IN_FLIGHT` здесь не при делах, и подставлять их сюда нельзя. В
/// обычном режиме это потолок ОДНОВРЕМЕННЫХ поисков за кадр, и он отмерен под
/// очередь, из которой гейт видимости уже выбросил ~97% заявок (реально
/// уходит ~9 поисков на кадр). Здесь гейта нет, заявки подают все 20 000
/// пешек, и та же тысяча означала бы совсем другую величину — до 65 000
/// поисков в реальную секунду.
///
/// Потолок «в полёте» остаётся страховкой: при тик-точном снятии в полёте по
/// построению ровно пачки последних `PATHFINDING_RETIRE_TICKS` тиков, то есть
/// в штатном ходе он не срабатывает никогда.
pub fn dispatch_pathfinding_requests_deterministic(
    mut commands: Commands,
    mut queues: Local<(Vec<QueuedRequest>, Vec<QueuedRequest>)>,
    run: Res<DeterministicRun>,
    arc_navmesh: Res<crate::navigation::ArcNavmesh>,
    tick: Res<SimTick>,
    requests: Query<(
        Entity,
        &SimPosition,
        &PathfindingRequest,
        &RequestedAt,
        // `Option`: `PawnId` есть у всех пешек симуляции, но не у тестового
        // ходока из `dev.rs`. Без `Option` его заявка молча не диспетчилась
        // бы вовсе, и он бы застыл — а так он просто последний в очереди
        Option<&crate::rng::PawnId>,
        Has<crate::human::Human>,
        Has<crate::human::HumanFleeTag>,
    )>,
    tasks: Query<(), With<PathfindingTask>>,
) {
    let in_flight_cap = (crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK
        + crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK) as usize
        * crate::settings::PATHFINDING_RETIRE_TICKS as usize;
    if requests.is_empty() || tasks.iter().count() >= in_flight_cap {
        return;
    }

    let (urgent, wander) = &mut *queues;
    // буферы переиспользуются между тиками: при длинной очереди (штатное
    // состояние этого режима) аллокация на каждый тик была бы заметнее самой
    // выборки
    urgent.clear();
    wander.clear();
    for (entity, sim_position, request, requested_at, pawn_id, is_human, is_fleeing) in &requests {
        let queued = QueuedRequest {
            requested_at: requested_at.0,
            species: u8::from(is_human),
            pawn_id: pawn_id.map_or(u32::MAX, |pawn_id| pawn_id.0),
            units: request_units(request.start_tile, request.end_tile),
            entity,
            // полигональный поиск стартует из реальной позиции пешки, а не из
            // центра её тайла: снап старта к центру — ровно та ступенька,
            // ради избавления от которой всё и делается
            start_world: sim_position.0,
            start_tile: request.start_tile,
            end_tile: request.end_tile,
        };
        if is_human && !is_fleeing {
            push_within_budget(
                wander,
                queued,
                crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK,
            );
        } else {
            push_within_budget(
                urgent,
                queued,
                crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK,
            );
        }
    }

    take_within_budget(urgent, crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK);
    take_within_budget(wander, crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK);

    let retire_at = tick.0 + crate::settings::PATHFINDING_RETIRE_TICKS;
    for request in urgent.iter().chain(wander.iter()) {
        let task = spawn_path_task(
            arc_navmesh.0.clone(),
            run.northstar.clone(),
            run.polymesh.clone(),
            run.algorithm,
            request.start_world,
            request.start_tile,
            request.end_tile,
        );
        commands
            .entity(request.entity)
            .remove::<(PathfindingRequest, RequestedAt)>()
            .insert((PathfindingTask(task), RetireAt(retire_at)));
    }
}

/// Приёмник детерминированного режима: снимает ровно те ответы, чей срок
/// настал на этом тике, — и ждёт их, если поиск ещё не закончился.
///
/// Ожидание (`block_on`) и есть смысл системы: без него момент, когда пешка
/// трогается с места, задавался бы тем, как быстро ОС домолотила задачу, то
/// есть загрузкой машины и частотой кадров. Медленная машина здесь замедляет
/// проигрывание, но не меняет содержимое тика.
///
/// Порядок обработки — по `PawnId`: обход запроса зависит от порядка спавна и
/// смертей, а применение ответа шлёт команды и трогает сетку людей.
#[allow(clippy::too_many_arguments)]
pub fn apply_pathfinding_results(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    // бэкенд — из замороженного снимка, а не из живого `Pathfinder`: иначе
    // достроившийся посреди прогона polymesh поменял бы проверку
    // проходимости под спасением застрявших
    run: Res<DeterministicRun>,
    arc_navmesh: Res<crate::navigation::ArcNavmesh>,
    mut load: ResMut<crate::sim_time::SimLoad>,
    tick: Res<SimTick>,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut tasks: Query<(
        Entity,
        Option<&crate::rng::PawnId>,
        &mut Movable,
        &mut SimPosition,
        &mut PreviousSimPosition,
        &mut PathfindingTask,
        &RetireAt,
        Has<crate::human::Human>,
    )>,
) {
    // `<=`, а не `==`: в штатном ходе тики идут подряд и срок совпадает
    // точно, но пропущенный тик (режим переключили, состояние сменилось)
    // оставил бы таск висеть навсегда — пешка застыла бы в `Pathfinding`, а
    // слот навсегда ушёл из бюджета. На детерминизм это не влияет: срок
    // вычисляется детерминированно, и подстраховка срабатывает только там,
    // где штатного хода уже не было
    // Вид перед номером — по той же причине, что и в очереди диспетчера:
    // `PawnId` уникален лишь внутри вида, а срок настаёт разом и у демонов, и
    // у людей. Без вида ничью разрешал бы `Entity`, а индексы сущностей
    // переиспользуются после смертей и рестарта в другом порядке.
    let mut due: Vec<(u8, u32, Entity)> = tasks
        .iter()
        .filter(|(.., retire_at, _)| retire_at.0 <= tick.0)
        .map(|(entity, pawn_id, .., is_human)| {
            (
                u8::from(is_human),
                pawn_id.map_or(u32::MAX, |pawn_id| pawn_id.0),
                entity,
            )
        })
        .collect();
    if due.is_empty() {
        return;
    }
    due.sort_unstable();

    let mut answered = 0u32;
    let mut failed = 0u32;
    let polymesh = run.polymesh.clone();
    let mut navmesh = None;
    for (_, _, entity) in due {
        let Ok((entity, _, mut movable, mut sim_position, mut previous, mut task, _, is_human)) =
            tasks.get_mut(entity)
        else {
            continue;
        };
        // простой главного потока замеряется отдельно от его работы: регулятор
        // скорости обязан их различать — работа от скорости не зависит,
        // а ожидание зависит прямо (см. `sim_time::SimLoad::observe`)
        let waited = std::time::Instant::now();
        let result = bevy::tasks::block_on(&mut task.0);
        load.add_frame_cost(waited.elapsed());
        commands
            .entity(entity)
            .remove::<(PathfindingTask, RetireAt)>();
        diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_DURATION_MS, || {
            result.duration.as_secs_f64() * 1000.0
        });
        answered += 1;
        failed += u32::from(result.path.is_none());

        apply_result(
            result,
            entity,
            &mut movable,
            &mut sim_position,
            &mut previous,
            is_human,
            &mut navmesh,
            &arc_navmesh,
            polymesh.as_deref(),
            &mut human_grid,
            &mut commands,
        );
    }

    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_ANSWERED, || {
        answered as f64
    });
    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_FAILED, || failed as f64);
}

/// Сам таск поиска пути — общий для обоих диспетчеров, чтобы режимы не
/// разъехались в том, ЧТО именно считается.
#[allow(clippy::too_many_arguments)]
fn spawn_path_task(
    navmesh: std::sync::Arc<std::sync::RwLock<crate::navigation::Navmesh>>,
    northstar: Option<std::sync::Arc<bevy_northstar::prelude::OrdinalGrid>>,
    polymesh: Option<std::sync::Arc<crate::navigation::PolymeshBuild>>,
    algorithm: PathfindingAlgorithm,
    start_world: Vec2,
    start_tile: IVec2,
    end_tile: IVec2,
) -> bevy::tasks::Task<PathfindingResult> {
    bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let (path, started_at) = match polymesh {
            Some(polymesh) => {
                let started_at = std::time::Instant::now();
                // цель осталась тайловой (её выбрало поведение по
                // проходимости сетки) — на меше это её центр
                let path = find_path_polymesh(&polymesh, start_world, tile_center(end_tile));
                (path, started_at)
            }
            None => {
                let (tiles, started_at) = grid_path(
                    &navmesh,
                    northstar.as_deref(),
                    algorithm,
                    start_tile,
                    end_tile,
                );
                let path =
                    tiles.map(|tiles| tiles.into_iter().map(tile_center).collect::<Vec<Vec2>>());
                (path, started_at)
            }
        };
        PathfindingResult {
            start_tile,
            end_tile,
            path,
            duration: started_at.elapsed(),
        }
    })
}

/// Сеточный поиск: иерархия northstar, если она построена, иначе плоский
/// алгоритм. Возвращает путь в тайлах и момент старта самого поиска — метрика
/// не должна включать ожидание `RwLock`.
fn grid_path(
    navmesh: &std::sync::RwLock<crate::navigation::Navmesh>,
    northstar: Option<&bevy_northstar::prelude::OrdinalGrid>,
    algorithm: PathfindingAlgorithm,
    start_tile: IVec2,
    end_tile: IVec2,
) -> (Option<Vec<IVec2>>, std::time::Instant) {
    let hierarchical = matches!(
        algorithm,
        PathfindingAlgorithm::Hpa | PathfindingAlgorithm::ThetaStar
    );
    if let Some(grid) = northstar.filter(|_| hierarchical) {
        let started_at = std::time::Instant::now();
        let path = find_path_northstar(
            grid,
            start_tile,
            end_tile,
            algorithm == PathfindingAlgorithm::ThetaStar,
        );
        return (path, started_at);
    }
    // сетка northstar ещё строится — до её готовности иерархические
    // алгоритмы обслуживает A*
    let algorithm = if hierarchical {
        PathfindingAlgorithm::Astar
    } else {
        algorithm
    };
    let navmesh = navmesh.read().unwrap();
    // после захвата лока: метрика — сам поиск, без RwLock
    let started_at = std::time::Instant::now();
    (
        find_path(&navmesh, start_tile, end_tile, algorithm),
        started_at,
    )
}

/// Сколько ведущих waypoint'ов нового пути можно срезать. Пока путь считался,
/// сущность шла по старому и докатывала за его концом: на 30x ответ опаздывает
/// на 1–1.5 виртуальных секунды, это 4–6 тайлов от стартового тайла заявки.
/// Каждый срез и так гейтится геометрией («следующий waypoint не дальше
/// текущего»); лимит — страховка от спрямления угла сквозь стену. На
/// полигональном пути гейт почти не срабатывает: его waypoint'ы — углы
/// препятствий, и следующий угол дальше предыдущего, пока тот не пройден.
const REPATH_TRIM_LIMIT: usize = 4;

/// Снимает готовые асинхронные ответы поиска пути.
///
/// Здесь же ловятся застрявшие. Провал поиска — единственный сигнал, который
/// сущность подаёт о себе сама, и он же отбирает ровно тех, кому спасение
/// нужно: плоский A* стартовый тайл не проверяет и из дома в тайл-другой
/// выводит сам, полигональный меш снапит старт на меш, а `None` возвращается,
/// когда выхода действительно нет — все восемь соседей непроходимы либо старт
/// не принадлежит ни одному чанку иерархии. Поэтому вместо периодического
/// прохода по всем 20 000 сущностей проверка стоит на ответе: один индекс в
/// `Vec` проходимости на каждый провал (сотые доли процента кадра), а кольцевой
/// поиск — только за теми, кто действительно в непроходимом.
pub fn listen_for_pathfinding_tasks(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    pathfinder: Pathfinder,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut tasks: Query<(
        Entity,
        &mut Movable,
        &mut SimPosition,
        &mut PreviousSimPosition,
        &mut PathfindingTask,
        Has<crate::human::Human>,
    )>,
) {
    let mut answered = 0u32;
    let mut failed = 0u32;
    let polymesh = pathfinder.polymesh_build();
    // read-лок берётся лениво и только под спасение: во время загрузки его на
    // секунды держит на запись поток заливки
    let mut navmesh = None;
    for (entity, mut movable, mut sim_position, mut previous, mut task, is_human) in &mut tasks {
        let Some(result) = check_ready(&mut task.0) else {
            continue;
        };
        commands.entity(entity).remove::<PathfindingTask>();
        diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_DURATION_MS, || {
            result.duration.as_secs_f64() * 1000.0
        });
        answered += 1;
        failed += u32::from(result.path.is_none());

        apply_result(
            result,
            entity,
            &mut movable,
            &mut sim_position,
            &mut previous,
            is_human,
            &mut navmesh,
            &pathfinder.navmesh,
            polymesh.as_deref(),
            &mut human_grid,
            &mut commands,
        );
    }

    // оба счётчика пишутся каждый кадр, в том числе нулями: доля считается в
    // UI как отношение средних, и это верно только пока обе истории одной
    // длины (см. `diagnostics::PATHFINDING_ANSWERED`)
    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_ANSWERED, || {
        answered as f64
    });
    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_FAILED, || failed as f64);
}

/// Применение ОДНОГО ответа поиска пути — общее тело обоих приёмников.
///
/// Вынесено намеренно: режимы отличаются только тем, КОГДА ответ снимается, а
/// не тем, что с ним делают. Две копии этой логики разъехались бы на первой же
/// правке, и детерминированный режим начал бы вести себя иначе не потому, что
/// он детерминированный.
///
/// `navmesh_lock` — ленивый read-лок: во время загрузки его на секунды держит
/// на запись поток заливки, а нужен он только под спасение застрявших.
#[allow(clippy::too_many_arguments)]
fn apply_result<'lock>(
    result: PathfindingResult,
    entity: Entity,
    movable: &mut Movable,
    sim_position: &mut SimPosition,
    previous: &mut PreviousSimPosition,
    is_human: bool,
    navmesh_lock: &mut Option<std::sync::RwLockReadGuard<'lock, crate::navigation::Navmesh>>,
    arc_navmesh: &'lock crate::navigation::ArcNavmesh,
    polymesh: Option<&crate::navigation::PolymeshBuild>,
    human_grid: &mut Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    commands: &mut Commands,
) {
    let MovableState::Pathfinding(end_tile) = movable.state else {
        return;
    };
    // устаревший ответ — уже запрошена другая цель
    if end_tile != result.end_tile {
        return;
    }

    let Some(path) = result.path else {
        let navmesh = navmesh_lock.get_or_insert_with(|| arc_navmesh.read());
        let walkable = Walkable { navmesh, polymesh };
        if rescue_from_impassable(&walkable, entity, movable, sim_position, previous, commands) {
            if is_human && let Some(grid) = human_grid.as_mut() {
                grid.insert(entity, sim_position.0);
            }
            return;
        }
        // не застрял — цель просто недостижима; новую выберет поведение
        movable.to_pathfinding_error(entity, end_tile, commands);
        return;
    };

    // путь всегда включает стартовую точку; один элемент — мы уже на месте
    if path.len() == 1 {
        movable.to_idle(entity, commands, true);
        return;
    }

    // перепрокладка шла на ходу, и сущность уже не в стартовой точке:
    // срезаем начало пути, пока следующий waypoint не дальше текущего —
    // иначе первый шаг был бы назад
    let mut path: std::collections::VecDeque<Vec2> = path.into_iter().skip(1).collect();
    let position = sim_position.0;
    let mut trimmed = 0;
    while trimmed < REPATH_TRIM_LIMIT
        && path.len() >= 2
        && position.distance_squared(path[1]) <= position.distance_squared(path[0])
    {
        path.pop_front();
        trimmed += 1;
    }
    movable.to_moving(end_tile, path, entity, commands);
}
