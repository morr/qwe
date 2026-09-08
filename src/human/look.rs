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
//! под одним из [`CORPSE_HEADINGS`] направлений, а под грудью у него кровь:
//! растекающаяся лужа и брызги вокруг неё (`silhouette::blood`, две дочерние
//! сущности). Поза, форма пятна, поворот, размер и тон берутся из битов
//! `Entity` — это косметика, не состояние прогона, и потоку решений пешки тут
//! делать нечего.

use std::f32::consts::TAU;

use bevy::prelude::*;
use rand::Rng;

use super::components::{Attire, HumanFleeTag};
use crate::rng::splitmix64;
use crate::settings::{CORPSE_HEIGHT, HUMAN_MIN_PX, HUMAN_SIZE, Z_CORPSE};
use crate::silhouette::{Glyph, Silhouette, Silhouettes, blood, figure, set_glyph};

/// Тон паники — один на всех: янтарь, тёплый, но не красный демона.
pub const PANIC_COLOR: Color = Color::srgb(1.0, 0.70, 0.15);
/// Оттенки одежды: холодная половина круга — от бирюзы через синий к лиловому.
const ATTIRE_HUE: std::ops::Range<f32> = 170.0..290.0;
/// Насыщенность и светлота одежды: темнее подложки карты, без кричащих тонов.
const ATTIRE_SATURATION: std::ops::Range<f32> = 0.15..0.50;
const ATTIRE_LIGHTNESS: std::ops::Range<f32> = 0.30..0.55;

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
// --- Кровь ---
/// Кровь: багровая, не HDR — лужа не светится.
///
/// Это цвет **тонкой** плёнки у кромки: густую середину пятна затемняет сам
/// глиф (`silhouette::blood`), а покрасить ярче цвета спрайта он не может —
/// яркость пишется в 8 бит и обрезается единицей. Отсюда и порядок: светлое
/// задаётся здесь, тёмное считается там.
///
/// Непрозрачность тоже принадлежит глифу, а не цвету, и потому альфа здесь
/// единица: прозрачной кровь делает тонкий слой, а не краска. Полупрозрачный
/// цвет пробовали — на светлой мостовой даже восьмая доля земли (её линейная
/// яркость 0,74 против 0,3 у крови) выбеливала лужу в бурое пятно.
const BLOOD_HUE: f32 = 2.0;
const BLOOD_SATURATION: f32 = 0.84;
const BLOOD_LIGHTNESS: f32 = 0.36;
const BLOOD_ALPHA: f32 = 1.0;
/// Разброс тона по телам: свежая кровь краснее и ярче, постоявшая — темнее и
/// буроватее. Шаг в одну сторону, не палитра: кровь обязана остаться кровью, и
/// тёплое на карте по-прежнему значит демонов и панику.
const BLOOD_HUE_SPREAD: f32 = 12.0;
const BLOOD_LIGHT_SPREAD: std::ops::Range<f32> = 0.78..1.14;
/// Разброс размера пятна по телам.
const BLOOD_SIZE: std::ops::Range<f32> = 0.80..1.20;
/// Сколько поворотов у пятна: шаг в шесть градусов, и соседние лужи не
/// читаются одним штампом даже при одном глифе.
const BLOOD_SPINS: u64 = 64;
/// Лужа в долях ячейки трупа: пятно шире торса, из-под тела видно с любого
/// бока.
const POOL_RATIO: f32 = 0.75;
/// Веер брызг в тех же долях: он обязан выходить далеко за тело, иначе это не
/// брызги, а кайма лужи.
const SPATTER_RATIO: f32 = 1.35;
/// Лужа и брызги по z относительно тела: обе под ним, лужа — поверх брызг
/// (капли легли первыми, кровь натекла на них).
const Z_POOL: f32 = -0.02;
const Z_SPATTER: f32 = -0.03;
/// За сколько секунд симуляции лужа растекается до полного размера и с какой
/// доли его начинает. Мгновенно возникшее под телом пятно — единственное, что
/// в этой картинке выдаёт штамп; полторы секунды хватает, чтобы убийство на
/// глазах читалось как убийство.
const SPREAD_SECS: f32 = 1.6;
const SPREAD_START: f32 = 0.30;
/// Соль перемешивания: кровь разыгрывается своим числом, а не битами позы —
/// иначе поза и форма лужи ходили бы парой, и на экране была бы видна
/// четвёрка пар вместо четырёх десятков сочетаний.
const BLOOD_SALT: u64 = 0x_c105_e700_0000_00a5;

/// Лужа крови под трупом — дочерняя сущность тела, как ореол у демона:
/// исчезает вместе с ним, своего `DespawnOnExit` не носит.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct BloodPool;

/// Брызги вокруг тела — вторая дочерняя сущность, отдельно от лужи.
///
/// Отдельно, потому что это два разных события: капли легли **разом**, в
/// момент удара, и больше не меняются, а лужа натекает **потом** и растёт на
/// глазах. Одним спрайтом пришлось бы выбирать — либо растить вместе с лужей и
/// капли (они не растут), либо отказаться от роста. Заодно ячейка веера почти
/// вдвое шире ячейки лужи, и одна на двоих отняла бы у лужи половину
/// разрешения атласа.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct BloodSpatter;

/// Лужа ещё растекается: доля натёкшего и полный размер, к которому она идёт.
/// Снимается по достижении полного — [`spread_blood`] ходит только по свежим.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct BloodSpread {
    grown: f32,
    full: Vec2,
}

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
    let hash = splitmix64(entity.to_bits());
    CorpsePose {
        glyph: Glyph::corpse((hash % figure::POSES as u64) as usize),
        heading: ((hash >> 8) % CORPSE_HEADINGS) as f32 / CORPSE_HEADINGS as f32 * TAU,
        flip: (hash >> 16) & 1 == 1,
    }
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
        sprite.flip_x = pose.flip;
        set_glyph(&mut sprite, pose.glyph);
    }
    if let Some(mut transform) = body.get_mut::<Transform>() {
        transform.translation.z = Z_CORPSE;
        transform.rotation = Quat::from_rotation_z(pose.heading);
    }
}

/// Как выглядит кровь под этим телом: какая из луж, какой веер брызг, как они
/// повёрнуты, насколько крупны и какого тона.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloodLook {
    pub pool: Glyph,
    pub spatter: Glyph,
    /// Свой поворот у лужи и у веера, радианы, — поверх поворота тела. Без
    /// него четыре формы читались бы четырьмя штампами: тел на карте тысячи.
    pub pool_spin: f32,
    pub spatter_spin: f32,
    /// Размер пятна в долях полного.
    pub size: f32,
    pub tint: Color,
}

impl BloodLook {
    /// Кровь без разброса: базовый тон, полный размер, без поворота. Ею
    /// витрина (`examples/demos/blood_gallery`) показывает сами формы —
    /// разброс там только мешал бы их сравнивать.
    pub fn plain(pool: Glyph, spatter: Glyph) -> Self {
        Self {
            pool,
            spatter,
            pool_spin: 0.0,
            spatter_spin: 0.0,
            size: 1.0,
            tint: Color::hsla(BLOOD_HUE, BLOOD_SATURATION, BLOOD_LIGHTNESS, BLOOD_ALPHA),
        }
    }
}

/// Кровь по битам `Entity` — как и поза, косметика, а не состояние прогона:
/// потоку решений пешки тут делать нечего.
pub fn blood_look(entity: Entity) -> BloodLook {
    let hash = splitmix64(entity.to_bits() ^ BLOOD_SALT);
    let spin = |shift: u32| ((hash >> shift) % BLOOD_SPINS) as f32 / BLOOD_SPINS as f32 * TAU;
    let unit = |shift: u32| ((hash >> shift) & 0xff) as f32 / 255.0;
    let span = |range: &std::ops::Range<f32>, at: f32| range.start + (range.end - range.start) * at;
    // сдвиги не перекрываются: каждая мелочь берёт свой кусок числа, иначе
    // размер и тон ходили бы за формой
    BloodLook {
        pool: Glyph::pool((hash % blood::POOLS as u64) as usize),
        spatter: Glyph::spatter(((hash >> 6) % blood::SPATTERS as u64) as usize),
        pool_spin: spin(12),
        spatter_spin: spin(20),
        size: span(&BLOOD_SIZE, unit(28)),
        tint: Color::hsla(
            BLOOD_HUE + BLOOD_HUE_SPREAD * unit(38),
            BLOOD_SATURATION,
            BLOOD_LIGHTNESS * span(&BLOOD_LIGHT_SPREAD, unit(48)),
            BLOOD_ALPHA,
        ),
    }
}

/// Лужа под грудью тела, чуть ниже по z, — ещё не натёкшая: размер ей ведёт
/// [`spread_blood`], пока не снимет с неё [`BloodSpread`].
///
/// Наружу — ради витрины крови (`examples/demos/blood_gallery`): она показывает
/// ту же сущность, что игра вешает на труп, а не свою копию. Тот же довод, по
/// которому наружу отдан `pick_wander_targets`.
pub fn blood_pool(silhouettes: &Silhouettes, pose: CorpsePose, look: BloodLook) -> impl Bundle {
    let full = Vec2::splat(CORPSE_SPAN * POOL_RATIO * look.size);
    let born = full * SPREAD_START;
    (
        BloodPool,
        BloodSpread { grown: 0.0, full },
        silhouettes.sprite(look.pool, look.tint, born),
        Silhouette::new(born, HUMAN_MIN_PX),
        stain_transform(pose, look.pool_spin, Z_POOL),
        Name::new("blood_pool"),
    )
}

/// Брызги вокруг тела: та же точка, шире лужи и под ней. Роста у них нет —
/// они легли разом. Наружу по тому же поводу, что и [`blood_pool`].
pub fn blood_spatter(silhouettes: &Silhouettes, pose: CorpsePose, look: BloodLook) -> impl Bundle {
    let size = Vec2::splat(CORPSE_SPAN * SPATTER_RATIO * look.size);
    (
        BloodSpatter,
        silhouettes.sprite(look.spatter, look.tint, size),
        Silhouette::new(size, HUMAN_MIN_PX),
        stain_transform(pose, look.spatter_spin, Z_SPATTER),
        Name::new("blood_spatter"),
    )
}

/// Где стоит пятно и как повёрнуто. Смещение — в системе тела до поворота:
/// дочерний `Transform` поворачивается вместе с родителем, а зеркало спрайта
/// переворачивает и грудь.
fn stain_transform(pose: CorpsePose, spin: f32, z: f32) -> Transform {
    let mut anchor = pose.glyph.pool_anchor() * CORPSE_SPAN / 2.0;
    if pose.flip {
        anchor.x = -anchor.x;
    }
    Transform::from_translation(anchor.extend(z)).with_rotation(Quat::from_rotation_z(spin))
}

/// Лужа растекается: от [`SPREAD_START`] до полного размера за
/// [`SPREAD_SECS`] секунд симуляции, и на этом [`BloodSpread`] с неё
/// снимается. Проход поэтому стоит ровно столько, сколько людей убили за
/// последние полторы секунды, а не сколько трупов лежит на карте.
///
/// Косметика — отсюда `Update` и `Res<Time>` (виртуальное время): на паузе
/// кровь стоит, на 30× течёт втридцатеро быстрее, то есть по часам симуляции
/// натекает всегда за одно и то же время. Размер пишется в `Silhouette`, а не
/// в `Sprite::custom_size`: тот принадлежит `silhouette`, и переписанный
/// силуэт доедет до спрайта его же системой (`size_fresh_silhouettes`).
pub fn spread_blood(
    time: Res<Time>,
    mut commands: Commands,
    mut pools: Query<(Entity, &mut BloodSpread, &mut Silhouette)>,
) {
    for (entity, mut spread, mut silhouette) in &mut pools {
        spread.grown = (spread.grown + time.delta_secs() / SPREAD_SECS).min(1.0);
        // быстро в начале, медленно к концу: кровь бежит, пока её толкает
        let eased = 1.0 - (1.0 - spread.grown).powi(2);
        silhouette.body = spread.full * (SPREAD_START + (1.0 - SPREAD_START) * eased);
        if spread.grown >= 1.0 {
            commands.entity(entity).remove::<BloodSpread>();
        }
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
            seen.insert((pose.glyph.cell(), step, pose.flip));
            per_glyph[pose.glyph.cell() - Glyph::corpse(0).cell()] += 1;
        }
        assert_eq!(seen.len(), figure::POSES * CORPSE_HEADINGS as usize * 2);
        for (glyph, count) in per_glyph.iter().enumerate() {
            let share = *count as f32 / sample as f32;
            assert!((0.2..0.3).contains(&share), "pose {glyph} takes {share}");
        }
    }

    /// Поза, у которой грудь заметно смещена от середины ячейки.
    const PRONE: CorpsePose = CorpsePose {
        glyph: Glyph::Corpse(1),
        heading: 0.0,
        flip: false,
    };

    /// Кровь любого тела — обе её сущности разом.
    fn blood_of(pose: CorpsePose, look: BloodLook) -> (World, Entity, Entity) {
        let mut world = World::new();
        let atlas = Silhouettes::default();
        let pool = world.spawn(blood_pool(&atlas, pose, look)).id();
        let spatter = world.spawn(blood_spatter(&atlas, pose, look)).id();
        (world, pool, spatter)
    }

    #[test]
    fn the_blood_follows_the_chest_into_the_mirror() {
        let mirrored = CorpsePose {
            flip: true,
            ..PRONE
        };
        let anchor = PRONE.glyph.pool_anchor() * CORPSE_SPAN / 2.0;
        assert!(anchor.x > 0.0, "the chest lies toward the head");
        let look = blood_look(Entity::from_raw_u32(7).unwrap());
        let at = |pose| {
            let (world, pool, spatter) = blood_of(pose, look);
            [pool, spatter].map(|entity| world.get::<Transform>(entity).unwrap().translation)
        };
        for translation in at(PRONE) {
            assert_eq!(translation.xy(), anchor);
            assert!(translation.z < 0.0, "the blood lies under the body");
        }
        for translation in at(mirrored) {
            assert_eq!(translation.xy(), Vec2::new(-anchor.x, anchor.y));
        }
        // брызги легли первыми, лужа натекла поверх них
        let (world, pool, spatter) = blood_of(PRONE, look);
        let z = |entity| world.get::<Transform>(entity).unwrap().translation.z;
        assert!(z(spatter) < z(pool));
    }

    /// Веер обязан выходить далеко за лужу, иначе это не брызги, а её кайма.
    #[test]
    fn the_spatter_reaches_well_past_the_pool() {
        let look = blood_look(Entity::from_raw_u32(11).unwrap());
        let (world, pool, spatter) = blood_of(PRONE, look);
        let body = |entity| world.get::<Silhouette>(entity).unwrap().body;
        assert!(body(spatter).x > body(pool).x * 2.0);
    }

    /// Кровь разыгрывается независимо от позы: одинаковых пар «поза + лужа» на
    /// экране быть не должно раньше, чем кончатся все сочетания.
    #[test]
    fn blood_spreads_over_every_glyph_spin_and_shade() {
        let mut pools = std::collections::HashSet::new();
        let mut spatters = std::collections::HashSet::new();
        let mut pairs = std::collections::HashSet::new();
        let mut spins = std::collections::HashSet::new();
        let mut tints = std::collections::HashSet::new();
        let sample = 4096u32;
        for index in 0..sample {
            let entity = Entity::from_raw_u32(index).unwrap();
            let look = blood_look(entity);
            pools.insert(look.pool);
            spatters.insert(look.spatter);
            pairs.insert((corpse_pose(entity).glyph, look.pool));
            spins.insert(look.pool_spin.to_bits());
            tints.insert(format!("{:?}", look.tint));
            assert!((BLOOD_SIZE.start..=BLOOD_SIZE.end).contains(&look.size));
        }
        assert_eq!(pools.len(), blood::POOLS, "не все лужи в ходу");
        assert_eq!(spatters.len(), blood::SPATTERS, "не все брызги в ходу");
        assert_eq!(
            pairs.len(),
            figure::POSES * blood::POOLS,
            "поза и лужа ходят парой"
        );
        assert_eq!(spins.len(), BLOOD_SPINS as usize);
        assert!(
            tints.len() > 16,
            "тон крови одинаков у всех: {}",
            tints.len()
        );
    }

    /// Лужа растекается и на этом перестаёт стоить хоть что-то.
    #[test]
    fn a_pool_spreads_to_its_full_size_and_then_stops_being_walked() {
        let mut app = App::new();
        app.add_systems(Update, spread_blood)
            .insert_resource(Time::<()>::default());
        let look = blood_look(Entity::from_raw_u32(3).unwrap());
        let pool = app
            .world_mut()
            .spawn(blood_pool(&Silhouettes::default(), PRONE, look))
            .id();
        let full = app.world().get::<BloodSpread>(pool).unwrap().full;
        let born = app.world().get::<Silhouette>(pool).unwrap().body;
        assert!(born.x < full.x, "лужа рождается натёкшей");

        // время двигают руками: `Time<()>` без обновления стоит на нуле
        for _ in 0..4 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(SPREAD_SECS / 2.0));
            app.update();
        }
        let grown = app.world().get::<Silhouette>(pool).unwrap().body;
        assert!(
            grown.distance(full) < 1e-4,
            "лужа встала на {grown} из {full}"
        );
        assert!(
            app.world().get::<BloodSpread>(pool).is_none(),
            "натёкшая лужа обязана уйти из прохода"
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
