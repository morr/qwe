//! Процедурные кроны деревьев в стиле Watabou Village Generator.
//! Алгоритм восстановлен из Village.js, подробный разбор —
//! `.claude/skills/osm-map/references/tree-algo.md`:
//! мятый 12-угольник → «bloat» (рекурсивное выдавливание середин рёбер) →
//! облачный контур; внутренние кольца-штрихи; тень — растянутый силуэт.

mod conifer;
mod crown;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

pub use self::conifer::{ConiferField, ConiferNoiseStyle};
// Приватные реэкспорты: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать геометрию.
use self::crown::{
    CROWN_COLOR, INK_COLOR, crown_geometry, crown_mesh, shadow_template, variant_rng,
};
use crate::loading::AppState;
use crate::map::SHADOW_COLOR;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, TreeCompose, TreeRowLayout, TreeRowPlacement};
use crate::map::roads::{RoadJoin, RoadSmoothing};
use crate::settings::{TREE_NOISE_MIX_DEFAULT, TREE_VARIANTS, Z_TREE, Z_TREE_SHADOW};

/// Форма кроны — `w.TREE_SHAPE` у watabou.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub enum TreeShape {
    /// `zb.cloud`: облачный контур (`Bloater`), кольца `BALL_BANDS2`.
    #[default]
    Cotton,
    /// `zb.pine`: колючий контур (`Spiker::simple`), кольца `CONE_BANDS3`.
    Conifer,
    /// `zb.palm`: изогнутые листья (`Spiker::bent`), кольца `PALM_BANDS2`.
    Palm,
    /// Смешанный лес: хвойные массивы среди облачных крон. Собственной
    /// геометрии не имеет — форму каждого дерева разрешает `resolve` по полю
    /// хвои (`conifer::ConiferField`).
    Mixed,
}

impl TreeShape {
    pub const ALL: [Self; 4] = [Self::Cotton, Self::Conifer, Self::Palm, Self::Mixed];
    /// Формы с собственной геометрией кроны — всё, кроме `Mixed`.
    #[cfg(test)]
    const CONCRETE: [Self; 3] = [Self::Cotton, Self::Conifer, Self::Palm];

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
            variance: 0.2,
            shape: TreeShape::default(),
            conifer_share: 0.1,
            noise_mix: TREE_NOISE_MIX_DEFAULT,
            density: 1.0,
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
    /// Стык ленты зелёной подложки аллеи (`map::spawn::spawn_tree_row_band`).
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
            // не `Off`, как у дорог: улица углом на повороте выглядит улицей, а
            // лес — никогда. Полоса без сглаживания читается как нарисованная
            // линия, а не как заросшая обочина
            smoothing: RoadSmoothing::Light,
            // у дороги кант отделяет полотно от фона, у зарослей отделять нечего:
            // подложка и так темнее газона, а второй зелёный контур читается как
            // ещё одна дорожка вдоль аллеи
            casing: false,
        }
    }
}

impl TreeStyle {
    /// Квантованные множители яркости: `2^(variance·bell)` для bell от −1 до 1.
    fn tint_factors(&self) -> [f32; 5] {
        [-1.0, -0.5, 0.0, 0.5, 1.0].map(|bell: f32| 2.0_f32.powf(self.variance * bell))
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

/// Крона или её тень — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
pub struct TreeTag;

/// Спавн деревьев: `TREE_VARIANTS` крон единичного радиуса, каждому дереву —
/// вариант, оттенок и масштаб детерминированно по индексу; ползунок плотности
/// отдаёт префикс набора (см. [`visible_count`]). Кроны — сущность на
/// дерево (свой оттенок и свой z), тени — **один слитый меш на все деревья**:
/// полупрозрачная сущность попадает в сортируемую фазу `Transparent2d`, а
/// тысяча таких сущностей в ней вместе с двадцатью тысячами спрайтов пешеходов
/// теряется по одной-две на кадр (тень мигает). Слой из одного меша — как
/// `building_shadows` — этой фазе не по зубам и рисуется одним вызовом.
pub fn spawn_trees(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    style: &TreeStyle,
    trees: &[(Vec2, f32)],
    appears_at: &[f32],
    field: &ConiferField,
) {
    // по пулу вариантов на каждую конкретную форму — у `Mixed` их два
    let pools: Vec<(TreeShape, Vec<(Handle<Mesh>, MeshBuilder)>)> = style
        .shape
        .crown_shapes()
        .iter()
        .map(|&shape| {
            let variants = (0..TREE_VARIANTS)
                .map(|variant| {
                    let mut rng = variant_rng(variant);
                    let geometry = crown_geometry(shape, &mut rng);
                    (
                        meshes.add(crown_mesh(&geometry, style, &mut rng)),
                        shadow_template(&geometry, &mut rng),
                    )
                })
                .collect();
            (shape, variants)
        })
        .collect();
    let tints: Vec<Handle<ColorMaterial>> = style
        .tint_factors()
        .iter()
        .map(|&factor| materials.add(Color::srgb(factor, factor, factor)))
        .collect();

    let mut shadows = MeshBuilder::default();
    let visible = visible_count(appears_at, style.density);
    for (index, &(position, radius)) in trees.iter().take(visible).enumerate() {
        let shape = style.shape.resolve(field.is_conifer(index));
        let variants = &pools
            .iter()
            .find(|(pooled, _)| *pooled == shape)
            .expect("crown_shapes covers every shape resolve can return")
            .1;
        let (crown, shadow) = &variants[index % variants.len()];
        // микрошаг по z: пересекающиеся кроны рисуются в стабильном порядке
        let z = Z_TREE + (index % 512) as f32 * 1e-3;
        commands.spawn((
            TreeTag,
            Mesh2d(crown.clone()),
            MeshMaterial2d(tints[(index * 7) % tints.len()].clone()),
            Transform::from_translation(position.extend(z)).with_scale(Vec3::splat(radius)),
            DespawnOnExit(AppState::Playing),
            Name::new("tree"),
        ));
        shadows.push_template(shadow, position, radius);
    }

    if !shadows.is_empty() {
        // слой теней — один меш на весь лес, и веер хвои весит вчетверо против
        // одиночного силуэта: при разборе просадок смотреть в первую очередь сюда
        debug!(
            "tree shadows: {} vertices for {visible} trees ({:?})",
            shadows.vertex_count(),
            style.shape
        );
        commands.spawn((
            TreeTag,
            Mesh2d(meshes.add(shadows.build())),
            MeshMaterial2d(materials.add(SHADOW_COLOR)),
            Transform::from_xyz(0.0, 0.0, Z_TREE_SHADOW),
            DespawnOnExit(AppState::Playing),
            Name::new("tree_shadows"),
        ));
    }
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

/// Пересборка крон после правки стиля из UI: деспавн старых сущностей и
/// повторный спавн из тех же позиций (`MapData::trees` не трогается).
pub fn rebuild_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
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
    spawn_trees(
        &mut commands,
        &mut meshes,
        &mut materials,
        &style,
        &map.trees,
        &map.tree_appears_at,
        &field,
    );
}

#[cfg(test)]
mod tests;
