use bevy::prelude::*;

use crate::human::components::{Human, HumanFleeTag, HumanStyle, Pace};
use crate::movement::Movable;
use crate::settings::{HUMAN_FLEE_SPEED, HUMAN_WALK_SPEED, RECOIL_CONE};

mod spawn;
#[cfg(test)]
mod tests;
mod wander;

pub use spawn::{
    PendingPopulation, PopulationBirth, PopulationSpawn, pump_pending_population, spawn_humans,
    spawn_population,
};
pub use wander::pick_wander_targets;
// `roll_wander_pause` наружу — её зовёт дальнее блуждание (`human/lod.rs`)
pub(crate) use wander::roll_wander_pause;

/// Лежит ли направление в запретном конусе вокруг `ban`. Косинус растёт с
/// уменьшением угла, поэтому «внутри» — это строгое `>`: точно на границе
/// конуса цель разрешена. Дальнее блуждание спрашивает ту же проверку
/// (`human/lod.rs`): у дальних нет «дел» у зданий, и запретную прогулку
/// отсеивает единственная ветка, что у них есть.
pub(crate) fn in_recoil_cone(direction: Vec2, ban: Vec2) -> bool {
    direction.dot(ban) > RECOIL_CONE.cos()
}

/// `HumanStyle` несёт два независимых поля: `spread` и `body_radius`. Гейт по
/// `resource_changed::<HumanStyle>` запускал обход всей популяции на каждый
/// шаг протяжки ползунка **Body radius**, хотя сама система читает только
/// `spread` и не трогает радиус вовсе.
///
/// Состояние хранится в `Local`; сравнение по значению заодно гасит запись
/// того же числа (например, `ResetSettings` при уже дефолтных настройках).
/// Смена режима детерминизма на ходу — каждой регистрации свой `Local`, и
/// включённая позже ветка на первом прогоне увидит расхождение и догонит
/// правку, ровно как пропущенная по старому ран-кондишену система.
pub fn spread_changed(style: Res<HumanStyle>, mut applied: Local<Option<f32>>) -> bool {
    let current = style.spread;
    let changed = *applied != Some(current);
    *applied = Some(current);
    changed
}

/// Ползунок разброса — людям, уже гуляющим по городу; аналог
/// `sync_demon_speed`, и так же не каждый кадр: гейт — [`spread_changed`].
///
/// База берётся по тегу состояния: пересчитать бегущего от `HUMAN_WALK_SPEED`
/// значило бы посадить его на шаг до самого конца паники — `flee` вернёт
/// беговую скорость только на выходе из состояния, а не на входе.
pub fn sync_human_pace(
    style: Res<HumanStyle>,
    mut humans: Query<(&mut Movable, &Pace, Has<HumanFleeTag>), With<Human>>,
) {
    for (mut movable, pace, fleeing) in &mut humans {
        let base = if fleeing {
            HUMAN_FLEE_SPEED
        } else {
            HUMAN_WALK_SPEED
        };
        movable.speed = pace.speed(base, style.spread);
    }
}
