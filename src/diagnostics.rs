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

/// Сколько ответов поиска снято за кадр и сколько из них без пути. Сеточный
/// поиск промахивается редко (цель заранее просеяна `find_passable_tile_near`),
/// а полигональный — всякий раз, когда цель или сама пешка оказались внутри
/// препятствия, раздутого радиусом агента. Отказ означает `PathfindingError`,
/// то есть стоящую пешку, так что цена выбранной семантики должна быть видна
/// числом, а не на глаз.
///
/// Записываются именно два счётчика, а не готовая доля: доля, посчитанная в
/// кадре и усреднённая по кадрам, считает кадры, а не ответы — кадр с одним
/// ответом (и потому ровно 0 % или ровно 100 %) весит в среднем столько же,
/// сколько кадр с сотней. И, хуже того, замер писался только в кадрах с
/// ответами: стоило потоку иссякнуть — пауза, зум за `WANDER_DISPATCH_MAX_ZOOM`,
/// упёршийся в лимит полимеш — как история переставала обновляться и на панели
/// навсегда оставалось последнее значение. Отношение средних по двум историям
/// одинаковой длины (обе пишутся каждый кадр, в том числе нулями) — это отказы
/// на ответы за окно истории, и оно сходит к нулю, когда ответов нет.
pub const PATHFINDING_ANSWERED: DiagnosticPath = DiagnosticPath::const_new("pathfinding/answered");
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
/// Расталкивание меряется на **прогон**, а не на тик: оно гейтится «не чаще
/// раза в кадр» (`movement/separation.rs`), так что прогонов ~60 в секунду
/// против ~64 × time_scale тиков у остальных `sim/*_ms`.
pub const SIM_SEPARATION_MS: DiagnosticPath = DiagnosticPath::const_new("sim/separation_ms");

/// Сглаженная цена **всего** тика — то, чем регулятор скорости объясняет своё
/// решение (`sim_time::SimLoad`). Сумма `sim/*_ms` выше её не заменяет: она
/// разрезана по системам и покрывает не весь `FixedUpdate`, а посильная
/// скорость считается ровно из этого числа.
pub const SIM_TICK_MS: DiagnosticPath = DiagnosticPath::const_new("sim/tick_ms");

/// Сколько из тика главный поток простоял, ожидая чужую работу (`block_on` над
/// поиском пути). Учитывается **отдельно** от `sim/tick_ms`: работа от скорости
/// не зависит, а ожидание зависит прямо — срок ответа отмерен в тиках, значит
/// на быстрых тиках пулу достаётся меньше реального времени.
pub const SIM_WAIT_MS: DiagnosticPath = DiagnosticPath::const_new("sim/wait_ms");

/// Пиковое ожидание на шаг (`sim_time::SimLoad::wait_peak_ms`) — то, по чему
/// на самом деле держится потолок конвейера: по среднему `sim/wait_ms` всплеск
/// размывается раньше, чем регулятор успевает ответить.
pub const SIM_WAIT_PEAK_MS: DiagnosticPath = DiagnosticPath::const_new("sim/wait_peak_ms");

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
        .register_diagnostic(Diagnostic::new(PATHFINDING_ANSWERED).with_suffix(" answers"))
        .register_diagnostic(Diagnostic::new(PATHFINDING_FAILED).with_suffix(" failures"))
        .register_diagnostic(Diagnostic::new(SIM_SPATIAL_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_PANIC_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_FLEE_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_CHASE_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_MOVE_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_SEPARATION_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_TICK_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_WAIT_MS).with_suffix(" ms"))
        .register_diagnostic(Diagnostic::new(SIM_WAIT_PEAK_MS).with_suffix(" ms"))
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
