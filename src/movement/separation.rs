//! Мягкое расталкивание пешек (anti-overlap): пешки в кадре не стоят друг на
//! друге. Пути идут через центры навтайлов, спавн и паника сгоняют толпу в
//! одну точку — и без «личного пространства» пешки регулярно сливаются в одну.
//!
//! Механизм намеренно локальный и косметический:
//! - **только вьюпорт и близкий зум** — за кадром и на отдалении перекрытие не
//!   видно, и симуляция за него не платит; пачка, въехавшая в кадр при движении
//!   камеры, расходится на глазах за доли секунды;
//! - **не чаще раза в кадр** — система живёт в `FixedUpdate` (ей нужен момент
//!   после `move_moving_entities`), но на 30x там ~1920 тиков в секунду, и
//!   даже 0.03 мс на тик съели бы ~6% реальной секунды. Чаще кадра
//!   расталкивание физически не видно, а на скорости 1x кадр ≈ тик, так что в
//!   режиме «медленно разглядываю толпу» гейт ничего не отнимает;
//! - **мягкое** — за прогон разрешается доля перекрытия ([`SEPARATION_RATE`],
//!   кламп [`SEPARATION_MAX_STEP`]): в давке мгновенное перекрытие возможно,
//!   но живёт доли секунды. Жёсткая релаксация до полного разведения в той же
//!   давке разлеталась бы цепными толчками, как телепорт.
//!
//! Одно намеренное исключение из «косметического»: пешка, чей курс упирается в
//! перекрытого соседа, до следующего прогона ходит ослабленным шагом
//! ([`SeparationHolds`], читает `move_moving_entities`) — иначе ходьба и
//! расталкивание гасят друг друга, и затор стоит вечно. В детерминированном
//! режиме набор придержанных пуст вместе со всем механизмом.
//!
//! Толчок пишется в `SimPosition` после шага движения: снимок
//! `PreviousSimPosition` сделан в начале тика, так что интерполяция доводит
//! сдвиг до экрана плавно, а троттлимая перепрокладка путей (0.4–1.2 с) съедает
//! боковой дрейф — накапливаться ему не во что. Демон в броске
//! (`DemonLungeTag`) исключён целиком: он двигает `SimPosition` сам и обязан
//! сомкнуться до `KILL_DISTANCE`. Пожирающий (`DemonDevourTag`) стоит над
//! трупом неподвижно (mobility 0), но толпу от себя отталкивает. Трупы вне
//! механизма по построению — у них нет `SimPosition`.

use bevy::diagnostic::FrameCount;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::window::PrimaryWindow;

use crate::demon::{Demon, DemonDevourTag, DemonLungeTag};
use crate::grid::world_to_tile;
use crate::human::Human;
use crate::movement::components::SimPosition;
use crate::movement::systems::VIEW_MARGIN;
use crate::settings::{
    DEMON_BODY_RADIUS, HUMAN_BODY_RADIUS, SEPARATION_BACKSTEP, SEPARATION_CELL, SEPARATION_HOLD,
    SEPARATION_MAX_STEP, SEPARATION_MAX_ZOOM, SEPARATION_RATE, SEPARATION_SIDESTEP,
};
use crate::spatial::SpatialGrid;

/// Подвижность демона относительно человека: в паре человек забирает 4/5
/// коррекции — толпа обтекает демона, а не демон толпу.
const DEMON_MOBILITY: f32 = 0.25;

/// «Конец цепочки» в связных списках мелкой сетки.
const NONE: u32 = u32::MAX;

/// Тумблер расталкивания — панель World. Выбор пользователя, переживает
/// рестарт и смену города (тот же контракт, что у `DemonStyle`). Выключение
/// ничего не откатывает: уже разведённые позиции просто остаются как есть.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "separation")]
pub struct SeparationStyle {
    pub enabled: bool,
    /// Сила обхода встречного в долях продольной коррекции (см. [`sidestep`]).
    /// Ручка, а не константа, потому что у неё узкое окно между двумя разными
    /// поломками — ниже слипание, выше карусель — и подбирается она только
    /// замером в толпе (`examples/demos/crowd_demo.rs`).
    pub sidestep: f32,
    /// Какая доля продольного толчка доживает до применения (см.
    /// [`damp_along_heading`]). 1 — ничего не давим, и догоняющий в очереди
    /// заметно пятится; 0 — толчок строго поперечный, и толпа, идущая в одну
    /// точку, схлопывается в пятно и вращается вместо того, чтобы раздаться.
    pub backstep: f32,
    /// Какая доля шага ходьбы остаётся у придержанной пешки (см.
    /// [`SeparationHolds`]): 1 — придержки нет и ходьба давит в упор до
    /// равновесия, 0 — полный стоп и рваное движение толпы. Дефолт и разбор
    /// компромисса — [`SEPARATION_HOLD`].
    pub hold: f32,
}

/// Пешки, упёршиеся курсом в перекрытого соседа, который СТОИТ или ИДЁТ
/// НАВСТРЕЧУ: до следующего прогона их шаг ходьбы ослаблен долей
/// [`SeparationStyle::hold`] (`systems::move_moving_entities`). Ровно эти два
/// случая, а не любое касание: попутный и поперечный сосед проходятся полным
/// шагом — их разводит само расталкивание, а придержка за касание заставляла
/// ползти весь поток (группа попутчиков душила сама себя).
///
/// Это разрыв равновесия «ходьба против расталкивания» — источника сразу двух
/// картинок: кучи, вставшей колом и дрожащей на месте (все давят внутрь, и
/// толчки в точности гасят шаги), и пары, наматывающей круги друг вокруг
/// друга (обоих везёт вперёд собственная ходьба, а боковой обход только
/// вращает связку). Пешка, упёршаяся в тело, перестаёт с ним бороться — и
/// расталкиванию хватает одного-двух прогонов развести затор.
///
/// Придерживаются только люди: демону в погоне смыкаться обязательно, а
/// «толпа обтекает демона» уже выражено подвижностью.
///
/// Наполняется только прогоном расталкивания, то есть под его гейтами —
/// вьюпорт, зум, недетерминированный режим; выключение и отзум чистят набор.
/// В детерминированном прогоне набор пуст с входа в мир, и движение от него
/// не зависит.
#[derive(Resource, Default)]
pub struct SeparationHolds(pub bevy::ecs::entity::EntityHashSet);

/// Вход в мир начинается без придержанных: набор пережил бы смену города
/// (сущности в нём уже мертвы) и рестарт с переключением режима.
pub fn reset_separation_holds(mut holds: ResMut<SeparationHolds>) {
    holds.0.clear();
}

/// Во столько раз радиус демона больше человеческого — как и спрайты
/// ([`DEMON_SIZE`] против [`HUMAN_SIZE`]).
pub const DEMON_RADIUS_RATIO: f32 = DEMON_BODY_RADIUS / HUMAN_BODY_RADIUS;

/// Радиус тела демона по радиусу тела человека — не отдельная ручка: он всегда
/// вдвое больше, как и спрайт.
pub fn demon_radius(human_radius: f32) -> f32 {
    human_radius * DEMON_RADIUS_RATIO
}

/// Сторона одноразовой мелкой сетки соседей. Считается от радиуса, а не
/// берётся константой: ячейка ОБЯЗАНА быть не меньше максимальной суммы
/// радиусов (демон+демон), иначе перекрывшаяся пара не попадёт в общие
/// 3 × 3 ячейки и её не найдут. С ручкой радиуса константа рано или поздно
/// оказалась бы мала.
pub fn separation_cell(human_radius: f32) -> f32 {
    (demon_radius(human_radius) * 2.0).max(SEPARATION_CELL)
}

impl Default for SeparationStyle {
    fn default() -> Self {
        Self {
            enabled: true,
            sidestep: SEPARATION_SIDESTEP,
            backstep: SEPARATION_BACKSTEP,
            hold: SEPARATION_HOLD,
        }
    }
}

impl Eq for SeparationStyle {}

/// Участник одного прогона: снятая позиция плюс всё, что нужно паре.
#[derive(Clone, Copy)]
struct Pawn {
    entity: Entity,
    /// Личный номер пешки — ось разведения совпавших позиций (см.
    /// [`coincident_direction`]).
    pawn_id: u32,
    position: Vec2,
    radius: f32,
    /// Доля коррекции пары, пропорциональная подвижности: человек 1.0, демон
    /// [`DEMON_MOBILITY`], пожирающий 0.0 (толкает, но не двигается).
    mobility: f32,
    /// Куда пешка идёт прямо сейчас, единичный; `ZERO` у стоящей. Нужен
    /// обходу встречного (см. [`sidestep`]) и придержке ([`SeparationHolds`]).
    heading: Vec2,
    /// Придерживают только людей, см. [`SeparationHolds`].
    human: bool,
}

/// Обход встречного: боковая добавка к толчку, чтобы двое идущих ЛОБ В ЛОБ
/// могли разойтись.
///
/// Без неё они расходиться не могут в принципе. Коррекция пары идёт строго
/// вдоль отрезка между центрами, а у лобовой встречи этот отрезок совпадает с
/// вектором движения обоих — то есть каждого толкает ровно НАЗАД по его же
/// пути, а на следующем тике движение снова закрывает зазор. Пара «слипается»
/// и топчется, пока симметрию не сломает что-то постороннее: разброс скоростей
/// (`Pace`), третья пешка рядом, смена waypoint'а. Боковой составляющей у
/// толчка нет ни в каком виде, поэтому шага в сторону не происходит никогда.
///
/// Добавка тем сильнее, чем более лобовая встреча (`frontal` — косинус между
/// курсом и направлением на соседа). Сторона — всегда ПРАВАЯ относительно
/// собственного курса, как у живых пешеходов: обход предсказуемый, а не в
/// случайную сторону. Величина — доля [`SEPARATION_SIDESTEP`] от продольной
/// коррекции той же пары.
///
/// Курс берётся у идущих; у стоящей пешки он `ZERO`, и добавки нет — стоящего
/// незачем «обходить», его достаточно раздвинуть.
///
/// Кому она достаётся — решают [`yields`] и «пара одна» на месте вызова; оба
/// ограничения существуют затем, чтобы обход не превращался во вращение, см.
/// их доки.
fn sidestep(heading: Vec2, to_other: Vec2, correction: f32, strength: f32) -> Vec2 {
    let frontal = heading.dot(to_other);
    if frontal <= 0.0 {
        return Vec2::ZERO;
    }
    // правая нормаль к курсу: `perp` в bevy — поворот на +90° (влево)
    -heading.perp() * (correction * strength * frontal)
}

/// Кто из встречной пары обходит: обходит РОВНО ОДИН.
///
/// Если вправо уходят оба, обход не работает совсем — он вырождается в
/// вращение. Курсы у встречных противоположны, поэтому «вправо от себя» у них
/// в мировых координатах противоположно, и две одинаковые по величине боковые
/// добавки складываются в пару сил: связка проворачивается как твёрдое тело,
/// а ВЗАИМНАЯ геометрия — расстояние и угол между курсом и осью пары — остаётся
/// прежней. Пока контроллер пути держит курс на цель, лежащую за соседом (в
/// «воронке» цель у всех одна точка), конфигурация воспроизводится каждый кадр:
/// пара наматывает круги вокруг общего центра и не расходится никогда. Именно
/// это и видно на экране как «слиплись и ходят по кругу».
///
/// Обходит один — и инварианта нет: ось пары поворачивается вокруг того, кто
/// держит линию, `frontal` у обходящего падает, продольная коррекция получает
/// боковую составляющую, и пара разъезжается. Ровно так расходятся живые
/// пешеходы — в сторону уходит кто-то один, а не оба сразу.
///
/// Уступает более подвижный: у пожирающего демона (mobility 0) уступать нечем,
/// и в паре «человек — демон» обходит человек. При равной подвижности —
/// старший [`PawnId`], лишь бы выбор был устойчив от прогона к прогону (по
/// `PawnId`, а не по `Entity`, по той же причине, что в
/// [`coincident_direction`]).
fn yields(a: &Pawn, b: &Pawn) -> bool {
    if a.mobility != b.mobility {
        return a.mobility > b.mobility;
    }
    a.pawn_id > b.pawn_id
}

/// Доли коррекции пары. По умолчанию — по подвижности (человек 1.0, демон
/// [`DEMON_MOBILITY`], пожирающий 0.0), но у пары, идущей ПРИМЕРНО ОДНИМ
/// КУРСОМ, всю коррекцию забирает ЗАДНИЙ, а переднего не трогает никто.
///
/// Иначе догоняющий расталкивает того, кого догнал: передний, ничего не
/// сделав, получает толчок в спину и сходит с линии, по которой шёл. На экране
/// это читается как «его пихнули», и виноват всегда не тот. В жизни уступает
/// тот, кто пришёл вторым, — он и обходит.
///
/// Кто задний, видно по курсу: если сосед у пешки СПЕРЕДИ (курс смотрит в его
/// сторону), значит эта пешка — догоняющая. Уступать может только подвижный:
/// у пожирающего демона (mobility 0) забирать нечего, и пара остаётся на
/// долях по подвижности.
fn shares(a: &Pawn, b: &Pawn, direction: Vec2) -> (f32, f32) {
    let weights = a.mobility + b.mobility;
    let by_mobility = (a.mobility / weights, b.mobility / weights);

    let aligned =
        a.heading != Vec2::ZERO && b.heading != Vec2::ZERO && a.heading.dot(b.heading) > 0.0;
    if !aligned {
        return by_mobility;
    }
    // `direction` смотрит от a к b: положительный dot — b впереди a
    if a.heading.dot(direction) > 0.0 && a.mobility > 0.0 {
        (1.0, 0.0)
    } else if b.heading.dot(-direction) > 0.0 && b.mobility > 0.0 {
        (0.0, 1.0)
    } else {
        by_mobility
    }
}

/// Придавить составляющую толчка ВДОЛЬ собственного курса пешки в `keep` раз;
/// поперечная остаётся целиком. У стоящей курса нет — её толчок не трогаем.
///
/// Ручка между двумя противоположными поломками, и обе видны глазом.
///
/// `keep` = 1 (ничего не давим) — продольный толчок цел, и в очереди
/// догоняющего отбрасывает НАЗАД по его же пути: на экране пешка
/// разворачивается, отходит и снова идёт вперёд, хотя ничего не решала.
///
/// `keep` = 0 (давим насмерть) — толчок становится строго поперечным, и тогда
/// толпа, идущая В ОДНУ ТОЧКУ, не может расшириться ВООБЩЕ: наружу — это назад
/// по курсу, а назад запрещено. Вся коррекция уходит в касательную, и затор не
/// рассасывается, а начинает вращаться вокруг общего центра. Ровно это и
/// случилось на сценарии «воронка»: 200 пешек в одном пятне, все 200 в
/// перекрытии.
///
/// Поэтому середина: продольная часть ослаблена (реверс в очереди перестаёт
/// бросаться в глаза), но не обнулена (сходящаяся толпа всё ещё умеет
/// раздаться). Подбирается ползунком на живой толпе, как и остальные.
fn damp_along_heading(push: Vec2, heading: Vec2, keep: f32) -> Vec2 {
    if heading == Vec2::ZERO {
        return push;
    }
    let along = heading * push.dot(heading);
    push - along * (1.0 - keep)
}

/// Буферы прогона — в `Local`, чтобы steady state обходился без аллокаций.
/// `pub(super)` — тип стоит в сигнатуре системы, которую регистрирует `mod.rs`.
#[derive(Default)]
pub(super) struct SeparationState {
    /// Кадр последнего прогона: гейт «не чаще раза в кадр».
    last_frame: Option<u32>,
    /// Виртуальное время, накопленное тиками с последнего прогона.
    pending_dt: f32,
    pawns: Vec<Pawn>,
    /// Мелкая сетка соседей: голова цепочки по ячейке + связный список по
    /// индексам `pawns` — ни одного `Vec` на ячейку.
    heads: HashMap<IVec2, u32>,
    next: Vec<u32>,
    /// Перекрывшиеся пары этого прогона, каждая по разу. Собираются отдельным
    /// проходом, потому что толчок пары зависит от того, сколько соседей у её
    /// участников (см. `contacts`), а это известно только когда найдены все.
    pairs: Vec<(u32, u32)>,
    /// Сколько перекрытий у каждой пешки в этом прогоне.
    contacts: Vec<u32>,
    /// Кто в этом прогоне упирался курсом в перекрытого соседа — источник
    /// [`SeparationHolds`].
    held: Vec<bool>,
    pushes: Vec<Vec2>,
}

fn fine_cell(pos: Vec2, cell: f32) -> IVec2 {
    (pos / cell).floor().as_ivec2()
}

/// Детерминированное направление разведения точно совпавших позиций — хэш
/// [`PawnId`], тот же трюк, что `personal_spread` у веера бегства: пара держит
/// свою ось от прогона к прогону, а не дрожит случайной.
///
/// По `PawnId`, а не по `Entity`, по той же причине, что и там: индексы
/// сущностей после рестарта переиспользуются в другом порядке.
fn coincident_direction(pawn_id: u32) -> Vec2 {
    let hash = pawn_id.wrapping_mul(2654435761);
    let angle = (hash >> 8) as f32 / (u32::MAX >> 8) as f32 * std::f32::consts::TAU;
    Vec2::from_angle(angle)
}

/// Толчки всех пар текущего набора — чистая функция над буферами, без ECS
/// (тестируется напрямую). `fraction` — доля перекрытия, разрешаемая этим
/// прогоном; в `pushes` — суммарный сдвиг каждой пешки, ещё без клампа.
/// Ручки одного прогона: доля перекрытия, разрешаемая к снятию, плюс то, что
/// задаёт [`SeparationStyle`]. Отдельной структурой, чтобы у чистой функции не
/// росла череда безымянных `f32`.
#[derive(Clone, Copy)]
struct Tuning {
    fraction: f32,
    sidestep: f32,
    cell: f32,
}

fn resolve_pushes(state: &mut SeparationState, tuning: Tuning) {
    let Tuning {
        fraction,
        sidestep: sidestep_strength,
        cell: cell_size,
    } = tuning;
    state.heads.clear();
    state.next.clear();
    state.pairs.clear();
    state.contacts.clear();
    state.contacts.resize(state.pawns.len(), 0);
    state.held.clear();
    state.held.resize(state.pawns.len(), false);
    state.pushes.clear();
    state.pushes.resize(state.pawns.len(), Vec2::ZERO);
    for (i, pawn) in state.pawns.iter().enumerate() {
        let head = state
            .heads
            .insert(fine_cell(pawn.position, cell_size), i as u32);
        state.next.push(head.unwrap_or(NONE));
    }

    // проход 1 — кто с кем перекрылся и сколько соседей у каждого
    for i in 0..state.pawns.len() {
        let a = state.pawns[i];
        let cell = fine_cell(a.position, cell_size);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(&head) = state.heads.get(&(cell + IVec2::new(dx, dy))) else {
                    continue;
                };
                let mut j = head;
                while j != NONE {
                    // пара встречается с обеих сторон — обрабатываем один раз
                    if j > i as u32 {
                        let b = state.pawns[j as usize];
                        let min_distance = a.radius + b.radius;
                        let overlapping = (b.position - a.position).length_squared()
                            < min_distance * min_distance;
                        if overlapping && a.mobility + b.mobility > 0.0 {
                            state.pairs.push((i as u32, j));
                            state.contacts[i] += 1;
                            state.contacts[j as usize] += 1;
                        }
                    }
                    j = state.next[j as usize];
                }
            }
        }
    }

    // проход 2 — толчки
    for pair in 0..state.pairs.len() {
        let (i, j) = state.pairs[pair];
        let (a, b) = (state.pawns[i as usize], state.pawns[j as usize]);
        let min_distance = a.radius + b.radius;
        let offset = b.position - a.position;
        let distance = offset.length();
        let direction = if distance > 1e-4 {
            offset / distance
        } else {
            coincident_direction(a.pawn_id)
        };
        let overlap = (min_distance - distance) * fraction;
        let correction = direction * overlap;
        let (share_a, share_b) = shares(&a, &b, direction);
        let a_frontal = a.heading.dot(direction) > 0.0;
        let b_frontal = b.heading.dot(-direction) > 0.0;
        // Придержка — только упёршемуся в СТОЯЩЕГО или ВСТРЕЧНОГО: там напор
        // бесполезен — ходьба давит ровно против коррекции этой же пары, и оба
        // усилия взаимно гасятся навечно (см. [`SeparationHolds`]). Попутного
        // и поперечного соседа это не касается: первый вариант правила
        // придерживал за любое касание, и поток целиком полз на доле шага —
        // группа попутчиков душила сама себя.
        if a.human && a_frontal && (b.heading == Vec2::ZERO || b_frontal) {
            state.held[i as usize] = true;
        }
        if b.human && b_frontal && (a.heading == Vec2::ZERO || a_frontal) {
            state.held[j as usize] = true;
        }
        // обход — только встречным: догоняющего сзади ничто не держит, а
        // боковая добавка расползлась бы по всей очереди, идущей в одну сторону
        let opposed = a_frontal && b_frontal;
        // …и только паре, у которой нет других соседей. Обход разводит ДВОИХ,
        // симметрию которых больше сломать некому; в куче симметрию ломает сама
        // многотельная геометрия, а одинаковый разворот вправо у полусотни
        // перекрывшихся складывается в общее вращение — затор начинает крутиться
        // вместо того, чтобы рассасываться.
        let alone = state.contacts[i as usize] == 1 && state.contacts[j as usize] == 1;
        let (side_a, side_b) = if opposed && alone {
            // уступает один: вправо уходят оба — и пара вращается, см. [`yields`]
            if yields(&a, &b) {
                (
                    sidestep(a.heading, direction, overlap * share_a, sidestep_strength),
                    Vec2::ZERO,
                )
            } else {
                (
                    Vec2::ZERO,
                    sidestep(b.heading, -direction, overlap * share_b, sidestep_strength),
                )
            }
        } else {
            (Vec2::ZERO, Vec2::ZERO)
        };
        state.pushes[i as usize] -= correction * share_a - side_a;
        state.pushes[j as usize] += correction * share_b + side_b;
    }
}

/// Прогон расталкивания: гейты → сбор видимых из грубых сеток → толчки →
/// применение с проверкой проходимости. Порядок в тике — строго после
/// `move_moving_entities` (см. цепочку в `movement/mod.rs`).
#[allow(clippy::too_many_arguments)]
pub fn separate_pawns(
    mut diagnostics: bevy::diagnostic::Diagnostics,
    style: Res<SeparationStyle>,
    // радиус тела живёт в настройках ЛЮДЕЙ, а не расталкивания: это свойство
    // тела, а расталкивание — лишь один из его читателей (второй — слоты
    // назначения, которые работают и когда расталкивание выключено)
    human_style: Res<crate::human::HumanStyle>,
    frames: Res<FrameCount>,
    time: Res<Time>,
    navmesh: Res<crate::navigation::ArcNavmesh>,
    mut humans: ResMut<SpatialGrid<Human>>,
    demons: Res<SpatialGrid<Demon>>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut holds: ResMut<SeparationHolds>,
    mut pawns: Query<
        (
            &mut SimPosition,
            &crate::rng::PawnId,
            &crate::movement::Movable,
            Has<crate::movement::MovableStateMovingTag>,
            Has<Demon>,
            Has<DemonDevourTag>,
        ),
        (Or<(With<Human>, With<Demon>)>, Without<DemonLungeTag>),
    >,
    mut state: Local<SeparationState>,
) {
    if !style.enabled {
        state.pending_dt = 0.0;
        holds.0.clear();
        return;
    }
    state.pending_dt += time.delta_secs();
    // не чаще раза в кадр: остальные тики того же кадра только копят dt, а
    // придержанные остаются придержанными до следующего прогона
    if state.last_frame == Some(frames.0) {
        return;
    }
    state.last_frame = Some(frames.0);
    let fraction = (SEPARATION_RATE * state.pending_dt).min(1.0);
    state.pending_dt = 0.0;
    // на таком отдалении пешка — 1–2 пикселя, перекрытие не читается; вместе
    // с расталкиванием выключается и придержка — иначе пешки, придержанные
    // последним прогоном перед отзумом, остались бы придержанными навсегда
    if camera.scale.x >= SEPARATION_MAX_ZOOM {
        holds.0.clear();
        return;
    }

    let started = std::time::Instant::now();
    let camera_position = camera.translation.truncate();
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * VIEW_MARGIN;
    let min = camera_position - half_view;
    let max = camera_position + half_view;

    let human_radius = human_style.body_radius;
    let demon_radius = demon_radius(human_radius);

    state.pawns.clear();
    {
        let pawn_buffer = &mut state.pawns;
        let mut collect = |entity: Entity| {
            // мимо запроса — бросок, труп, пешка чужого вида в чужой сетке
            let Ok((sim_position, pawn_id, movable, is_moving, is_demon, is_devouring)) =
                pawns.get(entity)
            else {
                return;
            };
            let position = sim_position.0;
            if position.x < min.x || position.x > max.x || position.y < min.y || position.y > max.y
            {
                return;
            }
            let (radius, mobility) = if is_devouring {
                (demon_radius, 0.0)
            } else if is_demon {
                (demon_radius, DEMON_MOBILITY)
            } else {
                (human_radius, 1.0)
            };
            pawn_buffer.push(Pawn {
                entity,
                pawn_id: pawn_id.0,
                position,
                radius,
                mobility,
                // у стоящей курс не берём: `last_direction` у неё остался от
                // прошлой ходьбы и увёл бы обход в сторону, куда она уже не идёт
                heading: if is_moving {
                    movable.last_direction.normalize_or_zero()
                } else {
                    Vec2::ZERO
                },
                human: !is_demon,
            });
        };
        humans.for_each_in_rect(min, max, &mut collect);
        demons.for_each_in_rect(min, max, &mut collect);
    }

    resolve_pushes(
        &mut state,
        Tuning {
            fraction,
            sidestep: style.sidestep,
            cell: separation_cell(human_radius),
        },
    );

    // придержанные — с чистого листа каждый прогон: ушедший из вьюпорта или
    // разошедшийся с соседом освобождается сам, без отдельной уборки
    holds.0.clear();
    for (index, pawn) in state.pawns.iter().enumerate() {
        if state.held[index] {
            holds.0.insert(pawn.entity);
        }
    }

    let navmesh = navmesh.read();
    for i in 0..state.pawns.len() {
        let pawn = state.pawns[i];
        let push = damp_along_heading(state.pushes[i], pawn.heading, style.backstep);
        if push == Vec2::ZERO {
            continue;
        }
        let target = pawn.position + push.clamp_length_max(SEPARATION_MAX_STEP);
        let tile = world_to_tile(target);
        // толчок в непроходимое отбрасывается: спасение (`rescue_*`) ловит
        // только провал поиска пути, задавленную в стену пешку оно бы не нашло
        if !navmesh.is_passable(tile.x, tile.y) {
            continue;
        }
        let Ok((mut sim_position, _, _, _, is_demon, _)) = pawns.get_mut(pawn.entity) else {
            continue;
        };
        sim_position.0 = target;
        // сетка людей инкрементальна; толчки мелкие, но стоячую пешку они
        // могут за много прогонов увести через границу 60-метровой ячейки
        if !is_demon && crate::spatial::cell_of(target) != crate::spatial::cell_of(pawn.position) {
            humans.insert(pawn.entity, target);
        }
    }
    crate::diagnostics::measure_ms(
        &mut diagnostics,
        &crate::diagnostics::SIM_SEPARATION_MS,
        started,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    /// Стоящая пешка: без курса обход встречного не включается, и тесты
    /// продольной коррекции меряют её одну.
    fn pawn(index: u32, position: Vec2, radius: f32, mobility: f32) -> Pawn {
        Pawn {
            entity: entity(index),
            pawn_id: index,
            position,
            radius,
            mobility,
            heading: Vec2::ZERO,
            human: true,
        }
    }

    fn walking(index: u32, position: Vec2, heading: Vec2) -> Pawn {
        Pawn {
            heading,
            ..pawn(index, position, 0.45, 1.0)
        }
    }

    /// Ручки по умолчанию: тесты меряют механику, а не подобранные значения.
    fn tuning(fraction: f32) -> Tuning {
        Tuning {
            fraction,
            sidestep: SEPARATION_SIDESTEP,
            cell: SEPARATION_CELL,
        }
    }

    fn state_with(pawns: Vec<Pawn>) -> SeparationState {
        SeparationState {
            pawns,
            ..Default::default()
        }
    }

    /// Перекрытая пара расходится в противоположные стороны вдоль своей оси,
    /// ровно на долю перекрытия.
    #[test]
    fn an_overlapping_pair_is_pushed_apart() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        // перекрытие 0.4 м делится пополам: каждому по 0.2 вдоль оси пары
        assert!((state.pushes[0] - Vec2::new(-0.2, 0.0)).length() < 1e-4);
        assert!((state.pushes[1] - Vec2::new(0.2, 0.0)).length() < 1e-4);
    }

    /// Пара на дистанции суммы радиусов — уже не перекрытие: толчков нет,
    /// разведённая толпа не продолжает расползаться.
    #[test]
    fn a_settled_pair_is_left_alone() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.95, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert_eq!(state.pushes[0], Vec2::ZERO);
        assert_eq!(state.pushes[1], Vec2::ZERO);
    }

    /// Точно совпавшие позиции разводятся по детерминированной оси, а не
    /// остаются на месте с нулевым направлением.
    #[test]
    fn coincident_pawns_get_a_deterministic_axis() {
        let position = Vec2::new(10.0, 10.0);
        let mut state = state_with(vec![
            pawn(1, position, 0.45, 1.0),
            pawn(2, position, 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.pushes[0].length() > 1e-4);
        assert!((state.pushes[0] + state.pushes[1]).length() < 1e-4);

        // тот же набор — та же ось: направление не дрожит от прогона к прогону
        let first = state.pushes[0];
        resolve_pushes(&mut state, tuning(1.0));
        assert!((state.pushes[0] - first).length() < 1e-6);
    }

    /// Неподвижный участник (пожирающий демон) толкает, но не двигается:
    /// вся коррекция достаётся подвижному.
    #[test]
    fn an_immovable_pawn_pushes_without_moving() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.9, 0.0),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert_eq!(state.pushes[0], Vec2::ZERO);
        let expected = 0.45 + 0.9 - 0.5;
        assert!((state.pushes[1] - Vec2::new(expected, 0.0)).length() < 1e-4);
    }

    /// Идущие ЛОБ В ЛОБ получают боковую составляющую — иначе их толкает
    /// строго назад по собственному пути, и разойтись они не могут вообще.
    ///
    /// Обходит РОВНО ОДИН (здесь — старший `PawnId`): вправо от себя у
    /// встречных — противоположные стороны мира, и две одинаковые добавки дали
    /// бы паре сил вместо обхода (см. [`yields`]).
    #[test]
    fn one_of_a_head_on_pair_steps_aside() {
        let mut state = state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        // продольная часть прежняя: каждого назад по своему курсу
        assert!(state.pushes[0].x < 0.0);
        assert!(state.pushes[1].x > 0.0);
        // в сторону уходит только уступающий, и вправо ОТ СЕБЯ: идущий на −X — в +Y
        assert_eq!(state.pushes[0].y, 0.0, "{:?}", state.pushes[0]);
        assert!(state.pushes[1].y > 0.0, "{:?}", state.pushes[1]);
    }

    /// Обход — только паре, которая одна: у встречных в куче он выключен.
    ///
    /// Одинаковый разворот вправо у всех перекрывшихся складывается в общее
    /// вращение, и затор начинает крутиться вместо того, чтобы рассасываться.
    #[test]
    fn a_head_on_pair_with_a_third_pawn_nearby_does_not_step_aside() {
        let mut state = state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
            // третий висит на встречном сзади — пара больше не одна
            walking(3, Vec2::new(11.2, 10.0), Vec2::NEG_X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        // толкает по-прежнему всех, но строго вдоль оси: боковой добавки нет
        assert!(state.pushes.iter().all(|push| push.x != 0.0));
        assert!(state.pushes.iter().all(|push| push.y == 0.0));
    }

    /// Идущие одним курсом: всю коррекцию забирает ЗАДНИЙ, переднего не
    /// трогает никто — его не за что толкать в спину.
    #[test]
    fn in_a_queue_only_the_pawn_behind_gives_way() {
        let mut state = state_with(vec![
            // догоняющий — первый: сосед у него спереди
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert_eq!(state.pushes[1], Vec2::ZERO, "переднего толкать нельзя");
        assert!(state.pushes[0].length() > 0.0);
        // и обход в сторону догоняющему тоже не полагается — он не встречный
        assert!(state.pushes[0].y.abs() < 1e-6, "{:?}", state.pushes[0]);
    }

    /// Придержан упёршийся в СТОЯЩЕГО: давить в того, кто не сдвинется с
    /// места сам, бесполезно. Стоящего не придерживают — у него нет курса.
    #[test]
    fn a_pawn_walking_into_a_standing_neighbour_is_held() {
        let mut state = state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.held[0], "упёршийся в стоящего придержан");
        assert!(!state.held[1], "стоящий не придержан");
    }

    /// Попутчиков не придерживают: очередь, идущая в одну сторону, проходится
    /// полным шагом — иначе поток целиком ползёт на доле скорости от любого
    /// касания. Догоняющего осаживает не придержка, а доля коррекции
    /// ([`shares`]: задний забирает её всю).
    #[test]
    fn a_queue_walking_the_same_way_is_not_held() {
        let mut state = state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.held.iter().all(|held| !held));
    }

    /// Лоб в лоб упираются оба — придержаны оба: без этого чей-то шаг
    /// продолжает гасить коррекцию пары, и равновесие лишь сдвигается.
    #[test]
    fn both_of_a_head_on_pair_are_held() {
        let mut state = state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.held[0] && state.held[1]);
    }

    /// Стоящих не придерживают (курса нет), разошедшихся — тоже: придержка
    /// живёт ровно столько же, сколько само перекрытие.
    #[test]
    fn standing_and_settled_pawns_are_not_held() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
            walking(3, Vec2::new(20.0, 10.0), Vec2::X),
            walking(4, Vec2::new(20.95, 10.0), Vec2::NEG_X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.held.iter().all(|held| !held));
    }

    /// Демона не придерживают никогда: погоня обязана смыкаться, а «толпа
    /// обтекает демона» уже выражено подвижностью. Человек навстречу демону
    /// придержан как обычно.
    #[test]
    fn a_demon_is_never_held() {
        let mut state = state_with(vec![
            Pawn {
                human: false,
                ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
            },
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(!state.held[0], "демон прёт сквозь толпу");
        assert!(state.held[1], "человек навстречу демону придержан");
    }

    /// Соседи через границу ячейки мелкой сетки видят друг друга: пара на
    /// стыке двух ячеек — всё ещё пара.
    #[test]
    fn a_pair_across_a_fine_cell_boundary_is_still_resolved() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(SEPARATION_CELL - 0.1, 1.0), 0.45, 1.0),
            pawn(2, Vec2::new(SEPARATION_CELL + 0.1, 1.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, tuning(1.0));

        assert!(state.pushes[0].x < 0.0);
        assert!(state.pushes[1].x > 0.0);
    }
}
