//! Труп и кровь под ним: поза лежащей фигуры, погасшая одежда, лужа и брызги,
//! и переход [`to_corpse`] — одна точка, где живой человек становится телом.
//!
//! Труп — фигура лежащего человека (`silhouette::figure`) в погасшей одежде,
//! под одним из [`CORPSE_HEADINGS`] направлений, а под грудью у него кровь:
//! растекающаяся лужа и брызги вокруг неё (`silhouette::blood`, две дочерние
//! сущности). Поза, форма пятна, поворот, размер и тон берутся из битов
//! `Entity` — это косметика, не состояние прогона, и потоку решений пешки тут
//! делать нечего.

use std::f32::consts::TAU;

use bevy::prelude::*;

use super::components::{
    Attire, CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanWanderTag,
    PanicRecoil, WanderPause,
};
use crate::camera::Viewport;
use crate::loading::WorldStarted;
use crate::map::z::Z_CORPSE;
use crate::rng::splitmix64;
use crate::settings::{CORPSE_HEIGHT, HUMAN_MIN_PX};
use crate::silhouette::{
    FreshSilhouettes, Glyph, Silhouette, Silhouettes, bake::CorpseQuad, blood, figure, set_glyph,
};

// --- Труп ---
/// Сторона ячейки трупа, м: рост фигуры с запасом на раскинутые конечности.
pub const CORPSE_SPAN: f32 = CORPSE_HEIGHT * figure::CELL_SPAN;
/// Сколько направлений у лежащего тела. Полный оборот, а не полуоборот, как
/// было у эллипса: у фигуры есть голова и ноги.
pub(crate) const CORPSE_HEADINGS: u64 = 16;
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
/// Ступеней у каждого разброса крови: тон, светлота и размер ходят по
/// четырём значениям, а не по всему байту — байт делится на четыре старшими
/// битами (`byte >> 6`), переход между соседними ступенями на глаз не виден,
/// планка та же, по которой квантована одежда.
///
/// Ступени не про вид, а про кеш материалов рендера: спрайты с одинаковым
/// `Sprite` собираются в один материал по **точному** ключу
/// (`bevy_sprite_render::SpriteMeshMaterialBucketKey` хеширует биты цвета и
/// `custom_size`), и непрерывная кровь платила бы кешу по записи на каждый
/// труп — лужа и брызги по записи, — а eviction-проход обходил бы весь кеш
/// каждый кадр, линейно с числом убитых и без потолка (~0.5 мс/кадр на
/// 20 000 записей, замер 11.2026). Со ступенями слой крови — лужа или веер —
/// укладывается в 10 глифов × 4 тона × 4 светлоты × 4 размера = 640 ключей,
/// оба слоя — ≤ 1300 на любом поле боя. Квантование после хеша, до цвета:
/// биты те же, поток решений не меняется (`determinism`).
const BLOOD_STEPS: u64 = 4;
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
/// Зум (метров на пиксель), дальше которого растекание крови не рисуется:
/// рост лужи укладывается в полтора пикселя, и лужа встаёт полной разом,
/// а труп сразу в очередь запекания (директива владельца в ходе Ф3 —
/// «анимация spread blood не нужна, если камера далеко»). Порог тот же, что
/// у прочих городских отсечек ([`SEPARATION_MAX_ZOOM`](crate::settings::
/// SEPARATION_MAX_ZOOM)), но константа своя: отсечки крутятся независимо.
const BLOOD_SPREAD_MAX_ZOOM: f32 = 0.75;

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

/// Трупы «к запеканию»: кровь доросла — пекарь (`silhouette::bake`) перепишет
/// тройку квадов в чанк и деспавнит тело. Список без сканирования: ставит
/// [`spread_blood`] тем же вызовом, каким снимает [`BloodSpread`], — триггер
/// уже существовал (Фаза 3 ×10).
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub struct BakeQueue(pub Vec<Entity>);

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
fn corpse_tint(attire: Option<&Attire>) -> Color {
    let Hsla {
        hue,
        saturation,
        lightness,
        ..
    } = attire.map_or(CORPSE_FALLBACK, |attire| attire.0).into();
    Color::hsl(hue, saturation * CORPSE_DRAIN, lightness * CORPSE_DRAIN)
}

/// Тело ложится: место смерти, глиф позы, погасшая одежда, зеркало, поворот
/// и `Z_CORPSE`. Одной командой с доступом к сущности — цвет считается из её
/// же одежды.
fn lay_down(body: &mut EntityWorldMut, pose: CorpsePose, at: Vec2) {
    let tint = corpse_tint(body.get::<Attire>());
    if let Some(mut sprite) = body.get_mut::<Sprite>() {
        sprite.color = tint;
        sprite.flip_x = pose.flip;
        set_glyph(&mut sprite, pose.glyph);
    }
    if let Some(mut transform) = body.get_mut::<Transform>() {
        transform.translation = at.extend(Z_CORPSE);
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
    let unit = |shift: u32| {
        // байт разброса — в ступень [`BLOOD_STEPS`]: четверть байта,
        // нормированная обратно в 0..1
        (((hash >> shift) & 0xff) / (256 / BLOOD_STEPS)) as f32 / (BLOOD_STEPS - 1) as f32
    };
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
/// силуэт доедет до спрайта его же системой (`size_fresh_silhouettes`) —
/// в этот же кадр: запись в [`FreshSilhouettes`] она разберёт своим прогоном,
/// который здесь `.before`-ом поставлен после растекания.
pub fn spread_blood(
    time: Res<Time>,
    viewport: Option<Res<Viewport>>,
    mut commands: Commands,
    mut pools: Query<(Entity, Option<&ChildOf>, &mut BloodSpread, &mut Silhouette)>,
    mut fresh: ResMut<FreshSilhouettes>,
    mut bake: ResMut<BakeQueue>,
) {
    // на паузе лужа стоит не только по смыслу, но и буквально: запись того же
    // значения всё равно помечает `Silhouette` изменённым, и лужа каждый кадр
    // идёт через перепись спрайта и переключение материала рендера
    if time.delta_secs() <= 0.0 {
        return;
    }
    // С городского зума анимации нет: лужа встаёт полной разом, труп сразу в
    // очередь запекания. `Viewport` нет (витрина, реплей) — анимация как была.
    if viewport.is_some_and(|view| view.zoom >= BLOOD_SPREAD_MAX_ZOOM) {
        for (entity, child_of, spread, mut silhouette) in &mut pools {
            silhouette.body = spread.full;
            fresh.0.push(entity);
            commands.entity(entity).remove::<BloodSpread>();
            if let Some(child_of) = child_of {
                bake.0.push(child_of.0);
            }
        }
        return;
    }
    for (entity, child_of, mut spread, mut silhouette) in &mut pools {
        spread.grown = (spread.grown + time.delta_secs() / SPREAD_SECS).min(1.0);
        // быстро в начале, медленно к концу: кровь бежит, пока её толкает
        let eased = 1.0 - (1.0 - spread.grown).powi(2);
        silhouette.body = spread.full * (SPREAD_START + (1.0 - SPREAD_START) * eased);
        fresh.0.push(entity);
        if spread.grown >= 1.0 {
            commands.entity(entity).remove::<BloodSpread>();
            // доросла — тело к запеканию (Фаза 3 ×10): родитель лужи. Ставит
            // тот же проход, что снимает с себя ростку работу. Лужа без
            // родителя (витрина крови показывает пятна сами по себе) — в ней
            // некому запекаться.
            if let Some(child_of) = child_of {
                bake.0.push(child_of.0);
            }
        }
    }
}

/// Три квада трупа в порядке рисования: брызги, лужа, тело. Вид не читается со
/// спрайтов, а считается теми же чистыми функциями, которые его нарисовали
/// ([`corpse_pose`], [`blood_look`], [`corpse_tint`]): поза, кровь и тон живут
/// в битах `Entity` и `Attire`, поэтому запекание не зависит от конвейера
/// спрайтов и работает без рендера. Зовёт пекарь (`silhouette::bake`). Мир —
/// глобальный трансформ: труп не двигается, и чтение его не спорит с
/// камерными писателями `Transform` за порядок в `Update`.
pub(crate) fn corpse_quads(
    entity: Entity,
    at: &GlobalTransform,
    attire: Option<&Attire>,
) -> [CorpseQuad; 3] {
    let pose = corpse_pose(entity);
    let look = blood_look(entity);
    let tint = corpse_tint(attire);
    // пятно стоит у груди — смещение в системе тела до поворота, зеркало позы
    // вместе с грудью (`stain_transform`); размер — полный, к моменту запекания
    // лужа доросла
    let stain = |ratio: f32, spin: f32, glyph: Glyph| {
        let stain = stain_transform(pose, spin, 0.0);
        let centre = at.translation().xy() + (at.rotation() * stain.translation).truncate();
        CorpseQuad {
            centre,
            half: CORPSE_SPAN * ratio * look.size / 2.0,
            floor_px: HUMAN_MIN_PX,
            tint: look.tint.to_linear().to_f32_array(),
            cell: glyph.cell() as u32,
            flip: false,
            // спрайт пятна — ребёнок тела: его мировое вращение сложено из
            // поворота тела и собственного спина, как у `stain_transform`
            angle: pose.heading + spin,
        }
    };
    [
        stain(SPATTER_RATIO, look.spatter_spin, look.spatter),
        stain(POOL_RATIO, look.pool_spin, look.pool),
        CorpseQuad {
            centre: at.translation().xy(),
            half: CORPSE_SPAN / 2.0,
            floor_px: HUMAN_MIN_PX,
            tint: tint.to_linear().to_f32_array(),
            cell: pose.glyph.cell() as u32,
            flip: pose.flip,
            angle: pose.heading,
        },
    ]
}

/// Новый мир — очередь запекания пуста: сущности прошлого не существуют, а под
/// тумблером детерминизма (пекарь стоит) список и не должен копиться от
/// рестарта к рестарту.
pub(crate) fn reset_bake_queue(_event: On<WorldStarted>, mut bake: ResMut<BakeQueue>) {
    bake.0.clear();
}

/// Человек становится трупом: поведение и движение снимаются, тело ложится.
///
/// Одна точка на весь переход, и живёт она у человека — не у демона. Обсервер
/// убийства (`demon::behavior::on_demon_caught_human`) перечислял шестнадцать
/// типов из двух чужих модулей; теперь он говорит, ЧТО случилось, а из чего
/// состоит человек и что таскает за собой движение, знают те, кому это
/// принадлежит. Как тело выглядит — поза, погасшая одежда, лужа и брызги под
/// грудью — живёт здесь же, рядом с переходом: вид трупа принадлежит
/// человеку, а не тому, кто его убил.
///
/// Что остаётся на теле намеренно: [`PawnId`](crate::rng::PawnId) и
/// `WanderIndex` — паспорт пешки, по нему труп опознаётся в отладке; `Pace` и
/// [`WanderHeading`](super::components::WanderHeading) — жребий, разыгранный
/// при спавне, читать его без `Movable` некому; `Attire` — из неё считается
/// цвет тела.
///
/// `at` — место смерти: у трупа `SimPosition` нет (её снял `strip_movement`
/// вместе с требованиями `Movable`), а `Transform` дальней пешки заморожен
/// с demote — xy тела пишет только он.
pub fn to_corpse(commands: &mut Commands, silhouettes: &Silhouettes, entity: Entity, at: Vec2) {
    crate::movement::strip_movement(commands, entity);
    // дальняя пешка умирает видимой: снимаем ярусный набор (маркеры среза
    // никто, кроме promote, не снимет) и возвращаем спрайт — свежий труп
    // рисуется как всегда, запекание — Фаза 3 плана ×10
    super::lod::remove_far(commands, entity);
    let pose = corpse_pose(entity);
    let blood = blood_look(entity);
    commands
        .entity(entity)
        .remove::<(
            Human,
            HumanWanderTag,
            HumanFleeTag,
            HumanFirstWanderTag,
            WanderPause,
            FleeRepath,
            PanicRecoil,
            // выдержка demote — людское состояние, трупу оно ни к чему
            // (ярус и срез снял `remove_far` выше)
            super::lod::LodDwell,
        )>()
        .insert((
            CorpseTag,
            Silhouette::new(Vec2::splat(CORPSE_SPAN), HUMAN_MIN_PX),
            Visibility::Visible,
        ))
        // после `remove`: снятая паника вернула бы одежду поверх цвета тела
        .queue(move |mut body: EntityWorldMut| lay_down(&mut body, pose, at))
        // порядок вставки читается как порядок событий: сперва разлетелись
        // брызги, потом на них натекла лужа. Рисование разводит их своим z
        // (`Z_POOL` выше `Z_SPATTER`), а не этим порядком
        .with_child(blood_spatter(silhouettes, pose, blood))
        .with_child(blood_pool(silhouettes, pose, blood));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::{
        BodyScale, DestinationClaim, Movable, MovableStateMovingTag, NeedsWanderTarget,
        PathfindingRequest, PreviousSimPosition, RequestedAt, RetireAt, SimPosition,
    };

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
        // тон квантован: 4 ступени тона × 4 светлоты ([`BLOOD_STEPS`]),
        // и все они должны быть в ходу
        assert_eq!(
            tints.len(),
            (BLOOD_STEPS * BLOOD_STEPS) as usize,
            "ступени тона не в полном ходу: {}",
            tints.len()
        );
    }

    /// Карточка сочетаний ключей кеша материалов рендера: тот собирает
    /// спрайты в материал по точным битам цвета и `custom_size`, а внутри
    /// бакета различает спрайты целиком — то есть и глиф атласа. Непрерывный
    /// разброс крови платил бы кешу записью на каждый труп; ступени
    /// ([`BLOOD_STEPS`]) кладут потолок: 10 глифов × 4 × 4 × 4 на слой.
    #[test]
    fn blood_cache_keys_capped_per_layer() {
        // эндшпиль записи 6: трупы только прибывают, перебор по концу шкалы
        let sample = 12_600u32;
        let mut pools = std::collections::HashSet::new();
        let mut spatters = std::collections::HashSet::new();
        for index in 0..sample {
            let look = blood_look(Entity::from_raw_u32(index).unwrap());
            // ключ рендера: биты цвета и custom_size; повороты не входят
            let color = look.tint.to_linear().to_f32_array().map(|c| c.to_bits());
            pools.insert((
                look.pool,
                color,
                (CORPSE_SPAN * POOL_RATIO * look.size).to_bits(),
            ));
            spatters.insert((
                look.spatter,
                color,
                (CORPSE_SPAN * SPATTER_RATIO * look.size).to_bits(),
            ));
        }
        let cap = (blood::POOLS as u64) * BLOOD_STEPS * BLOOD_STEPS * BLOOD_STEPS;
        assert!(
            pools.len() as u64 <= cap,
            "лужа держит {} ключей, потолок {cap}",
            pools.len()
        );
        assert!(
            spatters.len() as u64 <= cap,
            "брызги держат {} ключей, потолок {cap}",
            spatters.len()
        );
    }

    /// Лужа растекается и на этом перестаёт стоить хоть что-то.
    #[test]
    fn a_pool_spreads_to_its_full_size_and_then_stops_being_walked() {
        let mut app = App::new();
        app.add_systems(Update, spread_blood)
            .insert_resource(Time::<()>::default())
            // система теперь несёт рабочий список силуэтов — стенду он нужен
            // так же, как время, и очередь запекания, в которую она ставит
            // доросшие лужи
            .insert_resource(FreshSilhouettes::default())
            .init_resource::<BakeQueue>();
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

    /// Лужа доросла — тело встало в очередь запекания: тот же проход, что
    /// снимает `BloodSpread`, ставит родителя в `BakeQueue` (Фаза 3 ×10), и
    /// ставит его один раз.
    #[test]
    fn a_full_pool_queues_its_corpse_for_baking_once() {
        let mut app = App::new();
        app.add_systems(Update, spread_blood)
            .insert_resource(Time::<()>::default())
            .insert_resource(FreshSilhouettes::default())
            .init_resource::<BakeQueue>();
        let look = blood_look(Entity::from_raw_u32(5).unwrap());
        let body = app.world_mut().spawn(Transform::default()).id();
        let pool = app
            .world_mut()
            .spawn(blood_pool(&Silhouettes::default(), PRONE, look))
            .id();
        app.world_mut().commands().entity(body).add_child(pool);
        app.world_mut().flush();

        // до полного роста очередь молчит
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(SPREAD_SECS / 2.0));
        app.update();
        assert!(app.world().resource::<BakeQueue>().0.is_empty());

        // доросла — ровно один родитель в очереди
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(SPREAD_SECS));
        app.update();
        assert_eq!(app.world().resource::<BakeQueue>().0, vec![body]);
    }

    /// С далёкой камеры (городской зум) анимации нет: лужа полная разом,
    /// `BloodSpread` снят, тело в очереди запекания — на первом же кадре.
    #[test]
    fn a_far_camera_finishes_the_spread_at_once() {
        let mut app = App::new();
        app.add_systems(Update, spread_blood)
            .insert_resource(Time::<()>::default())
            .insert_resource(FreshSilhouettes::default())
            .init_resource::<BakeQueue>()
            .insert_resource(Viewport {
                centre: Vec2::ZERO,
                half_extent: Vec2::splat(400.0),
                zoom: BLOOD_SPREAD_MAX_ZOOM,
            });
        let look = blood_look(Entity::from_raw_u32(6).unwrap());
        let body = app.world_mut().spawn(Transform::default()).id();
        let pool = app
            .world_mut()
            .spawn(blood_pool(&Silhouettes::default(), PRONE, look))
            .id();
        app.world_mut().commands().entity(body).add_child(pool);
        app.world_mut().flush();

        let full = app
            .world()
            .get::<BloodSpread>(pool)
            .expect("лужа с ростком")
            .full;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(0.05));
        app.update();

        assert_eq!(
            app.world().get::<Silhouette>(pool).unwrap().body,
            full,
            "лужа полной разом"
        );
        assert!(
            app.world().get::<BloodSpread>(pool).is_none(),
            "росток снят — проход лужу не держит"
        );
        assert_eq!(app.world().resource::<BakeQueue>().0, vec![body]);
    }

    /// Человек в разгар паники, с полным набором рантайм-компонент: бежит по
    /// пути, держит слот назначения и ждёт ответа на новую заявку. Труп из
    /// такого — самый нагруженный из возможных.
    fn spawn_fleeing_human(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                Sprite::default(),
                Transform::default(),
                Attire(Color::hsl(200.0, 0.4, 0.5)),
                Human,
                HumanFleeTag,
                HumanFirstWanderTag,
                FleeRepath::default(),
                PanicRecoil(Some(Vec2::X)),
                WanderPause(Timer::from_seconds(1.0, TimerMode::Once)),
                Movable::new(1.0),
                MovableStateMovingTag,
                PathfindingRequest {
                    start_tile: IVec2::ZERO,
                    end_tile: IVec2::ONE,
                },
                RequestedAt(1),
                RetireAt(2),
                DestinationClaim(IVec2::ONE),
            ))
            .id()
    }

    /// То, ради чего у перехода одна точка входа: на теле не остаётся ничего,
    /// чем симуляция могла бы его повести.
    ///
    /// Перечисление здесь — спецификация, а не копия списка из [`to_corpse`]:
    /// оно называет то, чего быть НЕ должно, и потому не сходится с ним
    /// построчно. `RetireAt` в нём не случайно — «труп, держащий срок снятия
    /// таска, который никогда не настанет» уже был багом.
    #[test]
    fn a_corpse_keeps_nothing_the_simulation_could_move_it_by() {
        let mut app = App::new();
        let human = spawn_fleeing_human(&mut app);
        app.world_mut().commands().queue(move |world: &mut World| {
            let at = world.get::<SimPosition>(human).unwrap().0;
            let mut commands = world.commands();
            to_corpse(&mut commands, &Silhouettes::default(), human, at);
        });
        app.world_mut().flush();

        let corpse = app.world().entity(human);
        assert!(corpse.contains::<CorpseTag>(), "тело обязано стать трупом");
        for (name, present) in [
            ("Human", corpse.contains::<Human>()),
            ("HumanFleeTag", corpse.contains::<HumanFleeTag>()),
            (
                "HumanFirstWanderTag",
                corpse.contains::<HumanFirstWanderTag>(),
            ),
            ("FleeRepath", corpse.contains::<FleeRepath>()),
            ("PanicRecoil", corpse.contains::<PanicRecoil>()),
            ("WanderPause", corpse.contains::<WanderPause>()),
            ("Movable", corpse.contains::<Movable>()),
            (
                "MovableStateMovingTag",
                corpse.contains::<MovableStateMovingTag>(),
            ),
            (
                "PathfindingRequest",
                corpse.contains::<PathfindingRequest>(),
            ),
            ("RequestedAt", corpse.contains::<RequestedAt>()),
            ("RetireAt", corpse.contains::<RetireAt>()),
            ("DestinationClaim", corpse.contains::<DestinationClaim>()),
            // затянуты `#[require]` у `Movable` — и сняться обязаны вместе с ним
            ("SimPosition", corpse.contains::<SimPosition>()),
            ("BodyScale", corpse.contains::<BodyScale>()),
            (
                "PreviousSimPosition",
                corpse.contains::<PreviousSimPosition>(),
            ),
            ("NeedsWanderTarget", corpse.contains::<NeedsWanderTarget>()),
            // дальний ярус: труп убитого за кадром обязан вернуть видимый
            // спрайт и не таскать маркеры, которые никто не снимет
            ("LodDwell", corpse.contains::<crate::human::lod::LodDwell>()),
            ("FarTag", corpse.contains::<crate::human::lod::FarTag>()),
        ] {
            assert!(!present, "на трупе остался {name}");
        }
    }

    /// Дальняя пешка умирает там, где её убили: у трупа `SimPosition` нет
    /// (её снял `strip_movement` вместе с `#[require]` у `Movable`), а
    /// `Transform` дальнего заморожен с demote — xy тела обязан прийти из
    /// места смерти, иначе труп и запечённые квады ложатся на точку demote,
    /// за метры от места убийства.
    #[test]
    fn a_far_pawn_dies_where_it_was_killed() {
        let mut app = App::new();
        let demoted_at = Vec2::new(100.0, 100.0);
        let killed_at = Vec2::new(150.0, 90.0);
        let human = app
            .world_mut()
            .spawn((
                Transform::from_translation(demoted_at.extend(0.0)),
                Human,
                SimPosition(killed_at),
                crate::rng::PawnId(3),
            ))
            .id();
        app.world_mut()
            .entity_mut(human)
            .insert(crate::human::lod::FarTag);
        app.world_mut().flush();

        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            to_corpse(&mut commands, &Silhouettes::default(), human, killed_at);
        });
        app.world_mut().flush();

        let transform = app.world().get::<Transform>(human).unwrap();
        assert_eq!(
            transform.translation.truncate(),
            killed_at,
            "труп встаёт на место смерти, а не на точку demote"
        );
        assert_eq!(transform.translation.z, Z_CORPSE);
    }

    /// Тело лежит: вид трупа принадлежит человеку, и переход его меняет —
    /// погасшая одежда поверх снятой паники, поза по битам сущности, лужа под
    /// телом.
    #[test]
    fn a_corpse_lies_down_under_everything_that_walks() {
        let mut app = App::new();
        app.add_observer(crate::human::look::on_calm_tint);
        let human = spawn_fleeing_human(&mut app);
        app.world_mut().commands().queue(move |world: &mut World| {
            let at = world.get::<SimPosition>(human).unwrap().0;
            let mut commands = world.commands();
            to_corpse(&mut commands, &Silhouettes::default(), human, at);
        });
        app.world_mut().flush();

        let corpse = app.world().entity(human);
        let attire = corpse.get::<Attire>().expect("одежда остаётся на теле");
        let sprite = corpse.get::<Sprite>().expect("Sprite");
        assert_eq!(sprite.color, corpse_tint(Some(attire)));
        // размер трупа приходит через `Silhouette` — его пишут системы
        // `silhouette/`, а не `lay_down`
        assert_eq!(
            corpse.get::<crate::silhouette::Silhouette>().copied(),
            Some(crate::silhouette::Silhouette::new(
                Vec2::splat(CORPSE_SPAN),
                HUMAN_MIN_PX,
            ))
        );
        let pose = corpse_pose(human);
        assert_eq!(sprite.flip_x, pose.flip);
        let transform = corpse.get::<Transform>().expect("Transform");
        assert_eq!(transform.translation.z, Z_CORPSE);
        assert_eq!(transform.rotation, Quat::from_rotation_z(pose.heading));

        // кровь — две дочерние сущности тела: брызги легли разом, лужа ещё
        // натекает (`spread_blood`)
        let children = corpse.get::<Children>().expect("кровь — дети тела");
        assert_eq!(children.len(), 2);
        let has = |index: usize| {
            let child = app.world().entity(children[index]);
            (
                child.contains::<BloodSpatter>(),
                child.contains::<BloodPool>(),
            )
        };
        assert_eq!(has(0), (true, false), "первыми ложатся брызги");
        assert_eq!(has(1), (false, true), "лужа натекает поверх них");
    }
}
