//! Как рисуется живой человек: одежда, силуэт-диск, тон паники.
//!
//! Одежда — холодная и приглушённая, чтобы толпа читалась как толпа, а не как
//! конфетти, и чтобы тёплое на карте значило ровно одно: демонов и панику.
//! Бегущий перекрашивается в один на всех тёплый тон — фронт паники тогда
//! виден пятном, расползающимся от демонов; успокоившийся возвращает свою
//! одежду. Цвета живут здесь, у рисунка, как `demon_tint` у демона и
//! `roof_color` у зданий. Труп с его кровью — соседний модуль [`super::corpse`].

use bevy::prelude::*;
use rand::RngExt;

use super::components::{Attire, HumanFleeTag};
use crate::settings::{HUMAN_MIN_PX, HUMAN_SIZE};
use crate::silhouette::{Glyph, Silhouette, Silhouettes};

/// Тон паники — один на всех: янтарь, тёплый, но не красный демона.
pub(crate) const PANIC_COLOR: Color = Color::srgb(1.0, 0.70, 0.15);
/// Оттенки одежды: холодная половина круга — от бирюзы через синий к лиловому.
const ATTIRE_HUE: std::ops::Range<f32> = 170.0..290.0;
/// Насыщенность и светлота одежды: темнее подложки карты, без кричащих тонов.
const ATTIRE_SATURATION: std::ops::Range<f32> = 0.15..0.50;
const ATTIRE_LIGHTNESS: std::ops::Range<f32> = 0.30..0.55;

/// Шаги квантования цвета одежды, доли единицы и градусы. Цвет в полном
/// разрешении f32 у каждой пешки свой, а рендер собирает спрайты с одинаковым
/// `Sprite` в один материал по **точному** ключу (`bevy_sprite_render`:
/// `SpriteMeshMaterialBucketKey` хеширует биты цвета), и кеш из ~20 000
/// уникальных цветов обходится eviction-проходом **каждый кадр** — замер
/// 11.2026, ~0.5 мс/кадр на простое, растёт линейно с населением. Шаги ниже
/// дают ≤ 40 × 8 × 6 ≈ 1 900 бакетов (на деле ~1 000), глазом шаг не виден
/// на приглушённой палитре. Квантование после броска: число и порядок
/// бросков потока решений не меняются (`determinism`).
const ATTIRE_HUE_STEP: f32 = 3.0;
const ATTIRE_SATURATION_STEP: f32 = 0.05;
const ATTIRE_LIGHTNESS_STEP: f32 = 0.05;

/// Одежда — три броска потока решений пешки, как и прежде: число и порядок
/// бросков менять нельзя, за ними идут темп и курс.
///
/// Второй потребитель этой раскладки — стенд расталкивания
/// (`examples/demos/crowd_demo/scenario.rs::spawn_pawn`): он тратит на цвет
/// ровно те же три броска своей копией, а не вызовом — стенду нужна своя
/// палитра в полный круг тонов, иначе в куче не различить соседей. Совпадать
/// обязан только счёт: от него зависит, попадут ли темп и курс толпы стенда на
/// городские значения, а значит — сойдутся ли мерки «до» и «после» у замера,
/// к цвету отношения не имеющего. Счёт держит тест
/// `attire_spends_exactly_three_draws`.
pub(super) fn roll_attire(rng: &mut impl RngExt) -> Attire {
    // квантование — не про вид, а про кеш материалов рендера: палитра
    // дискретных значений схлопывает бакеты кеша с населения до палитры
    // (см. комментарий над шагами)
    Attire(Color::hsl(
        (rng.random_range(ATTIRE_HUE) / ATTIRE_HUE_STEP).round() * ATTIRE_HUE_STEP,
        (rng.random_range(ATTIRE_SATURATION) / ATTIRE_SATURATION_STEP).round()
            * ATTIRE_SATURATION_STEP,
        (rng.random_range(ATTIRE_LIGHTNESS) / ATTIRE_LIGHTNESS_STEP).round()
            * ATTIRE_LIGHTNESS_STEP,
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
pub fn on_panic_tint(event: On<Add<HumanFleeTag>>, mut sprites: Query<&mut Sprite>) {
    if let Ok(mut sprite) = sprites.get_mut(event.entity) {
        sprite.color = PANIC_COLOR;
    }
}

/// Паника снята — человек снова в своей одежде. Срабатывает и на трупе, и на
/// despawn: `to_corpse` красит тело уже после, а исчезающему всё равно.
pub fn on_calm_tint(event: On<Remove<HumanFleeTag>>, mut sprites: Query<(&Attire, &mut Sprite)>) {
    if let Ok((attire, mut sprite)) = sprites.get_mut(event.entity) {
        sprite.color = attire.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{RngDomain, WanderIndex, decision_stream};

    /// Одежда обязана стоить ровно три броска: следом в том же потоке идут темп
    /// и курс (`spawn_population`), и ту же раскладку своей копией держит стенд
    /// расталкивания. Лишний или пропавший бросок сдвинул бы обе стороны молча —
    /// та же страховка, что и `the_stroll_branch_spends_exactly_two_draws`.
    #[test]
    fn attire_spends_exactly_three_draws() {
        let mut rolled = decision_stream(1, RngDomain::Human, 0, WanderIndex::SPAWN);
        let _ = roll_attire(&mut rolled);
        let after_attire: f32 = rolled.random_range(0.0..1.0);

        let mut manual = decision_stream(1, RngDomain::Human, 0, WanderIndex::SPAWN);
        for range in [ATTIRE_HUE, ATTIRE_SATURATION, ATTIRE_LIGHTNESS] {
            let _: f32 = manual.random_range(range);
        }
        let after_manual: f32 = manual.random_range(0.0..1.0);

        assert_eq!(
            after_attire.to_bits(),
            after_manual.to_bits(),
            "после одежды поток обязан стоять на четвёртом броске"
        );
    }

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
