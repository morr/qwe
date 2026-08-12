//! Стейт-машина человека: Wander / Flee, спасение за краем карты.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::{ChaseTarget, Demon};
use crate::grid::world_to_tile;
use crate::human::components::{
    FleeRepath, Human, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, PanicRecoil, WanderHeading,
    WanderPause,
};
use crate::human::decide::{
    FLEE_STEP, FleeAction, FleeSense, Threat, ThreatProbe, decide, escaped, flee_target,
};
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::Backend;
use crate::rng::{PawnId, WanderIndex};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_PANIC_RADIUS, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, RADIUS_HYSTERESIS,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

/// Wander → Flee: демон в радиусе паники.
///
/// Цикл инвертирован: не «каждый из ~20 000 гуляющих опрашивает сетку
/// демонов», а «каждый из ~100 демонов собирает соседей по сетке людей».
/// Стоимость пропорциональна толпе возле демонов, а не населению карты —
/// и не меняется, сколько бы людей мирно ни гуляло на другом краю города.
#[allow(clippy::too_many_arguments)]
pub fn panic(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    humans: Res<SpatialGrid<Human>>,
    style: Res<HumanStyle>,
    demons: Query<&SimPosition, With<Demon>>,
    wanderers: Query<&SimPosition, (With<Human>, With<HumanWanderTag>)>,
    seed: Res<crate::rng::WorldSeed>,
    mut movables: Query<(&mut Movable, &Pace, &PawnId, &mut WanderIndex)>,
) {
    let started = std::time::Instant::now();
    // дедуп между демонами: человека в двух радиусах паникуем один раз
    let mut panicked: bevy::platform::collections::HashSet<Entity> =
        bevy::platform::collections::HashSet::default();
    for demon_position in &demons {
        humans.for_each_in_cells_around(demon_position.0, HUMAN_PANIC_RADIUS, |human| {
            if panicked.contains(&human) {
                return;
            }
            // уже бегущие и только что убитые отсеиваются самим запросом
            let Ok(human_position) = wanderers.get(human) else {
                return;
            };
            if human_position.0.distance_squared(demon_position.0)
                <= HUMAN_PANIC_RADIUS * HUMAN_PANIC_RADIUS
            {
                panicked.insert(human);
            }
        });
    }

    for &entity in &panicked {
        // период — из личного потока паникующего, и это не косметика:
        // `panicked` — хэш-множество, его порядок обхода зависит от битов
        // `Entity`, а те после рестарта другие. Общий генератор раздал бы
        // тем же людям другие периоды при том же seed.
        let mut period = 1.0;
        if let Ok((mut movable, pace, pawn_id, mut wander_index)) = movables.get_mut(entity) {
            movable.speed = pace.speed(HUMAN_FLEE_SPEED, style.spread);
            period = wander_index
                .next(seed.0, crate::rng::RngDomain::Human, pawn_id.0)
                .random_range(0.7..1.2);
        }
        let mut repath = FleeRepath::default();
        // первый путь — сразу, дальше по таймеру со случайным периодом
        repath
            .0
            .set_duration(std::time::Duration::from_secs_f32(period));
        commands
            .entity(entity)
            .remove::<HumanWanderTag>()
            // срочность заявки — вместе с паникой: убегающего диспетчер обязан
            // взять и за кадром, иначе паника вне экрана встанет. Маркер живёт
            // ровно столько же, сколько `HumanFleeTag`, и снимается вместе с ним
            .insert((HumanFleeTag, repath, crate::movement::UrgentPath));
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_PANIC_MS, started);
}

/// Flee: бег от ближайшего демона с троттлингом перепрокладки;
/// демоны отстали (×1.5 радиуса) — успокаивается.
///
/// Здесь только применение: какой ступенью обернулся тик, решает чистая
/// [`decide`](crate::human::decide::decide) — она же выбирает, каким вопросом
/// спросить про угрозу. Точный поиск ближайшего демона идёт только на тиках
/// решения (раз в 45–77 тиков на бегущего), на остальных его заменяет
/// проверка занятости ячеек ([`SpatialGrid::any_in_cells_around`]): до этого
/// точный поиск бежал каждый тик у каждого бегущего и стоил 40% тика
/// симуляции (0.42 мс при ~1900 бегущих: они толпятся именно там, где демоны,
/// и каждый заново обходил одни и те же плотные ячейки, дёргая `Query::get`
/// на каждого кандидата).
///
/// На каждой перепрокладке запоминаются **две** величины, и обе — потому, что
/// в ветке успокоения демона в радиусе уже нет: курс прогулки
/// (`WanderHeading`, вектор бегства с личным веером) и запрет
/// (`PanicRecoil`, чистый вектор на демона — [`FleeAction::Flee::ban`]).
/// Устаревание запрета ограничено: период перепрокладки 0.7–1.2 с при
/// скорости бегства 8 м/с — не больше 9.6 м пройденного пути против разрыва в
/// 90 м, и ещё столько же на ход самого демона, то есть ≲13° ошибки при
/// запретном конусе ±45°. Это единственная погрешность, которая в него
/// заложена: веер разбегания в запрет не попадает вовсе, см. `ban`.
#[allow(clippy::too_many_arguments)]
pub fn flee(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    time: Res<Time>,
    backend: Res<Backend>,
    demons: Res<SpatialGrid<Demon>>,
    demon_positions: Query<(&SimPosition, Option<&PawnId>), With<Demon>>,
    chasing: Query<&ChaseTarget, With<Demon>>,
    style: Res<HumanStyle>,
    seed: Res<crate::rng::WorldSeed>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut FleeRepath,
            &mut WanderPause,
            &mut Movable,
            &mut WanderHeading,
            &Pace,
            &PawnId,
            &mut WanderIndex,
        ),
        (With<Human>, With<HumanFleeTag>),
    >,
) {
    let started = std::time::Instant::now();
    let walkable = backend.walkable();
    // за кем прямо сейчас гонятся — те бегут по чистому вектору от демона
    let chased: bevy::platform::collections::HashSet<Entity> =
        chasing.iter().map(|chase_target| chase_target.0).collect();
    let radius = HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS;

    for (
        entity,
        sim_position,
        mut repath,
        mut pause,
        mut movable,
        mut heading,
        pace,
        pawn_id,
        mut wander_index,
    ) in &mut query
    {
        repath.0.tick(time.delta());
        let sense = FleeSense {
            position: sim_position.0,
            pawn_id: pawn_id.0,
            chased: chased.contains(&entity),
            repath_due: repath.0.just_finished(),
            needs_path: matches!(
                movable.state,
                MovableState::Idle | MovableState::PathfindingError(_)
            ),
        };
        let action = decide(&sense, |probe| match probe {
            ThreatProbe::Cells => {
                if demons.any_in_cells_around(sim_position.0, radius) {
                    Threat::Near
                } else {
                    Threat::None
                }
            }
            ThreatProbe::Nearest => demons
                .nearest_in_range(
                    sim_position.0,
                    radius,
                    |demon| crate::spatial::pawn_position(&demon_positions, demon),
                    |demon| crate::spatial::pawn_order(&demon_positions, demon),
                )
                .map_or(Threat::None, |(_, position)| Threat::At(position)),
        });

        let away = match action {
            FleeAction::CalmDown => {
                movable.speed = pace.speed(HUMAN_WALK_SPEED, style.spread);
                movable.to_idle(entity, &mut commands, false);
                pause.0.set_duration(std::time::Duration::from_secs_f32(
                    wander_index
                        .next(seed.0, crate::rng::RngDomain::Human, pawn_id.0)
                        .random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1),
                ));
                pause.0.reset();
                // `PanicRecoil` здесь НЕ ставится: он уже лежит на человеке с
                // последней перепрокладки, где демона было ещё видно. Здесь
                // его взять неоткуда — ветка и срабатывает потому, что поиск
                // никого не нашёл
                commands
                    .entity(entity)
                    .remove::<(HumanFleeTag, FleeRepath, crate::movement::UrgentPath)>()
                    .insert(HumanWanderTag);
                continue;
            }
            FleeAction::Hold => continue,
            FleeAction::Flee { away, ban } => {
                // память о направлении угрозы — до отсева непроходимой цели,
                // иначе неудачный кадр оставил бы запрет от прошлой
                // перепрокладки, а то и вовсе без запрета
                commands.entity(entity).insert(PanicRecoil(ban));
                away
            }
        };

        // память о направлении угрозы — пишется до отсева непроходимой цели,
        // иначе неудачный кадр оставил бы курс от прошлой перепрокладки
        heading.0 = away;
        // поток заводится здесь, а не заранее на итерацию: `next` сдвигает
        // номер решения, и заведённый заранее поток крутил бы счётчик каждый
        // тик у каждого бегущего — в том числе на тиках, где решения нет
        // вовсе. По той же причине своё `next` у ветки успокоения
        let step = wander_index
            .next(seed.0, crate::rng::RngDomain::Human, pawn_id.0)
            .random_range(FLEE_STEP.0..FLEE_STEP.1);
        let target = flee_target(sim_position.0, away, step);

        let Some(target_tile) = walkable.sift_target(world_to_tile(target)) else {
            continue;
        };
        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            target_tile,
            &mut commands,
        );
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_FLEE_MS, started);
}

/// Паникующий пересёк границу карты — «спасся», despawn [Q12].
pub fn escape(
    mut commands: Commands,
    mut telemetry: ResMut<Telemetry>,
    query: Query<(Entity, &SimPosition), (With<Human>, With<HumanFleeTag>)>,
) {
    for (entity, sim_position) in &query {
        if escaped(sim_position.0) {
            commands.entity(entity).despawn();
            telemetry.escaped += 1;
            debug!("human {entity} escaped (total {})", telemetry.escaped);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::rng::WorldSeed;

    /// Мир бегства: общий двор плюс то, что нужно именно людям — сетка
    /// демонов, от которых бегут, их стиль и жребий.
    fn app() -> App {
        let mut app = crate::sim_yard::behavior_yard();
        app.init_resource::<SpatialGrid<Demon>>()
            .init_resource::<HumanStyle>()
            .init_resource::<WorldSeed>()
            .add_systems(Update, flee);
        app
    }

    /// Демон, попадающий и в запрос позиций, и в сетку, — иначе точный поиск
    /// его не найдёт и лестница уйдёт в успокоение.
    fn spawn_demon(app: &mut App, position: Vec2) -> Entity {
        let demon = app.world_mut().spawn((Demon, SimPosition(position))).id();
        app.world_mut()
            .resource_mut::<SpatialGrid<Demon>>()
            .insert(demon, position);
        demon
    }

    /// Бегущий с досчитавшим таймером перепрокладки: тик решения, точный
    /// поиск. `heading` — курс с прошлой перепрокладки.
    fn spawn_runner(app: &mut App, position: Vec2, heading: Vec2) -> Entity {
        let mut repath = FleeRepath::default();
        // таймер обязан досчитать на первом же тике, иначе решения не будет
        repath.0.tick(Duration::from_secs(2));
        app.world_mut()
            .spawn((
                Human,
                HumanFleeTag,
                SimPosition(position),
                repath,
                WanderPause(Timer::from_seconds(1.0, TimerMode::Once)),
                Movable::new(1.0),
                WanderHeading(heading),
                Pace(1.0),
                PawnId(3),
                crate::rng::WanderIndex::default(),
            ))
            .id()
    }

    fn recoil(app: &App, entity: Entity) -> Option<Vec2> {
        app.world()
            .get::<PanicRecoil>(entity)
            .map(|recoil| recoil.0)
    }

    /// Проводка того, что решает [`decide`]: запрет, снятый с чистого вектора
    /// на демона, обязан доехать до компонента.
    ///
    /// Курс здесь заведомо «неправильный» — смотрит вбок, а не от демона.
    /// Синтез запрета из курса (`-WanderHeading`), как было раньше, дал бы
    /// ровно его, и тест это отличает.
    #[test]
    fn a_repath_remembers_where_the_demon_was_not_where_the_runner_looks() {
        let app = &mut app();
        let position = Vec2::new(100.0, 100.0);
        spawn_demon(app, position - Vec2::X * 20.0);
        let runner = spawn_runner(app, position, Vec2::Y);

        app.update();

        let ban = recoil(app, runner).expect("перепрокладка обязана запомнить угрозу");
        assert!(
            (ban - Vec2::NEG_X).length() < 1e-3,
            "запрет {ban:?} смотрит не на демона"
        );
    }

    /// Успокоение НЕ трогает запрет: демона в радиусе нет, взять новый
    /// неоткуда, а прежний — единственное, что о нём известно.
    #[test]
    fn calming_down_keeps_the_ban_the_last_repath_left() {
        let app = &mut app();
        // демонов в мире нет вовсе — первый же тик успокаивает
        let runner = spawn_runner(app, Vec2::new(100.0, 100.0), Vec2::Y);
        app.world_mut()
            .entity_mut(runner)
            .insert(PanicRecoil(Vec2::NEG_X));

        app.update();

        assert!(
            app.world().get::<HumanWanderTag>(runner).is_some(),
            "демонов нет — человек обязан успокоиться"
        );
        assert_eq!(
            recoil(app, runner),
            Some(Vec2::NEG_X),
            "успокоение переписало запрет курсом прогулки"
        );
    }
}
