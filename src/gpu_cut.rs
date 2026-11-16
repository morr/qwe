//! GPU-разрез кадра: GPU-время пассов рендера через встроенную
//! диагностику Bevy 0.20.
//!
//! `QWE_PERF_GPU_CUT=1` (см. `dev.rs::render_plugin` и
//! `add_perf_toggles`): у устройства запрашиваются фичи
//! `TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_PASSES` (Metal
//! поддерживает обе) и поднимается `RenderDiagnosticsPlugin`. Его
//! `DiagnosticsRecorder` на Metal не работает — так считает его
//! док-коммент («only Vulkan and DX12»), но код гейтится только
//! фичей: query set создаётся при `features.contains(TIMESTAMP_QUERY)`,
//! метки пишутся на границах пассов (`RenderPass::write_timestamp`,
//! не энкодерный путь — тот на macOS заглушен вверхстримом из-за
//! фликера, bevy#22257), readback и синк в `DiagnosticsStore` свои.
//! Результат: пути `render/<пасс>/elapsed_gpu` в диагностике — и их
//! уже читает этот модуль. Если док прав и GPU-значений на Metal нет,
//! строки будут пустыми — тогда обратный путь, свои timestamp-пассы
//! (спека 42, план Б).
//!
//! Принтер в main-мире: раз в секунду строка `gpu cut:` со значениями
//! `elapsed_gpu` всех пассов кадра (latest measurement каждого пути) и
//! зумом камеры для сопоставления; кадр с суммой выше порога
//! `QWE_PERF_GPU_SPIKES=<ms>` — пофреймовый `gpu spike:`.
//!
//! Чего в разрезе нет: msaa_writeback и очистка цели span-ов не
//! ставят (live между пассами — попадает в «total минус сумма»).

use bevy::camera_controller::pan_camera::PanCamera;
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;

/// Порог `gpu spike`, мс суммы GPU-пассов за кадр; absent — только
/// секундные строки.
#[derive(Resource)]
pub struct GpuSpikeThreshold(f32);

/// Порог `QWE_PERF_GPU_SPIKES=<ms>`, ресурс принтера.
pub struct GpuCutPlugin {
    pub spike_threshold: Option<f32>,
}

impl Plugin for GpuCutPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        if let Some(ms) = self.spike_threshold {
            app.insert_resource(GpuSpikeThreshold(ms));
        }
        app.add_systems(Update, print_gpu_cut);
    }
}

/// Одно значение `elapsed_gpu`: короткое имя пасса и мс последнего
/// замера.
struct GpuPass {
    name: String,
    ms: f64,
}

/// Значения `render/*/elapsed_gpu` на текущий кадр, сумма по всем.
fn gpu_passes(store: &DiagnosticsStore) -> (Vec<GpuPass>, f64) {
    let mut passes = Vec::new();
    for diagnostic in store.iter() {
        let path = diagnostic.path();
        let name = path.to_string();
        let Some(short) = name.strip_suffix("/elapsed_gpu") else {
            continue;
        };
        let short = short.strip_prefix("render/").unwrap_or(short);
        if let Some(m) = store.get_measurement(path) {
            passes.push(GpuPass {
                name: short.to_string(),
                ms: m.value,
            });
        }
    }
    let total = passes.iter().map(|p| p.ms).sum();
    (passes, total)
}

fn join_passes(passes: &[GpuPass]) -> String {
    passes
        .iter()
        .map(|p| format!("{} {:.2}", p.name, p.ms))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// main-мир: раз в секунду `gpu cut:`, кадр выше порога — `gpu spike:`.
fn print_gpu_cut(
    time: Res<Time<Real>>,
    store: Res<DiagnosticsStore>,
    camera: Single<&PanCamera, With<Camera2d>>,
    threshold: Option<Res<GpuSpikeThreshold>>,
    mut secs: Local<f32>,
) {
    let (passes, total) = gpu_passes(&store);
    if passes.is_empty() {
        return;
    }
    if let Some(threshold) = threshold
        && total >= threshold.0 as f64
    {
        info!(
            "gpu spike: {total:.2} ms gpu ({} zoom {:.2})",
            join_passes(&passes),
            camera.zoom_factor,
        );
    }
    *secs += time.delta_secs();
    if *secs >= 1.0 {
        *secs -= 1.0;
        info!(
            "gpu cut: total {total:.2} | {} ms (zoom {:.2})",
            join_passes(&passes),
            camera.zoom_factor,
        );
    }
}
