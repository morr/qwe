//! Процедурные кроны деревьев в стиле Watabou Village Generator.
//! Алгоритм восстановлен из Village.js, подробный разбор —
//! `.claude/skills/osm-map/references/tree-algo.md`:
//! мятый 12-угольник → «bloat» (рекурсивное выдавливание середин рёбер) →
//! облачный контур; внутренние кольца-штрихи; тень — растянутый силуэт.

mod canopy;
// `pub`: диапазоны ползунков панели Noise живут рядом со своим ресурсом, и
// панель ходит за ними сюда
pub mod conifer;
mod crown;

pub use self::canopy::{CrownMaterial, CrownMaterialHandle, init_crown_material};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

pub use self::conifer::{ConiferField, ConiferNoiseStyle};
// Приватные реэкспорты: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать геометрию.
pub use self::crown::CrownParams;
#[cfg(test)]
use self::crown::crown_mesh;
#[cfg(test)]
use self::crown::shadow_template;
use self::crown::{
    CROWN_COLOR, INK_COLOR, crown_builder, crown_geometry, far_crown, shadow_templates, variant_rng,
};
use crate::loading::AppState;
use crate::map::SunOnMap;
use crate::map::TREE_DENSITY_MAX;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::TreeSet;
use crate::map::osm::{MapData, TreeCompose, TreeRowLayout, TreeRowPlacement};
use crate::map::roads::RoadJoin;
use crate::map::smooth::Smoothing;
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use crate::settings::{TREE_VARIANTS, Z_TREE, Z_TREE_SHADOW};

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
    /// показывает префикс набора (см. [`TreeSet::visible_count`]) — деревья при движении
    /// ползунка не пересаживаются, а появляются и исчезают.
    pub density: f32,
    /// Лесные массивы включены. Выключение убирает из мира лес целиком, аллеи и
    /// одиночные деревья живут своими тумблерами.
    pub woods: bool,
    /// Одиночные деревья из OSM-нод (`natural=tree`) включены.
    pub standalone: bool,
}

/// Низ и шаг ползунка плотности (`TreeStyle::density`) — множитель к базовой
/// плотности посадки. Потолок здесь не лежит: он **считается** от минимального
/// зазора между деревьями — [`TREE_DENSITY_MAX`](crate::map::TREE_DENSITY_MAX)
/// в `map/osm/planting.rs`, рядом с `TREE_MIN_SPACING`, от которого зависит.
pub const TREE_DENSITY_MIN: f32 = 0.25;
pub const TREE_DENSITY_STEP: f32 = 0.25;
/// Умолчание плотности — названо константой, чтобы диапазон и оно лежали
/// рядом и проверялись ассертом ниже.
pub const TREE_DENSITY_DEFAULT: f32 = 4.0;

/// Границы и шаг ползунка доли хвои (`TreeStyle::conifer_share`) при форме
/// `Mixed`. Доля точная: порог поля берётся квантилем, а не фиксированным
/// уровнем шума, — 0 даёт лес без хвои, 1 — только хвою.
pub const TREE_CONIFER_SHARE_MIN: f32 = 0.0;
pub const TREE_CONIFER_SHARE_MAX: f32 = 1.0;
pub const TREE_CONIFER_SHARE_STEP: f32 = 0.05;
pub const TREE_CONIFER_SHARE_DEFAULT: f32 = 0.1;

/// Сила примеси (`TreeStyle::noise_mix`): к значению поля в дереве
/// добавляется `mix · jitter`, jitter ∈ ±0.5 детерминированно по позиции
/// ствола. Ноль — сплошные массивы; 0.1 рвёт их кромки; около 0.2 одиночные
/// ели добираются до сердцевины лиственных массивов (и наоборот), а массивы
/// ещё читаются; от ~0.35 кластеризация падает вдвое и лес уходит в
/// соль-перец — само поле в пределах массива гуляет лишь на 0.1–0.3, и
/// разброс примеси быстро его перекрикивает.
pub const TREE_NOISE_MIX_DEFAULT: f32 = 0.1;
pub const TREE_NOISE_MIX_MIN: f32 = 0.0;
pub const TREE_NOISE_MIX_MAX: f32 = 1.0;
pub const TREE_NOISE_MIX_STEP: f32 = 0.05;

/// Разброс яркости листвы (`TreeStyle::variance`). Диапазона у него нет —
/// ползунок панели не показывает его, строка Trees цикличная.
const TREE_VARIANCE_DEFAULT: f32 = 0.35;

// Умолчание каждого ползунка — внутри его же диапазона; правило и его цена
// записаны в `settings.rs`, в хвосте файла, там, где раньше стоял общий блок
// ассертов.
const _: () = {
    assert!(TREE_DENSITY_DEFAULT >= TREE_DENSITY_MIN && TREE_DENSITY_DEFAULT <= TREE_DENSITY_MAX);
    assert!(
        TREE_CONIFER_SHARE_DEFAULT >= TREE_CONIFER_SHARE_MIN
            && TREE_CONIFER_SHARE_DEFAULT <= TREE_CONIFER_SHARE_MAX
    );
    assert!(
        TREE_NOISE_MIX_DEFAULT >= TREE_NOISE_MIX_MIN
            && TREE_NOISE_MIX_DEFAULT <= TREE_NOISE_MIX_MAX
    );
};

impl Default for TreeStyle {
    fn default() -> Self {
        Self {
            foliage: CROWN_COLOR,
            details: INK_COLOR,
            variance: TREE_VARIANCE_DEFAULT,
            shape: TreeShape::default(),
            conifer_share: TREE_CONIFER_SHARE_DEFAULT,
            noise_mix: TREE_NOISE_MIX_DEFAULT,
            density: TREE_DENSITY_DEFAULT,
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
    pub smoothing: Smoothing,
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
            smoothing: Smoothing::Light,
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

/// Крона или её тень — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component, Clone, Copy)]
pub struct TreeTag;

/// Ступени зума ([`TREE_LODS`]), на которых кусок деревьев виден, — бит на
/// ступень.
///
/// Всё, что строят деревья, — кроны-сущности, слитые куски крон, слои теней —
/// собрано один раз на все ступени и несёт эту маску; смена ступени
/// ([`show_tree_lod`]) только переключает `Visibility`: ни пересборки, ни
/// заливки вершин на пересечении порога. Кроны-сущности при этом ещё и
/// досыпаются и убираются пачками ([`CrownStream`]).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct TreeLodMask(u8);

impl TreeLodMask {
    /// Маска ступеней `buckets`.
    pub fn of(buckets: impl IntoIterator<Item = usize>) -> Self {
        Self(
            buckets
                .into_iter()
                .fold(0, |mask, bucket| mask | 1 << bucket),
        )
    }

    /// Виден ли слой на ступени `bucket`.
    pub fn shows(self, bucket: usize) -> bool {
        self.0 & (1 << bucket) != 0
    }

    /// Пустая маска — слой не нужен ни одной ступени, строить его незачем.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn visibility(self, bucket: usize) -> Visibility {
        if self.shows(bucket) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    }
}

/// Слой деревьев вместе со ступенями, на которых он виден.
pub struct TreeLayer {
    pub shows: TreeLodMask,
    pub layer: LayerMesh,
}

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
    /// Шаблон тени дальних ступеней ([`CrownDetail::Merged`]) — та же тень по
    /// контуру, прореженному как у [`Self::far`].
    pub far_shadow: MeshBuilder,
    /// Шаблон дальней кроны — заливка прореженного контура средним цветом
    /// полной; из таких собраны слитые куски дальних ступеней зума
    /// ([`CrownDetail::Merged`], `MeshBuilder::push_crown`).
    pub far: MeshBuilder,
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
    let (crown, ink_share) = crown_builder(&geometry, style, &mut rng, params);
    let (shadow, far_shadow) = shadow_templates(&geometry, &mut rng, params);
    // дальняя крона генератора не трогает: она читает уже разыгранный контур
    let far = far_crown(&geometry, style, params, ink_share);
    CrownVariant {
        crown: crown.build(),
        shadow,
        far_shadow,
        far,
    }
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
    /// Материал слитых крон — тот же, что `layers` отдаёт по
    /// `MaterialSpec::Crown`; здесь он ради света, который переписывает
    /// пересборка.
    pub merged: Res<'w, CrownMaterialHandle>,
    pub layers: LayerMaterials<'w>,
}

/// Одна крона в мире: какой меш из пула поставить, где, какого радиуса и каким
/// оттенком.
///
/// На ближней ступени зума ([`CrownDetail::Full`]) крона — **сущность на
/// дерево** (свой оттенок и свой z) над общим мешем варианта, а не часть
/// слитого меша: полная крона — около тысячи вершин, и слитый из них лес
/// весил бы гигабайты. Поэтому там дерево не укладывается в [`LayerMesh`], и
/// шов принимает другую форму: сборка отдаёт пул крон и список мест, а
/// адаптер заливает пул в `Assets` и спавнит по сущности на место. Деление то
/// же самое — сборка говорит, **что** нарисовано, адаптер знает, **куда** это
/// деть. Дальние ступени ([`CrownDetail::Merged`]) обходятся без мест вовсе.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CrownPlacement {
    pub at: Vec2,
    pub radius: f32,
    pub z: f32,
    /// Ступени, на которых эта крона-сущность видна.
    pub shows: TreeLodMask,
    /// Номер пула — по конкретной форме кроны, в порядке
    /// `TreeShape::crown_shapes`: у `Mixed` пулов два, у прочих форм один.
    pub pool: usize,
    /// Номер варианта внутри пула.
    pub variant: usize,
    /// Слот оттенка в [`TreeStyle::tint_factors`].
    pub tint: usize,
}

/// Собранные деревья на все ступени зума сразу: пул крон, места крон-сущностей,
/// слитые куски крон и слои теней — каждое со своей маской ступеней.
///
/// Пул — готовые `Mesh`, а не `Handle<Mesh>`: хэндл берётся из мира, и это
/// единственное, ради чего сборке понадобился бы Bevy. Ровно то же соображение
/// стоит за `MaterialSpec` в [`LayerMesh`].
pub struct TreeMeshes {
    /// По пулу на каждую конкретную форму, `TREE_VARIANTS` крон в каждом.
    /// Пусто, если ни одна ступень не рисует полных крон.
    pub pools: Vec<Vec<Mesh>>,
    /// Множители яркости листвы — по слоту на оттенок.
    pub tints: Vec<f32>,
    /// Кроны-сущности — для ступеней [`CrownDetail::Full`].
    pub crowns: Vec<CrownPlacement>,
    /// Слитые кроны — для ступеней [`CrownDetail::Merged`]: по слою
    /// `tree_crowns` на полосу плотности × кусок карты [`CROWN_CHUNK`], в
    /// котором стоят стволы.
    pub merged: Vec<TreeLayer>,
    /// Слитые меши теней — **по полосе плотности** ([`step_counts`]), каждый со
    /// своей маской ступеней, и собраны они сразу на все ступени: дальняя
    /// ступень рисует префикс набора, так что её тени — это полосы ближней
    /// без хвоста, и на пересечении порога остаётся только спрятать хвост.
    /// Полоса ещё и режется по кускам карты [`CROWN_CHUNK`], как слитые кроны:
    /// кусок вне кадра отсекается целиком. Порядок слоёв — от первых деревьев
    /// набора к последним, внутри полосы — куски по месту на карте.
    ///
    /// Слитые, а не сущность на тень: полупрозрачная сущность попадает в
    /// сортируемую фазу `Transparent2d`, а тысяча таких сущностей на одном z
    /// вместе с двадцатью тысячами спрайтов пешеходов теряется по одной-две на
    /// кадр (тень мигает). Слоёв единицы, и у каждого свой z.
    pub shadows: Vec<TreeLayer>,
}

/// Счётчики, которыми была лог-строка слоя.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TreeReport {
    /// Крон-сущностей — на все ступени [`CrownDetail::Full`] вместе.
    pub crowns: usize,
    /// Деревьев на каждой ступени [`TREE_LODS`] ([`step_counts`]).
    pub steps: [usize; TREE_LODS.len()],
    /// Вершин во всех слоях теней — на все ступени сразу.
    pub shadow_vertices: usize,
    pub shape: TreeShape,
    /// Плотность ползунка; ступени урезают её своими потолками.
    pub density: f32,
    /// Слитых кусков крон — на все ступени [`CrownDetail::Merged`] вместе.
    pub chunks: usize,
    /// Вершин во всех слитых кусках крон.
    pub merged_vertices: usize,
    /// Время сборки — её платит правка стиля, а не пересечение порога зума.
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for TreeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            crowns,
            steps,
            shadow_vertices,
            shape,
            density,
            chunks,
            merged_vertices,
            elapsed,
        } = self;
        write!(
            f,
            "tree shadows: {shadow_vertices} vertices, trees per zoom step {steps:?} \
             ({shape:?}, density {density}), {crowns} crowns as entities, \
             crowns merged: {merged_vertices} vertices in {chunks} chunks in {elapsed:.1?}"
        )
    }
}

/// Как ступень зума рисует кроны.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrownDetail {
    /// Полная крона (заливка, контур, кольца — около тысячи вершин у облачной)
    /// **сущностью на дерево** над общим мешем варианта: вершины одни на
    /// вариант, дерево — это трансформ и материал оттенка.
    Full,
    /// Дальняя крона ([`CrownVariant::far`], 32–48 вершин, средний цвет) —
    /// **слитыми мешами по кускам карты** ([`CROWN_CHUNK`]), без единой
    /// сущности на дерево. На этих зумах штрихи — доли пикселя, а сущность
    /// на крону стоила видимости, извлечения, очереди и сортировки каждый
    /// кадр и спавна на каждой пересборке.
    Merged,
}

/// Ступень зум-LOD деревьев: до какого зума (метров на логический пиксель)
/// она действует, какой плотностью посадки ограничена и как рисует кроны.
pub struct TreeLod {
    pub max_zoom: f32,
    /// Потолок `TreeStyle::density` на этой ступени; `INFINITY` — без потолка.
    pub density_cap: f32,
    pub detail: CrownDetail,
}

/// Ступени деревьев. Крона в 2–6 м радиусом на зуме от 2 м/px — это 1–3 px
/// радиусом, на полном отдалении (4.5) — меньше полутора: лес там читается
/// заливкой `Wood` под ним, а не отдельными кронами. Дальние ступени
/// **урезают префикс** набора ([`TreeSet::visible`]), так что стоящие деревья
/// не переезжают, а одиночные деревья OSM (порог 0) остаются на любой
/// ступени, — и рисуют кроны слитыми дальними ([`CrownDetail::Merged`]).
///
/// Ближняя ступень без потолка, с полными кронами-сущностями, и доходит до
/// 2 м/px: дефолтный вид (0.4) и всё, на чём кроны различимы, рисуются как
/// раньше.
pub const TREE_LODS: [TreeLod; 3] = [
    TreeLod {
        max_zoom: 2.0,
        density_cap: f32::INFINITY,
        detail: CrownDetail::Full,
    },
    TreeLod {
        max_zoom: 3.5,
        density_cap: 3.0,
        detail: CrownDetail::Merged,
    },
    TreeLod {
        max_zoom: f32::INFINITY,
        density_cap: 2.0,
        detail: CrownDetail::Merged,
    },
];

/// Сторона куска карты, в который сливаются дальние кроны, в метрах.
///
/// Слитые куски нужны не ради слитности как таковой, а чтобы отсечение по
/// кадру ещё работало: меш на весь лес виден всегда. Дальние ступени
/// начинаются с 2 м/px, где окно 1600 px — это уже 3.2 км по ширине, почти
/// половина карты (7.6 км); километровый кусок — треть такого кадра, так что
/// сдвинутый в сторону кадр отбрасывает целые куски, а кусков на всю карту —
/// не больше 8 × 6 = 48, то есть 48 draw против ~120 групп крон-сущностей.
/// Мельче кусок — больше draw при том же полном отдалении, где отсекать
/// нечего; крупнее — отсечение перестаёт что-либо отсекать.
pub const CROWN_CHUNK: f32 = 1000.0;

/// Шаг z между слитыми кусками крон. У каждого куска свой z: кроны у границы
/// двух кусков перекрываются, и равный z отдал бы порядок на откуп фазе
/// `Transparent2d` — ловушка мигающих теней (см. `TreeMeshes::shadows`).
/// Куски режутся ещё и по полосам плотности (не больше трёх), так что шаг —
/// 1/256: 3 × 63 куска укладываются в `Z_TREE..Z_TREE + 1`.
const CROWN_CHUNK_Z_STEP: f32 = 1.0 / 256.0;

/// [`TREE_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`).
pub enum TreeLods {}

impl ZoomLods for TreeLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        TREE_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`TREE_LODS`]; пересечение порога пересобирает кроны и
/// переключает видимость теней.
pub type TreeZoomBucket = ZoomBucket<TreeLods>;

/// Шаг z между слоями теней деревьев: полос плотности три, и у каждого слоя
/// свой z — равный z отдал бы их порядок сортировке `Transparent2d` (мигание,
/// см. `TreeMeshes::shadows`).
const TREE_SHADOW_Z_STEP: f32 = 1.0 / 1024.0;

/// Сколько деревьев набора рисует каждая ступень [`TREE_LODS`]: префикс по
/// плотности ползунка, урезанной потолком ступени. От ближней ступени к дальней
/// не растёт — потолки убывают, — так что деревья дальней ступени это
/// начало деревьев ближней.
pub fn step_counts(style: &TreeStyle, planted: &TreeSet) -> [usize; TREE_LODS.len()] {
    TREE_LODS.map(|lod| planted.visible_count(style.density.min(lod.density_cap)))
}

/// Полосы плотности: полоса `b` — деревья с `counts[b + 1]` по `counts[b]`
/// (за последней ступенью — с нуля), и видна она на ступенях `0..=b`: ровно
/// там, где префикс ступени её накрывает. Порядок — от первых деревьев набора
/// к последним, то есть с последней полосы.
fn density_bands(
    counts: &[usize; TREE_LODS.len()],
) -> impl Iterator<Item = (std::ops::Range<usize>, TreeLodMask)> + '_ {
    (0..counts.len()).rev().map(|band| {
        let from = counts.get(band + 1).copied().unwrap_or(0);
        (from..counts[band], TreeLodMask::of(0..=band))
    })
}

/// Пулы вариантов крон по конкретным формам стиля и выбор варианта дереву.
struct Pools<'a> {
    shapes: &'static [TreeShape],
    variants: Vec<Vec<CrownVariant>>,
    style: &'a TreeStyle,
    field: &'a ConiferField,
}

impl<'a> Pools<'a> {
    /// По пулу вариантов на каждую конкретную форму — у `Mixed` их два.
    fn new(style: &'a TreeStyle, params: &CrownParams, field: &'a ConiferField) -> Self {
        let shapes = style.shape.crown_shapes();
        let variants = shapes
            .iter()
            .map(|&shape| {
                (0..TREE_VARIANTS)
                    .map(|variant| crown_variant(shape, variant, style, params))
                    .collect()
            })
            .collect();
        Self {
            shapes,
            variants,
            style,
            field,
        }
    }

    /// Пул и вариант дерева `index`: форма — по полю хвои, вариант — по номеру.
    fn pick(&self, index: usize) -> (usize, usize) {
        let shape = self.style.shape.resolve(self.field.is_conifer(index));
        let pool = self
            .shapes
            .iter()
            .position(|&pooled| pooled == shape)
            .expect("crown_shapes covers every shape resolve can return");
        (pool, index % self.variants[pool].len())
    }

    fn variant(&self, (pool, variant): (usize, usize)) -> &CrownVariant {
        &self.variants[pool][variant]
    }
}

/// Полосы плотности ([`density_bands`]), разложенные по подробности ступеней:
/// для каждой полосы и каждой подробности, которую рисует хоть одна её
/// ступень, — диапазон деревьев и маска именно этих ступеней. Порядок — от
/// первых деревьев набора к последним.
fn detailed_bands(
    counts: &[usize; TREE_LODS.len()],
) -> Vec<(std::ops::Range<usize>, TreeLodMask, CrownDetail)> {
    density_bands(counts)
        .flat_map(|(range, band)| {
            [CrownDetail::Full, CrownDetail::Merged]
                .into_iter()
                .filter_map(move |detail| {
                    let shows = TreeLodMask::of(
                        (0..TREE_LODS.len())
                            .filter(|&step| band.shows(step) && TREE_LODS[step].detail == detail),
                    );
                    (!shows.is_empty()).then(|| (range.clone(), shows, detail))
                })
        })
        .collect()
}

/// Сборка деревьев без мира, **на все ступени зума сразу**:
/// `TREE_VARIANTS` крон единичного радиуса на каждую конкретную форму, каждому
/// дереву — вариант, оттенок и масштаб детерминированно по индексу; ползунок
/// плотности отдаёт префикс набора (см. [`TreeSet::visible_count`]), а каждая
/// ступень урезает его своим потолком ([`TREE_LODS`], [`step_counts`]).
///
/// Всё собранное — кроны-сущности ближней ступени, слитые куски крон дальних
/// ([`CrownDetail`]), тени — разложено по полосам плотности и несёт маску
/// ступеней ([`TreeLodMask`]): смена ступени ничего не собирает, а только
/// прячет и показывает ([`show_tree_lod`]).
pub fn mesh_trees(
    style: &TreeStyle,
    params: &CrownParams,
    planted: &TreeSet,
    field: &ConiferField,
) -> (TreeMeshes, TreeReport) {
    let started = std::time::Instant::now();
    let pools = Pools::new(style, params, field);
    let counts = step_counts(style, planted);
    let bands = detailed_bands(&counts);
    let total = counts.iter().copied().max().unwrap_or(0);
    let trees = &planted.visible(f32::INFINITY)[..total];

    let shadows = shadow_layers(&pools, trees, &bands);
    let tint_factors = style.tint_factors();
    let tint_slots = TreeStyle::TINT_BELL.len();
    let mut crowns = Vec::new();
    // сколько крон уже стоит в каждой группе — их ранг внутри полосы группы
    let mut ranks = vec![0_usize; pools.shapes.len() * TREE_VARIANTS * tint_slots];
    let mut merged_chunks: Vec<(TreeLodMask, MeshBuilder)> = Vec::new();
    for (range, shows, detail) in &bands {
        match detail {
            CrownDetail::Full => {
                for index in range.clone() {
                    let (at, radius) = trees[index];
                    let (pool, variant) = pools.pick(index);
                    let tint = TreeStyle::tint_slot(index);
                    let group = (pool * TREE_VARIANTS + variant) * tint_slots + tint;
                    let rank = ranks[group];
                    ranks[group] += 1;
                    crowns.push(CrownPlacement {
                        at,
                        radius,
                        z: crown_z(group, rank),
                        shows: *shows,
                        pool,
                        variant,
                        tint,
                    });
                }
            }
            CrownDetail::Merged => {
                // слитые куски по ключу клетки [`CROWN_CHUNK`]; кроны ложатся в
                // кусок в порядке набора, и порядок треугольников в меше — это
                // порядок рисования: крона с бо́льшим номером лежит поверх
                let mut chunks: HashMap<IVec2, MeshBuilder> = HashMap::new();
                for index in range.clone() {
                    let (at, radius) = trees[index];
                    let tint = TreeStyle::tint_slot(index);
                    chunks
                        .entry(chunk_of(at))
                        .or_insert_with(MeshBuilder::with_crown_coords)
                        .push_crown(
                            &pools.variant(pools.pick(index)).far,
                            at,
                            radius,
                            tint_factors[tint],
                        );
                }
                merged_chunks.extend(
                    sorted_chunks(chunks)
                        .into_iter()
                        .map(|builder| (*shows, builder)),
                );
            }
        }
    }
    // полосы идут от первых деревьев набора, так что z растёт вместе с номером
    // дерева и между полосами: полоса дальних деревьев лежит поверх
    let merged: Vec<TreeLayer> = merged_chunks
        .into_iter()
        .enumerate()
        .map(|(ordinal, (shows, builder))| TreeLayer {
            shows,
            layer: LayerMesh::new(
                builder,
                Z_TREE + ordinal as f32 * CROWN_CHUNK_Z_STEP,
                "tree_crowns",
                MaterialSpec::Crown,
            ),
        })
        .collect();

    let report = TreeReport {
        crowns: crowns.len(),
        steps: counts,
        shadow_vertices: shadows
            .iter()
            .map(|shadow| shadow.layer.builder.vertex_count())
            .sum(),
        shape: style.shape,
        density: style.density,
        chunks: merged.len(),
        merged_vertices: merged
            .iter()
            .map(|merged| merged.layer.builder.vertex_count())
            .sum(),
        elapsed: started.elapsed(),
    };
    let pools = if crowns.is_empty() {
        Vec::new()
    } else {
        pools
            .variants
            .into_iter()
            .map(|pool| pool.into_iter().map(|built| built.crown).collect())
            .collect()
    };
    let built = TreeMeshes {
        pools,
        // множитель яркости кроны-сущности — в юниформе её материала: меш
        // один на вариант. У слитых крон он уже запечён в вершины
        tints: tint_factors.to_vec(),
        crowns,
        merged,
        shadows,
    };
    (built, report)
}

/// Тени деревьев на все ступени сразу: по слитому слою на полосу плотности
/// ([`density_bands`]), тени в слое — в порядке набора. Пустую полосу (ползунок
/// ниже потолка ступени) отбрасывает уже `spawn_layer`.
fn shadow_layers(
    pools: &Pools,
    trees: &[(Vec2, f32)],
    bands: &[(std::ops::Range<usize>, TreeLodMask, CrownDetail)],
) -> Vec<TreeLayer> {
    // шаблон тени — по подробности ступени: полный на ближней, прореженный
    // ([`CrownVariant::far_shadow`]) на дальних, где силуэт — пара пикселей
    bands
        .iter()
        .cloned()
        .flat_map(|(range, shows, detail)| {
            // по куску карты [`CROWN_CHUNK`] на слой, как у слитых крон: слой на
            // весь лес виден всегда, а кусок вне кадра отсекается целиком.
            // Тень дерева лежит в куске его ствола, так что на границе кусков
            // ничего не дублируется, а перекрытие теней соседних кусков темнит
            // ровно как внутри одного меша — цвет у всех теней один
            let mut chunks: HashMap<IVec2, MeshBuilder> = HashMap::new();
            for index in range {
                let (at, radius) = trees[index];
                let variant = pools.variant(pools.pick(index));
                let template = match detail {
                    CrownDetail::Full => &variant.shadow,
                    CrownDetail::Merged => &variant.far_shadow,
                };
                chunks
                    .entry(chunk_of(at))
                    .or_default()
                    .push_template(template, at, radius);
            }
            sorted_chunks(chunks)
                .into_iter()
                .map(move |builder| (shows, builder))
        })
        .enumerate()
        .map(|(ordinal, (shows, builder))| TreeLayer {
            shows,
            layer: LayerMesh::new(
                builder,
                Z_TREE_SHADOW + ordinal as f32 * TREE_SHADOW_Z_STEP,
                "tree_shadows",
                MaterialSpec::Blend,
            ),
        })
        .collect()
}

/// Кусок карты [`CROWN_CHUNK`], в котором стоит ствол.
fn chunk_of(at: Vec2) -> IVec2 {
    (at / CROWN_CHUNK).floor().as_ivec2()
}

/// Куски в порядке их места на карте (снизу вверх, слева направо), а не в
/// порядке обхода словаря: так z кусков не зависит ни от хэшера, ни от набора.
fn sorted_chunks(mut chunks: HashMap<IVec2, MeshBuilder>) -> Vec<MeshBuilder> {
    let mut keys: Vec<IVec2> = chunks.keys().copied().collect();
    keys.sort_by_key(|key| (key.y, key.x));
    keys.into_iter()
        .map(|key| chunks.remove(&key).expect("key came from the map"))
        .collect()
}

/// Полоса z одной группы крон — одного меша (пул × вариант) под одним
/// материалом (оттенок). Групп не больше `2 · TREE_VARIANTS · 5` = 120, так
/// что все полосы укладываются в `Z_TREE..Z_TREE + 1` и под
/// `Z_CONIFER_NOISE_OVERLAY`.
const CROWN_GROUP_Z_STEP: f32 = 1.0 / 128.0;
/// Микрошаг кроны внутри полосы своей группы: два ulp у `f32` около 20
/// (2⁻¹⁹ каждый), то есть 2048 различимых мест на полосу.
const CROWN_RANK_Z_STEP: f32 = 1.0 / 262_144.0;
const CROWN_RANKS_PER_GROUP: usize = (CROWN_GROUP_Z_STEP / CROWN_RANK_Z_STEP) as usize;

/// z кроны-сущности ([`CrownDetail::Full`]): полоса её группы плюс микрошаг по
/// рангу внутри группы. Слитые кроны дальних ступеней обходятся без него —
/// у них z на кусок, а порядок внутри куска — порядок треугольников.
///
/// **z группирует кроны по мешу и материалу, а не идёт по номеру дерева.**
/// Прозрачная фаза 2D (`Transparent2d`) сортирует только по z, а порядок
/// видимых сущностей до сортировки собирается параллельно и кусками — поэтому
/// соседями в фазе оказываются лишь элементы с близким z, и только общий
/// диапазон z кладёт одинаковые меш + материал подряд, а bevy сливает подряд
/// идущие одинаковые элементы в один draw. С микрошагом по номеру дерева
/// (`index % 512`) соседние по z кроны почти всегда были разными вариантами:
/// ~16 тыс. draw на Туле при полном отдалении; по группам — ~120. Замер
/// (без vsync, пауза, полное отдаление): Тула 17.6 → 15.9 мс кадр, Калуга
/// 58.8 → 28.6.
///
/// Цена — перекрытие двух крон разных групп теперь решает номер группы, а не
/// номер дерева; для крон одной группы микрошаг по рангу по-прежнему даёт
/// стабильный порядок.
fn crown_z(group: usize, rank: usize) -> f32 {
    Z_TREE
        + group as f32 * CROWN_GROUP_Z_STEP
        + (rank % CROWN_RANKS_PER_GROUP) as f32 * CROWN_RANK_Z_STEP
}

/// Собранные деревья — в мир: пул крон в `Assets`, слитые куски крон
/// (дальние ступени) и слои теней — через общий [`spawn_layers`], всё видимым
/// или спрятанным по своей маске и ступени `bucket`; дальше видимость ведёт
/// [`show_tree_lod`].
///
/// Кроны-сущности ближней ступени встают сразу, только если `bucket` их
/// рисует; иначе их места уходят в возвращённый [`CrownStream`], и в мир их
/// досыпает [`stream_tree_crowns`], когда зум до них дойдёт. Игра кладёт этот
/// поток в ресурс; витрине, которая смотрит вблизи, он не нужен.
pub fn spawn_tree_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut TreeMaterials,
    bucket: TreeZoomBucket,
    (built, report): (TreeMeshes, TreeReport),
) -> CrownStream {
    let TreeMeshes {
        pools,
        tints,
        crowns,
        merged,
        shadows,
    } = built;
    spawn_masked(commands, meshes, &materials.layers, bucket, merged);
    // материалы оттенков — только кронам-сущностям: слитым хватает общего
    let tints: Vec<Handle<CrownMaterial>> = if crowns.is_empty() {
        Vec::new()
    } else {
        tints
            .into_iter()
            .map(|factor| materials.crowns.add(CrownMaterial::of(factor)))
            .collect()
    };
    let pools: Vec<Vec<Handle<Mesh>>> = pools
        .into_iter()
        .map(|pool| pool.into_iter().map(|crown| meshes.add(crown)).collect())
        .collect();

    let mut stream = CrownStream {
        placements: crowns,
        pools,
        tints,
        entities: Vec::new(),
    };
    // на входе в мир (и на правке стиля) кроны нужной ступени встают сразу:
    // экран загрузки прикрывает цену, а досыпать кадрами было бы видно
    if stream.wanted_at(bucket.index) {
        stream.spawn_next(commands, usize::MAX, bucket.index);
    }

    // веер хвои весит вчетверо против одиночного силуэта: при разборе просадок
    // смотреть в первую очередь сюда
    debug!("{report}");
    spawn_masked(commands, meshes, &materials.layers, bucket, shadows);
    stream
}

/// Сколько крон-сущностей встаёт в мир за кадр на пути к ближней ступени.
/// Спавн разом стоил на Калуге (95 тыс. крон) секунды кадра под App Nap —
/// три-четыре тяжёлых кадра и без неё; пачкой в 4096 это 24 кадра по доле
/// той цены, а слитые кроны дальней ступени стоят до конца досыпки.
const CROWN_SPAWN_BATCH: usize = 4096;
/// Сколько спрятанных крон-сущностей уходит из мира за кадр на дальних
/// ступенях. Спрятанная сущность не бесплатна: её каждый кадр обходят
/// видимость, трансформы и извлечение рендера — 95 тыс. спрятанных крон
/// Калуги стоили на полном отдалении +8…15 мс `PostUpdate` (замер A/B одного
/// бинарника, экран заблокирован), и держать их там незачем.
const CROWN_DESPAWN_BATCH: usize = 16384;

/// Кроны-сущности ближней ступени, которые адаптер держит наготове: места,
/// хэндлы пула и материалов оттенков и уже стоящие сущности — **префикс**
/// мест, в том же порядке.
///
/// Ни спавн всех крон на пересечении порога, ни держать их спрятанными на
/// дальних ступенях не годятся: первое — секунда кадра на Калуге, второе —
/// постоянная цена каждого кадра полного отдаления. Поэтому на ближнюю ступень
/// кроны досыпаются пачками ([`CROWN_SPAWN_BATCH`]) спрятанными, а
/// показывается ступень ([`TreeLodShown`]) только когда встали все; на дальних
/// они сначала прячутся, потом уходят пачками ([`CROWN_DESPAWN_BATCH`]).
/// Хэндлы держат пул и материалы живыми, пока сущностей нет.
#[derive(Resource, Default)]
pub struct CrownStream {
    placements: Vec<CrownPlacement>,
    pools: Vec<Vec<Handle<Mesh>>>,
    tints: Vec<Handle<CrownMaterial>>,
    entities: Vec<Entity>,
}

impl CrownStream {
    /// Нужны ли кроны-сущности на ступени `step`. Маска у всех крон одна —
    /// ступени [`CrownDetail::Full`], — так что хватает первой.
    fn wanted_at(&self, step: usize) -> bool {
        self.placements
            .first()
            .is_some_and(|crown| crown.shows.shows(step))
    }

    /// Все ли кроны стоят в мире.
    fn complete(&self) -> bool {
        self.entities.len() == self.placements.len()
    }

    /// Поставить следующие `batch` крон; видимость — по ступени `shown`.
    fn spawn_next(&mut self, commands: &mut Commands, batch: usize, shown: usize) {
        let from = self.entities.len();
        let to = self.placements.len().min(from.saturating_add(batch));
        for crown in &self.placements[from..to] {
            let entity = commands
                .spawn((
                    TreeTag,
                    crown.shows,
                    crown.shows.visibility(shown),
                    Mesh2d(self.pools[crown.pool][crown.variant].clone()),
                    MeshMaterial2d(self.tints[crown.tint].clone()),
                    Transform::from_translation(crown.at.extend(crown.z))
                        .with_scale(Vec3::splat(crown.radius)),
                    DespawnOnExit(AppState::Playing),
                    Name::new("tree"),
                ))
                .id();
            self.entities.push(entity);
        }
    }

    /// Убрать из мира до `batch` последних стоящих крон.
    fn despawn_last(&mut self, commands: &mut Commands, batch: usize) {
        let keep = self.entities.len().saturating_sub(batch);
        for entity in self.entities.drain(keep..) {
            commands.entity(entity).despawn();
        }
    }
}

/// Ступень [`TREE_LODS`], которую деревья **показывают** — по ней
/// [`show_tree_lod`] ставит видимость. Совпадает с [`TreeZoomBucket`], кроме
/// одного случая: на пути к ближней ступени, пока кроны-сущности досыпаются
/// ([`CrownStream`]), показывается прежняя дальняя.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct TreeLodShown(pub usize);

/// Досыпка и уборка крон-сущностей по ступени зума — каждый кадр, дёшево,
/// когда делать нечего. См. [`CrownStream`].
pub fn stream_tree_crowns(
    mut commands: Commands,
    bucket: Res<TreeZoomBucket>,
    mut stream: ResMut<CrownStream>,
    mut shown: ResMut<TreeLodShown>,
) {
    if stream.wanted_at(bucket.index) {
        if stream.complete() {
            shown.set_if_neq(TreeLodShown(bucket.index));
        } else {
            // пока досыпаются — видна прежняя ступень, новые кроны спрятаны
            let showing = shown.0;
            stream.spawn_next(&mut commands, CROWN_SPAWN_BATCH, showing);
            if stream.complete() {
                shown.set_if_neq(TreeLodShown(bucket.index));
            }
        }
    } else {
        // дальняя ступень показывается сразу: кроны-сущности прячутся в этом
        // же кадре, а уходят из мира пачками в следующих
        shown.set_if_neq(TreeLodShown(bucket.index));
        if !stream.entities.is_empty() {
            stream.despawn_last(&mut commands, CROWN_DESPAWN_BATCH);
        }
    }
}

/// Слои с маской ступеней — в мир, видимыми или спрятанными по `bucket`.
fn spawn_masked(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    bucket: TreeZoomBucket,
    layers: Vec<TreeLayer>,
) {
    for TreeLayer { shows, layer } in layers {
        spawn_layers(
            commands,
            meshes,
            materials,
            [layer],
            (TreeTag, shows, shows.visibility(bucket.index)),
        );
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
    field.resample(map.trees.positions(), &noise, style.noise_mix);
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
    field.resample(map.trees.positions(), &noise, style.noise_mix);
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
    field.resample(map.trees.positions(), &noise, style.noise_mix);
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
///
/// Ступени зума ([`TreeZoomBucket`]) здесь **нет**: её смена не трогает ни
/// набор, ни поле, ни подложку аллей, ни тени — тени собраны на все ступени
/// сразу, — и идёт своими системами, [`stream_tree_crowns`] и
/// [`show_tree_lod`].
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<TreeStyle>
        .or_else(retuned::<TreeRowStyle>)
        .or_else(retuned::<ConiferNoiseStyle>)
        .or_else(retuned::<SunOnMap>)
}

/// Смена показанной ступени деревьев ([`TreeLodShown`]): только видимость.
/// Слитые куски крон и тени собраны на все ступени сразу ([`mesh_trees`]),
/// кроны-сущности досыпает [`stream_tree_crowns`], и всё несёт маску ступеней.
///
/// Раньше ступень была условием всей связки [`rebuilds_on`]: пересечение
/// порога заново строило и заливало весь меш теней (Калуга 10–15 млн вершин,
/// ~600 МБ), а вход на ближнюю ступень спавнил разом все кроны-сущности
/// (Тула 16 тыс., Калуга 95 тыс. — три-четыре тяжёлых кадра).
pub fn show_tree_lod(shown: Res<TreeLodShown>, mut masked: Query<(&TreeLodMask, &mut Visibility)>) {
    masked.par_iter_mut().for_each(|(shows, mut visibility)| {
        visibility.set_if_neq(shows.visibility(shown.0));
    });
}

/// Когда менять видимость деревьев — одно условие, одна регистрация.
pub fn switches_on() -> impl SystemCondition<()> {
    IntoSystem::into_system(retuned::<TreeLodShown>)
}

/// Пересборка деревьев после правки стиля из UI: деспавн старых сущностей и
/// повторный спавн из тех же позиций (`MapData::trees` не трогается).
pub fn rebuild_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: TreeMaterials,
    // парой — иначе подпись переваливает за предел clippy в семь аргументов
    (style, bucket): (Res<TreeStyle>, Res<TreeZoomBucket>),
    (map, mut field): (Res<MapData>, ResMut<ConiferField>),
    (mut stream, mut shown): (ResMut<CrownStream>, ResMut<TreeLodShown>),
    existing: Query<Entity, With<TreeTag>>,
) {
    // порог поля пересчитывается только если поехала сама доля — правка цвета
    // листвы не должна платить за сортировку значений
    field.set_share(style.conifer_share);
    // солнце — одно из условий пересборки: свет слитых крон едет вместе с
    // тенями (материал кроны-сущности заводится заново с каждой сборкой)
    canopy::relight_crown_material(&materials.merged, &mut materials.crowns);
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let built = mesh_trees(
        &style,
        // ручки геометрии кроны в игре не выведены никуда: город рисуется
        // дефолтом, а крутит их витрина `tree_gallery`
        &CrownParams::default(),
        &map.trees,
        &field,
    );
    // кроны ступени, которую видно, встают сразу — показывается она же
    *stream = spawn_tree_meshes(&mut commands, &mut meshes, &mut materials, *bucket, built);
    shown.set_if_neq(TreeLodShown(bucket.index));
}

#[cfg(test)]
mod tests;
