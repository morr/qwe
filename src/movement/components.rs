use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::tasks::Task;

use crate::navigation::PathfindingResult;

#[derive(Debug, Clone, Eq, PartialEq, Default, Reflect)]
pub enum MovableState {
    #[default]
    Idle,
    /// Ждём ответа асинхронного поиска пути к тайлу.
    Pathfinding(IVec2),
    /// Идём по пути к тайлу.
    Moving(IVec2),
    /// Поиск пути не удался — поведение должно выбрать новую цель.
    PathfindingError(IVec2),
}

/// Тег для дешёвой выборки движущихся сущностей.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct MovableStateMovingTag;

/// «Этой пешке пора выбрать новую цель»: висит ровно на состояниях
/// [`MovableState::Idle`] и [`MovableState::PathfindingError`].
///
/// Тег, а не проверка состояния в теле системы, потому что выбор целей
/// (`human::pick_wander_targets`, `demon::pick_wander_targets`) в
/// детерминированном режиме переезжает в `FixedUpdate`: на 30× это ~30
/// прогонов за кадр, и просмотр всех 17 000 гуляющих ради нескольких тысяч
/// стоящих стоил бы миллионы проверок в кадр. С тегом запрос сразу видит
/// только стоящих.
///
/// Ставится и снимается **только** переходами `Movable` — иначе тег разъедется
/// с состоянием, и пешка либо застынет навсегда, либо начнёт выбирать цель на
/// ходу.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct NeedsWanderTarget;

/// Позиция сущности в симуляции — источник истины для движения. Двигается в
/// `FixedUpdate`; `Transform` — визуальное представление, интерполируемое
/// между фиксированными шагами.
#[derive(Component, Debug, Default, Deref, DerefMut, Reflect)]
#[reflect(Component)]
pub struct SimPosition(pub Vec2);

/// `SimPosition` на прошлом фиксированном шаге — второй конец интерполяции.
#[derive(Component, Debug, Default, Deref, DerefMut, Reflect)]
#[reflect(Component)]
pub struct PreviousSimPosition(pub Vec2);

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
// свежая пешка стоит в `Idle`, то есть цель ей нужна с первого тика
#[require(SimPosition, PreviousSimPosition, NeedsWanderTarget)]
pub struct Movable {
    pub speed: f32,
    /// Waypoint'ы в мировых метрах, а не в тайлах: по полигональному мешу
    /// путь идёт углами препятствий, и центр тайла в нём смысла не имеет.
    /// Сеточный поиск отдаёт свои тайлы через `tile_center` — для движения
    /// оба источника выглядят одинаково.
    ///
    /// Цель при этом осталась тайлом (`MovableState`, `PathfindingRequest`):
    /// по ней отсеивается устаревший ответ и считается прибытие.
    pub path: VecDeque<Vec2>,
    pub state: MovableState,
    /// Направление последнего шага — вектор доката: когда путь дожёван раньше,
    /// чем пришёл ответ перепрокладки, сущность продолжает по нему двигаться
    /// (см. `move_moving_entities`). Из пути его не достать — путь уже пуст.
    pub last_direction: Vec2,
}

/// Асинхронный поиск пути этой сущности. Один таск на сущность: новый запрос
/// вытесняет старый, а дроп `Task` его отменяет.
#[derive(Component, Debug)]
pub struct PathfindingTask {
    pub task: Task<PathfindingResult>,
    /// Момент запуска — сторожок зависших поисков
    /// (`listen_for_pathfinding_tasks`): живой поиск отвечает за миллисекунды
    /// и падает по внутреннему бюджету, если расходится, — таск старше
    /// [`PATHFINDING_TASK_HANG_SECS`](crate::settings::PATHFINDING_TASK_HANG_SECS)
    /// означает зависший каким-то новым способом бэкенд, и это паника,
    /// а не тихое вечное ожидание.
    pub spawned_at: std::time::Instant,
}

impl PathfindingTask {
    pub fn new(task: Task<PathfindingResult>) -> Self {
        Self {
            task,
            spawned_at: std::time::Instant::now(),
        }
    }
}

/// Запрос поиска пути, ждущий своей очереди: таски запускает
/// `dispatch_pathfinding_requests` по приоритету близости к камере.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct PathfindingRequest {
    pub start_tile: IVec2,
    pub end_tile: IVec2,
}

/// «Этой пешке нельзя ждать камеру» — заявка обслуживается вне очереди
/// гуляющих и не гейтится видимостью.
///
/// Значение вместо видового тега. Правило «срочны демоны и убегающие люди»
/// прежде существовало тремя копиями, в трёх модулях и в трёх полярностях:
/// `!is_human || is_fleeing` в живом диспетчере, `is_human && !is_fleeing` в
/// детерминированном и третья, своя, в прогреве (`loading::poll_warmup`).
/// Копии разъезжались молча — у прогрева до сих пор свой запас видимости.
///
/// Владеют маркером **виды**: демон получает его при спавне и не теряет,
/// человек — вместе с `HumanFleeTag` и вместе с ним же его отдаёт, отладочный
/// ходок носит постоянно (он и раньше считался срочным, не будучи человеком).
/// Движение только спрашивает `Has<UrgentPath>` и ни одного вида не называет.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct UrgentPath;

/// Тик, на котором подана заявка — ключ FIFO-очереди детерминированного
/// диспетчера. Живёт и умирает вместе с [`PathfindingRequest`].
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct RequestedAt(pub u64);

/// Тик, на котором ответ обязан быть применён, — `тик диспетчеризации +
/// PATHFINDING_RETIRE_TICKS`. Не тик подачи ([`RequestedAt`]): до диспетчера
/// заявка стоит в очереди, и сколько тиков она там простояла, в срок не
/// входит.
///
/// Это и есть развязка симуляции с реальным временем: поиск считается
/// асинхронно, но результат ждёт своего тика, а если не успел — его дожидаются
/// (`block_on`). Пешка трогается с места на одном и том же тике при любой
/// частоте кадров и любой загрузке машины.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct RetireAt(pub u64);

#[derive(EntityEvent, Debug, Clone)]
pub struct MovableReachedDestinationEvent {
    pub entity: Entity,
    pub grid_tile: IVec2,
}

/// Одна пешка под правкой: тождество, три компонента, которые переезд и смена
/// состояния меняют ВМЕСТЕ, и буфер команд, которым эта смена объявляется.
///
/// Группа, а не пять параметров: `systems::rescue_from_impassable` и
/// `pathfinding::accept_answer` принимали ровно эту пятёрку в одном и том же
/// порядке, а третий вызывающий — `systems::rescue_trapped_entities` — собирал
/// её из своего запроса. Врозь она смысла не имеет: переезд обязан поправить
/// оба конца интерполяции (`SimPosition` и `PreviousSimPosition`) и сбросить
/// путь (`Movable`) одним движением — иначе интерполяция протянет пешку через
/// полгорода за кадр. Тот же приём, что у
/// [`SeparationInput`](super::separation::SeparationInput) /
/// `SeparationOutput`, только над заимствованиями компонентов, а не над
/// ресурсами.
///
/// `Drop` не реализуется намеренно: `rescue_trapped_entities` читает
/// `sim_position` уже ПОСЛЕ вызова спасения, и освободить заимствование там
/// может только NLL.
pub(super) struct PawnEdit<'a, 'w, 's> {
    pub entity: Entity,
    pub movable: &'a mut Movable,
    pub sim_position: &'a mut SimPosition,
    pub previous: &'a mut PreviousSimPosition,
    pub commands: &'a mut Commands<'w, 's>,
}

impl Movable {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            path: VecDeque::new(),
            state: MovableState::Idle,
            last_direction: Vec2::ZERO,
        }
    }

    /// Остановить движение и вернуть пешку в `Idle`.
    ///
    /// `destination_reached` публикует `MovableReachedDestinationEvent`, но
    /// только при уже пройденном пути: непройденный остаток означает, что до
    /// цели не дошли. Вызывающий, который знает о приходе больше, чем сам
    /// путь, — придержанный у цели в `step_along_path`, ответ «ты уже на
    /// месте» в `accept_answer` — обязан вычистить путь ДО вызова, иначе
    /// приход молча не публикуется.
    pub fn to_idle(&mut self, entity: Entity, commands: &mut Commands, destination_reached: bool) {
        if destination_reached
            && self.path.is_empty()
            && let MovableState::Moving(end_tile) | MovableState::Pathfinding(end_tile) = self.state
        {
            commands.trigger(MovableReachedDestinationEvent {
                entity,
                grid_tile: end_tile,
            });
        }

        self.stop_moving(entity, commands);
        self.state = MovableState::Idle;
        commands.entity(entity).insert(NeedsWanderTarget);
    }

    pub fn to_moving(
        &mut self,
        end_tile: IVec2,
        path: VecDeque<Vec2>,
        entity: Entity,
        commands: &mut Commands,
    ) {
        self.state = MovableState::Moving(end_tile);
        self.path = path;
        commands
            .entity(entity)
            .insert(MovableStateMovingTag)
            .remove::<NeedsWanderTarget>();
    }

    /// Запустить асинхронный поиск пути; ответ снимает
    /// `listen_for_pathfinding_tasks`.
    ///
    /// Текущий путь при этом НЕ сбрасывается — перепрокладка идёт на ходу.
    /// Между заявкой и ответом проходит минимум кадр (диспетчер и приёмник
    /// живут в `Update`), а убегающий перекладывается раз в ~1 с; на ускоренном
    /// времени этот кадр стоит заметную долю виртуальной секунды, и остановка
    /// на каждой перепрокладке держала четверть паникующих стоящими в любой
    /// момент времени.
    ///
    /// Ещё не уехавшая заявка **снимается и подаётся заново**, а не
    /// перезаписывается на месте. Оба её потребителя отбирают по
    /// `Added<PathfindingRequest>` — `stamp_pathfinding_requests` и
    /// `assign_destination_slots`, — а `insert` поверх живого компонента
    /// взводит только `Changed`. Перецеленная в очереди пешка поэтому уезжала
    /// с ключом FIFO от ПЕРВОЙ цели (`RequestedAt` тем старше, чем дольше
    /// заявка стояла) и без слота на новую, продолжая держать
    /// `DestinationClaim` на брошенной точке. Ждать этого приходится недолго:
    /// очередь мирных — штатно вся неохваченная популяция, а в живом режиме
    /// мирный вне кадра не диспетчится вовсе, так что паника перецеливает
    /// человека поверх заявки, которая никуда не уходила.
    ///
    /// `RequestedAt` уходит вместе с заявкой (см. её док) и рождается заново
    /// вместе с ней: `stamp_pathfinding_requests` стоит в хвосте того же тика.
    pub fn to_pathfinding(
        &mut self,
        entity: Entity,
        start_tile: IVec2,
        end_tile: IVec2,
        commands: &mut Commands,
    ) {
        self.state = MovableState::Pathfinding(end_tile);

        // в очередь; старый таск отменяется (дроп `Task`), старая заявка
        // снимается вместе со своей меткой тика и подаётся заново — снятие и
        // вставка идут двумя командами по порядку, поэтому `Added` взводится
        commands
            .entity(entity)
            .remove::<(
                PathfindingTask,
                RetireAt,
                NeedsWanderTarget,
                PathfindingRequest,
                RequestedAt,
            )>()
            .insert(PathfindingRequest {
                start_tile,
                end_tile,
            });
    }

    /// Поиск пути не удался. Путь, если он ещё не пройден, остаётся: пока
    /// поведение выбирает новую цель, идти по старому лучше, чем стоять.
    pub fn to_pathfinding_error(
        &mut self,
        entity: Entity,
        end_tile: IVec2,
        commands: &mut Commands,
    ) {
        self.state = MovableState::PathfindingError(end_tile);
        commands.entity(entity).insert(NeedsWanderTarget);
    }

    /// Останавливается ВСЁ, чем пешка едет дальше: путь, тег движения и ещё не
    /// уехавшая заявка вместе с меткой её тика.
    ///
    /// Заявка — потому что после перехода в `Idle` состояние уже не
    /// `Pathfinding(end_tile)`, и ответ на неё `accept_answer`
    /// (`pathfinding.rs`) выбросит первой же проверкой, — но выбросит ПОСЛЕ
    /// того, как диспетчер посчитает по ней полноценный A*. Снять её может
    /// только выдача в таск, так что оставленная заявка обязательно оплачена.
    /// На волне успокоения (`human::behavior::flee`, `FleeAction::CalmDown`,
    /// следом пауза 2–10 с, перезаписать её новой целью некому) это сотни
    /// выброшенных поисков за тик.
    ///
    /// Метка тика уходит строго вместе с заявкой ([`RequestedAt`]): заявка без
    /// метки для детерминированного диспетчера невидима, и пешка застыла бы
    /// навсегда. Новую пару заводит `to_pathfinding` — вставленная заново
    /// заявка снова взводит `Added`, и `stamp_pathfinding_requests` ставит
    /// свежую метку.
    ///
    /// Запущенный поиск (`PathfindingTask`, `RetireAt`) здесь не трогается: он
    /// уже оплачен, а его срок — часть тик-точной бухгалтерии приёмника.
    fn stop_moving(&mut self, entity: Entity, commands: &mut Commands) {
        self.path = [].into();
        commands
            .entity(entity)
            .remove::<(MovableStateMovingTag, PathfindingRequest, RequestedAt)>();
    }
}

/// Снять с сущности всё движение: она остаётся в мире, но симуляция её больше
/// не ведёт. Так человек становится трупом.
///
/// Список принадлежит **этому** модулю, и в этом весь смысл функции: раньше он
/// был выписан в `demon/behavior.rs`, где обсервер убийства перечислял
/// внутренности движения — заявку, её метку тика, срок снятия таска, заявку на
/// слот назначения. Новая покомпонентная мелочь движения означала правку в
/// чужом модуле, а отказ был молчалив: «труп, держащий `RetireAt`» уже
/// случался.
///
/// `remove_with_requires` — чтобы `#[require]` у [`Movable`] оставался
/// единственным местом, где записано, что таскает за собой движимая сущность:
/// добавленное туда снимется отсюда само.
pub fn strip_movement(commands: &mut Commands, entity: Entity) {
    commands
        .entity(entity)
        .remove_with_requires::<Movable>()
        .remove::<(
            MovableStateMovingTag,
            PathfindingTask,
            PathfindingRequest,
            // метки тиков живут вместе со своей заявкой/таском — без них они
            // означали бы срок, который никогда не наступит
            RequestedAt,
            RetireAt,
            // срочность — свойство заявки, а заявки у трупа больше нет
            UrgentPath,
            // держать за неподвижным слот назначения значило бы навсегда
            // вычесть место из живой толпы
            crate::movement::DestinationClaim,
        )>();
}
