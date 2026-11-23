use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{DEMON_CHASE_REPATH, DEMON_DEVOUR_PAUSE};

/// Кап и интервал спавна — дефолты [`DemonStyle`], системы читают ресурс, а не
/// эти константы. Интервал подобран под директиву владельца (ревью плана ×10,
/// 2026-11-21): фон должен заполнять кап за ~1–2 сим-минуты, а не за час, —
/// 3500 демонов при таймере 0.03 с (огонь раз в два тика на 64 Гц) выходят за
/// ~110 сим-секунд. Стартовый залп ручкой не стал и остался в `settings.rs`
/// рядом с прочим тюнингом демона ([`DEMON_INITIAL_BURST`]).
///
/// [`DEMON_INITIAL_BURST`]: crate::settings::DEMON_INITIAL_BURST
pub const DEMON_SPAWN_INTERVAL: f32 = 0.03;
pub const DEMON_CAP: usize = 3500;
/// Множитель к `DEMON_SPEED` и надбавка к ней на время броска — тоже дефолты
/// [`DemonStyle`]. Оба ползунка стоят на 30% своего хода: скорость 130% от
/// базы, бросок +30% к получившейся скорости.
pub const DEMON_SPEED_FACTOR: f32 = 1.3;
pub const DEMON_LUNGE_BOOST: f32 = 0.3;

/// Границы ползунков панели Demon.
pub const DEMON_CAP_MIN: f32 = 0.0;
pub const DEMON_CAP_MAX: f32 = 5000.0;
pub const DEMON_CAP_STEP: f32 = 25.0;
pub const DEMON_SPAWN_INTERVAL_MIN: f32 = 0.01;
pub const DEMON_SPAWN_INTERVAL_MAX: f32 = 10.0;
pub const DEMON_SPAWN_INTERVAL_STEP: f32 = 0.01;
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
    assert!(WAVE_SIZE_DEFAULT as f32 >= WAVE_SIZE_MIN && WAVE_SIZE_DEFAULT as f32 <= WAVE_SIZE_MAX);
    assert!(WAVE_EVERY_DEFAULT >= WAVE_EVERY_MIN && WAVE_EVERY_DEFAULT <= WAVE_EVERY_MAX);
    assert!(WAVE_LEN_DEFAULT as f32 >= WAVE_LEN_MIN && WAVE_LEN_DEFAULT as f32 <= WAVE_LEN_MAX);
};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
#[require(WarpOrigin)]
pub struct Demon;

/// Сколько тиков между проходами acquisition одного демона: срезов столько же,
/// на тике работает один (`tick % ACQUIRE_SLICE_TICKS == S`). Агро может
/// запоздать на K−1 тик — на 30x это сотые доли сим-секунды, невидимо
/// (фаза 6 плана ×10).
pub const ACQUIRE_SLICE_TICKS: u64 = 4;

/// Срез acquisition демона: `PawnId % ACQUIRE_SLICE_TICKS`, ставится в
/// `spawn_demon` и живёт на демоне до конца. Маркеров K, взаимоисключающих
/// по построению (срез пишет один сайт), и срезная система фильтруется своим:
/// на тике `t` работает система `t % K` и обходит только свою четверть
/// блуждающих. Const-generic, а не значение в одном компоненте, по той же
/// причине, что у [`crate::human::lod::FarSlice`] — фильтр «равен текущему
/// тику» не архетипен.
///
/// Под тумблером детерминизма срезов не существует — там acquisition идёт
/// целиком каждый тик ([`crate::demon::behavior::acquire_targets`]); маркер в
/// детерминированном прогоне остаётся, но никакая система его не читает.
#[derive(Component, Default)]
pub struct DemonSlice<const S: u8>;

/// Срез демона по его номеру — тот же остаток, что и у дальних людей.
pub fn acquire_slice_of(pawn_id: u32) -> u8 {
    (pawn_id % ACQUIRE_SLICE_TICKS as u32) as u8
}

/// Точка, от которой демон блуждает «наоборот» — `demon::systems::pick_wander_targets`
/// строит вектор `away` от неё, а не от портала. Рождённый у портала несёт
/// [`PortalPos`](crate::portal::PortalPos); рождённый волной орды — её очаг
/// (`demon::horde`), и вся стая волны расходится от своего района, а не от
/// портала. Позиция происхождения на жизнь демона: после блуждания она не
/// переставляется.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct WarpOrigin(pub Vec2);

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

impl DemonStyle {
    /// Период таймера спавнера, зажатый шкалой: старый файл настроек и запись
    /// по BRP могут принести что угодно, а отрицательный или нечисловой
    /// интервал паникует в `Duration::from_secs_f32` на первом тике спавнера.
    pub fn interval(&self) -> f32 {
        if self.interval.is_finite() {
            self.interval
                .clamp(DEMON_SPAWN_INTERVAL_MIN, DEMON_SPAWN_INTERVAL_MAX)
        } else {
            DEMON_SPAWN_INTERVAL
        }
    }

    /// Потолок капа: запись по BRP может принести сколь угодно большой, и
    /// спавнер заселил бы мир до исчерпания ресурсов. Ноль легитимен — спавн
    /// выключен.
    pub fn cap(&self) -> usize {
        self.cap.min(DEMON_CAP_MAX as usize)
    }
}

/// Границы ползунков панели Demon секции орды (`demon::horde`).
pub const WAVE_SIZE_MIN: f32 = 0.0;
pub const WAVE_SIZE_MAX: f32 = 2000.0;
pub const WAVE_SIZE_STEP: f32 = 10.0;
pub const WAVE_EVERY_MIN: f32 = 1.0;
pub const WAVE_EVERY_MAX: f32 = 600.0;
pub const WAVE_EVERY_STEP: f32 = 5.0;
pub const WAVE_LEN_MIN: f32 = 1.0;
pub const WAVE_LEN_MAX: f32 = 200.0;
pub const WAVE_LEN_STEP: f32 = 1.0;

/// Размер и ритм волн орды; расписание и выпуск — `demon::horde`.
/// Отдельно от [`DemonStyle`] по той же причине, что и все стили: это выбор
/// пользователя, пережающий рестарт, тогда как `HordeSchedule` — состояние
/// мира. `wave_size == 0` выключает волны вовсе — фон-only, сегодняшний мир.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "horde")]
pub struct HordeStyle {
    /// Демонов в одной волне. Ноль — волн нет.
    pub wave_size: usize,
    /// Секунды затишья между волнами (с разбросом — см. `WAVE_LULL_JITTER`).
    pub wave_every: f32,
    /// Порция выпуска за тик: волна выходит не одним залпом команд, а
    /// разливается по `wave_len` демонов за тик.
    pub wave_len: usize,
}

/// Дефолтные числа — выбор фазы 4 плана ×10
/// (`docs/population-x10-plan.md`): волна 150 каждые ~полторы минуты
/// заметна на фоне быстрого фонового заполнения, но сама по себе не
/// опустошает кап.
pub const WAVE_SIZE_DEFAULT: usize = 150;
pub const WAVE_EVERY_DEFAULT: f32 = 90.0;
pub const WAVE_LEN_DEFAULT: usize = 30;

impl Default for HordeStyle {
    fn default() -> Self {
        Self {
            wave_size: WAVE_SIZE_DEFAULT,
            wave_every: WAVE_EVERY_DEFAULT,
            wave_len: WAVE_LEN_DEFAULT,
        }
    }
}

impl HordeStyle {
    /// Демонов в волне, зажатые шкалой: запись по BRP и старый файл настроек
    /// могут принести сколь угодно большое число, а волна разыгрывается
    /// целиком и усекается до свободного места лишь после — значение за
    /// пределами `isize` падает на `Vec::with_capacity`, меньшее держит
    /// навмеш под чтением триллионами попыток разброса. Ноль легитимен —
    /// волны выключены.
    pub fn wave_size(&self) -> usize {
        self.wave_size.min(WAVE_SIZE_MAX as usize)
    }

    /// Секунды затишья, зажатые шкалой: старый файл настроек и запись по BRP
    /// могут принести что угодно, а отрицательный или нечисловой интервал
    /// разрушил бы расписание (`as u64` от NaN/отрицательного — ноль тиков,
    /// волна на каждом тике).
    pub fn wave_every(&self) -> f32 {
        if self.wave_every.is_finite() {
            self.wave_every.clamp(WAVE_EVERY_MIN, WAVE_EVERY_MAX)
        } else {
            WAVE_EVERY_DEFAULT
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Мусорные значения (BRP, старый settings.toml) не доходят до
    /// `Duration::from_secs_f32` — он паникует на отрицательном и нечисловом.
    #[test]
    fn interval_clamps_garbage_to_the_slider_scale() {
        assert_eq!(DemonStyle::default().interval(), DEMON_SPAWN_INTERVAL);
        assert_eq!(
            DemonStyle {
                interval: -1.0,
                ..Default::default()
            }
            .interval(),
            DEMON_SPAWN_INTERVAL_MIN
        );
        assert_eq!(
            DemonStyle {
                interval: 100.0,
                ..Default::default()
            }
            .interval(),
            DEMON_SPAWN_INTERVAL_MAX
        );
        assert_eq!(
            DemonStyle {
                interval: f32::NAN,
                ..Default::default()
            }
            .interval(),
            DEMON_SPAWN_INTERVAL
        );
    }

    /// Кап по BRP может быть сколь угодно большим — спавнер не заселяет мир
    /// до исчерпания ресурсов.
    #[test]
    fn cap_clamps_to_the_slider_ceiling() {
        assert_eq!(DemonStyle::default().cap(), DEMON_CAP);
        assert_eq!(
            DemonStyle {
                cap: usize::MAX,
                ..Default::default()
            }
            .cap(),
            DEMON_CAP_MAX as usize
        );
    }

    /// Размер волны по BRP может быть мусорным: волна разыгрывается целиком
    /// до усечения по капу, так что без клампа на чтении значение за
    /// пределами `isize` — падение на `Vec::with_capacity`, а не «вышло
    /// сколько влезло».
    #[test]
    fn wave_size_clamps_garbage_to_the_slider_scale() {
        assert_eq!(HordeStyle::default().wave_size(), WAVE_SIZE_DEFAULT);
        assert_eq!(
            HordeStyle {
                wave_size: 0,
                ..Default::default()
            }
            .wave_size(),
            0,
            "ноль — волны выключены, кламп его не трогает"
        );
        assert_eq!(
            HordeStyle {
                wave_size: usize::MAX,
                ..Default::default()
            }
            .wave_size(),
            WAVE_SIZE_MAX as usize
        );
    }

    /// Затишье по BRP может быть мусорным — NaN и отрицательное через
    /// `as u64` дали бы ноль тиков, то есть волну на каждом тике.
    #[test]
    fn wave_every_clamps_garbage_to_the_slider_scale() {
        assert_eq!(HordeStyle::default().wave_every(), WAVE_EVERY_DEFAULT);
        assert_eq!(
            HordeStyle {
                wave_every: -1.0,
                ..Default::default()
            }
            .wave_every(),
            WAVE_EVERY_MIN
        );
        assert_eq!(
            HordeStyle {
                wave_every: 10_000.0,
                ..Default::default()
            }
            .wave_every(),
            WAVE_EVERY_MAX
        );
        assert_eq!(
            HordeStyle {
                wave_every: f32::NAN,
                ..Default::default()
            }
            .wave_every(),
            WAVE_EVERY_DEFAULT
        );
    }
}
