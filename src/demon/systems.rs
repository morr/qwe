use std::time::Duration;

use bevy::prelude::*;
use rand::Rng;

use crate::demon::components::{
    ChaseTarget, Demon, DemonLungeTag, DemonSpawner, DemonStyle, DemonWanderTag,
};
use crate::grid::world_to_tile;
use crate::loading::AppState;
use crate::movement::{
    DrawMovePaths, MOVEPATH_ARROW_TIP, MOVEPATH_COLOR, Movable, MovableState, SimPosition,
};
use crate::navigation::{Pathfinder, find_passable_tile_near};
use crate::portal::PortalPos;
use crate::rng::{PawnId, RngDomain, WanderIndex, WorldSeed, decision_stream};
use crate::settings::{
    DEMON_INITIAL_BURST, DEMON_SIZE, DEMON_SPEED, MAP_SIZE, PORTAL_DIAMETER, unit_z,
};

/// Блуждание: дистанция до следующей случайной точки, м.
const WANDER_DISTANCE: (f32, f32) = (40.0, 120.0);
/// Разброс направления «от портала», радианы.
const WANDER_SPREAD: f32 = 1.3;
/// Отступ целей блуждания от края карты, м.
const MAP_MARGIN: f32 = 4.0;

/// Стартовый залп; в `FixedUpdate`, а не в `Startup` — после рестарта сцены
/// сброшенный спавнер выпускает залп заново без отдельного кода.
pub fn spawn_initial_burst(
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<DemonStyle>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
) {
    if spawner.initial_burst_done {
        return;
    }
    spawner.initial_burst_done = true;

    // залп тоже упирается в кап — иначе ползунок, выкрученный ниже восьми,
    // врал бы: демоны всё равно выходили бы залпом
    let burst = DEMON_INITIAL_BURST.min(style.cap);
    for index in 0..burst {
        let angle = index as f32 / burst as f32 * std::f32::consts::TAU;
        spawn_demon(
            &mut commands,
            seed.0,
            portal_pos.0,
            Some(angle),
            spawner.spawned,
            DEMON_SPEED * style.speed,
        );
        spawner.spawned += 1;
    }
}

pub fn tick_spawner(
    time: Res<Time>,
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<DemonStyle>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
) {
    // период таймера подтягивается здесь, а не отдельной системой на
    // `resource_changed`: рестарт и смена города пересоздают `DemonSpawner`
    // целиком (`restart.rs`, `city.rs`), и таймер вернулся бы к константе,
    // тогда как ресурс с тех пор не менялся — чинить было бы некому
    let interval = Duration::from_secs_f32(style.interval);
    if spawner.timer.duration() != interval {
        spawner.timer.set_duration(interval);
    }

    if spawner.spawned >= style.cap {
        return;
    }
    spawner.timer.tick(time.delta());
    if !spawner.timer.just_finished() {
        return;
    }

    spawn_demon(
        &mut commands,
        seed.0,
        portal_pos.0,
        None,
        spawner.spawned,
        DEMON_SPEED * style.speed,
    );
    spawner.spawned += 1;
}

/// Демон появляется у кромки портала и с первого же тика идёт по своим делам.
///
/// `angle: None` — угол выхода разыгрывается; залп задаёт его сам, рассаживая
/// демонов равномерно по кромке. Собственного потока у `DemonSpawner` нет
/// намеренно: угол тянется из потока **самого нового демона**, засеянного его
/// `index`, — поэтому демон номер N выходит одинаково, сколько бы демонов ни
/// успело родиться и умереть до него.
fn spawn_demon(
    commands: &mut Commands,
    world_seed: u64,
    portal_pos: Vec2,
    angle: Option<f32>,
    index: usize,
    speed: f32,
) {
    let mut rng = decision_stream(
        world_seed,
        RngDomain::Demon,
        index as u32,
        WanderIndex::SPAWN,
    );
    let angle = angle.unwrap_or_else(|| rng.random_range(0.0..std::f32::consts::TAU));
    let position = portal_pos + Vec2::from_angle(angle) * (PORTAL_DIAMETER / 2.0 + 1.0);

    // оттенки красного, чтобы демоны не сливались друг с другом
    let tint = 0.45 + (index % 5) as f32 * 0.08;
    commands.spawn((
        Sprite {
            color: Color::srgb(tint, 0.06, 0.10),
            custom_size: Some(Vec2::splat(DEMON_SIZE)),
            ..default()
        },
        Transform::from_translation(position.extend(unit_z(position.y))),
        Demon,
        DemonWanderTag,
        Movable::new(speed),
        PawnId(index as u32),
        WanderIndex::ready(),
        DespawnOnExit(AppState::Playing),
        Name::new("demon"),
    ));
}

/// Ползунок скорости — уже вышедшим демонам. `Movable::speed` пишется один раз,
/// при спавне, поэтому без этой системы новая скорость доставалась бы только
/// следующим демонам из портала, а сотня уже гуляющих осталась бы на старой.
/// Гоняется по `resource_changed::<DemonStyle>`, то есть на движение ползунка,
/// а не покадрово.
pub fn sync_demon_speed(style: Res<DemonStyle>, mut demons: Query<&mut Movable, With<Demon>>) {
    let speed = DEMON_SPEED * style.speed;
    for mut movable in &mut demons {
        movable.speed = speed;
    }
}

/// Блуждающий демон без пути выбирает случайную проходимую точку «от портала»
/// и запрашивает путь. У края карты направление естественно заворачивает
/// внутрь из-за клампа цели в границы.
pub fn pick_wander_targets(
    mut commands: Commands,
    pathfinder: Pathfinder,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            &PawnId,
            &mut WanderIndex,
        ),
        (
            With<Demon>,
            With<DemonWanderTag>,
            With<crate::movement::NeedsWanderTarget>,
        ),
    >,
) {
    let navmesh = pathfinder.navmesh.read();

    for (entity, sim_position, mut movable, pawn_id, mut wander_index) in &mut query {
        if !matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        ) {
            continue;
        }

        // после отсева, а не до: `next` сдвигает номер решения, и заведённый
        // заранее поток крутил бы его вхолостую на каждом кадре
        let rng = &mut wander_index.next(seed.0, RngDomain::Demon, pawn_id.0);

        let away = (sim_position.0 - portal_pos.0).normalize_or(Vec2::from_angle(
            rng.random_range(0.0..std::f32::consts::TAU),
        ));
        let direction =
            Vec2::from_angle(rng.random_range(-WANDER_SPREAD..WANDER_SPREAD)).rotate(away);
        let distance = rng.random_range(WANDER_DISTANCE.0..WANDER_DISTANCE.1);
        let target = (sim_position.0 + direction * distance)
            .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN);

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
}

/// Movepath броска: в финальной фазе тайловый путь снят (демона ведёт
/// `chase`, а не `move_moving_entities`), и обычная отрисовка путей такого
/// демона не видит. Рисуем стрелку напрямую в текущую позицию жертвы.
pub fn draw_lunge_paths(
    draw: Res<DrawMovePaths>,
    mut gizmos: Gizmos,
    lunging: Query<(&SimPosition, &ChaseTarget), (With<Demon>, With<DemonLungeTag>)>,
    targets: Query<&SimPosition>,
) {
    if !draw.0 {
        return;
    }

    // отсечения по экрану, как у `draw_move_paths`, здесь нет: бросок — это
    // единицы демонов на всю карту, не десятки тысяч линий
    for (sim_position, chase_target) in &lunging {
        let Ok(target_position) = targets.get(chase_target.0) else {
            continue;
        };
        let distance = sim_position.0.distance(target_position.0);
        gizmos
            .arrow_2d(sim_position.0, target_position.0, MOVEPATH_COLOR)
            .with_tip_length(MOVEPATH_ARROW_TIP.min(distance * 0.5));
    }
}
