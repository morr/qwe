use bevy::prelude::*;

use crate::settings::DEMON_SPAWN_INTERVAL;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Demon;

/// Стейт-машина демона: Wander / Chase / Devour — эксклюзивные теги.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonWanderTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonChaseTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonDevourTag;

/// Цель погони.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ChaseTarget(pub Entity);

impl Default for ChaseTarget {
    fn default() -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

/// Троттлинг перепрокладки пути во время погони.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ChaseRepath(pub Timer);

impl Default for ChaseRepath {
    fn default() -> Self {
        Self(Timer::from_seconds(0.4, TimerMode::Repeating))
    }
}

/// Пауза «пожирания» над трупом.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct DevourUntil(pub Timer);

impl Default for DevourUntil {
    fn default() -> Self {
        Self(Timer::from_seconds(1.5, TimerMode::Once))
    }
}

/// Демон догнал человека: жертва умирает, демон переходит в Devour.
#[derive(Event, Debug)]
pub struct DemonCaughtHumanEvent {
    pub demon: Entity,
    pub human: Entity,
}

/// Спавнер демонов: стартовый залп, затем по таймеру до капа.
#[derive(Resource)]
pub struct DemonSpawner {
    pub timer: Timer,
    pub spawned: usize,
    pub initial_burst_done: bool,
}

impl Default for DemonSpawner {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(DEMON_SPAWN_INTERVAL, TimerMode::Repeating),
            spawned: 0,
            initial_burst_done: false,
        }
    }
}
