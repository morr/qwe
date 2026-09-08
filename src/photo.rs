//! Фотографический проход: то, что отличает **снимок** города от его
//! безупречно чистого рендера.
//!
//! Всё, что до сих пор делалось с картой, — материалы, тени, наклон, палитра —
//! про то, **что** в кадре. Этот проход про то, **чем** кадр снят: у всякой
//! настоящей фотографии есть зерно сенсора, дымка между камерой и городом,
//! расхождение каналов к краям объектива, ореол шарпинга (спутниковый кадр
//! почти всегда пан-шарпен) и S-образная кривая контраста. По отдельности
//! ничего из этого не видно; вместе они и есть «похоже на фотографию».
//!
//! Проход полноэкранный, идёт **после bloom и до тонмаппинга** (тот выключен,
//! `post.rs`), читает результат основного прохода и пишет в него же через
//! `ViewTarget::post_process_write`. Сила всего разом — ползунок
//! [`PhotoStyle::amount`] секции Photo; ноль возвращает прежний кадр
//! в точности, потому что при нулевой силе шейдер отдаёт исходный отсчёт.
//!
//! Если пайплайн почему-либо не собрался (ошибка компиляции шейдера, чужой
//! формат цели), система выходит **до** `post_process_write`, и кадр
//! рисуется без эффекта, а не чёрным.

use bevy::camera::Hdr;
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core2d, Core2dSystems};
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, Operations,
    PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, TextureFormat,
    TextureSampleType, TextureViewId,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::Shader;

use crate::prefs::TrackPrefExt;
use crate::settings::PHOTO_AMOUNT_DEFAULT;

const SHADER_PATH: &str = "shaders/photo.wgsl";

/// Амплитуды эффектов при силе 1.0. Подобраны так, чтобы каждый по
/// отдельности был на грани заметности: фотография узнаётся не по силе
/// приёмов, а по тому, что они есть все сразу.
///
/// Зерно — множитель яркости; 3 % это шум сенсора на среднем ISO. Оно
/// **неподвижно в экранных координатах**, как и положено сенсору: карта под
/// ним едет, зерно стоит.
const GRAIN: f32 = 0.03;
/// Дымка поднимает только тени и только к холодному: между камерой и городом
/// километр воздуха, и он подсвечен небом.
const HAZE: f32 = 0.05;
/// Расхождение каналов у края кадра, доли ширины экрана. Растёт как квадрат
/// расстояния от центра — в центре объектив чист.
const ABERRATION: f32 = 0.0016;
/// Нерезкое маскирование: ореол, которым выдаёт себя пан-шарпен спутникового
/// снимка. Больше 0.5 — и появляется каёмка вокруг каждой крыши.
const SHARPEN: f32 = 0.35;
/// S-образная кривая контраста, доля подмешивания `smoothstep`.
const CONTRAST: f32 = 0.12;

/// Сила фотографического прохода; ползунок Photo и BRP, сохраняется между
/// запусками. Ноль — прежний чистый кадр.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "photo")]
pub struct PhotoStyle {
    pub amount: f32,
}

impl Default for PhotoStyle {
    fn default() -> Self {
        Self {
            amount: PHOTO_AMOUNT_DEFAULT,
        }
    }
}

/// Параметры прохода на камере — они же юниформ шейдера. Компонент, а не
/// ресурс, потому что проход находит вид именно по нему.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
pub struct PhotoSettings {
    grain: f32,
    haze: f32,
    aberration: f32,
    sharpen: f32,
    contrast: f32,
    /// Общая сила: ноль — шейдер отдаёт исходный отсчёт и выходит.
    amount: f32,
}

impl PhotoSettings {
    fn of(style: PhotoStyle) -> Self {
        Self {
            grain: GRAIN,
            haze: HAZE,
            aberration: ABERRATION,
            sharpen: SHARPEN,
            contrast: CONTRAST,
            amount: style.amount,
        }
    }
}

impl Default for PhotoSettings {
    fn default() -> Self {
        Self::of(PhotoStyle::default())
    }
}

pub struct PhotoPlugin;

impl Plugin for PhotoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhotoStyle>()
            .register_type::<PhotoStyle>()
            .track_pref::<PhotoStyle>()
            .add_plugins((
                ExtractComponentPlugin::<PhotoSettings>::default(),
                UniformComponentPlugin::<PhotoSettings>::default(),
            ))
            .add_systems(Update, retune_photo);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_photo_pipeline)
            .add_systems(
                Core2d,
                photo_pass
                    // после bloom (свечение — часть картинки, зерно ложится
                    // поверх него) и до тонмаппинга, который у нас выключен
                    .after(bevy::post_process::bloom::bloom)
                    .in_set(Core2dSystems::PostProcess),
            );
    }
}

/// Ползунок панели — в компонент камеры; юниформ его подхватит сам.
fn retune_photo(style: Res<PhotoStyle>, mut settings: Query<&mut PhotoSettings>) {
    if !style.is_changed() {
        return;
    }
    for mut camera in &mut settings {
        *camera = PhotoSettings::of(*style);
    }
}

#[derive(Resource)]
struct PhotoPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

fn init_photo_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "photo_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PhotoSettings>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let shader: Handle<Shader> = asset_server.load(SHADER_PATH);
    let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("photo_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader,
            targets: vec![Some(ColorTargetState {
                // камера всегда HDR (`post::camera_post_process` вешает
                // `Hdr`), и цель прохода — та же HDR-текстура. Формат задан
                // прямо: пайплайн собирается один раз, до всякого вида, а
                // специализация по виду ради единственной камеры — лишняя
                // машинерия. `photo_pass` на не-HDR виде просто не рисует.
                format: TextureFormat::Rgba16Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(PhotoPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Default)]
struct PhotoBindGroupCache {
    cached: Option<(TextureViewId, BindGroup)>,
}

fn photo_pass(
    view: ViewQuery<(
        &ViewTarget,
        &PhotoSettings,
        &DynamicUniformIndex<PhotoSettings>,
        Has<Hdr>,
    )>,
    photo_pipeline: Option<Res<PhotoPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<PhotoSettings>>,
    mut cache: Local<PhotoBindGroupCache>,
    mut ctx: RenderContext,
) {
    let Some(photo_pipeline) = photo_pipeline else {
        return;
    };
    let (view_target, settings, settings_index, hdr) = view.into_inner();
    // цель прохода описана HDR-форматом; на не-HDR камере он не заработал бы,
    // и лучше не рисовать эффект, чем писать в чужой формат
    if !hdr || settings.amount <= 0.0 {
        return;
    }
    let Some(pipeline) = pipeline_cache.get_render_pipeline(photo_pipeline.pipeline_id) else {
        return;
    };
    let Some(settings_binding) = uniforms.uniforms().binding() else {
        return;
    };

    // с этого вызова цель уже перевёрнута: писать обязаны мы
    let post_process = view_target.post_process_write();
    let bind_group = match &mut cache.cached {
        Some((texture_id, bind_group)) if post_process.source.id() == *texture_id => bind_group,
        cached => {
            let bind_group = ctx.render_device().create_bind_group(
                "photo_bind_group",
                &pipeline_cache.get_bind_group_layout(&photo_pipeline.layout),
                &BindGroupEntries::sequential((
                    post_process.source,
                    &photo_pipeline.sampler,
                    settings_binding.clone(),
                )),
            );
            let (_, bind_group) = cached.insert((post_process.source.id(), bind_group));
            bind_group
        }
    };

    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("photo_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    render_pass.set_pipeline(pipeline);
    render_pass.set_bind_group(0, bind_group, &[settings_index.index()]);
    render_pass.draw(0..3, 0..1);
}
