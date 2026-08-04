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
    fn is_missing(&self) -> bool {
        self.grid.is_none() && self.task.is_none()
    }
}

/// Строить ли иерархию сейчас: она нужна только выбранному бэкенду и только
/// двум его алгоритмам. Двенадцать секунд всех ядер за сетку, по которой в
/// этом запуске никто не пойдёт, — самая дорогая работа в загрузке.
pub fn northstar_wanted(
    polymesh: Res<crate::navigation::PolymeshDebug>,
    algorithm: Res<crate::navigation::PathfindingAlgorithm>,
    grid: Res<NorthstarGrid>,
) -> bool {
    grid.is_missing() && !polymesh.enabled && algorithm.needs_northstar()
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
pub fn poll_northstar_build(mut grid: ResMut<NorthstarGrid>) {
    if grid.task.is_none() {
        return;
    }
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

/// Путь через иерархию: `refined` (HPA* с трассировкой) либо Theta*
/// (any-angle, точки пути не обязаны быть соседними тайлами — движение
/// идёт к центрам точек по очереди, смежность ему не нужна).
pub fn find_path_northstar(
    grid: &OrdinalGrid,
    start: IVec2,
    end: IVec2,
    any_angle: bool,
) -> Option<Vec<IVec2>> {
    if start.min_element() < 0 || end.min_element() < 0 {
        return None;
    }
    let mut args = PathfindArgs::new(
        UVec3::new(start.x as u32, start.y as u32, 0),
        UVec3::new(end.x as u32, end.y as u32, 0),
    );
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
