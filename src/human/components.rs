use bevy::prelude::*;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Human;

/// Стейт-машина человека: Wander / Flee — эксклюзивные теги.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HumanWanderTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HumanFleeTag;

/// Пауза между прогулками; тикает, пока человек стоит.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct WanderPause(pub Timer);

impl Default for WanderPause {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0, TimerMode::Once))
    }
}

/// Труп: остаётся навсегда, в поведении и сетках не участвует.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct CorpseTag;

/// Троттлинг перепрокладки пути при бегстве.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FleeRepath(pub Timer);

impl Default for FleeRepath {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0, TimerMode::Repeating))
    }
}
