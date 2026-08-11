//! Решение бегства, отделённое от его применения.
//!
//! Тот же раздел, что и у погони демона (`demon::decide`): [`FleeSense`] —
//! что человек знает о себе на этом тике, [`FleeAction`] — что из этого
//! следует, а `Commands`, таймеры и заявки на путь остаются в
//! `behavior::flee`. Проверка бегства раньше требовала `App` с навмешем,
//! пространственной сеткой демонов и `Time`; здесь она — структура на входе и
//! вариант перечисления на выходе.
//!
//! Дорогие вопросы задаются лениво и ровно один раз, [`ThreatProbe`] —
//! которым именно. Это не оптимизация задним числом, а само правило: точный
//! поиск ближайшего демона стоил 40% тика симуляции, когда бежал у каждого
//! бегущего каждый тик, и на тиках без решения его заменяет грубая проверка
//! занятости ячеек.

use bevy::prelude::*;

use crate::rng::hash_fraction;
use crate::settings::MAP_SIZE;

/// Шаг бегства: насколько далеко от себя прокладывается точка «от демона», м.
pub const FLEE_STEP: (f32, f32) = (40.0, 60.0);
/// Зона у границы карты, попадание в которую при бегстве — «спасся», м.
pub const ESCAPE_MARGIN: f32 = 2.0;
/// Веер разбегания: максимальное отклонение от вектора «прочь от демона»,
/// радианы (±≈34°). Толпа без него выстраивается в колонну.
const FLEE_SPREAD: f32 = 0.6;

/// Всё, из чего складывается решение бегства на одном тике.
#[derive(Clone, Copy, Debug)]
pub struct FleeSense {
    pub position: Vec2,
    pub pawn_id: u32,
    /// За этим человеком гонится демон прямо сейчас — такие бегут по чистому
    /// вектору от демона, без веера.
    pub chased: bool,
    /// Таймер перепрокладки досчитал на этом тике.
    pub repath_due: bool,
    /// Путь потерян (`Idle` или ошибка поиска) — новый нужен независимо от
    /// таймера.
    pub needs_path: bool,
}

/// Каким вопросом проверять угрозу на этом тике.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatProbe {
    /// Тик без решения — у подавляющего большинства бегущих. Хватает грубой
    /// проверки занятости ячеек демонской сетки, накрывающих радиус: пустое
    /// окно гарантирует «в радиусе никого», и успокоение срабатывает как
    /// раньше; занятое окно у уже безопасного бегущего (демон завис в
    /// 90–150 м) откладывает успокоение до его тика решения — не дольше
    /// периода перепрокладки.
    Cells,
    /// Тик решения: точный поиск ближайшего демона, раз в 45–77 тиков на
    /// бегущего.
    Nearest,
}

/// Ответ на [`ThreatProbe`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Threat {
    /// В радиусе никого.
    None,
    /// Демон в радиусе есть, но где именно — грубая проверка не знает.
    Near,
    /// Ближайший демон здесь.
    At(Vec2),
}

/// Что человек делает на этом тике.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FleeAction {
    /// Демоны отстали: мирный шаг, пауза «отдышаться» и возврат в Wander.
    CalmDown,
    /// Угроза рядом, но решать нечего — бежим дальше по тому, что есть.
    Hold,
    /// Прокладываем новый путь бегства; `away` — единичный курс прочь от
    /// демона, уже с личным углом веера.
    Flee { away: Vec2 },
}

/// Лестница бегства. `threat` спрашивается ровно один раз — тем вопросом,
/// который решение сочтёт уместным на этом тике (см. [`ThreatProbe`]).
pub fn decide(sense: &FleeSense, threat: impl FnOnce(ThreatProbe) -> Threat) -> FleeAction {
    let probe = if sense.repath_due || sense.needs_path {
        ThreatProbe::Nearest
    } else {
        ThreatProbe::Cells
    };
    match threat(probe) {
        // демоны далеко — мирный режим, отдышаться перед новой прогулкой
        Threat::None => FleeAction::CalmDown,
        Threat::Near => FleeAction::Hold,
        Threat::At(demon) => {
            let mut away = (sense.position - demon).normalize_or(Vec2::X);
            // не преследуемые разбегаются веером — каждый под своим углом
            if !sense.chased {
                away = Vec2::from_angle(personal_spread(sense.pawn_id)).rotate(away);
            }
            FleeAction::Flee { away }
        }
    }
}

/// Куда бежать: точка в `step` метрах по курсу, прижатая к карте.
///
/// К «безопасной» зоне не клампим намеренно: цель у самой границы — путь к
/// спасению, и отступ здесь заведомо меньше [`ESCAPE_MARGIN`], иначе толпа
/// сбегалась бы к краю и никогда за него не выходила.
pub fn flee_target(position: Vec2, away: Vec2, step: f32) -> Vec2 {
    (position + away * step).clamp(Vec2::splat(1.0), MAP_SIZE - 1.0)
}

/// Бегущий пересёк границу карты — «спасся».
pub fn escaped(position: Vec2) -> bool {
    position.x <= ESCAPE_MARGIN
        || position.y <= ESCAPE_MARGIN
        || position.x >= MAP_SIZE.x - ESCAPE_MARGIN
        || position.y >= MAP_SIZE.y - ESCAPE_MARGIN
}

/// Персональный угол веера: детерминирован по `PawnId`, чтобы человек между
/// перепрокладками держал свою сторону, а не зигзагами метался.
///
/// Именно `PawnId`, а не `Entity`: индексы сущностей после рестарта
/// переиспользуются в другом порядке (свободный список зависит от того, кого
/// съели в прошлом прогоне), и веер разъехался бы между прогоном и его
/// повтором при абсолютно одинаковом seed.
fn personal_spread(pawn_id: u32) -> f32 {
    let hash = pawn_id.wrapping_mul(2654435761);
    (hash_fraction(hash) * 2.0 - 1.0) * FLEE_SPREAD
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Человек в середине карты, за которым никто не гонится, на тике без
    /// решения.
    fn sense() -> FleeSense {
        FleeSense {
            position: MAP_SIZE / 2.0,
            pawn_id: 7,
            chased: false,
            repath_due: false,
            needs_path: false,
        }
    }

    /// Угроза, которая запоминает, каким вопросом её спросили.
    fn asked(
        answer: Threat,
    ) -> (
        impl FnOnce(ThreatProbe) -> Threat,
        std::rc::Rc<Cell<Option<ThreatProbe>>>,
    ) {
        let probe = std::rc::Rc::new(Cell::new(None));
        let seen = probe.clone();
        (
            move |probe| {
                seen.set(Some(probe));
                answer
            },
            probe,
        )
    }

    #[test]
    fn a_tick_without_a_decision_only_checks_the_cells() {
        let (threat, probe) = asked(Threat::Near);
        decide(&sense(), threat);
        assert_eq!(probe.get(), Some(ThreatProbe::Cells));
    }

    #[test]
    fn a_decision_tick_asks_for_the_nearest_demon() {
        let by_timer = FleeSense {
            repath_due: true,
            ..sense()
        };
        // потерянный путь — тоже решение, и такта перепрокладки он не ждёт
        let by_lost_path = FleeSense {
            needs_path: true,
            ..sense()
        };
        for sense in [by_timer, by_lost_path] {
            let (threat, probe) = asked(Threat::None);
            decide(&sense, threat);
            assert_eq!(probe.get(), Some(ThreatProbe::Nearest));
        }
    }

    #[test]
    fn an_empty_radius_calms_the_human_down() {
        assert_eq!(decide(&sense(), |_| Threat::None), FleeAction::CalmDown);
    }

    #[test]
    fn an_occupied_cell_keeps_the_human_running() {
        assert_eq!(decide(&sense(), |_| Threat::Near), FleeAction::Hold);
    }

    #[test]
    fn the_chased_run_straight_away_from_the_demon() {
        let mut sense = sense();
        sense.chased = true;
        let demon = sense.position - Vec2::X * 10.0;
        let FleeAction::Flee { away } = decide(&sense, |_| Threat::At(demon)) else {
            panic!("демон рядом — бежим");
        };
        assert!((away - Vec2::X).length() < 1e-5);
    }

    #[test]
    fn the_rest_fan_out_within_the_spread() {
        let demon = sense().position - Vec2::X * 10.0;
        let mut angles = Vec::new();
        for pawn_id in 0..64 {
            let mut sense = sense();
            sense.pawn_id = pawn_id;
            let FleeAction::Flee { away } = decide(&sense, |_| Threat::At(demon)) else {
                panic!("демон рядом — бежим");
            };
            let angle = away.to_angle();
            assert!(
                angle.abs() <= FLEE_SPREAD + 1e-5,
                "веер не шире ±{FLEE_SPREAD} рад, а тут {angle}"
            );
            angles.push(angle);
        }
        // толпа не выстраивается в колонну: углы действительно разные
        angles.sort_by(f32::total_cmp);
        let spread = angles.last().unwrap() - angles.first().unwrap();
        assert!(spread > FLEE_SPREAD, "разброс всего {spread} рад");
    }

    #[test]
    fn the_personal_angle_does_not_change_between_repaths() {
        let demon = sense().position - Vec2::X * 10.0;
        let once = decide(&sense(), |_| Threat::At(demon));
        let again = decide(&sense(), |_| Threat::At(demon));
        assert_eq!(once, again);
    }

    #[test]
    fn a_human_standing_on_the_demon_still_gets_a_course() {
        let mut sense = sense();
        sense.chased = true;
        let FleeAction::Flee { away } = decide(&sense, |_| Threat::At(sense.position)) else {
            panic!("демон рядом — бежим");
        };
        assert_eq!(away, Vec2::X);
    }

    #[test]
    fn a_flee_target_never_leaves_the_map() {
        for corner in [Vec2::ZERO, MAP_SIZE, Vec2::new(0.0, MAP_SIZE.y)] {
            let away = (corner - MAP_SIZE / 2.0).normalize();
            let target = flee_target(MAP_SIZE / 2.0, away, MAP_SIZE.length());
            assert!(target.cmpge(Vec2::ONE).all() && target.cmple(MAP_SIZE - 1.0).all());
        }
    }

    #[test]
    fn a_target_at_the_edge_lies_inside_the_escape_zone() {
        // иначе бегущие сбивались бы в полосу у границы и не спасались:
        // путь есть, а спасение не срабатывает
        for away in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
            let target = flee_target(MAP_SIZE / 2.0, away, MAP_SIZE.length());
            assert!(escaped(target), "{target:?} — у самого края, а не спасение");
        }
    }

    #[test]
    fn the_middle_of_the_map_is_not_an_escape() {
        assert!(!escaped(MAP_SIZE / 2.0));
    }
}
