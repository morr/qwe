//! GPU-разрез кадра: wgpu timestamp queries по границам сегментов 2d-кадра.
//!
//! `QWE_PERF_GPU_CUT=1` (см. `dev.rs::render_plugin` и
//! `add_perf_toggles`): у устройства запрашивается фича
//! `TIMESTAMP_QUERY` (Metal поддерживает), в расписание `Core2d` — его
//! `camera_driver` прогоняет на каждую камеру — встают маркер-системы.
//! Каждая открывает **render pass на 1×1-текстуру с фиктивным draw** и
//! `timestamp_writes` в дескрипторе: метка пишется на начале пасса —
//! энкодеры систем уходят в сабмит в топологическом порядке систем
//! (`RenderContextState: SystemBuffer`), GPU исполняет энкодеры в
//! порядке сабмита, и начало пустого пасса = кумулятивная GPU-граница
//! между сегментами. Дельта соседних границ = GPU-длительность
//! сегмента.
//!
//! Почему не другие пути: `RenderPass::write_timestamp` внутри пасса
//! требует `TIMESTAMP_QUERY_INSIDE_PASSES` — Metal его не даёт
//! (RequestDeviceError), из-за чего встроенный
//! `RenderDiagnosticsPlugin` 0.20 на Metal CPU-only (док «only Vulkan
//! and DX12» точен); `write_timestamp` на CommandEncoder bevy глушит
//! на macOS сам (фликер на Tahoe, bevy#22257). И даже пассовый путь
//! без работы не пишет метки: Metal молча возвращает нули из энкодера
//! без операций (известный баг «blit encoders won't write timestamps
//! if they don't have a blit operation», wgpu-hal metal comment;
//! воркараунд Dawn — фиктивная работа в энкодере). Поэтому маркер —
//! **render pass на 1×1-текстуру с одним фиктивным draw** (три
//! вершины, закрашивают один пиксель): энкодер с работой метки пишет.
//!
//! Метки 0..=5: 0 до opaque-пасса, 1 после opaque (до transparent),
//! 2 после transparent, 3 после PostProcess (bloom + tonemapping), 4
//! после `ui_pass` (до upscaling), 5 после upscaling. Сегменты —
//! дельты соседних меток; msaa_writeback и очистка цели отдельной
//! метки не имеют (между 0 и 1 — попадают в «opaque»).
//!
//! Данные возвращаются `map_async`-ом в main-мир: resolve-система
//! переписывает шесть меток в ring-буфер из четырёх (map до сабмита
//! wgpu запрещает — «buffer is still mapped» на submit, поэтому слот
//! мапится системой после `Submit`), poll-система раз в кадр гонит
//! колбэки, drain читает и размапливает; `Update`-принтер печатает
//! `gpu cut:` раз в секунду (среднее сегментов + зум для
//! сопоставления), а кадр выше порога `QWE_PERF_GPU_SPIKES=<ms>` —
//! пофреймовый `gpu spike:`. Офскрин-камера (`dev.rs`, кадр в текстуру)
//! меток не ставит — её кадр с оконным не сравнивают.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc;
use std::sync::Mutex;

use bevy::camera::NormalizedRenderTarget;
use bevy::camera_controller::pan_camera::PanCamera;
use bevy::core_pipeline::core_2d::{main_opaque_pass_2d, main_transparent_pass_2d};
use bevy::core_pipeline::{Core2d, Core2dSystems};
use bevy::core_pipeline::upscaling::upscaling;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderStartup;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::{
    BufferDescriptor, BufferUsages, LoadOp, MapMode, Operations, PollType,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
};
use bevy::render::renderer::{
    RenderContext, RenderDevice, RenderGraphSystems, RenderQueue, ViewQuery,
};
use bevy::ui_render::ui_pass;
use wgpu::{
    QuerySet, QuerySetDescriptor, QueryType, RenderPassTimestampWrites, RenderPipeline,
    ShaderModuleDescriptor, ShaderSource,
};

/// Меток GPU-границ в кадре (и слотов query set / буфера).
const SLOTS: usize = 6;

/// Имена сегментов между соседними метками, порядок = порядок строк.
const SEGMENTS: [&str; SLOTS - 1] = ["opaque", "transparent", "post", "ui", "upscale"];

/// Последняя решённая рамка меток: кумулятивные тики GPU-часов.
static TS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];

/// Сколько рамок решено (дельта = новый кадр в статиках).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Период тика GPU-часов в мс, биты [`f32`]; ноль — ещё не снят.
static PERIOD_NS: AtomicU64 = AtomicU64::new(0);

// TEMP perf44 cut — счётчики звеньев readback-цепочки для probe-строки
// (гейт QWE_PERF_GPU_CUT_PROBES, не коммитить).
static RESOLVES: AtomicU64 = AtomicU64::new(0);
static MAPS: AtomicU64 = AtomicU64::new(0);

fn probes_enabled() -> bool {
    static PROBES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROBES.get_or_init(|| std::env::var_os("QWE_PERF_GPU_CUT_PROBES").is_some())
}

/// Query set, маркерная цель и пайплайн, ring readback-буферов; `tx` —
/// канал колбэков `map_async`. Жизненный цикл буфера: resolve (до
/// сабмита) → map_async (после сабмита — mapped до сабмита wgpu
/// запрещает) → колбэк → drain читает и размапливает. Ринг из четырёх:
/// колбэк доезжает за 1–2 кадра, к четвёртому буфер свободен; если GPU
/// встал и буфер ещё mapped — кадр решается без метки (сэмпл
/// пропускается), не валидация.
#[derive(Resource)]
struct GpuCutRes {
    query_set: QuerySet,
    pipeline: RenderPipeline,
    target_view: wgpu::TextureView,
    buffers: [wgpu::Buffer; 4],
    mapped: [bool; 4],
    cur: usize,
    pending: VecDeque<usize>,
    tx: mpsc::Sender<(usize, wgpu::Buffer)>,
}

/// Приёмник решённых буферов; наполняется колбэками во время poll.
/// `Mutex` — Resource требует Sync, std-канал его не даёт.
#[derive(Resource)]
struct GpuCutRx(Mutex<mpsc::Receiver<(usize, wgpu::Buffer)>>);

/// Порог `gpu spike`, мс GPU на кадр; absent — печатаются только секундные.
#[derive(Resource)]
pub struct GpuSpikeThreshold(f32);

/// Порог `QWE_PERF_GPU_SPIKES=<ms>`, ресурс принтера.
pub struct GpuCutPlugin {
    pub spike_threshold: Option<f32>,
}

impl Plugin for GpuCutPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, setup)
            .add_systems(
                Core2d,
                (
                    mark::<0>.before(main_opaque_pass_2d),
                    mark::<1>
                        .after(main_opaque_pass_2d)
                        .before(main_transparent_pass_2d),
                    mark::<2>
                        .after(main_transparent_pass_2d)
                        .before(Core2dSystems::PostProcess),
                    mark::<3>.after(Core2dSystems::PostProcess).before(ui_pass),
                    mark::<4>.after(ui_pass).before(upscaling),
                    mark::<5>.after(upscaling),
                    resolve.after(mark::<5>),
                ),
            )
            .add_systems(
                RenderGraph,
                (
                    map_pending.after(RenderGraphSystems::Submit),
                    poll_device.after(map_pending),
                    drain.after(poll_device),
                ),
            );
        if let Some(ms) = self.spike_threshold {
            app.insert_resource(GpuSpikeThreshold(ms));
        }
        app.add_systems(Update, print_cut);
    }
}

/// Размер одного readback-буфера: шесть тиков с выравниванием 256
/// (wgpu: offset resolve-копии должен делиться на 256).
fn size_bytes() -> u64 {
    (SLOTS as u64 * 8).next_multiple_of(256)
}

/// Шейдер маркерного draw: треугольник на весь target (1×1) и фрагмент
/// с заливкой — работа, без которой Metal метки в энкодере не пишет.
const MARKER_WGSL: &str = "
@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 3.0), vec2(3.0, -1.0));
    return vec4<f32>(p[vi], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
";

/// Query set, маркерная цель с пайплайном и readback-буферы; период
/// тика GPU-часов — в статику.
fn setup(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let wgpu_device = device.wgpu_device();
    let query_set = wgpu_device.create_query_set(&QuerySetDescriptor {
        label: Some("gpu_cut_timestamps"),
        ty: QueryType::Timestamp,
        count: SLOTS as u32,
    });
    let module = wgpu_device.create_shader_module(ShaderModuleDescriptor {
        label: Some("gpu_cut_marker_shader"),
        source: ShaderSource::Wgsl(MARKER_WGSL.into()),
    });
    let pipeline = wgpu_device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gpu_cut_marker_pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let target = wgpu_device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu_cut_marker_target"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let make_buffer = |n: &str| {
        wgpu_device.create_buffer(&BufferDescriptor {
            label: Some(n),
            size: size_bytes(),
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    };
    PERIOD_NS.store(f32::to_bits(queue.get_timestamp_period()) as u64, Relaxed);
    // TEMP perf44 cut — плагин встал, ресурсы созданы (не коммитить)
    info!(
        "gpu cut probe: setup ok, period {:.4} ns/tick",
        queue.get_timestamp_period()
    );
    let (tx, rx) = mpsc::channel();
    commands.insert_resource(GpuCutRes {
        query_set,
        pipeline,
        target_view,
        buffers: [
            make_buffer("gpu_cut_readback_0"),
            make_buffer("gpu_cut_readback_1"),
            make_buffer("gpu_cut_readback_2"),
            make_buffer("gpu_cut_readback_3"),
        ],
        mapped: [false; 4],
        cur: 0,
        pending: VecDeque::new(),
        tx,
    });
    commands.insert_resource(GpuCutRx(Mutex::new(rx)));
}

/// Одна GPU-граница: render pass на 1×1-текстуру с меткой на начале и
/// одним фиктивным draw (без него Metal метки в энкодере не пишет).
/// Только для камеры в окно — офскрин-кадр меток не ставит.
fn mark<const I: usize>(
    mut ctx: RenderContext,
    res: Res<GpuCutRes>,
    camera: ViewQuery<&ExtractedCamera>,
) {
    if !matches!(
        camera.into_inner().target,
        Some(NormalizedRenderTarget::Window(_))
    ) {
        return;
    }
    let color = [Some(RenderPassColorAttachment {
        view: &res.target_view,
        resolve_target: None,
        ops: Operations {
            load: LoadOp::Load,
            store: StoreOp::Discard,
        },
        depth_slice: None,
    })];
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("gpu_cut_marker"),
        color_attachments: &color,
        depth_stencil_attachment: None,
        timestamp_writes: Some(RenderPassTimestampWrites {
            query_set: &res.query_set,
            beginning_of_pass_write_index: Some(I as u32),
            end_of_pass_write_index: None,
        }),
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let raw = pass.wgpu_pass();
    raw.set_pipeline(&res.pipeline);
    raw.draw(0..3, 0..1);
}

/// Копия меток кадра в ring-буфер; map-ить до сабмита нельзя — слот
/// встаёт в очередь и мапится `map_pending` уже после сабмита. Если слот
/// предыдущего оборота ещё mapped (GPU не доехал) — кадр без сэмпла.
fn resolve(mut ctx: RenderContext, mut res: ResMut<GpuCutRes>) {
    let cur = res.cur;
    res.cur = (res.cur + 1) % res.buffers.len();
    if res.mapped[cur] {
        return;
    }
    ctx.command_encoder()
        .resolve_query_set(&res.query_set, 0..SLOTS as u32, &res.buffers[cur], 0);
    res.pending.push_back(cur);
    RESOLVES.fetch_add(1, Relaxed); // TEMP perf44 cut
}

/// После сабмита: map_async на решённые в этом кадре буферы.
fn map_pending(mut res: ResMut<GpuCutRes>) {
    let slots: Vec<usize> = res.pending.drain(..).collect();
    for slot in slots {
        let (tx, cb) = (res.tx.clone(), res.buffers[slot].clone());
        res.buffers[slot].map_async(MapMode::Read, 0..size_bytes(), move |result| {
            match result {
                Ok(()) => {
                    let _ = tx.send((slot, cb));
                }
                Err(err) => warn!("gpu cut: readback map failed ({err:?}), buffer dropped"),
            }
        });
        res.mapped[slot] = true;
        MAPS.fetch_add(1, Relaxed); // TEMP perf44 cut
    }
}

/// Колбэки `map_async` исполняются только под poll — раз в кадр, после
/// сабмита. `PollType::Poll` не ждёт GPU: незавершённые доедут позже.
fn poll_device(device: Res<RenderDevice>) {
    let _ = device.poll(PollType::Poll);
}

/// Решённые буферы → статики (кумулятивные тики + номер рамки);
/// буфер размапливается — слот вернётся в оборот через оборот ринга.
fn drain(rx: Res<GpuCutRx>, mut res: ResMut<GpuCutRes>) {
    let Ok(rx) = rx.0.lock() else {
        return;
    };
    while let Ok((slot, buffer)) = rx.try_recv() {
        // колбэк map-ит буфер до отправки — ошибка здесь не бывает
        let view = buffer
            .get_mapped_range(0..size_bytes())
            .expect("gpu cut readback mapped");
        for (i, ticks) in view.chunks_exact(8).take(SLOTS).enumerate() {
            TS[i].store(u64::from_le_bytes(ticks.try_into().unwrap()), Relaxed);
        }
        drop(view);
        buffer.unmap();
        res.mapped[slot] = false;
        SEQ.fetch_add(1, Relaxed);
    }
    // TEMP perf44 cut — probe-строка: где цепочка readback рвётся
    // (гейт QWE_PERF_GPU_CUT_PROBES, не коммитить)
    if probes_enabled() {
        static PROBE_FRAME: AtomicU64 = AtomicU64::new(0);
        if PROBE_FRAME.fetch_add(1, Relaxed).is_multiple_of(300) {
            info!(
                "gpu cut probe: resolves {} maps {} drained {} seq {} period {:.4} ts {:?}",
                RESOLVES.load(Relaxed),
                MAPS.load(Relaxed),
                SEQ.load(Relaxed),
                SEQ.load(Relaxed),
                f32::from_bits(PERIOD_NS.load(Relaxed) as u32),
                std::array::from_fn::<_, SLOTS, _>(|i| TS[i].load(Relaxed)),
            );
        }
    }
}

/// Накопитель сегментов между печатями (секундное окно ≈ 60 кадров).
#[derive(Default)]
struct CutSums {
    last_seq: u64,
    frames: u32,
    segs: [f64; SLOTS - 1],
    total: f64,
}

/// main-мир: каждый решённый кадр ложится в накопитель; раз в ~60 кадров —
/// `gpu cut:` (средние сегментов + зум для сопоставления), кадр выше
/// порога — пофреймовый `gpu spike:`.
fn print_cut(
    camera: Single<&PanCamera, With<Camera2d>>,
    threshold: Option<Res<GpuSpikeThreshold>>,
    mut sums: Local<CutSums>,
) {
    let seq = SEQ.load(Relaxed);
    if seq == sums.last_seq {
        return;
    }
    sums.last_seq = seq;
    // период тика в наносекундах (wgpu: ticks × period = ns)
    let period_ns = f64::from(f32::from_bits(PERIOD_NS.load(Relaxed) as u32));
    if period_ns <= 0.0 {
        return;
    }
    let ts: [u64; SLOTS] = std::array::from_fn(|i| TS[i].load(Relaxed));
    if ts[SLOTS - 1] <= ts[0] {
        return;
    }
    let seg_ms = |a: usize, b: usize| {
        (ts[b] as i64 - ts[a] as i64).max(0) as f64 * period_ns / 1_000_000.0
    };
    let total_ms = (ts[SLOTS - 1] - ts[0]) as f64 * period_ns / 1_000_000.0;
    if let Some(threshold) = threshold
        && total_ms >= threshold.0 as f64
    {
        let segs: Vec<String> = (0..SLOTS - 1)
            .map(|i| format!("{} {:.2}", SEGMENTS[i], seg_ms(i, i + 1)))
            .collect();
        info!(
            "gpu spike: {total_ms:.2} ms gpu ({}) zoom {:.2} seq {seq}",
            segs.join(" "),
            camera.zoom_factor,
        );
    }
    for (sum, i) in sums.segs.iter_mut().zip(0..SLOTS - 1) {
        *sum += seg_ms(i, i + 1);
    }
    sums.total += total_ms;
    sums.frames += 1;
    if sums.frames >= 60 {
        let segs: String = sums
            .segs
            .iter()
            .zip(SEGMENTS)
            .map(|(s, n)| format!("{n} {:.2}", s / f64::from(sums.frames)))
            .collect::<Vec<_>>()
            .join(" | ");
        info!(
            "gpu cut: total {:.2} | {segs} ms (avg over {} frames, zoom {:.2})",
            sums.total / f64::from(sums.frames),
            sums.frames,
            camera.zoom_factor,
        );
        let last = sums.last_seq;
        *sums = CutSums {
            last_seq: last,
            ..Default::default()
        };
    }
}
