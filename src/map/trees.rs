//! Процедурные кроны деревьев в стиле Watabou Village Generator.
//! Алгоритм восстановлен из Village.js, подробный разбор —
//! `.claude/skills/osm-map/references/tree-algo.md`:
//! мятый 12-угольник → «bloat» (рекурсивное выдавливание середин рёбер) →
//! облачный контур; внутренние кольца-штрихи; тень — растянутый силуэт.

mod canopy;
mod conifer;
mod crown;

pub use self::canopy::CrownMaterial;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

pub use self::conifer::{ConiferField, ConiferNoiseStyle};
// Приватные реэкспорты: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать геометрию.
pub use self::crown::CrownParams;
use self::crown::{
    CROWN_COLOR, INK_COLOR, crown_geometry, crown_mesh, shadow_template, variant_rng,
};
use crate::loading::AppState;
use crate::map::SunOnMap;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, TreeCompose, TreeRowLayout, TreeRowPlacement};
use crate::map::roads::{RoadJoin, RoadSmoothing};
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::prefs::retuned;
use crate::settings::{TREE_NOISE_MIX_DEFAULT, TREE_VARIANTS, Z_TREE, Z_TREE_SHADOW};

/// Форма кроны — `w.TREE_SHAPE` у watabou.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub enum TreeShape {
    /// `zb.cloud`: облачный контур (`Bloater`), кольца `BALL_BANDS2`.
    Cotton,
    /// `zb.pine`: колючий контур (`Spiker::simple`), кольца `CONE_BANDS3`.
    Conifer,
    /// `zb.palm`: изогнутые листья (`Spiker::bent`), кольца `PALM_BANDS2`.
    Palm,
    /// Смешанный лес: хвойные массивы среди облачных крон. Собственной
    /// геометрии не имеет — форму каждого дерева разрешает `resolve` по полю
    /// хвои (`conifer::ConiferField`). Один вид кроны на весь город читается
    /// как узор обоев; массивы хвои разбивают его, ничего не стоя на карте.
    #[default]
    Mixed,
}

impl TreeShape {
    pub const ALL: [Self; 4] = [Self::Cotton, Self::Conifer, Self::Palm, Self::Mixed];
    /// Формы с собственной геометрией кроны — всё, кроме `Mixed`.
    pub const CONCRETE: [Self; 3] = [Self::Cotton, Self::Conifer, Self::Palm];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cotton => "Cotton",
            Self::Conifer => "Conifer",
            Self::Palm => "Palm",
            Self::Mixed => "Mixed",
        }
    }
}

/// Стиль деревьев — вкладка Trees из «Style settings» watabou. Меняется на
/// лету из UI-панели; каждое изменение пересобирает кроны (`rebuild_trees`).
#[derive(Resource, Reflect, SettingsGroup, Clone, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "trees")]
pub struct TreeStyle {
    /// Цвет листвы (`colorTree` / «Foliage»).
    pub foliage: Color,
    /// Цвет контура и штрихов (`colorTreeDetails` / «Crown details»).
    pub details: Color,
    /// Разброс яркости листвы (`treeVariance`): множитель `2^(variance·bell)`.
    pub variance: f32,
    pub shape: TreeShape,
    /// Доля хвои при форме `Mixed`, 0..1. Доля точная: порог поля хвои —
    /// квантиль его значений в деревьях (см. [`ConiferField::set_share`]).
    /// На прочих формах не используется.
    pub conifer_share: f32,
    /// Сила примеси пород при форме `Mixed`, 0..1: к значению поля в дереве
    /// добавляется `noise_mix · jitter` по позиции ствола — лиственные
    /// вкрапления в хвойных массивах и одиночные ели среди лиственных. Ноль —
    /// сплошные массивы; долю хвои примесь не сдвигает (квантиль считается по
    /// значениям с примесью).
    pub noise_mix: f32,
    /// Плотность посадки, множитель к базовой (`TREE_DENSITY_MIN..MAX`):
    /// `1` — одно дерево на `TREE_AREA_PER_TREE` (410 м²) леса.
    /// `map::osm::planting` засаживает лес сразу по `TREE_DENSITY_MAX`, а спавн
    /// показывает префикс набора (см. [`visible_count`]) — деревья при движении
    /// ползунка не пересаживаются, а появляются и исчезают.
    pub density: f32,
    /// Лесные массивы включены. Выключение убирает из мира лес целиком, аллеи и
    /// одиночные деревья живут своими тумблерами.
    pub woods: bool,
    /// Одиночные деревья из OSM-нод (`natural=tree`) включены.
    pub standalone: bool,
}

impl Default for TreeStyle {
    fn default() -> Self {
        Self {
            foliage: CROWN_COLOR,
            details: INK_COLOR,
            variance: 0.35,
            shape: TreeShape::default(),
            conifer_share: 0.1,
            noise_mix: TREE_NOISE_MIX_DEFAULT,
            density: 4.0,
            woods: true,
            standalone: true,
        }
    }
}

/// Стиль аллей (`natural=tree_row`) — своя панель Tree rows, отдельная от
/// Trees так же, как Buildings: у аллей свой набор ручек — состав посадки и
/// вид зелёной подложки. Вид крон аллейные деревья наследуют из [`TreeStyle`].
#[derive(Resource, Reflect, SettingsGroup, Clone, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "tree_rows")]
pub struct TreeRowStyle {
    /// Аллеи включены. Выключение убирает и деревья рядов, и зелёную подложку.
    pub enabled: bool,
    /// Что делать с деревом аллеи, попавшим на занятое место. Обе раскладки
    /// посчитаны на загрузке, так что переключение только пересобирает
    /// `MapData::trees` (`recompose_row_trees`).
    pub placement: TreeRowPlacement,
    /// Слушать ли шаг посадки из тегов OSM (`spacing` / `count`). `true` — такой
    /// ряд стоит целиком на любом шаге ползунка плотности, `false` — теги
    /// игнорируются и ряд подчиняется ползунку наравне с лесом. Меняет позиции,
    /// а не вид, поэтому раскладка под неё считается на загрузке заранее.
    pub osm_spacing: bool,
    /// Стык ленты зелёной подложки аллеи (`map::spawn::mesh_tree_row_band`).
    pub join: RoadJoin,
    /// Сглаживание той же подложки — Chaikin, как у дорог.
    pub smoothing: RoadSmoothing,
    /// Тёмный кант по краю подложки, отдельным слоем под заливкой.
    pub casing: bool,
}

impl Default for TreeRowStyle {
    fn default() -> Self {
        Self {
            enabled: true,
            placement: TreeRowPlacement::default(),
            osm_spacing: TreeRowLayout::default().osm_spacing,
            join: RoadJoin::default(),
            // задано явно, а не через `default()`: у дорог сглаживание — вкус, а
            // здесь требование. Полоса без него читается как нарисованная линия,
            // а не как заросшая обочина, и `Off` в этом поле — всегда ошибка
            smoothing: RoadSmoothing::Light,
            // у дороги кант отделяет полотно от фона, у зарослей отделять нечего:
            // подложка и так темнее газона, а второй зелёный контур читается как
            // ещё одна дорожка вдоль аллеи
            casing: false,
        }
    }
}

impl TreeStyle {
    /// Точки колокола, по которым квантована яркость листвы: пять оттенков —
    /// пять материалов на весь лес вместо материала на дерево.
    const TINT_BELL: [f32; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];

    /// Квантованные множители яркости: `2^(variance·bell)` для bell от −1 до 1.
    /// При `variance == 0` все пять равны единице, и лес стоит одноцветным.
    ///
    /// Публичной сделана по той же причине, что [`crown_variant`]: витрина
    /// `tree_gallery` показывает игровые оттенки, а не свою копию формулы.
    pub fn tint_factors(&self) -> [f32; 5] {
        Self::TINT_BELL.map(|bell| 2.0_f32.powf(self.variance * bell))
    }

    /// Слот в пуле [`TreeStyle::tint_factors`] для дерева номер `index`. Шаг 7
    /// взаимно прост с числом оттенков, поэтому пять подряд стоящих деревьев
    /// получают пять разных оттенков, а не идут полосами по яркости.
    pub fn tint_slot(index: usize) -> usize {
        (index * 7) % Self::TINT_BELL.len()
    }
}

/// Сколько деревьев показать при такой плотности: `MapData::trees`
/// отсортированы по плотности появления, так что нужен префикс, а не фильтр.
/// Доля каждого леса при этом точна — порог посчитан от его площади, — и
/// прореживание монотонно: шаг ползунка вверх только добавляет деревья, уже
/// стоящие не переезжают.
///
/// Породе прореживание ортогонально: её решает поле хвои по координатам, так
/// что доля хвои в прореженном наборе та же, а дерево при движении ползунка
/// плотности породу не меняет.
pub fn visible_count(appears_at: &[f32], density: f32) -> usize {
    appears_at.partition_point(|&at| at <= density)
}

/// Что сажать: где стоят деревья (позиция и радиус кроны) и при какой
/// плотности каждое появляется. Два поля `MapData`, которые всегда ходят
/// парой — и порядок в них общий, так что разъехаться им нельзя.
#[derive(Clone, Copy)]
pub struct PlantedTrees<'a> {
    pub positions: &'a [(Vec2, f32)],
    pub appears_at: &'a [f32],
}

/// Крона или её тень — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component, Clone, Copy)]
pub struct TreeTag;

/// Геометрия одного варианта кроны: то, что [`mesh_trees`] кладёт в свой пул
/// и потом повторяет под каждым деревом этого варианта.
///
/// Публичной эта сборка сделана ради витрины `tree_gallery`: демо обязано
/// показывать ровно ту геометрию, что попадает в игру, а не свою копию
/// вызовов.
pub struct CrownVariant {
    /// Меш кроны единичного радиуса — заливка, чернильный контур, штрихи.
    pub crown: Mesh,
    /// Шаблон силуэта тени; в игре он копируется в общий меш теней
    /// (`MeshBuilder::push_template`).
    pub shadow: MeshBuilder,
}

/// Крона варианта `variant` формы `shape` под стилем `style`. Вариант задан
/// целиком своим номером: крона и тень разыгрываются из одного потока
/// [`variant_rng`] подряд.
///
/// `shape` должна быть конкретной ([`TreeShape::CONCRETE`]) — `Mixed`
/// геометрии не имеет и разрешается в `Cotton`/`Conifer` раньше. `params` в
/// игре всегда [`CrownParams::default`]; крутит их только витрина
/// `tree_gallery`.
pub fn crown_variant(
    shape: TreeShape,
    variant: usize,
    style: &TreeStyle,
    params: &CrownParams,
) -> CrownVariant {
    let mut rng = variant_rng(variant, params);
    let geometry = crown_geometry(shape, &mut rng, params);
    let crown = crown_mesh(&geometry, style, &mut rng, params);
    let shadow = shadow_template(&geometry, &mut rng, params);
    CrownVariant { crown, shadow }
}

/// Хранилища материалов, которые нужны дереву: крона красится своим
/// [`CrownMaterial`], слой теней — общими материалами слоёв ([`LayerMaterials`],
/// он же разворачивает `MaterialSpec::Blend` в хэндл).
///
/// Одним параметром, а не двумя: пересборке они нужны только вместе, а её
/// подпись и без того на пределе clippy — та же причина, по которой
/// [`LayerMaterials`] сам собран из трёх ресурсов.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TreeMaterials<'w> {
    pub crowns: ResMut<'w, Assets<CrownMaterial>>,
    pub layers: LayerMaterials<'w>,
}

/// Одна крона в мире: какой меш из пула поставить, где, какого радиуса и каким
/// оттенком.
///
/// Крона — **сущность на дерево** (свой оттенок и свой z), а не часть слитого
/// меша, поэтому дерево не укладывается в [`LayerMesh`], и шов здесь принимает
/// другую форму: сборка отдаёт пул крон и список мест, а адаптер заливает пул
/// в `Assets` и спавнит по сущности на место. Деление то же самое — сборка
/// говорит, **что** нарисовано, адаптер знает, **куда** это деть.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CrownPlacement {
    pub at: Vec2,
    pub radius: f32,
    pub z: f32,
    /// Номер пула — по конкретной форме кроны, в порядке
    /// `TreeShape::crown_shapes`: у `Mixed` пулов два, у прочих форм один.
    pub pool: usize,
    /// Номер варианта внутри пула.
    pub variant: usize,
    /// Слот оттенка в [`TreeStyle::tint_factors`].
    pub tint: usize,
}

/// Собранные деревья: пул крон, места и слой теней.
///
/// Пул — готовые `Mesh`, а не `Handle<Mesh>`: хэндл берётся из мира, и это
/// единственное, ради чего сборке понадобился бы Bevy. Ровно то же соображение
/// стоит за `MaterialSpec` в [`LayerMesh`].
pub struct TreeMeshes {
    /// По пулу на каждую конкретную форму, `TREE_VARIANTS` крон в каждом.
    pub pools: Vec<Vec<Mesh>>,
    /// Множители яркости листвы — по слоту на оттенок.
    pub tints: Vec<f32>,
    pub crowns: Vec<CrownPlacement>,
    /// Слитый меш теней, одним слоем. Один, а не сущность на тень:
    /// полупрозрачная сущность попадает в сортируемую фазу `Transparent2d`, а
    /// тысяча таких сущностей в ней вместе с двадцатью тысячами спрайтов
    /// пешеходов теряется по одной-две на кадр (тень мигает). Слой из одного
    /// меша — как `building_shadows` — этой фазе не по зубам.
    pub shadows: Vec<LayerMesh>,
}

/// Счётчики, которыми была лог-строка слоя.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TreeReport {
    pub crowns: usize,
    pub shadow_vertices: usize,
    pub shape: TreeShape,
}

impl std::fmt::Display for TreeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            crowns,
            shadow_vertices,
            shape,
        } = self;
        write!(
            f,
            "tree shadows: {shadow_vertices} vertices for {crowns} trees ({shape:?})"
        )
    }
}

/// Сборка деревьев без мира: `TREE_VARIANTS` крон единичного радиуса на каждую
/// конкретную форму, каждому дереву — вариант, оттенок и масштаб
/// детерминированно по индексу; ползунок плотности отдаёт префикс набора (см.
/// [`visible_count`]).
pub fn mesh_trees(
    style: &TreeStyle,
    params: &CrownParams,
    planted: PlantedTrees,
    field: &ConiferField,
) -> (TreeMeshes, TreeReport) {
    let PlantedTrees {
        positions,
        appears_at,
    } = planted;
    // по пулу вариантов на каждую конкретную форму — у `Mixed` их два
    let shapes = style.shape.crown_shapes();
    let pools: Vec<Vec<CrownVariant>> = shapes
        .iter()
        .map(|&shape| {
            (0..TREE_VARIANTS)
                .map(|variant| crown_variant(shape, variant, style, params))
                .collect()
        })
        .collect();

    let mut shadows = MeshBuilder::default();
    let visible = visible_count(appears_at, style.density);
    let mut crowns = Vec::with_capacity(visible);
    for (index, &(at, radius)) in positions.iter().take(visible).enumerate() {
        let shape = style.shape.resolve(field.is_conifer(index));
        let pool = shapes
            .iter()
            .position(|&pooled| pooled == shape)
            .expect("crown_shapes covers every shape resolve can return");
        let variant = index % pools[pool].len();
        crowns.push(CrownPlacement {
            at,
            radius,
            // микрошаг по z: пересекающиеся кроны рисуются в стабильном порядке
            z: Z_TREE + (index % 512) as f32 * 1e-3,
            pool,
            variant,
            tint: TreeStyle::tint_slot(index),
        });
        shadows.push_template(&pools[pool][variant].shadow, at, radius);
    }

    let report = TreeReport {
        crowns: crowns.len(),
        shadow_vertices: shadows.vertex_count(),
        shape: style.shape,
    };
    let built = TreeMeshes {
        pools: pools
            .into_iter()
            .map(|pool| pool.into_iter().map(|built| built.crown).collect())
            .collect(),
        // множитель яркости уехал из цвета материала в юниформ: цвет кроне
        // теперь считает шейдер (`canopy`), и слотов ровно столько же
        tints: style.tint_factors().to_vec(),
        crowns,
        shadows: vec![LayerMesh::new(
            shadows,
            Z_TREE_SHADOW,
            "tree_shadows",
            MaterialSpec::Blend,
        )],
    };
    (built, report)
}

/// Собранные деревья — в мир: пул крон в `Assets`, по сущности на место, слой
/// теней через общий [`spawn_layers`].
pub fn spawn_tree_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut TreeMaterials,
    (built, report): (TreeMeshes, TreeReport),
) {
    let TreeMeshes {
        pools,
        tints,
        crowns,
        shadows,
    } = built;
    let pools: Vec<Vec<Handle<Mesh>>> = pools
        .into_iter()
        .map(|pool| pool.into_iter().map(|crown| meshes.add(crown)).collect())
        .collect();
    let tints: Vec<Handle<CrownMaterial>> = tints
        .into_iter()
        .map(|factor| materials.crowns.add(CrownMaterial::of(factor)))
        .collect();

    for crown in &crowns {
        commands.spawn((
            TreeTag,
            Mesh2d(pools[crown.pool][crown.variant].clone()),
            MeshMaterial2d(tints[crown.tint].clone()),
            Transform::from_translation(crown.at.extend(crown.z))
                .with_scale(Vec3::splat(crown.radius)),
            DespawnOnExit(AppState::Playing),
            Name::new("tree"),
        ));
    }

    // веер хвои весит вчетверо против одиночного силуэта: при разборе просадок
    // смотреть в первую очередь сюда
    debug!("{report}");
    spawn_layers(commands, meshes, &materials.layers, shadows, TreeTag);
}

/// Сборка `MapData::trees` из включённых источников: одиночные деревья, лес и
/// аллеи выбранной политики размещения.
///
/// Меняется сам набор деревьев, а не только их вид, поэтому вслед за сборкой
/// пересчитывается поле хвои: оно индексировано по `MapData::trees`, и без
/// пересемплирования порода поехала бы на все деревья после первой же аллеи.
///
/// Признак «уже собрано» лежит в `MapData::composed_for`, а не в `Local`: при
/// смене города ресурс заменяется целиком, а `Local` пережил бы замену и решил,
/// что для нового города работа сделана.
pub fn recompose_row_trees(
    mut map: ResMut<MapData>,
    style: Res<TreeStyle>,
    rows: Res<TreeRowStyle>,
    noise: Res<ConiferNoiseStyle>,
    mut field: ResMut<ConiferField>,
) {
    let compose = TreeCompose {
        layout: TreeRowLayout {
            placement: rows.placement,
            osm_spacing: rows.osm_spacing,
        },
        woods: style.woods,
        rows: rows.enabled,
        standalone: style.standalone,
    };
    if map.composed_for == Some(compose) {
        return;
    }
    map.compose_trees(compose);
    field.resample(&map.trees, &noise, style.noise_mix);
    field.set_share(style.conifer_share);
}

/// Значение поля хвои в каждом посаженном дереве — до спавна крон, потому что
/// именно по нему `resolve` выбирает форму. Считается один раз на город: у
/// нового города свой набор деревьев, а старые значения к нему не относятся.
pub fn build_conifer_field(
    mut field: ResMut<ConiferField>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
    noise: Res<ConiferNoiseStyle>,
) {
    let started = std::time::Instant::now();
    field.resample(&map.trees, &noise, style.noise_mix);
    field.set_share(style.conifer_share);
    debug!(
        "conifer field: {} trees sampled in {:.1?}",
        map.trees.len(),
        started.elapsed()
    );
}

/// Пересемплирование поля после правки параметров шума (панель Noise) или
/// примеси (`TreeStyle::noise_mix`). Идёт в цепочке между
/// [`recompose_row_trees`] и [`rebuild_trees`], и выходит сразу, если поле уже
/// посчитано под текущие параметры, — так смена состава не платит за второй
/// resample, а правка цвета листвы не платит вовсе.
pub fn retune_conifer_field(
    mut field: ResMut<ConiferField>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
    noise: Res<ConiferNoiseStyle>,
) {
    if field.sampled_for(&noise, style.noise_mix) {
        return;
    }
    let started = std::time::Instant::now();
    field.resample(&map.trees, &noise, style.noise_mix);
    field.set_share(style.conifer_share);
    debug!(
        "conifer field retuned: {} trees resampled in {:.1?}",
        map.trees.len(),
        started.elapsed()
    );
}

/// Когда пересобирать деревья — **и всю их связку целиком**: состав набора
/// (`recompose_row_trees`), поле хвои (`retune_conifer_field`), подложку аллей
/// и сами кроны.
///
/// Тумблеры состава и политика аллей меняют сам набор деревьев, а не только их
/// вид, поэтому пересборка идёт после сборки набора; солнце здесь потому, что
/// тень дерева строится по нему же, только запечена в шаблон варианта.
///
/// `retuned`, а не `resource_changed`: в кадре, где настройки легли на ресурс,
/// кроны ещё не спавнены и пересобирать нечего.
///
/// **Условие одно, регистрация одна** (см. `crate::map::roads::rebuilds_on`) —
/// здесь тем более: одно условие держит всю связку из четырёх систем.
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<TreeStyle>
        .or_else(retuned::<TreeRowStyle>)
        .or_else(retuned::<ConiferNoiseStyle>)
        .or_else(retuned::<SunOnMap>)
}

/// Пересборка крон после правки стиля из UI: деспавн старых сущностей и
/// повторный спавн из тех же позиций (`MapData::trees` не трогается).
pub fn rebuild_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: TreeMaterials,
    style: Res<TreeStyle>,
    map: Res<MapData>,
    mut field: ResMut<ConiferField>,
    existing: Query<Entity, With<TreeTag>>,
) {
    // порог поля пересчитывается только если поехала сама доля — правка цвета
    // листвы не должна платить за сортировку значений
    field.set_share(style.conifer_share);
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let built = mesh_trees(
        &style,
        // ручки геометрии кроны в игре не выведены никуда: город рисуется
        // дефолтом, а крутит их витрина `tree_gallery`
        &CrownParams::default(),
        PlantedTrees {
            positions: &map.trees,
            appears_at: &map.tree_appears_at,
        },
        &field,
    );
    spawn_tree_meshes(&mut commands, &mut meshes, &mut materials, built);
}

#[cfg(test)]
mod tests;
