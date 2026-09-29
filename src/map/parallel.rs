//! Разбор независимых заданий карты **по потокам** — один на все проходы
//! загрузки и сборки слоёв, которым он нужен: карманы земли и газоны обочин
//! (`osm/parse/pockets.rs`, `osm/parse/verges.rs`), замощение стоянок
//! (`osm/parse/lots.rs`), кузова машин (`cars::mesh_bodies`).
//!
//! Потоки — свои (`std::thread::scope`), а не `ComputeTaskPool`: игра отдаёт
//! тому пулу треть ядер (половина уходит A*-таскам, `main.rs`), а загрузка
//! и переход порога зума держат всё целиком, и ждать им некого — пусть
//! работают все ядра.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Сколько потоков брать: по числу ядер, не меньше одного.
pub(crate) fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// `work` над каждым из `items` по потокам — результаты в порядке `items`,
/// так что от числа потоков и их гонки ничего не зависит. Задания разбираются
/// по одному со счётчика, а не кусками поровну: плитка центра дороже плитки
/// окраины в десятки раз, и поток с кучей центральных плиток держал бы всех.
/// Одно задание (или одно ядро) считается на вызывающем потоке: поток ради
/// него стоил бы дороже, чем оно само у маленького города или витрины.
pub(crate) fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = workers().min(items.len());
    if workers <= 1 {
        return items.iter().map(work).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let (next, work) = (&next, &work);
                scope.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            break done;
                        };
                        done.push((index, work(item)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("поток разбора карты упал"))
            .collect()
    });
    done.sort_by_key(|&(index, _)| index);
    done.into_iter().map(|(_, result)| result).collect()
}
