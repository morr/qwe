//! Портал: вихрь, посчитанный в шейдере (`assets/shaders/portal.wgsl`), и
//! выжженное пятно под ним. Присутствует с начала сцены.
//!
//! Спрайтшит на девять кадров ушёл: со 160 px на 36 м он читался пиксельной
//! кашей уже на зуме толпы, а художника, который перерисует его крупнее, у
//! проекта нет. Шейдер стоит один квад и несколько формул, масштаба не
//! боится и тянет за собой HDR-яркие рукава под bloom камеры (`camera.rs`).

use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

use crate::city::City;
use crate::loading::{AppState, WorldInitSet};
use crate::settings::{PORTAL_DIAMETER, Z_PORTAL, Z_PORTAL_STAIN};
use crate::silhouette::{Glyph, Silhouettes};

const SHADER_PATH: &str = "shaders/portal.wgsl";
/// Ядро вихря: почти чёрное с фиолетовым отливом.
const PORTAL_CORE: LinearRgba = LinearRgba::new(0.02, 0.0, 0.05, 1.0);
/// Рукава и ободок: пурпур ярче 1.0 — HDR, светится через bloom. Фиолетовый
/// на карте больше ничего не значит: красное — демоны, янтарь — паника,
/// синее — вода и трамвай.
const PORTAL_RIM: LinearRgba = LinearRgba::new(2.2, 0.45, 3.0, 1.0);
/// Вращение, рад/с.
const PORTAL_SPIN: f32 = 0.9;
/// Число рукавов вихря.
const PORTAL_ARMS: f32 = 3.0;
/// Закрутка рукавов к центру (лог-спираль).
const PORTAL_TWIST: f32 = 2.5;
/// Зерно турбулентности.
const PORTAL_GRAIN: f32 = 4.0;
/// Выжженная земля под порталом: диаметр в порталах и цвет пятна.
const STAIN_RATIO: f32 = 2.6;
const STAIN_COLOR: Color = Color::srgba(0.16, 0.05, 0.20, 0.75);

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct Portal;

/// Фактическая позиция портала. Стартует с хинта города (`City::portal_hint`);
/// после заполнения navmesh снапится к ближайшему проходимому тайлу.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct PortalPos(pub Vec2);

impl Default for PortalPos {
    fn default() -> Self {
        Self(City::default().portal_hint())
    }
}

/// Униформ шейдера — раскладка та же, что у `PortalUniform` в WGSL.
#[derive(ShaderType, Clone, Copy, Debug)]
struct PortalUniform {
    core: Vec4,
    rim: Vec4,
    /// x: вращение, y: рукава, z: закрутка, w: зерно.
    params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PortalMaterial {
    #[uniform(0)]
    uniform: PortalUniform,
}

impl PortalMaterial {
    fn new() -> Self {
        Self {
            uniform: PortalUniform {
                core: Vec4::from_array(PORTAL_CORE.to_f32_array()),
                rim: Vec4::from_array(PORTAL_RIM.to_f32_array()),
                params: Vec4::new(PORTAL_SPIN, PORTAL_ARMS, PORTAL_TWIST, PORTAL_GRAIN),
            },
        }
    }
}

impl Material2d for PortalMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

pub struct PortalPlugin;

impl Plugin for PortalPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<PortalMaterial>::default())
            .register_type::<Portal>()
            .register_type::<PortalPos>()
            .init_resource::<PortalPos>()
            // пятно берёт глиф ореола из атласа силуэтов; сам атлас собирает
            // `SilhouettePlugin`, здесь только гарантия, что ресурс есть
            .init_resource::<Silhouettes>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_portal.in_set(WorldInitSet::Spawn),
            );
    }
}

fn spawn_portal(
    mut commands: Commands,
    portal_pos: Res<PortalPos>,
    silhouettes: Res<Silhouettes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PortalMaterial>>,
) {
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::from_size(Vec2::splat(PORTAL_DIAMETER)))),
        MeshMaterial2d(materials.add(PortalMaterial::new())),
        Transform::from_translation(portal_pos.0.extend(Z_PORTAL)),
        DespawnOnExit(AppState::Playing),
        Portal,
        Name::new("portal"),
    ));
    // выжженная земля — под трупами и над дорогами: перекрёсток под порталом
    // обуглен, а тела на нём остаются видны
    commands.spawn((
        silhouettes.sprite(
            Glyph::Halo,
            STAIN_COLOR,
            Vec2::splat(PORTAL_DIAMETER * STAIN_RATIO),
        ),
        Transform::from_translation(portal_pos.0.extend(Z_PORTAL_STAIN)),
        DespawnOnExit(AppState::Playing),
        Name::new("portal_stain"),
    ));
}
