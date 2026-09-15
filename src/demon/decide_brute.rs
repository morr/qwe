//! Решение Громилы, отделённое от применения — та же форма, что лестница
//! погони (`decide.rs`): [`BruteSense`] — что Громила знает о себе и о цели
//! на этом тике, чистый [`decide`], [`BruteAction`] — что из этого следует.
//! Применение — `besiege.rs`.
//!
//! Ступени: (1) есть цель — руина или исчезла → снять; в досягаемости →
//! стоять и бить (сам удар — `combat::strike`); иначе идти, перепрокладка по
//! такту; (2) цели нет → ближайший **фронтовой** бастион со свободным местом
//! (фронт — стоящий бастион в не осквернённом районе с осквернённым соседом;
//! кто фронтовой, знает применение, лестница только спрашивает ближайший);
//! (3) фронта нет → блуждание по демонской политике.

use bevy::prelude::*;

use crate::demon::decide::{PathRung, PathSense, path_rung};
use crate::settings::ATTACK_REACH;

/// Лимит Громил на один бастион: расходятся по фронту, а не ломают один
/// втроём, пока соседний стоит. Правило лестницы, в терминах которого
/// написан `BastionClaims` (`claims.rs`), — потому здесь, а не в `settings.rs`.
pub const MAX_BRUTES_PER_BASTION: usize = 3;

/// Всё, из чего складывается решение Громилы на одном тике.
#[derive(Clone, Debug)]
pub struct BruteSense {
    pub position: Vec2,
    /// Цель — или `None`, если цели нет. Исчезнувшая цель (сущность
    /// despawn'нута) тоже `None`; что `AttackTarget` при этом ещё висит,
    /// знает применение и снимает его как [`BruteAction::Done`].
    pub target: Option<BruteTarget>,
    /// Путь к бастиону — то же чувство, что у погони.
    pub path: PathSense,
}

/// Что Громила знает о своей цели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BruteTarget {
    pub position: Vec2,
    /// Уже руина — бить нечего.
    pub ruined: bool,
}

/// Бастион фронта, который нашёл поиск.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Front {
    pub entity: Entity,
    pub position: Vec2,
}

/// Что Громила делает на этом тике.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BruteAction {
    /// Цель сломана или исчезла — снять её; фронт спросится на следующем
    /// тике, уже без неё.
    Done,
    /// В досягаемости удара: стоять; удар наносит `combat::strike`.
    Strike,
    /// Первого пути ещё нет, поиск в полёте — ждать (см. `decide::WaitForPath`).
    WaitForPath,
    /// Идём по тому, что есть: такт перепрокладки не настал.
    Hold,
    /// Такт перепрокладки: путь к точке бастиона.
    Repath { target: Vec2 },
    /// Цели нет, фронт есть: занять место у ближайшего бастиона фронта.
    Engage { to: Front },
    /// Фронта нет — блуждание по демонской политике.
    Wander,
}

/// Лестница Громилы. Поиск фронта — чувство, спрашивается лениво и только на
/// ступени без цели: список фронтовых бастионов один на тик, но выбирать из
/// него ближайший каждому Громиле с целью незачем.
pub fn decide(sense: &BruteSense, nearest_front: impl FnOnce() -> Option<Front>) -> BruteAction {
    let Some(target) = sense.target else {
        return match nearest_front() {
            Some(to) => BruteAction::Engage { to },
            None => BruteAction::Wander,
        };
    };
    if target.ruined {
        return BruteAction::Done;
    }

    if sense.position.distance(target.position) <= ATTACK_REACH {
        return BruteAction::Strike;
    }

    // те же ступени пути, что у погони: перепрокладка уронила бы ещё не
    // отвеченный первый поиск, и Громила стоял бы у портала, пока конвейер
    // медленнее тика
    match path_rung(&sense.path) {
        Some(PathRung::WaitForPath) => return BruteAction::WaitForPath,
        Some(PathRung::Hold) => return BruteAction::Hold,
        None => {}
    }
    BruteAction::Repath {
        target: target.position,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::MovableState;

    fn front() -> Front {
        Front {
            entity: Entity::from_raw_u32(7).expect("entity"),
            position: Vec2::new(100.0, 0.0),
        }
    }

    /// Громила в начале координат, цель по оси X на `distance` метров, обычный
    /// тик похода: путь есть, такт перепрокладки не настал.
    fn sense(distance: f32) -> BruteSense {
        BruteSense {
            position: Vec2::ZERO,
            target: Some(BruteTarget {
                position: Vec2::new(distance, 0.0),
                ruined: false,
            }),
            path: PathSense {
                state: MovableState::Moving(IVec2::ZERO),
                has_path: true,
                walked: true,
                search_in_flight: false,
                repath_due: false,
            },
        }
    }

    #[test]
    fn without_a_target_the_brute_takes_the_front_or_wanders() {
        let mut sense = sense(10.0);
        sense.target = None;
        assert_eq!(
            decide(&sense, || Some(front())),
            BruteAction::Engage { to: front() }
        );
        assert_eq!(decide(&sense, || None), BruteAction::Wander);
    }

    #[test]
    fn a_ruin_is_dropped_before_anything_else() {
        let mut sense = sense(1.0);
        sense.target.as_mut().unwrap().ruined = true;
        assert_eq!(decide(&sense, || Some(front())), BruteAction::Done);
    }

    #[test]
    fn within_reach_the_brute_stands_and_strikes() {
        assert_eq!(decide(&sense(ATTACK_REACH), || None), BruteAction::Strike);
        assert_ne!(
            decide(&sense(ATTACK_REACH + 0.1), || None),
            BruteAction::Strike
        );
    }

    #[test]
    fn the_first_path_is_awaited_then_held_then_repathed() {
        let mut waiting = sense(50.0);
        waiting.path.state = MovableState::Pathfinding(IVec2::ZERO);
        waiting.path.has_path = false;
        waiting.path.walked = false;
        waiting.path.search_in_flight = true;
        assert_eq!(decide(&waiting, || None), BruteAction::WaitForPath);

        assert_eq!(decide(&sense(50.0), || None), BruteAction::Hold);

        let mut due = sense(50.0);
        due.path.repath_due = true;
        assert_eq!(
            decide(&due, || None),
            BruteAction::Repath {
                target: Vec2::new(50.0, 0.0)
            }
        );
        let mut idle = sense(50.0);
        idle.path.state = MovableState::Idle;
        assert!(matches!(decide(&idle, || None), BruteAction::Repath { .. }));
    }
}
