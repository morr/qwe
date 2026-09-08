//! Фактура поверхностей карты: земля, зелень, песок, вода, асфальт и тротуар
//! рисуются одним материалом [`SurfaceMaterial`] (шейдер
//! `assets/shaders/surface.wgsl`) вместо плоского `ColorMaterial`.
//!
//! Базовый цвет по-прежнему вершинный — слитые меши слоёв собираются как и
//! раньше; шейдер кладёт поверх него процедурный шум по **мировым**
//! координатам: крупную «облачность» тона, мелкое зерно, крапинки травы, дрейф
//! ряби на воде и линии разметки на проезжей части. Ни текстур, ни
//! художника: вся фактура — функция координаты пикселя, и потому две
//! перекрывающиеся ленты одного слоя красятся одинаково (стык дорог в узле
//! остаётся невидимым), а на любом зуме шум либо виден, либо погашен, но
//! никогда не мерцает.
//!
//! Набор параметров на вид поверхности — [`SurfaceKind::params`]; сила всей
//! фактуры разом — ползунок [`SurfaceStyle::texture`] (панель Surfaces), ноль
//! возвращает прежние плоские заливки.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use crate::loading::AppState;
use crate::map::meshing::{ATTRIBUTE_RIBBON, MeshBuilder};
use crate::settings::SURFACE_TEXTURE_DEFAULT;

const SHADER_PATH: &str = "shaders/surface.wgsl";

/// Ширина линии разметки, м — как у настоящей (10–15 см). На экране линия
/// всё равно не тоньше ~1.3 px (шейдер расширяет её), так что число задаёт
/// вид вблизи.
const MARKING_WIDTH: f32 = 0.15;
/// Штрих и пропуск штриховой линии, м.
const MARKING_DASH: f32 = 3.0;
const MARKING_GAP: f32 = 3.0;
/// Цвет разметки — белый, чуть прозрачный: на сером асфальте белая линия
/// читается, а прозрачность оставляет под ней зерно покрытия.
const MARKING_COLOR: LinearRgba = LinearRgba::new(0.88, 0.88, 0.86, 0.85);

/// Параметры фактуры — юниформ шейдера. Зеркало `SurfaceParams` в
/// `surface.wgsl`: порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct SurfaceParams {
    /// Сдвиг тона по крупному шуму, множитель на канал: положительный шум
    /// тянет цвет в `1 + tint`, отрицательный — в `1 - tint`. Так луг
    /// переливается жёлто-зелёным и сине-зелёным, а не только светлее/темнее.
    pub tint: Vec4,
    pub marking_color: Vec4,
    /// Амплитуда крупной «облачности» яркости и её шаг, м.
    pub mottle_amp: f32,
    pub mottle_scale: f32,
    /// Амплитуда мелкого зерна и его шаг, м.
    pub grain_amp: f32,
    pub grain_scale: f32,
    /// Крапинки: сила затемнения, шаг поля и порог (выше — реже).
    pub speckle_amp: f32,
    pub speckle_scale: f32,
    pub speckle_threshold: f32,
    /// Скорость дрейфа облачности, м/с — рябь на воде.
    pub drift: f32,
    /// Разметка: ширина линии (ноль — без разметки), штрих и пропуск, м. Где
    /// линии лежат и где рвутся — в координатах ленты
    /// (`meshing::ATTRIBUTE_RIBBON`), не здесь.
    pub marking_width: f32,
    pub marking_dash: f32,
    pub marking_gap: f32,
    /// Общий множитель амплитуд — ползунок панели.
    pub intensity: f32,
}

impl SurfaceParams {
    const FLAT: Self = Self {
        tint: Vec4::ZERO,
        marking_color: Vec4::ZERO,
        mottle_amp: 0.0,
        mottle_scale: 1.0,
        grain_amp: 0.0,
        grain_scale: 1.0,
        speckle_amp: 0.0,
        speckle_scale: 1.0,
        speckle_threshold: 1.0,
        drift: 0.0,
        marking_width: 0.0,
        marking_dash: MARKING_DASH,
        marking_gap: MARKING_GAP,
        intensity: SURFACE_TEXTURE_DEFAULT,
    };
}

/// Вид поверхности — свой набор [`SurfaceParams`] и свой материал.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceKind {
    /// Земля под всей картой.
    Ground,
    Park,
    Wood,
    Grass,
    Sand,
    /// Площадная вода и русла.
    Water,
    /// Проезжая часть улицы — асфальт с разметкой.
    Street,
    /// Настил моста: тот же асфальт с разметкой, в одном меше и улицы, и
    /// пешеходные мостики (те кода разметки не получают).
    Deck,
    /// Дорожка, тропа.
    Alley,
    Sidewalk,
}

impl SurfaceKind {
    pub const ALL: [Self; 10] = [
        Self::Ground,
        Self::Park,
        Self::Wood,
        Self::Grass,
        Self::Sand,
        Self::Water,
        Self::Street,
        Self::Deck,
        Self::Alley,
        Self::Sidewalk,
    ];

    /// Фактура вида при силе `texture`.
    pub fn params(self, texture: f32) -> SurfaceParams {
        let flat = SurfaceParams::FLAT;
        let params = match self {
            // земля: пятна от 80 до 10 м и зерно от 2.4 м до 60 см —
            // утоптанный двор, а не лист бумаги
            Self::Ground => SurfaceParams {
                tint: Vec4::new(0.03, 0.015, -0.022, 0.0),
                mottle_amp: 0.045,
                mottle_scale: 80.0,
                grain_amp: 0.04,
                grain_scale: 2.4,
                ..flat
            },
            Self::Park => SurfaceParams {
                tint: Vec4::new(0.05, 0.03, -0.045, 0.0),
                mottle_amp: 0.06,
                mottle_scale: 40.0,
                grain_amp: 0.05,
                grain_scale: 2.0,
                speckle_amp: 0.06,
                speckle_scale: 1.6,
                speckle_threshold: 0.64,
                ..flat
            },
            Self::Grass => SurfaceParams {
                tint: Vec4::new(0.05, 0.03, -0.045, 0.0),
                mottle_amp: 0.06,
                mottle_scale: 30.0,
                grain_amp: 0.055,
                grain_scale: 1.8,
                speckle_amp: 0.07,
                speckle_scale: 1.4,
                speckle_threshold: 0.64,
                ..flat
            },
            Self::Wood => SurfaceParams {
                tint: Vec4::new(0.04, 0.045, -0.03, 0.0),
                mottle_amp: 0.09,
                mottle_scale: 25.0,
                grain_amp: 0.06,
                grain_scale: 2.0,
                speckle_amp: 0.09,
                speckle_scale: 1.8,
                speckle_threshold: 0.6,
                ..flat
            },
            Self::Sand => SurfaceParams {
                tint: Vec4::new(0.03, 0.015, -0.02, 0.0),
                mottle_amp: 0.05,
                mottle_scale: 20.0,
                grain_amp: 0.06,
                grain_scale: 1.2,
                ..flat
            },
            Self::Water => SurfaceParams {
                tint: Vec4::new(-0.02, 0.0, 0.03, 0.0),
                mottle_amp: 0.06,
                mottle_scale: 18.0,
                grain_amp: 0.025,
                grain_scale: 3.0,
                drift: 0.6,
                ..flat
            },
            // асфальт: заплаты в десятки метров и мелкое зерно покрытия
            Self::Street | Self::Deck => SurfaceParams {
                marking_color: Vec4::from_array(MARKING_COLOR.to_f32_array()),
                mottle_amp: 0.03,
                mottle_scale: 60.0,
                grain_amp: 0.04,
                grain_scale: 1.2,
                marking_width: MARKING_WIDTH,
                ..flat
            },
            Self::Alley => SurfaceParams {
                tint: Vec4::new(0.02, 0.01, -0.01, 0.0),
                mottle_amp: 0.035,
                mottle_scale: 30.0,
                grain_amp: 0.05,
                grain_scale: 1.5,
                ..flat
            },
            Self::Sidewalk => SurfaceParams {
                tint: Vec4::new(0.01, 0.01, 0.0, 0.0),
                mottle_amp: 0.025,
                mottle_scale: 40.0,
                grain_amp: 0.035,
                grain_scale: 1.5,
                ..flat
            },
        };
        SurfaceParams {
            intensity: texture,
            ..params
        }
    }
}

/// Материал поверхностей: вершинный цвет × процедурный шум. Один на вид
/// поверхности, хэндлы — в [`SurfaceMaterials`].
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SurfaceMaterial {
    #[uniform(0)]
    pub params: SurfaceParams,
}

impl Material2d for SurfaceMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }

    /// Своя раскладка вершин: позиция, цвет и [`ATTRIBUTE_RIBBON`] — меш без
    /// него этим материалом не нарисовать, и это намеренно: собирать слой для
    /// него надо через `MeshBuilder::with_surface_coords`.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_RIBBON.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// Сила фактуры поверхностей; ползунок Texture панели Surfaces и BRP,
/// сохраняется между запусками. Правка не пересобирает мешей — меняется
/// только юниформ каждого материала ([`retune_surface_materials`]).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "surfaces")]
pub struct SurfaceStyle {
    /// Множитель всех амплитуд шума, 0 — плоские заливки как прежде.
    pub texture: f32,
}

impl Default for SurfaceStyle {
    fn default() -> Self {
        Self {
            texture: SURFACE_TEXTURE_DEFAULT,
        }
    }
}

/// Хэндл материала на каждый [`SurfaceKind`], один на всё приложение: слои
/// пересобираются на каждый город и на каждую правку стиля дорог, а материалы
/// живут и переиспользуются.
#[derive(Resource)]
pub struct SurfaceMaterials {
    handles: [Handle<SurfaceMaterial>; SurfaceKind::ALL.len()],
}

impl SurfaceMaterials {
    pub fn handle(&self, kind: SurfaceKind) -> Handle<SurfaceMaterial> {
        self.handles[kind as usize].clone()
    }
}

/// Материалы по одному на вид — на старте приложения, с силой фактуры из
/// сохранённых настроек.
pub fn init_surface_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    style: Res<SurfaceStyle>,
) {
    let handles = SurfaceKind::ALL.map(|kind| {
        materials.add(SurfaceMaterial {
            params: kind.params(style.texture),
        })
    });
    commands.insert_resource(SurfaceMaterials { handles });
}

/// Чем красить слой карты: плоским `ColorMaterial` (кант, рельсы, стены —
/// всё, чему фактура ни к чему) или фактурным материалом поверхности.
pub enum LayerMaterial {
    Flat(Handle<ColorMaterial>),
    Surface(Handle<SurfaceMaterial>),
}

/// Слой карты из собранного меша: пустой сборщик не спавнится вовсе. Меш для
/// [`LayerMaterial::Surface`] обязан быть собран через
/// `MeshBuilder::with_surface_coords`, иначе материал его не примет.
pub fn spawn_layer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    builder: MeshBuilder,
    z: f32,
    name: &'static str,
    material: LayerMaterial,
    tag: impl Bundle,
) {
    if builder.is_empty() {
        return;
    }
    let mut layer = commands.spawn((
        tag,
        Mesh2d(meshes.add(builder.build())),
        Transform::from_xyz(0.0, 0.0, z),
        DespawnOnExit(AppState::Playing),
        Name::new(name),
    ));
    match material {
        LayerMaterial::Flat(handle) => layer.insert(MeshMaterial2d(handle)),
        LayerMaterial::Surface(handle) => layer.insert(MeshMaterial2d(handle)),
    };
}

/// Правка ползунка Texture — новые параметры в каждый материал; меши не
/// трогаются.
pub fn retune_surface_materials(
    style: Res<SurfaceStyle>,
    surfaces: Res<SurfaceMaterials>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
) {
    for kind in SurfaceKind::ALL {
        if let Some(mut material) = materials.get_mut(&surfaces.handle(kind)) {
            material.params = kind.params(style.texture);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_slot() {
        for (index, kind) in SurfaceKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, index, "{kind:?} is out of place in ALL");
        }
    }

    #[test]
    fn only_carriageways_carry_markings() {
        for kind in SurfaceKind::ALL {
            let marked = kind.params(1.0).marking_width > 0.0;
            let carriageway = matches!(kind, SurfaceKind::Street | SurfaceKind::Deck);
            assert_eq!(marked, carriageway, "{kind:?}");
        }
    }

    #[test]
    fn texture_zero_flattens_every_surface() {
        for kind in SurfaceKind::ALL {
            assert_eq!(kind.params(0.0).intensity, 0.0, "{kind:?}");
        }
    }
}
