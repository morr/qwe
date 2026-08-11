//! Шаг по пути — проверяется значениями, без `App`.
//!
//! Ради этого он и вынесен: до выноса те же правила стоили ~44 строк фикстуры
//! (девять плагинов, пять ресурсов навигации, ручное `Time<Virtual>`) на
//! проверку, а скольжение и порог отпускания курса не проверялись вовсе —
//! обе ручки по умолчанию выключены, и целым `App` до них было не добраться.

use std::sync::{Arc, RwLock};

use super::*;
use crate::navigation::{Backend, Navmesh};

/// Бэкенд с перечисленными непроходимыми тайлами — нужен шагу только под
/// докат.
fn backend(blocked: &[IVec2]) -> Backend {
    let mut navmesh = Navmesh::default();
    for tile in blocked {
        navmesh.set_passable(tile.x, tile.y, false);
    }
    Backend::from_grid(Arc::new(RwLock::new(navmesh)))
}

/// Настройки шага по умолчанию: обе лабораторные ручки выключены, как в игре.
fn tuning() -> StepTuning {
    StepTuning {
        rest: 1.0,
        arrive_slack: 1.0,
        steer_release: 2.0,
        slide: 0.0,
    }
}

/// Идущая сущность со скоростью 1 м/с: за `dt` секунд проходит `dt` метров.
fn walker(path: &[Vec2], target: IVec2) -> Movable {
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::Moving(target);
    movable.path = path.iter().copied().collect();
    movable
}

// --- ход по пути ---

#[test]
fn a_step_advances_towards_the_waypoint_and_records_the_course() {
    let backend = backend(&[]);
    let mut movable = walker(&[Vec2::new(10.0, 0.0)], IVec2::new(5, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::new(1.0, 0.0));
    assert_eq!(movable.last_direction, Vec2::new(1.0, 0.0));
    assert_eq!(movable.path.len(), 1, "waypoint ещё не достигнут");
}

/// Остаток времени переносится через waypoint: один тик проходит несколько
/// точек, а не останавливается на первой.
#[test]
fn one_step_walks_through_several_waypoints() {
    let backend = backend(&[]);
    let path = [
        Vec2::new(1.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(10.0, 0.0),
    ];
    let mut movable = walker(&path, IVec2::new(5, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        3.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::new(3.0, 0.0));
    assert_eq!(movable.path.len(), 1, "две первые точки пройдены");
}

// --- приход ---

#[test]
fn an_empty_path_on_the_target_tile_arrives() {
    let backend = backend(&[]);
    let target = IVec2::new(5, 5);
    let mut movable = walker(&[], target);
    let mut position = tile_center(target);

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(
        outcome,
        StepOutcome::Arrived {
            destination_reached: true
        }
    );
}

#[test]
fn an_empty_path_far_from_the_target_stops_without_arriving() {
    let backend = backend(&[]);
    let mut movable = walker(&[], IVec2::new(50, 50));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(
        outcome,
        StepOutcome::Arrived {
            destination_reached: false
        }
    );
}

// --- докат ---

#[test]
fn an_exhausted_path_while_repathing_coasts_along_the_last_course() {
    let backend = backend(&[]);
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::Pathfinding(IVec2::new(50, 50));
    movable.last_direction = Vec2::new(1.0, 0.0);
    let mut position = Vec2::new(20.0, 20.0);

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::new(21.0, 20.0));
}

#[test]
fn coasting_into_an_impassable_tile_halts() {
    let start = Vec2::new(20.0, 20.0);
    let ahead = world_to_tile(start + Vec2::new(1.0, 0.0));
    let backend = backend(&[ahead]);
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::Pathfinding(IVec2::new(50, 50));
    movable.last_direction = Vec2::new(1.0, 0.0);
    let mut position = start;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Halted);
    assert_eq!(position, start, "упёршийся не сдвинулся");
}

#[test]
fn coasting_without_a_course_halts() {
    let backend = backend(&[]);
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::Pathfinding(IVec2::new(50, 50));
    let mut position = Vec2::new(20.0, 20.0);

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Halted);
}

#[test]
fn a_failed_search_with_an_empty_path_halts() {
    let backend = backend(&[]);
    let mut movable = Movable::new(1.0);
    movable.state = MovableState::PathfindingError(IVec2::new(50, 50));
    movable.last_direction = Vec2::new(1.0, 0.0);
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers::default(),
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Halted, "докатывать некуда");
    assert_eq!(position, Vec2::ZERO);
}

// --- придержка ---

#[test]
fn a_hold_shortens_the_step_by_its_factor() {
    let backend = backend(&[]);
    let mut movable = walker(&[Vec2::new(10.0, 0.0)], IVec2::new(5, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            hold: Some(0.25),
            ..default()
        },
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::new(0.25, 0.0));
}

/// Придержанный у самой цели дошёл: ближе его не пустит то самое тело, а без
/// засчитанного прихода он толкался бы с ним до скончания века.
#[test]
fn a_held_pawn_within_the_slack_arrives_and_drops_its_path() {
    let backend = backend(&[]);
    let target = IVec2::new(5, 5);
    let mut movable = walker(&[tile_center(target)], target);
    let mut position = tile_center(target) + Vec2::new(0.5, 0.0);

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            hold: Some(0.25),
            ..default()
        },
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(
        outcome,
        StepOutcome::Arrived {
            destination_reached: true
        }
    );
    assert!(
        movable.path.is_empty(),
        "путь чистится шагом — иначе событие прихода не сработает"
    );
}

// --- доворот курса ---

/// Ключевой инвариант: в `last_direction` пишется ЖЕЛАЕМЫЙ курс, а не
/// отклонённый. Иначе доворот считался бы от уже довёрнутого и пешка наматывала
/// бы круги вместо обхода.
#[test]
fn steering_deflects_the_step_but_not_the_recorded_course() {
    let backend = backend(&[]);
    let mut movable = walker(&[Vec2::new(100.0, 0.0)], IVec2::new(50, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            aside: Vec2::new(0.0, 1.0),
            ..default()
        },
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(
        movable.last_direction,
        Vec2::new(1.0, 0.0),
        "записан желаемый курс, до отклонения"
    );
    assert!(position.y > 0.0, "а шаг всё же отклонён вбок");
    assert!(
        (position.length() - 1.0).abs() < 1e-5,
        "длина шага сохранена"
    );
}

/// У самого waypoint'а курс не доворачивается: отклонённый шаг не сокращает
/// остаток до точки, и пешка не дошла бы до неё никогда.
#[test]
fn steering_releases_near_the_waypoint() {
    let backend = backend(&[]);
    let waypoint = Vec2::new(1.5, 0.0);
    let mut movable = walker(&[waypoint], IVec2::new(50, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            aside: Vec2::new(0.0, 1.0),
            ..default()
        },
        // остаток до точки (1.5) меньше порога отпускания (2.0)
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::new(1.0, 0.0), "шаг не отклонён");
}

// --- скольжение по контакту ---

/// Лобовой контакт при полном скольжении съедает шаг целиком: пешка упирается,
/// а не продавливает чужое тело.
#[test]
fn sliding_removes_the_component_into_the_body() {
    let backend = backend(&[]);
    let mut movable = walker(&[Vec2::new(10.0, 0.0)], IVec2::new(5, 0));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            barrier: Vec2::new(1.0, 0.0),
            ..default()
        },
        StepTuning {
            slide: 1.0,
            ..tuning()
        },
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert_eq!(position, Vec2::ZERO, "весь шаг был направлен в тело");
}

/// Поперечная составляющая остаётся целиком, а длину шага НЕ восстанавливаем:
/// иначе лобовая пешка выстреливала бы вбок на полной скорости.
#[test]
fn sliding_keeps_the_transverse_component_without_restoring_the_length() {
    let backend = backend(&[]);
    let diagonal = Vec2::new(10.0, 10.0);
    let mut movable = walker(&[diagonal], IVec2::new(5, 5));
    let mut position = Vec2::ZERO;

    let outcome = step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            barrier: Vec2::new(1.0, 0.0),
            ..default()
        },
        StepTuning {
            slide: 1.0,
            ..tuning()
        },
        &backend.walkable(),
    );

    assert_eq!(outcome, StepOutcome::Moved);
    assert!(position.x.abs() < 1e-5, "составляющая в тело снята");
    assert!(
        (position.y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5,
        "поперечная осталась как есть, длина не восстановлена"
    );
}

/// Ручка выключена (по умолчанию именно так) — контакт на шаг не влияет.
#[test]
fn a_barrier_does_nothing_while_sliding_is_off() {
    let backend = backend(&[]);
    let mut movable = walker(&[Vec2::new(10.0, 0.0)], IVec2::new(5, 0));
    let mut position = Vec2::ZERO;

    step_along_path(
        &mut movable,
        &mut position,
        1.0,
        StepModifiers {
            barrier: Vec2::new(1.0, 0.0),
            ..default()
        },
        tuning(),
        &backend.walkable(),
    );

    assert_eq!(position, Vec2::new(1.0, 0.0));
}
