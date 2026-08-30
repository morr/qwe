use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{HUMAN_BODY_RADIUS, HUMAN_SPEED_SPREAD};

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

/// Цвет и размер лежащего тела — вид трупа принадлежит человеку, а не тому,
/// кто его убил.
const CORPSE_COLOR: Color = Color::srgb(0.35, 0.16, 0.14);
const CORPSE_SIZE: Vec2 = Vec2::new(1.6, 0.8);

/// Человек становится трупом: поведение и движение снимаются, тело ложится.
///
/// Одна точка на весь переход, и живёт она у человека — не у демона. Обсервер
/// убийства (`demon::behavior::on_demon_caught_human`) перечислял шестнадцать
/// типов из двух чужих модулей; теперь он говорит, ЧТО случилось, а из чего
/// состоит человек и что таскает за собой движение, знают те, кому это
/// принадлежит.
///
/// Что остаётся на теле намеренно: [`PawnId`](crate::rng::PawnId) и
/// `WanderIndex` — паспорт пешки, по нему труп опознаётся в отладке; `Pace` и
/// [`WanderHeading`] — жребий, разыгранный при спавне, читать его без
/// `Movable` некому.
pub fn to_corpse(commands: &mut Commands, entity: Entity) {
    crate::movement::strip_movement(commands, entity);
    let mut corpse = commands.entity(entity);
    corpse
        .remove::<(
            Human,
            HumanWanderTag,
            HumanFleeTag,
            HumanFirstWanderTag,
            WanderPause,
            FleeRepath,
            PanicRecoil,
        )>()
        .insert(CorpseTag);
    corpse.entry::<Sprite>().and_modify(|mut sprite| {
        sprite.color = CORPSE_COLOR;
        sprite.custom_size = Some(CORPSE_SIZE);
    });
    corpse.entry::<Transform>().and_modify(|mut transform| {
        transform.translation.z = crate::settings::Z_CORPSE;
    });
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
        DestinationClaim, Movable, MovableStateMovingTag, NeedsWanderTarget, PathfindingRequest,
        PreviousSimPosition, RequestedAt, RetireAt, SimPosition,
    };

    /// Человек в разгар паники, с полным набором рантайм-компонент: бежит по
    /// пути, держит слот назначения и ждёт ответа на новую заявку. Труп из
    /// такого — самый нагруженный из возможных.
    fn spawn_fleeing_human(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                Sprite::default(),
                Transform::default(),
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
            to_corpse(&mut commands, human);
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
            (
                "PreviousSimPosition",
                corpse.contains::<PreviousSimPosition>(),
            ),
            ("NeedsWanderTarget", corpse.contains::<NeedsWanderTarget>()),
        ] {
            assert!(!present, "на трупе остался {name}");
        }
    }

    /// Тело лежит: вид трупа принадлежит человеку, и переход его меняет.
    #[test]
    fn a_corpse_lies_down_under_everything_that_walks() {
        let mut app = App::new();
        let human = spawn_fleeing_human(&mut app);
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            to_corpse(&mut commands, human);
        });
        app.world_mut().flush();

        let corpse = app.world().entity(human);
        assert_eq!(corpse.get::<Sprite>().expect("Sprite").color, CORPSE_COLOR);
        assert_eq!(
            corpse.get::<Sprite>().expect("Sprite").custom_size,
            Some(CORPSE_SIZE)
        );
        assert_eq!(
            corpse.get::<Transform>().expect("Transform").translation.z,
            crate::settings::Z_CORPSE
        );
    }
}
