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

/// Тик, на котором подана заявка — ключ FIFO-очереди детерминированного
/// диспетчера. Живёт и умирает вместе с [`PathfindingRequest`].
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct RequestedAt(pub u64);

/// Тик, на котором ответ обязан быть применён, — `тик заявки +
/// PATHFINDING_RETIRE_TICKS`.
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

impl Movable {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            path: VecDeque::new(),
            state: MovableState::Idle,
            last_direction: Vec2::ZERO,
        }
    }

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
    pub fn to_pathfinding(
        &mut self,
        entity: Entity,
        start_tile: IVec2,
        end_tile: IVec2,
        commands: &mut Commands,
    ) {
        self.state = MovableState::Pathfinding(end_tile);

        // в очередь; старый таск отменяется (дроп `Task`), старый запрос
        // вытесняется вставкой
        commands
            .entity(entity)
            .remove::<(PathfindingTask, RetireAt, NeedsWanderTarget)>()
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

    fn stop_moving(&mut self, entity: Entity, commands: &mut Commands) {
        self.path = [].into();
        commands.entity(entity).remove::<MovableStateMovingTag>();
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
            // держать за неподвижным слот назначения значило бы навсегда
            // вычесть место из живой толпы
            crate::movement::DestinationClaim,
        )>();
}
