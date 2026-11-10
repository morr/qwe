//! Стены фаз кадра — диагностика длинных кадров: `QWE_PERF_PHASE_SPIKES=<ms>`.
//!
//! `frame spike` (dev.rs) показывает только длину кадра; эта диагностика
//! добавляет разрез длинного кадра на фазы. Строки печатаются только для
//! кадров длиннее порога — на шторме входа в мир и на зум-лагах; без
//! переменной ничего не меняется.
//!
//! Три строки на длинный кадр:
//!
//! - `main phases` — стены сегментов главного расписания: `pre` (First +
//!   PreUpdate), `trans` (StateTransition — здесь сидит спавн-флеш входа в
//!   мир), `fixed`, `update` (Update + SpawnScene), `post` (PostUpdate) и
//!   `inter` (Last прошлого кадра → First текущего — extract мира и события
//!   окна: sync/extract выполняются на main-потоке после Last,
//!   `bevy_extract`).
//! - `sprite scans` — скобки вокруг двух конвейеров материалов спрайтов
//!   (`check_entities_needing_specialization`,
//!   `mark_2d_meshes_as_changed_if_their_materials_changed`, PostUpdate
//!   main-потока): фильтр `AssetChanged` не архетипный, и когда хоть один
//!   материал типа изменился, скан проходит все сущности с компонентом.
//! - `render phases` — стены фаз render-расписания: `apply` (применение
//!   вытянутых команд — создание сущностей и вставка компонент в
//!   render-мир), `prep_assets`, `specialize`, `queue`, `sort`, `prepare`,
//!   `render` (граф и сабмит). `RenderDiagnosticsPlugin` 0.20 на большинстве
//!   пассов не пишет ничего (`diagnostic_recorder` зовёт только
//!   sparse-buffer-пасс), поэтому стены берутся штампами по границам
//!   `RenderSystems`.
//!
//! Штампы — эксклюзивные системы на границах; в стену сегмента входит и их
//! собственная цена (единицы микросекунд).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bevy::app::RunFixedMainLoopSystems;
use bevy::prelude::*;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssetBytesPerFrameLimiter;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::sprite_render::{
    SpriteMeshMaterial, check_entities_needing_specialization,
    mark_2d_meshes_as_changed_if_their_materials_changed,
};

/// Порог печати, в мс длины кадра.
#[derive(Resource, Clone, Copy)]
struct PhaseThreshold(f32);

/// Границы сегментов main-кадра. Сегмент = следующая граница − текущая;
/// `inter` считается от границы `post` прошлого кадра до головы текущего.
#[derive(Resource, Default)]
struct MainStamps {
    stamps: [Option<Instant>; 6],
    prev_post: Option<Instant>,
    scans: SpriteScans,
}

const MAIN_NAMES: [&str; 6] = ["pre", "trans", "fixed", "update", "post", "inter"];

/// Скобки конвейеров материалов спрайтов: старт держит `Instant`, конец
/// пишет стену в `wall`.
#[derive(Default)]
struct SpriteScans {
    check_start: Option<Instant>,
    mark_start: Option<Instant>,
    check_wall: Duration,
    mark_wall: Duration,
}

/// Границы фаз render-кадра. Сегмент = следующая граница − текущая; пары
/// `pre1/erased` и `pre2/image` — скобки вокруг отдельных систем набора
/// `PrepareAssets` (порядок систем между собой не задан, каждая пара меряет
/// свою систему сама).
#[derive(Resource, Default)]
struct RenderStamps {
    stamps: [Option<Instant>; 15],
}

const RENDER_NAMES: [&str; 14] = [
    "apply",
    "alloc",
    "meshprep",
    "pre1",
    "erased",
    "pre2",
    "image",
    "img2",
    "pre3",
    "specialize",
    "queue",
    "sort",
    "prepare",
    "render",
];

pub struct FramePhasesPlugin;

impl Plugin for FramePhasesPlugin {
    fn build(&self, app: &mut App) {
        let Ok(ms) = std::env::var("QWE_PERF_PHASE_SPIKES") else {
            return;
        };
        let ms: f32 = ms
            .parse()
            .unwrap_or_else(|_| panic!("QWE_PERF_PHASE_SPIKES is not a number: {ms:?}"));
        app.insert_resource(PhaseThreshold(ms))
            .init_resource::<MainStamps>()
            .add_systems(First, main_frame_head)
            .add_systems(
                StateTransition,
                main_trans_head
                    .before(bevy::state::state::StateTransitionSystems::DependentTransitions),
            )
            .add_systems(
                StateTransition,
                main_trans_tail.after(bevy::state::state::StateTransitionSystems::EnterSchedules),
            )
            .add_systems(
                RunFixedMainLoop,
                main_fixed_tail.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
            )
            .add_systems(SpawnScene, main_update_tail)
            .add_systems(Last, main_post_tail)
            .add_systems(
                PostUpdate,
                sprite_check_start
                    .before(check_entities_needing_specialization::<SpriteMeshMaterial>),
            )
            .add_systems(
                PostUpdate,
                sprite_check_end.after(check_entities_needing_specialization::<SpriteMeshMaterial>),
            )
            .add_systems(
                PostUpdate,
                sprite_mark_start.before(
                    mark_2d_meshes_as_changed_if_their_materials_changed::<SpriteMeshMaterial>,
                ),
            )
            .add_systems(
                PostUpdate,
                sprite_mark_end.after(
                    mark_2d_meshes_as_changed_if_their_materials_changed::<SpriteMeshMaterial>,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        if app.world().get_resource::<PhaseThreshold>().is_none() {
            return;
        }
        let ms = app.world().resource::<PhaseThreshold>().0;
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(PhaseThreshold(ms))
            .init_resource::<RenderStamps>()
            .add_systems(
                Render,
                (
                    render_frame_head.before(RenderSystems::ExtractCommands),
                    (|world: &mut World| stamp_render(world, 1))
                        .after(RenderSystems::ExtractCommands),
                    (|world: &mut World| stamp_render(world, 2))
                        .after(bevy::render::mesh::allocator::allocate_and_free_meshes)
                        .before(bevy::render::render_asset::prepare_assets::<RenderMesh>),
                    (|world: &mut World| stamp_render(world, 3))
                        .after(bevy::render::render_asset::prepare_assets::<RenderMesh>),
                    (|world: &mut World| stamp_render(world, 4)).before(
                        bevy::render::erased_render_asset::prepare_erased_assets::<
                            MeshMaterial2d<SpriteMeshMaterial>,
                        >,
                    ),
                    (|world: &mut World| stamp_render(world, 5)).after(
                        bevy::render::erased_render_asset::prepare_erased_assets::<
                            MeshMaterial2d<SpriteMeshMaterial>,
                        >,
                    ),
                    (|world: &mut World| stamp_render(world, 6))
                        .before(bevy::render::render_asset::prepare_assets::<GpuImage>),
                    (|world: &mut World| stamp_render(world, 7))
                        .after(bevy::render::render_asset::prepare_assets::<GpuImage>),
                    (|world: &mut World| stamp_render(world, 8))
                        .after(RenderSystems::PrepareAssets),
                    (|world: &mut World| stamp_render(world, 9)).after(RenderSystems::Specialize),
                    (|world: &mut World| stamp_render(world, 10)).after(RenderSystems::Queue),
                    (|world: &mut World| stamp_render(world, 11)).after(RenderSystems::PhaseSort),
                    (|world: &mut World| stamp_render(world, 12))
                        .after(RenderSystems::PrepareBindGroups),
                    (|world: &mut World| stamp_render(world, 13)).after(RenderSystems::Render),
                    render_frame_tail.after(RenderSystems::PostCleanup),
                ),
            );
    }
}

// --- main-кадр ------------------------------------------------------------

fn stamp_main(world: &mut World, slot: usize) {
    world.resource_mut::<MainStamps>().stamps[slot] = Some(Instant::now());
}

fn main_frame_head(world: &mut World) {
    let threshold = world.resource::<PhaseThreshold>().0;
    let now = Instant::now();
    let stamps = world.resource::<MainStamps>();
    // стена прошлого кадра известна целиком к началу текущего
    let bounds = [
        stamps.stamps[0],
        stamps.stamps[1],
        stamps.stamps[2],
        stamps.stamps[3],
        stamps.stamps[4],
        stamps.stamps[5],
        Some(now),
    ];
    let mut walls = [Duration::ZERO; 6];
    for i in 0..6 {
        if let (Some(a), Some(b)) = (bounds[i], bounds[i + 1]) {
            walls[i] = b - a;
        }
    }
    let frame: Duration = walls[..5].iter().sum();
    let ms = |d: Duration| d.as_secs_f32() * 1000.0;
    let (check_wall, mark_wall) = (stamps.scans.check_wall, stamps.scans.mark_wall);
    let long = walls.iter().any(|w| ms(*w) >= threshold) || ms(frame) >= threshold;
    let assets = if long {
        Some((
            world.resource::<Assets<Mesh>>().len(),
            world.resource::<Assets<ColorMaterial>>().len(),
            world.resource::<Assets<SpriteMeshMaterial>>().len(),
        ))
    } else {
        None
    };
    if long {
        let walls: Vec<String> = walls
            .iter()
            .zip(MAIN_NAMES)
            .map(|(w, n)| format!("{n} {:.1}", ms(*w)))
            .collect();
        match assets {
            Some((meshes, color, sprite)) => info!(
                "main phases: {} (frame {:.1} ms, assets: {} meshes, {} color, {} sprite)",
                walls.join(" "),
                ms(frame),
                meshes,
                color,
                sprite
            ),
            None => info!(
                "main phases: {} (frame {:.1} ms)",
                walls.join(" "),
                ms(frame)
            ),
        }
        if !check_wall.is_zero() || !mark_wall.is_zero() {
            info!(
                "sprite scans: check {:.1} mark {:.1} ms",
                ms(check_wall),
                ms(mark_wall)
            );
        }
    }
    let mut stamps = world.resource_mut::<MainStamps>();
    stamps.scans = SpriteScans::default();
    stamps.prev_post = stamps.stamps[5];
    stamps.stamps = [None; 6];
    stamps.stamps[0] = Some(now);
}

fn main_trans_head(world: &mut World) {
    stamp_main(world, 1);
}

fn main_trans_tail(world: &mut World) {
    stamp_main(world, 2);
}

fn main_fixed_tail(world: &mut World) {
    stamp_main(world, 3);
}

fn main_update_tail(world: &mut World) {
    stamp_main(world, 4);
}

fn main_post_tail(world: &mut World) {
    stamp_main(world, 5);
}

// --- скобки материалов спрайтов (PostUpdate, main-поток) ------------------

fn sprite_check_start(world: &mut World) {
    world.resource_mut::<MainStamps>().scans.check_start = Some(Instant::now());
}

fn sprite_check_end(world: &mut World) {
    let mut stamps = world.resource_mut::<MainStamps>();
    if let Some(start) = stamps.scans.check_start {
        stamps.scans.check_wall += Instant::now() - start;
    }
}

fn sprite_mark_start(world: &mut World) {
    world.resource_mut::<MainStamps>().scans.mark_start = Some(Instant::now());
}

fn sprite_mark_end(world: &mut World) {
    let mut stamps = world.resource_mut::<MainStamps>();
    if let Some(start) = stamps.scans.mark_start {
        stamps.scans.mark_wall += Instant::now() - start;
    }
}

// --- render-кадр ----------------------------------------------------------

fn stamp_render(world: &mut World, slot: usize) {
    world.resource_mut::<RenderStamps>().stamps[slot] = Some(Instant::now());
}

fn render_frame_head(world: &mut World) {
    world.resource_mut::<RenderStamps>().stamps[0] = Some(Instant::now());
}

fn render_frame_tail(world: &mut World) {
    let threshold = world.resource::<PhaseThreshold>().0;
    let now = Instant::now();
    let uploaded = world
        .resource::<RenderAssetBytesPerFrameLimiter>()
        .bytes_written
        .load(Ordering::Relaxed);
    let mut stamps = world.resource_mut::<RenderStamps>();
    let mut bounds = stamps.stamps;
    bounds[14] = Some(now);
    let mut walls = [Duration::ZERO; 14];
    for i in 0..14 {
        if let (Some(a), Some(b)) = (bounds[i], bounds[i + 1]) {
            walls[i] = b - a;
        }
    }
    let ms = |d: Duration| d.as_secs_f32() * 1000.0;
    let total: Duration = walls.iter().sum();
    if walls.iter().any(|w| ms(*w) >= threshold) {
        let walls: Vec<String> = walls
            .iter()
            .zip(RENDER_NAMES)
            .map(|(w, n)| format!("{n} {:.1}", ms(*w)))
            .collect();
        info!(
            "render phases: {} (total {:.1} ms, prepared {:.0} MiB)",
            walls.join(" "),
            ms(total),
            uploaded as f32 / (1024.0 * 1024.0)
        );
    }
    stamps.stamps = [None; 15];
}
