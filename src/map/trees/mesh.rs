//! Меш-сборка деревьев и её адаптер в мир: на все ступени зума сразу — пул
//! крон, места крон-сущностей, слитые куски крон и слои теней
//! ([`mesh_trees`], [`TreeMeshes`]); [`spawn_tree_meshes`] ставит собранное
//! в мир и отдаёт кроны-сущности ближней ступени потоку [`CrownStream`],
//! которого каждый кадр досыпает [`stream_tree_crowns`]. Когда пересобирать
//! и что показывать — решает родитель: [`super::rebuilds_on`],
//! [`super::show_tree_lod`].

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::{
    CROWN_CHUNK, ConiferField, CrownDetail, CrownMaterial, CrownParams, CrownVariant, TREE_LODS,
    TreeLayer, TreeLodMask, TreeMaterials, TreeReport, TreeShape, TreeStyle, TreeTag,
    TreeZoomBucket, crown_variant, step_counts,
};
use crate::loading::AppState;
use crate::map::meshing::{MeshBuilder, sorted_by_cell};
use crate::map::osm::model::TreeSet;
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::z::{Z_TREE, Z_TREE_SHADOW};
use crate::settings::TREE_VARIANTS;

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
#[derive(Default)]
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

/// Шаг z между слитыми кусками крон. У каждого куска свой z: кроны у границы
/// двух кусков перекрываются, и равный z отдал бы порядок на откуп фазе
/// `Transparent2d` — ловушка мигающих теней (см. `TreeMeshes::shadows`).
/// Куски режутся ещё и по полосам плотности (не больше трёх), так что шаг —
/// 1/256: 3 × 63 куска укладываются в `Z_TREE..Z_TREE + 1`.
pub(in crate::map::trees) const CROWN_CHUNK_Z_STEP: f32 = 1.0 / 256.0;

/// Шаг z между слоями теней деревьев. Слой — полоса плотности × подробность
/// шаблона × кусок карты [`CROWN_CHUNK`], и у каждого свой z — равный z отдал
/// бы их порядок сортировке `Transparent2d` (мигание, см.
/// `TreeMeshes::shadows`).
pub(in crate::map::trees) const TREE_SHADOW_Z_STEP: f32 = 1.0 / 1024.0;

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
/// прячет и показывает ([`super::show_tree_lod`]).
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
                let chunks = sorted_chunks(
                    range.clone(),
                    trees,
                    MeshBuilder::with_crown_coords,
                    |chunk, index, at, radius| {
                        let far = &pools.variant(pools.pick(index)).far;
                        let tint = tint_factors[TreeStyle::tint_slot(index)];
                        chunk.push_crown(far, at, radius, tint);
                    },
                );
                merged_chunks.extend(chunks.into_iter().map(|builder| (*shows, builder)));
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
            sorted_chunks(
                range,
                trees,
                MeshBuilder::default,
                |chunk, index, at, radius| {
                    let variant = pools.variant(pools.pick(index));
                    let template = match detail {
                        CrownDetail::Full => &variant.shadow,
                        CrownDetail::Merged => &variant.far_shadow,
                    };
                    chunk.push_template(template, at, radius);
                },
            )
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

/// Деревья `range` по кускам карты [`CROWN_CHUNK`] их стволов — одна раскладка
/// на слитые кроны и на тени: `push` кладёт дерево (номер, ствол, радиус) в
/// сборщик его куска, свежий кусок берётся из `new`. Внутри куска деревья идут
/// в порядке набора, а сами куски — в порядке их места на карте (снизу вверх,
/// слева направо), а не обхода словаря: так z кусков не зависит ни от хэшера,
/// ни от набора.
fn sorted_chunks(
    range: std::ops::Range<usize>,
    trees: &[(Vec2, f32)],
    new: fn() -> MeshBuilder,
    mut push: impl FnMut(&mut MeshBuilder, usize, Vec2, f32),
) -> Vec<MeshBuilder> {
    let mut chunks: HashMap<IVec2, MeshBuilder> = HashMap::new();
    for index in range {
        let (at, radius) = trees[index];
        let chunk = chunks.entry(chunk_of(at)).or_insert_with(new);
        push(chunk, index, at, radius);
    }
    sorted_by_cell(chunks)
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

/// Сборка деревьев с игровыми ручками кроны — потоку загрузки
/// (`map/prepare.rs`): ручки геометрии кроны в игре не выведены никуда,
/// город рисуется дефолтом, крутит их витрина `tree_gallery`.
pub(crate) fn mesh_game_trees(
    style: &TreeStyle,
    planted: &TreeSet,
    field: &ConiferField,
) -> (TreeMeshes, TreeReport) {
    mesh_trees(style, &CrownParams::default(), planted, field)
}

/// Собранные деревья — в мир: пул крон в `Assets`, слитые куски крон
/// (дальние ступени) и слои теней — через общий [`spawn_layers`], всё видимым
/// или спрятанным по своей маске и ступени `bucket`; дальше видимость ведёт
/// [`super::show_tree_lod`].
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
pub(in crate::map::trees) const CROWN_SPAWN_BATCH: usize = 4096;
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
    pub(in crate::map::trees) placements: Vec<CrownPlacement>,
    pub(in crate::map::trees) pools: Vec<Vec<Handle<Mesh>>>,
    pub(in crate::map::trees) tints: Vec<Handle<CrownMaterial>>,
    pub(in crate::map::trees) entities: Vec<Entity>,
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
    /// Возвращает, сколько встало.
    fn spawn_next(&mut self, commands: &mut Commands, batch: usize, shown: usize) -> usize {
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
        to - from
    }

    /// Убрать из мира до `batch` последних стоящих крон. Возвращает, сколько
    /// ушло.
    fn despawn_last(&mut self, commands: &mut Commands, batch: usize) -> usize {
        let keep = self.entities.len().saturating_sub(batch);
        let removed = self.entities.len() - keep;
        for entity in self.entities.drain(keep..) {
            commands.entity(entity).despawn();
        }
        removed
    }
}

/// Ступень [`TREE_LODS`], которую деревья **показывают** — по ней
/// [`super::show_tree_lod`] ставит видимость. Совпадает с [`TreeZoomBucket`],
/// кроме одного случая: на пути к ближней ступени, пока кроны-сущности
/// досыпаются ([`CrownStream`]), показывается прежняя дальняя.
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
            let added = stream.spawn_next(&mut commands, CROWN_SPAWN_BATCH, showing);
            debug!(
                "crown stream: +{added} = {} of {} standing",
                stream.entities.len(),
                stream.placements.len()
            );
            if stream.complete() {
                debug!("crown stream: complete, showing step {}", bucket.index);
                shown.set_if_neq(TreeLodShown(bucket.index));
            }
        }
    } else {
        // дальняя ступень показывается сразу: кроны-сущности прячутся в этом
        // же кадре, а уходят из мира пачками в следующих
        shown.set_if_neq(TreeLodShown(bucket.index));
        if !stream.entities.is_empty() {
            let removed = stream.despawn_last(&mut commands, CROWN_DESPAWN_BATCH);
            debug!("crown stream: -{removed} = {} left", stream.entities.len());
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
