//! Фактура поверхностей карты: земля, зелень, песок, вода, асфальт и тротуар
//! рисуются одним материалом [`SurfaceMaterial`] (шейдер
//! `assets/shaders/surface.wgsl`) вместо плоского `ColorMaterial`.
//!
//! Базовый цвет по-прежнему вершинный — слитые меши слоёв собираются как и
//! раньше; шейдер кладёт поверх него процедурный шум по **мировым**
//! координатам: крупную «облачность» тона, мелкое зерно, крапинки травы, дрейф
//! ряби на воде и колею на проезжей части. Ни текстур, ни
//! художника: вся фактура — функция координаты пикселя, и потому две
//! перекрывающиеся ленты одного слоя красятся одинаково (стык дорог в узле
//! остаётся невидимым), а на любом зуме шум либо виден, либо погашен, но
//! никогда не мерцает.
//!
//! Набор параметров на вид поверхности — [`SurfaceKind::params`]; сила всей
//! фактуры разом — ползунок [`SurfaceStyle::texture`] (панель Surfaces), ноль
//! возвращает прежние плоские заливки.

use std::time::Duration;

use bevy::ecs::system::SystemParam;
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
use crate::map::buildings::material::{RoofMaterial, RoofMaterialHandle};
use crate::map::meshing::{ATTRIBUTE_RIBBON, MeshBuilder};
use crate::map::roads::paint::{PaintMaterial, PaintParams, PaintPass, RoadPaintStyle};
use crate::map::water::{WATER_SHORE_COLOR, WATER_SHORE_WIDTH};
use crate::prefs::retuned;

const SHADER_PATH: &str = "shaders/surface.wgsl";

/// Дефолт и границы ползунка Texture ([`SurfaceStyle::texture`]) — общий
/// множитель амплитуд шума поверхностей: 0 — плоские заливки, какими они были
/// до фактуры; 1 — фактура как задумана; полтора — заметно грубее, дальше шум
/// перекрикивает цвет слоя.
pub const SURFACE_TEXTURE_DEFAULT: f32 = 1.0;
pub const SURFACE_TEXTURE_MIN: f32 = 0.0;
pub const SURFACE_TEXTURE_MAX: f32 = 1.5;
pub const SURFACE_TEXTURE_STEP: f32 = 0.1;

// Умолчание ползунка — внутри его же диапазона.
const _: () = {
    assert!(
        SURFACE_TEXTURE_DEFAULT >= SURFACE_TEXTURE_MIN
            && SURFACE_TEXTURE_DEFAULT <= SURFACE_TEXTURE_MAX
    );
};

/// Параметры фактуры — юниформ шейдера. Зеркало `SurfaceParams` в
/// `surface.wgsl`: порядок полей обязан совпадать.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct SurfaceParams {
    /// Сдвиг тона по крупному шуму, множитель на канал: положительный шум
    /// тянет цвет в `1 + tint`, отрицательный — в `1 - tint`. Так луг
    /// переливается жёлто-зелёным и сине-зелёным, а не только светлее/темнее.
    pub tint: Vec4,
    /// Цвет отмели на кромке ленты (линейный; `a` не читается). Только у воды:
    /// площадной воде отмель кладёт геометрия (`water::mesh_water_areas`),
    /// а ленте русла — шейдер по её координате поперёк, см. `shore_width`.
    pub shore_color: Vec4,
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
    /// Износ покрытия: амплитуда колеи (ручка «Wear», `roads::paint::
    /// RoadPaintStyle`). Ноль — ровный асфальт; считается только на лентах с
    /// раскладкой полос (`meshing::LaneFrame`) — у проезжей части улицы.
    /// Площадная заливка того же материала ленты не несёт и износа не
    /// получает. Линии полос тут больше не рисуются: они — слой краски
    /// (`roads/paint.rs`).
    pub wear: f32,
    /// Отмель на кромках ленты, м (ноль — без отмели): от цвета
    /// `shore_color` на краю к вершинному цвету ленты на этой глубине — то же
    /// поле расстояний до берега, что у площадной воды
    /// (`water::mesh_water_areas`). Вдоль ленты отмель гаснет в разрыве
    /// («до разрыва» от нуля до минус `shore_width`): разрыв ставится только на
    /// конце, отрезанном берегом площадной воды (`map::water`), и вода под
    /// ним — у той же глубины тот же цвет. Площадная заливка ленты не несёт
    /// (полуширина ноль) и отмели от шейдера не получает.
    pub shore_width: f32,
    /// Общий множитель амплитуд — ползунок панели.
    pub intensity: f32,
}

impl SurfaceParams {
    const FLAT: Self = Self {
        tint: Vec4::ZERO,
        shore_color: Vec4::ZERO,
        mottle_amp: 0.0,
        mottle_scale: 1.0,
        grain_amp: 0.0,
        grain_scale: 1.0,
        speckle_amp: 0.0,
        speckle_scale: 1.0,
        speckle_threshold: 1.0,
        drift: 0.0,
        wear: 0.0,
        shore_width: 0.0,
        intensity: SURFACE_TEXTURE_DEFAULT,
    };
}

/// Вид поверхности — свой набор [`SurfaceParams`] и свой материал.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceKind {
    /// Земля под всей картой.
    Ground,
    /// Двор жилого квартала: трава, но истоптанная и вперемешку с проплешинами
    /// голой земли — потому и не [`Self::Grass`], у которой рисунок ровнее.
    Yard,
    Park,
    Wood,
    Grass,
    Sand,
    /// Площадная вода и русла.
    Water,
    /// Проезжая часть улицы и настил моста — асфальт с колеёй. Настил
    /// отдельного вида не получает: покрытие то же, а пешеходный мостик в
    /// том же меше без раскладки полос и так остаётся без колеи.
    Street,
    /// Дорожка, тропа.
    Alley,
    Sidewalk,
}

impl SurfaceKind {
    pub const ALL: [Self; 10] = [
        Self::Ground,
        Self::Yard,
        Self::Park,
        Self::Wood,
        Self::Grass,
        Self::Sand,
        Self::Water,
        Self::Street,
        Self::Alley,
        Self::Sidewalk,
    ];

    /// Фактура вида при силе `texture` и колее асфальта `wear`.
    pub fn params(self, texture: f32, wear: f32) -> SurfaceParams {
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
            // двор: пятна крупнее амплитудой и мельче шагом, чем у газона
            // (0.105 на 22 м против 0.06 на 30), а крап реже и слабее
            // (0.05 на 2.2 м с порогом 0.7 против 0.07 на 1.4 с 0.64) — это и
            // есть разница между лугом и двором, по которому ходят:
            // проплешины у подъездов, трава по углам
            Self::Yard => SurfaceParams {
                tint: Vec4::new(0.08, 0.045, -0.06, 0.0),
                mottle_amp: 0.105,
                mottle_scale: 22.0,
                grain_amp: 0.055,
                grain_scale: 1.6,
                speckle_amp: 0.05,
                speckle_scale: 2.2,
                speckle_threshold: 0.7,
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
                shore_color: Vec4::from_array(WATER_SHORE_COLOR.to_linear().to_f32_array()),
                shore_width: WATER_SHORE_WIDTH,
                ..flat
            },
            // асфальт: заплаты в десятки метров и мелкое зерно покрытия
            Self::Street => SurfaceParams {
                mottle_amp: 0.03,
                mottle_scale: 60.0,
                grain_amp: 0.04,
                grain_scale: 1.2,
                wear,
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
///
/// Рядом лежит и материал слоя краски (`roads/paint.rs`): краска — то, что
/// нанесено на асфальт, и её юниформ правит та же ручка, что колею асфальта
/// (`RoadPaintStyle`).
#[derive(Resource)]
pub struct SurfaceMaterials {
    handles: [Handle<SurfaceMaterial>; SurfaceKind::ALL.len()],
    /// По материалу краски на проход (`PaintPass`, в порядке `PAINT_PASSES`).
    paints: [Handle<PaintMaterial>; 3],
}

/// Проходы материала краски — порядок хэндлов [`SurfaceMaterials::paints`].
const PAINT_PASSES: [PaintPass; 3] = [PaintPass::Lines, PaintPass::WearMask, PaintPass::Wear];

impl SurfaceMaterials {
    pub fn handle(&self, kind: SurfaceKind) -> Handle<SurfaceMaterial> {
        self.handles[kind as usize].clone()
    }
}

/// Материалы по одному на вид и материал краски — на старте приложения, с
/// силой фактуры и краской из сохранённых настроек. Стиля краски может не
/// быть (витрина без дорог) — тогда умолчание.
pub fn init_surface_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    mut paints: ResMut<Assets<PaintMaterial>>,
    style: Res<SurfaceStyle>,
    paint: Option<Res<RoadPaintStyle>>,
) {
    let paint = paint.map_or_else(RoadPaintStyle::default, |paint| *paint);
    let handles = SurfaceKind::ALL.map(|kind| {
        materials.add(SurfaceMaterial {
            params: kind.params(style.texture, paint.wear()),
        })
    });
    let paints = PAINT_PASSES.map(|pass| {
        paints.add(PaintMaterial {
            params: PaintParams::new(paint),
            pass,
        })
    });
    commands.insert_resource(SurfaceMaterials { handles, paints });
}

/// Чем красить слой карты: плоским `ColorMaterial` (кант, рельсы, стены —
/// всё, чему фактура ни к чему), фактурным материалом поверхности или
/// материалом кровель (`map::buildings::material`).
///
/// Приватен вместе со [`spawn_layer`]: наружу модуль отдаёт [`MaterialSpec`],
/// а готовый хэндл существует только между `resolve` и спавном.
enum LayerMaterial {
    Flat(Handle<ColorMaterial>),
    Surface(Handle<SurfaceMaterial>),
    Roof(Handle<RoofMaterial>),
    Paint(Handle<PaintMaterial>),
}

/// Чем красить слой — **описанием, а не хэндлом**.
///
/// Хэндл берётся из `Assets`, то есть из мира, и это единственное, ради чего
/// сборке слоя нужен был бы Bevy. Описание о мире не знает ничего, поэтому
/// `mesh_*` остаётся чистой функцией: её зовут и игра, и тест, и офлайн-бенч
/// (`examples/bench/map_meshing`) — одним и тем же вызовом, а не тремя разными
/// путями. Разворачивает описание в хэндл адаптер, [`spawn_layers`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MaterialSpec {
    /// Белый непрозрачный: кант, рельсы, стены — всё, чему фактура ни к чему.
    Flat,
    /// Белый **с блендингом**: слой, в котором есть полупрозрачное — тень
    /// моста, тень забора. Непрозрачный материал съел бы вершинную альфу.
    Blend,
    /// Фактурный материал поверхности. Меш обязан быть собран через
    /// [`MeshBuilder::with_surface_coords`], иначе материал его не примет.
    Surface(SurfaceKind),
    /// Материал кровель (`map::buildings::material`). Меш обязан быть собран
    /// через [`MeshBuilder::with_roof_coords`]. Один на всё приложение, как и
    /// фактурные, — вариант появился вместе со зданиевыми слоями.
    Roof,
    /// Материал слоя краски (`roads/paint.rs`) на проходе `PaintPass`: линии
    /// или маска и наложение колеи узлов. Меш — полосы
    /// `MeshBuilder::push_paint_strip` в сборщике с координатами поверхности.
    Paint(PaintPass),
}

/// Собранный слой карты: меш плюс всё, что нужно знать, чтобы положить его в
/// мир, — рунга z, имя и вид материала.
///
/// **Один тип на все слои карты**, а не свой на каждый модуль: дороги отдают
/// девять таких, промзона пять, рельсы три, забор один. Модуль, собранный как
/// `-> Vec<LayerMesh>`, читается тем же способом, что и любой соседний, и его
/// адаптер — один вызов [`spawn_layers`], а не переписанный цикл.
///
/// `name` — не выдуманный идентификатор: это ровно та строка, под которой
/// сущность слоя видна в живом приложении (`Name`), то есть та, по которой её
/// ищут через BRP.
pub struct LayerMesh {
    pub builder: MeshBuilder,
    pub z: f32,
    pub name: &'static str,
    pub material: MaterialSpec,
}

impl LayerMesh {
    pub fn new(builder: MeshBuilder, z: f32, name: &'static str, material: MaterialSpec) -> Self {
        Self {
            builder,
            z,
            name,
            material,
        }
    }
}

/// Два плоских `ColorMaterial` на всё приложение — непрозрачный и с
/// блендингом, ровно те, что называет [`MaterialSpec`].
///
/// Ресурс, а не `materials.add(...)` в каждой пересборке: слой пересобирается
/// на каждую ступень зума и на каждое осевшее солнце, а материал у него всё
/// время один и тот же. Ровесник [`SurfaceMaterials`] и живёт по тому же
/// правилу.
#[derive(Resource)]
pub struct FlatMaterials {
    opaque: Handle<ColorMaterial>,
    blend: Handle<ColorMaterial>,
}

/// Плоские материалы — на старте приложения, рядом с фактурными.
pub fn init_flat_materials(mut commands: Commands, mut materials: ResMut<Assets<ColorMaterial>>) {
    let opaque = materials.add(Color::WHITE);
    let blend = materials.add(ColorMaterial {
        alpha_mode: AlphaMode2d::Blend,
        ..default()
    });
    commands.insert_resource(FlatMaterials { opaque, blend });
}

/// Слой карты из собранного меша: пустой сборщик не спавнится вовсе. Меш для
/// [`LayerMaterial::Surface`] обязан быть собран через
/// `MeshBuilder::with_surface_coords`, иначе материал его не примет.
///
/// Приватен: после шва это примитив, на котором стоит [`spawn_layers`], и
/// звать его снаружи модуля незачем — адаптеры слоёв ходят через шов.
fn spawn_layer(
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
        LayerMaterial::Roof(handle) => layer.insert(MeshMaterial2d(handle)),
        LayerMaterial::Paint(handle) => layer.insert(MeshMaterial2d(handle)),
    };
}

/// Всё, во что разворачивается [`MaterialSpec`], одним параметром системы.
///
/// Одним, а не двумя ресурсами по отдельности: адаптеру слоя они нужны только
/// вместе и только чтобы отдать их в [`spawn_layers`], а подпись системы
/// пересборки и без них длинная — у промзоны и трамвая два отдельных `Res`
/// уводили её за предел clippy. Идиома `ui/debug/mod.rs::DebugValues`.
#[derive(SystemParam)]
pub struct LayerMaterials<'w> {
    flats: Res<'w, FlatMaterials>,
    surfaces: Res<'w, SurfaceMaterials>,
    roof: Res<'w, RoofMaterialHandle>,
}

impl LayerMaterials<'_> {
    /// Описание — в хэндл. Единственное место, где это происходит.
    fn resolve(&self, spec: MaterialSpec) -> LayerMaterial {
        match spec {
            MaterialSpec::Flat => LayerMaterial::Flat(self.flats.opaque.clone()),
            MaterialSpec::Blend => LayerMaterial::Flat(self.flats.blend.clone()),
            MaterialSpec::Surface(kind) => LayerMaterial::Surface(self.surfaces.handle(kind)),
            MaterialSpec::Roof => LayerMaterial::Roof(self.roof.handle()),
            MaterialSpec::Paint(pass) => {
                let slot = PAINT_PASSES.iter().position(|&known| known == pass);
                LayerMaterial::Paint(self.surfaces.paints[slot.unwrap_or(0)].clone())
            }
        }
    }
}

/// Положить в мир всё, что собрал `mesh_*` одного модуля, под одной меткой.
///
/// Это вторая половина шва: сборка сказала, **что** нарисовано, адаптер знает,
/// **куда** это деть. Разворачивание [`MaterialSpec`] в хэндл живёт здесь и
/// только здесь, так что описание слоя остаётся тем, что можно вернуть из
/// чистой функции и сравнить в тесте.
pub fn spawn_layers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    layers: impl IntoIterator<Item = LayerMesh>,
    tag: impl Bundle + Clone,
) {
    for layer in layers {
        let material = materials.resolve(layer.material);
        spawn_layer(
            commands,
            meshes,
            layer.builder,
            layer.z,
            layer.name,
            material,
            tag.clone(),
        );
    }
}

/// Во что обошёлся один слой: имя, вершины, время сборки.
///
/// Живёт рядом с [`layer_costs`], который его и собирает: строка замера — это
/// слой, посчитанный на шве, а не деталь зданий, где её впервые понадобилось
/// печатать. Зданиям и машинам она нужна тем же типом, они берут её отсюда.
pub struct LayerCost {
    pub name: &'static str,
    pub vertices: usize,
    pub elapsed: Duration,
}

/// Слои, собранные `mesh_*`, — строками офлайн-замера
/// (`examples/bench/map_meshing.rs`).
///
/// Время одно на всю сборку и стоит первой строкой (`build`), с нулём вершин:
/// `mesh_*` строит все свои слои одним проходом, и делить миллисекунды между
/// ними нечем — та же форма, что у шагов `breaks`/`districts` в
/// `measure_cars`. Дальше идут слои: имя — то самое, под которым слой виден в
/// живом мире, — и вершины.
///
/// Существует ради того, чтобы `measure_*` каждого модуля был одной строкой:
/// **своей сборки у замера нет**, он зовёт игровой `mesh_*`. Это и есть то,
/// ради чего делался шов, и это отличает их от `buildings::measure_layers` и
/// `cars::measure_cars`, которые повторяют шаги сборки нарочно — им надо
/// развести их по строкам.
pub fn layer_costs(layers: &[LayerMesh], elapsed: Duration) -> Vec<LayerCost> {
    std::iter::once(LayerCost {
        name: "build",
        vertices: 0,
        elapsed,
    })
    .chain(layers.iter().map(|layer| LayerCost {
        name: layer.name,
        vertices: layer.builder.vertex_count(),
        elapsed: Duration::ZERO,
    }))
    .collect()
}

/// Правка ползунка Texture, Paint или Wear — новые параметры в каждый
/// материал; меши не трогаются.
pub fn retune_surface_materials(
    style: Res<SurfaceStyle>,
    paint: Res<RoadPaintStyle>,
    surfaces: Res<SurfaceMaterials>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    mut paints: ResMut<Assets<PaintMaterial>>,
) {
    for kind in SurfaceKind::ALL {
        if let Some(mut material) = materials.get_mut(&surfaces.handle(kind)) {
            material.params = kind.params(style.texture, paint.wear());
        }
    }
    for handle in &surfaces.paints {
        if let Some(mut material) = paints.get_mut(handle) {
            material.params = PaintParams::new(*paint);
        }
    }
}

/// Когда перенастраивать материалы — обе ручки, одной регистрацией.
pub fn retunes_on() -> impl SystemCondition<()> {
    retuned::<SurfaceStyle>.or_else(retuned::<RoadPaintStyle>)
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
    fn only_carriageways_wear() {
        for kind in SurfaceKind::ALL {
            let worn = kind.params(1.0, 0.075).wear > 0.0;
            let carriageway = matches!(kind, SurfaceKind::Street);
            assert_eq!(worn, carriageway, "{kind:?}");
        }
    }

    #[test]
    fn texture_zero_flattens_every_surface() {
        for kind in SurfaceKind::ALL {
            assert_eq!(kind.params(0.0, 0.075).intensity, 0.0, "{kind:?}");
        }
    }
}
