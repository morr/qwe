//! Бюджетная очередь заявок детерминированного диспетчера: предсказанная цена
//! заявки, добавление, не дающее буферу разрастаться, и отбор того, что
//! влезает в темп выдачи. Отдельный модуль, потому что арифметика чистая —
//! работает над значениями, без ECS, — и проверяется юнит-тестами без целого
//! приложения; диспетчер вокруг неё — сборка очереди из запросов и выдача
//! тасков ([`super::dispatch_pathfinding_requests_deterministic`]).

use bevy::prelude::*;

/// Заявка в очереди детерминированного диспетчера.
///
/// Ключ отбора — тик заявки, а ничью на нём разрывает
/// [`pawn_key`](crate::movement::order::pawn_key); ключ целиком **уникален**.
///
/// На уникальности держится детерминизм частичной выборки (см.
/// [`take_within_budget`]), поэтому она проверяется `debug_assert`.
///
/// Тип `pub`, потому что он в сигнатуре
/// [`super::dispatch_pathfinding_requests_deterministic`], ре-экспортируемой
/// наверх; поля — только строителю очереди.
pub struct QueuedRequest {
    pub(super) requested_at: u64,
    pub(super) pawn: (u8, u32),
    pub(super) units: u32,
    pub(super) entity: Entity,
    pub(super) start_world: Vec2,
    pub(super) start_tile: IVec2,
    pub(super) end_tile: IVec2,
}

impl QueuedRequest {
    fn key(&self) -> (u64, u8, u32) {
        (self.requested_at, self.pawn.0, self.pawn.1)
    }
}

/// Предсказанная цена поиска в единицах бюджета — целая, по расстоянию между
/// тайлами старта и цели (метрика Чебышёва).
///
/// Целочисленная намеренно: сумма float зависела бы от порядка слагаемых, а
/// порядок обхода запроса меняется от спавнов и смертей. См.
/// [`crate::settings::PATHFINDING_UNIT_TILES`] — там измеренные цены, из
/// которых выведен делитель.
pub(super) fn request_units(start_tile: IVec2, end_tile: IVec2) -> u32 {
    let offset = (end_tile - start_tile).abs();
    let tiles = offset.x.max(offset.y).max(0);
    1 + (tiles / crate::settings::PATHFINDING_UNIT_TILES) as u32
}

/// Добавить заявку в буфер кандидатов, не давая ему разрастаться.
///
/// Заявок в очереди этого режима — вся неохваченная популяция, до двадцати
/// тысяч, и держать их все в буфере незачем: каждая стоит хотя бы одну
/// единицу, значит уехать может не больше `budget` штук. Буфер поэтому
/// подрезается O(n)-выборкой, как только перерастает вдвое, и в
/// установившемся ходе занимает пару сотен записей вместо двадцати тысяч.
///
/// Это не микрооптимизация: очередь длиной в популяцию — **штатное**
/// состояние режима (см. док-комментарий диспетчера), поэтому её обход идёт
/// каждый тик, а полная копия означала бы ~1 МБ записи в память на тик, то
/// есть 70 МБ/с на скорости 1× и вдвое больше на любой мелкой правке темпа.
pub(super) fn push_within_budget(
    queue: &mut Vec<QueuedRequest>,
    request: QueuedRequest,
    budget: u32,
) {
    queue.push(request);
    let cap = budget as usize;
    if queue.len() > cap * 2 {
        queue.select_nth_unstable_by_key(cap, QueuedRequest::key);
        queue.truncate(cap);
    }
}

/// Отобрать из буфера то, что влезает в бюджет: первые по ключу, пока хватает
/// единиц.
///
/// Порядок работы, а не просто сортировка с обрезкой: единиц у заявок разное
/// число, поэтому сколько их пройдёт — заранее неизвестно. Но каждая стоит
/// хотя бы одну, значит пройдёт не больше `budget` штук.
///
/// Заявка, которая не влезла, обрывает набор, а не пропускается: иначе
/// дорогое поручение вечно уступало бы дешёвым прогулкам, приходящим следом,
/// и не уехало бы никогда. Оборвавшись, она остаётся первой в очереди
/// следующего тика.
pub(super) fn take_within_budget(queue: &mut Vec<QueuedRequest>, budget: u32) {
    let cap = budget as usize;
    if queue.len() > cap {
        queue.select_nth_unstable_by_key(cap, QueuedRequest::key);
        queue.truncate(cap);
    }
    queue.sort_unstable_by_key(QueuedRequest::key);
    // сущности в сообщении обязательны: ключ называет вид и номер, но не
    // говорит, КТО их занял, — а чинить столкновение двух демонов и
    // столкновение демона с не-демоном нужно в разных местах
    debug_assert!(
        queue.windows(2).all(|pair| pair[0].key() < pair[1].key()),
        "ключ очереди обязан быть уникален, иначе частичная выборка недетерминирована; \
         столкнулись {:?}",
        queue
            .windows(2)
            .find(|pair| pair[0].key() >= pair[1].key())
            .map(|pair| (
                (pair[0].entity, pair[0].key()),
                (pair[1].entity, pair[1].key())
            ))
    );

    let mut spent = 0;
    let taken = queue
        .iter()
        .take_while(|request| {
            spent += request.units;
            spent <= budget
        })
        .count();
    queue.truncate(taken);
}

#[cfg(test)]
mod tests;
