//! Стейт-машина демона: Wander / Chase / Devour.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::claims::ChaseClaims;
use crate::demon::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonLungeTag, DemonStyle, DemonWanderTag, DevourUntil,
};
use crate::demon::decide::{ChaseAction, ChaseSense, decide};
use crate::grid::world_to_tile;
use crate::human::Human;
use crate::movement::{Movable, MovableState, PathfindingRequest, PathfindingTask, SimPosition};
use crate::navigation::Backend;
use crate::settings::{DEMON_AGGRO_RADIUS, DEMON_DEVOUR_PAUSE};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

/// Wander → Chase: ближайший человек в радиусе агро, у которого ещё нет
/// `MAX_CHASERS_PER_TARGET` преследователей. Демон берёт агро с первого же
/// тика после выхода из портала.
pub fn acquire_targets(
    mut commands: Commands,
    humans: Res<SpatialGrid<Human>>,
    positions: Query<(&SimPosition, Option<&crate::rng::PawnId>), With<Human>>,
    chasing: Query<&ChaseTarget, With<Demon>>,
    query: Query<(Entity, &SimPosition), (With<Demon>, With<DemonWanderTag>)>,
) {
    let mut claims = ChaseClaims::of(chasing.iter().map(|chase_target| chase_target.0));

    for (entity, sim_position) in &query {
        let Some((human, _)) = humans.nearest_in_range_where(
            sim_position.0,
            DEMON_AGGRO_RADIUS,
            |candidate| crate::spatial::pawn_position(&positions, candidate),
            |candidate| crate::spatial::pawn_order(&positions, candidate),
            |candidate| !claims.is_full(candidate),
        ) else {
            continue;
        };
        claims.claim(human);

        // скорость здесь не трогаем: она одна на все состояния демона и живёт
        // в `Movable::speed` со спавна (`sync_demon_speed` ведёт её дальше)
        commands.entity(entity).remove::<DemonWanderTag>().insert((
            DemonChaseTag,
            ChaseTarget(human),
            ChaseRepath::default(),
        ));
        debug!("demon {entity} Wander => Chase {human}");
    }
}

/// Chase: догоняем цель; цель умерла/сбежала/далеко — обратно в Wander;
/// догнали — `DemonCaughtHumanEvent`. В такт перепрокладки пробуем сменить
/// цель: если делим её с другим демоном — на никем не занятого человека не
/// дальше ×1.5 текущей дистанции, иначе — на любого, кто ближе ×0.7 её.
/// В обоих случаях только при прямой видимости. Вблизи — бросок напрямую
/// (см. `DEMON_LUNGE_RANGE`).
///
/// Здесь только применение: какая ступень лестницы сработала, решает чистая
/// [`decide`] — там же лежат все пороги и их обоснования, там же они и
/// проверяются, без `App` и навмеша.
///
/// `Without<Human>` в фильтре обязателен: обе выборки трогают `SimPosition`,
/// и без него планировщик видит конфликт доступа.
#[allow(clippy::too_many_arguments)]
pub fn chase(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    time: Res<Time>,
    style: Res<DemonStyle>,
    backend: Res<Backend>,
    humans: Res<SpatialGrid<Human>>,
    mut query: Query<
        (
            Entity,
            &mut SimPosition,
            &mut ChaseTarget,
            &mut ChaseRepath,
            &mut Movable,
            Has<DemonLungeTag>,
            Has<PathfindingTask>,
            Has<PathfindingRequest>,
        ),
        (With<Demon>, With<DemonChaseTag>, Without<Human>),
    >,
    targets: Query<(&SimPosition, Option<&crate::rng::PawnId>), With<Human>>,
) {
    let started = std::time::Instant::now();
    let walkable = backend.walkable();
    // один труп — одно убийство: дедупликация внутри тика, пока команды
    // (снятие `Human`) ещё не применились
    let mut killed_this_tick: bevy::platform::collections::HashSet<Entity> =
        bevy::platform::collections::HashSet::default();

    let mut claims = ChaseClaims::of(query.iter().map(|(_, _, chase_target, ..)| chase_target.0));

    for (
        entity,
        mut sim_position,
        mut chase_target,
        mut repath,
        mut movable,
        lunging,
        has_task,
        has_request,
    ) in &mut query
    {
        let target = targets
            .get(chase_target.0)
            .ok()
            .map(|(position, _)| position.0)
            .filter(|_| !killed_this_tick.contains(&chase_target.0));
        let sense = ChaseSense {
            position: sim_position.0,
            target,
            speed: movable.speed,
            lunge_bonus: style.lunge,
            delta_secs: time.delta_secs(),
            state: movable.state.clone(),
            has_path: !movable.path.is_empty(),
            walked: movable.last_direction != Vec2::ZERO,
            search_in_flight: has_task || has_request,
            // спрашиваем таймер, а не крутим его: тикать он обязан только на
            // тех ступенях, до которых лестница дошла, — бросок и ожидание
            // первого пути его замораживают
            repath_due: repath.0.remaining() <= time.delta(),
            shared_target: claims.is_full(chase_target.0),
        };
        let action = decide(&sense, || {
            target.is_some_and(|position| walkable.line_of_sight(sim_position.0, position))
        });

        // цель разорвала дистанцию или ушла за угол — бросок отменён
        if lunging && action.cancels_lunge() {
            commands.entity(entity).remove::<DemonLungeTag>();
        }
        // какие выходы из погони освобождают место в очереди на жертву —
        // правило `ChaseAction`, не этой лестницы
        if action.releases_claim() {
            claims.release(chase_target.0);
        }

        let (mut target_pos, switch_rule) = match action {
            // выходы из погони отличались только тем, освобождает ли выход
            // место в очереди на жертву, — а это теперь спрошено выше
            ChaseAction::LostTarget | ChaseAction::GaveUp => {
                back_to_wander(&mut commands, entity);
                continue;
            }
            ChaseAction::Kill => {
                killed_this_tick.insert(chase_target.0);
                commands.trigger(DemonCaughtHumanEvent {
                    demon: entity,
                    human: chase_target.0,
                });
                continue;
            }
            ChaseAction::Lunge { advance } => {
                // путь больше не нужен: дальше демона ведёт бросок, а не
                // `move_moving_entities`. Надбавка к скорости живёт внутри
                // `advance`, а не в `Movable::speed`, — снимать её с выходом
                // из броска не нужно, её просто некому унести.
                if !matches!(movable.state, MovableState::Idle) {
                    movable.to_idle(entity, &mut commands, false);
                }
                if !lunging {
                    commands.entity(entity).insert(DemonLungeTag);
                }
                sim_position.0 += advance;
                continue;
            }
            ChaseAction::WaitForPath => continue,
            ChaseAction::Hold => {
                repath.0.tick(time.delta());
                continue;
            }
            ChaseAction::Repath { target, switch } => {
                repath.0.tick(time.delta());
                (target, switch)
            }
        };

        let switch = humans
            .nearest_in_range_where(
                sim_position.0,
                switch_rule.radius,
                |candidate| crate::spatial::pawn_position(&targets, candidate),
                |candidate| crate::spatial::pawn_order(&targets, candidate),
                |candidate| {
                    candidate != chase_target.0
                        && !killed_this_tick.contains(&candidate)
                        && claims.has_room_for(candidate, switch_rule.max_chasers)
                },
            )
            // Прямая видимость — только у победителя поиска: близкий по евклиду,
            // но отрезанный домом или рекой человек недостижим, и демон топтался
            // бы, перекидывая цель туда-обратно. В фильтре выше `line_of_sight`
            // прогнался бы по каждому кандидату в 3×3 клетках (при 20 000
            // человек это десятки), поэтому он здесь. Не прошёл — цель остаётся,
            // следующая попытка через такт перепрокладки.
            .filter(|&(_, pos)| walkable.line_of_sight(sim_position.0, pos));
        if let Some((new_target, new_pos)) = switch {
            claims.release(chase_target.0);
            claims.claim(new_target);
            debug!(
                "demon {entity} switches chase {} => {new_target}",
                chase_target.0
            );
            chase_target.0 = new_target;
            target_pos = new_pos;
        }

        let target_tile = world_to_tile(target_pos);
        let current_goal = match movable.state {
            MovableState::Moving(goal) | MovableState::Pathfinding(goal) => Some(goal),
            _ => None,
        };
        if current_goal == Some(target_tile) {
            continue;
        }

        let Some(goal_tile) = walkable.sift_target(target_tile) else {
            continue;
        };
        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            goal_tile,
            &mut commands,
        );
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_CHASE_MS, started);
}

fn back_to_wander(commands: &mut Commands, entity: Entity) {
    commands
        .entity(entity)
        .remove::<(DemonChaseTag, ChaseTarget, ChaseRepath, DemonLungeTag)>()
        .insert(DemonWanderTag);
    debug!("demon {entity} Chase => Wander");
}

/// Наблюдатель убийства: человек становится трупом, демон — в Devour.
pub fn on_demon_caught_human(
    event: On<DemonCaughtHumanEvent>,
    mut commands: Commands,
    mut telemetry: ResMut<Telemetry>,
    humans: Query<(), With<Human>>,
    seed: Res<crate::rng::WorldSeed>,
    mut movables: Query<(
        &mut Movable,
        &crate::rng::PawnId,
        &mut crate::rng::WanderIndex,
    )>,
) {
    let DemonCaughtHumanEvent { demon, human } = *event;

    // два демона могли догнать одновременно — труп не убивают дважды
    if humans.get(human).is_err() {
        return;
    }

    // из чего состоит человек и что таскает за собой движение — знают человек
    // и движение; отсюда видно только, что случилось
    crate::human::to_corpse(&mut commands, human);
    telemetry.killed += 1;

    // демон → Devour; пауза — из личного потока демона, а не общего: убийства
    // прилетают обсерверами, и их порядок в тике задан порядком команд
    let mut pause = DEMON_DEVOUR_PAUSE.0;
    if let Ok((mut movable, pawn_id, mut wander_index)) = movables.get_mut(demon) {
        movable.to_idle(demon, &mut commands, false);
        pause = wander_index
            .next(seed.0, crate::rng::RngDomain::Demon, pawn_id.0)
            .random_range(DEMON_DEVOUR_PAUSE.0..DEMON_DEVOUR_PAUSE.1);
    }
    commands
        .entity(demon)
        .remove::<(
            DemonChaseTag,
            ChaseTarget,
            ChaseRepath,
            DemonLungeTag,
            PathfindingTask,
            PathfindingRequest,
        )>()
        .insert((
            DemonDevourTag,
            DevourUntil(Timer::from_seconds(pause, TimerMode::Once)),
        ));
    debug!(
        "demon {demon} Chase => Devour (killed {human}, total {})",
        telemetry.killed
    );
}

/// Devour → Wander по истечении паузы.
pub fn devour(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<
        (Entity, &mut DevourUntil, &mut Transform),
        (With<Demon>, With<DemonDevourTag>),
    >,
) {
    for (entity, mut devour_until, mut transform) in &mut query {
        devour_until.0.tick(time.delta());
        if !devour_until.0.is_finished() {
            continue;
        }
        transform.scale = Vec3::ONE;
        commands
            .entity(entity)
            .remove::<(DemonDevourTag, DevourUntil)>()
            .insert(DemonWanderTag);
        debug!("demon {entity} Devour => Wander");
    }
}

/// Период пульсации пожирающего демона, сек.
const DEVOUR_PULSE_PERIOD: f32 = 0.5;
/// Амплитуда пульсации: от ×1 до ×1.5 размера.
const DEVOUR_PULSE_MAX_SCALE: f32 = 1.5;

/// Пожирающий демон пульсирует: размер ходит по синусоиде ×1 → ×1.5 → ×1.
pub fn pulse_devouring(
    mut query: Query<(&DevourUntil, &mut Transform), (With<Demon>, With<DemonDevourTag>)>,
) {
    use std::f32::consts::TAU;

    for (devour_until, mut transform) in &mut query {
        let phase = devour_until.0.elapsed_secs() / DEVOUR_PULSE_PERIOD * TAU;
        let scale = 1.0 + (DEVOUR_PULSE_MAX_SCALE - 1.0) * 0.5 * (1.0 - phase.cos());
        transform.scale = Vec3::splat(scale);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    use super::*;
    use crate::navigation::{ArcNavmesh, Backend, Navmesh};

    /// Бэкенд, которым ищет `chase`: пустая проходимая сетка, ни меша, ни
    /// иерархии — как на старте мира.
    fn app() -> App {
        let mut app = App::new();
        let navmesh = Arc::new(RwLock::new(Navmesh::default()));
        app.insert_resource(Backend::from_grid(navmesh.clone()))
            .insert_resource(ArcNavmesh(navmesh))
            .init_resource::<bevy::diagnostic::DiagnosticsStore>()
            .init_resource::<SpatialGrid<Human>>()
            .init_resource::<DemonStyle>()
            .init_resource::<Time>()
            .add_systems(Update, chase);
        app
    }

    /// Демон в погоне с поиском в полёте: заявка подана к `pending_goal`,
    /// пути ещё нет. `last_direction` задаёт, шагал ли он хоть раз.
    fn spawn_chaser(app: &mut App, target: Entity, pending_goal: IVec2, walked: bool) -> Entity {
        spawn_chaser_at(app, Vec2::new(10.0, 10.0), target, pending_goal, walked)
    }

    fn spawn_chaser_at(
        app: &mut App,
        position: Vec2,
        target: Entity,
        pending_goal: IVec2,
        walked: bool,
    ) -> Entity {
        let mut movable = Movable::new(1.0);
        movable.state = MovableState::Pathfinding(pending_goal);
        if walked {
            movable.last_direction = Vec2::X;
        }
        app.world_mut()
            .spawn((
                Demon,
                DemonChaseTag,
                ChaseTarget(target),
                ChaseRepath::default(),
                movable,
                SimPosition(position),
                PathfindingRequest {
                    start_tile: world_to_tile(position),
                    end_tile: pending_goal,
                },
            ))
            .id()
    }

    /// Такт с дельтой больше периода `ChaseRepath` — перепрокладке пора.
    fn run_repath_tick(app: &mut App) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0));
        app.update();
    }

    #[test]
    fn a_chaser_without_a_first_path_keeps_its_search_in_flight() {
        let app = &mut app();
        let human = app
            .world_mut()
            .spawn((Human, SimPosition(Vec2::new(30.0, 10.0))))
            .id();
        let pending_goal = IVec2::new(5, 5);
        let demon = spawn_chaser(app, human, pending_goal, false);

        run_repath_tick(app);

        // заявка в полёте не тронута: без неё демону нечем идти — ни пути,
        // ни доката, — и отмена замораживала бы его до конца погони
        let request = app
            .world()
            .get::<PathfindingRequest>(demon)
            .expect("заявка обязана пережить такт перепрокладки");
        assert_eq!(request.end_tile, pending_goal);
        assert_eq!(
            app.world().get::<Movable>(demon).expect("Movable").state,
            MovableState::Pathfinding(pending_goal)
        );
    }

    #[test]
    fn a_chaser_that_has_walked_repaths_to_the_target() {
        let app = &mut app();
        let target_position = Vec2::new(30.0, 10.0);
        let human = app
            .world_mut()
            .spawn((Human, SimPosition(target_position)))
            .id();
        let demon = spawn_chaser(app, human, IVec2::new(5, 5), true);

        run_repath_tick(app);

        // у шагавшего демона есть докат — устаревшую заявку штатно вытесняет
        // перепрокладка к текущему тайлу цели
        let target_tile = world_to_tile(target_position);
        let request = app
            .world()
            .get::<PathfindingRequest>(demon)
            .expect("перепрокладка обязана подать новую заявку");
        assert_eq!(request.end_tile, target_tile);
        assert_eq!(
            app.world().get::<Movable>(demon).expect("Movable").state,
            MovableState::Pathfinding(target_tile)
        );
    }

    fn spawn_human(app: &mut App, position: Vec2) -> Entity {
        let human = app.world_mut().spawn((Human, SimPosition(position))).id();
        app.world_mut()
            .resource_mut::<SpatialGrid<Human>>()
            .insert(human, position);
        human
    }

    /// Место, освобождённое отставшим демоном, видит тот, кто делил с ним
    /// жертву, — и перестаёт вести себя как половина «клещей».
    ///
    /// Единственное наблюдаемое следствие освобождения заявки внутри тика:
    /// раскрываются «клещи» — демон в них ищет замену вдвое шире (×1.5 против
    /// ×0.7 дистанции) и берёт только никем не занятого. Ушедший напарник
    /// снимает это правило, и свободная жертва посередине между двумя
    /// радиусами перестаёт быть кандидатом.
    #[test]
    fn a_partner_giving_up_takes_the_shared_target_rule_with_him() {
        let app = &mut app();
        // 100 м от отставшего — дальше гистерезиса (45 × 1.5 = 67.5): он сдаётся
        let shared = spawn_human(app, Vec2::new(100.0, 10.0));
        // 45 м от остающегося: дальше ×0.7 его дистанции (28 м), но ближе ×1.5 (60 м)
        let free = spawn_human(app, Vec2::new(60.0, 55.0));
        // порядок спавна — порядок обхода: сдающийся обязан пройти первым
        spawn_chaser_at(app, Vec2::ZERO, shared, IVec2::new(5, 5), true);
        let stays = spawn_chaser_at(app, Vec2::new(60.0, 10.0), shared, IVec2::new(5, 5), true);

        run_repath_tick(app);

        assert_eq!(
            app.world()
                .get::<ChaseTarget>(stays)
                .expect("ChaseTarget")
                .0,
            shared,
            "жертва больше не делится — свободная в {free} слишком далеко для смены цели"
        );
    }
}
