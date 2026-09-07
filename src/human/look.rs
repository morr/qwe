//! Как рисуется человек: одежда, силуэт-диск, тон паники и труп.
//!
//! Одежда — холодная и приглушённая, чтобы толпа читалась как толпа, а не как
//! конфетти, и чтобы тёплое на карте значило ровно одно: демонов и панику.
//! Бегущий перекрашивается в один на всех тёплый тон — фронт паники тогда
//! виден пятном, расползающимся от демонов; успокоившийся возвращает свою
//! одежду. Цвета живут здесь, у рисунка, как `demon_tint` у демона и
//! `roof_color` у зданий.
//!
//! Труп — фигура лежащего человека (`silhouette::figure`) в погасшей одежде,
//! под одним из [`CORPSE_HEADINGS`] направлений, с лужей крови под грудью.
//! Поза, направление и зеркало берутся из битов `Entity` — это косметика, не
//! состояние прогона, и потоку решений пешки тут делать нечего.

use std::f32::consts::TAU;

use bevy::prelude::*;
use rand::Rng;

use super::components::{Attire, HumanFleeTag};
use crate::settings::{CORPSE_HEIGHT, HUMAN_MIN_PX, HUMAN_SIZE, Z_CORPSE};
use crate::silhouette::{Glyph, Silhouette, Silhouettes, figure, set_glyph};

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

// --- Труп ---
/// Сторона ячейки трупа, м: рост фигуры с запасом на раскинутые конечности.
pub const CORPSE_SPAN: f32 = CORPSE_HEIGHT * figure::CELL_SPAN;
/// Сколько направлений у лежащего тела. Полный оборот, а не полуоборот, как
/// было у эллипса: у фигуры есть голова и ноги.
pub const CORPSE_HEADINGS: u64 = 16;
/// Во сколько раз труп бледнее и темнее своей одежды: одежда остаётся, цвет
/// уходит — мёртвого видно по тому, что он погас, а не по чужой краске.
const CORPSE_DRAIN: f32 = 0.5;
/// Одежда тела без `Attire` (пешка из теста): серо-бурая.
const CORPSE_FALLBACK: Color = Color::srgb(0.35, 0.30, 0.30);
/// Кровь: багровая, чуть прозрачная — на асфальте и на траве читается пятном,
/// не краской. Не HDR: лужа не светится.
const BLOOD_COLOR: Color = Color::srgba(0.50, 0.03, 0.04, 0.9);
/// Лужа в долях ячейки трупа: пятно шире торса, из-под тела видно с любого
/// бока.
const POOL_RATIO: f32 = 0.75;

/// Лужа крови под трупом — дочерняя сущность тела, как ореол у демона:
/// исчезает вместе с ним, своего `DespawnOnExit` не носит.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct BloodPool;

/// Как лежит тело: поза, направление и зеркало.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorpsePose {
    pub glyph: Glyph,
    /// Поворот вокруг z, радианы.
    pub heading: f32,
    pub flip: bool,
}

/// Поза по битам `Entity`: соседние индексы — соседи по числу, а позы соседних
/// трупов обязаны различаться, поэтому биты сперва перемешиваются (splitmix64).
pub fn corpse_pose(entity: Entity) -> CorpsePose {
    let hash = mix(entity.to_bits());
    CorpsePose {
        glyph: Glyph::corpse((hash % figure::POSES as u64) as usize),
        heading: ((hash >> 8) % CORPSE_HEADINGS) as f32 / CORPSE_HEADINGS as f32 * TAU,
        flip: (hash >> 16) & 1 == 1,
    }
}

fn mix(bits: u64) -> u64 {
    let mut z = bits.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Цвет трупа: своя одежда, погасшая.
pub(super) fn corpse_tint(attire: Option<&Attire>) -> Color {
    let Hsla {
        hue,
        saturation,
        lightness,
        ..
    } = attire.map_or(CORPSE_FALLBACK, |attire| attire.0).into();
    Color::hsl(hue, saturation * CORPSE_DRAIN, lightness * CORPSE_DRAIN)
}

/// Тело ложится: глиф позы, погасшая одежда, зеркало, поворот и `Z_CORPSE`.
/// Одной командой с доступом к сущности — цвет считается из её же одежды.
pub(super) fn lay_down(body: &mut EntityWorldMut, pose: CorpsePose) {
    let tint = corpse_tint(body.get::<Attire>());
    if let Some(mut sprite) = body.get_mut::<Sprite>() {
        sprite.color = tint;
        sprite.custom_size = Some(Vec2::splat(CORPSE_SPAN));
        sprite.flip_x = pose.flip;
        set_glyph(&mut sprite, pose.glyph);
    }
    if let Some(mut transform) = body.get_mut::<Transform>() {
        transform.translation.z = Z_CORPSE;
        transform.rotation = Quat::from_rotation_z(pose.heading);
    }
}

/// Лужа под грудью тела, чуть ниже по z. Смещение — в системе тела до
/// поворота: дочерний `Transform` поворачивается вместе с родителем, а зеркало
/// спрайта переворачивает и грудь.
pub(super) fn blood_pool(silhouettes: &Silhouettes, pose: CorpsePose) -> impl Bundle {
    let size = Vec2::splat(CORPSE_SPAN * POOL_RATIO);
    let mut anchor = pose.glyph.pool_anchor() * CORPSE_SPAN / 2.0;
    if pose.flip {
        anchor.x = -anchor.x;
    }
    (
        BloodPool,
        silhouettes.sprite(Glyph::Pool, BLOOD_COLOR, size),
        Silhouette::new(size, HUMAN_MIN_PX),
        Transform::from_translation(anchor.extend(-0.02)),
        Name::new("blood_pool"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpse_tint_keeps_the_hue_and_drains_the_light() {
        let attire = Attire(Color::hsl(200.0, 0.4, 0.5));
        let Hsla {
            hue,
            saturation,
            lightness,
            ..
        } = corpse_tint(Some(&attire)).into();
        assert!((hue - 200.0).abs() < 0.5, "hue drifted to {hue}");
        assert!((saturation - 0.2).abs() < 1e-3);
        assert!((lightness - 0.25).abs() < 1e-3);
        assert_ne!(corpse_tint(None), attire.0);
    }

    #[test]
    fn corpse_poses_cover_every_variant_and_spread_evenly() {
        let mut seen = std::collections::HashSet::new();
        let mut per_glyph = [0usize; figure::POSES];
        let sample = 4096u32;
        for index in 0..sample {
            let pose = corpse_pose(Entity::from_raw_u32(index).unwrap());
            let step = (pose.heading / TAU * CORPSE_HEADINGS as f32).round() as u64;
            seen.insert((pose.glyph as usize, step, pose.flip));
            per_glyph[pose.glyph as usize - Glyph::corpse(0) as usize] += 1;
        }
        assert_eq!(seen.len(), figure::POSES * CORPSE_HEADINGS as usize * 2);
        for (glyph, count) in per_glyph.iter().enumerate() {
            let share = *count as f32 / sample as f32;
            assert!((0.2..0.3).contains(&share), "pose {glyph} takes {share}");
        }
    }

    #[test]
    fn the_pool_follows_the_chest_into_the_mirror() {
        let plain = CorpsePose {
            glyph: Glyph::Prone,
            heading: 0.0,
            flip: false,
        };
        let mirrored = CorpsePose {
            flip: true,
            ..plain
        };
        let anchor = Glyph::Prone.pool_anchor() * CORPSE_SPAN / 2.0;
        assert!(anchor.x > 0.0, "the chest lies toward the head");
        let at = |pose| {
            let mut world = World::new();
            let pool = world.spawn(blood_pool(&Silhouettes::default(), pose)).id();
            world.get::<Transform>(pool).unwrap().translation
        };
        assert_eq!(at(plain).xy(), anchor);
        assert_eq!(at(mirrored).xy(), Vec2::new(-anchor.x, anchor.y));
        assert!(at(plain).z < 0.0, "the pool lies under the body");
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
