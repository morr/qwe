//! Материал кроны: вершинный цвет × освещение полога × рябь листвы.
//!
//! Геометрию кроны делает [`super::crown`] по алгоритму watabou и трогать её
//! незачем — она рисует правильный **силуэт**. Мультяшной крону делало не
//! это, а плоская заливка: с воздуха полог — шар, у которого одна сторона к
//! солнцу, другая в собственной тени, и весь он в ряби отдельных ветвей.
//!
//! Считается всё в шейдере (`assets/shaders/crown.wgsl`), потому что меша на
//! дерево нет: их несколько на весь город, и каждый повторён тысячами
//! трансформов. Освещение берётся из **локальной** координаты (меш единичного
//! радиуса, так что это сразу направление от ствола), рябь — из **мировой**,
//! иначе два соседних дерева одного варианта вышли бы копиями.
//!
//! Материалов столько же, сколько было `ColorMaterial`-ов: по одному на слот
//! яркости (`TreeStyle::tint_factors`) — множитель уехал в юниформ.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use crate::map::sun_light;

const SHADER_PATH: &str = "shaders/crown.wgsl";

/// Юниформ кроны — зеркало `CrownUniform` в шейдере: порядок полей обязан
/// совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct CrownUniform {
    /// Направление **на солнце** в плане — то же, что у кровель.
    pub light: Vec2,
    /// Множитель яркости этого дерева: раньше он был серым цветом
    /// `ColorMaterial`, теперь число в юниформе.
    pub tint: f32,
    /// Сила освещения и ряби; ноль возвращает прежнюю плоскую заливку.
    pub intensity: f32,
}

/// Материал кроны. Своего ресурса-ползунка у него нет: полог освещён ровно
/// настолько, насколько освещено всё остальное, и отдельная ручка на это
/// была бы ручкой «выключить солнце для деревьев».
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct CrownMaterial {
    #[uniform(0)]
    pub params: CrownUniform,
}

impl CrownMaterial {
    /// Материал слота яркости.
    pub fn of(tint: f32, intensity: f32) -> Self {
        Self {
            params: CrownUniform {
                light: sun_light(),
                tint,
                intensity,
            },
        }
    }
}

impl Material2d for CrownMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    /// Крона рисуется с блендингом, как и раньше: у неё есть полупрозрачные
    /// куски по краю выреза, и непрозрачный режим оставил бы там ступеньки.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}
