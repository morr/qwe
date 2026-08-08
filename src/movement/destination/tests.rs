use super::*;

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

fn anywhere(_: IVec2) -> bool {
    true
}

/// Индекс на решётке со стороной `side` тайлов.
fn claims_with(side: i32) -> DestinationClaims {
    let mut claims = DestinationClaims::default();
    claims.sync(side, CLAIM_SEARCH_METERS);
    claims
}

/// Сторона слота округляется ВВЕРХ до целого числа тайлов: шаг решётки
/// обязан быть не меньше дистанции покоя, иначе гарантии нет вовсе.
#[test]
fn the_slot_side_covers_the_rest_distance() {
    // навтайл по умолчанию 2 м: покой 1.8 умещается в один тайл
    assert_eq!(slot_side(1.8), 1);
    // а покой 2.4 (ползунок радиуса на максимуме) — уже нет
    assert_eq!(slot_side(2.4), 2);
    // вырожденный случай: слот не бывает меньше тайла
    assert_eq!(slot_side(0.1), 1);
}

/// Цели соседних слотов отстоят ровно на сторону слота — это и есть та
/// гарантия, ради которой слот перестал быть одним тайлом.
#[test]
fn targets_of_neighbouring_slots_are_a_full_side_apart() {
    for side in 1..=3 {
        let here = slot_target(IVec2::new(4, 4), side);
        let right = slot_target(IVec2::new(5, 4), side);
        let up = slot_target(IVec2::new(4, 5), side);
        assert_eq!(right - here, IVec2::new(side, 0), "side {side}");
        assert_eq!(up - here, IVec2::new(0, side), "side {side}");
    }
}

/// Отрицательные тайлы не склеивают соседние слоты: у обычного деления
/// −1 и 0 дают один и тот же ноль.
#[test]
fn slots_do_not_fold_around_the_origin() {
    assert_ne!(slot_of(IVec2::new(-1, 0), 2), slot_of(IVec2::new(0, 0), 2));
}

/// Свободный слот достаётся как есть — обычный случай, поиска нет.
#[test]
fn a_free_slot_is_claimed_as_is() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);

    assert_eq!(
        claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere),
        Some((desired, desired))
    );
}

/// Занятый слот уводит цель на соседний — иначе двое целятся в одну точку,
/// и вечный упор между ними неизбежен.
#[test]
fn a_taken_slot_moves_the_claim_to_a_free_neighbour() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

    let (_, target) = claims
        .claim_slot(entity(2), None, desired, tile_center(desired), anywhere)
        .expect("сосед свободен");
    assert_ne!(target, desired);
    // соседний, а не любой: кольцевой поиск отдаёт ближайший
    assert_eq!((target - desired).abs().max_element(), 1);
}

/// На решётке со стороной 2 тайла цели двух пешек расходятся на два тайла,
/// а не на один: ровно этого не умел слот-в-один-тайл при `NavtileBase::M1`.
#[test]
fn a_wide_slot_keeps_two_pawns_two_tiles_apart() {
    let mut claims = claims_with(2);
    let desired = IVec2::new(10, 10);
    let (_, first) = claims
        .claim_slot(entity(1), None, desired, tile_center(desired), anywhere)
        .expect("свободно");
    let (_, second) = claims
        .claim_slot(entity(2), None, desired, tile_center(desired), anywhere)
        .expect("сосед свободен");

    assert_eq!((first - second).abs().max_element(), 2);
}

/// Из одинаково близких к цели слотов пешке достаётся тот, что с ЕЁ
/// стороны. Без этого выбор внутри кольца произволен, и подошедшие с разных
/// краёв меняются местами: обе идут через занятую цель навстречу друг другу.
#[test]
fn a_pawn_takes_the_free_slot_on_its_own_side() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

    let west = tile_center(desired) - Vec2::new(40.0, 0.0);
    let east = tile_center(desired) + Vec2::new(40.0, 0.0);
    let (_, from_west) = claims
        .claim_slot(entity(2), None, desired, west, anywhere)
        .expect("сосед свободен");
    let (_, from_east) = claims
        .claim_slot(entity(3), None, desired, east, anywhere)
        .expect("сосед свободен");

    assert!(from_west.x < desired.x, "подошедший с запада: {from_west}");
    assert!(from_east.x > desired.x, "подошедший с востока: {from_east}");
}

/// Своя же заявка себе не помеха: пешка, выбравшая ту же цель второй раз,
/// не должна уезжать в сторону от неё.
#[test]
fn an_own_claim_is_not_an_obstacle_to_itself() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    let (slot, _) = claims
        .claim_slot(entity(1), None, desired, tile_center(desired), anywhere)
        .expect("свободно");

    assert_eq!(
        claims.claim_slot(
            entity(1),
            Some(slot),
            desired,
            tile_center(desired),
            anywhere
        ),
        Some((slot, desired))
    );
    assert_eq!(claims.len(), 1);
}

/// Смена цели освобождает прошлый слот — иначе индекс за прогон зарастает
/// заявками, которых никто не держит.
#[test]
fn re_claiming_frees_the_old_slot() {
    let mut claims = claims_with(1);
    let old = IVec2::new(10, 10);
    claims.claim_slot(entity(1), None, old, tile_center(old), anywhere);

    let far = IVec2::new(40, 40);
    claims.claim_slot(entity(1), Some(old), far, tile_center(far), anywhere);

    assert!(claims.is_free(old, entity(2)));
    assert_eq!(claims.len(), 1);
}

/// Свободного слота нет — цель остаётся общей и без заявки. Это сегодняшнее
/// поведение: лучше, чем оставить пешку без цели вовсе.
#[test]
fn nothing_free_falls_back_to_the_shared_target() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);

    assert_eq!(
        claims.claim_slot(entity(1), None, desired, tile_center(desired), |_| false),
        None
    );
    assert_eq!(claims.len(), 0);
}

/// Пачка на общую цель: у каждого свой слот, ни один не повторился.
#[test]
fn a_batch_gives_every_member_its_own_slot() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    let members: Vec<_> = ring(8, 40.0, desired);
    let mut out = Vec::new();

    claims.claim_group(desired, &members, anywhere, &mut out);

    let slots: Vec<IVec2> = out.iter().map(|slot| slot.expect("место есть").0).collect();
    assert_eq!(slots.len(), 8);
    for (index, slot) in slots.iter().enumerate() {
        assert!(
            !slots[..index].contains(slot),
            "слот {slot} выдан дважды: {slots:?}"
        );
    }
    assert_eq!(claims.len(), 8);
}

/// То, ради чего пакетный режим и написан: суммарный путь пачки, идущей в
/// одну точку, не больше, чем у жадного. Двенадцати пешек на ободе хватает,
/// чтобы жадный начал раздавать крайние слоты тем, кто остался, — то есть
/// не глядя на то, с какой стороны пешка подходит.
#[test]
fn a_batch_never_walks_further_than_the_greedy() {
    let desired = IVec2::new(10, 10);
    let members = ring(12, 40.0, desired);

    let cost = |matching| {
        let mut claims = claims_with(1);
        let batch: Vec<_> = members
            .iter()
            .map(|(entity, previous, from)| (*entity, *previous, desired, *from))
            .collect();
        let mut out = Vec::new();
        claim_batch(&mut claims, matching, &batch, anywhere, &mut out);
        out.iter()
            .zip(&members)
            .map(|(slot, (.., from))| {
                let (_, target) = slot.expect("место есть");
                (tile_center(target) - *from).length()
            })
            .sum::<f32>()
    };

    let greedy = cost(SlotMatching::Greedy);
    let batch = cost(SlotMatching::Batch);
    assert!(batch < greedy, "пакетный {batch} против жадного {greedy}");
}

/// Мест меньше, чем пешек, — хвост остаётся без слота и идёт в общую точку.
/// Тот же режим отказа, что у одиночного поиска.
#[test]
fn a_batch_short_of_slots_leaves_the_tail_without_one() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    let members = ring(8, 40.0, desired);
    let mut out = Vec::new();

    // проходима ровно одна клетка — цель, и больше ничего
    claims.claim_group(desired, &members, |tile| tile == desired, &mut out);

    assert_eq!(out.iter().filter(|slot| slot.is_some()).count(), 1);
}

/// Пачка из одного — это обычный поиск: пакетный режим не должен менять
/// ответ там, где выбирать не из чего (в игре из таких почти весь пакет).
#[test]
fn a_lone_member_of_a_batch_gets_the_greedy_answer() {
    let desired = IVec2::new(10, 10);
    let from = tile_center(desired) - Vec2::new(40.0, 0.0);
    let batch = [(entity(1), None, desired, from)];

    let answer = |matching| {
        let mut claims = claims_with(1);
        let mut out = Vec::new();
        claim_batch(&mut claims, matching, &batch, anywhere, &mut out);
        out[0]
    };

    assert_eq!(answer(SlotMatching::Batch), answer(SlotMatching::Greedy));
}

/// Пешки, равномерно расставленные по ободу вокруг цели.
fn ring(count: u32, radius: f32, around: IVec2) -> Vec<(Entity, Option<IVec2>, Vec2)> {
    (0..count)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            (
                entity(index + 1),
                None,
                tile_center(around) + Vec2::from_angle(angle) * radius,
            )
        })
        .collect()
}

/// Смена стороны слота выбрасывает индекс: ключи прошлой решётки не значат
/// ничего, а неверная занятость держалась бы весь прогон.
#[test]
fn changing_the_slot_side_drops_the_index() {
    let mut claims = claims_with(1);
    let desired = IVec2::new(10, 10);
    claims.claim_slot(entity(1), None, desired, tile_center(desired), anywhere);

    claims.sync(2, CLAIM_SEARCH_METERS);

    assert_eq!(claims.len(), 0);
}

/// Снятие заявки чужим претендентом ничего не трогает.
#[test]
fn releasing_someone_elses_slot_is_a_no_op() {
    let mut claims = claims_with(1);
    let tile = IVec2::new(10, 10);
    let (slot, _) = claims
        .claim_slot(entity(1), None, tile, tile_center(tile), anywhere)
        .expect("свободно");

    claims.release(slot, entity(2));

    assert!(!claims.is_free(slot, entity(3)));
}

/// Деспавн освобождает слот — на нём держится вся уборка: побег за край,
/// рестарт, смена города.
#[test]
fn despawn_releases_the_claim() {
    let mut app = App::new();
    app.init_resource::<DestinationClaims>()
        .add_observer(on_destination_claim_removed);

    let pawn = app.world_mut().spawn_empty().id();
    let tile = IVec2::new(10, 10);
    let slot = {
        let mut claims = app.world_mut().resource_mut::<DestinationClaims>();
        claims.sync(1, CLAIM_SEARCH_METERS);
        claims
            .claim_slot(pawn, None, tile, tile_center(tile), anywhere)
            .expect("свободно")
            .0
    };
    app.world_mut()
        .entity_mut(pawn)
        .insert(DestinationClaim(slot));

    app.world_mut().entity_mut(pawn).despawn();

    assert_eq!(app.world().resource::<DestinationClaims>().len(), 0);
}
