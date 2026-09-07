//! Как рисуется человек: одежда, силуэт-диск и тон паники.
//!
//! Одежда — холодная и приглушённая, чтобы толпа читалась как толпа, а не как
//! конфетти, и чтобы тёплое на карте значило ровно одно: демонов и панику.
//! Бегущий перекрашивается в один на всех тёплый тон — фронт паники тогда
//! виден пятном, расползающимся от демонов; успокоившийся возвращает свою
//! одежду. Цвета живут здесь, у рисунка, как `demon_tint` у демона и
//! `roof_color` у зданий.

use bevy::prelude::*;
use rand::Rng;

use super::components::{Attire, HumanFleeTag};
use crate::settings::{HUMAN_MIN_PX, HUMAN_SIZE};
use crate::silhouette::{Glyph, Silhouette, Silhouettes};

/// Тон паники — один на всех: янтарь, тёплый, но не красный демона.
pub const PANIC_COLOR: Color = Color::srgb(1.0, 0.70, 0.15);
/// Оттенки одежды: холодная половина круга — от бирюзы через синий к лиловому.
const ATTIRE_HUE: std::ops::Range<f32> = 170.0..290.0;
/// Насыщенность и светлота одежды: темнее подложки карты, без кричащих тонов.
const ATTIRE_SATURATION: std::ops::Range<f32> = 0.15..0.50;
const ATTIRE_LIGHTNESS: std::ops::Range<f32> = 0.30..0.55;

/// Одежда — три броска потока решений пешки, как и прежде: число и порядок
/// бросков менять нельзя, за ними идут темп и курс.
pub(super) fn roll_attire(rng: &mut impl Rng) -> Attire {
    Attire(Color::hsl(
        rng.random_range(ATTIRE_HUE),
        rng.random_range(ATTIRE_SATURATION),
        rng.random_range(ATTIRE_LIGHTNESS),
    ))
}

/// Спрайт и силуэт человека в его одежде.
pub(super) fn human_body(silhouettes: &Silhouettes, attire: &Attire) -> (Sprite, Silhouette) {
    (
        silhouettes.sprite(Glyph::Disc, attire.0, Vec2::splat(HUMAN_SIZE)),
        Silhouette::new(Vec2::splat(HUMAN_SIZE), HUMAN_MIN_PX),
    )
}

/// Паника надета — человек в тоне паники.
pub fn on_panic_tint(event: On<Add, HumanFleeTag>, mut sprites: Query<&mut Sprite>) {
    if let Ok(mut sprite) = sprites.get_mut(event.entity) {
        sprite.color = PANIC_COLOR;
    }
}

/// Паника снята — человек снова в своей одежде. Срабатывает и на трупе, и на
/// despawn: `to_corpse` красит тело уже после, а исчезающему всё равно.
pub fn on_calm_tint(event: On<Remove, HumanFleeTag>, mut sprites: Query<(&Attire, &mut Sprite)>) {
    if let Ok((attire, mut sprite)) = sprites.get_mut(event.entity) {
        sprite.color = attire.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_dresses_the_human_amber_and_calm_undresses_it() {
        let mut app = App::new();
        app.add_observer(on_panic_tint).add_observer(on_calm_tint);
        let attire = Attire(Color::srgb(0.2, 0.3, 0.4));
        let human = app
            .world_mut()
            .spawn((Sprite::from_color(attire.0, Vec2::ONE), attire))
            .id();

        app.world_mut().entity_mut(human).insert(HumanFleeTag);
        assert_eq!(app.world().get::<Sprite>(human).unwrap().color, PANIC_COLOR);

        app.world_mut().entity_mut(human).remove::<HumanFleeTag>();
        assert_eq!(app.world().get::<Sprite>(human).unwrap().color, attire.0);
    }
}
