//! Иерархический поиск пути из `bevy_northstar` (HPA* / Theta*). Плагин
//! крейта не используется — его `Grid` строится из нашего navmesh один раз
//! после заливки и дёргается напрямую из async-тасков (`Grid: Send + Sync`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_northstar::prelude::{GridSettingsBuilder, Nav, OrdinalGrid, PathfindArgs};

use crate::navigation::{ArcNavmesh, Navmesh};

/// Размер чанка иерархии в метрах мира; в тайлах — `50 / tile_size`, то есть
/// 25 при тайле 2 м и 50 при 1 м. Масштабировать чанк вместе с тайлом
/// обязательно: при тайле 1 м с чанком 25 постройка взрывается со ~14 с до
/// ~140 с (число чанков ×4 при том же входе на чанк). Обе конфигурации делят
/// сетку нацело (2800×1850/25 и 5600×3700/50 — одни и те же 112×74 чанка),
/// иначе northstar округляет с warning'ом.
const CHUNK_WORLD_METERS: f32 = 50.0;

/// Иерархическая сетка northstar; `None`, пока она не построена.
///
/// Постройка на карте 5600 × 3700 занимает ~11 с при тайле 2 м (~14 с при
/// 1 м), и в главном потоке это ровно столько замершего экрана загрузки —
/// поэтому она уходит в `AsyncComputeTaskPool`, а пути до её готовности
/// ищет A*.
#[derive(Resource, Default)]
pub struct NorthstarGrid {
    grid: Option<Arc<OrdinalGrid>>,
    task: Option<Task<Option<OrdinalGrid>>>,
    /// Флаг отмены текущей постройки. Роняя `Task`, отменить уже запущенную
    /// постройку нельзя: её тело синхронно и await-точек, на которых пул мог
    /// бы её выбросить, в нём нет.
    cancelled: Option<Arc<AtomicBool>>,
}

impl NorthstarGrid {
    /// Готовая сетка либо `None`, если постройка ещё идёт.
    pub fn get(&self) -> Option<Arc<OrdinalGrid>> {
        self.grid.clone()
    }

    /// Сброс перед сменой карты: сетка старого города описывает уже не ту
    /// проходимость, а таск постройки — тем более. Постройка старого города
    /// доедает все ядра ещё десяток секунд, поэтому её просят выйти по флагу,
    /// а не просто отпускают.
    pub fn clear(&mut self) {
        self.grid = None;
        self.task = None;
        if let Some(cancelled) = self.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Сетки нет и постройка не идёт — то есть её ещё надо запустить.
    /// Двенадцать секунд всех ядер за сетку, по которой в этом запуске никто
    /// не пойдёт, — самая дорогая работа в загрузке, поэтому вопрос «нужна ли
    /// она вообще» задаёт [`NavMode`](super::NavMode), а здесь остаётся только
    /// «а не строится ли уже».
    pub(super) fn is_missing(&self) -> bool {
        self.grid.is_none() && self.task.is_none()
    }

    /// Постройка летит прямо сейчас. Условие расписания для
    /// [`poll_northstar_build`]: опрашивать таск, которого нет, незачем
    /// (образец — `PolyNavmesh::is_building`).
    pub(super) fn is_building(&self) -> bool {
        self.task.is_some()
    }
}

/// Постройка стартует по входу в `Playing` — navmesh к этому моменту
/// заполнен и прорежен фоновым потоком загрузки.
pub fn start_northstar_build(arc_navmesh: Res<ArcNavmesh>, mut grid: ResMut<NorthstarGrid>) {
    // снапшот, а не `Arc` под read-локом: постройка идёт ~10 с, и всё это
    // время лок держал бы поток загрузки следующего города на
    // `navmesh.write()`. Копия сетки — один memcpy на несколько мегабайт.
    let started = std::time::Instant::now();
    let snapshot = arc_navmesh.read().clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    grid.cancelled = Some(cancelled.clone());
    grid.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        let built = build_checked(&snapshot, Some(&cancelled));
        match &built {
            Some(_) => info!("northstar grid built in {:?}", started.elapsed()),
            None => info!("northstar build cancelled after {:?}", started.elapsed()),
        }
        built
    }));
}

/// Снятие готовой сетки с таска; до этого HPA*/Theta* работают как A*.
/// Гейт «постройка летит» — в расписании (`NorthstarGrid::is_building`),
/// а не ранним выходом здесь.
pub fn poll_northstar_build(mut grid: ResMut<NorthstarGrid>) {
    let Some(built) = grid.task.as_mut().and_then(check_ready) else {
        return;
    };
    grid.task = None;
    grid.cancelled = None;
    // `None` — постройку отменили; сетки у нас нет, и это не ошибка
    if let Some(built) = built {
        grid.grid = Some(Arc::new(built));
    }
}

/// Постройка сетки northstar из заполненного navmesh (входы чанков,
/// кеши внутренних путей — считается параллельно внутри крейта).
pub fn build_from_navmesh(navmesh: &Navmesh) -> OrdinalGrid {
    build_checked(navmesh, None).expect("build without a cancel flag cannot be cancelled")
}

/// То же с проверкой отмены между строками сетки и перед `build()` — на них
/// приходится вся длительность постройки, а внутрь крейта не заглянуть.
fn build_checked(navmesh: &Navmesh, cancelled: Option<&AtomicBool>) -> Option<OrdinalGrid> {
    let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));

    // размеры — только из снапшота: отменённая постройка, пережившая смену
    // размера навтайла, ходит по своей сетке, а не по уже переключённому
    // атомику
    let grid_size = navmesh.grid_size;
    let settings = GridSettingsBuilder::new_2d(grid_size.x as u32, grid_size.y as u32)
        .chunk_size((CHUNK_WORLD_METERS / navmesh.tile_size) as u32)
        .build();
    let mut grid = OrdinalGrid::new(&settings);
    for x in 0..grid_size.x {
        if is_cancelled() {
            return None;
        }
        for y in 0..grid_size.y {
            let nav = if navmesh.is_passable(x, y) {
                Nav::Passable(1)
            } else {
                Nav::Impassable
            };
            grid.set_nav(UVec3::new(x as u32, y as u32, 0), nav);
        }
    }
    if is_cancelled() {
        return None;
    }
    grid.build();
    Some(grid)
}

/// Тайл как точка сетки northstar; `None` — тайл вне сетки.
///
/// Проверяются обе границы, а не только нижняя: `OrdinalGrid::pathfind` на
/// выходе за верхнюю не паникует, но пишет `log::error!` на каждый вызов, и
/// пешка, ушедшая за край карты, спамит лог с частотой диспетчера. Тихий
/// `None` — то же, чем на такой запрос отвечает сеточный A*.
///
/// Размер спрашивается у самой сетки, а не у `settings::grid_size()`: сетка
/// строится из снапшота navmesh и переживает смену размера навтайла (см.
/// [`build_checked`]), поэтому её собственные размеры — единственные, по
/// которым её индексируют.
fn grid_point(grid: &OrdinalGrid, tile: IVec2) -> Option<UVec3> {
    if tile.min_element() < 0 {
        return None;
    }
    let point = UVec3::new(tile.x as u32, tile.y as u32, 0);
    grid.in_bounds(point).then_some(point)
}

/// Путь через иерархию: `refined` (HPA* с трассировкой) либо Theta*
/// (any-angle, точки пути не обязаны быть соседними тайлами — движение
/// идёт к центрам точек по очереди, смежность ему не нужна).
pub fn find_path_northstar(
    grid: &OrdinalGrid,
    start: IVec2,
    end: IVec2,
    any_angle: bool,
) -> Option<Vec<IVec2>> {
    let mut args = PathfindArgs::new(grid_point(grid, start)?, grid_point(grid, end)?);
    args = if any_angle {
        args.thetastar()
    } else {
        args.refined()
    };
    let path = grid.pathfind(&mut args)?;
    Some(
        path.path()
            .iter()
            .map(|point| IVec2::new(point.x as i32, point.y as i32))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Пустая всюду проходимая сетка 8×8: минимальный размер, который
    /// принимает `GridSettingsBuilder`, и мгновенная постройка.
    fn tiny_grid() -> OrdinalGrid {
        let settings = GridSettingsBuilder::new_2d(8, 8).chunk_size(4).build();
        let mut grid = OrdinalGrid::new(&settings);
        grid.build();
        grid
    }

    #[test]
    fn a_tile_past_the_far_edge_is_not_a_grid_point() {
        let grid = tiny_grid();
        assert_eq!(grid_point(&grid, IVec2::new(8, 0)), None);
        assert_eq!(grid_point(&grid, IVec2::new(0, 8)), None);
        assert_eq!(grid_point(&grid, IVec2::new(-1, 0)), None);
    }

    /// Верхняя граница исключающая: последний тайл 8×8 — седьмой, и он внутри.
    #[test]
    fn the_far_corner_tile_is_still_inside() {
        let grid = tiny_grid();
        assert_eq!(
            grid_point(&grid, IVec2::new(7, 7)),
            Some(UVec3::new(7, 7, 0))
        );
    }

    #[test]
    fn a_search_out_of_bounds_fails_quietly() {
        let grid = tiny_grid();
        assert!(find_path_northstar(&grid, IVec2::new(0, 0), IVec2::new(8, 0), false).is_none());
        assert!(find_path_northstar(&grid, IVec2::new(8, 0), IVec2::new(0, 0), true).is_none());
    }

    /// Гвард не съедает законный запрос до дальнего угла.
    #[test]
    fn a_search_inside_the_grid_still_finds_a_path() {
        let grid = tiny_grid();
        let path = find_path_northstar(&grid, IVec2::new(0, 0), IVec2::new(7, 7), true)
            .expect("путь по пустой сетке");
        assert_eq!(path.last().copied(), Some(IVec2::new(7, 7)));
    }

    /// Предикат гейта расписания: пока таск не заведён — опрашивать нечего,
    /// после `clear()` — тем более.
    #[test]
    fn is_building_follows_the_in_flight_task() {
        use bevy::tasks::TaskPool;

        AsyncComputeTaskPool::get_or_init(TaskPool::default);
        let mut grid = NorthstarGrid::default();
        assert!(!grid.is_building());
        grid.task = Some(AsyncComputeTaskPool::get().spawn(async { None }));
        assert!(grid.is_building());
        grid.clear();
        assert!(!grid.is_building());
    }

    /// Снятый ранний выход ничего не сторожил: без летящего таска система —
    /// no-op, а не паника (стенды поднимают её и без гейта).
    #[test]
    fn polling_without_a_task_is_a_no_op() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.init_resource::<NorthstarGrid>();
        world
            .run_system_once(poll_northstar_build)
            .expect("система обязана отработать");
        assert!(world.resource::<NorthstarGrid>().is_missing());
    }
}
