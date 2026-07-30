//! Пространственные сетки для поиска ближайших демонов/людей без O(люди ×
//! демоны). Ячейка 60 м ≥ максимального радиуса (паника 60 м), поэтому поиск
//! в радиусе сводится к обходу 3 × 3 соседних ячеек.
//!
//! Ячейка хранит только `Entity` — позицию кандидата потребитель читает
//! живьём из `SimPosition` через замыкание `pos_of`. Хранить `Vec2` в ячейке
//! нельзя, не пересобирая сетку каждый тик: позиция протухала бы на величину
//! до размера ячейки, и погоня/паника промахивались бы молча.
//!
//! Сетка людей ведётся инкрементально: спавн и смерть — observers на
//! `Human`, переезд между ячейками — из `move_moving_entities` при
//! пересечении границы (редкое событие: гуляющий пересекает 60-метровую
//! ячейку раз в ~21 виртуальную секунду). Сетка демонов пересобирается
//! целиком — их ~100, пересборка дешевле бухгалтерии.

use std::marker::PhantomData;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::demon::Demon;
use crate::human::Human;
use crate::movement::SimPosition;
use crate::settings::MAP_SIZE;

pub const CELL_SIZE: f32 = 60.0;

const GRID_WIDTH: i32 = (MAP_SIZE.x / CELL_SIZE) as i32 + 1;
const GRID_HEIGHT: i32 = (MAP_SIZE.y / CELL_SIZE) as i32 + 1;

/// Ячейка сетки по мировой позиции; выходы за карту прижимаются к краю.
pub fn cell_of(pos: Vec2) -> IVec2 {
    IVec2::new(
        ((pos.x / CELL_SIZE) as i32).clamp(0, GRID_WIDTH - 1),
        ((pos.y / CELL_SIZE) as i32).clamp(0, GRID_HEIGHT - 1),
    )
}

/// Порядок симуляции в `FixedUpdate`: сетки → демоны → люди. Демоны раньше
/// людей, чтобы убийства применились до `escape` и человек не был засчитан
/// и убитым, и спасшимся в один тик.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    SpatialRebuild,
    DemonBehavior,
    HumanBehavior,
}

/// Равномерная Vec-сетка сущностей типа-маркера `T`; ячейка — по позиции,
/// но сама позиция в ячейке не хранится (см. заголовок модуля).
#[derive(Resource)]
pub struct SpatialGrid<T: Send + Sync + 'static> {
    cells: Vec<Vec<Entity>>,
    /// Обратный индекс «сущность → её ячейка»: O(1) переезд и удаление.
    index: HashMap<Entity, usize>,
    _marker: PhantomData<T>,
}

impl<T: Send + Sync + 'static> Default for SpatialGrid<T> {
    fn default() -> Self {
        Self {
            cells: (0..GRID_WIDTH * GRID_HEIGHT).map(|_| Vec::new()).collect(),
            index: HashMap::default(),
            _marker: PhantomData,
        }
    }
}

fn flat_index(cell: IVec2) -> usize {
    (cell.x * GRID_HEIGHT + cell.y) as usize
}

impl<T: Send + Sync + 'static> SpatialGrid<T> {
    /// Поставить сущность в ячейку по позиции (upsert): та же ячейка — no-op,
    /// другая — переезд.
    pub fn insert(&mut self, entity: Entity, pos: Vec2) {
        let cell = flat_index(cell_of(pos));
        if let Some(&current) = self.index.get(&entity) {
            if current == cell {
                return;
            }
            Self::remove_from_cell(&mut self.cells[current], entity);
        }
        self.cells[cell].push(entity);
        self.index.insert(entity, cell);
    }

    /// Убрать сущность из сетки; отсутствующая — no-op.
    pub fn remove(&mut self, entity: Entity) {
        let Some(cell) = self.index.remove(&entity) else {
            return;
        };
        Self::remove_from_cell(&mut self.cells[cell], entity);
    }

    /// Скан ячейки допустим: ячейки маленькие, а переезды и смерти редкие.
    fn remove_from_cell(cell: &mut Vec<Entity>, entity: Entity) {
        if let Some(slot) = cell.iter().position(|&entry| entry == entity) {
            cell.swap_remove(slot);
        }
    }

    pub fn rebuild(&mut self, entries: impl Iterator<Item = (Entity, Vec2)>) {
        for cell in &mut self.cells {
            cell.clear();
        }
        self.index.clear();
        for (entity, pos) in entries {
            let cell = flat_index(cell_of(pos));
            self.cells[cell].push(entity);
            self.index.insert(entity, cell);
        }
    }

    /// Обойти всех кандидатов в ячейках, накрывающих круг `radius` вокруг
    /// `pos`. Грубый охват: вызывающий сам меряет точную дистанцию.
    pub fn for_each_in_cells_around(&self, pos: Vec2, radius: f32, mut f: impl FnMut(Entity)) {
        let center = cell_of(pos);
        let cell_span = (radius / CELL_SIZE).ceil() as i32;
        for x in (center.x - cell_span).max(0)..=(center.x + cell_span).min(GRID_WIDTH - 1) {
            for y in (center.y - cell_span).max(0)..=(center.y + cell_span).min(GRID_HEIGHT - 1) {
                for &entity in &self.cells[flat_index(IVec2::new(x, y))] {
                    f(entity);
                }
            }
        }
    }

    /// Ближайшая сущность не дальше `radius` от `pos`; позиция кандидата —
    /// из `pos_of` (живой `SimPosition`), `None` пропускает кандидата.
    pub fn nearest_in_range(
        &self,
        pos: Vec2,
        radius: f32,
        pos_of: impl Fn(Entity) -> Option<Vec2>,
    ) -> Option<(Entity, Vec2)> {
        self.nearest_in_range_where(pos, radius, pos_of, |_| true)
    }

    /// Ближайшая сущность не дальше `radius`, проходящая фильтр
    /// (например, «её ещё никто не преследует»).
    pub fn nearest_in_range_where(
        &self,
        pos: Vec2,
        radius: f32,
        pos_of: impl Fn(Entity) -> Option<Vec2>,
        mut filter: impl FnMut(Entity) -> bool,
    ) -> Option<(Entity, Vec2)> {
        let mut best: Option<(Entity, Vec2)> = None;
        let mut best_distance_squared = radius * radius;

        self.for_each_in_cells_around(pos, radius, |entity| {
            let Some(entry_pos) = pos_of(entity) else {
                return;
            };
            let distance_squared = pos.distance_squared(entry_pos);
            if distance_squared <= best_distance_squared && filter(entity) {
                best_distance_squared = distance_squared;
                best = Some((entity, entry_pos));
            }
        });
        best
    }
}

pub struct SpatialPlugin;

impl Plugin for SpatialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpatialGrid<Demon>>()
            .init_resource::<SpatialGrid<Human>>()
            .add_observer(on_human_added)
            .add_observer(on_human_removed)
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::SpatialRebuild,
                    SimSet::DemonBehavior,
                    SimSet::HumanBehavior,
                )
                    .chain()
                    .run_if(in_state(crate::loading::AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                rebuild_demon_grid.in_set(SimSet::SpatialRebuild),
            );
    }
}

/// Демонов ~100 — их сетку дешевле пересобрать за тик, чем вести
/// инкрементально (lunge двигает `SimPosition` мимо системы движения).
fn rebuild_demon_grid(
    mut diagnostics: bevy::diagnostic::Diagnostics,
    mut grid: ResMut<SpatialGrid<Demon>>,
    query: Query<(Entity, &SimPosition), With<Demon>>,
) {
    let started = std::time::Instant::now();
    grid.rebuild(query.iter().map(|(entity, pos)| (entity, pos.0)));
    crate::diagnostics::measure_ms(
        &mut diagnostics,
        &crate::diagnostics::SIM_SPATIAL_MS,
        started,
    );
}

/// Человек появился в мире — в сетку. Позиция — из `Transform` бандла спавна:
/// `SimPosition` в момент срабатывания observer'а ещё дефолтный, его заполняет
/// observer добавления `Movable` из того же `Transform`.
fn on_human_added(
    event: On<Add, Human>,
    mut grid: ResMut<SpatialGrid<Human>>,
    query: Query<&Transform>,
) {
    if let Ok(transform) = query.get(event.entity) {
        grid.insert(event.entity, transform.translation.truncate());
    }
}

/// `On<Remove>` срабатывает и при despawn, так что одна пара observers
/// покрывает весь жизненный цикл: смерть (снятие `Human` при убийстве),
/// escape, рестарт по R, смену города.
fn on_human_removed(event: On<Remove, Human>, mut grid: ResMut<SpatialGrid<Human>>) {
    grid.remove(event.entity);
}
