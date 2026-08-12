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
//! `Human`, сдвиг — из шага движения и из расталкивания, оба через
//! [`SpatialGrid::moved`], который и решает, что считать переездом (редкое
//! событие: гуляющий пересекает 60-метровую ячейку раз в ~21 виртуальную
//! секунду). Сетка демонов пересобирается целиком — их ~100, пересборка
//! дешевле бухгалтерии.

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
/// Наружу не выставлена намеренно: единственное, ради чего её спрашивали со
/// стороны, — «пересечена ли граница», и это теперь [`SpatialGrid::moved`].
fn cell_of(pos: Vec2) -> IVec2 {
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

    /// Сущность сдвинулась из `from` в `to`: переезд, только если пересечена
    /// граница ячейки.
    ///
    /// Отдельный вход, а не голый [`Self::insert`], потому что сравнение ячеек
    /// — арифметика без hash, а `insert` начинается с поиска в `index`. Пешка
    /// двигается каждый тик, а границу 60-метровой ячейки пересекает раз в ~21
    /// виртуальную секунду, так что дешёвая проверка снимает почти все
    /// обращения к таблице; стоимость не растёт ни от зум-аута, ни от
    /// населения.
    ///
    /// Правило живёт здесь, а не у вызывающих: копий было две — шаг движения и
    /// расталкивание, — и разъехаться им было нечем помешать.
    pub fn moved(&mut self, entity: Entity, from: Vec2, to: Vec2) {
        if cell_of(to) != cell_of(from) {
            self.insert(entity, to);
        }
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

    /// Есть ли хоть один кандидат в ячейках, накрывающих круг `radius` вокруг
    /// `pos`. Грубый охват, как у [`Self::for_each_in_cells_around`]: `true`
    /// может означать кандидата и дальше `radius` (в пределах диагонали
    /// ячейки), но `false` гарантирует, что в радиусе никого нет.
    pub fn any_in_cells_around(&self, pos: Vec2, radius: f32) -> bool {
        let center = cell_of(pos);
        let cell_span = (radius / CELL_SIZE).ceil() as i32;
        for x in (center.x - cell_span).max(0)..=(center.x + cell_span).min(GRID_WIDTH - 1) {
            for y in (center.y - cell_span).max(0)..=(center.y + cell_span).min(GRID_HEIGHT - 1) {
                if !self.cells[flat_index(IVec2::new(x, y))].is_empty() {
                    return true;
                }
            }
        }
        false
    }

    /// Обойти всех кандидатов в ячейках, пересекающих прямоугольник
    /// `[min, max]` (мировые метры). Грубый охват, как у
    /// `for_each_in_cells_around`: точное попадание в прямоугольник меряет
    /// вызывающий. Выходы за карту прижимаются к краю самим `cell_of`.
    pub fn for_each_in_rect(&self, min: Vec2, max: Vec2, mut f: impl FnMut(Entity)) {
        let lo = cell_of(min);
        let hi = cell_of(max);
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                for &entity in &self.cells[flat_index(IVec2::new(x, y))] {
                    f(entity);
                }
            }
        }
    }

    /// Ближайшая сущность не дальше `radius` от `pos`; позиция кандидата — из
    /// `pos_of` (живой `SimPosition`), `None` пропускает кандидата, ничья
    /// разрешается `order_of` (см. [`Self::nearest_in_range_where`]).
    pub fn nearest_in_range(
        &self,
        pos: Vec2,
        radius: f32,
        pos_of: impl Fn(Entity) -> Option<Vec2>,
        order_of: impl Fn(Entity) -> u32,
    ) -> Option<(Entity, Vec2)> {
        self.nearest_in_range_where(pos, radius, pos_of, order_of, |_| true)
    }

    /// Ближайшая сущность не дальше `radius`, проходящая фильтр
    /// (например, «её ещё никто не преследует»).
    ///
    /// `order_of` — **порядковый номер** пешки
    /// (`movement::order::pawn_number`), которым разрывается ничья по
    /// расстоянию: из равноудалённых побеждает меньший номер. Вид в ключ не
    /// входит, в отличие от `pawn_key`: сетка типизирована маркером `T`, так
    /// что все кандидаты одного вида по построению.
    ///
    /// Ничья редка — на Туле **5 случаев на 3.84 млн поисков** за пять
    /// симулированных минут, — и ровно поэтому номер спрашивается **только**
    /// при точном равенстве расстояний, отдельным замыканием, а не приезжает
    /// вместе с позицией: поиск обходит десятки кандидатов и зовётся миллионы
    /// раз за прогон, и лишнее чтение колонки `PawnId` на каждого кандидата
    /// стоило бы куда дороже того, что оно решает.
    ///
    /// Редкость — не повод оставить ничью обходу. Иначе победителя выбирает
    /// порядок внутри ячейки, то есть история спавнов и смертей (`swap_remove`
    /// перекладывает хвост на место удалённого), — ровно то, от чего симуляция
    /// обязана не зависеть; правило записано в `movement/order.rs`, а это было
    /// последнее место, где его не применили.
    pub fn nearest_in_range_where(
        &self,
        pos: Vec2,
        radius: f32,
        pos_of: impl Fn(Entity) -> Option<Vec2>,
        order_of: impl Fn(Entity) -> u32,
        mut filter: impl FnMut(Entity) -> bool,
    ) -> Option<(Entity, Vec2)> {
        // граница радиуса включительна («не дальше `radius`») и это НЕ то же
        // сравнение, что «ближе найденного»: их совмещение и отдавало ничью
        // порядку обхода
        let mut best_distance_squared = radius * radius;
        let mut best: Option<(Entity, Vec2)> = None;
        // номер лидера считается лениво и сбрасывается со сменой лидера: без
        // ничьей его не спрашивают ни разу
        let mut best_order: Option<u32> = None;

        self.for_each_in_cells_around(pos, radius, |entity| {
            let Some(entry_pos) = pos_of(entity) else {
                return;
            };
            let distance_squared = pos.distance_squared(entry_pos);
            if distance_squared > best_distance_squared {
                return;
            }
            if distance_squared == best_distance_squared
                && let Some((leader, _)) = best
                && order_of(entity) > *best_order.get_or_insert_with(|| order_of(leader))
            {
                return;
            }
            if !filter(entity) {
                return;
            }
            best_distance_squared = distance_squared;
            best = Some((entity, entry_pos));
            best_order = None;
        });
        best
    }
}

/// Позиция кандидата поиска — живой `SimPosition`. Читается для каждого
/// кандидата, поэтому берёт из выборки только её.
pub fn pawn_position<F: bevy::ecs::query::QueryFilter>(
    query: &Query<(&SimPosition, Option<&crate::rng::PawnId>), F>,
    candidate: Entity,
) -> Option<Vec2> {
    query.get(candidate).ok().map(|(position, _)| position.0)
}

/// Номер кандидата — ключ, которым [`SpatialGrid::nearest_in_range_where`]
/// разрывает ничью; зовётся только на ничьей.
///
/// Заведён рядом с [`pawn_position`], чтобы обе половины контракта поиска
/// брались из одной выборки и не собирались руками в каждом вызове: ровно так
/// ключ порядка и разъезжался по копиям раньше (`movement/order.rs`).
/// `Option<&PawnId>` — пешка без номера (отладочный ходок) уезжает в хвост
/// порядка, а не выпадает из поиска молча.
pub fn pawn_order<F: bevy::ecs::query::QueryFilter>(
    query: &Query<(&SimPosition, Option<&crate::rng::PawnId>), F>,
    candidate: Entity,
) -> u32 {
    crate::movement::order::pawn_number(query.get(candidate).ok().and_then(|(_, pawn_id)| pawn_id))
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

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::*;
    use crate::rng::PawnId;

    /// Мир проводки: сами observers и пересборка демонов, без состояний и
    /// расписания игры.
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<bevy::diagnostic::DiagnosticsStore>()
            .init_resource::<SpatialGrid<Human>>()
            .init_resource::<SpatialGrid<Demon>>()
            .add_observer(on_human_added)
            .add_observer(on_human_removed)
            .add_systems(Update, rebuild_demon_grid);
        app
    }

    fn found_near<T: Send + Sync + 'static>(app: &App, pos: Vec2) -> Vec<Entity> {
        let mut seen = Vec::new();
        app.world()
            .resource::<SpatialGrid<T>>()
            .for_each_in_cells_around(pos, 0.0, |entity| seen.push(entity));
        seen
    }

    /// Позиция берётся из `Transform`, а не из `SimPosition`: в момент
    /// срабатывания observer'а тот ещё дефолтный (нулевой), и человек ушёл бы
    /// в угол карты — там его бы и искали демоны.
    #[test]
    fn a_spawned_human_enters_the_grid_where_his_transform_stands() {
        let app = &mut app();
        let at = Vec2::new(400.0, 400.0);
        let human = app
            .world_mut()
            .spawn((Human, Transform::from_translation(at.extend(0.0))))
            .id();

        assert_eq!(found_near::<Human>(app, at), vec![human]);
        assert!(found_near::<Human>(app, Vec2::ZERO).is_empty());
    }

    /// Одна пара observers покрывает весь жизненный цикл: смерть — это снятие
    /// `Human` с живой сущности, escape и смена города — despawn. `On<Remove>`
    /// обязан отработать в обоих случаях, иначе труп остаётся кандидатом в
    /// жертвы.
    #[test]
    fn a_human_leaves_the_grid_both_by_losing_the_tag_and_by_despawn() {
        let app = &mut app();
        let at = Vec2::new(400.0, 400.0);
        let spawn = |app: &mut App| {
            app.world_mut()
                .spawn((Human, Transform::from_translation(at.extend(0.0))))
                .id()
        };

        let corpse = spawn(app);
        app.world_mut().entity_mut(corpse).remove::<Human>();
        assert!(
            found_near::<Human>(app, at).is_empty(),
            "труп остался в сетке"
        );

        let escaped = spawn(app);
        app.world_mut().entity_mut(escaped).despawn();
        assert!(
            found_near::<Human>(app, at).is_empty(),
            "despawn прошёл мимо сетки"
        );
    }

    /// Сетка демонов не ведётся инкрементально вовсе: бросок двигает
    /// `SimPosition` мимо системы движения, и единственное, что держит её в
    /// согласии с миром, — полная пересборка каждый тик.
    #[test]
    fn the_demon_grid_follows_a_position_written_past_the_movement_system() {
        let app = &mut app();
        let from = Vec2::new(100.0, 100.0);
        let to = Vec2::new(400.0, 400.0);
        let demon = app.world_mut().spawn((Demon, SimPosition(from))).id();

        app.update();
        assert_eq!(found_near::<Demon>(app, from), vec![demon]);

        // ровно то, что делает бросок: запись в `SimPosition` напрямую
        app.world_mut().entity_mut(demon).insert(SimPosition(to));
        app.update();

        assert!(found_near::<Demon>(app, from).is_empty());
        assert_eq!(found_near::<Demon>(app, to), vec![demon]);
    }

    /// Проводка ключа порядка от компонента до разрешения ничьи: два
    /// равноудалённых демона, и побеждает меньший `PawnId`, а не тот, кого
    /// пересборка положила в ячейку последним.
    #[test]
    fn the_search_breaks_a_tie_by_the_pawn_id_the_entity_carries() {
        let app = &mut app();
        let middle = Vec2::new(100.0, 100.0);
        // меньший номер спавнится ПЕРВЫМ, и это не безразлично: пересборка
        // укладывает демонов в ячейку в порядке выборки, так что при разрешении
        // ничьи обходом победил бы как раз старший
        let junior = app
            .world_mut()
            .spawn((Demon, SimPosition(middle - Vec2::X * 10.0), PawnId(2)))
            .id();
        let senior = app
            .world_mut()
            .spawn((Demon, SimPosition(middle + Vec2::X * 10.0), PawnId(9)))
            .id();
        app.update();

        // поиск прогоняется настоящей системой: проверяется в том числе то,
        // что `probe_pawn` доходит до `PawnId` через обычную выборку
        fn nearest(
            grid: Res<SpatialGrid<Demon>>,
            demons: Query<(&SimPosition, Option<&PawnId>), With<Demon>>,
        ) -> Option<Entity> {
            grid.nearest_in_range(
                Vec2::new(100.0, 100.0),
                60.0,
                |candidate| pawn_position(&demons, candidate),
                |candidate| pawn_order(&demons, candidate),
            )
            .map(|(entity, _)| entity)
        }
        let found = app
            .world_mut()
            .run_system_once(nearest)
            .expect("система поиска прогоняется");

        assert_eq!(found, Some(junior), "ничью разрешил не `PawnId`");
        assert_ne!(found, Some(senior));
    }
}
