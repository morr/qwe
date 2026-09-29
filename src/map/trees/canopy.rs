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
//! яркости (`TreeStyle::tint_factors`) — множитель уехал в юниформ. Это кроны
//! ближней ступени; слитые куски дальних рисует один общий материал
//! ([`CrownMaterialHandle`]) с яркостью, запечённой в вершины, а локальную
//! координату несёт атрибут `meshing::ATTRIBUTE_CROWN`.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use crate::map::meshing::ATTRIBUTE_CROWN;
use crate::map::sun_light;
use crate::settings::CROWN_SHADING;

const SHADER_PATH: &str = "shaders/crown.wgsl";

/// Юниформ кроны — зеркало `CrownUniform` в шейдере: порядок полей обязан
/// совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct CrownUniform {
    /// Направление **на солнце** в плане — то же, что у кровель.
    pub light: Vec2,
    /// Множитель яркости этого дерева: раньше он был серым цветом
    /// `ColorMaterial`, теперь число в юниформе. Не путать с локальной `tint`
    /// в шейдере — та, как и у кровель, сдвиг тона.
    pub brightness: f32,
    /// Сила освещения полога и ряби листвы — [`CROWN_SHADING`].
    pub shading: f32,
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
    pub fn of(brightness: f32) -> Self {
        Self {
            params: CrownUniform {
                light: sun_light(),
                brightness,
                shading: CROWN_SHADING,
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
        let mut attributes = vec![
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
        ];
        // слитый меш дальних крон несёт координату внутри кроны отдельно:
        // позиция в нём уже мировая. Решает меш, а не материал, — один и тот
        // же материал рисует и кроны-сущности, и слитые куски
        if layout.0.contains(ATTRIBUTE_CROWN) {
            attributes.push(ATTRIBUTE_CROWN.at_shader_location(2));
            descriptor.vertex.shader_defs.push("CROWN_LOCAL".into());
        }
        descriptor.vertex.buffers = vec![layout.0.get_layout(&attributes)?];
        Ok(())
    }
}

/// Материал слитых крон (`trees::CrownDetail::Merged`): один на всё
/// приложение, с яркостью 1 — слот яркости дерева запечён в цвет его вершин
/// (`MeshBuilder::push_crown`). Живёт вне мира, как кровельный; свет в нём
/// переписывает `rebuild_trees` (солнце и так в условиях её пересборки).
#[derive(Resource)]
pub struct CrownMaterialHandle(Handle<CrownMaterial>);

impl CrownMaterialHandle {
    pub fn handle(&self) -> Handle<CrownMaterial> {
        self.0.clone()
    }
}

/// Материал слитых крон на старте приложения — после `apply_sun`, из
/// глобали которого [`CrownMaterial::of`] читает свет.
pub fn init_crown_material(mut commands: Commands, mut materials: ResMut<Assets<CrownMaterial>>) {
    let handle = materials.add(CrownMaterial::of(1.0));
    commands.insert_resource(CrownMaterialHandle(handle));
}

/// Свет осевшего солнца — в материал слитых крон; меши не трогаются.
pub fn relight_crown_material(handle: &CrownMaterialHandle, materials: &mut Assets<CrownMaterial>) {
    if let Some(mut material) = materials.get_mut(&handle.0) {
        material.params.light = sun_light();
    }
}
