//! Мягкое расталкивание пешек (anti-overlap): пешки в кадре не стоят друг на
//! друге. Пути идут через центры навтайлов, спавн и паника сгоняют толпу в
//! одну точку — и без «личного пространства» пешки регулярно сливаются в одну.
//!
//! Механизм намеренно локальный и косметический:
//! - **только вьюпорт и близкий зум** — за кадром и на отдалении перекрытие не
//!   видно, и симуляция за него не платит; пачка, въехавшая в кадр при движении
//!   камеры, расходится на глазах за доли секунды;
//! - **не чаще раза в кадр** — система живёт в `FixedUpdate` (ей нужен момент
//!   после `move_moving_entities`), но на 30x там ~1920 тиков в секунду, и
//!   даже 0.03 мс на тик съели бы ~6% реальной секунды. Чаще кадра
//!   расталкивание физически не видно, а на скорости 1x кадр ≈ тик, так что в
//!   режиме «медленно разглядываю толпу» гейт ничего не отнимает;
//! - **мягкое** — за прогон разрешается доля перекрытия ([`SEPARATION_RATE`],
//!   кламп [`SEPARATION_MAX_STEP`]): в давке мгновенное перекрытие возможно,
//!   но живёт доли секунды. Жёсткая релаксация до полного разведения в той же
//!   давке разлеталась бы цепными толчками, как телепорт.
//!
//! Толчок пишется в `SimPosition` после шага движения: снимок
//! `PreviousSimPosition` сделан в начале тика, так что интерполяция доводит
//! сдвиг до экрана плавно, а троттлимая перепрокладка путей (0.4–1.2 с) съедает
//! боковой дрейф — накапливаться ему не во что. Демон в броске
//! (`DemonLungeTag`) исключён целиком: он двигает `SimPosition` сам и обязан
//! сомкнуться до `KILL_DISTANCE`. Пожирающий (`DemonDevourTag`) стоит над
//! трупом неподвижно (mobility 0), но толпу от себя отталкивает. Трупы вне
//! механизма по построению — у них нет `SimPosition`.

use bevy::diagnostic::FrameCount;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::window::PrimaryWindow;

use crate::demon::{Demon, DemonDevourTag, DemonLungeTag};
use crate::grid::world_to_tile;
use crate::human::Human;
use crate::movement::components::SimPosition;
use crate::movement::systems::VIEW_MARGIN;
use crate::settings::{
    DEMON_BODY_RADIUS, HUMAN_BODY_RADIUS, SEPARATION_CELL, SEPARATION_MAX_STEP,
    SEPARATION_MAX_ZOOM, SEPARATION_RATE,
};
use crate::spatial::SpatialGrid;

/// Подвижность демона относительно человека: в паре человек забирает 4/5
/// коррекции — толпа обтекает демона, а не демон толпу.
const DEMON_MOBILITY: f32 = 0.25;

/// «Конец цепочки» в связных списках мелкой сетки.
const NONE: u32 = u32::MAX;

/// Тумблер расталкивания — панель World. Выбор пользователя, переживает
/// рестарт и смену города (тот же контракт, что у `DemonStyle`). Выключение
/// ничего не откатывает: уже разведённые позиции просто остаются как есть.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "separation")]
pub struct SeparationStyle {
    pub enabled: bool,
}

impl Default for SeparationStyle {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Участник одного прогона: снятая позиция плюс всё, что нужно паре.
#[derive(Clone, Copy)]
struct Pawn {
    entity: Entity,
    position: Vec2,
    radius: f32,
    /// Доля коррекции пары, пропорциональная подвижности: человек 1.0, демон
    /// [`DEMON_MOBILITY`], пожирающий 0.0 (толкает, но не двигается).
    mobility: f32,
}

/// Буферы прогона — в `Local`, чтобы steady state обходился без аллокаций.
/// `pub(super)` — тип стоит в сигнатуре системы, которую регистрирует `mod.rs`.
#[derive(Default)]
pub(super) struct SeparationState {
    /// Кадр последнего прогона: гейт «не чаще раза в кадр».
    last_frame: Option<u32>,
    /// Виртуальное время, накопленное тиками с последнего прогона.
    pending_dt: f32,
    pawns: Vec<Pawn>,
    /// Мелкая сетка соседей: голова цепочки по ячейке + связный список по
    /// индексам `pawns` — ни одного `Vec` на ячейку.
    heads: HashMap<IVec2, u32>,
    next: Vec<u32>,
    pushes: Vec<Vec2>,
}

fn fine_cell(pos: Vec2) -> IVec2 {
    (pos / SEPARATION_CELL).floor().as_ivec2()
}

/// Детерминированное направление разведения точно совпавших позиций — хэш
/// сущности, тот же трюк, что `personal_spread` у веера бегства: пара держит
/// свою ось от прогона к прогону, а не дрожит случайной.
fn coincident_direction(entity: Entity) -> Vec2 {
    let hash = entity.index().index().wrapping_mul(2654435761);
    let angle = (hash >> 8) as f32 / (u32::MAX >> 8) as f32 * std::f32::consts::TAU;
    Vec2::from_angle(angle)
}

/// Толчки всех пар текущего набора — чистая функция над буферами, без ECS
/// (тестируется напрямую). `fraction` — доля перекрытия, разрешаемая этим
/// прогоном; в `pushes` — суммарный сдвиг каждой пешки, ещё без клампа.
fn resolve_pushes(state: &mut SeparationState, fraction: f32) {
    state.heads.clear();
    state.next.clear();
    state.pushes.clear();
    state.pushes.resize(state.pawns.len(), Vec2::ZERO);
    for (i, pawn) in state.pawns.iter().enumerate() {
        let head = state.heads.insert(fine_cell(pawn.position), i as u32);
        state.next.push(head.unwrap_or(NONE));
    }

    for i in 0..state.pawns.len() {
        let a = state.pawns[i];
        let cell = fine_cell(a.position);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(&head) = state.heads.get(&(cell + IVec2::new(dx, dy))) else {
                    continue;
                };
                let mut j = head;
                while j != NONE {
                    // пара встречается с обеих сторон — обрабатываем один раз
                    if j > i as u32 {
                        let b = state.pawns[j as usize];
                        let weights = a.mobility + b.mobility;
                        let min_distance = a.radius + b.radius;
                        let offset = b.position - a.position;
                        let distance_squared = offset.length_squared();
                        if weights > 0.0 && distance_squared < min_distance * min_distance {
                            let distance = distance_squared.sqrt();
                            let direction = if distance > 1e-4 {
                                offset / distance
                            } else {
                                coincident_direction(a.entity)
                            };
                            let correction = direction * ((min_distance - distance) * fraction);
                            state.pushes[i] -= correction * (a.mobility / weights);
                            state.pushes[j as usize] += correction * (b.mobility / weights);
                        }
                    }
                    j = state.next[j as usize];
                }
            }
        }
    }
}

/// Прогон расталкивания: гейты → сбор видимых из грубых сеток → толчки →
/// применение с проверкой проходимости. Порядок в тике — строго после
/// `move_moving_entities` (см. цепочку в `movement/mod.rs`).
#[allow(clippy::too_many_arguments)]
pub fn separate_pawns(
    mut diagnostics: bevy::diagnostic::Diagnostics,
    style: Res<SeparationStyle>,
    frames: Res<FrameCount>,
    time: Res<Time>,
    navmesh: Res<crate::navigation::ArcNavmesh>,
    mut humans: ResMut<SpatialGrid<Human>>,
    demons: Res<SpatialGrid<Demon>>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut pawns: Query<
        (&mut SimPosition, Has<Demon>, Has<DemonDevourTag>),
        (Or<(With<Human>, With<Demon>)>, Without<DemonLungeTag>),
    >,
    mut state: Local<SeparationState>,
) {
    if !style.enabled {
        state.pending_dt = 0.0;
        return;
    }
    state.pending_dt += time.delta_secs();
    // не чаще раза в кадр: остальные тики того же кадра только копят dt
    if state.last_frame == Some(frames.0) {
        return;
    }
    state.last_frame = Some(frames.0);
    let fraction = (SEPARATION_RATE * state.pending_dt).min(1.0);
    state.pending_dt = 0.0;
    // на таком отдалении пешка — 1–2 пикселя, перекрытие не читается
    if camera.scale.x >= SEPARATION_MAX_ZOOM {
        return;
    }

    let started = std::time::Instant::now();
    let camera_position = camera.translation.truncate();
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x * VIEW_MARGIN;
    let min = camera_position - half_view;
    let max = camera_position + half_view;

    state.pawns.clear();
    {
        let pawn_buffer = &mut state.pawns;
        let mut collect = |entity: Entity| {
            // мимо запроса — бросок, труп, пешка чужого вида в чужой сетке
            let Ok((sim_position, is_demon, is_devouring)) = pawns.get(entity) else {
                return;
            };
            let position = sim_position.0;
            if position.x < min.x || position.x > max.x || position.y < min.y || position.y > max.y
            {
                return;
            }
            let (radius, mobility) = if is_devouring {
                (DEMON_BODY_RADIUS, 0.0)
            } else if is_demon {
                (DEMON_BODY_RADIUS, DEMON_MOBILITY)
            } else {
                (HUMAN_BODY_RADIUS, 1.0)
            };
            pawn_buffer.push(Pawn {
                entity,
                position,
                radius,
                mobility,
            });
        };
        humans.for_each_in_rect(min, max, &mut collect);
        demons.for_each_in_rect(min, max, &mut collect);
    }

    resolve_pushes(&mut state, fraction);

    let navmesh = navmesh.read();
    for i in 0..state.pawns.len() {
        let pawn = state.pawns[i];
        let push = state.pushes[i];
        if push == Vec2::ZERO {
            continue;
        }
        let target = pawn.position + push.clamp_length_max(SEPARATION_MAX_STEP);
        let tile = world_to_tile(target);
        // толчок в непроходимое отбрасывается: спасение (`rescue_*`) ловит
        // только провал поиска пути, задавленную в стену пешку оно бы не нашло
        if !navmesh.is_passable(tile.x, tile.y) {
            continue;
        }
        let Ok((mut sim_position, is_demon, _)) = pawns.get_mut(pawn.entity) else {
            continue;
        };
        sim_position.0 = target;
        // сетка людей инкрементальна; толчки мелкие, но стоячую пешку они
        // могут за много прогонов увести через границу 60-метровой ячейки
        if !is_demon && crate::spatial::cell_of(target) != crate::spatial::cell_of(pawn.position) {
            humans.insert(pawn.entity, target);
        }
    }
    crate::diagnostics::measure_ms(
        &mut diagnostics,
        &crate::diagnostics::SIM_SEPARATION_MS,
        started,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    fn pawn(index: u32, position: Vec2, radius: f32, mobility: f32) -> Pawn {
        Pawn {
            entity: entity(index),
            position,
            radius,
            mobility,
        }
    }

    fn state_with(pawns: Vec<Pawn>) -> SeparationState {
        SeparationState {
            pawns,
            ..Default::default()
        }
    }

    /// Перекрытая пара расходится в противоположные стороны вдоль своей оси,
    /// ровно на долю перекрытия.
    #[test]
    fn an_overlapping_pair_is_pushed_apart() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, 1.0);

        // перекрытие 0.4 м делится пополам: каждому по 0.2 вдоль оси пары
        assert!((state.pushes[0] - Vec2::new(-0.2, 0.0)).length() < 1e-4);
        assert!((state.pushes[1] - Vec2::new(0.2, 0.0)).length() < 1e-4);
    }

    /// Пара на дистанции суммы радиусов — уже не перекрытие: толчков нет,
    /// разведённая толпа не продолжает расползаться.
    #[test]
    fn a_settled_pair_is_left_alone() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.95, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, 1.0);

        assert_eq!(state.pushes[0], Vec2::ZERO);
        assert_eq!(state.pushes[1], Vec2::ZERO);
    }

    /// Точно совпавшие позиции разводятся по детерминированной оси, а не
    /// остаются на месте с нулевым направлением.
    #[test]
    fn coincident_pawns_get_a_deterministic_axis() {
        let position = Vec2::new(10.0, 10.0);
        let mut state = state_with(vec![
            pawn(1, position, 0.45, 1.0),
            pawn(2, position, 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, 1.0);

        assert!(state.pushes[0].length() > 1e-4);
        assert!((state.pushes[0] + state.pushes[1]).length() < 1e-4);

        // тот же набор — та же ось: направление не дрожит от прогона к прогону
        let first = state.pushes[0];
        resolve_pushes(&mut state, 1.0);
        assert!((state.pushes[0] - first).length() < 1e-6);
    }

    /// Неподвижный участник (пожирающий демон) толкает, но не двигается:
    /// вся коррекция достаётся подвижному.
    #[test]
    fn an_immovable_pawn_pushes_without_moving() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.9, 0.0),
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, 1.0);

        assert_eq!(state.pushes[0], Vec2::ZERO);
        let expected = 0.45 + 0.9 - 0.5;
        assert!((state.pushes[1] - Vec2::new(expected, 0.0)).length() < 1e-4);
    }

    /// Соседи через границу ячейки мелкой сетки видят друг друга: пара на
    /// стыке двух ячеек — всё ещё пара.
    #[test]
    fn a_pair_across_a_fine_cell_boundary_is_still_resolved() {
        let mut state = state_with(vec![
            pawn(1, Vec2::new(SEPARATION_CELL - 0.1, 1.0), 0.45, 1.0),
            pawn(2, Vec2::new(SEPARATION_CELL + 0.1, 1.0), 0.45, 1.0),
        ]);
        resolve_pushes(&mut state, 1.0);

        assert!(state.pushes[0].x < 0.0);
        assert!(state.pushes[1].x > 0.0);
    }
}
