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
