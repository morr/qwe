//! Достижимость по сетке: обход по 4-связности, соседи и стоимости шага для
//! A*, прунинг недостижимого и пара «калитки, затем прунинг»
//! ([`Navmesh::open_gates_and_prune`]).

use bevy::prelude::*;

use super::Navmesh;
use crate::map::osm::model::MapData;

/// Стоимость шага между тайлами (для A*): прямой и диагональный.
const COST_STRAIGHT: i32 = 100;
const COST_DIAGONAL: i32 = 141;

/// Итог [`Navmesh::open_gates_and_prune`]. Замер прунинга — поле, а не дело
/// вызывающего: загрузчик пишет по строке на шаг, и обе строки — ready-маркеры
/// `live-app` (`navmesh: opened N fence gates`,
/// `navmesh: pruned N unreachable tiles`), формат которых менять нельзя.
/// Итог калиток — число и замер — отдаётся раньше и только колбэком стадии,
/// чтобы строка о них не ждала конца прунинга; в итоге его нет, чтобы одна
/// величина не приезжала к вызывающему двумя путями.
pub struct GatesAndPrune {
    /// Срезанных недостижимых тайлов.
    pub pruned: usize,
    pub prune_time: std::time::Duration,
}

impl Navmesh {
    /// Первый проходимый тайл, начиная с `from` по внутренней индексации
    /// (`x * grid_size.y + y`) и с заворотом через конец сетки; `None` —
    /// проходимых тайлов нет вовсе (в том числе когда сетка пуста).
    ///
    /// Нужен размещению населения (`human::spawn_population`) в двух ролях:
    /// дешёвая проверка «сетка вообще не пустая» (скан обрывается на первом
    /// же открытом тайле) и детерминированный запасной выбор, когда бюджет
    /// случайных выборок исчерпан. Заворот — чтобы запасной выбор зависел от
    /// последней выборки и не сваливал всё население в один тайл.
    pub fn passable_from(&self, from: IVec2) -> Option<IVec2> {
        let len = self.passable.len();
        if len == 0 {
            return None;
        }
        let start = self.index(from.x, from.y).unwrap_or(0);
        (0..len).find_map(|step| {
            let index = (start + step) % len;
            self.passable[index].then(|| {
                IVec2::new(
                    index as i32 / self.grid_size.y,
                    index as i32 % self.grid_size.y,
                )
            })
        })
    }

    /// Соседи тайла для A*: 8 направлений, диагональ только когда оба смежных
    /// прямых тайла проходимы (чтобы путь не резал углы зданий).
    pub fn successors(&self, x: i32, y: i32) -> Vec<(IVec2, i32)> {
        let mut result = Vec::with_capacity(8);
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if !self.is_passable(nx, ny) {
                continue;
            }
            let is_diagonal = dx != 0 && dy != 0;
            if is_diagonal && !(self.is_passable(x, ny) && self.is_passable(nx, y)) {
                continue;
            }
            result.push((
                IVec2::new(nx, ny),
                if is_diagonal {
                    COST_DIAGONAL
                } else {
                    COST_STRAIGHT
                },
            ));
        }
        result
    }

    /// Дорастить достижимость после того, как у точек `around` открылись
    /// тайлы: обход стартует от проходимых тайлов в радиусе, которые касаются
    /// уже достижимого.
    pub(super) fn extend_reachable(&self, reachable: &mut [bool], around: &[Vec2], radius: f32) {
        let mut seeds = Vec::new();
        for &point in around {
            let (min, max) = (self.to_tile(point - radius), self.to_tile(point + radius));
            for x in min.x..=max.x {
                for y in min.y..=max.y {
                    if let Some(index) = self.index(x, y)
                        && self.passable[index]
                        && !reachable[index]
                        && self.neighbours(index).any(|next| reachable[next])
                    {
                        seeds.push(index);
                    }
                }
            }
        }
        self.flood(&self.passable, reachable, seeds);
    }

    /// Обход по 4-связности — связности прунинга: от `seeds` по тайлам,
    /// проходимым в `passable`, всё достигнутое метится в `reachable`.
    /// Индексная арифметика вместо [`Self::neighbours`] — сетка на 5 млн
    /// тайлов, и на `opt-level = 1` итераторы соседей стоили обходу втрое.
    pub(super) fn flood(&self, passable: &[bool], reachable: &mut [bool], seeds: Vec<usize>) {
        let height = self.grid_size.y as usize;
        let len = passable.len();
        let mut stack = Vec::with_capacity(seeds.len().max(1024));
        for seed in seeds {
            if passable[seed] && !reachable[seed] {
                reachable[seed] = true;
                stack.push(seed);
            }
        }
        while let Some(index) = stack.pop() {
            let y = index % height;
            let neighbours = [
                index.checked_sub(height),
                (index + height < len).then_some(index + height),
                (y > 0).then(|| index - 1),
                (y + 1 < height).then_some(index + 1),
            ];
            for next in neighbours.into_iter().flatten() {
                if passable[next] && !reachable[next] {
                    reachable[next] = true;
                    stack.push(next);
                }
            }
        }
    }

    /// Четыре соседа по стороне — связность прунинга.
    pub(super) fn neighbours(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = (
            index as i32 / self.grid_size.y,
            index as i32 % self.grid_size.y,
        );
        [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .filter_map(move |(dx, dy)| self.index(x + dx, y + dy))
    }

    /// Связная компонента `mask` от `start`; обойдённое метится в `seen`.
    pub(super) fn component(
        &self,
        start: usize,
        mask: &impl Fn(usize) -> bool,
        seen: &mut [bool],
    ) -> Vec<usize> {
        seen[start] = true;
        let mut tiles = vec![start];
        let mut cursor = 0;
        while cursor < tiles.len() {
            let index = tiles[cursor];
            cursor += 1;
            for next in self.neighbours(index) {
                if !seen[next] && mask(next) {
                    seen[next] = true;
                    tiles.push(next);
                }
            }
        }
        tiles
    }

    /// Калитки по умолчанию, затем прунинг — двумя шагами, но одним вызовом.
    ///
    /// Порядок здесь — правило, а не последовательность двух независимых
    /// операций: ограда, отрезавшая участок с дверями, обязана открыться
    /// РАНЬШЕ, чем этот участок выбросят как недостижимый. Пять мест сборки
    /// навмеша писали эту пару руками, и порядок в каждом держался на
    /// внимательности.
    ///
    /// Заливка и снап портала остаются у вызывающего: заливка бывает разной
    /// (`fill_from_mapdata` против собранной руками сетки теста), а снап
    /// умеет не найти места и вернуть сырую подсказку — оба решения
    /// принадлежат сборке, а не навмешу.
    ///
    /// `stage` зовётся МЕЖДУ шагами и получает итог калиток: загрузчику нужно
    /// на этом месте переключить `JobState::Pruning` и написать свою строку,
    /// не дожидаясь конца прунинга. Кому стадия не нужна — передаёт
    /// `|_, _| {}`.
    ///
    /// Кому этот метод не подходит: аудиты (`fence_prune_audit`,
    /// `navmesh_probe`) снимают `clone()` между шагами, поэтому зовут
    /// [`Self::open_sealed_fences`] и [`Self::prune_unreachable`] по
    /// отдельности — обе остаются `pub` ровно для этого.
    pub fn open_gates_and_prune(
        &mut self,
        map: &mut MapData,
        portal: Vec2,
        stage: impl FnOnce(usize, std::time::Duration),
    ) -> GatesAndPrune {
        let started = std::time::Instant::now();
        let gates = self.open_sealed_fences(map, portal);
        stage(gates, started.elapsed());

        let started = std::time::Instant::now();
        let pruned = self.prune_unreachable(portal);
        GatesAndPrune {
            pruned,
            prune_time: started.elapsed(),
        }
    }

    /// Тайлы, недостижимые из `start`, становятся непроходимыми: замкнутые
    /// дворы и острова иначе порождают заведомо безуспешные A*-поиски,
    /// обходящие всю карту (десятки мс каждый). 4-связность совпадает с
    /// достижимостью A*: диагональ требует обоих смежных прямых тайлов.
    ///
    /// Точка мировая, а не тайл: перевод берётся из снимка сетки самого
    /// навмеша ([`Self::to_tile`]), а не из глобального атомика размера
    /// навтайла, который к моменту прунинга мог уже смениться.
    ///
    /// **Ранний выход обязателен.** Старт вне сетки или на непроходимом тайле
    /// — это случай `no clear spot for portal`: обход от такого старта не
    /// достигает ничего, и «залить и вырезать недостигнутое» вырезало бы всю
    /// карту. Здесь прунинг не режет ничего.
    pub fn prune_unreachable(&mut self, start: Vec2) -> usize {
        let start_tile = self.to_tile(start);
        let Some(start_index) = self.index(start_tile.x, start_tile.y) else {
            return 0;
        };
        if !self.passable[start_index] {
            return 0;
        }

        let mut reachable = vec![false; self.passable.len()];
        self.flood(&self.passable, &mut reachable, vec![start_index]);

        let mut pruned = 0;
        for (index, is_reachable) in reachable.iter().enumerate() {
            if self.passable[index] && !is_reachable {
                self.passable[index] = false;
                pruned += 1;
            }
        }
        pruned
    }
}

#[cfg(test)]
mod tests;
