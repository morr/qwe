mod behavior;
mod claims;
mod components;
mod decide;
mod systems;

use bevy::prelude::*;

use self::behavior::{acquire_targets, chase, devour, on_demon_caught_human, pulse_devouring};
pub use self::components::{
    ChaseRepath, ChaseTarget, Demon, DemonCaughtHumanEvent, DemonChaseTag, DemonDevourTag,
    DemonLungeTag, DemonSpawner, DemonStyle, DemonWanderTag, DevourUntil,
};
use self::systems::{
    draw_lunge_paths, pick_wander_targets, spawn_initial_burst, sync_demon_speed, tick_spawner,
};
use crate::loading::{AppState, PlayPhase, WorldStarted};
use crate::prefs::TrackPrefExt;
use crate::spatial::SimSet;

pub struct DemonPlugin;

impl Plugin for DemonPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Demon>()
            .register_type::<DemonWanderTag>()
            .register_type::<DemonChaseTag>()
            .register_type::<DemonDevourTag>()
            .register_type::<DemonLungeTag>()
            .register_type::<ChaseTarget>()
            .register_type::<ChaseRepath>()
            .register_type::<DevourUntil>()
            .register_type::<DemonStyle>()
            .init_resource::<DemonSpawner>()
            .init_resource::<DemonStyle>()
            .track_pref::<DemonStyle>()
            .add_observer(on_demon_caught_human)
            .add_observer(on_world_started)
            // `Live`, а не `Playing`: номера демонам раздаёт спавнер, а обнуляет
            // его `WorldStarted` — объявляемый на входе в `Live`. Тик, успевший
            // пройти раньше объявления, выпускает залп со старым счётчиком, и
            // сброс раздаёт те же `PawnId` второму залпу: в мире оказываются два
            // демона №1, а на них стоит весь детерминизм (поток ГПСЧ пешки, ключ
            // очереди диспетчера). До сих пор этого не случалось только потому,
            // что прогрев держит мир на паузе, — но пауза принадлежит
            // `sim_time`, снимается пробелом и в детерминированном режиме длится
            // все 11–14 с постройки навигации.
            .add_systems(
                FixedUpdate,
                (spawn_initial_burst, tick_spawner)
                    .chain()
                    .run_if(in_state(PlayPhase::Live)),
            )
            // выбор цели блуждания — в `FixedUpdate`, а не в `Update`: это
            // решение симуляции, и в `Update` оно шло по разу на кадр, то есть
            // зависело от fps. Демонов сотни, лишних прогонов эта система не
            // боится
            .add_systems(
                FixedUpdate,
                (pick_wander_targets, acquire_targets, chase, devour)
                    .chain()
                    .in_set(SimSet::DemonBehavior),
            )
            .add_systems(
                Update,
                (
                    // косметика: пульсация масштаба спрайта и отрисовка
                    // траектории броска — на симуляцию не влияют
                    pulse_devouring,
                    draw_lunge_paths,
                    sync_demon_speed.run_if(resource_changed::<DemonStyle>),
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

/// Новый прогон мира — спавнер с нуля: `spawned = 0` возвращает демонам те же
/// номера, а значит и те же потоки ГПСЧ (см. `src/rng.rs`).
///
/// Ровно поэтому у сброса есть предусловие: **живых демонов быть не должно**.
/// Возврат номеров безопасен лишь тогда, когда прежние их владельцы уже сняты
/// со сцены; иначе в мире оказываются два демона с одним `PawnId`, а на его
/// уникальности стоят и поток ГПСЧ пешки, и ключ очереди детерминированного
/// диспетчера (`movement::order::pawn_key`). Все три штатных пути сброса —
/// вход в `Live`, R и смена города — снимают демонов раньше события; жалоба
/// здесь означает четвёртый путь, а не мелочь в счёте.
fn on_world_started(
    _event: On<WorldStarted>,
    mut spawner: ResMut<DemonSpawner>,
    alive: Query<(), With<Demon>>,
) {
    let alive = alive.iter().count();
    if alive > 0 {
        warn!("world start with {alive} demons still alive: their PawnIds are about to be reused");
    }
    debug_assert_eq!(alive, 0, "сброс спавнера под живыми демонами");
    *spawner = DemonSpawner::default();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::navigation::{ArcNavmesh, Backend, Navmesh};
    use crate::portal::PortalPos;
    use crate::rng::PawnId;

    /// Расписание берётся у самого [`DemonPlugin`]: гейт фазы и есть то, что
    /// проверяется, и повторять его регистрацию в тесте значило бы проверять
    /// копию вместо оригинала.
    fn app() -> App {
        let mut app = App::new();
        // косметика (`draw_lunge_paths`) просит `Gizmos` из рендера, которого
        // здесь нет; симуляции она не касается
        app.set_error_handler(bevy::ecs::error::warn);
        let navmesh = Arc::new(RwLock::new(Navmesh::default()));
        app.add_plugins((MinimalPlugins, StatesPlugin, DemonPlugin))
            // без ручного шага кадр длится микросекунды, `Time<Fixed>` не
            // набирает ни одного шага — и обе половины теста были бы пусты не
            // потому, что залпа нет, а потому, что тиков нет
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(Backend::from_grid(navmesh.clone()))
            .insert_resource(ArcNavmesh(navmesh))
            .insert_resource(PortalPos(Vec2::splat(100.0)))
            .insert_resource(crate::rng::WorldSeed(1))
            .init_resource::<bevy::diagnostic::DiagnosticsStore>()
            .init_resource::<crate::spatial::SpatialGrid<crate::human::Human>>()
            .init_resource::<crate::telemetry::Telemetry>()
            .add_plugins(crate::loading::SimBootPlugin);
        app
    }

    fn demon_pawn_ids(app: &mut App) -> Vec<u32> {
        let mut query = app.world_mut().query_filtered::<&PawnId, With<Demon>>();
        let mut ids: Vec<u32> = query.iter(app.world()).map(|pawn_id| pawn_id.0).collect();
        ids.sort_unstable();
        ids
    }

    /// Прогрев, в котором мир поехал: пауза принадлежит `sim_time` и снимается
    /// пробелом, а в детерминированном режиме прогрев длится все 11–14 с
    /// постройки навигации — так что тик до объявления старта не гипотеза.
    fn tick_through_a_moving_warmup(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        for _ in 0..4 {
            app.update();
        }
    }

    /// Регрессия: два демона №1 в одном мире.
    ///
    /// Номера раздаёт спавнер, а обнуляет его `WorldStarted` на входе в `Live`.
    /// Залп, успевший пройти раньше объявления, доставался второму залпу теми
    /// же `PawnId` — и `dispatch_pathfinding_requests_deterministic` падал на
    /// неуникальном ключе очереди тиков через тридцать после старта.
    #[test]
    fn a_warmup_that_ticks_does_not_hand_out_a_pawn_id_twice() {
        let app = &mut app();
        tick_through_a_moving_warmup(app);
        // без этого тест был бы пуст: демонов нет не потому, что их не пустил
        // гейт, а потому, что `FixedUpdate` вообще не прогонялся
        assert!(
            app.world().resource::<Time<Fixed>>().elapsed_secs() > 0.0,
            "прогрев обязан тикать, иначе проверять нечего"
        );
        assert!(
            demon_pawn_ids(app).is_empty(),
            "до объявления старта мира демонов быть не должно"
        );

        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        for _ in 0..4 {
            app.update();
        }

        let ids = demon_pawn_ids(app);
        assert!(!ids.is_empty(), "в `Live` залп обязан выйти");
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique, "номера демонов повторились: {ids:?}");
    }
}
