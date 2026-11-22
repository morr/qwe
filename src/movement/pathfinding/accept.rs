//! Приёмка ОДНОГО ответа поиска пути — семантика ответа отдельной единицей:
//! учёт в диагностике, отброс устаревшего, спасение застрявшего, приход и
//! подрезка пути под уже пройденное. Оба приёмника
//! ([`super::apply_pathfinding_results`] и
//! [`super::listen_for_pathfinding_tasks`]) добывают ответ каждый своим
//! способом и передают его сюда; что с ответом делать — только здесь.

use bevy::prelude::*;

use crate::movement::components::{MovableState, PawnEdit};
use crate::movement::systems::rescue_from_impassable;
use crate::navigation::{Backend, PathfindingResult, Walkable};
use crate::settings::REPATH_TRIM_LIMIT;

/// Счётчики приёмки за один прогон системы: замер длительности поиска и пара
/// «ответов / отказов».
///
/// Пишутся в [`Drop`], а не в конце системы, и это единственная причина, по
/// которой тип существует. Инвариант панели — **оба счётчика каждый прогон, в
/// том числе нулями**: доля считается как отношение средних по двум историям,
/// и верно это, только пока истории одной длины (см.
/// [`crate::diagnostics::PATHFINDING_ANSWERED`]). Дисциплиной он уже не
/// удержался дважды: сперва замер писали только в кадрах с ответами и панель
/// застывала на последнем значении, потом ранний выход
/// `if due.is_empty() { return; }` вернул ровно то же в детерминированном
/// приёмнике. `Drop` делает нарушение невыразимым — из системы нельзя выйти
/// мимо него.
pub(super) struct AnswerTally<'a, 'w, 's> {
    diagnostics: &'a mut bevy::diagnostic::Diagnostics<'w, 's>,
    answered: u32,
    failed: u32,
}

impl<'a, 'w, 's> AnswerTally<'a, 'w, 's> {
    pub(super) fn new(diagnostics: &'a mut bevy::diagnostic::Diagnostics<'w, 's>) -> Self {
        Self {
            diagnostics,
            answered: 0,
            failed: 0,
        }
    }

    /// Учесть один снятый ответ — чем бы он ни кончился дальше: устаревший и
    /// недостижимый ответы тоже посчитаны, они стоили поиска.
    fn record(&mut self, result: &PathfindingResult) {
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_DURATION_MS, || {
                result.duration.as_secs_f64() * 1000.0
            });
        self.answered += 1;
        self.failed += u32::from(result.path.is_none());
    }
}

impl Drop for AnswerTally<'_, '_, '_> {
    fn drop(&mut self) {
        let (answered, failed) = (self.answered, self.failed);
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_ANSWERED, || {
                answered as f64
            });
        self.diagnostics
            .add_measurement(&crate::diagnostics::PATHFINDING_FAILED, || failed as f64);
    }
}

/// Приёмка ОДНОГО ответа поиска пути — общее тело обоих приёмников: учёт в
/// счётчиках и применение к пешке.
///
/// Вынесено намеренно: режимы отличаются только тем, КОГДА ответ снимается, а
/// не тем, что с ним делают. Две копии этой логики разъехались бы на первой же
/// правке, и детерминированный режим начал бы вести себя иначе не потому, что
/// он детерминированный. Что у режимов и правда своё — способ добыть ответ
/// (`check_ready` со сторожком зависания против `block_on` со счётом простоя в
/// `SimLoad`) и порядок обхода; всё остальное живёт здесь.
///
/// `walkable` — ленивый взгляд на проходимость бэкенда: его read-лок во время
/// загрузки на секунды держит на запись поток заливки, а нужен он только под
/// спасение застрявших.
///
/// `pawn` — [`PawnEdit`]: тождество, три компонента, которые спасение правит
/// вместе, и буфер команд. Ровно то же принимает `rescue_from_impassable`,
/// которому эта приёмка пешку и передаёт.
#[allow(clippy::too_many_arguments)]
pub(super) fn accept_answer<'backend>(
    result: PathfindingResult,
    tally: &mut AnswerTally<'_, '_, '_>,
    mut pawn: PawnEdit<'_, '_, '_>,
    is_human: bool,
    is_demon: bool,
    walkable: &mut Option<Walkable<'backend>>,
    backend: &'backend Backend,
    human_grid: &mut Option<ResMut<crate::spatial::SpatialGrid<crate::human::Human>>>,
    demon_grid: &mut Option<ResMut<crate::spatial::SpatialGrid<crate::demon::Demon>>>,
) {
    tally.record(&result);

    let MovableState::Pathfinding(end_tile) = pawn.movable.state else {
        return;
    };
    // устаревший ответ — уже запрошена другая цель
    if end_tile != result.end_tile {
        return;
    }

    let Some(path) = result.path else {
        let walkable = walkable.get_or_insert_with(|| backend.walkable());
        if rescue_from_impassable(walkable, &mut pawn) {
            if is_human && let Some(grid) = human_grid.as_mut() {
                grid.insert(pawn.entity, pawn.sim_position.0);
            }
            if is_demon && let Some(grid) = demon_grid.as_mut() {
                grid.insert(pawn.entity, pawn.sim_position.0);
            }
            return;
        }
        // не застрял — цель просто недостижима; новую выберет поведение
        pawn.movable
            .to_pathfinding_error(pawn.entity, end_tile, pawn.commands);
        return;
    };

    // путь всегда включает стартовую точку; один элемент — мы уже на месте
    if path.len() == 1 {
        // старый путь снимается ЗДЕСЬ: перепрокладка шла на ходу и его не
        // сбрасывала, а `to_idle` объявляет приход только при пустом пути —
        // иначе событие теряется молча (то же делает `step.rs` перед своим
        // `to_idle`, и по той же причине)
        pawn.movable.path.clear();
        pawn.movable.to_idle(pawn.entity, pawn.commands, true);
        return;
    }

    // перепрокладка шла на ходу, и сущность уже не в стартовой точке:
    // срезаем начало пути, пока следующий waypoint не дальше текущего —
    // иначе первый шаг был бы назад
    let mut path: std::collections::VecDeque<Vec2> = path.into_iter().skip(1).collect();
    let position = pawn.sim_position.0;
    let mut trimmed = 0;
    while trimmed < REPATH_TRIM_LIMIT
        && path.len() >= 2
        && position.distance_squared(path[1]) <= position.distance_squared(path[0])
    {
        path.pop_front();
        trimmed += 1;
    }
    pawn.movable
        .to_moving(end_tile, path, pawn.entity, pawn.commands);
}

#[cfg(test)]
mod tests;
