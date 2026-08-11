//! Заявки на жертв: сколько демонов уже гонятся за каждым человеком.
//!
//! Инвариант «максимум [`MAX_CHASERS_PER_TARGET`] преследователей на одного
//! человека» держался шестью рукописными правками `HashMap` в двух системах, а
//! освобождение места было `*entry(x).or_insert(1) -= 1` по `usize`: одна
//! новая ветка выхода из погони, не знающая, что вычитать надо не всегда, — и
//! счётчик уходит под ноль. Здесь занятость мест знает одно значение, а какие
//! выходы её освобождают — [`ChaseAction::releases_claim`].
//!
//! Счёт живёт ровно один тик: обе системы собирают его заново из своей
//! выборки. Не ресурс и не компонент намеренно — единственный источник правды
//! о том, кто кого гонит, это `ChaseTarget` на демоне, и отдельный счётчик
//! между тиками пришлось бы вести в такт спавнам и смертям.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::decide::MAX_CHASERS_PER_TARGET;

#[derive(Default, Debug)]
pub struct ChaseClaims(HashMap<Entity, usize>);

impl ChaseClaims {
    /// Счёт по целям уже идущих погонь — `ChaseTarget` каждого демона выборки.
    pub fn of(targets: impl Iterator<Item = Entity>) -> Self {
        let mut claims = Self::default();
        for target in targets {
            claims.claim(target);
        }
        claims
    }

    /// Демон встал в очередь на эту жертву.
    pub fn claim(&mut self, target: Entity) {
        *self.0.entry(target).or_insert(0) += 1;
    }

    /// Демон вышел из погони за ЖИВОЙ жертвой и освободил своё место.
    ///
    /// Освобождение места, которого никто не занимал, — не арифметическая
    /// мелочь, а признак того, что вызывающий отпустил цель дважды; счёт при
    /// этом остаётся на нуле, но в отладочной сборке это падает здесь, а не
    /// расходится тихо.
    pub fn release(&mut self, target: Entity) {
        let taken = self.0.entry(target).or_insert(0);
        debug_assert!(*taken > 0, "освобождено место, которое никто не занимал");
        *taken = taken.saturating_sub(1);
    }

    /// Мест больше нет: цель делят [`MAX_CHASERS_PER_TARGET`] демонов.
    pub fn is_full(&self, target: Entity) -> bool {
        !self.has_room_for(target, MAX_CHASERS_PER_TARGET)
    }

    /// Влезет ли ещё один преследователь при таком лимите. Лимит — параметр, а
    /// не константа: распадаясь из «клещей», демон ищет строго никем не занятую
    /// жертву, а не просто неполную (`SwitchRule::max_chasers`).
    pub fn has_room_for(&self, target: Entity, max: usize) -> bool {
        self.chasers(target) < max
    }

    fn chasers(&self, target: Entity) -> usize {
        self.0.get(&target).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(index: u32) -> Entity {
        Entity::from_raw_u32(index).expect("entity")
    }

    #[test]
    fn a_target_nobody_chases_has_room() {
        let claims = ChaseClaims::default();
        assert!(!claims.is_full(target(1)));
    }

    /// «Клещи» из двух допустимы, третий демон — уже толпа.
    #[test]
    fn two_chasers_fill_a_target() {
        let mut claims = ChaseClaims::default();
        claims.claim(target(1));
        assert!(!claims.is_full(target(1)), "одного мало для полной цели");
        claims.claim(target(1));
        assert!(claims.is_full(target(1)));
    }

    /// Заявки считаются по каждой цели отдельно.
    #[test]
    fn filling_one_target_leaves_the_others_free() {
        let mut claims = ChaseClaims::of([target(1), target(1)].into_iter());
        assert!(claims.is_full(target(1)));
        assert!(!claims.is_full(target(2)));
        claims.claim(target(2));
        assert!(!claims.is_full(target(2)));
    }

    /// Отказ от погони возвращает место следующему демону — то, ради чего
    /// `GaveUp` вообще освобождает заявку.
    #[test]
    fn releasing_frees_the_slot_for_the_next_demon() {
        let mut claims = ChaseClaims::of([target(1), target(1)].into_iter());
        claims.release(target(1));
        assert!(!claims.is_full(target(1)));
    }

    /// Лимит смены цели на «клещах»: годится только жертва, которую не
    /// преследует вообще никто.
    #[test]
    fn a_limit_of_one_wants_a_target_nobody_took() {
        let mut claims = ChaseClaims::default();
        assert!(claims.has_room_for(target(1), 1));
        claims.claim(target(1));
        assert!(!claims.has_room_for(target(1), 1));
        // при общем лимите место ещё есть — правила разные, счёт один
        assert!(!claims.is_full(target(1)));
    }

    /// Двойное освобождение — ошибка вызывающего, а не «ну и ладно»: раньше
    /// оно вычитало из `usize` и уводило счётчик под ноль.
    #[test]
    #[should_panic(expected = "никто не занимал")]
    fn releasing_a_slot_nobody_took_is_caught() {
        let mut claims = ChaseClaims::default();
        claims.release(target(1));
    }
}
