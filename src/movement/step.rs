//! Шаг пешки по готовому пути — то, что `move_moving_entities` считает, в
//! отрыве от того, как оно достаётся из мира.
//!
//! Здесь живут все пять правил шага разом: приход, докат за концом пути,
//! придержка, доворот курса и скольжение по контакту. До выноса они были
//! вперемешку с обходом запроса, тремя ресурсами расталкивания и ведением
//! сетки людей — и проверялись только целым `App`, из-за чего скольжение
//! (лабораторная ручка, по умолчанию выключенная) не проверялось вовсе.
//!
//! Наружу отсюда не уходит ничего от ECS: ни `Entity`, ни `Commands`, ни
//! запросов. Что делать с исходом — решает система (тот же приём, что у
//! `human::decide` / `demon::decide`).

use bevy::prelude::*;

use crate::grid::{tile_center, world_to_tile};
use crate::movement::components::{Movable, MovableState};
use crate::navigation::Walkable;

/// Исход шага — что система обязана сделать с сущностью после него.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepOutcome {
    /// Сущность продолжает движение; трогать её не нужно.
    Moved,
    /// Идти больше некуда: путь кончился в состоянии `Moving`. Флаг — считать
    /// ли это приходом в цель (событие прихода в `Movable::to_idle` гейтится
    /// ещё и пустым путём, поэтому шаг чистит путь сам).
    Arrived { destination_reached: bool },
    /// Движение прекращается без прихода: докат упёрся в непроходимое или
    /// стоит на месте, либо состояние вообще не подразумевает ходьбы. Системе
    /// остаётся снять `MovableStateMovingTag`.
    Halted,
}

/// Что расталкивание навязало шагу ЭТОЙ пешки на этот прогон — её доля общей
/// выдачи механизма (`separation::SeparationOutput`).
#[derive(Clone, Copy, Default, Debug)]
pub struct StepModifiers {
    /// Придержка: `Some(множитель времени шага)`. Упёршийся курсом в чужое
    /// тело не давит в него полным шагом — и доходит до цели с допуском, ведь
    /// ближе его не пустит то самое тело, а без засчитанного прихода он
    /// толкался бы с ним до скончания века.
    pub hold: Option<f32>,
    /// Доворот курса вбок: упёршаяся пешка не ждёт толчка, а обходит на полной
    /// скорости.
    pub aside: Vec2,
    /// Единичный вектор на перегородившего курс. Вдоль него шаг срезается —
    /// то, чем пешка лезет В тело, снимается, поперечное остаётся целиком.
    pub barrier: Vec2,
}

/// Настройки шага, общие для вида: дистанция покоя и ручки стенда
/// расталкивания.
#[derive(Clone, Copy, Debug)]
pub struct StepTuning {
    /// Дистанция покоя — ближе неё расталкивание пешек друг к другу не
    /// подпускает, поэтому приход засчитывается с этим допуском.
    pub rest: f32,
    /// Во сколько дистанций покоя обходится допуск прихода придержанному
    /// ([`separation::SeparationLab::arrive_slack`](crate::movement::separation::SeparationLab)).
    pub arrive_slack: f32,
    /// Ближе этого остатка до waypoint'а курс не доворачивается: отклонённый
    /// шаг не сокращает остаток до точки, а снимается она только когда бюджет
    /// шага этот остаток накрывает.
    pub steer_release: f32,
    /// Какая доля составляющей «в тело» снимается со шага. 0 — скольжения нет.
    pub slide: f32,
}

/// Один шаг сущности по её пути.
///
/// Двигает `position` и подъедает `movable.path`; `movable.last_direction`
/// пишется ЖЕЛАЕМЫМ курсом, до отклонения (см. ниже — иначе доворот
/// накапливается сам на себя).
pub fn step_along_path(
    movable: &mut Movable,
    position: &mut Vec2,
    dt: f32,
    modifiers: StepModifiers,
    tuning: StepTuning,
    walkable: &Walkable,
) -> StepOutcome {
    let mut remaining_time = dt;
    if let Some(hold) = modifiers.hold {
        let slack = tuning.rest * tuning.arrive_slack;
        if let MovableState::Moving(target) = movable.state
            && (tile_center(target) - *position).length_squared() <= slack * slack
        {
            movable.path.clear();
            return StepOutcome::Arrived {
                destination_reached: true,
            };
        }
        remaining_time *= hold;
    }

    loop {
        if movable.path.is_empty() {
            return match movable.state {
                MovableState::Moving(target) => StepOutcome::Arrived {
                    destination_reached: world_to_tile(*position) == target
                        || (tile_center(target) - *position).length_squared()
                            <= tuning.rest * tuning.rest,
                },
                // старый путь пройден раньше, чем посчитан новый — докат
                MovableState::Pathfinding(_) => {
                    let step = movable.last_direction * movable.speed * remaining_time;
                    let coasted = *position + step;
                    // стоять на месте (нулевой вектор) или упереться в
                    // непроходимое (за картой — непроходимо само по себе) —
                    // конец доката
                    if step == Vec2::ZERO || !walkable.coast_allows(coasted) {
                        StepOutcome::Halted
                    } else {
                        *position = coasted;
                        StepOutcome::Moved
                    }
                }
                // ошибка поиска или явная остановка — поведение выберет новую
                // цель, докатывать некуда
                _ => StepOutcome::Halted,
            };
        }

        let target = *movable.path.front().expect("path is non-empty");
        let to_target = target - *position;
        let distance = to_target.length();
        let distance_to_move = movable.speed * remaining_time;

        if distance_to_move < distance {
            let direction = to_target.normalize_or_zero();
            // `last_direction` — ЖЕЛАЕМЫЙ курс, до отклонения. Записать сюда
            // отклонённый нельзя: расталкивание берёт сторону обхода как
            // правую нормаль к `last_direction`, и курс доворачивался бы
            // вправо от уже довёрнутого — каждый кадр ещё на столько же.
            // Пешка при этом не отходит вбок, а наматывает круги, и сила
            // отклонения перестаёт на что-либо влиять (замер стенда: разброс
            // 0.11 м одинаково при 0.3 и при 1.5)
            movable.last_direction = direction;
            // …и не у самого waypoint'а, см. `StepTuning::steer_release`
            let step = if modifiers.aside != Vec2::ZERO && distance > tuning.steer_release {
                (direction + modifiers.aside).normalize_or_zero()
            } else {
                direction
            };
            // скольжение по контакту. Длину НЕ восстанавливаем — иначе лобовая
            // пешка, у которой поперечного почти нет, выстреливала бы вбок на
            // полной скорости
            let step = if modifiers.barrier != Vec2::ZERO {
                step - modifiers.barrier * (step.dot(modifiers.barrier).max(0.0) * tuning.slide)
            } else {
                step
            };
            *position += step * distance_to_move;
            return StepOutcome::Moved;
        }

        // дошли до waypoint'а — встаём на него и тратим остаток времени
        if distance > 0.0 {
            movable.last_direction = to_target / distance;
        }
        *position = target;
        movable.path.pop_front();
        remaining_time -= distance / movable.speed;
        if remaining_time <= 0.0 {
            return StepOutcome::Moved;
        }
    }
}

#[cfg(test)]
mod tests;
