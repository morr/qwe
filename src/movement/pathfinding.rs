//! Конвейер поиска пути: заявка → очередь диспетчера с приоритетом и бюджетом
//! → асинхронный таск → снятый ответ, подрезанный под уже пройденное.
//! Ходьбу по готовому пути ведёт [`super::systems`].

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::window::PrimaryWindow;

use super::VIEW_MARGIN;
use super::systems::rescue_from_impassable;
use crate::camera::Viewport;
use crate::determinism::SimTick;
use crate::movement::components::{
    Movable, MovableState, PathfindingRequest, PathfindingTask, PawnEdit, PreviousSimPosition,
    RequestedAt, RetireAt, SimPosition,
};
use crate::navigation::{Backend, PathfindingResult, Walkable};
use crate::settings::{MAX_PATHFINDING_IN_FLIGHT, REPATH_TRIM_LIMIT};

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

/// Место заявки в очереди: приоритет, затем удалённость от центра кадра.
/// `None` — заявку не брать вовсе: мирный гуляющий вне кадра ждёт камеру.
///
/// Отдельно от системы, потому что это всё правило видимости целиком, а
/// система вокруг него — только сборка очереди и её усечение под бюджет.
fn queue_key(view: &Viewport, position: Vec2, urgent: bool) -> Option<(u8, f32)> {
    let distance = view.distance_from_centre_squared(position);
    if urgent {
        return Some((priority::URGENT, distance));
    }
    if !wanderers_dispatched_at_zoom(view.zoom) || !view.contains(position) {
        return None;
    }
    Some((priority::WANDER_ON_SCREEN, distance))
}

/// Запуск тасков поиска пути из очереди запросов. МИРНО гуляющие люди вне
/// экрана путь НЕ получают вовсе — их заявки ждут, пока камера не приедет;
/// демоны и убегающие люди обсчитываются всегда (иначе инвазия и паника за
/// кадром встанут) и первыми. Внутри приоритета — по удалённости от центра
/// кадра.
pub fn dispatch_pathfinding_requests(
    mut commands: Commands,
    backend: Res<Backend>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    requests: Query<(
        Entity,
        &SimPosition,
        &PathfindingRequest,
        Has<super::components::UrgentPath>,
    )>,
    tasks: Query<(), With<PathfindingTask>>,
) {
    let budget = MAX_PATHFINDING_IN_FLIGHT.saturating_sub(tasks.iter().count());
    if budget == 0 || requests.is_empty() {
        return;
    }

    let view = Viewport::of(&window, &camera, VIEW_MARGIN);

    let mut queue: Vec<(u8, f32, Entity, Vec2, IVec2, IVec2)> = requests
        .iter()
        .filter_map(|(entity, sim_position, request, urgent)| {
            let (priority, distance) = queue_key(&view, sim_position.0, urgent)?;
            (
                priority,
                distance,
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

    for (_, _, entity, start_world, start_tile, end_tile) in queue {
        let task = spawn_path_task((*backend).clone(), start_world, start_tile, end_tile);
        commands
            .entity(entity)
            .remove::<PathfindingRequest>()
            .insert(PathfindingTask::new(task));
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
/// Ключ отбора — тик заявки, а ничью на нём разрывает
/// [`pawn_key`](super::order::pawn_key); ключ целиком **уникален**.
///
/// На уникальности держится детерминизм частичной выборки (см.
/// `take_within_budget`), поэтому она проверяется `debug_assert`.
pub struct QueuedRequest {
    requested_at: u64,
    pawn: (u8, u32),
    units: u32,
    entity: Entity,
    start_world: Vec2,
    start_tile: IVec2,
    end_tile: IVec2,
}

impl QueuedRequest {
    fn key(&self) -> (u64, u8, u32) {
        (self.requested_at, self.pawn.0, self.pawn.1)
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
    // сущности в сообщении обязательны: ключ называет вид и номер, но не
    // говорит, КТО их занял, — а чинить столкновение двух демонов и
    // столкновение демона с не-демоном нужно в разных местах
    debug_assert!(
        queue.windows(2).all(|pair| pair[0].key() < pair[1].key()),
        "ключ очереди обязан быть уникален, иначе частичная выборка недетерминирована; \
         столкнулись {:?}",
        queue
            .windows(2)
            .find(|pair| pair[0].key() >= pair[1].key())
            .map(|pair| (
                (pair[0].entity, pair[0].key()),
                (pair[1].entity, pair[1].key())
            ))
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
    backend: Res<Backend>,
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
        Option<&crate::rng::Species>,
        Has<super::components::UrgentPath>,
    )>,
    tasks: Query<(), With<PathfindingTask>>,
) {
    let in_flight_cap = (crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK
        + crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK) as usize
        * crate::settings::PATHFINDING_RETIRE_TICKS as usize;
    if requests.is_empty() || tasks.iter().count() >= in_flight_cap {
        return;
    }

    let (urgent_queue, wander) = &mut *queues;
    // буферы переиспользуются между тиками: при длинной очереди (штатное
    // состояние этого режима) аллокация на каждый тик была бы заметнее самой
    // выборки
    urgent_queue.clear();
    wander.clear();
    for (entity, sim_position, request, requested_at, pawn_id, species, urgent) in &requests {
        let queued = QueuedRequest {
            requested_at: requested_at.0,
            pawn: super::order::pawn_key(species, pawn_id),
            units: request_units(request.start_tile, request.end_tile),
            entity,
            // полигональный поиск стартует из реальной позиции пешки, а не из
            // центра её тайла: снап старта к центру — ровно та ступенька,
            // ради избавления от которой всё и делается
            start_world: sim_position.0,
            start_tile: request.start_tile,
            end_tile: request.end_tile,
        };
        if urgent {
            push_within_budget(
                urgent_queue,
                queued,
                crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK,
            );
        } else {
            push_within_budget(
                wander,
                queued,
                crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK,
            );
        }
    }

    take_within_budget(
        urgent_queue,
        crate::settings::PATHFINDING_URGENT_UNITS_PER_TICK,
    );
    take_within_budget(wander, crate::settings::PATHFINDING_WANDER_UNITS_PER_TICK);

    let retire_at = tick.0 + crate::settings::PATHFINDING_RETIRE_TICKS;
    for request in urgent_queue.iter().chain(wander.iter()) {
        let task = spawn_path_task(
            (*backend).clone(),
            request.start_world,
            request.start_tile,
            request.end_tile,
        );
        commands
            .entity(request.entity)
            .remove::<(PathfindingRequest, RequestedAt)>()
            .insert((PathfindingTask::new(task), RetireAt(retire_at)));
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
    // в этом режиме ресурс заморожен на весь прогон (записан один раз на
    // `WorldStarted`): иначе достроившийся посреди прогона polymesh
    // поменял бы проверку проходимости под спасением застрявших
    backend: Res<Backend>,
    mut load: ResMut<crate::sim_time::SimLoad>,
    tick: Res<SimTick>,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut tasks: Query<(
        Entity,
        Option<&crate::rng::PawnId>,
        // вид — для ключа порядка; `Has<Human>` ниже — совсем про другое: про
        // то, надо ли поправить человеческую пространственную сетку
        Option<&crate::rng::Species>,
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
    // срок настаёт разом у демонов и у людей, так что порядок держит общий
    // ключ пешки (`order::pawn_key`), а не обход запроса
    let mut due: Vec<(u8, u32, Entity)> = tasks
        .iter()
        .filter(|(.., retire_at, _)| retire_at.0 <= tick.0)
        .map(|(entity, pawn_id, pawn_species, ..)| {
            let (species, number) = super::order::pawn_key(pawn_species, pawn_id);
            (species, number, entity)
        })
        .collect();
    due.sort_unstable();

    // счётчики заводятся ДО обхода и пишутся его `Drop`'ом: тик, на котором
    // ничего не подошло к сроку, обязан записать нули, иначе панель застынет
    // на последнем значении (см. [`AnswerTally`])
    let mut tally = AnswerTally::new(&mut diagnostics);
    let mut walkable = None;
    for (_, _, entity) in due {
        let Ok((entity, _, _, mut movable, mut sim_position, mut previous, mut task, _, is_human)) =
            tasks.get_mut(entity)
        else {
            continue;
        };
        // простой главного потока замеряется отдельно от его работы: регулятор
        // скорости обязан их различать — работа от скорости не зависит,
        // а ожидание зависит прямо (см. `sim_time::SimLoad::observe`)
        let waited = std::time::Instant::now();
        let result = bevy::tasks::block_on(&mut task.task);
        load.add_frame_cost(waited.elapsed());
        commands
            .entity(entity)
            .remove::<(PathfindingTask, RetireAt)>();
        accept_answer(
            result,
            &mut tally,
            PawnEdit {
                entity,
                movable: &mut movable,
                sim_position: &mut sim_position,
                previous: &mut previous,
                commands: &mut commands,
            },
            is_human,
            &mut walkable,
            &backend,
            &mut human_grid,
        );
    }
}

/// Сам таск поиска пути — общий для обоих диспетчеров. ЧТО именно считается,
/// решает унесённый в таск снимок бэкенда ([`Backend::search`]) — режимы
/// отличаются только тем, КОГДА снимается ответ.
fn spawn_path_task(
    backend: Backend,
    start_world: Vec2,
    start_tile: IVec2,
    end_tile: IVec2,
) -> bevy::tasks::Task<PathfindingResult> {
    bevy::tasks::AsyncComputeTaskPool::get()
        .spawn(async move { backend.search(start_world, start_tile, end_tile) })
}

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
    backend: Res<Backend>,
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
    let mut tally = AnswerTally::new(&mut diagnostics);
    // взгляд на проходимость берётся лениво и только под спасение: его
    // read-лок во время загрузки на секунды держит на запись поток заливки
    let mut walkable = None;
    for (entity, mut movable, mut sim_position, mut previous, mut task, is_human) in &mut tasks {
        let Some(result) = check_ready(&mut task.task) else {
            // Сторожок второго эшелона: бюджет внутри polymesh-поиска ловит
            // расходящуюся воронку, а этот порог — любой другой способ
            // зависнуть (лок, дедлок, бесконечный цикл в бэкенде). Живой
            // поиск отвечает за миллисекунды; порог щедрый, потому что при
            // насыщенном пуле здоровый таск может простоять в очереди
            // исполнителя единицы секунд.
            assert!(
                task.spawned_at.elapsed().as_secs_f32()
                    < crate::settings::PATHFINDING_TASK_HANG_SECS,
                "pathfinding task hung: {entity} at {:?} has waited {:?} in state {:?} — \
                 the search backend is stuck, not merely slow",
                sim_position.0,
                task.spawned_at.elapsed(),
                movable.state,
            );
            continue;
        };
        commands.entity(entity).remove::<PathfindingTask>();
        accept_answer(
            result,
            &mut tally,
            PawnEdit {
                entity,
                movable: &mut movable,
                sim_position: &mut sim_position,
                previous: &mut previous,
                commands: &mut commands,
            },
            is_human,
            &mut walkable,
            &backend,
            &mut human_grid,
        );
    }
}

/// Счётчики приёмки за один прогон системы: замер длительности поиска и пара
/// «ответов / отказов».
///
/// Пишутся в [`Drop`], а не в конце системы, и это единственная причина, по
/// которой тип существует. Инвариант панели — **оба счётчика каждый прогон, в
/// том числе нулями**: доля считается как отношение средних по двум историям,
/// и верно это, только пока истории одной длины (см.
/// [`crate::diagnostics::PATHFINDING_ANSWERED`]). Дисциплиной он уже не
/// удержался дважды: сперва замер писали только в кадрах с ответами и панель
/// застывала на последнем значении, потом ранний выход
/// `if due.is_empty() { return; }` вернул ровно то же в детерминированном
/// приёмнике. `Drop` делает нарушение невыразимым — из системы нельзя выйти
/// мимо него.
struct AnswerTally<'a, 'w, 's> {
    diagnostics: &'a mut bevy::diagnostic::Diagnostics<'w, 's>,
    answered: u32,
    failed: u32,
}

impl<'a, 'w, 's> AnswerTally<'a, 'w, 's> {
    fn new(diagnostics: &'a mut bevy::diagnostic::Diagnostics<'w, 's>) -> Self {
        Self {
            diagnostics,
            answered: 0,
            failed: 0,
        }
    }

    /// Учесть один снятый ответ — чем бы он ни кончился дальше: устаревший и
    /// недостижимый ответы тоже посчитаны, они стоили поиска.
    fn record(&mut self, result: &PathfindingResult) {
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_DURATION_MS, || {
                result.duration.as_secs_f64() * 1000.0
            });
        self.answered += 1;
        self.failed += u32::from(result.path.is_none());
    }
}

impl Drop for AnswerTally<'_, '_, '_> {
    fn drop(&mut self) {
        let (answered, failed) = (self.answered, self.failed);
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_ANSWERED, || {
                answered as f64
            });
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_FAILED, || failed as f64);
    }
}

/// Приёмка ОДНОГО ответа поиска пути — общее тело обоих приёмников: учёт в
/// счётчиках и применение к пешке.
///
/// Вынесено намеренно: режимы отличаются только тем, КОГДА ответ снимается, а
/// не тем, что с ним делают. Две копии этой логики разъехались бы на первой же
/// правке, и детерминированный режим начал бы вести себя иначе не потому, что
/// он детерминированный. Что у режимов и правда своё — способ добыть ответ
/// (`check_ready` со сторожком зависания против `block_on` со счётом простоя в
/// `SimLoad`) и порядок обхода; всё остальное живёт здесь.
///
/// `walkable` — ленивый взгляд на проходимость бэкенда: его read-лок во время
/// загрузки на секунды держит на запись поток заливки, а нужен он только под
/// спасение застрявших.
///
/// `pawn` — [`PawnEdit`]: тождество, три компонента, которые спасение правит
/// вместе, и буфер команд. Ровно то же принимает `rescue_from_impassable`,
/// которому эта приёмка пешку и передаёт.
fn accept_answer<'backend>(
    result: PathfindingResult,
    tally: &mut AnswerTally<'_, '_, '_>,
    mut pawn: PawnEdit<'_, '_, '_>,
    is_human: bool,
    walkable: &mut Option<Walkable<'backend>>,
    backend: &'backend Backend,
    human_grid: &mut Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
) {
    tally.record(&result);

    let MovableState::Pathfinding(end_tile) = pawn.movable.state else {
        return;
    };
    // устаревший ответ — уже запрошена другая цель
    if end_tile != result.end_tile {
        return;
    }

    let Some(path) = result.path else {
        let walkable = walkable.get_or_insert_with(|| backend.walkable());
        if rescue_from_impassable(walkable, &mut pawn) {
            if is_human && let Some(grid) = human_grid.as_mut() {
                grid.insert(pawn.entity, pawn.sim_position.0);
            }
            return;
        }
        // не застрял — цель просто недостижима; новую выберет поведение
        pawn.movable
            .to_pathfinding_error(pawn.entity, end_tile, pawn.commands);
        return;
    };

    // путь всегда включает стартовую точку; один элемент — мы уже на месте
    if path.len() == 1 {
        // старый путь снимается ЗДЕСЬ: перепрокладка шла на ходу и его не
        // сбрасывала, а `to_idle` объявляет приход только при пустом пути —
        // иначе событие теряется молча (то же делает `step.rs` перед своим
        // `to_idle`, и по той же причине)
        pawn.movable.path.clear();
        pawn.movable.to_idle(pawn.entity, pawn.commands, true);
        return;
    }

    // перепрокладка шла на ходу, и сущность уже не в стартовой точке:
    // срезаем начало пути, пока следующий waypoint не дальше текущего —
    // иначе первый шаг был бы назад
    let mut path: std::collections::VecDeque<Vec2> = path.into_iter().skip(1).collect();
    let position = pawn.sim_position.0;
    let mut trimmed = 0;
    while trimmed < REPATH_TRIM_LIMIT
        && path.len() >= 2
        && position.distance_squared(path[1]) <= position.distance_squared(path[0])
    {
        path.pop_front();
        trimmed += 1;
    }
    pawn.movable
        .to_moving(end_tile, path, pawn.entity, pawn.commands);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WANDER_DISPATCH_MAX_ZOOM;

    /// Кадр 1000×600 логических пикселей вокруг начала координат: на зуме 0.1
    /// это ±50×30 метров, на зуме общего плана — на порядок больше.
    fn view(zoom: f32) -> Viewport {
        let mut window = Window::default();
        window.resolution.set(1000.0, 600.0);
        Viewport::of(
            &window,
            &Transform::from_scale(Vec3::splat(zoom)),
            VIEW_MARGIN,
        )
    }

    const CLOSE: f32 = 0.1;
    const NEAR_ZOOM: f32 = 0.1;
    /// Достаточно далеко, чтобы не попасть в кадр ни на каком из зумов ниже.
    const OFF_SCREEN: Vec2 = Vec2::new(100_000.0, 0.0);

    // Демон и убегающий человек подают диспетчеру **одно и то же** — оба
    // носят `UrgentPath`, и в этом всё содержание правки: раньше диспетчер
    // выводил их срочность сам, из пары видовых тегов. Имена оставлены
    // раздельными, потому что проверки читаются ими, а не булевым литералом.
    fn demon(view: &Viewport, position: Vec2) -> Option<(u8, f32)> {
        queue_key(view, position, true)
    }
    fn wanderer(view: &Viewport, position: Vec2) -> Option<(u8, f32)> {
        queue_key(view, position, false)
    }
    fn fleeing(view: &Viewport, position: Vec2) -> Option<(u8, f32)> {
        queue_key(view, position, true)
    }

    /// Заявка мирного гуляющего вне кадра не выбрасывается, а ждёт камеру, —
    /// и это единственный случай, когда диспетчер вообще проходит мимо.
    #[test]
    fn a_peaceful_wanderer_off_screen_waits_for_the_camera() {
        let view = view(NEAR_ZOOM);
        assert_eq!(wanderer(&view, OFF_SCREEN), None);
        assert_eq!(
            wanderer(&view, Vec2::new(CLOSE, CLOSE)).map(|(priority, _)| priority),
            Some(priority::WANDER_ON_SCREEN)
        );
    }

    /// Демон и паникующий человек считаются везде: без пути они стоят, и
    /// стоят рядом с демоном — инвазия за кадром обязана идти.
    #[test]
    fn demons_and_fleeing_humans_dispatch_off_screen_too() {
        let view = view(NEAR_ZOOM);
        assert_eq!(
            demon(&view, OFF_SCREEN).map(|(priority, _)| priority),
            Some(priority::URGENT)
        );
        assert_eq!(
            fleeing(&view, OFF_SCREEN).map(|(priority, _)| priority),
            Some(priority::URGENT)
        );
    }

    /// Срочные идут раньше мирных в кадре, даже когда стоят дальше от центра.
    #[test]
    fn the_urgent_outrank_a_wanderer_standing_nearer_the_centre() {
        let view = view(NEAR_ZOOM);
        let far_demon = demon(&view, Vec2::new(10.0, 0.0)).unwrap();
        let near_wanderer = wanderer(&view, Vec2::new(CLOSE, 0.0)).unwrap();
        assert!(far_demon < near_wanderer);
    }

    /// Внутри приоритета — ближе к центру кадра раньше.
    #[test]
    fn within_a_priority_the_nearer_to_the_centre_goes_first() {
        let view = view(NEAR_ZOOM);
        assert!(demon(&view, Vec2::new(1.0, 0.0)) < demon(&view, Vec2::new(2.0, 0.0)));
        assert!(wanderer(&view, Vec2::new(1.0, 0.0)) < wanderer(&view, Vec2::new(2.0, 0.0)));
    }

    /// На общем плане мирные не берутся вовсе — даже стоящие в самом центре
    /// кадра: пешка там точка, а «в кадре» — полкарты.
    #[test]
    fn no_wanderer_is_dispatched_at_the_wide_zoom() {
        let wide = view(WANDER_DISPATCH_MAX_ZOOM);
        assert_eq!(wanderer(&wide, Vec2::new(CLOSE, CLOSE)), None);
        assert_eq!(
            demon(&wide, Vec2::new(CLOSE, CLOSE)).map(|(priority, _)| priority),
            Some(priority::URGENT)
        );
    }
}
