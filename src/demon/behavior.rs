//! Стейт-машина демона: Wander / Chase / Devour.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::claims::ChaseClaims;
use crate::demon::components::{
    ChaseComponents, ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag,
    DemonDevourTag, DemonLungeTag, DemonStyle, DemonWanderTag, DevourUntil,
};
use crate::demon::decide::{ChaseAction, ChaseSense, Victim, decide};
use crate::grid::world_to_tile;
use crate::human::Human;
use crate::movement::{
    Movable, MovableState, PathfindingRequest, PathfindingTask, SimPosition, request_wander_path,
};
use crate::navigation::Backend;
use crate::settings::{
    DEMON_AGGRO_RADIUS, DEMON_DEVOUR_PAUSE, DEVOUR_PULSE_MAX_SCALE, DEVOUR_PULSE_PERIOD,
};
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
        let action = decide(
            &sense,
            || target.is_some_and(|position| walkable.line_of_sight(sim_position.0, position)),
            // Поиск замены — чувство, а не хвост применения: лестница зовёт его
            // сама и ровно на своей ступени. Прямая видимость проверяется
            // только у победителя поиска: близкий по евклиду, но отрезанный
            // домом или рекой человек недостижим, а гонять `line_of_sight` по
            // каждому кандидату в 3×3 клетках (при 20 000 человек это десятки)
            // — совсем другие деньги.
            |rule| {
                humans
                    .nearest_in_range_where(
                        sim_position.0,
                        rule.radius,
                        |candidate| crate::spatial::pawn_position(&targets, candidate),
                        |candidate| crate::spatial::pawn_order(&targets, candidate),
                        |candidate| {
                            candidate != chase_target.0
                                && !killed_this_tick.contains(&candidate)
                                && claims.has_room_for(candidate, rule.max_chasers)
                        },
                    )
                    .filter(|&(_, position)| walkable.line_of_sight(sim_position.0, position))
                    .map(|(entity, position)| Victim { entity, position })
            },
        );

        // цель разорвала дистанцию или ушла за угол — бросок отменён
        if lunging && action.cancels_lunge() {
            commands.entity(entity).remove::<DemonLungeTag>();
        }
        // какие выходы из погони освобождают место в очереди на жертву —
        // правило `ChaseAction`, не этой лестницы
        if action.releases_claim() {
            claims.release(chase_target.0);
        }

        let target_pos = match action {
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
            ChaseAction::Repath { target } => {
                repath.0.tick(time.delta());
                target
            }
            ChaseAction::Switch { to } => {
                repath.0.tick(time.delta());
                // место переезжает одной операцией: раньше это была пара
                // `release` + `claim`, четвёртый сайт правки счёта и
                // единственный, мимо которого шёл исчерпывающий `match`
                claims.transfer(chase_target.0, to.entity);
                debug!(
                    "demon {entity} switches chase {} => {}",
                    chase_target.0, to.entity
                );
                chase_target.0 = to.entity;
                to.position
            }
        };

        let target_tile = world_to_tile(target_pos);
        let current_goal = match movable.state {
            MovableState::Moving(goal) | MovableState::Pathfinding(goal) => Some(goal),
            _ => None,
        };
        if current_goal == Some(target_tile) {
            continue;
        }

        // хвост скелета прогулки: просев цели и подача заявки — один и тот же
        // шаг независимо от того, кто выбрал цель. Возврат (фактически
        // выбранный тайл) здесь не нужен: курса у демона нет, его ведёт цель
        // погони, а не память о направлении.
        request_wander_path(
            &mut commands,
            &walkable,
            entity,
            &mut movable,
            sim_position.0,
            target_pos,
        );
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_CHASE_MS, started);
}

/// Выход из погони без убийства: снимается набор погони — и только он.
///
/// `Movable` здесь не трогается вовсе, и поэтому заявка/таск поиска обязаны
/// остаться в полёте: демон дойдёт по ним до последнего известного места
/// жертвы, там `to_idle` вернёт ему `NeedsWanderTarget`, и блуждание выберет
/// новую цель. Снятая здесь заявка оставила бы его в
/// `MovableState::Pathfinding` без заявки и без метки — то есть навсегда
/// (`pick_wander_targets` требует `ready_to_pick`).
fn back_to_wander(commands: &mut Commands, entity: Entity) {
    commands
        .entity(entity)
        .remove::<ChaseComponents>()
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
    // прилетают обсерверами, и их порядок в тике задан порядком команд.
    // **Убийство стоит демону одного номера решения** — единственный сдвиг
    // `WanderIndex` не из лестницы решений; обсервер зовётся со сброса команд
    // `chase`, то есть внутри того же тика `FixedUpdate`, и сдвиг входит в
    // контракт повтора (пин — `a_kill_spends_the_demons_next_decision_number`)
    let mut pause = DEMON_DEVOUR_PAUSE.0;
    if let Ok((mut movable, pawn_id, mut wander_index)) = movables.get_mut(demon) {
        movable.to_idle(demon, &mut commands, false);
        pause = wander_index
            .next(seed.0, crate::rng::RngDomain::Demon, pawn_id.0)
            .random_range(DEMON_DEVOUR_PAUSE.0..DEMON_DEVOUR_PAUSE.1);
    }
    // сверх набора погони — заявка и таск поиска: ответ, прилетевший в
    // Devour, прошёл бы через `accept_answer` → `Movable::to_moving` и
    // увёл бы демона с трупа посреди паузы. Снять их можно ровно потому,
    // что `to_idle` выше уже перевёл `Movable` в `Idle` и вернул
    // `NeedsWanderTarget`; в `back_to_wander`, где `Movable` не трогают,
    // то же снятие заморозило бы демона навсегда
    commands
        .entity(demon)
        .remove::<(ChaseComponents, PathfindingTask, PathfindingRequest)>()
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
    use std::time::Duration;

    use super::*;
    use crate::navigation::ArcNavmesh;

    /// Мир погони: общий двор плюс то, что нужно именно демонам — сетка людей,
    /// за которыми гонятся, и их стиль.
    fn app() -> App {
        let mut app = crate::sim_yard::behavior_yard();
        app.init_resource::<SpatialGrid<Human>>()
            .init_resource::<DemonStyle>()
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

    /// Заткнуть тайл и всех восьмерых соседей — ровно ту окрестность, в
    /// которой ищет `Walkable::sift_target`.
    fn block_3x3(app: &mut App, tile: IVec2) {
        let mut navmesh = app.world().resource::<ArcNavmesh>().write();
        for dx in -1..=1 {
            for dy in -1..=1 {
                navmesh.set_passable(tile.x + dx, tile.y + dy, false);
            }
        }
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

    /// Цель непроходима и рядом ничего нет: перепрокладка молча пропускает
    /// демона, а старая заявка доживает такт — отменять её нечем, новой нет.
    ///
    /// Это та самая ветка, которая после перевода хвоста на
    /// `request_wander_path` выражается его `?`, а не собственным `else`.
    #[test]
    fn a_chaser_whose_target_has_no_passable_tile_keeps_its_request() {
        let app = &mut app();
        let target_position = Vec2::new(30.0, 10.0);
        let human = app
            .world_mut()
            .spawn((Human, SimPosition(target_position)))
            .id();
        let pending_goal = IVec2::new(5, 5);
        let demon = spawn_chaser(app, human, pending_goal, true);
        block_3x3(app, world_to_tile(target_position));

        run_repath_tick(app);

        let request = app
            .world()
            .get::<PathfindingRequest>(demon)
            .expect("старая заявка обязана пережить неудачный просев");
        assert_eq!(request.end_tile, pending_goal);
        assert_eq!(
            app.world().get::<Movable>(demon).expect("Movable").state,
            MovableState::Pathfinding(pending_goal)
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

    /// Выход из погони без убийства снимает набор погони — и **только** его.
    ///
    /// Заявка поиска остаётся в полёте намеренно: `back_to_wander` не трогает
    /// `Movable`, и снятая здесь заявка оставила бы демона в
    /// `MovableState::Pathfinding` без заявки и без `NeedsWanderTarget` —
    /// `pick_wander_targets` такого не поднимет никогда.
    #[test]
    fn a_demon_giving_up_the_chase_keeps_its_search_in_flight() {
        let app = &mut app();
        // 100 м — дальше гистерезиса (45 × 1.5 = 67.5): демон сдаётся
        let human = spawn_human(app, Vec2::new(100.0, 10.0));
        let pending_goal = IVec2::new(5, 5);
        let demon = spawn_chaser_at(app, Vec2::ZERO, human, pending_goal, true);
        // бросок в наборе погони есть, а `GaveUp` его не отменяет
        // (`ChaseAction::cancels_lunge`) — унести его обязан выход из погони
        app.world_mut().entity_mut(demon).insert(DemonLungeTag);

        run_repath_tick(app);

        assert!(app.world().get::<DemonWanderTag>(demon).is_some());
        for absent in [
            app.world().get::<DemonChaseTag>(demon).is_some(),
            app.world().get::<ChaseTarget>(demon).is_some(),
            app.world().get::<ChaseRepath>(demon).is_some(),
            app.world().get::<DemonLungeTag>(demon).is_some(),
        ] {
            assert!(!absent, "набор погони обязан сняться целиком");
        }
        let request = app
            .world()
            .get::<PathfindingRequest>(demon)
            .expect("заявка обязана пережить выход из погони");
        assert_eq!(request.end_tile, pending_goal);
        assert_eq!(
            app.world().get::<Movable>(demon).expect("Movable").state,
            MovableState::Pathfinding(pending_goal)
        );
    }

    /// Двор убийства: к погоне добавлены обсервер убийства и то, что он читает.
    fn kill_app() -> App {
        let mut app = app();
        app.init_resource::<Telemetry>()
            .insert_resource(crate::rng::WorldSeed(42))
            .add_observer(on_demon_caught_human);
        app
    }

    /// Убийство тратит **один** номер решения демона и берёт паузу из его
    /// личного потока — тот же номер, который иначе достался бы следующему
    /// выбору цели. Поток пешки сдвигается здесь вне лестницы решений, и это
    /// часть контракта повтора: сдвиг обязан случаться на тех же тиках.
    #[test]
    fn a_kill_spends_the_demons_next_decision_number() {
        let app = &mut kill_app();
        // ближе KILL_DISTANCE (1 м) — лестница отвечает Kill до всех прочих ступеней
        let human = spawn_human(app, Vec2::new(10.5, 10.0));
        let demon = spawn_chaser(app, human, IVec2::new(5, 5), true);
        app.world_mut()
            .entity_mut(demon)
            .insert((crate::rng::PawnId(7), crate::rng::WanderIndex::ready()));

        app.update();

        let expected = crate::rng::decision_stream(
            42,
            crate::rng::RngDomain::Demon,
            7,
            crate::rng::WanderIndex::ready().0,
        )
        .random_range(DEMON_DEVOUR_PAUSE.0..DEMON_DEVOUR_PAUSE.1);
        let devour = app.world().get::<DevourUntil>(demon).expect("DevourUntil");
        assert_eq!(
            devour.0.duration(),
            Duration::from_secs_f32(expected),
            "пауза пожирания взята не из личного потока демона"
        );
        assert_eq!(
            app.world()
                .get::<crate::rng::WanderIndex>(demon)
                .expect("WanderIndex")
                .0,
            crate::rng::WanderIndex::ready().0 + 1,
            "убийство обязано стоить ровно один номер решения"
        );
    }

    /// Стенд убийства: от двора нужны только часы, а сверх него — счётчики,
    /// зерно (пауза пожирания тянется из потока демона) и сам обсервер.
    fn devour_app() -> App {
        let mut app = crate::sim_yard::behavior_yard();
        app.insert_resource(crate::rng::WorldSeed(1))
            .init_resource::<crate::telemetry::Telemetry>()
            .add_observer(on_demon_caught_human);
        app
    }

    /// Обсервер убийства снимает сверх набора погони заявку и таск поиска:
    /// ответ, прилетевший в Devour, увёл бы демона с трупа. Законно это лишь
    /// вместе с `to_idle` — поэтому тест проверяет обе половины сразу.
    #[test]
    fn a_demon_starting_to_devour_drops_its_search_and_parks_its_movable() {
        let app = &mut devour_app();
        let human = app.world_mut().spawn((Human, SimPosition(Vec2::ZERO))).id();
        let pending_goal = IVec2::new(5, 5);
        let mut movable = Movable::new(1.0);
        movable.state = MovableState::Pathfinding(pending_goal);
        let demon = app
            .world_mut()
            .spawn((
                Demon,
                DemonChaseTag,
                ChaseTarget(human),
                ChaseRepath::default(),
                DemonLungeTag,
                movable,
                SimPosition(Vec2::ZERO),
                crate::rng::PawnId(0),
                crate::rng::WanderIndex::ready(),
                PathfindingRequest {
                    start_tile: IVec2::ZERO,
                    end_tile: pending_goal,
                },
            ))
            .id();
        // как у настоящего преследователя: метку снял `to_pathfinding`
        app.world_mut()
            .entity_mut(demon)
            .remove::<crate::movement::NeedsWanderTarget>();

        app.world_mut()
            .trigger(DemonCaughtHumanEvent { demon, human });
        app.world_mut().flush();

        assert!(app.world().get::<DemonDevourTag>(demon).is_some());
        assert!(app.world().get::<DemonChaseTag>(demon).is_none());
        assert!(
            app.world().get::<PathfindingRequest>(demon).is_none(),
            "ответ поиска увёл бы демона с трупа посреди пожирания"
        );
        assert!(app.world().get::<PathfindingTask>(demon).is_none());
        // то, что делает снятие заявки безопасным
        assert_eq!(
            app.world().get::<Movable>(demon).expect("Movable").state,
            MovableState::Idle
        );
        assert!(
            app.world()
                .get::<crate::movement::NeedsWanderTarget>(demon)
                .is_some(),
            "без метки демон не поднимется в блуждание после паузы"
        );
    }
}
