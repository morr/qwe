//! Диагностика pathfinding (порт `zxc/src/diagnostics.rs`): сколько тасков в
//! полёте и сколько миллисекунд занимает поиск пути.

use bevy::diagnostic::{
    Diagnostic, DiagnosticPath, Diagnostics, EntityCountDiagnosticsPlugin, RegisterDiagnostic,
};
use bevy::prelude::*;

use crate::movement::{PathfindingRequest, PathfindingTask};

/// Сколько pathfinding-тасков сейчас в полёте. Значение производное — считается
/// по живым компонентам, а не счётчиком, который надо синхронно уменьшать
/// внутри самой future.
pub const PATHFINDING_IN_FLIGHT: DiagnosticPath =
    DiagnosticPath::const_new("pathfinding/in_flight");

/// Сколько запросов ждёт в очереди диспетчера (ещё не превратились в таски).
pub const PATHFINDING_QUEUED: DiagnosticPath = DiagnosticPath::const_new("pathfinding/queued");

/// Сколько миллисекунд занял поиск пути (замер внутри async-блока).
pub const PATHFINDING_DURATION_MS: DiagnosticPath =
    DiagnosticPath::const_new("pathfinding/duration_ms");

/// Доля снятых ответов без пути, проценты. Сеточный поиск промахивается
/// редко (цель заранее просеяна `find_passable_tile_near`), а полигональный —
/// всякий раз, когда цель или сама пешка оказались внутри препятствия,
/// раздутого радиусом агента. Отказ означает `PathfindingError`, то есть
/// стоящую пешку, так что цена выбранной семантики должна быть видна числом,
/// а не на глаз.
pub const PATHFINDING_FAILED: DiagnosticPath = DiagnosticPath::const_new("pathfinding/failed");

/// Длительность систем симуляции, мс на один тик `FixedUpdate`. На высоких
/// скоростях тиков в секунду становится в разы больше (64 × time_scale), и
/// главный поток упирается именно в эту сумму — без разреза по системам
/// «тормозит на 15x» не диагностируется.
pub const SIM_SPATIAL_MS: DiagnosticPath = DiagnosticPath::const_new("sim/spatial_ms");
pub const SIM_PANIC_MS: DiagnosticPath = DiagnosticPath::const_new("sim/panic_ms");
pub const SIM_FLEE_MS: DiagnosticPath = DiagnosticPath::const_new("sim/flee_ms");
pub const SIM_CHASE_MS: DiagnosticPath = DiagnosticPath::const_new("sim/chase_ms");
pub const SIM_MOVE_MS: DiagnosticPath = DiagnosticPath::const_new("sim/move_ms");

/// Записать длительность системы, начавшейся в `started`.
pub fn measure_ms(
    diagnostics: &mut Diagnostics,
    path: &DiagnosticPath,
    started: std::time::Instant,
) {
    diagnostics.add_measurement(path, || started.elapsed().as_secs_f64() * 1000.0);
}

pub struct GameDiagnosticsPlugin;

impl Plugin for GameDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.register_diagnostic(
            Diagnostic::new(PATHFINDING_IN_FLIGHT)
                .with_suffix(" tasks")
                .with_max_history_length(1),
        )
        .register_diagnostic(
            Diagnostic::new(PATHFINDING_QUEUED)
                .with_suffix(" requests")
                .with_max_history_length(1),
        )
        .register_diagnostic(Diagnostic::new(PATHFINDING_DURATION_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(PATHFINDING_FAILED).with_suffix(" %"))
        .register_diagnostic(Diagnostic::new(SIM_SPATIAL_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_PANIC_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_FLEE_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_CHASE_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_MOVE_MS).with_suffix(" ms"))
        .add_plugins(EntityCountDiagnosticsPlugin::default())
        .add_systems(Update, measure_pathfinding_in_flight);
    }
}

fn measure_pathfinding_in_flight(
    mut diagnostics: Diagnostics,
    tasks: Query<&PathfindingTask>,
    requests: Query<&PathfindingRequest>,
) {
    let in_flight = tasks.iter().count();
    diagnostics.add_measurement(&PATHFINDING_IN_FLIGHT, || in_flight as f64);
    let queued = requests.iter().count();
    diagnostics.add_measurement(&PATHFINDING_QUEUED, || queued as f64);
}
