use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{DEMON_CHASE_REPATH, DEMON_DEVOUR_PAUSE};

/// Кап и интервал спавна — дефолты [`DemonStyle`], системы читают ресурс, а не
/// эти константы. Стартовый залп ручкой не стал и остался в `settings.rs`
/// рядом с прочим тюнингом демона ([`DEMON_INITIAL_BURST`]).
///
/// [`DEMON_INITIAL_BURST`]: crate::settings::DEMON_INITIAL_BURST
pub const DEMON_SPAWN_INTERVAL: f32 = 1.0;
pub const DEMON_CAP: usize = 350;
/// Множитель к `DEMON_SPEED` и надбавка к ней на время броска — тоже дефолты
/// [`DemonStyle`]. Оба ползунка стоят на 30% своего хода: скорость 130% от
/// базы, бросок +30% к получившейся скорости.
pub const DEMON_SPEED_FACTOR: f32 = 1.3;
pub const DEMON_LUNGE_BOOST: f32 = 0.3;

/// Границы ползунков панели Demon.
pub const DEMON_CAP_MIN: f32 = 0.0;
pub const DEMON_CAP_MAX: f32 = 500.0;
pub const DEMON_CAP_STEP: f32 = 5.0;
pub const DEMON_SPAWN_INTERVAL_MIN: f32 = 0.1;
pub const DEMON_SPAWN_INTERVAL_MAX: f32 = 10.0;
pub const DEMON_SPAWN_INTERVAL_STEP: f32 = 0.1;
pub const DEMON_SPEED_FACTOR_MIN: f32 = 1.0;
pub const DEMON_SPEED_FACTOR_MAX: f32 = 2.0;
pub const DEMON_SPEED_FACTOR_STEP: f32 = 0.05;
pub const DEMON_LUNGE_BOOST_MIN: f32 = 0.0;
pub const DEMON_LUNGE_BOOST_MAX: f32 = 1.0;
pub const DEMON_LUNGE_BOOST_STEP: f32 = 0.05;

// Умолчание каждого ползунка — внутри его же диапазона.
const _: () = {
    assert!(DEMON_CAP as f32 >= DEMON_CAP_MIN && DEMON_CAP as f32 <= DEMON_CAP_MAX);
    assert!(
        DEMON_SPAWN_INTERVAL >= DEMON_SPAWN_INTERVAL_MIN
            && DEMON_SPAWN_INTERVAL <= DEMON_SPAWN_INTERVAL_MAX
    );
    assert!(
        DEMON_SPEED_FACTOR >= DEMON_SPEED_FACTOR_MIN
            && DEMON_SPEED_FACTOR <= DEMON_SPEED_FACTOR_MAX
    );
    assert!(
        DEMON_LUNGE_BOOST >= DEMON_LUNGE_BOOST_MIN && DEMON_LUNGE_BOOST <= DEMON_LUNGE_BOOST_MAX
    );
};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Demon;

/// Стейт-машина демона: Wander / Chase / Devour — эксклюзивные теги.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonWanderTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonChaseTag;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonDevourTag;

/// Цель погони. Компонент есть ⇔ демон кого-то гонит: ставится в `acquire_targets`
/// вместе с `DemonChaseTag`, снимается вместе с ним в `back_to_wander` и в
/// `on_demon_caught_human`. `Default` намеренно нет — «цели по умолчанию» не бывает,
/// её отсутствие выражается отсутствием компонента, а не битым `Entity`.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ChaseTarget(pub Entity);

/// Финальный бросок: демон идёт напрямую на текущую позицию жертвы, минуя
/// тайловый путь, и с надбавкой к скорости (`DemonStyle::lunge`). Ставится и
/// снимается в `chase`; movepath-гизмо по нему рисует стрелку прямо в цель,
/// а не по остаткам старого пути.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct DemonLungeTag;

/// Троттлинг перепрокладки пути во время погони.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ChaseRepath(pub Timer);

impl Default for ChaseRepath {
    fn default() -> Self {
        Self(Timer::from_seconds(
            DEMON_CHASE_REPATH,
            TimerMode::Repeating,
        ))
    }
}

/// Из чего состоит погоня: набор снимается **целиком** на любом выходе из
/// Chase — и в блуждание (`back_to_wander`), и в Devour (обсервер убийства).
/// Одно имя вместо двух списков: новая компонента погони дописывается сюда, и
/// оба выхода узнают о ней сами.
///
/// Заявки и таска поиска здесь нет намеренно — они принадлежат движению, и
/// снимать их вправе только тот выход, который заодно паркует `Movable`
/// (см. `demon::behavior::on_demon_caught_human`).
pub type ChaseComponents = (DemonChaseTag, ChaseTarget, ChaseRepath, DemonLungeTag);

/// Пауза «пожирания» над трупом.
///
/// Дефолт — только нижняя граница `DEMON_DEVOUR_PAUSE`: живой таймер собирает
/// `on_demon_caught_human`, разыгрывая длительность по всему диапазону из
/// личного потока демона.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct DevourUntil(pub Timer);

impl Default for DevourUntil {
    fn default() -> Self {
        Self(Timer::from_seconds(DEMON_DEVOUR_PAUSE.0, TimerMode::Once))
    }
}

/// Демон догнал человека: жертва умирает, демон переходит в Devour.
#[derive(Event, Debug)]
pub struct DemonCaughtHumanEvent {
    pub demon: Entity,
    pub human: Entity,
}

/// Настройки демонов, крутятся ползунками панели Demon и сохраняются между
/// запусками. Отдельно от `DemonSpawner`: тот — состояние мира и сбрасывается
/// на рестарте и смене города, а это — выбор пользователя, который рестарт
/// переживает.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "demon")]
pub struct DemonStyle {
    /// Потолок числа демонов; дойдя до него, спавнер молчит. Понижение уже
    /// вышедших демонов не убирает — оно видно только после рестарта.
    pub cap: usize,
    /// Секунды между демонами после стартового залпа.
    pub interval: f32,
    /// Множитель к `DEMON_SPEED`, 1.0…2.0. Пишется в `Movable::speed` при
    /// спавне, а уже вышедшим демонам его раздаёт `sync_demon_speed`.
    pub speed: f32,
    /// Надбавка к скорости на время броска (`DemonLungeTag`), 0.0…1.0.
    /// В `Movable::speed` не попадает: бросок двигает `SimPosition` сам,
    /// мимо `move_moving_entities`, и множитель нужен только там.
    pub lunge: f32,
}

impl Default for DemonStyle {
    fn default() -> Self {
        Self {
            cap: DEMON_CAP,
            interval: DEMON_SPAWN_INTERVAL,
            speed: DEMON_SPEED_FACTOR,
            lunge: DEMON_LUNGE_BOOST,
        }
    }
}

/// Спавнер демонов: стартовый залп, затем по таймеру до капа.
///
/// Состояние мира, а не настройка: `WorldStarted` пересобирает его целиком
/// (`demon::on_world_started`). В реестре типов — ради живого осмотра по BRP,
/// рядом с остальным состоянием прогона (`Telemetry`, `SimTick`, `TickDebt`).
/// Регистрация даёт и запись: правка `spawned` руками по BRP раздаст уже
/// выданные `PawnId` второй раз, а на их уникальности стоят и поток ГПСЧ
/// пешки, и ключ очереди диспетчера.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct DemonSpawner {
    pub timer: Timer,
    pub spawned: usize,
    pub initial_burst_done: bool,
}

impl Default for DemonSpawner {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(DEMON_SPAWN_INTERVAL, TimerMode::Repeating),
            spawned: 0,
            initial_burst_done: false,
        }
    }
}
