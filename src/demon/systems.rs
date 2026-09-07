use std::time::Duration;

use bevy::prelude::*;
use rand::Rng;

use crate::demon::components::{
    ChaseTarget, Demon, DemonLungeTag, DemonSpawner, DemonStyle, DemonWanderTag,
};
use crate::demon::look::{demon_body, halo};
use crate::loading::AppState;
use crate::movement::{
    DrawMovePaths, MOVEPATH_ARROW_TIP, MOVEPATH_COLOR, Movable, SimPosition, point_in_cone,
    ready_to_pick, request_wander_path,
};
use crate::navigation::Backend;
use crate::portal::PortalPos;
use crate::rng::{PawnId, RngDomain, Species, WanderIndex, WorldSeed, decision_stream};
use crate::settings::{
    DEMON_INITIAL_BURST, DEMON_SPEED, DEMON_WANDER_CONE, DEMON_WANDER_RANGE, PORTAL_DIAMETER,
    unit_z,
};
use crate::silhouette::Silhouettes;

/// Стартовый залп; в `FixedUpdate`, а не в `Startup` — после рестарта сцены
/// сброшенный спавнер выпускает залп заново без отдельного кода.
pub fn spawn_initial_burst(
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<DemonStyle>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
    silhouettes: Res<Silhouettes>,
) {
    if spawner.initial_burst_done {
        return;
    }
    spawner.initial_burst_done = true;

    // залп тоже упирается в кап — иначе ползунок, выкрученный ниже восьми,
    // врал бы: демоны всё равно выходили бы залпом
    let burst = DEMON_INITIAL_BURST.min(style.cap);
    let birth = DemonBirth::new(&seed, &portal_pos, &style, &silhouettes);
    for index in 0..burst {
        let angle = index as f32 / burst as f32 * std::f32::consts::TAU;
        spawn_demon(&mut commands, &mut spawner, &birth, Some(angle));
    }
}

pub fn tick_spawner(
    time: Res<Time>,
    mut commands: Commands,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<DemonStyle>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
    silhouettes: Res<Silhouettes>,
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

    let birth = DemonBirth::new(&seed, &portal_pos, &style, &silhouettes);
    spawn_demon(&mut commands, &mut spawner, &birth, None);
}

/// Всё, что демон получает при рождении помимо номера и угла. Четыре ресурса
/// читаются одинаково в обеих системах спавна, поэтому ездят одним значением,
/// а не пятёркой позиционных аргументов.
struct DemonBirth<'a> {
    world_seed: u64,
    portal_pos: Vec2,
    speed: f32,
    silhouettes: &'a Silhouettes,
}

impl<'a> DemonBirth<'a> {
    fn new(
        seed: &WorldSeed,
        portal_pos: &PortalPos,
        style: &DemonStyle,
        silhouettes: &'a Silhouettes,
    ) -> Self {
        Self {
            world_seed: seed.0,
            portal_pos: portal_pos.0,
            speed: DEMON_SPEED * style.speed,
            silhouettes,
        }
    }
}

/// Демон появляется у кромки портала и с первого же тика идёт по своим делам.
///
/// Номер демон получает здесь: `spawned` читается до инкремента и становится
/// `PawnId`, инкремент лежит рядом с чтением — у вызывающих его можно забыть,
/// и тогда два демона делят номер, а на его уникальности стоят и поток ГПСЧ
/// пешки, и ключ очереди детерминированного диспетчера.
///
/// `angle: None` — угол выхода разыгрывается; залп задаёт его сам, рассаживая
/// демонов равномерно по кромке. Собственного потока у `DemonSpawner` нет
/// намеренно: угол тянется из потока **самого нового демона**, засеянного его
/// `index`, — поэтому демон номер N выходит одинаково, сколько бы демонов ни
/// успело родиться и умереть до него.
fn spawn_demon(
    commands: &mut Commands,
    spawner: &mut DemonSpawner,
    birth: &DemonBirth,
    angle: Option<f32>,
) {
    let index = spawner.spawned;
    spawner.spawned += 1;

    let mut rng = decision_stream(
        birth.world_seed,
        RngDomain::Demon,
        index as u32,
        WanderIndex::SPAWN,
    );
    let angle = angle.unwrap_or_else(|| rng.random_range(0.0..std::f32::consts::TAU));
    let position = birth.portal_pos + Vec2::from_angle(angle) * (PORTAL_DIAMETER / 2.0 + 1.0);

    let (sprite, silhouette) = demon_body(birth.silhouettes, index);
    commands
        .spawn((
            sprite,
            silhouette,
            Transform::from_translation(position.extend(unit_z(position.y))),
            Demon,
            DemonWanderTag,
            Movable::new(birth.speed),
            PawnId(index as u32),
            // номер уникален только внутри вида, поэтому вид едет рядом с ним
            Species::Demon,
            // демон срочен всегда: инвазия за кадром не должна вставать, и
            // снимать маркер с него нечему — в отличие от человека, у которого
            // он приходит и уходит вместе с паникой
            crate::movement::UrgentPath,
            // и тело своё демон тоже носит сам: явный компонент в кортеже
            // спавна перебивает умолчание `#[require]` у `Movable`
            crate::movement::BodyScale::DEMON,
            WanderIndex::ready(),
            DespawnOnExit(AppState::Playing),
            Name::new("demon"),
        ))
        // ореол — дочерняя сущность: уходит вместе с демоном (despawn
        // рекурсивен), пульсирует его масштабом и не нуждается в своём
        // `DespawnOnExit`
        .with_child(halo(birth.silhouettes));
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
    backend: Res<Backend>,
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
    let walkable = backend.walkable();

    for (entity, sim_position, mut movable, pawn_id, mut wander_index) in &mut query {
        if !ready_to_pick(&movable.state) {
            continue;
        }

        // после отсева, а не до: `next` сдвигает номер решения, и заведённый
        // заранее поток крутил бы его вхолостую на каждом кадре
        let rng = &mut wander_index.next(seed.0, RngDomain::Demon, pawn_id.0);

        // «от портала» — вся видовая политика демона и есть; остальное общее
        let away = (sim_position.0 - portal_pos.0).normalize_or(Vec2::from_angle(
            rng.random_range(0.0..std::f32::consts::TAU),
        ));
        let target = point_in_cone(
            rng,
            sim_position.0,
            away,
            DEMON_WANDER_CONE,
            DEMON_WANDER_RANGE,
        );

        request_wander_path(
            &mut commands,
            &walkable,
            entity,
            &mut movable,
            sim_position.0,
            target,
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
