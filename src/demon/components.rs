use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{
    BRUTE, DEMON_CAP, DEMON_CHASE_REPATH, DEMON_DEVOUR_PAUSE, DEMON_LUNGE_BOOST,
    DEMON_SPEED_FACTOR, DemonKindStats, IMP,
};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Demon;

/// Вид демона. Бес — охотник за людьми, Громила — ломает бастионы и людей не
/// ест; разделение труда — то, ради чего игрок призывает обоих. Числа вида —
/// `settings::IMP` / `settings::BRUTE`. Рядом с перечислением едет маркер
/// вида ([`ImpTag`] / [`BruteTag`]): лестницы фильтруют выборки по нему, а по
/// варианту перечисления запрос не отфильтровать.
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Component)]
pub enum DemonKind {
    #[default]
    Imp,
    Brute,
}

impl DemonKind {
    pub fn stats(self) -> &'static DemonKindStats {
        match self {
            Self::Imp => &IMP,
            Self::Brute => &BRUTE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Imp => "imp",
            Self::Brute => "brute",
        }
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;
    use crate::settings::MAX_BODY_SCALE;

    /// Бес — прежний демон: его тело и есть `BodyScale::DEMON`. Громила
    /// крупнее и медленнее, и именно он задаёт ячейку расталкивания.
    #[test]
    fn the_imp_is_the_old_demon_and_the_brute_is_the_largest_body() {
        let imp = DemonKind::Imp.stats();
        let brute = DemonKind::Brute.stats();
        assert_eq!(imp.body_scale, crate::movement::BodyScale::DEMON.0);
        assert_eq!(imp.speed_mul, 1.0);
        assert_eq!(imp.damage, 0.0);
        assert!(brute.body_scale > imp.body_scale);
        assert!(brute.speed_mul < imp.speed_mul);
        assert!(brute.damage > 0.0);
        assert_eq!(MAX_BODY_SCALE, brute.body_scale.max(imp.body_scale));
    }
}

/// Маркер Беса — ставится при спавне вместе с [`DemonKind::Imp`].
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct ImpTag;

/// Маркер Громилы — ставится при спавне вместе с [`DemonKind::Brute`].
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct BruteTag;

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
/// переживает. Старый `settings.toml` с ключом `interval` (интервальный
/// спавнер снят) читается как прежде — лишний ключ `bevy_settings` пропускает.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "demon")]
pub struct DemonStyle {
    /// Потолок числа демонов; дойдя до него, залп и призыв молчат. Понижение
    /// уже вышедших демонов не убирает — оно видно только после рестарта.
    pub cap: usize,
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
            speed: DEMON_SPEED_FACTOR,
            lunge: DEMON_LUNGE_BOOST,
        }
    }
}

/// Спавнер демонов: стартовый залп, дальше — только призыв за души.
/// Интервального спавна нет (city-siege references/m1-baseline.md, решение 5): он противоречил бы
/// «души покупают демонов».
///
/// Состояние мира, а не настройка: `WorldStarted` пересобирает его целиком
/// (`demon::on_world_started`). В реестре типов — ради живого осмотра по BRP,
/// рядом с остальным состоянием прогона (`Telemetry`, `SimTick`, `TickDebt`).
/// Регистрация даёт и запись: правка `spawned` руками по BRP раздаст уже
/// выданные `PawnId` второй раз, а на их уникальности стоят и поток ГПСЧ
/// пешки, и ключ очереди диспетчера.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub struct DemonSpawner {
    pub spawned: usize,
    pub initial_burst_done: bool,
}
