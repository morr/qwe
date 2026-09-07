//! Бой M1: одна сторона бьёт, другая теряет здоровье. Здесь — здоровье и
//! событие гибели; кто именно погиб и что с ним делать, решает владелец
//! сущности своим обсервером (`bastion::on_destroyed`). Удар (`Attack`,
//! `strike`) — следующий шаг `ROADMAP.md`.

use bevy::prelude::*;

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

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Health>();
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
}
