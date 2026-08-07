use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::HUMAN_SPEED_SPREAD;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Human;

/// Личный разброс скорости, **нормированный**: −1…+1, разыгрывается один раз
/// при спавне. Реальная скорость — `base × (1 + Pace × HumanStyle::spread)`,
/// то есть отрицательный жребий замедляет, положительный ускоряет, ноль
/// оставляет базу как есть. Множитель один на обе базы, шаг и бег: быстрый
/// человек быстр и в прогулке, и в панике.
///
/// Хранится нормированным, а не готовым множителем, ради ползунка разброса:
/// так ползунок раздвигает уже разыгранный порядок толпы (на 0% все идут
/// ровно, дальше расходятся), а не перекидывает каждому новый жребий на
/// каждый кадр перетаскивания.
///
/// Компонентом, а не выводом из `Movable::speed`: ту переписывает каждый
/// переход Wander ⇄ Flee, и первая же паника стёрла бы разброс.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Pace(pub f32);

impl Pace {
    /// Скорость этого человека при базовой `base`.
    pub fn speed(&self, base: f32, spread: f32) -> f32 {
        debug_assert!((-1.0..=1.0).contains(&self.0), "Pace вне −1…+1: {}", self.0);
        base * (1.0 + self.0 * spread)
    }
}

/// Настройки людей, крутятся ползунками панели Human и сохраняются между
/// запусками — тот же контракт, что у `DemonStyle`: это выбор пользователя, а
/// не состояние мира, и рестарт он переживает.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "human")]
pub struct HumanStyle {
    /// Полуширина разброса личной скорости: множитель каждого человека лежит
    /// в 1 ± spread. На нуле вся толпа идёт с базовой скоростью.
    pub spread: f32,
}

impl Default for HumanStyle {
    fn default() -> Self {
        Self {
            spread: HUMAN_SPEED_SPREAD,
        }
    }
}

/// Стейт-машина человека: Wander / Flee — эксклюзивные теги.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HumanWanderTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HumanFleeTag;

/// Первая прогулка после спавна — всегда короткая, «по делам» человек идёт
/// только со второй. Иначе 20 000 маршрутов через весь город подаются в один
/// кадр: такой A* стоит сотни мс на запрос, и пешки в кадре разъезжаются
/// секундами (см. фазу прогрева в `loading.rs`). Тег снимается при выборе
/// первой цели.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HumanFirstWanderTag;

/// Курс прогулки — единичный вектор последнего направления движения.
/// Следующая цель, и короткая и дальняя, выбирается в конусе вокруг него
/// (`HUMAN_WANDER_CONE`): без памяти направления пешка на каждом шаге
/// разворачивалась случайно и топталась на месте.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct WanderHeading(pub Vec2);

impl Default for WanderHeading {
    fn default() -> Self {
        Self(Vec2::X)
    }
}

/// Запретный конус после паники: единичный вектор в сторону демона,
/// запомненный на последней перепрокладке бегства. Пока компонент висит,
/// первая цель после успокоения обязана быть дальней («по делам») и не
/// попадать в конус `RECOIL_CONE` вокруг этого вектора; снимается при первом
/// же удачном выборе цели — дальше человек гуляет как обычно.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct PanicRecoil(pub Vec2);

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
