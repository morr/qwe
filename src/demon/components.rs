use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{DEMON_CAP, DEMON_SPAWN_INTERVAL, DEMON_SPAWN_PAUSE};

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

/// Финальный бросок: демон идёт напрямую на текущую позицию жертвы, минуя
/// тайловый путь. Ставится и снимается в `chase`; movepath-гизмо по нему
/// рисует стрелку прямо в цель, а не по остаткам старого пути.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonLungeTag;

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

/// Пауза сразу после выхода из портала: демон стоит на месте, пока тикает, —
/// и не выбирает цель прогулки, и не берёт агро. Компонент снимает
/// `tick_spawn_pause`, поэтому «не в паузе» проверяется одним
/// `Without<DemonSpawnPause>` в фильтре запроса, без чтения таймера.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct DemonSpawnPause(pub Timer);

impl Default for DemonSpawnPause {
    fn default() -> Self {
        Self(Timer::from_seconds(DEMON_SPAWN_PAUSE.0, TimerMode::Once))
    }
}

/// Демон догнал человека: жертва умирает, демон переходит в Devour.
#[derive(Event, Debug)]
pub struct DemonCaughtHumanEvent {
    pub demon: Entity,
    pub human: Entity,
}

/// Настройки спавна, крутятся ползунками панели World и сохраняются между
/// запусками. Отдельно от `DemonSpawner`: тот — состояние мира и сбрасывается
/// на рестарте и смене города, а это — выбор пользователя, который рестарт
/// переживает.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "demon_spawn")]
pub struct DemonSpawnStyle {
    /// Потолок числа демонов; дойдя до него, спавнер молчит. Понижение уже
    /// вышедших демонов не убирает — оно видно только после рестарта.
    pub cap: usize,
    /// Секунды между демонами после стартового залпа.
    pub interval: f32,
}

impl Default for DemonSpawnStyle {
    fn default() -> Self {
        Self {
            cap: DEMON_CAP,
            interval: DEMON_SPAWN_INTERVAL,
        }
    }
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
