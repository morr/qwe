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
mod mesh;

pub use self::canopy::{CrownMaterial, CrownMaterialHandle, init_crown_material};

use self::crown::{
    CROWN_COLOR, INK_COLOR, crown_builder, crown_geometry, far_crown, shadow_templates, variant_rng,
};
use crate::map::SunOnMap;
use crate::map::TREE_DENSITY_MAX;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::TreeSet;
use crate::map::osm::{MapData, TreeRowLayout, TreeRowPlacement};
use crate::map::prepare::PreparedMeshes;
use crate::map::roads::RoadJoin;
use crate::map::smooth::Smoothing;
use crate::map::surface::{LayerMaterials, LayerMesh, MapLayer};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

// Состав набора и поле хвои — в `trees/conifer.rs`; ре-экспорты держат
// внешние пути (`crate::map::trees::…`) прежними.
pub(crate) use self::conifer::tree_compose;
pub use self::conifer::{
    ConiferField, ConiferNoiseStyle, build_conifer_field, recompose_row_trees, resample_field,
    retune_conifer_field,
};
// Приватные реэкспорты: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать геометрию.
pub use self::crown::CrownParams;
#[cfg(test)]
use self::crown::crown_mesh;
#[cfg(test)]
use self::crown::shadow_template;
// Меш-сборка и поток крон — `trees/mesh.rs`; те же ре-экспорты ради тех же
// внешних путей.
pub(crate) use self::mesh::mesh_game_trees;
pub use self::mesh::{
    CrownPlacement, CrownStream, TreeLodShown, TreeMeshes, mesh_trees, spawn_tree_meshes,
    stream_tree_crowns,
};

/// Форма кроны — `w.TREE_SHAPE` у watabou.
///
/// Не ресурс: это тип поля [`TreeStyle::shape`]. В реестре типов он нужен,
/// чтобы `res set TreeStyle .shape` дешериализовал вариант по имени.
/// `Resource` стоял тут от прежней формы, где форма кроны жила отдельным
/// ресурсом, — в мир он никогда не вставлялся, и `res get TreeShape` отвечал
/// «Resource not registered» при живой регистрации типа.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
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
pub(crate) const TREE_DENSITY_DEFAULT: f32 = 4.0;

/// Границы и шаг ползунка доли хвои (`TreeStyle::conifer_share`) при форме
/// `Mixed`. Доля точная: порог поля берётся квантилем, а не фиксированным
/// уровнем шума, — 0 даёт лес без хвои, 1 — только хвою.
pub const TREE_CONIFER_SHARE_MIN: f32 = 0.0;
pub const TREE_CONIFER_SHARE_MAX: f32 = 1.0;
pub const TREE_CONIFER_SHARE_STEP: f32 = 0.05;
pub(crate) const TREE_CONIFER_SHARE_DEFAULT: f32 = 0.1;

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
    pub(in crate::map::trees) const TINT_BELL: [f32; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];

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

    pub(in crate::map::trees) fn visibility(self, bucket: usize) -> Visibility {
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

/// Счётчики, которыми была лог-строка слоя.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
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
            "trees: {steps:?} per zoom step ({shape:?}, density {density}), \
             {crowns} crowns as entities, {merged_vertices} merged crown vertices in \
             {chunks} chunks, {shadow_vertices} shadow vertices, built in {elapsed:.1?}"
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
pub(crate) struct TreeLod {
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
/// Ближняя ступень без потолка, с полными кронами-сущностями, и начинается
/// только при действительно близком зуме (0.2 м/px — крона от 10 px радиусом;
/// `MIN_ZOOM` камеры 0.05): крона-сущность стоит видимости и извлечения
/// каждый кадр, а стоят они на **весь город** — на плотном Париже это было
/// 28.6k сущностей на любом зуме до 2 м/px, из которых кадр видел ~200
/// (запись 10). Всё от 0.2 до 3.5 м/px, включая дефолтный вид (0.4), рисуют
/// слитые куски: у ближнего шага остался только тот диапазон, где слитая
/// крона (32-вершинная заливка без контура и колец) выдала бы заметную
/// разницу. Цена — контур, кольца и потолок плотности 3 на диапазоне
/// 0.2–2.0, где раньше потолка не было: при плотности ≤ 3 (дефолт 1.0)
/// ничего не тонет.
pub(crate) const TREE_LODS: [TreeLod; 3] = [
    TreeLod {
        max_zoom: 0.2,
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
/// начинаются с 0.2 м/px, где окно 1600 px — 320 м, и километровый кусок
/// ещё не кулься вовсе; от ~0.6 м/px (кадр ~1 км) отсечение начинает
/// отбрасывать целые куски, а к 2 м/px кадр — уже 3.2 км, почти половина
/// карты (7.6 км), и кусков на всю карту — не больше 8 × 6 = 48. Мельче
/// кусок — больше draw при том же полном отдалении, где отсекать нечего;
/// крупнее — отсечение перестаёт что-либо отсекать. На ближней половине
/// слитого диапазона (0.2–0.6) кусок не кулься — но там вся слитая геометрия
/// леса (у Парижа ~1.5M вершин крон) на порядок легче сущностной цены
/// ближнего шага, которую куски заменили. Равен `surface::MAP_CHUNK`: кроны
/// и слои режутся по одной решётке, и равенство держит assert рядом с той
/// константой.
pub const CROWN_CHUNK: f32 = 1000.0;

/// [`TREE_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`).
pub enum TreeLods {}

impl ZoomLods for TreeLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        TREE_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`TREE_LODS`]; пересечение порога ничего не пересобирает:
/// кроны-сущности досыпает и убирает [`stream_tree_crowns`], а видимость
/// слитых крон и теней переключает [`show_tree_lod`].
pub type TreeZoomBucket = ZoomBucket<TreeLods>;

/// Сколько деревьев набора рисует каждая ступень [`TREE_LODS`]: префикс по
/// плотности ползунка, урезанной потолком ступени. От ближней ступени к дальней
/// не растёт — потолки убывают, — так что деревья дальней ступени это
/// начало деревьев ближней.
pub fn step_counts(style: &TreeStyle, planted: &TreeSet) -> [usize; TREE_LODS.len()] {
    TREE_LODS.map(|lod| planted.visible_count(style.density.min(lod.density_cap)))
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

/// Сколько кадров прогрева слоёв осталось; ноль — прогрева нет.
#[derive(Resource, Default)]
pub(crate) struct LayerWarmup(u32);

/// Начать прогрев слоёв деревьев — в цепочке входа в мир, следом за
/// [`rebuild_trees`], который их заспавнил.
pub(crate) fn begin_layer_warmup(mut warmup: ResMut<LayerWarmup>) {
    warmup.0 = 2;
}

/// Прогрев слитых кусков крон и слоёв теней на входе в мир. Спрятанный при
/// спавне слой рендерится впервые только на первом пересечении порога зума,
/// и тот кадр платит его первую подготовку рендером — на Туле раз на мир
/// 0.9–10 с замирания (замер 10.2026: первый переход 2.0 наружу после
/// открытия на ближней ступени — 8 кадров по 275–2949 мс; второе то же
/// пересечение — в пределах обычной бури). Один кадр показывает всё, второй
/// возвращает видимость по маске — цена уходит под экран загрузки; дальше
/// система выходит сразу. Прогрев ведут только слои-слои ([`MapLayer`]):
/// кроны-сущности при входе на ближнюю ступень и так встают видимыми, а на
/// дальней показываются стримингом, чьи кадры дёшевы (замер: 4 пачки за
/// ~180 мс).
pub(crate) fn warm_layers(
    mut warmup: ResMut<LayerWarmup>,
    shown: Res<TreeLodShown>,
    mut masked: Query<(&TreeLodMask, &mut Visibility), With<MapLayer>>,
) {
    if warmup.0 == 0 {
        return;
    }
    warmup.0 -= 1;
    let warm = warmup.0 == 1;
    masked.par_iter_mut().for_each(|(shows, mut visibility)| {
        visibility.set_if_neq(if warm {
            Visibility::Visible
        } else {
            shows.visibility(shown.0)
        });
    });
}

/// Принять собранные деревья в мир: порог поля хвои под долю, свет слитых
/// крон по свежим теням, спавн и показ ступени. Общий хвост входа в мир
/// ([`spawn_prepared_trees`]) и пересборки после правки стиля
/// ([`rebuild_trees`]); расходятся они только тем, откуда берётся сборка, —
/// поэтому `built` приходит лениво и зовётся после [`ConiferField::set_share`]:
/// пересборка строит по полю хвои и должна видеть его свежий порог.
#[allow(clippy::too_many_arguments)]
fn adopt_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: TreeMaterials,
    share: f32,
    bucket: TreeZoomBucket,
    mut field: ResMut<ConiferField>,
    mut stream: ResMut<CrownStream>,
    mut shown: ResMut<TreeLodShown>,
    built: impl FnOnce(&ConiferField) -> (TreeMeshes, TreeReport),
) {
    // порог поля пересчитывается только если поехала сама доля — правка цвета
    // листвы не должна платить за сортировку значений
    field.set_share(share);
    // свет слитых крон едет вместе с тенями: материал кроны-сущности
    // заводится заново с каждой сборкой
    canopy::relight_crown_material(&materials.merged, &mut materials.crowns);
    let built = built(&field);
    // кроны ступени, которую видно, встают сразу — показывается она же
    *stream = spawn_tree_meshes(&mut commands, &mut meshes, &mut materials, bucket, built);
    shown.set_if_neq(TreeLodShown(bucket.index));
}

/// Деревья из потока загрузки — в мир: общий хвост [`adopt_trees`], сборка
/// отыграла в потоке (`map/prepare.rs`), и в свежем мире деспавнить нечего.
///
/// Ресурс поля хвои здесь уже пересемплирован (`build_conifer_field` в
/// цепочке выше) под тот же набор деревьев и те же параметры, что и у
/// потока: значения совпадают, остаётся порог под долю.
#[allow(clippy::too_many_arguments)]
pub fn spawn_prepared_trees(
    commands: Commands,
    meshes: ResMut<Assets<Mesh>>,
    materials: TreeMaterials,
    style: Res<TreeStyle>,
    bucket: Res<TreeZoomBucket>,
    field: ResMut<ConiferField>,
    stream: ResMut<CrownStream>,
    shown: ResMut<TreeLodShown>,
    mut prepared: ResMut<PreparedMeshes>,
) {
    adopt_trees(
        commands,
        meshes,
        materials,
        style.conifer_share,
        *bucket,
        field,
        stream,
        shown,
        |_| std::mem::take(&mut prepared.0.trees),
    );
}

/// Пересборка деревьев после правки стиля из UI: деспавн старых сущностей и
/// повторный спавн из тех же позиций (`MapData::trees` не трогается).
#[allow(clippy::too_many_arguments)]
pub fn rebuild_trees(
    mut commands: Commands,
    meshes: ResMut<Assets<Mesh>>,
    materials: TreeMaterials,
    style: Res<TreeStyle>,
    bucket: Res<TreeZoomBucket>,
    map: Res<MapData>,
    field: ResMut<ConiferField>,
    stream: ResMut<CrownStream>,
    shown: ResMut<TreeLodShown>,
    existing: Query<Entity, With<TreeTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    adopt_trees(
        commands,
        meshes,
        materials,
        style.conifer_share,
        *bucket,
        field,
        stream,
        shown,
        |field| mesh_game_trees(&style, &map.trees, field),
    );
}

#[cfg(test)]
mod tests;
