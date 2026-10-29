//! Материал слоя краски: шейдер `paint.wgsl`, юниформы, два прохода колеи
//! и гашение по зуму.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};

use super::{
    APPROACH, AXIS_ZOOM_MAX, DASH, DOUBLE_OFFSET, EDGE_WIDTH, FADE_FROM, GAP, HATCH_PERIOD,
    HATCH_WIDTH, LANE_ZOOM_MAX, LINE_WIDTH, PaintTag, RUT_OFFSET, RUT_SIGMA, RoadPaintStyle,
    YIELD_DASH, YIELD_GAP, ZEBRA_FILL, ZEBRA_PERIOD, ZEBRA_ZOOM_MAX,
};
use crate::map::meshing::ATTRIBUTE_RIBBON;
use crate::map::roads::node_paint::{STOP_WIDTH, ZEBRA_LENGTH};
use crate::map::roads::shape::lane_width;
use crate::map::zoom::{ZoomBucket, ZoomLods};

const SHADER_PATH: &str = "shaders/paint.wgsl";

/// Ступени слоя краски: до [`LANE_ZOOM_MAX`] — всё, до [`ZEBRA_ZOOM_MAX`] —
/// осевые и зебры, до [`AXIS_ZOOM_MAX`] — осевые, дальше — ничего.
pub enum PaintLods {}

impl ZoomLods for PaintLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [LANE_ZOOM_MAX, ZEBRA_ZOOM_MAX, AXIS_ZOOM_MAX, f32::INFINITY].into_iter()
    }
}

pub type PaintZoomBucket = ZoomBucket<PaintLods>;

/// Меш краски виден, пока его линии на этой ступени есть. Пересборки нет: оба
/// меша строятся один раз, ступень только прячет их — так дальний план не
/// гоняет вершины линий, которых не видно.
pub fn show_paint(bucket: Res<PaintZoomBucket>, mut layers: Query<(&PaintTag, &mut Visibility)>) {
    for (tag, mut visibility) in &mut layers {
        let shown = match tag {
            PaintTag::Lanes => bucket.index < 1,
            PaintTag::Zebras => bucket.index < 2,
            // колея траекторий гаснет вместе с осевыми: к порогу шейдер её
            // уже погасил
            PaintTag::Axes | PaintTag::Wear => bucket.index < 3,
        };
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// Юниформ шейдера краски. Зеркало `PaintParams` в `paint.wgsl`: порядок
/// полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct PaintParams {
    /// Непрозрачность краски — [`RoadPaintStyle::paint`].
    pub paint: f32,
    pub width: f32,
    pub dash: f32,
    pub gap: f32,
    pub approach: f32,
    pub double_offset: f32,
    pub lane_zoom: f32,
    pub axis_zoom: f32,
    pub fade_from: f32,
    pub stop_width: f32,
    pub yield_dash: f32,
    pub yield_gap: f32,
    pub zebra_half: f32,
    pub zebra_period: f32,
    pub zebra_fill: f32,
    pub zebra_zoom: f32,
    /// Колея траекторий узла — [`RoadPaintStyle::turn_wear`].
    pub turn_wear: f32,
    pub rut_offset: f32,
    pub rut_sigma: f32,
    pub lane_width: f32,
    /// Островки у колец (`roads/gores.rs`): шаг и ширина косой полосы,
    /// ширина обводки.
    pub hatch_period: f32,
    pub hatch_width: f32,
    pub edge_width: f32,
}

impl PaintParams {
    pub fn new(style: RoadPaintStyle) -> Self {
        Self {
            paint: style.paint(),
            width: LINE_WIDTH,
            dash: DASH,
            gap: GAP,
            approach: APPROACH,
            double_offset: DOUBLE_OFFSET,
            lane_zoom: LANE_ZOOM_MAX,
            axis_zoom: AXIS_ZOOM_MAX,
            fade_from: FADE_FROM,
            stop_width: STOP_WIDTH,
            yield_dash: YIELD_DASH,
            yield_gap: YIELD_GAP,
            zebra_half: ZEBRA_LENGTH / 2.0,
            zebra_period: ZEBRA_PERIOD,
            zebra_fill: ZEBRA_FILL,
            zebra_zoom: ZEBRA_ZOOM_MAX,
            turn_wear: style.turn_wear(),
            rut_offset: RUT_OFFSET,
            rut_sigma: RUT_SIGMA,
            lane_width: lane_width(),
            hatch_period: HATCH_PERIOD,
            hatch_width: HATCH_WIDTH,
            edge_width: EDGE_WIDTH,
        }
    }
}

/// Проход материала краски. Колея траекторий узла ложится **как тень** —
/// перекрытие двух колей не светлее одной, — а обычный блендинг перекрытия
/// складывает. Поэтому она рисуется в два прохода, и кадр между ними хранит
/// её в своей альфе (асфальт под ней непрозрачен, альфа там единица):
///
/// - [`Self::WearMask`] пишет только альфу, операцией `Min`, значение
///   `1 − колея`: в пикселе остаётся **наибольшая** колея из всех полос;
/// - [`Self::Wear`] умножает цвет на `2 − альфа`, то есть на `1 + колея` —
///   ту же колею, что множитель `surface.wgsl`, — и возвращает альфу в
///   единицу. Повторное наложение там, где полосы перекрылись, видит
///   единицу и цвета не меняет.
///
/// Меш маски лежит ниже меша наложения (`Z_ROAD_WEAR_MASK` < `Z_ROAD_WEAR`),
/// и прозрачная фаза рисует их по порядку целиком.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PaintPass {
    #[default]
    Lines,
    WearMask,
    Wear,
}

/// Материал слоя краски: вершинный цвет с альфой рождения линии × линия,
/// которую шейдер рисует по координатам полосы. По одному на проход
/// ([`PaintPass`]) на приложение, хэндлы — в `surface::SurfaceMaterials`
/// рядом с материалами поверхностей.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(PaintKey)]
pub struct PaintMaterial {
    #[uniform(0)]
    pub params: PaintParams,
    pub pass: PaintPass,
}

/// Ключ конвейера материала краски — проход.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaintKey {
    pass: PaintPass,
}

impl From<&PaintMaterial> for PaintKey {
    fn from(material: &PaintMaterial) -> Self {
        Self {
            pass: material.pass,
        }
    }
}

impl Material2d for PaintMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    /// Та же раскладка вершин, что у материала поверхностей: позиция, цвет и
    /// `ATTRIBUTE_RIBBON`. Проходы колеи — свой шейдерный вариант и своё
    /// смешивание ([`PaintPass`]).
    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_RIBBON.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        let (define, blend, write_mask) = match key.bind_group_data.pass {
            PaintPass::Lines => return Ok(()),
            PaintPass::WearMask => (
                "WEAR_MASK",
                BlendState {
                    color: BlendComponent::REPLACE,
                    alpha: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Min,
                    },
                },
                ColorWrites::ALPHA,
            ),
            PaintPass::Wear => (
                "WEAR_APPLY",
                BlendState {
                    // цвет × (1 + колея): src — единица, умноженная на dst,
                    // плюс dst × (1 − альфа маски)
                    color: BlendComponent {
                        src_factor: BlendFactor::Dst,
                        dst_factor: BlendFactor::OneMinusDstAlpha,
                        operation: BlendOperation::Add,
                    },
                    // альфа — снова единица
                    alpha: BlendComponent::REPLACE,
                },
                ColorWrites::ALL,
            ),
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(define.into());
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(blend);
                target.write_mask = write_mask;
            }
        }
        Ok(())
    }
}
