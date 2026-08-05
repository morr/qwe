use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::tasks::futures::check_ready;
use bevy::window::PrimaryWindow;

use crate::grid::{tile_center, world_to_tile};
use crate::movement::components::{
    Movable, MovableState, MovableStateMovingTag, PathfindingRequest, PathfindingTask,
    PreviousSimPosition, SimPosition,
};
use crate::navigation::{
    Pathfinder, PathfindingAlgorithm, PathfindingResult, find_path, find_path_northstar,
    find_path_polymesh, nearest_tile_where,
};
use crate::settings::{RESCUE_SEARCH_TILES, unit_z};

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
/// фронт (см. `polymesh::MAX_SEARCH_POLLS`), а незавершённый сеточный —
/// ничего заметного. Но и не 32, как было в первой попытке удержать память
/// лимитом: сам по себе лимит от разрастания не спасает (взрывается один
/// запрос, а не их число), зато низкий намертво останавливает мир — пара
/// тяжёлых поисков занимает почти все слоты, и диспетчер перестаёт выдавать
/// заявки всем остальным. Память ограничена бюджетом шагов, слоты — здесь.
const MAX_POLYMESH_PATHFINDING_IN_FLIGHT: usize = MAX_PATHFINDING_IN_FLIGHT;
/// Запас видимости к полуразмеру экрана — чтобы пешки у кромки кадра не
/// «замирали» при лёгком движении камеры.
const VIEW_MARGIN: f32 = 1.2;

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
        let navmesh = pathfinder.navmesh.0.clone();
        let northstar = pathfinder.northstar.get();
        let polymesh = polymesh.clone();
        let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
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
                    let path = tiles
                        .map(|tiles| tiles.into_iter().map(tile_center).collect::<Vec<Vec2>>());
                    (path, started_at)
                }
            };
            PathfindingResult {
                start_tile,
                end_tile,
                path,
                duration: started_at.elapsed(),
            }
        });
        commands
            .entity(entity)
            .remove::<PathfindingRequest>()
            .insert(PathfindingTask(task));
    }
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

/// Снимок позиции на начало фиксированного шага — второй конец интерполяции.
///
/// `Changed<SimPosition>` вместо «всех подряд»: у сущности, не сдвинувшейся
/// с прошлого снимка, `PreviousSimPosition` уже равен `SimPosition`, и копия
/// ничего бы не изменила — а стоящих ~90% из 20 000. Остановившаяся сущность
/// не протухает: последний сдвиг и есть последнее изменение, его снимок
/// выравнивает оба конца интерполяции. Спавн тоже покрыт — `Added` входит
/// в `Changed`.
pub fn snapshot_previous_sim_positions(
    mut query: Query<(&mut PreviousSimPosition, &SimPosition), Changed<SimPosition>>,
) {
    for (mut previous, current) in &mut query {
        previous.0 = current.0;
    }
}

/// Движение в `FixedUpdate`, двигает `SimPosition` по waypoint'ам пути.
///
/// `MovableStateMovingTag` означает «есть путь или докат», а не «состояние
/// `Moving`»: при перепрокладке на ходу состояние уже `Pathfinding`, а идти
/// по старому пути — и докатывать за его концом — надо до прихода нового.
///
/// **Докат**: путь дожёван, а состояние ещё `Pathfinding` — ответ поиска
/// опаздывает (конвейер заявка → диспетчер → таск → приёмка стоит 2–3 кадра,
/// на 30x это 1–1.5 виртуальных секунды). Вместо остановки сущность
/// продолжает двигаться по `last_direction`, пока тайл впереди проходим:
/// бегущий и так бежит «прочь», демону докат к жертве только на пользу, а
/// видимое «бежит — замер — бежит» исчезает. Пришедший ответ (`to_moving`)
/// или `PathfindingError` завершают докат штатно.
///
/// Заодно ведёт сетку людей: пересёк границу 60-метровой ячейки — переезд.
/// Сравнение ячеек — арифметика без hash, само событие редкое (гуляющий
/// пересекает ячейку раз в ~21 виртуальную секунду), так что стоимость не
/// растёт ни от зум-аута, ни от населения. `Option` — плагин движения
/// используется в тестах без `SpatialPlugin`.
pub fn move_moving_entities(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    navmesh: Res<crate::navigation::ArcNavmesh>,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut query: Query<
        (
            Entity,
            &mut Movable,
            &mut SimPosition,
            Has<crate::human::Human>,
        ),
        With<MovableStateMovingTag>,
    >,
    time: Res<Time>,
) {
    let started = std::time::Instant::now();
    let navmesh = navmesh.read();
    for (entity, mut movable, mut sim_position, is_human) in &mut query {
        let cell_before = crate::spatial::cell_of(sim_position.0);
        let mut remaining_time = time.delta_secs();
        loop {
            if movable.path.is_empty() {
                match movable.state {
                    MovableState::Moving(target) => {
                        let destination_reached = world_to_tile(sim_position.0) == target;
                        movable.to_idle(entity, &mut commands, destination_reached);
                    }
                    // старый путь пройден раньше, чем посчитан новый — докат
                    MovableState::Pathfinding(_) => {
                        let step = movable.last_direction * movable.speed * remaining_time;
                        let coasted = sim_position.0 + step;
                        let tile = world_to_tile(coasted);
                        // стоять на месте (нулевой вектор) или упереться в
                        // непроходимое (за картой — непроходимо само по себе) —
                        // конец доката
                        if step == Vec2::ZERO || !navmesh.is_passable(tile.x, tile.y) {
                            commands.entity(entity).remove::<MovableStateMovingTag>();
                        } else {
                            sim_position.0 = coasted;
                        }
                    }
                    // ошибка поиска или явная остановка — поведение выберет
                    // новую цель, докатывать некуда
                    _ => {
                        commands.entity(entity).remove::<MovableStateMovingTag>();
                    }
                }
                break;
            }

            let target = *movable.path.front().expect("path is non-empty");
            let to_target = target - sim_position.0;
            let distance = to_target.length();
            let distance_to_move = movable.speed * remaining_time;

            if distance_to_move < distance {
                let direction = to_target.normalize_or_zero();
                movable.last_direction = direction;
                sim_position.0 += direction * distance_to_move;
                break;
            }

            // дошли до waypoint'а — встаём на него и тратим остаток времени
            if distance > 0.0 {
                movable.last_direction = to_target / distance;
            }
            sim_position.0 = target;
            movable.path.pop_front();
            remaining_time -= distance / movable.speed;
            if remaining_time <= 0.0 {
                break;
            }
        }

        if is_human
            && crate::spatial::cell_of(sim_position.0) != cell_before
            && let Some(grid) = human_grid.as_mut()
        {
            grid.insert(entity, sim_position.0);
        }
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_MOVE_MS, started);
}

/// Чем меряется «свободно» при спасении — тем же бэкендом, по которому пешки
/// ходят. Полигональный меш строже сетки: его контуры раздуты на радиус агента,
/// и свободный тайл вплотную к стене на меше уже внутри препятствия. Пока меш
/// строится или выключен, остаётся одна сетка.
struct Walkable<'a> {
    navmesh: &'a crate::navigation::Navmesh,
    polymesh: Option<&'a crate::navigation::PolymeshBuild>,
}

impl Walkable<'_> {
    fn allows(&self, point: Vec2) -> bool {
        let tile = world_to_tile(point);
        // сетка первой: индекс в `Vec` против запроса в BVH
        self.navmesh.is_passable(tile.x, tile.y)
            && self.polymesh.is_none_or(|mesh| mesh.contains(point))
    }

    /// Куда переставить застрявшего. Сперва — снап меша (метровый допуск): в
    /// инфляцию контура пешка попадает сантиметрами, и перебирать за неё тайлы
    /// с запросом в BVH на каждый незачем. Не помог — значит она не у стены, а
    /// внутри дома, и тогда работает кольцевой поиск по тайлам, тот же, что и
    /// на голой сетке.
    fn nearest_free_point(&self, point: Vec2) -> Option<Vec2> {
        self.polymesh
            .and_then(|mesh| mesh.nearest_free_point(point))
            .filter(|&snapped| self.allows(snapped))
            .or_else(|| {
                nearest_tile_where(world_to_tile(point), RESCUE_SEARCH_TILES, |candidate| {
                    self.allows(tile_center(candidate))
                })
                .map(tile_center)
            })
    }
}

/// Переезд одной сущности на ближайший свободный тайл, если она стоит в
/// непроходимом. `true` — переехала.
///
/// Попасть внутрь препятствия можно не одним способом: спавн просеивает тайлы
/// по сетке, но пешку ставит в центр тайла, чей край может быть уже внутри дома
/// (заливка метит тайл по центру); постройка меша с новым радиусом агента
/// раздувает контуры под уже стоящими пешками; докат за концом пути и бросок
/// демона двигают `SimPosition` напрямую. Лечить каждый вход отдельно
/// бессмысленно — итог один: поиск из непроходимого старта не находит пути,
/// поведение выбирает новую цель, поиск снова не находит, и так навсегда.
///
/// Переезд ставит и `PreviousSimPosition`: иначе интерполяция протянула бы
/// пешку через полгорода за один кадр. Старый путь сбрасывается — он ведёт из
/// места, где сущности больше нет.
fn rescue_from_impassable(
    walkable: &Walkable,
    entity: Entity,
    movable: &mut Movable,
    sim_position: &mut SimPosition,
    previous: &mut PreviousSimPosition,
    commands: &mut Commands,
) -> bool {
    if walkable.allows(sim_position.0) {
        return false;
    }
    // не нашлось — оставляем как есть: телепорт за пределы кольца увёл бы
    // пешку дальше, чем она вообще могла бы дойти
    let Some(free) = walkable.nearest_free_point(sim_position.0) else {
        return false;
    };
    sim_position.0 = free;
    previous.0 = sim_position.0;
    movable.to_idle(entity, commands, false);
    true
}

/// Полный скан населения — там, где меняется сама геометрия проходимости: на
/// входе в мир (сетка залита и прорежена, население только что расставлено) и
/// на каждой готовой постройке полигонального меша, потому что новый радиус
/// агента раздувает контуры под уже стоящими пешками. В остальное время
/// застрявших ловит провал поиска (`listen_for_pathfinding_tasks`), и проход по
/// 20 000 сущностей не повторяется ни разу.
pub fn rescue_trapped_entities(
    mut commands: Commands,
    pathfinder: Pathfinder,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut query: Query<(
        Entity,
        &mut SimPosition,
        &mut PreviousSimPosition,
        &mut Movable,
        Has<crate::human::Human>,
    )>,
) {
    let navmesh = pathfinder.navmesh.read();
    let polymesh = pathfinder.polymesh_build();
    let walkable = Walkable {
        navmesh: &navmesh,
        polymesh: polymesh.as_deref(),
    };
    let started = std::time::Instant::now();
    let mut rescued = 0;
    for (entity, mut sim_position, mut previous, mut movable, is_human) in &mut query {
        if !rescue_from_impassable(
            &walkable,
            entity,
            &mut movable,
            &mut sim_position,
            &mut previous,
            &mut commands,
        ) {
            continue;
        }
        rescued += 1;
        if is_human && let Some(grid) = human_grid.as_mut() {
            grid.insert(entity, sim_position.0);
        }
    }
    if rescued > 0 {
        info!(
            "rescued {rescued} entities standing in impassable places in {:?}",
            started.elapsed()
        );
    }
}

/// Отдал ли таск постройки меша **новый** результат с прошлого кадра.
/// `resource_changed` здесь не годится: `sync_polymesh_build` берёт
/// `ResMut<PolyNavmesh>` и на старте постройки, и на отмене, а геометрия
/// меняется ровно тогда, когда растёт `generation`.
pub fn polymesh_rebuilt(poly: Res<crate::navigation::PolyNavmesh>, mut seen: Local<u32>) -> bool {
    let current = poly.generation();
    let rebuilt = current != *seen;
    *seen = current;
    rebuilt
}

/// Визуальная позиция: лерп между прошлым и текущим фиксированным шагом,
/// плюс y-сортировка (z от y). Живёт в `AfterFixedMainLoop`.
pub fn interpolate_movable_transforms(
    fixed_time: Res<Time<Fixed>>,
    mut query: Query<(&mut Transform, &SimPosition, &PreviousSimPosition)>,
) {
    let alpha = fixed_time.overstep_fraction();

    for (mut transform, sim_position, previous) in &mut query {
        let rendered = previous.0.lerp(sim_position.0, alpha);
        if transform.translation.truncate() == rendered {
            continue;
        }
        transform.translation = rendered.extend(unit_z(rendered.y));
    }
}

/// `SimPosition` инициализируется из `Transform`: позицию при спавне задают
/// трансформом.
pub fn on_movable_added_init_sim_position(
    event: On<Add, Movable>,
    mut query: Query<(&Transform, &mut SimPosition, &mut PreviousSimPosition)>,
) {
    let Ok((transform, mut sim_position, mut previous)) = query.get_mut(event.entity) else {
        return;
    };

    sim_position.0 = transform.translation.truncate();
    previous.0 = sim_position.0;
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

        let MovableState::Pathfinding(end_tile) = movable.state else {
            continue;
        };
        // устаревший ответ — уже запрошена другая цель
        if end_tile != result.end_tile {
            continue;
        }

        let Some(path) = result.path else {
            let navmesh = navmesh.get_or_insert_with(|| pathfinder.navmesh.read());
            let walkable = Walkable {
                navmesh,
                polymesh: polymesh.as_deref(),
            };
            if rescue_from_impassable(
                &walkable,
                entity,
                &mut movable,
                &mut sim_position,
                &mut previous,
                &mut commands,
            ) {
                if is_human && let Some(grid) = human_grid.as_mut() {
                    grid.insert(entity, sim_position.0);
                }
                continue;
            }
            // не застрял — цель просто недостижима; новую выберет поведение
            movable.to_pathfinding_error(end_tile);
            continue;
        };

        // путь всегда включает стартовую точку; один элемент — мы уже на месте
        if path.len() == 1 {
            movable.to_idle(entity, &mut commands, true);
            continue;
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
        movable.to_moving(end_tile, path, entity, &mut commands);
    }

    // оба счётчика пишутся каждый кадр, в том числе нулями: доля считается в
    // UI как отношение средних, и это верно только пока обе истории одной
    // длины (см. `diagnostics::PATHFINDING_ANSWERED`)
    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_ANSWERED, || {
        answered as f64
    });
    diagnostics.add_measurement(&crate::diagnostics::PATHFINDING_FAILED, || failed as f64);
}

/// Цвет пути — фиолетовый полупрозрачный: жёлтый на этой карте не читался.
pub const MOVEPATH_COLOR: Color = Color::srgba(0.9, 0.2, 0.9, 0.7);
/// Длина «крыльев» стрелки на конце пути; на коротком последнем сегменте
/// ужимается до половины его длины, иначе наконечник перекрывает сам путь.
pub const MOVEPATH_ARROW_TIP: f32 = 4.0;
/// Пути рисуются на текущем экране и на соседних: 3 × 3 экрана вокруг камеры,
/// то есть полуразмер кадра ×3 по каждой оси.
const MOVEPATH_VIEW_SCREENS: f32 = 3.0;

/// Отрисовка путей движущихся сущностей; в финальной сцене выключена,
/// переключается клавишей M (и вместе с doors — клавишей G).
#[derive(Resource, Reflect, SettingsGroup, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "debug", key = "movepath")]
pub struct DrawMovePaths(pub bool);

pub fn toggle_draw_move_paths(mut draw: ResMut<DrawMovePaths>) {
    draw.0 = !draw.0;
    info!("draw move paths: {}", draw.0);
}

pub fn draw_move_paths(
    draw: Res<DrawMovePaths>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut gizmos: Gizmos,
    query: Query<(&SimPosition, &Movable), With<MovableStateMovingTag>>,
) {
    if !draw.0 {
        return;
    }

    let camera_position = camera.translation.truncate();
    let half_view =
        Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * MOVEPATH_VIEW_SCREENS;

    for (sim_position, movable) in &query {
        // на всю карту это десятки тысяч линий за кадр; за соседним экраном
        // путь всё равно не увидеть
        let offset = (sim_position.0 - camera_position).abs();
        if offset.x > half_view.x || offset.y > half_view.y {
            continue;
        }
        // как в zxc: промежуточные сегменты — линии, последний — стрелка
        // в сторону цели
        let last = movable.path.len().saturating_sub(1);
        let mut prev = sim_position.0;
        for (index, &next) in movable.path.iter().enumerate() {
            if index < last {
                gizmos.line_2d(prev, next, MOVEPATH_COLOR);
            } else {
                gizmos
                    .arrow_2d(prev, next, MOVEPATH_COLOR)
                    .with_tip_length(MOVEPATH_ARROW_TIP.min(prev.distance(next) * 0.5));
            }
            prev = next;
        }
    }
}
