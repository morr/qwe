use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::window::PrimaryWindow;

use crate::camera::Viewport;
use crate::movement::components::{
    Movable, MovableStateMovingTag, PawnEdit, PreviousSimPosition, SimPosition,
};
use crate::navigation::{Backend, Walkable};
use crate::settings::unit_z;

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
/// Заодно ведёт сетку людей: где именно проходит граница «сдвинулся» и
/// «переехал», решает `SpatialGrid::moved`, а не эта система. `Option` —
/// плагин движения используется в тестах без `SpatialPlugin`.
#[allow(clippy::too_many_arguments)]
pub fn move_moving_entities(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    backend: Res<Backend>,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    separation: Res<super::separation::SeparationStyle>,
    pushes: super::separation::SeparationInput,
    lab: Res<super::separation::SeparationLab>,
    human_style: Res<crate::human::HumanStyle>,
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
    let walkable = backend.walkable();
    // «дошёл» с допуском в дистанцию покоя: точнее неё пешки друг к другу не
    // подпускает само расталкивание, так что требовать точный тайл — значит
    // не засчитывать приход столкнутому с тайла в последний момент
    let tuning_for = |rest| super::step::StepTuning {
        rest,
        arrive_slack: lab.arrive_slack,
        steer_release: lab.steer_release,
        slide: lab.slide,
    };
    let human_tuning = tuning_for(2.0 * human_style.body_radius);
    let demon_tuning = tuning_for(2.0 * super::separation::demon_radius(human_style.body_radius));
    let pushes = pushes.reader(separation.hold, lab.slide);
    let dt = time.delta_secs();

    for (entity, mut movable, mut sim_position, is_human) in &mut query {
        let was_at = sim_position.0;
        // позиция пишется обратно только если шаг её и правда сдвинул:
        // `snapshot_previous_sim_positions` фильтрует по `Changed<SimPosition>`,
        // и безусловная запись расширила бы ему выборку на всех стоящих в
        // приходе и в упёршемся докате
        let mut position = sim_position.0;
        let outcome = super::step::step_along_path(
            &mut movable,
            &mut position,
            dt,
            pushes.modifiers_for(entity),
            if is_human { human_tuning } else { demon_tuning },
            &walkable,
        );
        if position != sim_position.0 {
            sim_position.0 = position;
        }
        match outcome {
            super::step::StepOutcome::Moved => {}
            super::step::StepOutcome::Arrived {
                destination_reached,
            } => movable.to_idle(entity, &mut commands, destination_reached),
            super::step::StepOutcome::Halted => {
                commands.entity(entity).remove::<MovableStateMovingTag>();
            }
        }

        if is_human && let Some(grid) = human_grid.as_mut() {
            grid.moved(entity, was_at, sim_position.0);
        }
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_MOVE_MS, started);
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
pub(super) fn rescue_from_impassable(walkable: &Walkable, pawn: &mut PawnEdit<'_, '_, '_>) -> bool {
    if walkable.allows(pawn.sim_position.0) {
        return false;
    }
    // не нашлось — оставляем как есть: телепорт за пределы кольца увёл бы
    // пешку дальше, чем она вообще могла бы дойти
    let Some(free) = walkable.nearest_free_point(pawn.sim_position.0) else {
        return false;
    };
    pawn.sim_position.0 = free;
    pawn.previous.0 = pawn.sim_position.0;
    pawn.movable.to_idle(pawn.entity, pawn.commands, false);
    true
}

/// Полный скан населения — на каждой готовой постройке полигонального меша:
/// это единственный момент, когда проходимость меняется под уже стоящими
/// пешками (контуры раздуваются на радиус агента, и свободный тайл у стены
/// уходит внутрь препятствия).
///
/// Скана на входе в мир нет намеренно, хотя напрашивается. Сетка к этому
/// моменту финальна (заливка и прунинг сделаны потоком загрузки), а
/// `spawn_population` выбирает тайлы ровно тем же `is_passable` и ставит пешку
/// в центр выбранного тайла — скан проверял бы предикат, который спавн только
/// что применил, и по построению не мог бы найти никого. Проверено живьём на
/// Туле: ни одного спасённого. Меша в этот момент тоже нет — постройка
/// асинхронна и стартует в этом же `OnEnter`, а старый очищен `city::reload`.
///
/// В остальное время застрявших ловит провал поиска
/// (`listen_for_pathfinding_tasks`), и проход по 20 000 сущностей не
/// повторяется ни разу за жизнь города.
pub fn rescue_trapped_entities(
    mut commands: Commands,
    backend: Res<Backend>,
    mut human_grid: Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    mut query: Query<(
        Entity,
        &mut SimPosition,
        &mut PreviousSimPosition,
        &mut Movable,
        Has<crate::human::Human>,
    )>,
) {
    let walkable = backend.walkable();
    let started = std::time::Instant::now();
    let mut rescued = 0;
    for (entity, mut sim_position, mut previous, mut movable, is_human) in &mut query {
        let mut pawn = PawnEdit {
            entity,
            movable: &mut movable,
            sim_position: &mut sim_position,
            previous: &mut previous,
            commands: &mut commands,
        };
        if !rescue_from_impassable(&walkable, &mut pawn) {
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

    let view = Viewport::of(&window, &camera, MOVEPATH_VIEW_SCREENS);

    for (sim_position, movable) in &query {
        // на всю карту это десятки тысяч линий за кадр; за соседним экраном
        // путь всё равно не увидеть
        if !view.contains(sim_position.0) {
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
