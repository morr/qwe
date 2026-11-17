//! Бюджетная очередь — проверяется значениями, без `App`: арифметика единиц,
//! границы бюджета, правило «не влезла — обрывает, а не пропускается» и
//! порядок FIFO-ключа раньше держались только целым приложением.

use super::*;

/// Заявка с заданным ключом и ценой; тайлы не важны — очередь их не читает.
fn request(requested_at: u64, pawn: (u8, u32), units: u32) -> QueuedRequest {
    QueuedRequest {
        requested_at,
        pawn,
        units,
        entity: Entity::from_raw_u32(pawn.1).unwrap(),
        start_world: Vec2::ZERO,
        start_tile: IVec2::ZERO,
        end_tile: IVec2::ZERO,
    }
}

// --- цена заявки ---

#[test]
fn a_request_costs_one_unit_at_zero_distance() {
    assert_eq!(request_units(IVec2::ZERO, IVec2::ZERO), 1);
}

/// Рост цены — ступенями по [`crate::settings::PATHFINDING_UNIT_TILES`]:
/// грань делителя ложится ровно на расстояние в один шаг.
#[test]
fn units_grow_in_steps_of_the_unit_distance() {
    let step = crate::settings::PATHFINDING_UNIT_TILES;
    assert_eq!(request_units(IVec2::ZERO, IVec2::new(step - 1, 0)), 1);
    assert_eq!(request_units(IVec2::ZERO, IVec2::new(step, 0)), 2);
    assert_eq!(request_units(IVec2::ZERO, IVec2::new(0, 2 * step)), 3);
    // метрика Чебышёва: расстояние — дальняя из двух осей, а не их сумма
    assert_eq!(request_units(IVec2::ZERO, IVec2::new(step, 2 * step)), 3);
}

// --- добавление под бюджетом ---

/// Подрезка срабатывает только после удвоения бюджета: до того буфер растёт
/// свободно (полсотни заявок при бюджете в единицы — штатный ход).
#[test]
fn push_trims_only_past_double_the_budget() {
    let budget = 10;
    let mut queue = Vec::new();
    for tick in 0..15u64 {
        push_within_budget(&mut queue, request(tick, (0, tick as u32), 1), budget);
    }
    assert_eq!(queue.len(), 15, "полтора бюджета не подрезаются");
}

/// Переросший вдвое буфер подрезается до бюджета и хранит самые дешёвые по
/// ключу заявки — те, что подали раньше всех.
#[test]
fn push_keeps_the_earliest_keys_when_it_trims() {
    let budget = 10;
    let mut queue = Vec::new();
    // 21 заявка — ровно на одну больше двойного бюджета: одна подрезка,
    // и в буфере остаются десять самых ранних
    for tick in 0..=20u64 {
        push_within_budget(&mut queue, request(tick, (0, tick as u32), 1), budget);
    }
    assert_eq!(queue.len(), budget as usize);
    assert!(queue.iter().all(|r| r.requested_at < budget as u64));
}

/// Каждая заявка стоит хотя бы одну единицу, поэтому подрезка до `budget`
/// не теряет никого, кто мог бы уехать в этот тик (инвариант из док-комментария).
#[test]
fn the_trim_never_drops_anything_that_could_leave_this_tick() {
    let budget = 8;
    let mut queue = Vec::new();
    for tick in 0..20u64 {
        push_within_budget(&mut queue, request(tick, (0, tick as u32), 1), budget);
    }
    take_within_budget(&mut queue, budget);
    assert_eq!(queue.len(), budget as usize, "все дешёвые уехали");
}

// --- отбор под бюджетом ---

#[test]
fn take_stops_when_the_budget_runs_out() {
    let mut queue = vec![
        request(0, (0, 0), 2),
        request(1, (0, 1), 2),
        request(2, (0, 2), 2),
    ];
    take_within_budget(&mut queue, 5);
    assert_eq!(queue.len(), 2, "третья заявка не влезла: 2+2+2 > 5");
    assert_eq!(queue[0].requested_at, 0);
    assert_eq!(queue[1].requested_at, 1);
}

/// Бюджет тратится до границы включительно: сумма ровно в бюджет уезжает целиком.
#[test]
fn an_exact_fit_is_taken_whole() {
    let mut queue = vec![request(0, (0, 0), 3), request(1, (0, 1), 2)];
    take_within_budget(&mut queue, 5);
    assert_eq!(queue.len(), 2);
}

/// Заявка, которая не влезла, обрывает набор, а не пропускается: дорогое
/// поручение в голове очереди блокирует всё, что пришло следом, — иначе оно
/// вечно уступало бы дешёвым прогулкам и не уехало бы никогда.
#[test]
fn a_request_that_does_not_fit_takes_nothing_behind_it() {
    let mut queue = vec![
        request(0, (0, 0), 10),
        request(1, (0, 1), 1),
        request(2, (0, 2), 1),
    ];
    take_within_budget(&mut queue, 5);
    assert!(queue.is_empty(), "ничего не уехало: дорогое не пропускает дешёвых");
}

// --- FIFO-ключ ---

/// Отбор идёт по ключу (тик, вид, номер), а не по порядку подачи: очередь
/// собирается из обхода запроса, чей порядок от спавнов и смертей не
/// детерминирован, и сортировка обязана его пересобрать.
#[test]
fn take_orders_by_the_fifo_key_not_by_the_filing_order() {
    let mut queue = vec![
        request(1, (1, 0), 1), // второй тик, вид 1
        request(1, (0, 9), 1), // второй тик, вид 0 — вид идёт раньше номера
        request(2, (0, 0), 1), // третий тик — тик идёт раньше вида
        request(0, (1, 5), 1), // первый тик — уезжает первым
    ];
    take_within_budget(&mut queue, 10);
    let order: Vec<(u64, u8, u32)> = queue.iter().map(|r| r.key()).collect();
    assert_eq!(
        order,
        vec![(0, 1, 5), (1, 0, 9), (1, 1, 0), (2, 0, 0)],
        "сначала тик, потом вид, потом номер"
    );
}

/// Столкновение ключей делает частичную выборку недетерминированной —
/// отбор обязан об этом кричать.
#[test]
#[should_panic(expected = "ключ очереди обязан быть уникален")]
fn duplicate_keys_are_a_bug() {
    let mut queue = vec![request(0, (0, 7), 1), request(0, (0, 7), 1)];
    take_within_budget(&mut queue, 10);
}
