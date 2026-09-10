//! Заборы: линия и её тень.
//!
//! В частном секторе с воздуха видны не дома, а **участки**: земля разрезана
//! сеткой заборов, и без неё дома стоят в общем поле, чего в жизни не бывает.
//! В Туле таких линий 427 (356 заборов, 71 стена).
//!
//! Забор сверху — это волосок в четверть метра, и держится он на снимке
//! **тенью**: собственная линия почти не видна, а тень от неё лежит рядом
//! тёмной ниткой. Поэтому рисуются обе, и тень — первой.
//!
//! Ширина линии растёт с отдалением ([`FENCE_LODS`], приём трамвая): четверть
//! метра — это меньше пикселя уже на 0.3 м/px, то есть ровно там, где забор
//! обязан быть виден. Целиться в ~1.5 экранных пикселя честнее, чем рисовать
//! честные 25 см и не показать ничего.
//!
//! **Навмеша заборы не касаются.** Настоящий забор непроходим, но 427 линий,
//! режущих кварталы, отрезали бы дворы от улиц, а толпа ходит именно там; та
//! же причина, по которой сквозь машины ходят насквозь.

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::{FenceKind, FenceLine, MapData};
use crate::map::roads::{RoadJoin, push_ribbon};
use crate::map::surface::{self, LayerMaterial};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};
use crate::settings::Z_FENCE;

/// Высота забора, м: по ней считается длина тени тем же котангенсом высоты
/// солнца, что у домов и вагонов. Двухметровый глухой забор частного сектора.
const FENCE_HEIGHT: f32 = 2.0;
/// Живая изгородь ниже и мягче.
const HEDGE_HEIGHT: f32 = 1.4;

/// Ступень зум-LOD: ширина линии, м. Считана из ~1.5 экранных пикселей на
/// **худшем** краю ступени — той же меркой, что ширина трамвайной линии.
pub struct FenceLod {
    pub max_zoom: f32,
    pub width: f32,
}

pub const FENCE_LODS: [FenceLod; 4] = [
    // вблизи — настоящая доска
    FenceLod {
        max_zoom: 0.12,
        width: 0.25,
    },
    FenceLod {
        max_zoom: 0.35,
        width: 0.5,
    },
    FenceLod {
        max_zoom: 0.9,
        width: 1.3,
    },
    // дальше забор не рисуется вовсе: на общем плане сетка участков
    // превращается в грязь
    FenceLod {
        max_zoom: f32::INFINITY,
        width: 0.0,
    },
];

pub enum FenceLods {}

impl ZoomLods for FenceLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        FENCE_LODS.iter().map(|lod| lod.max_zoom)
    }
}

pub type FenceZoomBucket = ZoomBucket<FenceLods>;

/// Слой заборов — своя метка: пересобирается он по ступени зума и по солнцу.
#[derive(Component)]
pub struct FenceLayerTag;

/// Цвета: доска и профнастил серо-бурые, бетонная стена светлее и холоднее,
/// изгородь зелёная.
const FENCE_COLOR: Color = Color::srgb(0.435, 0.404, 0.353);
const WALL_COLOR: Color = Color::srgb(0.549, 0.541, 0.522);
const HEDGE_COLOR: Color = Color::srgb(0.298, 0.376, 0.243);

pub fn rebuild_fences(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<FenceZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<FenceLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let width = FENCE_LODS[bucket.index].width;
    if width <= 0.0 {
        return;
    }
    let builder = mesh_fences(&map.fences, width);
    let vertices = builder.vertex_count();
    if builder.is_empty() {
        return;
    }
    // тень полупрозрачна, сама линия нет — один меш с блендингом
    let material = materials.add(ColorMaterial {
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..default()
    });
    surface::spawn_layer(
        &mut commands,
        &mut meshes,
        builder,
        Z_FENCE,
        "fences",
        LayerMaterial::Flat(material),
        FenceLayerTag,
    );
    info!(
        "fences: {} lines at {width:.2} m ({vertices} verts)",
        map.fences.len()
    );
}

/// Сначала все тени, потом все линии: иначе тень одного забора легла бы на
/// соседний — то же правило, что у машин и вагонов.
fn mesh_fences(fences: &[FenceLine], width: f32) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let shadow = SHADOW_COLOR.to_linear();
    for fence in fences {
        let height = match fence.kind {
            FenceKind::Hedge => HEDGE_HEIGHT,
            _ => FENCE_HEIGHT,
        };
        let offset = shadow_dir() * (height * shadow_length_scale());
        let shifted: Vec<Vec2> = fence.points.iter().map(|point| *point + offset).collect();
        push_ribbon(&mut builder, &shifted, width, shadow, RoadJoin::Round);
    }
    for fence in fences {
        let color = match fence.kind {
            FenceKind::Fence => FENCE_COLOR,
            FenceKind::Wall => WALL_COLOR,
            FenceKind::Hedge => HEDGE_COLOR,
        };
        push_ribbon(
            &mut builder,
            &fence.points,
            width,
            color.to_linear(),
            RoadJoin::Round,
        );
    }
    builder
}
