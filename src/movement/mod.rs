mod components;
mod destination;
mod pathfinding;
mod separation;
mod systems;
#[cfg(test)]
mod tests;

use bevy::app::RunFixedMainLoop;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

pub use self::components::{
    Movable, MovableReachedDestinationEvent, MovableState, MovableStateMovingTag,
    NeedsWanderTarget, PathfindingRequest, PathfindingTask, PreviousSimPosition, RequestedAt,
    RetireAt, SimPosition,
};
pub use self::destination::{
    DestinationClaim, DestinationClaims, SlotLab, SlotMatching, SlotSearch,
    assign_destination_slots, claim_batch, regroup_onto_slots, slot_side, slot_target,
};
pub use self::pathfinding::wanderers_dispatched_at_zoom;
pub use self::separation::{
    SeparationHolds, SeparationLab, SeparationStats, SeparationSteer, SeparationStyle,
    demon_radius, separation_allowed_by_mode, separation_cell, separation_runs,
};
pub use self::systems::{DrawMovePaths, MOVEPATH_ARROW_TIP, MOVEPATH_COLOR};
use crate::loading::AppState;
use crate::prefs::TrackPrefExt;
use crate::spatial::SimSet;

use self::pathfinding::{
    apply_pathfinding_results, dispatch_pathfinding_requests,
    dispatch_pathfinding_requests_deterministic, listen_for_pathfinding_tasks,
    stamp_pathfinding_requests,
};
use self::systems::{
    draw_move_paths, interpolate_movable_transforms, move_moving_entities,
    on_movable_added_init_sim_position, polymesh_rebuilt, rescue_trapped_entities,
    snapshot_previous_sim_positions, toggle_draw_move_paths,
};

/// Запас видимости к полуразмеру экрана — чтобы пешки у кромки кадра не
/// «замирали» при лёгком движении камеры. Один и тот же запас у диспетчера
/// заявок (`pathfinding.rs`) и у расталкивания (`separation/`), поэтому живёт
/// у их общего родителя.
pub(crate) const VIEW_MARGIN: f32 = 1.2;

pub struct MovementPlugin;

impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Movable>()
            .register_type::<MovableStateMovingTag>()
            .register_type::<SimPosition>()
            .register_type::<PreviousSimPosition>()
            .init_resource::<DrawMovePaths>()
            .register_type::<DrawMovePaths>()
            .track_pref::<DrawMovePaths>()
            .init_resource::<SeparationStyle>()
            .register_type::<SeparationStyle>()
            .track_pref::<SeparationStyle>()
            .init_resource::<SeparationHolds>()
            .init_resource::<SeparationSteer>()
            .init_resource::<self::separation::SeparationBlock>()
            // стенд замеров (`examples/demos/crowd_demo.rs`): дефолт равен
            // константам, так что игра идёт по тем же веткам, что и раньше
            .init_resource::<SeparationLab>()
            .register_type::<SeparationLab>()
            .init_resource::<SeparationStats>()
            .register_type::<SeparationStats>()
            .init_resource::<DestinationClaims>()
            .register_type::<DestinationClaim>()
            .init_resource::<SlotSearch>()
            .register_type::<SlotSearch>()
            .init_resource::<SlotLab>()
            .register_type::<SlotLab>()
            // индекс заявок переживает сущности только через это снятие:
            // деспавн поднимает `Remove` на каждый компонент
            .add_observer(self::destination::on_destination_claim_removed)
            .add_systems(
                OnEnter(AppState::Playing),
                (
                    self::destination::reset_destination_claims,
                    self::separation::reset_separation_holds,
                    self::separation::reset_separation_steer,
                    self::separation::reset_separation_block,
                ),
            )
            // придержки чистит сам прогон расталкивания (тумблер, отзум), но
            // режим выключает его целиком — и переключение навигации на сетку
            // посреди игры оставило бы придержанных придержанными навсегда,
            // то есть замедленными без причины
            .add_systems(
                Update,
                (
                    self::separation::reset_separation_holds
                        .run_if(|holds: Res<SeparationHolds>| !holds.0.is_empty()),
                    self::separation::reset_separation_steer
                        .run_if(|steer: Res<SeparationSteer>| !steer.0.is_empty()),
                    self::separation::reset_separation_block.run_if(
                        |block: Res<self::separation::SeparationBlock>| !block.0.is_empty(),
                    ),
                )
                    .run_if(not(self::separation::separation_runs)),
            )
            // системы плагина пишут диагностику; без стора их параметры
            // не валидируются и шаг движения молча не выполняется
            .init_resource::<bevy::diagnostic::DiagnosticsStore>()
            // по той же причине: счётчик тиков стоит в цепочке этого плагина,
            // а плагин используется в тестах без `DeterminismPlugin`
            .init_resource::<crate::determinism::SimTick>()
            // и по ней же — радиус тела: расталкивание и слоты берут его из
            // настроек людей, а `HumanPlugin` в тестах и в демо-сценах может
            // отсутствовать. `init_resource` идемпотентен, так что настоящий
            // `HumanStyle` из `HumanPlugin` это не подменяет
            .init_resource::<crate::human::HumanStyle>();

        app.register_type::<PathfindingRequest>()
            .register_type::<NeedsWanderTarget>()
            .register_type::<self::components::RequestedAt>()
            .register_type::<self::components::RetireAt>();
        app.add_observer(on_movable_added_init_sim_position)
            // единственный полный скан — на готовую постройку полигонального
            // меша: только там проходимость меняется под уже стоящими пешками
            // (раздутые на радиус агента контуры). Скана на входе в мир нет
            // намеренно, см. `rescue_trapped_entities`
            .add_systems(
                Update,
                rescue_trapped_entities
                    .run_if(in_state(AppState::Playing))
                    .run_if(polymesh_rebuilt)
                    // в детерминированном режиме бэкенд заморожен на весь
                    // прогон (`DeterministicRun`), проходимость под стоящими
                    // пешками посреди прогона не меняется — спасать не от чего
                    .run_if(not(crate::determinism::deterministic)),
            )
            .add_systems(
                Update,
                // приёмка ДО диспетчера: снятые готовые таски освобождают
                // бюджет in-flight в этом же кадре. В обратном порядке бюджет
                // каждый кадр видел ~250 уже готовых, но не снятых тасков и
                // выдавал вдвое меньше новых — на 30x диспетчер хронически
                // голодал (156 из 258 стоящих бегущих ждали в очереди)
                (
                    listen_for_pathfinding_tasks,
                    // слот — ДО диспетчера и ПОСЛЕ выбора цели. Обе связи
                    // обязаны быть явными: `pick_wander_targets` людей живёт в
                    // `Update` вне этой цепочки, и без них заявка успевала
                    // уехать в поиск раньше, чем ей назначен слот, — через раз
                    // и невоспроизводимо
                    assign_destination_slots.after(crate::human::pick_wander_targets),
                    dispatch_pathfinding_requests,
                )
                    .chain()
                    // в детерминированном режиме этот конвейер заменён
                    // тик-локованным ниже: здесь ответ применяется в тот кадр,
                    // когда посчитался, а приоритет считается от камеры
                    .run_if(not(crate::determinism::deterministic)),
            )
            .add_systems(
                Update,
                (
                    toggle_draw_move_paths
                        .run_if(input_just_pressed(KeyCode::KeyM))
                        .run_if(not(crate::ui::typing_in_text_input)),
                    draw_move_paths,
                ),
            )
            // Симуляция движения — фиксированным шагом, визуальный `Transform` —
            // интерполяцией после цикла фиксированных шагов (идиома из
            // `examples/movement/physics_in_fixed_timestep.rs`).
            // Порядок внутри шага явный: снимок «прошлой» позиции — до того,
            // как её тронет поведение (демон в броске двигает `SimPosition`
            // сам), а шаг по пути — после всего поведения.
            .add_systems(
                FixedUpdate,
                // `.chain()` обязателен и сам по себе: привязки к `SimSet`
                // ничего не упорядочивают, когда эти множества пусты (плагин
                // движения используется отдельно в тестах)
                (
                    // счётчик тиков — голова цепочки: всё, что решает этот
                    // шаг, ссылается на один и тот же номер
                    crate::determinism::advance_sim_tick.run_if(in_state(AppState::Playing)),
                    snapshot_previous_sim_positions.before(SimSet::SpatialRebuild),
                    // ответы — до поведения: путь, приземлившийся на этом
                    // тике, на нём же и идётся
                    apply_pathfinding_results
                        .before(SimSet::SpatialRebuild)
                        .run_if(in_state(AppState::Playing))
                        .run_if(crate::determinism::deterministic),
                    move_moving_entities.after(SimSet::HumanBehavior),
                    // расталкивание — строго после шага движения: только там
                    // позиции тика финальны, а снимок уже сделан, и толчок
                    // доедет до экрана интерполяцией.
                    // Режимы, в которых его нет вовсе (детерминизм, сеточная
                    // навигация), — в `separation_runs`
                    separation::separate_pawns
                        .run_if(in_state(AppState::Playing))
                        .run_if(separation::separation_runs),
                    // возврат на свой слот — сразу после расталкивания, которое
                    // и есть источник схода: заявка, поданная этим тиком,
                    // уезжает в диспетчер в конце того же тика
                    self::destination::regroup_onto_slots.run_if(in_state(AppState::Playing)),
                    // слоты — только под детерминизмом: в обычном режиме любую
                    // заявку этого тика (и демонскую из `FixedUpdate`) до
                    // диспетчера успевает развести Update-копия выше —
                    // `FixedUpdate` идёт раньше `Update` в том же кадре, а
                    // `Added` взводится до первого наблюдения системой. Копия
                    // здесь платила бы `Added`-скан по архетипу из ~20k заявок
                    // на каждый тик (Added — проверка тиков по всем строкам, не
                    // индекс), на 30x — до ~30 сканов за кадр впустую
                    assign_destination_slots
                        .run_if(in_state(AppState::Playing))
                        .run_if(crate::determinism::deterministic),
                    // диспетчер — в самом конце тика: заявки, поданные
                    // поведением этого шага, командами применяются на точках
                    // синхронизации цепочки и уезжают в тот же тик
                    (
                        stamp_pathfinding_requests,
                        dispatch_pathfinding_requests_deterministic,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing))
                        .run_if(crate::determinism::deterministic),
                )
                    .chain(),
            )
            .add_systems(
                RunFixedMainLoop,
                interpolate_movable_transforms.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
            );
    }
}
