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

// `PanicRecoil` тянется require'ом: постоянный у каждого живого человека,
// живёт значением (`Some`/`None`) — переносы между архетипами на цикле
// «перепрокладка → выбор цели» остаются только у тегов состояний.
// `LodDwell` — тем же способом: выдержка demote — состояние значением,
// ноль у свежей пешки (расселение уводит толпу за кадр без выдержки)
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
#[require(PanicRecoil, crate::human::lod::LodDwell)]
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
///
/// Отражён для BRP: проверка инварианта телеметрии живьём
/// (`живые + убитые + спасшиеся == население`) читает население отсюда —
/// и не обязана знать, чья константа стояла при старте (`QWE_PERF_POP`,
/// реплей-двор).
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource)]
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
/// запомненный на последней перепрокладке бегства. Пока в `Some`,
/// первая цель после успокоения обязана быть дальней («по делам») и не
/// попадать в конус `RECOIL_CONE` вокруг этого вектора; гаснет при первом
/// же удачном выборе цели — дальше человек гуляет как обычно.
///
/// Компонент **постоянный** — тянется `#[require]` у [`Human`], а живёт
/// значением: бегство пишет `Some`, выбор цели вписывает `None` на месте.
/// Прежний вариант «компонент есть/нет» перегонял человека между архетипами
/// на каждой перепрокладке и на каждом выборе цели — сотни переносов в
/// секунду в шторме, и ось «есть/нет» удваивала пространство архетипов,
/// которое волны поведения и так заполняют.
///
/// Пишется **во время бегства**, каждой перепрокладкой
/// ([`FleeAction::Flee::ban`](crate::human::decide::FleeAction::Flee::ban)), а
/// не в момент успокоения: там демона в радиусе уже нет по определению
/// ветки. Поэтому у человека, который запаниковал и успокоился раньше первой
/// перепрокладки (демон ушёл за 0.7–1.2 с), запрета не будет вовсе — и это
/// правильнее прежнего поведения, синтезировавшего вектор из прогулочного
/// курса, к демону отношения не имевшего.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct PanicRecoil(pub Option<Vec2>);

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

/// Троттлинг перепрокладки пути при бегстве.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FleeRepath(pub Timer);

impl Default for FleeRepath {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0, TimerMode::Repeating))
    }
}
