//! Осада: применение лестницы Громилы (`decide_brute.rs`). Фронт считается
//! раз на тик из районов, скверны и стоящих бастионов; каждый Громила без цели
//! берёт ближайший бастион фронта со свободным местом (`BastionClaims`),
//! идёт к его точке, в досягаемости встаёт — бьёт `combat::strike`, хвост
//! той же цепочки.

use bevy::prelude::*;

use crate::bastion::{Bastion, RuinTag};
use crate::combat::AttackTarget;
use crate::corruption::Corruption;
use crate::demon::behavior::repath_towards;
use crate::demon::claims::BastionClaims;
use crate::demon::components::{BruteTag, ChaseRepath, Demon, DemonWanderTag};
use crate::demon::decide::PathSense;
use crate::demon::decide_brute::{BruteAction, BruteSense, BruteTarget, Front, decide};
use crate::district::Districts;
use crate::movement::{Movable, MovableState, PathfindingRequest, PathfindingTask, SimPosition};
use crate::navigation::Backend;

/// Бастион фронта: стоит, его район не осквернён, а хотя бы один сосед — да.
struct FrontBastion {
    entity: Entity,
    position: Vec2,
    /// Номер места — для ничьей по дистанции: `Entity` в порядке не участвует
    /// (`movement/order.rs`), а места на одной карте раздаются одинаково.
    site: u16,
}

/// Осада на тике: для каждого Громилы — лестница и её применение. Читает
/// районы, скверну и бастионы, пишет цель, теги и заявку на путь; `SimPosition`
/// не трогает — в отличие от погони, у Громилы броска нет.
#[allow(clippy::too_many_arguments)]
pub fn besiege(
    mut commands: Commands,
    time: Res<Time>,
    backend: Res<Backend>,
    districts: Res<Districts>,
    corruption: Res<Corruption>,
    bastions: Query<(Entity, &Bastion, &Transform, Has<RuinTag>)>,
    mut brutes: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            Option<&AttackTarget>,
            Option<&mut ChaseRepath>,
            Has<PathfindingTask>,
            Has<PathfindingRequest>,
        ),
        (With<Demon>, With<BruteTag>),
    >,
) {
    // фронт — один список на тик: десятки бастионов, сотни Громил читают его
    let front: Vec<FrontBastion> = bastions
        .iter()
        .filter(|(_, bastion, _, ruined)| {
            !ruined && corruption.on_front(&districts, bastion.district)
        })
        .map(|(entity, bastion, transform, _)| FrontBastion {
            entity,
            position: transform.translation.truncate(),
            site: bastion.site,
        })
        .collect();
    let mut claims = BastionClaims::of(
        brutes
            .iter()
            .filter_map(|(_, _, _, target, ..)| target.map(|target| target.0)),
    );
    let walkable = backend.walkable();

    for (entity, sim_position, mut movable, target, mut repath, has_task, has_request) in
        &mut brutes
    {
        let target_sense = target.and_then(|target| {
            bastions
                .get(target.0)
                .ok()
                .map(|(_, _, transform, ruined)| BruteTarget {
                    position: transform.translation.truncate(),
                    ruined,
                })
        });
        let sense = BruteSense {
            position: sim_position.0,
            target: target_sense,
            path: PathSense::of(
                &movable,
                has_task || has_request,
                repath.as_ref().map(|repath| &repath.0),
                time.delta(),
            ),
        };
        // цель исчезла вовсе (сущности нет) — для лестницы «цели нет», для
        // применения — компонент, который пора снять
        let action = if target.is_some() && target_sense.is_none() {
            BruteAction::Done
        } else {
            decide(&sense, || {
                front
                    .iter()
                    .filter(|bastion| claims.has_room(bastion.entity))
                    .min_by(|a, b| {
                        let da = sim_position.0.distance_squared(a.position);
                        let db = sim_position.0.distance_squared(b.position);
                        da.total_cmp(&db).then(a.site.cmp(&b.site))
                    })
                    .map(|bastion| Front {
                        entity: bastion.entity,
                        position: bastion.position,
                    })
            })
        };

        let target_pos = match action {
            BruteAction::Done => {
                commands
                    .entity(entity)
                    .remove::<(AttackTarget, ChaseRepath)>()
                    .insert(DemonWanderTag);
                debug!("brute {entity} target done => Wander");
                continue;
            }
            BruteAction::Strike => {
                // дошёл: путь больше не нужен, бьёт с места
                if !matches!(movable.state, MovableState::Idle) {
                    movable.to_idle(entity, &mut commands, false);
                }
                continue;
            }
            BruteAction::WaitForPath | BruteAction::Wander => continue,
            BruteAction::Hold => {
                if let Some(repath) = repath.as_mut() {
                    repath.0.tick(time.delta());
                }
                continue;
            }
            BruteAction::Repath { target } => {
                if let Some(repath) = repath.as_mut() {
                    repath.0.tick(time.delta());
                }
                target
            }
            BruteAction::Engage { to } => {
                claims.claim(to.entity);
                commands
                    .entity(entity)
                    .remove::<DemonWanderTag>()
                    .insert((AttackTarget(to.entity), ChaseRepath::default()));
                debug!("brute {entity} Wander => besiege {}", to.entity);
                to.position
            }
        };

        repath_towards(
            &mut commands,
            &walkable,
            entity,
            &mut movable,
            sim_position.0,
            target_pos,
        );
    }
}
