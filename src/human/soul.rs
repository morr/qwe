//! Душа: золотая искра, поднимающаяся над телом убитого и гаснущая за
//! полторы секунды. Убийство — единственное, что игрок с толпы получает
//! (`Telemetry::killed`, будущий ресурс душ), и оно обязано быть видно с зума
//! толпы, а не только числом в HUD.
//!
//! Косметика в `FixedUpdate`, а не в `Update`: искра — сущность мира, а
//! despawn из середины `Update` этому проекту запрещён (CLAUDE.md, «Where a
//! mass despawn may happen»). Шаг в 64 Гц на полуторасекундном подъёме
//! неразличим, а на 30× искра всё равно живёт три кадра.

use bevy::prelude::*;

use crate::loading::AppState;
use crate::settings::{SOUL_LIFE, SOUL_MIN_PX, SOUL_RISE, SOUL_SIZE, Z_SOUL};
use crate::silhouette::{Glyph, Silhouette, Silhouettes};

/// Золото ярче белого — HDR, светится через bloom (`post.rs`).
const SOUL_COLOR: LinearRgba = LinearRgba::new(2.6, 2.1, 0.9, 1.0);

/// Искра души над телом; `age` — сколько она уже живёт.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct SoulMote {
    pub age: Timer,
}

/// Выпустить душу в точке `at` — зовётся обсервером убийства.
pub fn release_soul(commands: &mut Commands, silhouettes: &Silhouettes, at: Vec2) {
    commands.spawn((
        SoulMote {
            age: Timer::from_seconds(SOUL_LIFE, TimerMode::Once),
        },
        silhouettes.sprite(Glyph::Halo, SOUL_COLOR.into(), Vec2::splat(SOUL_SIZE)),
        Silhouette::new(Vec2::splat(SOUL_SIZE), SOUL_MIN_PX),
        Transform::from_translation(at.extend(Z_SOUL)),
        DespawnOnExit(AppState::Playing),
        Name::new("soul"),
    ));
}

/// Искра поднимается и гаснет квадратично; отжившая — despawn.
pub fn rise_souls(
    mut commands: Commands,
    time: Res<Time>,
    mut souls: Query<(Entity, &mut SoulMote, &mut Transform, &mut Sprite)>,
) {
    for (entity, mut soul, mut transform, mut sprite) in &mut souls {
        soul.age.tick(time.delta());
        if soul.age.is_finished() {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation.y += SOUL_RISE / SOUL_LIFE * time.delta_secs();
        let left = 1.0 - soul.age.fraction();
        sprite.color = Color::from(SOUL_COLOR).with_alpha(left * left);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;

    /// Искра живёт ровно [`SOUL_LIFE`]: поднимается, пока жива, и исчезает
    /// на первом тике после.
    #[test]
    fn a_soul_rises_and_then_despawns() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rise_souls);
        // ровно один шаг `Time` на `app.update()`, чтобы считать жизнь тиками
        let step = Duration::from_millis(100);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(step));

        let start = Vec2::new(10.0, 20.0);
        // атлас пуст, как в любом приложении без рендера: искра — квадрат
        let silhouettes = Silhouettes::default();
        release_soul(&mut app.world_mut().commands(), &silhouettes, start);
        app.world_mut().flush();
        let soul = app
            .world_mut()
            .query_filtered::<Entity, With<SoulMote>>()
            .single(app.world())
            .expect("soul spawned");

        // первый `update` лишь заводит часы (дельта нулевая), второй — шаг
        app.update();
        app.update();
        let y = app.world().get::<Transform>(soul).unwrap().translation.y;
        assert!(y > start.y, "the soul should rise: y = {y}");

        let ticks = (SOUL_LIFE / step.as_secs_f32()).ceil() as usize + 1;
        for _ in 0..ticks {
            app.update();
        }
        assert!(
            app.world().get_entity(soul).is_err(),
            "the soul should be gone after {SOUL_LIFE} s"
        );
    }
}
