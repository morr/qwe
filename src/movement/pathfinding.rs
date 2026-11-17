//! Конвейер поиска пути: заявка → очередь диспетчера с приоритетом и бюджетом
//! → асинхронный таск → снятый ответ, подрезанный под уже пройденное.
//! Ходьбу по готовому пути ведёт [`super::systems`].

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;

use self::accept::{accept_answer, AnswerTally};
use self::budget::{push_within_budget, request_units, take_within_budget, QueuedRequest};
use super::VIEW_MARGIN;
use crate::camera::Viewport;
use crate::determinism::SimTick;
use crate::movement::components::{
    Movable, PathfindingRequest, PathfindingTask, PawnEdit, PreviousSimPosition, RequestedAt,
    RetireAt, SimPosition, UrgentPath,
};
use crate::navigation::{Backend, PathfindingResult};
use crate::settings::MAX_PATHFINDING_IN_FLIGHT;
use crate::spatial::SpatialGrid;

mod accept;
mod budget;

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
/// Зум-половина правила [`waits_for_camera`]: ждать заявок, которых диспетчер
/// не берёт, нельзя никому — прогрев (`loading.rs::poll_warmup`) держал бы
/// экран загрузки до таймаута с неподвижным счётчиком — заявки, которых
/// диспетчер не берёт, не закроются.
fn wanderers_dispatched_at_zoom(camera_scale: f32) -> bool {
    camera_scale < crate::settings::WANDER_DISPATCH_MAX_ZOOM
}

/// Ждёт ли пешка, пока камера её увидит — единственная копия правила видимой
/// половины отбора заявок. Конъюнкт «зум + в кадре» раньше был написан дважды:
/// у [`queue_key`] и свой у прогрева (`loading.rs::poll_warmup`), и запас у
/// копий разъезжался молча (та же история, что с срочностью до
/// [`UrgentPath`]).
///
/// Срочной (`urgent`) ждать не положено никогда — её диспетчер берёт при
/// любом зуме и вне кадра. Мирная гуляющая ждёт, если на этом зуме мирных
/// не берут вовсе ([`wanderers_dispatched_at_zoom`]) или она вне кадра
/// с запасом `margin`.
///
/// `margin` — запас к полуразмеру кадра, в тех же экранах, что у
/// [`Viewport::with_margin`]: `1.0` — ровно кадр, запаса нет (`0.0` сжал бы
/// кадр в точку). У каждого спрашивающего свой: диспетчер меряет
/// [`VIEW_MARGIN`] (пешке у кромки нельзя замирать при лёгком движении
/// камеры), прогрев — `1.0`: его вопрос «видит ли игрок стоящую пешку», а не
/// «возьмёт ли её диспетчер».
pub fn waits_for_camera(view: &Viewport, position: Vec2, urgent: bool, margin: f32) -> bool {
    if urgent {
        return false;
    }
    let view = view.with_margin(margin);
    !wanderers_dispatched_at_zoom(view.zoom) || !view.contains(position)
}

/// Место заявки в очереди: приоритет, затем удалённость от центра кадра.
/// `None` — заявку не брать вовсе: мирный гуляющий вне кадра ждёт камеру.
///
/// Отдельно от системы, потому что это всё правило видимости целиком, а
/// система вокруг него — только сборка очереди и её усечение под бюджет.
/// Кадр берёт без запаса — диспетчерский добавляет внутри, через
/// [`waits_for_camera`].
fn queue_key(view: &Viewport, position: Vec2, urgent: bool) -> Option<(u8, f32)> {
    if waits_for_camera(view, position, urgent, VIEW_MARGIN) {
        return None;
    }
    let priority = if urgent {
        priority::URGENT
    } else {
        priority::WANDER_ON_SCREEN
    };
    Some((priority, view.distance_from_centre_squared(position)))
}

/// Работает ли живой диспетчер в этом кадре: мир не на паузе — или пауза
/// прогрева.
///
/// На паузе игрока пешки не идут, а очередь от кадра к кадру не меняется, и
/// проход по всем ждущим заявкам (~17k мирных вне кадра) был чистой ценой
/// кадра, ~0.3 мс. Заявки копятся и уходят в первый же кадр после паузы.
///
/// **Пауза прогрева — исключение, и обязательное**: `loading::pause_world`
/// держит `Time<Virtual>` на паузе весь `PlayPhase::Warmup`, а прогрев ждёт
/// ответов на заявки пешек в кадре — без диспетчера экран загрузки висел бы до
/// таймаута. Нет состояния вовсе (стенды `MovementPlugin` без состояний) —
/// работает.
///
/// Реплей не задет: это живая ветка (`SimPipeline::Live`); детерминированный
/// диспетчер живёт в `FixedUpdate`, который на паузе не идёт и так.
pub fn live_dispatch_runs(
    time: Res<Time<Virtual>>,
    phase: Option<Res<State<crate::loading::PlayPhase>>>,
) -> bool {
    !time.is_paused() || phase.is_none_or(|phase| *phase.get() != crate::loading::PlayPhase::Live)
}

/// Маркер ресурса-сетки стоящих мирных заявок, `SpatialGrid<QueuedWander>`.
/// Не компонент: сетка типизирована им только для имени, записи в ней ведут
/// observers жизненного цикла заявки и срочности.
///
/// Зачем сетка: полный скан стоящих заявок стоил кадру 0.8–1.2 мс на штормовые
/// 20 тысяч (`dispatch_bench`), а гейт видимости — чистая геометрия кадра, и
/// ~97% скана выбрасывалось. Диспетчер собирает кандидатов из ячеек, накрывающих
/// кадр: на близком зуме это сотни вместо двадцати тысяч. В сетке живут только
/// мирные заявки — срочных немного, их берёт архетипный запрос `With<UrgentPath>`
/// при любом зуме.
///
/// Ведение — observers на компоненты, а не писцы: у заявки много источников
/// снятия (приёмка ответа, `to_idle`, раздевание трупа, despawn при побеге,
/// рестарте и смене города), и все они поднимают `Remove` — единственный путь,
/// которым запись покидает сущность. Выдача в таск заявку не снимает (см.
/// выдачу в [`dispatch_pathfinding_requests`]), так что запись доживает в
/// сетке до приёмки — диспетчер отсекает выданных проверкой `PathfindingTask`.
/// Тот же принцип полноты событий, что у
/// [`DestinationClaims`](crate::movement::destination::DestinationClaims).
///
/// Мирная заявка стоит до выдачи (подаётся в простое), а между выдачей и
/// приёмкой может уехать (перепрокладка на ходу) — ячейка записи за это окно
/// способна протухнуть; это нечестно только для «КТО читать», а не «ГДЕ»:
/// диспетчер и так читает позицию живьём, сетка решает только, КОГО читать.
pub struct QueuedWander;

/// Мирная заявка встала — в сетку. Срочная не попадает: её диспетчер берёт и
/// без кадра. Порядок команд внутри одной цепочки не имеет значения:
/// `UrgentPath`, вставленный после заявки, исправит запись observer
/// [`on_urgent_path_added`], вставленный до — заявка просто не встанет.
pub fn on_pathfinding_request_added(
    event: On<Add<PathfindingRequest>>,
    mut queued: ResMut<SpatialGrid<QueuedWander>>,
    pawns: Query<(&SimPosition, Has<UrgentPath>)>,
) {
    let Ok((position, urgent)) = pawns.get(event.entity) else {
        return;
    };
    if !urgent {
        queued.insert(event.entity, position.0);
    }
}

/// Заявка снята — прочь из сетки. Никаких чтений компонент, только сущность
/// и ресурс: despawn поднимает `Remove` на каждый компонент, и observer
/// обязан отработать и там (побег, рестарт, смена города, раздевание трупа).
pub fn on_pathfinding_request_removed(
    event: On<Remove<PathfindingRequest>>,
    mut queued: ResMut<SpatialGrid<QueuedWander>>,
) {
    queued.remove(event.entity);
}

/// Мирный стал срочным (паника, убегание) — из сетки прочь: его теперь берёт
/// архетипный запрос срочных. Сущность может в этот момент уже сниматься
/// (despawn убирает компоненты по одному): нечитаемая пропускается — её
/// заявка поднимет свой `Remove` и вычистит запись сама.
pub fn on_urgent_path_added(
    event: On<Add<UrgentPath>>,
    mut queued: ResMut<SpatialGrid<QueuedWander>>,
    queued_requests: Query<(), With<PathfindingRequest>>,
) {
    if queued_requests.contains(event.entity) {
        queued.remove(event.entity);
    }
}

/// Обратный ход: срочность снята, а заявка ещё стоит — запись возвращается.
/// В штатном ходе срочность снимается вместе с заявкой (`to_idle`,
/// `strip_movement`), и порядок двух `Remove` не гарантирован: если заявка
/// ушла первой, запрос здесь ничего не увидит, если ещё стоит — запись
/// вернётся и будет вычищена её собственным `Remove`.
pub fn on_urgent_path_removed(
    event: On<Remove<UrgentPath>>,
    mut queued: ResMut<SpatialGrid<QueuedWander>>,
    pawns: Query<&SimPosition, With<PathfindingRequest>>,
) {
    if let Ok(position) = pawns.get(event.entity) {
        queued.insert(event.entity, position.0);
    }
}

/// Вход в мир начинается с пустой сеткой очереди. Записи вычищаются своими
/// `Remove`-observer'ами и так; это страховка от любой дыры в том покрытии —
/// по образцу [`crate::movement::destination::reset_destination_claims`].
pub fn reset_queued_wanderers(mut queued: ResMut<SpatialGrid<QueuedWander>>) {
    queued.rebuild(std::iter::empty());
}

/// Запуск тасков поиска пути из очереди запросов. МИРНО гуляющие люди вне
/// экрана путь НЕ получают вовсе — их заявки ждут, пока камера не приедет;
/// демоны и убегающие люди обсчитываются всегда (иначе инвазия и паника за
/// кадром встанут) и первыми. Внутри приоритета — по удалённости от центра
/// кадра.
///
/// Мирные собираются из сетки очереди ([`QueuedWander`]) — ячейки под кадром,
/// а не все стоящие заявки; срочные — архетипным запросом. Правило отбора у
/// обеих половин одно, [`queue_key`]: сбор по ячейкам грубый (ячейка крупнее
/// точности гейта), точное попадание меряет он; позиция читается живьём из
/// запроса, записи сетки говорят лишь, КОГО читать.
///
/// Выдача ставит таск **поверх живой заявки**, не снимая её: пара
/// `remove`+`insert` — два переноса между архетипами на каждый поиск, и на
/// шторме входа (сотни-тысяча диспатчей за кадр) это миллисекунды на
/// применении команд. Заявку снимает приёмка (`listen_for_pathfinding_tasks`)
/// одним пакетом с таском; до приёмки выданные отфильтровываются здесь
/// проверкой таска. Детерминированный диспетчер ведёт свою пару
/// заявка/таск как раньше — там выдача идёт тик-точным хвостом и цену его
/// держит не команда, а ожидание пула.
pub fn dispatch_pathfinding_requests(
    mut commands: Commands,
    backend: Res<Backend>,
    frame: Res<Viewport>,
    queued: Res<SpatialGrid<QueuedWander>>,
    urgent_requests: Query<(Entity, &SimPosition, &PathfindingRequest), With<UrgentPath>>,
    wanderers: Query<(Entity, &SimPosition, &PathfindingRequest), Without<UrgentPath>>,
    tasks: Query<(), With<PathfindingTask>>,
) {
    let budget = MAX_PATHFINDING_IN_FLIGHT.saturating_sub(tasks.iter().count());
    if budget == 0 {
        return;
    }

    // запас диспетчера: queue_key меряет им точное попадание (внутри
    // waits_for_camera, кадр ей нужен без запаса), а здесь он же — грубый
    // прямоугольник сбора ячеек, которому достаточно накрыть кадр с запасом
    let view = frame.with_margin(VIEW_MARGIN);

    // выданная заявка остаётся на сущности до приёмки (см. выдачу ниже), так
    // что «уже в полёте» доходит сюда — и отсекается здесь, до сборки очереди
    let in_flight = |entity: Entity| tasks.contains(entity);

    let mut queue: Vec<(u8, f32, Entity, Vec2, IVec2, IVec2)> = urgent_requests
        .iter()
        .filter_map(|(entity, sim_position, request)| {
            if in_flight(entity) {
                return None;
            }
            let (priority, distance) = queue_key(&frame, sim_position.0, true)?;
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

    if wanderers_dispatched_at_zoom(view.zoom) {
        queued.for_each_in_rect(view.min(), view.max(), |entity| {
            let Ok((_, sim_position, request)) = wanderers.get(entity) else {
                // снятая сущность: запись вычистится её Remove-observer'ом,
                // попасть сюда она могла только в пределах одного кадра
                return;
            };
            // запись в сетке доживает до приёмки, а не до выдачи: выданная
            // (таск уже стоит) пропускается, чистить её отсюда не нужно
            if in_flight(entity) {
                return;
            }
            if let Some((priority, distance)) = queue_key(&frame, sim_position.0, false) {
                queue.push((
                    priority,
                    distance,
                    entity,
                    sim_position.0,
                    request.start_tile,
                    request.end_tile,
                ));
            }
        });
    }

    if queue.len() > budget {
        // top-k вместо полного сорта: всё, что дальше `budget`, всё равно
        // обрезается, а порядок внутри отрезанной части ни на что не влияет —
        // ничья на (приоритет, дистанция) и раньше не была определена (сорт
        // был unstable). O(n) выборка против O(n log n): ~160 мкс на ~6000
        // кандидатов при переполнении (запись 9) против ~20 мкс выборки
        queue.select_nth_unstable_by(budget, |a, b| {
            (a.0, a.1)
                .partial_cmp(&(b.0, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        queue.truncate(budget);
    }

    for (_, _, entity, start_world, start_tile, end_tile) in queue {
        let task = spawn_path_task((*backend).clone(), start_world, start_tile, end_tile);
        // таск ставится поверх живой заявки, без её снятия: снятие — перенос
        // между архетипами, и на шторме входа их тысячи на кадр (по паре на
        // каждый поиск: выдача и приёмка). Заявку теперь снимает приёмка —
        // одним пакетом с таском, одна команда = один перенос
        commands.entity(entity).insert(PathfindingTask::new(task));
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
        // таск и заявка уходят одним пакетом — одна команда, один перенос
        // архетипа: заявка пережила выдачу (см. диспетчер), свой срок она
        // отслужила
        commands
            .entity(entity)
            .remove::<(PathfindingTask, PathfindingRequest)>();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WANDER_DISPATCH_MAX_ZOOM;

    /// Кадр 1000×600 логических пикселей вокруг начала координат: на зуме 0.1
    /// это ±50×30 метров, на зуме общего плана — на порядок больше. Ровно
    /// кадр, как лежит в ресурсе: запас queue_key добавляет сам.
    fn view(zoom: f32) -> Viewport {
        let mut window = Window::default();
        window.resolution.set(1000.0, 600.0);
        Viewport::of(&window, &Transform::from_scale(Vec3::splat(zoom)), 1.0)
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

    /// Граница видимости у каждого спрашивающего своя: кромка кадра своя и у
    /// прогрева (`1.0`, запаса нет), полоса запаса диспетчера
    /// ([`VIEW_MARGIN`]) диспетчером обслуживается, но прогреву её уже не
    /// видно, а за полосой не видно никому. Срочная не ждёт ни в одной зоне.
    #[test]
    fn the_visibility_edge_sits_where_the_asker_margin_puts_it() {
        let frame = view(NEAR_ZOOM); // ровно кадр: ±50 м по горизонтали
        let edge = Vec2::new(50.0, 0.0); // кромка считается своей
        assert!(!waits_for_camera(&frame, edge, false, 1.0));
        assert!(!waits_for_camera(&frame, edge, false, VIEW_MARGIN));

        let in_dispatch_band = Vec2::new(55.0, 0.0); // за кадром, в полосе запаса
        assert!(!waits_for_camera(
            &frame,
            in_dispatch_band,
            false,
            VIEW_MARGIN
        ));
        assert!(waits_for_camera(&frame, in_dispatch_band, false, 1.0));

        let beyond = Vec2::new(61.0, 0.0); // и за полосой запаса
        assert!(waits_for_camera(&frame, beyond, false, VIEW_MARGIN));

        // срочной ждать не положено нигде — и вне кадра тоже
        assert!(!waits_for_camera(&frame, beyond, true, VIEW_MARGIN));

        // на общем плане мирная ждёт и в центре кадра: мирных там не берут
        // вовсе, а срочную берут
        let wide = view(WANDER_DISPATCH_MAX_ZOOM);
        assert!(waits_for_camera(
            &wide,
            Vec2::new(CLOSE, CLOSE),
            false,
            VIEW_MARGIN
        ));
        assert!(!waits_for_camera(
            &wide,
            Vec2::new(CLOSE, CLOSE),
            true,
            VIEW_MARGIN
        ));
    }
}
