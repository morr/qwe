//! Слоты назначения (destination slots): у каждой пешки своя конечная точка.
//!
//! **Слот** — блок `k × k` навтайлов, зарезервированный одной пешкой под цель.
//! Заявка ([`DestinationClaim`]) выдаётся при выборе цели; занятый слот уводит
//! цель на ближайший свободный — кольцевым поиском наружу.
//!
//! Зачем. Без заявок несколько пешек целятся в одну точку, а занять её может
//! одна. Выхода из этого у симуляции нет: точка пути снимается, только когда
//! шаг тика накрывает остаток дистанции (`systems::move_moving_entities`), а
//! упёршуюся пешку расталкивание отбрасывает назад ровно настолько, насколько
//! она шагнула, — перекрытие встаёт на равновесие
//! `HUMAN_WALK_SPEED / (SEPARATION_RATE · доля)` = 0.7 м и держится вечно.
//! Прибытие же требует точного совпадения тайла. В итоге пешки либо наматывают
//! круги вокруг общей цели, либо стоят колом и дрожат на шаг за кадр, либо,
//! дойдя, никогда не замирают.
//!
//! # Почему блок тайлов, а не тайл и не своя решётка
//!
//! Цель здесь везде ключуется навтайлом — `MovableState`,
//! [`PathfindingRequest::end_tile`], фильтр устаревших ответов, тест прибытия,
//! да и полигональный меш ищет в `tile_center(end_tile)`. Точка, не являющаяся
//! центром тайла, — это точка, до которой пешка не может «дойти» по
//! определению, поэтому отдельной решётки с дробным шагом тут быть не может.
//!
//! Но и «один хозяин на тайл» не годится: гарантия «осевшая толпа не
//! перекрывается» держалась бы, только пока дистанция покоя `2 · radius`
//! не больше тайла. Её ломает и `NavtileBase::M1` (тайл 1 м против покоя 1.8 м),
//! и ползунок [`HumanStyle::body_radius`], который задуман изменчивым.
//!
//! Отсюда блок: `k = ceil(дистанция покоя / navtile_size())`, слот —
//! `tile.div_euclid(k)`, цель слота — строго ЦЕНТРАЛЬНЫЙ тайл блока
//! (`slot · k + k / 2`). Цели соседних слотов отстоят ровно на `k · navtile`,
//! то есть не меньше дистанции покоя, при любой комбинации настроек, и всё
//! это в целых числах — без пересчёта координат и накопления ошибки.
//!
//! Центральный тайл фиксирован намеренно. Если позволить блоку выбирать в себе
//! любой проходимый тайл, два соседних блока выберут смежные углы и сойдутся на
//! расстояние в один тайл — ровно та беда, от которой блок и заводился. Плата:
//! блок с непроходимым центром не используется вовсе, и шаг округляется вверх
//! до кратного тайлу (до ~2× дистанции покоя в худшем случае), то есть толпа
//! местами паркуется просторнее необходимого.
//!
//! Заявка живёт от выбора цели до СЛЕДУЮЩЕГО выбора; на прибытии она
//! намеренно не снимается — стоящая на своём слоте пешка и есть та занятость,
//! которую слоты моделируют.
//!
//! # Кого слоты не касаются
//!
//! - **погоня** (`DemonChaseTag`) — общая цель у преследователей это механика
//!   («клещи», `MAX_CHASERS_PER_TARGET`), сдвиг сломал бы и её, и передачу в
//!   бросок;
//! - **бегство** (`HumanFleeTag`) — цели переприкладываются раз в 0.7–1.2 с,
//!   разведены веером `personal_spread` и часто лежат за краем карты. Заявки на
//!   них — сотни кольцевых поисков в секунду впустую, а давка при панике всё
//!   равно определяется не целями, а шириной улицы.
//!
//! # Детерминизм
//!
//! В отличие от расталкивания слоты работают в ОБОИХ режимах: это симуляция, а
//! не косметика. Поэтому здесь нет ни гейта по камере (незаявленные кучи
//! копились бы за кадром, и камера приезжала бы ровно в них), ни зависимости от
//! `FrameCount`. Итерации по `HashMap` в выход не попадают — только точечные
//! `get`/`insert`/`remove`, — а порядок назначения задаётся сортировкой пакета
//! по `(вид, PawnId)`, как в `apply_pathfinding_results`.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::grid::tile_center;
use crate::human::HumanStyle;
use crate::movement::components::{Movable, MovableState, PathfindingRequest};
use crate::navigation::ArcNavmesh;
use crate::settings::{CLAIM_SEARCH_METERS, navtile_size};

/// Слот, зарезервированный этой пешкой под конечную точку (координаты решётки
/// слотов, не тайлов). Источник истины; [`DestinationClaims`] — обратный индекс.
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Component)]
pub struct DestinationClaim(pub IVec2);

/// Докуда ищется свободный слот, м. Ресурс, а не константа: у величины нет
/// правильного значения, есть компромисс — мало, и хвост большой толпы остаётся
/// без слотов (уходит в общую точку); много, и цель уезжает от задуманной
/// поведением так далеко, что это уже другая цель. Подбирается ползунком на
/// живой толпе (`examples/demos/crowd_demo.rs`), как радиус расталкивания.
///
/// Намеренно НЕ `SettingsGroup`: в игре величина не имеет смысла как вкус
/// пользователя, а демо, подвинув её, не должно править конфиг игры.
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
pub struct SlotSearch(pub f32);

impl Default for SlotSearch {
    fn default() -> Self {
        Self(CLAIM_SEARCH_METERS)
    }
}

/// Сторона слота в тайлах: `ceil(дистанция покоя / navtile_size())`, но не
/// меньше одного тайла.
pub fn slot_side(rest_distance: f32) -> i32 {
    (rest_distance / navtile_size()).ceil().max(1.0) as i32
}

/// Слот, которому принадлежит тайл. `div_euclid`, а не деление: тайлы у края
/// карты бывают отрицательными, а обычное деление на нуле «складывается»
/// и склеило бы два соседних слота в один.
pub fn slot_of(tile: IVec2, side: i32) -> IVec2 {
    IVec2::new(tile.x.div_euclid(side), tile.y.div_euclid(side))
}

/// Цель слота — центральный тайл блока. Только он, см. док модуля.
pub fn slot_target(slot: IVec2, side: i32) -> IVec2 {
    slot * side + IVec2::splat(side / 2)
}

/// Кто какой слот занял. Индекс, а не владелец: сущность без
/// [`DestinationClaim`] здесь появиться не может.
#[derive(Resource, Default)]
pub struct DestinationClaims {
    by_slot: HashMap<IVec2, Entity>,
    /// Сторона слота, на которой построен индекс. Ползунок радиуса и
    /// переключатель навтайла меняют её на лету, а ключи от прошлой решётки
    /// после этого не значат ничего.
    side: i32,
    /// Докуда искать свободный слот, м — снимок [`SlotSearch`] на прогон.
    search_meters: f32,
}

impl DestinationClaims {
    /// Свободен ли слот для этого претендента. Своя же заявка помехой себе не
    /// является — иначе повторный выбор той же цели уводил бы пешку в сторону.
    pub fn is_free(&self, slot: IVec2, claimant: Entity) -> bool {
        self.by_slot
            .get(&slot)
            .is_none_or(|owner| *owner == claimant)
    }

    /// Принять текущие настройки решётки: сторону слота и радиус поиска.
    ///
    /// Смена стороны выбрасывает индекс — старые ключи не пересчитываются:
    /// заявки восстановятся сами при следующем выборе цели, а неверная
    /// занятость держалась бы весь прогон. Радиус поиска ничего не
    /// перекеивает, он живёт только внутри одного поиска.
    pub fn sync(&mut self, side: i32, search_meters: f32) {
        if self.side != side {
            self.by_slot.clear();
            self.side = side;
        }
        self.search_meters = search_meters;
    }

    /// Занять слот под `desired` или ближайший свободный к нему; вернуть слот и
    /// его целевой тайл. `from` — где пешка стоит сейчас.
    ///
    /// Ближайший — по расстоянию ДО ЦЕЛИ: толпа обязана набиваться от цели
    /// наружу, иначе первые же пришедшие встанут по краю круга поиска и в
    /// середине останется дыра. Но среди одинаково близких к цели слот достаётся
    /// тому, кто ближе К ПЕШКЕ, то есть лежащему с её стороны. Без этого выбор
    /// внутри кольца произволен: пешка с одного края обода получала место у
    /// противоположного, шла через всю толпу и толкалась со встречными всю
    /// дорогу.
    ///
    /// `None` — если в пределах [`SlotSearch`] свободного слота с
    /// проходимой целью нет. Тогда цель остаётся общей и БЕЗ заявки: это ровно
    /// сегодняшнее поведение, и оно лучше, чем застопорить пешке выбор цели.
    ///
    /// Кольцо не знает о связности: свободный слот может оказаться за стеной,
    /// тогда поиск пути провалится и поведение выберет новую цель. Режим отказа
    /// тот же, что уже терпит `rescue_from_impassable`, и ограничен радиусом
    /// поиска.
    pub fn claim_slot(
        &mut self,
        claimant: Entity,
        previous: Option<IVec2>,
        desired: IVec2,
        from: Vec2,
        passable: impl Fn(IVec2) -> bool,
    ) -> Option<(IVec2, IVec2)> {
        // старую заявку снимаем ДО поиска: иначе пешка, выбравшая ту же
        // окрестность, обходила бы собственный прошлый слот
        if let Some(previous) = previous {
            self.release(previous, claimant);
        }
        let side = self.side;
        let radius = (self.search_meters / (side as f32 * navtile_size())).ceil() as i32;
        let slot = self.nearest_free_slot(
            claimant,
            slot_of(desired, side),
            radius.max(1),
            from,
            &passable,
        )?;
        self.by_slot.insert(slot, claimant);
        Some((slot, slot_target(slot, side)))
    }

    /// Свободный слот, ближайший к `desired`, а при равенстве — к `from`.
    ///
    /// Кольцевой поиск, как `navigation::nearest_tile_where`, но со вторым
    /// ключом, ради которого он и написан отдельно. Кольца чебышёвские, а
    /// первый ключ евклидов, поэтому кольцо, где нашёлся лучший кандидат, не
    /// последнее: тайл кольца `r` не ближе `r`, значит обход идёт, пока
    /// `r² ≤ лучшего` — на одно кольцо дальше, чем при поиске без второго
    /// ключа, чтобы кандидаты на том же расстоянии от цели не потерялись.
    fn nearest_free_slot(
        &self,
        claimant: Entity,
        desired: IVec2,
        radius: i32,
        from: Vec2,
        passable: &impl Fn(IVec2) -> bool,
    ) -> Option<IVec2> {
        let mut best: Option<(i32, f32, IVec2)> = None;
        for ring in 0..=radius {
            if let Some((to_target, ..)) = best
                && ring * ring > to_target
            {
                break;
            }
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    // только само кольцо: его внутренность просмотрена раньше
                    if ring > 0 && dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let slot = desired + IVec2::new(dx, dy);
                    if !self.is_free(slot, claimant) {
                        continue;
                    }
                    let target = slot_target(slot, self.side);
                    if !passable(target) {
                        continue;
                    }
                    let score = (
                        dx * dx + dy * dy,
                        (tile_center(target) - from).length_squared(),
                    );
                    if best.is_none_or(|(to_target, to_pawn, _)| score < (to_target, to_pawn)) {
                        best = Some((score.0, score.1, slot));
                    }
                }
            }
        }
        best.map(|(.., slot)| slot)
    }

    /// Снять заявку — только если слот числится за этой сущностью: после
    /// перестройки решётки ключ мог достаться кому-то другому.
    pub fn release(&mut self, slot: IVec2, claimant: Entity) {
        if self.by_slot.get(&slot) == Some(&claimant) {
            self.by_slot.remove(&slot);
        }
    }

    pub fn clear(&mut self) {
        self.by_slot.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_slot.len()
    }
}

/// Развести свежие заявки на поиск пути по своим слотам.
///
/// Врезка одна и не в системы поведения: так слоты покрывают блуждание людей,
/// блуждание демонов и тестового ходока, не трогая ни `human/*`, ни `demon/*`.
///
/// Перезапись `movable.state` обязательна и обязана быть условной: фильтр
/// устаревших ответов в `apply_result` сверяет `end_tile` ответа с целью в
/// состоянии, и разъехавшись, они выбросили бы каждый найденный путь.
pub fn assign_destination_slots(
    mut commands: Commands,
    navmesh: Res<ArcNavmesh>,
    style: Res<HumanStyle>,
    search: Res<SlotSearch>,
    mut claims: ResMut<DestinationClaims>,
    mut fresh: Query<
        (
            Entity,
            Option<&crate::rng::PawnId>,
            Has<crate::human::Human>,
            &crate::movement::SimPosition,
            &mut Movable,
            &mut PathfindingRequest,
            Option<&DestinationClaim>,
        ),
        (
            Added<PathfindingRequest>,
            Without<crate::human::HumanFleeTag>,
            Without<crate::demon::DemonChaseTag>,
        ),
    >,
    mut order: Local<Vec<(f32, u8, u32, Entity)>>,
) {
    claims.sync(slot_side(style.body_radius * 2.0), search.0);

    // Пакет обрабатывается от ближних к своей цели к дальним: место у цели
    // должно достаться тому, кто рядом, а не тому, у кого меньше `PawnId`.
    // Иначе пешка с дальнего края забирает середину, а стоящая вплотную уезжает
    // наружу — лишний крюк обоим и встречный поток между ними.
    //
    // Хвост ключа — вид и номер пешки: сам по себе порядок обхода архетипов
    // ничего не гарантирует, а равные расстояния в толпе обычное дело. Тот же
    // ключ, что у детерминированного диспетчера.
    order.clear();
    order.extend(
        fresh
            .iter()
            .map(|(entity, pawn_id, is_human, position, _, request, _)| {
                (
                    (tile_center(request.end_tile) - position.0).length_squared(),
                    u8::from(is_human),
                    pawn_id.map_or(u32::MAX, |pawn_id| pawn_id.0),
                    entity,
                )
            }),
    );
    if order.is_empty() {
        return;
    }
    order.sort_unstable_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| (left.1, left.2).cmp(&(right.1, right.2)))
    });

    let navmesh = navmesh.read();
    for (.., entity) in order.iter() {
        let Ok((_, _, _, position, mut movable, mut request, claim)) = fresh.get_mut(*entity)
        else {
            continue;
        };
        let desired = request.end_tile;
        let slot = claims.claim_slot(
            *entity,
            claim.map(|claim| claim.0),
            desired,
            position.0,
            |tile| navmesh.is_passable(tile.x, tile.y),
        );
        match slot {
            Some((slot, target)) => {
                if target != desired {
                    request.end_tile = target;
                    if movable.state == MovableState::Pathfinding(desired) {
                        movable.state = MovableState::Pathfinding(target);
                    }
                }
                commands.entity(*entity).insert(DestinationClaim(slot));
            }
            // свободного слота нет — идём в общую точку, как раньше
            None => {
                commands.entity(*entity).remove::<DestinationClaim>();
            }
        }
    }
}

/// Снятие компонента — единственный путь освобождения индекса при сносе
/// сущности: деспавн поднимает `Remove` на каждый её компонент, и обсервер
/// успевает прочитать заявку, пока она ещё видна.
///
/// Покрывает побег за край карты, рестарт и смену города
/// (`DespawnOnExit(AppState::Playing)`), а также раздевание трупа.
pub fn on_destination_claim_removed(
    event: On<Remove, DestinationClaim>,
    mut claims: ResMut<DestinationClaims>,
    claimed: Query<&DestinationClaim>,
) {
    if let Ok(claim) = claimed.get(event.entity) {
        claims.release(claim.0, event.entity);
    }
}

/// Вход в мир начинается с пустого индекса.
pub fn reset_destination_claims(mut claims: ResMut<DestinationClaims>) {
    claims.clear();
}

#[cfg(test)]
mod tests {
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
}
