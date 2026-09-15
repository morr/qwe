//! Бой M1: одна сторона бьёт, другая теряет здоровье. Здесь — здоровье,
//! удар и событие гибели; кто именно погиб и что с ним делать, решает
//! владелец сущности своим обсервером (`bastion::on_destroyed`), а кто кого
//! бьёт — лестница атакующего (`AttackTarget` ставит Громила).

use std::time::Duration;

use bevy::prelude::*;

use crate::movement::SimPosition;
use crate::settings::ATTACK_REACH;

/// Здоровье: `hp` от `max` вниз до нуля. Обе стороны с HP — M2, в M1 его
/// носят только бастионы.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
}

impl Health {
    pub fn full(max: f32) -> Self {
        Self { hp: max, max }
    }

    /// Снять `amount`; `true` — этим ударом добито (ровно один раз: по уже
    /// нулевому здоровью удар не «добивает» повторно).
    pub fn damage(&mut self, amount: f32) -> bool {
        if self.hp <= 0.0 {
            return false;
        }
        self.hp = (self.hp - amount).max(0.0);
        self.hp <= 0.0
    }

    pub fn heal(&mut self) {
        self.hp = self.max;
    }

    pub fn is_destroyed(&self) -> bool {
        self.hp <= 0.0
    }
}

/// Сущность с [`Health`] дошла до нуля. Кто она, знает её владелец: бастион
/// становится руиной в `bastion::on_destroyed`.
#[derive(Event, Debug, Clone, Copy)]
pub struct Destroyed {
    pub entity: Entity,
}

/// Удар: сколько снимает. Период живёт в одном месте — длительности таймера
/// [`AttackCooldown`]. Урон фиксированный — ГПСЧ в бою M1 нет, детерминизм не
/// тронут.
#[derive(Component, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct Attack {
    pub damage: f32,
}

/// Перезарядка удара. Тикается `Res<Time>` внутри `FixedUpdate` — то есть
/// `Time<Fixed>`, шагом тика (правило `CLAUDE.md` про таймеры).
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct AttackCooldown(pub Timer);

impl AttackCooldown {
    /// Готовый к первому удару сразу: таймер уже дотикан.
    pub fn ready(period: f32) -> Self {
        let mut timer = Timer::from_seconds(period, TimerMode::Once);
        timer.tick(Duration::from_secs_f32(period));
        Self(timer)
    }
}

/// Кого бьёт. Ставит и снимает лестница атакующего; компонент есть ⇔ есть
/// цель, битого `Entity` «по умолчанию» не бывает.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct AttackTarget(pub Entity);

/// Вердикт удара на тике.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    /// Цель уже добита — бить некого, лестница увидит это на следующем тике.
    Finished,
    OutOfReach,
    Cooling,
    Hit,
}

/// Чистое правило удара: цель жива, в [`ATTACK_REACH`], перезарядка вышла.
pub fn strike_verdict(distance: f32, cooldown_ready: bool, target_alive: bool) -> Strike {
    if !target_alive {
        Strike::Finished
    } else if distance > ATTACK_REACH {
        Strike::OutOfReach
    } else if !cooldown_ready {
        Strike::Cooling
    } else {
        Strike::Hit
    }
}

/// Удар на тике: каждому атакующему с целью — тик перезарядки и, если цель
/// в досягаемости и перезарядка вышла, `hp −= damage`; добитая цель
/// объявляется [`Destroyed`]. Цель здесь неподвижна (бастион), её позиция —
/// `Transform`; исчезнувшая цель просто пропускается, снять `AttackTarget` —
/// дело лестницы.
pub fn strike(
    time: Res<Time>,
    mut commands: Commands,
    mut attackers: Query<(&Attack, &mut AttackCooldown, &AttackTarget, &SimPosition)>,
    mut targets: Query<(&Transform, &mut Health)>,
) {
    for (attack, mut cooldown, target, position) in &mut attackers {
        cooldown.0.tick(time.delta());
        let Ok((transform, mut health)) = targets.get_mut(target.0) else {
            continue;
        };
        let distance = position.0.distance(transform.translation.truncate());
        if strike_verdict(distance, cooldown.0.is_finished(), !health.is_destroyed()) != Strike::Hit
        {
            continue;
        }
        cooldown.0.reset();
        if health.damage(attack.damage) {
            commands.trigger(Destroyed { entity: target.0 });
        }
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Health>()
            .register_type::<Attack>()
            .register_type::<AttackCooldown>()
            .register_type::<AttackTarget>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_finishes_exactly_once() {
        let mut health = Health::full(10.0);
        assert!(!health.damage(4.0));
        assert!(health.damage(6.0));
        assert!(health.is_destroyed());
        assert!(
            !health.damage(1.0),
            "a second blow on zero must not finish again"
        );
        health.heal();
        assert_eq!(health, Health::full(10.0));
    }

    #[test]
    fn strike_verdict_by_distance_cooldown_and_life() {
        assert_eq!(
            strike_verdict(ATTACK_REACH + 0.1, true, true),
            Strike::OutOfReach
        );
        assert_eq!(strike_verdict(ATTACK_REACH, false, true), Strike::Cooling);
        assert_eq!(strike_verdict(ATTACK_REACH, true, true), Strike::Hit);
        assert_eq!(strike_verdict(0.0, true, false), Strike::Finished);
    }

    /// Счётчик объявлений гибели — то, что видит владелец цели.
    #[derive(Resource, Default)]
    struct DestroyedLog(Vec<Entity>);

    /// Два удара по цели с запасом на два удара: первый ранит, второй добивает
    /// и объявляет гибель ровно раз, третий тик по добитой — ничего.
    #[test]
    fn strikes_land_on_the_cooldown_and_finish_the_target_once() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.init_resource::<DestroyedLog>();
        world.add_observer(|event: On<Destroyed>, mut log: ResMut<DestroyedLog>| {
            log.0.push(event.entity);
        });
        // `Res<Time>` — обобщённые часы; здесь они тикают ровно на секунду за
        // прогон системы
        let mut time: Time = Time::default();
        time.advance_by(Duration::from_secs(1));
        world.insert_resource(time);

        let target = world
            .spawn((Transform::from_xyz(10.0, 0.0, 0.0), Health::full(10.0)))
            .id();
        world.spawn((
            Attack { damage: 6.0 },
            AttackCooldown::ready(1.0),
            AttackTarget(target),
            SimPosition(Vec2::new(10.0 + ATTACK_REACH, 0.0)),
        ));

        world.run_system_once(strike).unwrap();
        world.flush();
        assert_eq!(world.entity(target).get::<Health>().unwrap().hp, 4.0);
        assert!(world.resource::<DestroyedLog>().0.is_empty());

        world.run_system_once(strike).unwrap();
        world.flush();
        assert_eq!(world.entity(target).get::<Health>().unwrap().hp, 0.0);
        assert_eq!(world.resource::<DestroyedLog>().0, vec![target]);

        world.run_system_once(strike).unwrap();
        world.flush();
        assert_eq!(world.resource::<DestroyedLog>().0, vec![target]);
    }
}
