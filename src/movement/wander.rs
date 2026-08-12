//! Общий скелет одного шага выбора цели прогулки.
//!
//! Люди и демоны гуляют по-разному, но *порядок*, в котором это делается, у них
//! один и тот же, и раньше был выписан дважды — в `human/systems.rs` и
//! `demon/systems.rs`, — местами дословно: отсев по состоянию (шесть строк
//! знак в знак), подрезка цели к карте (вместе с константой отступа,
//! объявленной в обоих файлах в обход правила «тюнинг живёт в `settings.rs`»),
//! просев цели и подача заявки (эти две — четырежды, считая перепрокладку
//! бегства и погони).
//!
//! Различается ровно одно: **куда** пешка хочет пойти. Оно и остаётся у вида.
//!
//! ## Порядок шагов — сам по себе инвариант
//!
//! Поток решений обязан сдвигаться ровно на тех тиках, на которых сдвигался
//! раньше (см. `CONTEXT.md`, «Decision stream»), поэтому скелет **не** заводит
//! `SimRng` сам: вид заводит его на своём месте — после отсева и после паузы, —
//! и передаёт сюда уже готовую точку. Функция, взявшая на себя `next`, сдвигала
//! бы счётчик и на тех пешках, которые в этом кадре решения не принимают, и
//! повтор бы разъехался.

use bevy::prelude::*;

use super::components::{Movable, MovableState};
use crate::grid::{tile_center, world_to_tile};
use crate::navigation::Walkable;
use crate::rng::SimRng;
use crate::settings::{MAP_SIZE, WANDER_MAP_MARGIN};

/// Принимает ли пешка решение прямо сейчас.
///
/// Те же два состояния, что держит маркер `NeedsWanderTarget`; проверка в теле
/// цикла осталась подстраховкой на случай, если маркер разъедется с состоянием.
pub fn ready_to_pick(state: &MovableState) -> bool {
    matches!(
        state,
        MovableState::Idle | MovableState::PathfindingError(_)
    )
}

/// Случайная точка в конусе вокруг курса, подрезанная к карте.
///
/// Общая форма всех трёх «побродить» в проекте: повернуть курс на случайный
/// угол в пределах `half_angle`, отойти на случайное расстояние из `range`,
/// прижать результат к карте. Отличаются они только числами — у человека конус
/// 60° и 20–40 м, у демона 1.3 рад и 40–120 м.
///
/// **Два броска и ровно в этом порядке** — сначала угол, потом расстояние: их
/// число и очерёдность входят в поток решений пешки.
pub fn point_in_cone(
    rng: &mut SimRng,
    from: Vec2,
    heading: Vec2,
    half_angle: f32,
    range: (f32, f32),
) -> Vec2 {
    use rand::Rng;

    let turn = rng.random_range(-half_angle..half_angle);
    let direction = Vec2::from_angle(turn).rotate(heading);
    let distance = rng.random_range(range.0..range.1);
    clamp_to_map(from + direction * distance)
}

/// Прижать цель к карте, оставив отступ от края.
///
/// У самой кромки это разворачивает направление внутрь — то, на чём держится
/// «у края карты демон естественно заворачивает».
pub fn clamp_to_map(point: Vec2) -> Vec2 {
    point.clamp(Vec2::splat(WANDER_MAP_MARGIN), MAP_SIZE - WANDER_MAP_MARGIN)
}

/// Просеять цель по проходимости и подать заявку на путь.
///
/// Возвращает **фактически выбранный** тайл — не тот, что просили: просев
/// сдвигает цель на ближайший проходимый, и курс человека обязан считаться уже
/// по нему, иначе пешка запоминает направление, которым не пошла.
///
/// `None` — цель непроходима и рядом ничего нет; вызывающий пропускает пешку до
/// следующего решения.
pub fn request_wander_path(
    commands: &mut Commands,
    walkable: &Walkable,
    entity: Entity,
    movable: &mut Movable,
    from: Vec2,
    target: Vec2,
) -> Option<IVec2> {
    let target_tile = walkable.sift_target(world_to_tile(target))?;
    movable.to_pathfinding(entity, world_to_tile(from), target_tile, commands);
    Some(target_tile)
}

/// Направление на фактически выбранный тайл — то, чем человек обновляет курс.
/// `None`, когда цель совпала с текущим тайлом: курс тогда остаётся прежним.
pub fn heading_towards(from: Vec2, target_tile: IVec2) -> Option<Vec2> {
    (tile_center(target_tile) - from).try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{RngDomain, decision_stream};

    fn rng() -> SimRng {
        decision_stream(1, RngDomain::Human, 0, 1)
    }

    #[test]
    fn a_pawn_mid_route_picks_nothing() {
        assert!(ready_to_pick(&MovableState::Idle));
        assert!(ready_to_pick(&MovableState::PathfindingError(IVec2::ZERO)));
        assert!(!ready_to_pick(&MovableState::Moving(IVec2::ZERO)));
        assert!(!ready_to_pick(&MovableState::Pathfinding(IVec2::ZERO)));
    }

    /// Конус держит курс: любая выборка ложится в пределах полураствора.
    #[test]
    fn every_point_lands_inside_the_cone() {
        let from = MAP_SIZE / 2.0;
        let heading = Vec2::X;
        let half_angle = 0.5;
        let rng = &mut rng();

        for _ in 0..200 {
            let point = point_in_cone(rng, from, heading, half_angle, (20.0, 40.0));
            let direction = (point - from).normalize();
            let angle = direction.angle_to(heading).abs();
            assert!(angle <= half_angle + 1e-4, "угол {angle} вне конуса");
        }
    }

    /// Дальность — из заданного диапазона, а не из ниоткуда.
    #[test]
    fn the_distance_stays_in_range() {
        let from = MAP_SIZE / 2.0;
        let rng = &mut rng();

        for _ in 0..200 {
            let point = point_in_cone(rng, from, Vec2::Y, 0.3, (40.0, 120.0));
            let distance = point.distance(from);
            assert!(
                (40.0..=120.0).contains(&distance),
                "дистанция {distance} вне диапазона"
            );
        }
    }

    /// Цель за краем карты подрезается внутрь — иначе просев ищет проходимый
    /// тайл за границей сетки и не находит ничего.
    #[test]
    fn a_target_past_the_edge_is_pulled_back_inside() {
        let far = clamp_to_map(Vec2::new(-500.0, MAP_SIZE.y + 500.0));
        assert_eq!(far.x, WANDER_MAP_MARGIN);
        assert_eq!(far.y, MAP_SIZE.y - WANDER_MAP_MARGIN);
        // точка внутри карты остаётся собой
        let inside = MAP_SIZE / 2.0;
        assert_eq!(clamp_to_map(inside), inside);
    }

    /// Курс считается по выбранному тайлу, а не по запрошенной точке.
    #[test]
    fn the_heading_follows_the_chosen_tile() {
        let from = Vec2::new(100.0, 100.0);
        let east = world_to_tile(from + Vec2::X * 50.0);
        let direction = heading_towards(from, east).expect("тайл не совпадает с позицией");
        assert!(direction.x > 0.9, "курс не смотрит на восток: {direction}");
    }

    /// Цель в своём же тайле курс не переписывает: направления там нет.
    #[test]
    fn a_target_on_the_spot_leaves_the_heading_alone() {
        let from = tile_center(IVec2::new(50, 50));
        assert!(heading_towards(from, IVec2::new(50, 50)).is_none());
    }
}
