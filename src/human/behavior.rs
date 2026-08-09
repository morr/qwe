//! Стейт-машина человека: Wander / Flee, спасение за краем карты.

use bevy::prelude::*;
use rand::Rng;

use crate::demon::{ChaseTarget, Demon};
use crate::grid::world_to_tile;
use crate::human::components::{
    FleeRepath, Human, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, PanicRecoil, WanderHeading,
    WanderPause,
};
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::{Pathfinder, find_passable_tile_near};
use crate::rng::{PawnId, WanderIndex, hash_fraction};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_PANIC_RADIUS, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, MAP_SIZE,
    RADIUS_HYSTERESIS,
};
use crate::spatial::SpatialGrid;
use crate::telemetry::Telemetry;

/// Шаг бегства: насколько далеко от себя прокладывается точка «от демона», м.
const FLEE_STEP: (f32, f32) = (40.0, 60.0);
/// Зона у границы карты, попадание в которую при бегстве — «спасся», м.
const ESCAPE_MARGIN: f32 = 2.0;
/// Веер разбегания: максимальное отклонение от вектора «прочь от демона»,
/// радианы (±≈34°). Толпа без него выстраивается в колонну.
const FLEE_SPREAD: f32 = 0.6;

/// Персональный угол веера: детерминирован по [`PawnId`], чтобы человек между
/// перепрокладками держал свою сторону, а не зигзагами метался.
///
/// Именно `PawnId`, а не `Entity`: индексы сущностей после рестарта
/// переиспользуются в другом порядке (свободный список зависит от того, кого
/// съели в прошлом прогоне), и веер разъехался бы между прогоном и его
/// повтором при абсолютно одинаковом seed.
fn personal_spread(pawn_id: u32) -> f32 {
    let hash = pawn_id.wrapping_mul(2654435761);
    (hash_fraction(hash) * 2.0 - 1.0) * FLEE_SPREAD
}

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
            .insert((HumanFleeTag, repath));
    }
    crate::diagnostics::measure_ms(&mut diagnostics, &crate::diagnostics::SIM_PANIC_MS, started);
}

/// Flee: бег от ближайшего демона с троттлингом перепрокладки;
/// демоны отстали (×1.5 радиуса) — успокаивается.
///
/// Точный поиск ближайшего демона — только на тиках решения (сработал таймер
/// перепрокладки или потерян путь), то есть раз в 45–77 тиков на бегущего. На
/// остальных тиках вместо него — проверка занятости ячеек демонской сетки,
/// накрывающих радиус ([`SpatialGrid::any_in_cells_around`]): пустое окно
/// гарантирует «в радиусе никого», и успокоение срабатывает как раньше;
/// занятое окно у уже безопасного бегущего (демон завис в 90–150 м)
/// откладывает успокоение до его тика решения — не дольше периода
/// перепрокладки. До этого точный поиск бежал каждый тик у каждого бегущего
/// и стоил 40% тика симуляции (0.42 мс при ~1900 бегущих: они толпятся
/// именно там, где демоны, и каждый заново обходил одни и те же плотные
/// ячейки, дёргая `Query::get` на каждого кандидата).
///
/// Курс (`WanderHeading`) переписывается вектором бегства на каждой
/// перепрокладке: в ветке успокоения демона в радиусе уже нет — она и
/// срабатывает потому, что поиск никого не нашёл, — так что направление
/// угрозы надо запомнить заранее. Устаревание ограничено: период
/// перепрокладки 0.7–1.2 с при скорости бегства 8 м/с — не больше 9.6 м
/// пройденного пути против разрыва в 90 м, то есть ≲13° ошибки, что заведомо
/// внутри ±45° запретного конуса.
#[allow(clippy::too_many_arguments)]
pub fn flee(
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    time: Res<Time>,
    pathfinder: Pathfinder,
    demons: Res<SpatialGrid<Demon>>,
    demon_positions: Query<&SimPosition, With<Demon>>,
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
    let navmesh = pathfinder.navmesh.read();
    // за кем прямо сейчас гонятся — те бегут по чистому вектору от демона
    let chased: bevy::platform::collections::HashSet<Entity> =
        chasing.iter().map(|chase_target| chase_target.0).collect();

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
        let needs_path = matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        );
        // тик без решения — у подавляющего большинства бегущих: хватает
        // грубой проверки занятости ячеек (см. док-комментарий системы)
        if !repath.0.just_finished() && !needs_path {
            if !demons.any_in_cells_around(sim_position.0, HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS) {
                calm_down(
                    entity,
                    &mut commands,
                    &style,
                    seed.0,
                    &mut movable,
                    &mut pause,
                    &heading,
                    pace,
                    pawn_id,
                    &mut wander_index,
                );
            }
            continue;
        }

        // поток заводится в каждой из двух точек решения отдельно, а не разом
        // на итерацию: `next` сдвигает номер решения, и заведённый заранее
        // поток крутил бы счётчик каждый тик у каждого бегущего — в том числе
        // на тиках, где решения нет вовсе (таймер перепрокладки не сработал)
        let Some((_, demon_position)) = demons.nearest_in_range(
            sim_position.0,
            HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS,
            |d| demon_positions.get(d).ok().map(|p| p.0),
        ) else {
            // демоны далеко — мирный режим, отдышаться перед новой прогулкой
            calm_down(
                entity,
                &mut commands,
                &style,
                seed.0,
                &mut movable,
                &mut pause,
                &heading,
                pace,
                pawn_id,
                &mut wander_index,
            );
            continue;
        };

        let mut away = (sim_position.0 - demon_position).normalize_or(Vec2::X);
        // не преследуемые разбегаются веером — каждый под своим углом
        if !chased.contains(&entity) {
            away = Vec2::from_angle(personal_spread(pawn_id.0)).rotate(away);
        }
        // память о направлении угрозы — пишется до отсева непроходимой цели,
        // иначе неудачный кадр оставил бы курс от прошлой перепрокладки
        heading.0 = away;
        let step = wander_index
            .next(seed.0, crate::rng::RngDomain::Human, pawn_id.0)
            .random_range(FLEE_STEP.0..FLEE_STEP.1);
        // не клампим к «безопасной» зоне: цель у самой границы — путь к спасению
        let target = (sim_position.0 + away * step).clamp(Vec2::splat(1.0), MAP_SIZE - 1.0);

        let Some(target_tile) = find_passable_tile_near(&navmesh, world_to_tile(target)) else {
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

/// Ветка успокоения [`flee`]: демоны отстали — мирный шаг, пауза
/// «отдышаться» перед новой прогулкой и возврат в Wander. Вынесена, потому
/// что вызывается из двух мест: по пустому окну ячеек на любом тике и по
/// точному поиску, никого не нашедшему, на тике решения.
#[allow(clippy::too_many_arguments)]
fn calm_down(
    entity: Entity,
    commands: &mut Commands,
    style: &HumanStyle,
    seed: u64,
    movable: &mut Movable,
    pause: &mut WanderPause,
    heading: &WanderHeading,
    pace: &Pace,
    pawn_id: &PawnId,
    wander_index: &mut WanderIndex,
) {
    movable.speed = pace.speed(HUMAN_WALK_SPEED, style.spread);
    movable.to_idle(entity, commands, false);
    pause.0.set_duration(std::time::Duration::from_secs_f32(
        wander_index
            .next(seed, crate::rng::RngDomain::Human, pawn_id.0)
            .random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1),
    ));
    pause.0.reset();
    // курс уже смотрит прочь от демона, обратный ему и есть центр
    // запретного конуса. Он отклонён веером разбегания (±0.6 рад ≈
    // 34°), то есть направление на самого демона всё равно внутри
    // ±45° — а читается это как «не возвращайся тем же путём»
    commands
        .entity(entity)
        .remove::<(HumanFleeTag, FleeRepath)>()
        .insert((HumanWanderTag, PanicRecoil(-heading.0)));
}

/// Паникующий пересёк границу карты — «спасся», despawn [Q12].
pub fn escape(
    mut commands: Commands,
    mut telemetry: ResMut<Telemetry>,
    query: Query<(Entity, &SimPosition), (With<Human>, With<HumanFleeTag>)>,
) {
    for (entity, sim_position) in &query {
        let pos = sim_position.0;
        if pos.x <= ESCAPE_MARGIN
            || pos.y <= ESCAPE_MARGIN
            || pos.x >= MAP_SIZE.x - ESCAPE_MARGIN
            || pos.y >= MAP_SIZE.y - ESCAPE_MARGIN
        {
            commands.entity(entity).despawn();
            telemetry.escaped += 1;
            debug!("human {entity} escaped (total {})", telemetry.escaped);
        }
    }
}
