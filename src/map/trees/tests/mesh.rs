//! Тесты на `mesh_trees` — слой целиком. До шва деревья спавнились прямо в
//! системе Bevy, и правило расстановки — префикс по плотности, разрешение
//! породы, слот оттенка, микрошаг по z — было недостижимо ни для одного
//! теста: из него нельзя было достать ни одной кроны, не подняв мир.

use super::super::mesh::{CROWN_CHUNK_Z_STEP, TREE_SHADOW_Z_STEP};
use super::super::*;
use super::support::params;
use crate::map::surface::MaterialSpec;
use crate::map::z::{Z_TREE, Z_TREE_SHADOW};
use crate::settings::TREE_VARIANTS;

/// Десять деревьев в ряд, с порогами появления 0, 1, … 9: ползунок плотности
/// режет такой набор ровно по своему значению.
fn ten_trees() -> TreeSet {
    TreeSet::of((0..10).map(|i| {
        (
            Vec2::new(i as f32 * 10.0, 0.0),
            1.0 + i as f32 * 0.1,
            i as f32,
        )
    }))
}

fn mesh_ten(shape: TreeShape, density: f32) -> (TreeMeshes, TreeReport) {
    let style = TreeStyle {
        shape,
        density,
        ..default()
    };
    mesh_trees(&style, &params(), &ten_trees(), &ConiferField::default())
}

/// Дальние ступени зума урезают **префикс** набора до своего потолка
/// плотности: стоящие кроны не переезжают, а ближняя ступень рисует всё, что
/// дал ползунок.
#[test]
fn a_far_zoom_step_trims_the_tail_of_the_set() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_ten(TreeShape::Cotton, 9.0);
    assert_eq!(report.crowns, 10, "ближняя ступень без потолка");
    for (step, lod) in TREE_LODS.iter().enumerate().skip(1) {
        // пороги у `ten_trees` — 0..=9
        assert_eq!(report.steps[step], lod.density_cap as usize + 1);
    }
    // кроны-сущности — только ближней ступени, все десять
    assert!(
        built
            .crowns
            .iter()
            .all(|crown| crown.shows == TreeLodMask::of([0]))
    );
    // префикс: тени дальней ступени — ровно тени первых крон ступени ближе
    // (сравнимы ступени одной подробности: у ближней шаблон тени полный)
    let far = shadows_shown(&built, TREE_LODS.len() - 1);
    let nearer = shadows_shown(&built, TREE_LODS.len() - 2);
    assert!(far.len() < nearer.len());
    assert_eq!(far, &nearer[..far.len()], "кроны переехали");
}

/// Вершины теней, которые видны на ступени `bucket`, в порядке слоёв.
fn shadows_shown(built: &TreeMeshes, bucket: usize) -> Vec<[f32; 3]> {
    built
        .shadows
        .iter()
        .filter(|shadow| shadow.shows.shows(bucket))
        .flat_map(|shadow| shadow.layer.builder.positions_for_test().iter().copied())
        .collect()
}

/// Тени собраны на все ступени сразу: смена ступени их только прячет и
/// показывает ([`show_tree_lod`]), а не пересобирает. Каждая ступень видит
/// ровно тени своего префикса, и каждого дерева — ровно одна на ступень.
#[test]
fn tree_shadows_are_built_once_for_every_zoom_step() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle {
        shape: TreeShape::Cotton,
        density: 9.0,
        ..default()
    };
    let (near, _) = mesh_trees(&style, &params(), &ten_trees(), &ConiferField::default());
    // видимые на ступени вершины — ровно шаблоны теней её префикса
    let counts = step_counts(&style, &ten_trees());
    assert_eq!(counts, [10, 4, 3]);
    for (bucket, lod) in TREE_LODS.iter().enumerate() {
        let shown = shadows_shown(&near, bucket).len();
        let expected: usize = (0..counts[bucket])
            .map(|index| {
                let variant =
                    crown_variant(TreeShape::Cotton, index % TREE_VARIANTS, &style, &params());
                match lod.detail {
                    CrownDetail::Full => variant.shadow.vertex_count(),
                    CrownDetail::Merged => variant.far_shadow.vertex_count(),
                }
            })
            .sum();
        assert_eq!(shown, expected, "ступень {bucket}");
    }
}

/// Тень дальних ступеней — та же тень по прореженному контуру: у облачной
/// кроны она в разы легче полной и по площади от неё почти не отличается, у
/// ели и пальмы (контур не прореживается) совпадает с полной целиком.
#[test]
fn the_far_shadow_is_a_thinned_copy_of_the_full_one() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle::default();
    for shape in TreeShape::CONCRETE {
        for variant in 0..TREE_VARIANTS {
            let built = crown_variant(shape, variant, &style, &params());
            let (full, far) = (&built.shadow, &built.far_shadow);
            if shape == TreeShape::Cotton {
                assert!(
                    far.vertex_count() * 3 < full.vertex_count(),
                    "{shape:?} #{variant}: {} против {}",
                    far.vertex_count(),
                    full.vertex_count()
                );
                let (far_area, full_area) = (far.area_since(0), full.area_since(0));
                assert!(
                    (far_area / full_area - 1.0).abs() < 0.1,
                    "{shape:?} #{variant}: площадь {far_area} против {full_area}"
                );
            } else {
                assert_eq!(
                    far.positions_for_test(),
                    full.positions_for_test(),
                    "{shape:?} #{variant}"
                );
            }
        }
    }
}

/// Дальние ступени рисуют кроны не сущностями, а слитыми кусками: по слою
/// `tree_crowns` на кусок карты со своим z — две кроны в двух кусках дают два
/// слоя на разных z в полосе крон, — а кроны-сущности тех же деревьев видны
/// только ближней ступени. Обе формы собраны сразу, смена ступени лишь
/// переключает маски.
#[test]
fn far_zoom_steps_merge_crowns_into_chunks() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle {
        shape: TreeShape::Cotton,
        density: 9.0,
        ..default()
    };
    let planted = TreeSet::of([
        (Vec2::new(10.0, 10.0), 3.0, 0.0),
        (Vec2::new(20.0, 10.0), 3.0, 0.0),
        (Vec2::new(CROWN_CHUNK + 10.0, 10.0), 3.0, 0.0),
    ]);
    let (built, report) = mesh_trees(&style, &params(), &planted, &ConiferField::default());
    let near_only = TreeLodMask::of([0]);
    assert_eq!(built.crowns.len(), 3);
    assert!(built.crowns.iter().all(|crown| crown.shows == near_only));
    assert_eq!(built.pools.len(), 1);
    assert_eq!(built.merged.len(), 2);
    assert_eq!(report.chunks, 2);
    let far_steps = TreeLodMask::of(1..TREE_LODS.len());
    for merged in &built.merged {
        let layer = &merged.layer;
        assert_eq!(merged.shows, far_steps);
        assert_eq!(layer.name, "tree_crowns");
        assert_eq!(layer.material, MaterialSpec::Crown);
        assert!(layer.z >= Z_TREE && layer.z < Z_TREE + 1.0);
    }
    assert!(
        built.merged[0].layer.z != built.merged[1].layer.z,
        "два куска на одном z"
    );
    let vertices: usize = built
        .merged
        .iter()
        .map(|merged| merged.layer.builder.vertex_count())
        .sum();
    assert_eq!(report.merged_vertices, vertices);
    // первый кусок — кроны 0 и 1 (варианты 0 и 1), второй — крона 2
    let far = |variant| {
        crown_variant(TreeShape::Cotton, variant, &style, &params())
            .far
            .vertex_count()
    };
    assert_eq!(
        built.merged[0].layer.builder.vertex_count(),
        far(0) + far(1)
    );
    assert_eq!(built.merged[1].layer.builder.vertex_count(), far(2));
}

/// Слитые кроны режутся ещё и по полосам плотности: дальняя ступень с
/// меньшим потолком прячет полосу хвоста целиком, а полоса хвоста лежит выше
/// по z — крона с бо́льшим номером остаётся поверх и между полосами.
#[test]
fn merged_crowns_are_split_by_density_band() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_ten(TreeShape::Cotton, 9.0);
    // `ten_trees` в одном куске: полоса 0..3 — обеим дальним ступеням, 3..4 —
    // только первой из них
    assert_eq!(report.steps, [10, 4, 3]);
    assert_eq!(built.merged.len(), 2);
    let (head, tail) = (&built.merged[0], &built.merged[1]);
    assert_eq!(head.shows, TreeLodMask::of([1, 2]));
    assert_eq!(tail.shows, TreeLodMask::of([1]));
    assert!(tail.layer.z > head.layer.z);
    let far = |variant| {
        crown_variant(
            TreeShape::Cotton,
            variant,
            &TreeStyle {
                density: 9.0,
                ..default()
            },
            &params(),
        )
        .far
        .vertex_count()
    };
    assert_eq!(head.layer.builder.vertex_count(), far(0) + far(1) + far(2));
    assert_eq!(tail.layer.builder.vertex_count(), far(3));
}

/// Яркость дерева в слитом куске запечена в цвет вершин — тем же множителем,
/// что у кроны-сущности лежит в юниформе, — а координата кроны несёт место в
/// шаблоне единичного радиуса, не мировое.
#[test]
fn merged_crowns_bake_the_tint_and_keep_the_local_coordinate() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle {
        shape: TreeShape::Cotton,
        variance: 0.35,
        ..default()
    };
    let template = crown_variant(TreeShape::Cotton, 0, &style, &params()).far;
    let factor = style.tint_factors()[TreeStyle::tint_slot(0)];
    let at = Vec2::new(100.0, 50.0);
    let mut merged = MeshBuilder::with_crown_coords();
    merged.push_crown(&template, at, 4.0, factor);

    let base = template.colors_for_test()[0];
    let baked = merged.colors_for_test()[0];
    for channel in 0..3 {
        assert!((baked[channel] - base[channel] * factor).abs() < 1e-6);
    }
    assert_eq!(baked[3], base[3]);
    let local = merged
        .crown_coords_for_test()
        .expect("сборщик с координатами кроны");
    let placed = merged.positions_for_test();
    let unit = template.positions_for_test();
    for ((local, placed), unit) in local.iter().zip(placed).zip(unit) {
        assert_eq!(*local, [unit[0], unit[1]]);
        assert!((placed[0] - (unit[0] * 4.0 + at.x)).abs() < 1e-4);
    }
}

/// Все куски карты — на каждую полосу плотности — помещаются в полосу крон: у
/// каждого свой z, и их не больше, чем шагов `CROWN_CHUNK_Z_STEP` в
/// `Z_TREE..Z_TREE + 1`.
#[test]
fn every_chunk_of_the_map_fits_the_crown_z_band() {
    let chunks = (crate::settings::MAP_SIZE / CROWN_CHUNK).ceil();
    // кусок ствола на самой границе карты — ещё один ряд
    let worst = (chunks.x + 1.0) * (chunks.y + 1.0) * TREE_LODS.len() as f32;
    assert!(worst * CROWN_CHUNK_Z_STEP <= 1.0, "{worst} кусков");
}

/// Тени режутся по тем же кускам карты, что слитые кроны: два ствола в двух
/// кусках — два слоя одной полосы на разных z, и тень каждого дерева лежит в
/// куске своего ствола, без дублей на границе.
#[test]
fn tree_shadows_are_chunked_like_the_merged_crowns() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle {
        shape: TreeShape::Cotton,
        density: 9.0,
        ..default()
    };
    let planted = TreeSet::of([
        (Vec2::new(10.0, 10.0), 3.0, 0.0),
        (Vec2::new(CROWN_CHUNK - 1.0, 10.0), 3.0, 0.0),
        (Vec2::new(CROWN_CHUNK + 1.0, 10.0), 3.0, 0.0),
    ]);
    let (built, _) = mesh_trees(&style, &params(), &planted, &ConiferField::default());
    let near: Vec<&TreeLayer> = built
        .shadows
        .iter()
        .filter(|shadow| shadow.shows.shows(0))
        .collect();
    assert_eq!(near.len(), 2, "три дерева в двух кусках — два слоя");
    assert_ne!(near[0].layer.z, near[1].layer.z);
    let shadow = |variant| {
        crown_variant(TreeShape::Cotton, variant, &style, &params())
            .shadow
            .vertex_count()
    };
    assert_eq!(near[0].layer.builder.vertex_count(), shadow(0) + shadow(1));
    assert_eq!(near[1].layer.builder.vertex_count(), shadow(2));
}

/// Все слои теней карты помещаются в полосу теней: куски на каждый вид слоя
/// (полоса плотности × подробность шаблона) — не больше шагов
/// `TREE_SHADOW_Z_STEP` в `Z_TREE_SHADOW..Z_TREE_SHADOW + 1`.
#[test]
fn every_shadow_layer_of_the_map_fits_the_shadow_z_band() {
    let chunks = (crate::settings::MAP_SIZE / CROWN_CHUNK).ceil();
    let worst_chunks = (chunks.x + 1.0) * (chunks.y + 1.0);
    let kinds = (2 * TREE_LODS.len()) as f32;
    assert!(
        worst_chunks * kinds * TREE_SHADOW_Z_STEP <= 1.0,
        "{worst_chunks} кусков × {kinds}"
    );
}

/// Ползунок плотности отдаёт **префикс** набора: стоящие деревья не переезжают,
/// к ним только добавляются следующие. Это и есть [`TreeSet::visible_count`], но здесь
/// оно проверено на том, что реально попадает в мир, — на местах крон.
#[test]
fn the_density_slider_only_adds_crowns() {
    let _sun = crate::map::default_sun();
    let (thin, thin_report) = mesh_ten(TreeShape::Cotton, 3.0);
    let (thick, thick_report) = mesh_ten(TreeShape::Cotton, 9.0);

    assert_eq!(thin_report.crowns, 4, "пороги 0..=3");
    assert_eq!(thick_report.crowns, 10);
    assert_eq!(thin.crowns, thick.crowns[..thin.crowns.len()]);
    assert!(thin_report.shadow_vertices < thick_report.shadow_vertices);
}

/// У `Mixed` пула два — облачный и хвойный, — у конкретной формы один. Порода
/// дерева берётся из поля хвои, и на пустом поле (тестовый набор его не
/// семплировал) всякое дерево читается как лиственное, то есть встаёт в
/// нулевой пул.
#[test]
fn mixed_carries_two_pools_and_a_concrete_shape_one() {
    let _sun = crate::map::default_sun();
    for shape in TreeShape::CONCRETE {
        let (built, _) = mesh_ten(shape, 9.0);
        assert_eq!(built.pools.len(), 1, "{shape:?}");
        assert!(built.crowns.iter().all(|crown| crown.pool == 0));
    }

    let (mixed, report) = mesh_ten(TreeShape::Mixed, 9.0);
    assert_eq!(mixed.pools.len(), 2);
    assert_eq!(report.shape, TreeShape::Mixed);
    assert!(mixed.crowns.iter().all(|crown| crown.pool == 0));
    for pool in &mixed.pools {
        assert_eq!(pool.len(), TREE_VARIANTS);
    }
}

/// Тени — слитые слои по полосам плотности, полупрозрачные: цвет лежит в
/// вершинах, материалу остаётся блендинг. У каждого слоя свой z в полосе теней
/// (равный z отдал бы порядок сортировке — мигание). Кроны в эти слои не
/// попадают.
#[test]
fn the_shadows_are_blended_layers_on_their_own_z() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_ten(TreeShape::Cotton, 9.0);

    // полоса на ступень, и у полос, которые видят обе подробности, — по слою
    // на каждую (полный шаблон и прореженный)
    assert!(built.shadows.len() >= TREE_LODS.len());
    let mut zs = Vec::new();
    for shadow in &built.shadows {
        let layer = &shadow.layer;
        assert_eq!(layer.name, "tree_shadows");
        assert!(layer.z >= Z_TREE_SHADOW && layer.z < Z_TREE_SHADOW + 1.0);
        assert_eq!(layer.material, MaterialSpec::Blend);
        assert!(!shadow.shows.is_empty());
        zs.push(layer.z);
    }
    zs.dedup();
    assert_eq!(zs.len(), built.shadows.len(), "два слоя теней на одном z");
    let vertices: usize = built
        .shadows
        .iter()
        .map(|shadow| shadow.layer.builder.vertex_count())
        .sum();
    assert_eq!(vertices, report.shadow_vertices);
    assert!(report.shadow_vertices > 0);
}

/// Соседние кроны различаются оттенком: пять подряд стоящих деревьев берут
/// все пять слотов яркости, а не идут полосами.
#[test]
fn neighbouring_crowns_differ_in_tint() {
    let _sun = crate::map::default_sun();
    let (built, _) = mesh_ten(TreeShape::Cotton, 9.0);

    let tints: std::collections::HashSet<usize> =
        built.crowns[..5].iter().map(|crown| crown.tint).collect();
    assert_eq!(tints.len(), 5, "оттенки пошли полосами");
    assert_eq!(built.tints.len(), 5);
}

/// z группирует кроны по мешу и материалу: после сортировки по z — а только
/// по нему сортирует прозрачная фаза — кроны одной группы (пул, вариант,
/// оттенок) стоят подряд, иначе bevy не сольёт их в один draw. Внутри группы
/// z растёт с номером дерева (стабильный порядок перекрытий), и вся полоса
/// лежит между `Z_TREE` и следующей ступенью стопки.
#[test]
fn crowns_of_one_mesh_and_tint_are_contiguous_in_z() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle {
        shape: TreeShape::Mixed,
        density: 1000.0,
        ..default()
    };
    let spots: Vec<(Vec2, f32)> = (0..400)
        .map(|i| (Vec2::new(i as f32 * 10.0, 0.0), 1.0))
        .collect();
    let planted = TreeSet::of(spots.iter().map(|&(at, radius)| (at, radius, 0.0)));
    // хвоя по полю с полной примесью — чтобы в наборе стояли обе формы
    let mut field = ConiferField::default();
    field.resample(&spots, &ConiferNoiseStyle::default(), 1.0);
    field.set_share(0.5);
    let (built, _) = mesh_trees(&style, &params(), &planted, &field);
    assert_eq!(built.crowns.len(), 400);

    let group = |crown: &CrownPlacement| (crown.pool, crown.variant, crown.tint);
    let mut by_z: Vec<(usize, &CrownPlacement)> = built.crowns.iter().enumerate().collect();
    by_z.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
    let mut seen = std::collections::HashSet::new();
    for pair in by_z.windows(2) {
        let (a, b) = (pair[0].1, pair[1].1);
        assert!(b.z > a.z, "две кроны на одном z");
        if group(a) == group(b) {
            assert!(pair[1].0 > pair[0].0, "внутри группы z не растёт с номером");
        } else {
            assert!(
                seen.insert(group(a)),
                "группа {:?} разорвана по z",
                group(a)
            );
        }
    }
    let pools: std::collections::HashSet<usize> =
        built.crowns.iter().map(|crown| crown.pool).collect();
    assert_eq!(pools.len(), 2, "у Mixed две формы — два пула");
    assert!(
        built
            .crowns
            .iter()
            .all(|crown| crown.z >= Z_TREE && crown.z < Z_TREE + 1.0),
        "полоса крон вышла за Z_TREE..Z_TREE + 1"
    );
}

/// Плотность ниже порога появления первого дерева — пустой лес: ни одной кроны
/// и ни одной тени. Ползунок в игре так низко не опускается
/// (`TREE_DENSITY_MIN` 0.25, а одиночные деревья стоят на пороге 0), так что
/// это граница сборки, а не состояние из UI. Слой при этом всё равно описан, и
/// его пустоту отбрасывает уже адаптер (`spawn_layer`) — второй дороги, на
/// которой можно забыть деспавн, нет.
#[test]
fn a_density_under_the_first_threshold_plants_nothing() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_ten(TreeShape::Cotton, -1.0);

    assert!(built.crowns.is_empty());
    assert_eq!(report.crowns, 0);
    assert!(
        built
            .shadows
            .iter()
            .all(|shadow| shadow.layer.builder.is_empty())
    );
}
