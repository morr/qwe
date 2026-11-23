//! Иерархический поиск пути из `bevy_northstar` (HPA* / Theta*). Плагин
//! крейта не используется — его `Grid` строится из нашего navmesh один раз
//! после заливки и дёргается напрямую из async-тасков (`Grid: Send + Sync`).
//!
//! # Сколько строится
//!
//! Десятки секунд всех ядер. Точное число зависит от машины и от того, что
//! ещё на ней работает, поэтому в комментариях к коду — только этот порядок,
//! а числа и их история — здесь. Тула, профиль dev:
//!
//! | когда | карта | где | постройка |
//! |---|---|---|---|
//! | до раздвига карты | 5600 × 3700 м | — | ~11 с при тайле 2 м, ~14 с при 1 м |
//! | 2026-11, звенья 03–14 | 7600 × 5700 м | — | ~22–28 с (NOTES, план углубления) |
//! | 2026-11-23, звено 73 | 7600 × 5700 м | `pathfinding_bench` | 51,7 и 54,4 с |
//! | 2026-11-23, звено 73 | 7600 × 5700 м | игра | 59,3 с |
//!
//! Последние два замера в 2–2,5 раза выше предыдущего на той же карте.
//! Загрузкой машины это объясняется лишь отчасти: рядом шёл второй экземпляр
//! игры (0,6–1,2 ядра), а по контролю — плоскому A* на тех же задачах —
//! такой фон стоил около 10 %. Причина роста не найдена; кандидат — число
//! входов чанков и кешей внутренних путей, которое зависит от рисунка
//! проходимости, а заливка сетки за это время менялась. Замер на пустой
//! машине, и если он подтвердит — поиск причины, в плане углубления.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_northstar::grid::GridSettings;
use bevy_northstar::prelude::{GridSettingsBuilder, Nav, OrdinalGrid, PathfindArgs};

use crate::navigation::{ArcNavmesh, Navmesh};

/// Размер чанка иерархии в метрах мира; в тайлах — `50 / tile_size`, то есть
/// 25 при тайле 2 м и 50 при 1 м. Масштабировать чанк вместе с тайлом
/// обязательно: при тайле 1 м с чанком 25 постройка взрывается со ~14 с до
/// ~140 с (число чанков ×4 при том же входе на чанк; замер на карте
/// 5600 × 3700). Обе конфигурации делят сетку нацело (3800×2850/25 и
/// 7600×5700/50 — одни и те же 152×114 чанка), иначе northstar округляет
/// с warning'ом.
const CHUNK_WORLD_METERS: f32 = 50.0;

/// Иерархическая сетка northstar; `None`, пока она не построена.
///
/// Постройка занимает десятки секунд всех ядер (числа и их история — в доке
/// модуля) — в главном потоке это ровно столько замершего экрана загрузки,
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
    /// доедает все ядра ещё десятки секунд, поэтому её просят выйти по флагу,
    /// а не просто отпускают.
    pub fn clear(&mut self) {
        self.grid = None;
        self.task = None;
        if let Some(cancelled) = self.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Сетки нет и постройка не идёт — то есть её ещё надо запустить.
    /// Десятки секунд всех ядер за сетку, по которой в этом запуске никто
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
    // снапшот, а не `Arc` под read-локом: постройка идёт десятки секунд, и всё
    // это время лок держал бы поток загрузки следующего города на
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

/// Настройки сетки northstar: размеры, чанк в тайлах и набор фильтров соседей
/// — сегодня пустой. Ровно по ним [`build_checked`] собирает боевую
/// иерархию; тестовые сетки собираются по ним же, так что любое изменение
/// конфигурации (в том числе появление фильтра) видно и тестам.
fn grid_settings(width: u32, height: u32, chunk_tiles: u32) -> GridSettings {
    GridSettingsBuilder::new_2d(width, height)
        .chunk_size(chunk_tiles)
        .build()
}

/// То же с проверкой отмены между строками сетки и перед `build()` — на них
/// приходится вся длительность постройки, а внутрь крейта не заглянуть.
fn build_checked(navmesh: &Navmesh, cancelled: Option<&AtomicBool>) -> Option<OrdinalGrid> {
    let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));

    // размеры — только из снапшота: отменённая постройка, пережившая смену
    // размера навтайла, ходит по своей сетке, а не по уже переключённому
    // атомику
    let grid_size = navmesh.grid_size;
    let settings = grid_settings(
        grid_size.x as u32,
        grid_size.y as u32,
        (CHUNK_WORLD_METERS / navmesh.tile_size) as u32,
    );
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
/// Размер спрашивается у самой сетки, а не у `grid::grid_size()`: сетка
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
///
/// Ответ начинается со стартового тайла — тот же контракт, что у плоского
/// [`find_path`](super::find_path), и на нём стоит приёмка ответа
/// (`movement::pathfinding::accept::accept_answer`): один элемент — «уже на
/// месте», несколько — срезают первый. Крейт же у HPA* старт снимает сам
/// (`path.path.pop_front()` в каждой ветке `bevy_northstar`'овского
/// `pathfind`), у Theta* — оставляет. Старт возвращается здесь, потребителю
/// не нужно знать, каким из двух режимов шёл поиск.
///
/// Что было без вставки: путь `refined` — тайл за тайлом (линии Брезенхэма
/// между узлами иерархии), и ответ начинался со второго тайла маршрута — на
/// каждом ответе (`pathfinding_bench`, 1000 из 1000). Приёмка срезала ещё
/// один, и пешка шла напрямик ко второму тайлу — срезая первое колено
/// маршрута; ответ на соседний тайл, `[старт, цель]`, оставался одной точкой
/// и читался как приход без движения. Старт, совпадающий с целью, крейт у
/// HPA* отвечает пустым списком — здесь это `[start]`.
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
    let mut tiles: Vec<IVec2> = path
        .path()
        .iter()
        .map(|point| IVec2::new(point.x as i32, point.y as i32))
        .collect();
    if tiles.first() != Some(&start) {
        tiles.insert(0, start);
    }
    Some(tiles)
}

/// Пустая всюду проходимая сетка 8×8: минимальный размер, который принимает
/// `GridSettingsBuilder`, и мгновенная постройка. Собрана по [`grid_settings`]
/// с чанком в 4 тайла, так что путь из (1, 1) до (6, 1) переходит границу
/// чанка, а до (2, 2) — нет: у HPA* это две разные ветки крейта. Нужна и
/// тестам `backend`, где играет роль готовой иерархии.
#[cfg(test)]
pub(super) fn tiny_grid() -> OrdinalGrid {
    let settings = grid_settings(8, 8, 4);
    let mut grid = OrdinalGrid::new(&settings);
    grid.build();
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Контракт ответа един с плоским поиском: путь начинается со стартового
    /// тайла. Крейт у HPA* старт снимает (`pathfind.rs` крейта, `pop_front` в
    /// каждой ветке), у Theta* — оставляет; до вставки в `find_path_northstar`
    /// `refined` отвечал без старта и внутри одного чанка, и через границу.
    #[test]
    fn an_answer_starts_from_the_tile_the_search_starts_from() {
        let grid = tiny_grid();
        let start = IVec2::new(1, 1);
        for end in [IVec2::new(2, 2), IVec2::new(6, 1), IVec2::new(6, 6)] {
            for any_angle in [false, true] {
                let path = find_path_northstar(&grid, start, end, any_angle)
                    .unwrap_or_else(|| panic!("путь по пустой сетке до {end:?}"));
                assert_eq!(path.first(), Some(&start), "any_angle={any_angle}");
                assert_eq!(path.last(), Some(&end), "any_angle={any_angle}");
            }
        }
    }

    /// Цель там же, где старт, — ответ `[start]`, как у плоского поиска:
    /// приёмка читает один элемент как «уже на месте» (пустой список крейт
    /// у HPA* тоже выдаёт — только для этого случая).
    #[test]
    fn a_search_to_the_tile_it_starts_on_answers_with_that_tile_alone() {
        let grid = tiny_grid();
        for any_angle in [false, true] {
            assert_eq!(
                find_path_northstar(&grid, IVec2::new(1, 1), IVec2::new(1, 1), any_angle),
                Some(vec![IVec2::new(1, 1)]),
                "any_angle={any_angle}"
            );
        }
    }

    /// Зафиксированное расхождение иерархии с плоской сеткой — не норма.
    /// Заслон из тайлов, соприкасающихся углами, для своего A* — стена
    /// (`Navmesh::successors` не режет углов), а иерархия, собранная по
    /// [`grid_settings`] без фильтра соседей, проходит его насквозь.
    ///
    /// У крейта есть ровно наш правилный фильтр — `filter::NoCornerClipping`
    /// (диагональ запрещена, когда заняты обе ортогонали; с ним этот же поиск
    /// отвечает `None`, что здесь и проверено). Но фильтр действует и на
    /// постройку, и на каждый поиск, цена не замерена, решение — за
    /// владельцем. Пока оно не принято, тест держит текущее поведение:
    /// добавите фильтр в [`grid_settings`] — переверните и его.
    ///
    /// Узкие русла от этого класса закрыты растрово: `set_polyline` рисует
    /// их четырёхсвязной цепочкой — тест
    /// `a_narrow_diagonal_waterway_leaves_no_corner_squeeze` в `tests/navigation.rs`.
    /// Остаются касания углов разных препятствий.
    #[test]
    fn the_hierarchy_crosses_a_corner_touching_chain_the_flat_grid_treats_as_a_wall() {
        let build = |settings: GridSettings| {
            let mut grid = OrdinalGrid::new(&settings);
            // диагональ x == y — заблокированные тайлы, касающиеся углами
            for x in 0..9u32 {
                for y in 0..9u32 {
                    let nav = if x == y {
                        Nav::Impassable
                    } else {
                        Nav::Passable(1)
                    };
                    grid.set_nav(UVec3::new(x, y, 0), nav);
                }
            }
            grid.build();
            grid
        };

        let (start, goal) = (IVec2::new(1, 0), IVec2::new(0, 1));
        // плоская сетка: шаг к цели диагональный, обе ортогонали заняты
        let mut flat = Navmesh::default();
        flat.set_passable(0, 0, false);
        flat.set_passable(1, 1, false);
        assert!(
            !flat
                .successors(start.x, start.y)
                .iter()
                .any(|&(tile, _)| tile == goal)
        );

        // иерархия без фильтра: оба режима переходят заслон одним шагом
        let unfiltered = build(grid_settings(9, 9, 3));
        for any_angle in [false, true] {
            assert_eq!(
                find_path_northstar(&unfiltered, start, goal, any_angle),
                Some(vec![start, goal]),
                "any_angle={any_angle}"
            );
        }

        // с фильтром крейта — тот же запрет, что у плоской сетки
        let filtered = build(
            GridSettingsBuilder::new_2d(9, 9)
                .chunk_size(3)
                .add_neighbor_filter(bevy_northstar::filter::NoCornerClipping)
                .build(),
        );
        for any_angle in [false, true] {
            assert_eq!(
                find_path_northstar(&filtered, start, goal, any_angle),
                None,
                "any_angle={any_angle}"
            );
        }
    }
}
