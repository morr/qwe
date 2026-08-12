//! Решение погони, отделённое от его применения.
//!
//! Правила погони — когда демон бросается, когда сдаётся, когда меняет
//! жертву — жили ступенями одной лестницы `chase`, где каждая ступень и
//! решала, и действовала: дёргала `Commands`, двигала `SimPosition`, подавала
//! заявку на путь. Проверить хоть одно из них значило поднять `App`, навмеш,
//! пространственную сетку и `Time`, и даже тогда наблюдать правило только по
//! его следам в мире.
//!
//! Здесь остаётся сама лестница: [`ChaseSense`] — что демон знает о себе и о
//! цели на этом тике, [`ChaseAction`] — что из этого следует. Ни `Entity`, ни
//! `Commands`, ни запросов; тест — структура на входе, вариант перечисления на
//! выходе. Применение (снять тег, подать заявку, послать событие) целиком в
//! `behavior::chase`, и порядок ступеней теперь виден в одном месте, а не
//! разбросан по `continue`.
//!
//! Единственный вход, который не значение, а вопрос, — луч видимости: он
//! стоит дорого, и лестница обязана задать его ровно на той ступени, где он
//! нужен. На всех остальных он не считается вовсе.

use bevy::prelude::*;

use crate::movement::MovableState;
use crate::settings::{DEMON_AGGRO_RADIUS, DEMON_LUNGE_RANGE, KILL_DISTANCE, RADIUS_HYSTERESIS};

/// Лимит демонов на одну цель — «клещи» из двух допустимы, толпа — нет.
pub const MAX_CHASERS_PER_TARGET: usize = 2;
/// Переключение на свободного человека, если он не дальше ×1.5 текущей цели.
const SWITCH_DISTANCE_FACTOR: f32 = 1.5;
/// Переключение на заметно более близкого человека: новая цель должна быть
/// ближе текущей минимум на треть — иначе две почти равноудалённые жертвы
/// перекидывают демона каждый такт перепрокладки, а каждое переключение
/// стоит нового запроса пути.
const CLOSER_SWITCH_FACTOR: f32 = 0.7;

/// Всё, из чего складывается решение погони на одном тике.
#[derive(Clone, Debug)]
pub struct ChaseSense {
    /// Где стоит сам демон.
    pub position: Vec2,
    /// Позиция цели — или `None`, если цели больше нет: она despawn'нута,
    /// стала трупом или её убил другой демон раньше в этом же тике.
    pub target: Option<Vec2>,
    /// `Movable::speed` демона, м/с. Надбавка броска к ней не приложена —
    /// это делает [`decide`], потому что живёт надбавка только в броске.
    pub speed: f32,
    /// `DemonStyle::lunge` — доля, на которую бросок ускоряет демона.
    pub lunge_bonus: f32,
    /// Длительность тика, сек.
    pub delta_secs: f32,
    /// Состояние движения: есть ли цель у пути и ждём ли ответа поиска.
    pub state: MovableState,
    /// В `Movable::path` ещё остались waypoint'ы.
    pub has_path: bool,
    /// Демон хоть раз шагал — у него есть `last_direction`, то есть докат.
    pub walked: bool,
    /// Заявка на путь подана и ответа ещё нет.
    pub search_in_flight: bool,
    /// Таймер перепрокладки досчитает на этом тике.
    pub repath_due: bool,
    /// Цель делим с другим демоном — преследователей у неё уже
    /// [`MAX_CHASERS_PER_TARGET`].
    pub shared_target: bool,
}

/// Условия поиска замены цели: кандидат годится, если он не дальше `radius`
/// от демона и его преследуют меньше `max_chasers` демонов.
///
/// Сам поиск — обход пространственной сетки, он остаётся в системе; сюда
/// вынесены его *условия*, потому что правило именно в них.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchRule {
    pub radius: f32,
    pub max_chasers: usize,
}

/// Жертва, которую нашёл поиск замены.
///
/// `Entity` в чистой лестнице — сознательное послабление к правилу «никаких
/// `Entity` в `decide`»: без него лестница не может *назвать* выбранную жертву,
/// и решение «сменить цель» пришлось бы собирать в применении по остаточным
/// признакам. Смысл правила при этом цел — ни мира, ни запросов здесь нет, и
/// тесты по-прежнему строятся на `Entity::from_raw_u32` без `App`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Victim {
    pub entity: Entity,
    pub position: Vec2,
}

/// Что демон делает на этом тике.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChaseAction {
    /// Цели больше нет — обратно в блуждание. Место в очереди на неё
    /// освобождать не нужно: мёртвую цель никто уже не выберет.
    LostTarget,
    /// Демон отстал за гистерезисом радиуса агро — блуждание, и место в
    /// очереди на брошенную цель освобождается.
    GaveUp,
    /// Достал: `DemonCaughtHumanEvent`.
    Kill,
    /// Финальный бросок — вектор смещения `SimPosition` за этот тик.
    Lunge { advance: Vec2 },
    /// Первого пути ещё нет, поиск в полёте: ждём ответа, не трогая ни
    /// заявку, ни таймер перепрокладки.
    WaitForPath,
    /// Идём по тому, что есть: такт перепрокладки не настал.
    Hold,
    /// Такт перепрокладки: путь к текущей цели (`target` — её позиция).
    Repath { target: Vec2 },
    /// Такт перепрокладки, и поиск нашёл цель получше: место переезжает с
    /// прежней жертвы на новую (`ChaseClaims::transfer`), путь — к ней.
    Switch { to: Victim },
}

impl ChaseAction {
    /// Демон остаётся в погоне, но ведёт его не бросок — тег броска пора
    /// снять. На выходах из погони (`LostTarget`, `GaveUp`, `Kill`) тег
    /// уносят `back_to_wander` и обсервер убийства, поэтому их здесь нет.
    pub fn cancels_lunge(&self) -> bool {
        matches!(
            self,
            Self::WaitForPath | Self::Hold | Self::Repath { .. } | Self::Switch { .. }
        )
    }

    /// Демон уходит от ЖИВОЙ цели — место в очереди на неё
    /// ([`ChaseClaims`](super::claims::ChaseClaims)) пора освободить.
    ///
    /// Правило асимметрично, и в этом вся суть: `Kill` и `LostTarget` места не
    /// освобождают, потому что цели после них уже нет — мёртвую никто не
    /// выберет, а лишнее вычитание уводило бы счётчик под ноль. Раньше это
    /// решал доккомментарий у одной из веток применения; здесь match
    /// исчерпывающий, так что новый выход из погони не пройдёт мимо вопроса.
    pub fn releases_claim(&self) -> bool {
        match self {
            Self::GaveUp => true,
            Self::Kill | Self::LostTarget => false,
            // смена жертвы место не освобождает, а перевозит: обе половины
            // делает `ChaseClaims::transfer` одним вызовом
            Self::Switch { .. } => false,
            Self::Lunge { .. } | Self::WaitForPath | Self::Hold | Self::Repath { .. } => false,
        }
    }
}

/// Лестница погони.
///
/// Оба дорогих чувства спрашиваются лениво и не больше одного раза каждое:
/// `line_of_sight` — только когда дистанция уже позволяет бросок,
/// `better_victim` — только на такте перепрокладки, до которого лестница ещё
/// должна дойти. Поиск замены — обход пространственной сетки с проверкой
/// видимости победителя, десятки кандидатов; спрашивать его на каждом тике
/// каждого демона было бы вдесятеро дороже самой лестницы.
pub fn decide(
    sense: &ChaseSense,
    line_of_sight: impl FnOnce() -> bool,
    better_victim: impl FnOnce(SwitchRule) -> Option<Victim>,
) -> ChaseAction {
    // цель умерла (труп/despawn/её только что съел сосед) — снова блуждание
    let Some(target) = sense.target else {
        return ChaseAction::LostTarget;
    };
    let distance = sense.position.distance(target);

    // гистерезис выхода из погони
    if distance > DEMON_AGGRO_RADIUS * RADIUS_HYSTERESIS {
        return ChaseAction::GaveUp;
    }

    if distance < KILL_DISTANCE {
        return ChaseAction::Kill;
    }

    // Финальный бросок. Тайловый путь ведёт к ЦЕНТРУ тайла жертвы, а та
    // внутри тайла продолжает двигаться: остаток до полутора метров тайловой
    // навигацией не покрывается, и демон бесконечно «почти догоняет». Вблизи
    // идём прямо на текущую позицию цели — но только при прямой видимости:
    // жертва, скрывшаяся за углом здания, снова догоняется обычным путём,
    // сквозь стены бросок не проходит.
    if distance <= DEMON_LUNGE_RANGE && line_of_sight() {
        let speed = sense.speed * (1.0 + sense.lunge_bonus);
        // шаг подрезан дистанцией: бросок доводит демона ровно до жертвы и
        // никогда не пролетает мимо неё
        let step = (speed * sense.delta_secs).min(distance);
        return ChaseAction::Lunge {
            advance: (target - sense.position).normalize_or_zero() * step,
        };
    }

    // Первого пути ещё нет: путь пуст, доката нет (демон ни разу не шагал), а
    // поиск уже в полёте. Перепрокладка отменила бы его — `to_pathfinding`
    // роняет таск, — и пока конвейер отвечает медленнее, чем цель меняет тайл
    // (постройка northstar на старте, высокая скорость), демон обрывал бы
    // каждый ответ до прихода и стоял у портала вечно, отвисая только на
    // паузе. Ждём ответ: он даст путь и `last_direction`, дальше промежутки
    // перепрокладки прикрывает докат.
    if sense.search_in_flight
        && matches!(sense.state, MovableState::Pathfinding(_))
        && !sense.has_path
        && !sense.walked
    {
        return ChaseAction::WaitForPath;
    }

    // перепрокладка пути к цели — по таймеру, не каждый тик; потерянный путь
    // ждать такта не обязан
    let needs_first_path = matches!(
        sense.state,
        MovableState::Idle | MovableState::PathfindingError(_)
    );
    if !sense.repath_due && !needs_first_path {
        return ChaseAction::Hold;
    }

    // Смена цели, два случая. Цель делим с другим демоном — берём любого
    // никем не занятого человека не дальше ×1.5 текущей дистанции («клещи»
    // распадаются). Цель своя — берём человека, оказавшегося заметно ближе
    // неё, иначе демон пробегает сквозь толпу мимо доступной добычи.
    // Радиус пропорционален текущей дистанции, и это ровно то, что нужно:
    // вплотную к жертве (2 м) он 1.4 м — демон уже никуда не сворачивает;
    // в хвосте гистерезиса (67.5 м) — 47 м, обход сетки остаётся 3×3.
    let switch = if sense.shared_target {
        SwitchRule {
            radius: distance * SWITCH_DISTANCE_FACTOR,
            max_chasers: 1,
        }
    } else {
        SwitchRule {
            radius: distance * CLOSER_SWITCH_FACTOR,
            max_chasers: MAX_CHASERS_PER_TARGET,
        }
    };
    match better_victim(switch) {
        Some(victim) => ChaseAction::Switch { to: victim },
        None => ChaseAction::Repath { target },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Демон в начале координат, жертва — строго по оси X на `distance`
    /// метров. Всё остальное — «обычный тик погони»: демон идёт по пути,
    /// такт перепрокладки не настал, цель своя.
    fn sense(distance: f32) -> ChaseSense {
        ChaseSense {
            position: Vec2::ZERO,
            target: Some(Vec2::new(distance, 0.0)),
            speed: 10.0,
            lunge_bonus: 0.5,
            delta_secs: 1.0 / 64.0,
            state: MovableState::Moving(IVec2::ZERO),
            has_path: true,
            walked: true,
            search_in_flight: false,
            repath_due: false,
            shared_target: false,
        }
    }

    /// Видимость, которая помнит, спрашивали ли её.
    fn spy(answer: bool) -> (impl FnOnce() -> bool, std::rc::Rc<Cell<bool>>) {
        let asked = std::rc::Rc::new(Cell::new(false));
        let flag = asked.clone();
        (
            move || {
                flag.set(true);
                answer
            },
            asked,
        )
    }

    #[test]
    fn a_dead_target_ends_the_chase() {
        let mut sense = sense(10.0);
        sense.target = None;
        assert_eq!(decide(&sense, || true, |_| None), ChaseAction::LostTarget);
    }

    #[test]
    fn the_chase_ends_only_past_the_hysteresis_ring() {
        let ring = DEMON_AGGRO_RADIUS * RADIUS_HYSTERESIS;
        // внутри кольца демон не сдаётся, хотя радиус агро давно позади
        assert_ne!(
            decide(&sense(ring - 1.0), || false, |_| None),
            ChaseAction::GaveUp
        );
        assert_eq!(
            decide(&sense(ring + 1.0), || false, |_| None),
            ChaseAction::GaveUp
        );
    }

    #[test]
    fn contact_kills_before_the_lunge_is_considered() {
        // вплотную демон уже внутри дальности броска — порядок ступеней и
        // решает, что здесь убийство, а не ещё один шаг
        let distance = KILL_DISTANCE / 2.0;
        assert!(distance <= DEMON_LUNGE_RANGE);
        let (los, asked) = spy(true);
        assert_eq!(decide(&sense(distance), los, |_| None), ChaseAction::Kill);
        assert!(!asked.get(), "луч видимости на убийстве не нужен");
    }

    #[test]
    fn the_lunge_needs_line_of_sight() {
        let close = sense(DEMON_LUNGE_RANGE - 1.0);
        assert!(matches!(
            decide(&close, || true, |_| None),
            ChaseAction::Lunge { .. }
        ));
        // жертва за углом дома догоняется обычным путём
        assert!(!matches!(
            decide(&close, || false, |_| None),
            ChaseAction::Lunge { .. }
        ));
    }

    #[test]
    fn a_distant_target_is_never_asked_for_line_of_sight() {
        let (los, asked) = spy(true);
        decide(&sense(DEMON_LUNGE_RANGE + 1.0), los, |_| None);
        assert!(!asked.get(), "луч не считается вдали от цели");
    }

    #[test]
    fn a_lunge_never_overshoots_the_victim() {
        let distance = DEMON_LUNGE_RANGE - 1.0;
        let mut sense = sense(distance);
        // за тик демон прошёл бы много больше остатка
        sense.speed = 1000.0;
        let ChaseAction::Lunge { advance } = decide(&sense, || true, |_| None) else {
            panic!("вблизи и при видимости — бросок");
        };
        assert!((advance.length() - distance).abs() < 1e-3);
    }

    #[test]
    fn the_lunge_carries_its_speed_bonus() {
        let sense = sense(DEMON_LUNGE_RANGE - 1.0);
        let ChaseAction::Lunge { advance } = decide(&sense, || true, |_| None) else {
            panic!("вблизи и при видимости — бросок");
        };
        let expected = sense.speed * (1.0 + sense.lunge_bonus) * sense.delta_secs;
        assert!((advance.length() - expected).abs() < 1e-3);
    }

    #[test]
    fn a_demon_without_a_first_path_waits_for_its_search() {
        let mut sense = sense(20.0);
        sense.state = MovableState::Pathfinding(IVec2::ZERO);
        sense.search_in_flight = true;
        sense.has_path = false;
        sense.walked = false;
        assert_eq!(decide(&sense, || false, |_| None), ChaseAction::WaitForPath);
    }

    #[test]
    fn a_demon_that_has_walked_does_not_wait() {
        let mut sense = sense(20.0);
        sense.state = MovableState::Pathfinding(IVec2::ZERO);
        sense.search_in_flight = true;
        sense.has_path = false;
        sense.walked = true;
        // докат несёт его дальше, ответа ждать незачем
        assert_eq!(decide(&sense, || false, |_| None), ChaseAction::Hold);
    }

    #[test]
    fn the_repath_timer_gates_the_chase() {
        assert_eq!(decide(&sense(20.0), || false, |_| None), ChaseAction::Hold);
        let mut due = sense(20.0);
        due.repath_due = true;
        assert!(matches!(
            decide(&due, || false, |_| None),
            ChaseAction::Repath { .. }
        ));
    }

    #[test]
    fn a_lost_path_repaths_without_waiting_for_the_timer() {
        for state in [
            MovableState::Idle,
            MovableState::PathfindingError(IVec2::ZERO),
        ] {
            let mut sense = sense(20.0);
            sense.state = state.clone();
            sense.has_path = false;
            assert!(
                matches!(
                    decide(&sense, || false, |_| None),
                    ChaseAction::Repath { .. }
                ),
                "{state:?} — путь потерян, ждать такта нечего"
            );
        }
    }

    /// Правило поиска — то, с чем лестница зовёт замену; ловим его прямо из
    /// замыкания, потому что именно так его теперь и получает применение.
    fn asked_rule(sense: &ChaseSense) -> SwitchRule {
        let seen = std::cell::Cell::new(None);
        decide(
            sense,
            || false,
            |rule| {
                seen.set(Some(rule));
                None
            },
        );
        seen.get().expect("лестница не дошла до поиска замены")
    }

    #[test]
    fn a_shared_target_is_swapped_for_any_free_victim_nearby() {
        let mut sense = sense(20.0);
        sense.repath_due = true;
        sense.shared_target = true;
        // ищем шире текущей дистанции, но только никем не занятых
        assert_eq!(
            asked_rule(&sense),
            SwitchRule {
                radius: 30.0,
                max_chasers: 1,
            }
        );
    }

    #[test]
    fn an_own_target_is_swapped_only_for_a_much_closer_one() {
        let mut sense = sense(20.0);
        sense.repath_due = true;
        // ближе текущей цели на треть, зато делить кандидата с соседом можно
        assert_eq!(
            asked_rule(&sense),
            SwitchRule {
                radius: 14.0,
                max_chasers: MAX_CHASERS_PER_TARGET,
            }
        );
    }

    /// Ступень, которой раньше не было в лестнице вовсе: нашёлся кандидат —
    /// решение называет его само, а не оставляет применению доискиваться.
    #[test]
    fn a_found_candidate_becomes_the_new_target() {
        let mut sense = sense(20.0);
        sense.repath_due = true;
        let victim = Victim {
            entity: Entity::from_raw_u32(7).expect("entity"),
            position: Vec2::new(5.0, 0.0),
        };

        assert_eq!(
            decide(&sense, || false, |_| Some(victim)),
            ChaseAction::Switch { to: victim }
        );
    }

    /// Поиск дорог, и лестница обязана не звать его на ступенях, до которых
    /// сама не дошла: бросок, ожидание первого пути, выход из погони.
    #[test]
    fn the_search_is_not_asked_before_the_repath_rung() {
        let asked = std::cell::Cell::new(false);
        let ask = |sense: &ChaseSense| {
            asked.set(false);
            decide(
                sense,
                || true,
                |_| {
                    asked.set(true);
                    None
                },
            );
            asked.get()
        };

        assert!(!ask(&sense(1.0)), "убийство");
        assert!(!ask(&sense(DEMON_LUNGE_RANGE - 1.0)), "бросок");
        assert!(!ask(&sense(20.0)), "такт не настал");

        let mut due = sense(20.0);
        due.repath_due = true;
        assert!(ask(&due), "такт перепрокладки — здесь поиск обязан быть");
    }

    /// Место в очереди на жертву освобождает ровно один выход — тот, после
    /// которого жертва остаётся живой и достаётся кому-то ещё.
    #[test]
    fn only_giving_up_frees_the_slot_on_the_victim() {
        assert!(ChaseAction::GaveUp.releases_claim());
        assert!(!ChaseAction::LostTarget.releases_claim());
        assert!(!ChaseAction::Kill.releases_claim());
    }

    /// Погоня продолжается — место остаётся занятым. Смена жертвы в такт
    /// перепрокладки места тоже не освобождает: там их сразу два, старое и
    /// новое, и ведёт их применение (`behavior::chase`), а не эта ступень.
    #[test]
    fn staying_in_the_chase_keeps_the_slot() {
        for action in [
            ChaseAction::Lunge { advance: Vec2::X },
            ChaseAction::WaitForPath,
            ChaseAction::Hold,
            ChaseAction::Repath { target: Vec2::X },
            // смена — тоже «остался в погоне»: место не освобождается, а
            // переезжает одним `ChaseClaims::transfer`
            ChaseAction::Switch {
                to: Victim {
                    entity: Entity::from_raw_u32(1).expect("entity"),
                    position: Vec2::X,
                },
            },
        ] {
            assert!(!action.releases_claim(), "{action:?} держит своё место");
        }
    }
}
