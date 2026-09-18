use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

/// Полуширина разброса личной скорости — дефолт [`HumanStyle::spread`], системы
/// читают ресурс, а не эту константу. ±15%: толпа перестаёт двигаться
/// монолитом, но обгон остаётся медленным и строй распадается не мгновенно.
pub const HUMAN_SPEED_SPREAD: f32 = 0.15;
/// Границы ползунка панели Human. Потолок не круглый, а выведенный: демон
/// быстрее бегущего человека ровно на 35% ([`DEMON_SPEED`]), и на своём
/// минимальном множителе ([`DEMON_SPEED_FACTOR_MIN`]) он идёт как раз
/// `HUMAN_FLEE_SPEED × 1.35`. Разброс выше 0.35 сделал бы самых быстрых людей
/// недогоняемыми — паника перестала бы кончаться смертью в принципе.
///
/// [`DEMON_SPEED`]: crate::settings::DEMON_SPEED
/// [`DEMON_SPEED_FACTOR_MIN`]: crate::demon::DEMON_SPEED_FACTOR_MIN
pub const HUMAN_SPEED_SPREAD_MIN: f32 = 0.0;
pub const HUMAN_SPEED_SPREAD_MAX: f32 = 0.35;
pub const HUMAN_SPEED_SPREAD_STEP: f32 = 0.05;

/// Радиус «тела» человека, м — дефолт [`HumanStyle::body_radius`]. Вдвое больше
/// прежних 0.45, то есть ЗАМЕТНО больше половины спрайта
/// ([`HUMAN_SIZE`](crate::settings::HUMAN_SIZE)). Так и задумано: на прежнем
/// радиусе дистанция покоя пары (0.9 м) была меньше самого спрайта (1.0 м), и
/// правильно разведённая толпа всё равно рисовалась сплошной мозаикой из
/// наезжающих друг на друга квадратов — «расталкивание не работает» на глаз.
/// Теперь между спрайтами в покое почти корпус зазора (1.8 м против 1.0 м).
///
/// Значение подобрано ползунком `Body radius` на живой толпе — это вопрос вида,
/// а не расчёта, и ручка есть и в панели Navigation, и в демо-сцене
/// (`examples/demos/crowd_demo.rs`); константа ей лишь дефолт. Радиус демона не
/// отдельная ручка: он всегда вдвое больше
/// ([`DEMON_BODY_RADIUS`](crate::settings::DEMON_BODY_RADIUS)).
///
/// Сумма радиусов демон+человек (2.7 м) больше
/// [`KILL_DISTANCE`](crate::settings::KILL_DISTANCE) — это не мешает убийству,
/// потому что смыкается демон в фазе броска, а бросок из расталкивания исключён
/// целиком.
pub const HUMAN_BODY_RADIUS: f32 = 0.9;
/// Границы ползунка радиуса тела. Снизу — меньше половины спрайта, то есть
/// разведённая пара перекрывается спрайтами (так и было, пока радиус не стал
/// ручкой); сверху — заведомо избыточное личное пространство, на котором видно,
/// как решётка слотов назначения переходит на блок в два тайла.
pub const HUMAN_BODY_RADIUS_MIN: f32 = 0.3;
pub const HUMAN_BODY_RADIUS_MAX: f32 = 1.2;
pub const HUMAN_BODY_RADIUS_STEP: f32 = 0.01;

// Умолчание каждого ползунка — внутри его же диапазона.
const _: () = {
    assert!(
        HUMAN_SPEED_SPREAD >= HUMAN_SPEED_SPREAD_MIN
            && HUMAN_SPEED_SPREAD <= HUMAN_SPEED_SPREAD_MAX
    );
    assert!(
        HUMAN_BODY_RADIUS >= HUMAN_BODY_RADIUS_MIN && HUMAN_BODY_RADIUS <= HUMAN_BODY_RADIUS_MAX
    );
};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Human;

/// Одежда — свой цвет человека, разыгранный при спавне из его потока решений.
/// Хранится отдельно от `Sprite::color`: тот на время паники перекрашивается
/// в общий тон (`look.rs`), и возвращать после неё надо именно одежду.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct Attire(pub Color);

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

/// Сколько людей расселяет спавн. Ручки нет и в настройках не живёт: в игре
/// это всегда [`HUMAN_COUNT`](crate::settings::HUMAN_COUNT), а параметром
/// сделано ради сцен, которым толпа не нужна, — реплей-теста
/// (`tests/determinism.rs`) и демо-стендов.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PopulationSize(pub usize);

impl Default for PopulationSize {
    fn default() -> Self {
        Self(crate::settings::HUMAN_COUNT)
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
    /// Радиус «тела», м — сколько личного пространства человек держит вокруг
    /// себя. Радиус демона не отдельная ручка: он всегда вдвое больше
    /// (`separation::demon_radius`), как и спрайт.
    ///
    /// Живёт здесь, а не в `SeparationStyle`, хотя ползунок появился ради
    /// расталкивания: это свойство ТЕЛА, и читателей у него двое. Второй —
    /// слоты назначения (`movement::destination`), которые работают и тогда,
    /// когда расталкивание выключено, в том числе в детерминированном режиме.
    /// Пока величина лежала в настройках расталкивания, панель писала у него
    /// `off`, а ручка продолжала менять геометрию слотов.
    pub body_radius: f32,
}

impl Default for HumanStyle {
    fn default() -> Self {
        Self {
            spread: HUMAN_SPEED_SPREAD,
            body_radius: HUMAN_BODY_RADIUS,
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
/// ([`WANDER_CONE`](crate::settings::WANDER_CONE)): без памяти направления
/// пешка на каждом шаге разворачивалась случайно и топталась на месте.
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
///
/// Пишется **во время бегства**, каждой перепрокладкой
/// ([`FleeAction::Flee::ban`](crate::human::decide::FleeAction::Flee::ban)), а
/// не в момент успокоения: там демона в радиусе уже нет по определению
/// ветки. Поэтому у человека, который запаниковал и успокоился раньше первой
/// перепрокладки (демон ушёл за 0.7–1.2 с), запрета не будет вовсе — и это
/// правильнее прежнего поведения, синтезировавшего вектор из прогулочного
/// курса, к демону отношения не имевшего.
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

/// Человек становится трупом: поведение и движение снимаются, тело ложится.
///
/// Одна точка на весь переход, и живёт она у человека — не у демона. Обсервер
/// убийства (`demon::behavior::on_demon_caught_human`) перечислял шестнадцать
/// типов из двух чужих модулей; теперь он говорит, ЧТО случилось, а из чего
/// состоит человек и что таскает за собой движение, знают те, кому это
/// принадлежит. Как тело выглядит — поза, погасшая одежда, лужа и брызги под
/// грудью — принадлежит рисунку человека (`look`), и вид трупа принадлежит
/// человеку, а не тому, кто его убил.
///
/// Что остаётся на теле намеренно: [`PawnId`](crate::rng::PawnId) и
/// `WanderIndex` — паспорт пешки, по нему труп опознаётся в отладке; `Pace` и
/// [`WanderHeading`] — жребий, разыгранный при спавне, читать его без
/// `Movable` некому; `Attire` — из неё считается цвет тела.
pub fn to_corpse(
    commands: &mut Commands,
    silhouettes: &crate::silhouette::Silhouettes,
    entity: Entity,
) {
    crate::movement::strip_movement(commands, entity);
    let pose = super::look::corpse_pose(entity);
    let blood = super::look::blood_look(entity);
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
        )>()
        .insert((
            CorpseTag,
            crate::silhouette::Silhouette::new(
                Vec2::splat(super::look::CORPSE_SPAN),
                crate::settings::HUMAN_MIN_PX,
            ),
        ))
        // после `remove`: снятая паника вернула бы одежду поверх цвета тела
        .queue(move |mut body: EntityWorldMut| super::look::lay_down(&mut body, pose))
        // порядок вставки читается как порядок событий: сперва разлетелись
        // брызги, потом на них натекла лужа. Рисование разводит их своим z
        // (`look::Z_POOL` выше `look::Z_SPATTER`), а не этим порядком
        .with_child(super::look::blood_spatter(silhouettes, pose, blood))
        .with_child(super::look::blood_pool(silhouettes, pose, blood));
}

/// Троттлинг перепрокладки пути при бегстве.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FleeRepath(pub Timer);

impl Default for FleeRepath {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0, TimerMode::Repeating))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::{
        BodyScale, DestinationClaim, Movable, MovableStateMovingTag, NeedsWanderTarget,
        PathfindingRequest, PreviousSimPosition, RequestedAt, RetireAt, SimPosition,
    };

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
                PanicRecoil(Vec2::X),
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
            let mut commands = world.commands();
            to_corpse(
                &mut commands,
                &crate::silhouette::Silhouettes::default(),
                human,
            );
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
        ] {
            assert!(!present, "на трупе остался {name}");
        }
    }

    /// Тело лежит: вид трупа принадлежит человеку, и переход его меняет —
    /// погасшая одежда поверх снятой паники, поза по битам сущности, лужа под
    /// телом.
    #[test]
    fn a_corpse_lies_down_under_everything_that_walks() {
        let mut app = App::new();
        app.add_observer(super::super::look::on_calm_tint);
        let human = spawn_fleeing_human(&mut app);
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            to_corpse(
                &mut commands,
                &crate::silhouette::Silhouettes::default(),
                human,
            );
        });
        app.world_mut().flush();

        let corpse = app.world().entity(human);
        let attire = corpse.get::<Attire>().expect("одежда остаётся на теле");
        let sprite = corpse.get::<Sprite>().expect("Sprite");
        assert_eq!(sprite.color, super::super::look::corpse_tint(Some(attire)));
        // размер трупа приходит через `Silhouette` — его пишут системы
        // `silhouette/`, а не `lay_down`
        assert_eq!(
            corpse.get::<crate::silhouette::Silhouette>().copied(),
            Some(crate::silhouette::Silhouette::new(
                Vec2::splat(super::super::look::CORPSE_SPAN),
                crate::settings::HUMAN_MIN_PX,
            ))
        );
        let pose = super::super::look::corpse_pose(human);
        assert_eq!(sprite.flip_x, pose.flip);
        let transform = corpse.get::<Transform>().expect("Transform");
        assert_eq!(transform.translation.z, crate::settings::Z_CORPSE);
        assert_eq!(transform.rotation, Quat::from_rotation_z(pose.heading));

        // кровь — две дочерние сущности тела: брызги легли разом, лужа ещё
        // натекает (`look::spread_blood`)
        let children = corpse.get::<Children>().expect("кровь — дети тела");
        assert_eq!(children.len(), 2);
        let has = |index: usize| {
            let child = app.world().entity(children[index]);
            (
                child.contains::<super::super::look::BloodSpatter>(),
                child.contains::<super::super::look::BloodPool>(),
            )
        };
        assert_eq!(has(0), (true, false), "первыми ложатся брызги");
        assert_eq!(has(1), (false, true), "лужа натекает поверх них");
    }
}
