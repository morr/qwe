use std::time::Duration;

use bevy::prelude::*;
use rand::RngExt;

use crate::demon::components::{
    ChaseTarget, Demon, DemonLungeTag, DemonSpawner, DemonStyle, DemonWanderTag, WarpOrigin,
};
use crate::demon::horde::HordeSchedule;
use crate::demon::look::{demon_body, halo};
use crate::diagnostics::{SIM_SPAWNER_MS, SimMeasure};
use crate::loading::AppState;
use crate::map::z::unit_z;
use crate::movement::{
    DrawMovePaths, MOVEPATH_ARROW_TIP, MOVEPATH_COLOR, Movable, SimPosition, assert_ready_to_pick,
    point_in_cone, request_wander_path,
};
use crate::navigation::Backend;
use crate::portal::PortalPos;
use crate::rng::{PawnId, RngDomain, Species, WanderIndex, WorldSeed, decision_stream};
use crate::settings::{
    DEMON_INITIAL_BURST, DEMON_SPEED, DEMON_WANDER_CONE, DEMON_WANDER_RANGE, PORTAL_DIAMETER,
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
    let burst = DEMON_INITIAL_BURST.min(style.cap());
    let birth = DemonBirth::new(&seed, &portal_pos, &style, &silhouettes);
    for index in 0..burst {
        let angle = index as f32 / burst as f32 * std::f32::consts::TAU;
        spawn_demon(
            &mut commands,
            &mut spawner,
            &birth,
            DemonArrival::Portal(Some(angle)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn tick_spawner(
    time: Res<Time>,
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<DemonStyle>,
    horde: Res<HordeSchedule>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
    silhouettes: Res<Silhouettes>,
) {
    let _measure = SimMeasure::new(&mut diagnostics, &SIM_SPAWNER_MS);
    // период таймера подтягивается здесь, а не отдельной системой на
    // `resource_changed`: рестарт и смена города пересоздают `DemonSpawner`
    // целиком (`restart.rs`, `city.rs`), и таймер вернулся бы к константе,
    // тогда как ресурс с тех пор не менялся — чинить было бы некому
    let interval = Duration::from_secs_f32(style.interval());
    if spawner.timer.duration() != interval {
        spawner.timer.set_duration(interval);
    }

    // кап общий на все источники: очередь невыпущенной волны — тоже
    // обязательство, и фон, не считающий её, переполнял бы кап на время
    // выпуска (координаты волны посчитаны под свободное место до её постановки)
    if spawner.spawned + horde.pending() >= style.cap() {
        return;
    }
    spawner.timer.tick(time.delta());
    if !spawner.timer.just_finished() {
        return;
    }

    let birth = DemonBirth::new(&seed, &portal_pos, &style, &silhouettes);
    spawn_demon(
        &mut commands,
        &mut spawner,
        &birth,
        DemonArrival::Portal(None),
    );
}

/// База скорости демона на этом прогоне: единственная формула и для скорости
/// при рождении, и для `sync_demon_speed` (ползунок), — две копии выражения
/// разъехались бы молча: свежерождённый демон побежал бы с другой базой,
/// чем вся стая.
fn demon_speed(style: &DemonStyle) -> f32 {
    DEMON_SPEED * style.speed
}

/// Всё, что демон получает при рождении помимо номера и угла. Четыре ресурса
/// читаются одинаково в обеих системах спавна, поэтому ездят одним значением,
/// а не пятёркой позиционных аргументов.
pub(super) struct DemonBirth<'a> {
    world_seed: u64,
    portal_pos: Vec2,
    speed: f32,
    silhouettes: &'a Silhouettes,
}

impl<'a> DemonBirth<'a> {
    pub(super) fn new(
        seed: &WorldSeed,
        portal_pos: &PortalPos,
        style: &DemonStyle,
        silhouettes: &'a Silhouettes,
    ) -> Self {
        Self {
            world_seed: seed.0,
            portal_pos: portal_pos.0,
            speed: demon_speed(style),
            silhouettes,
        }
    }
}

/// Откуда демон выходит: у кромки портала или из волны орды.
pub enum DemonArrival {
    /// Кромка портала; `None` — угол выхода разыгрывается из потока самого
    /// нового демона (фон), залп задаёт угол сам, рассаживая демонов по кромке.
    Portal(Option<f32>),
    /// Волна: точка выпуска и очаг уже посчитаны расписанием орды из её
    /// чистого потока (`demon::horde`), здесь только применяются.
    Wave { position: Vec2, origin: Vec2 },
}

/// Демон появляется в точке своего прибытия и с первого же тика идёт по своим
/// делам.
///
/// Номер демон получает здесь: `spawned` читается до инкремента и становится
/// `PawnId`, инкремент лежит рядом с чтением — у вызывающих его можно забыть,
/// и тогда два демона делят номер, а на его уникальности стоят и поток ГПСЧ
/// пешки, и ключ очереди детерминированного диспетчера.
///
/// `angle: None` у порталового прибытия — угол разыгрывается из потока
/// **самого нового демона**, засеянного его `index`, — поэтому демон номер N
/// выходит одинаково, сколько бы демонов ни успело родиться и умереть до него.
/// Волновое прибытие жребия рождения не тратит вовсе: вся его случайность
/// посчитана расписанием волны, а собственный поток демона начинается с
/// решения номер 1, как и у порталового (разница в расходе номера 0 на
/// решение не влияет — см. пин эквивалентности в `demon::horde`).
pub fn spawn_demon(
    commands: &mut Commands,
    spawner: &mut DemonSpawner,
    birth: &DemonBirth,
    arrival: DemonArrival,
) {
    let index = spawner.spawned;
    spawner.spawned += 1;

    let (position, origin) = match arrival {
        DemonArrival::Portal(angle) => {
            let angle = match angle {
                Some(angle) => angle,
                None => decision_stream(
                    birth.world_seed,
                    RngDomain::Demon,
                    index as u32,
                    WanderIndex::SPAWN,
                )
                .random_range(0.0..std::f32::consts::TAU),
            };
            (
                birth.portal_pos + Vec2::from_angle(angle) * (PORTAL_DIAMETER / 2.0 + 1.0),
                birth.portal_pos,
            )
        }
        DemonArrival::Wave { position, origin } => (position, origin),
    };

    let (sprite, silhouette) = demon_body(birth.silhouettes, index);
    commands
        .spawn((
            sprite,
            silhouette,
            Transform::from_translation(position.extend(unit_z(position.y))),
            Demon,
            WarpOrigin(origin),
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

/// Гейт [`sync_demon_speed`]: только поле `speed`, а не весь ресурс —
/// `DemonStyle` несёт ещё потолок, интервал и надбавку броска, и гейт на
/// целом ресурсе переписывал бы скорость всей стае на каждый шаг чужого
/// ползунка. `Local`-сравнение, а не `resource_changed`: тот же приём, что у
/// человеческого `spread_changed` (`human/systems.rs`).
pub fn demon_speed_changed(style: Res<DemonStyle>, mut applied: Local<Option<f32>>) -> bool {
    let current = style.speed;
    let changed = *applied != Some(current);
    *applied = Some(current);
    changed
}

/// Ползунок скорости — уже вышедшим демонам. `Movable::speed` пишется один раз,
/// при спавне, поэтому без этой системы новая скорость доставалась бы только
/// следующим демонам из портала, а сотня уже гуляющих осталась бы на старой.
/// Гоняется по [`demon_speed_changed`], то есть на движение ползунка скорости,
/// а не покадрово.
pub fn sync_demon_speed(style: Res<DemonStyle>, mut demons: Query<&mut Movable, With<Demon>>) {
    let speed = demon_speed(&style);
    for mut movable in &mut demons {
        movable.speed = speed;
    }
}

/// Блуждающий демон без пути выбирает случайную проходимую точку «от своего
/// происхождения» ([`WarpOrigin`]: портал — у born-у-портала, очаг волны — у
/// волнового) и запрашивает путь. У края карты направление естественно
/// заворачивает внутрь из-за клампа цели в границы.
pub fn pick_wander_targets(
    mut commands: Commands,
    backend: Res<Backend>,
    seed: Res<WorldSeed>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &WarpOrigin,
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

    for (entity, sim_position, warp_origin, mut movable, pawn_id, mut wander_index) in &mut query {
        assert_ready_to_pick(&movable.state);

        // после отсева, а не до: `next` сдвигает номер решения, и заведённый
        // заранее поток крутил бы его вхолостую на каждом кадре
        let rng = &mut wander_index.next(seed.0, RngDomain::Demon, pawn_id.0);

        // «от происхождения» — вся видовая политика демона и есть; остальное
        // общее. У рождённого порталом происхождение — сам портал, и вектор
        // совпадает бит-в-бит с прежним «от портала» (пин эквивалентности —
        // `demon::horde`)
        let away = (sim_position.0 - warp_origin.0).normalize_or(Vec2::from_angle(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demon::components::{DEMON_SPEED_FACTOR_MAX, DEMON_SPEED_FACTOR_MIN};

    /// Мир одного демона: гейту хватает ресурса, системе — `Movable`.
    fn speed_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<DemonStyle>()
            .add_systems(Update, sync_demon_speed.run_if(demon_speed_changed));

        let entity = app.world_mut().spawn((Demon, Movable::new(0.0))).id();

        // первый прогон: Local = None → система отработает раз
        app.update();

        (app, entity)
    }

    /// После прогрева чужой ползунок (`cap`) не должен менять скорости стаи —
    /// часовой на `Movable::speed`, ровно как человеческий
    /// `the_body_radius_slider_leaves_the_crowd_alone`: при неизменном `speed`
    /// система переписала бы ровно то же число, и «лишний проход» иначе
    /// неотличим от его отсутствия.
    #[test]
    fn the_cap_slider_leaves_the_pack_alone() {
        let (mut app, entity) = speed_app();
        {
            // `entity_mut` держится в переменной: `Mut<Movable>` заимствует
            // именно его, и временное значение умерло бы раньше заимствования
            let mut entity_mut = app.world_mut().entity_mut(entity);
            let mut movable = entity_mut.get_mut::<Movable>().unwrap();
            movable.speed = -1.0; // часовой
        }

        app.world_mut().resource_mut::<DemonStyle>().cap = 12;
        app.update();

        // если система запустилась, -1.0 был бы перезаписан
        assert_eq!(
            app.world().entity(entity).get::<Movable>().unwrap().speed,
            -1.0
        );
    }

    /// Смена поля `speed` должна гонять систему.
    #[test]
    fn moving_the_speed_slider_retunes_the_pack() {
        let (mut app, entity) = speed_app();
        let factor = DEMON_SPEED_FACTOR_MIN + DEMON_SPEED_FACTOR_MAX;
        app.world_mut().resource_mut::<DemonStyle>().speed = factor;
        app.update();

        let expected = DEMON_SPEED * factor;
        assert_eq!(
            app.world().entity(entity).get::<Movable>().unwrap().speed,
            expected
        );
    }
}
