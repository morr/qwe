//! Силуэт, экструзия, рампа тона крыш — и слои целиком (`mesh_buildings`):
//! состав слоёв, чанки экструзии и оба теневых списка.

use super::super::fixtures::{building, detail, extruded_mesh, ground_shadows, rect, square};
use super::super::layers::{roof_color, silhouette_edges};
use super::super::material::*;
use super::super::*;
use super::support::{building_plan, layer_names, one_building};
use crate::map::osm::{AreaKind, BuildingUse};
use crate::map::shadow_dir;

#[test]
fn silhouette_picks_edges_facing_the_shadow() {
    let _sun = crate::map::default_sun();
    // свет сверху-слева, тень вправо-вниз: силуэт — нижнее и правое рёбра
    let edges = silhouette_edges(&square(), shadow_dir());
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|(a, b)| {
        let bottom = a.y == 0.0 && b.y == 0.0;
        let right = a.x == 10.0 && b.x == 10.0;
        bottom || right
    }));
}

#[test]
fn silhouette_is_winding_independent() {
    let _sun = crate::map::default_sun();
    let ccw = square();
    let cw: Vec<Vec2> = square().into_iter().rev().collect();
    let mut ccw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&ccw, shadow_dir());
    let mut cw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&cw, shadow_dir())
        .into_iter()
        .map(|(a, b)| (b, a))
        .collect();
    let key = |(a, b): &(Vec2, Vec2)| (a.x + a.y).min(b.x + b.y);
    ccw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
    cw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
    assert_eq!(ccw_edges.len(), 2);
    for (ccw_edge, cw_edge) in ccw_edges.iter().zip(&cw_edges) {
        let matches = (ccw_edge.0 == cw_edge.0 && ccw_edge.1 == cw_edge.1)
            || (ccw_edge.0 == cw_edge.1 && ccw_edge.1 == cw_edge.0);
        assert!(matches, "{ccw_edge:?} vs {cw_edge:?}");
    }
}

#[test]
fn extrusion_walls_face_away_from_the_lift() {
    let _sun = crate::map::default_sun();
    // подъём вверх-вправо: у квадрата видимы южная и западная стены
    let lift = Lean::of().dir();
    assert!(
        lift.x > 0.0 && lift.y > 0.0,
        "the lift is oblique: {lift:?}"
    );
    let edges = silhouette_edges(&square(), -lift);
    assert_eq!(edges.len(), 2);
    assert!(
        edges.iter().any(|(a, b)| a.y == 0.0 && b.y == 0.0),
        "south wall"
    );
    assert!(
        edges.iter().any(|(a, b)| a.x == 0.0 && b.x == 0.0),
        "west wall"
    );
}

#[test]
fn extrusion_sorts_the_far_end_of_the_lift_first() {
    let _sun = crate::map::default_sun();
    let north = building(
        square()
            .iter()
            .map(|p| *p + Vec2::new(0.0, 100.0))
            .collect(),
        Some(30.0),
        AreaKind::Building,
    );
    let south = building(square(), Some(3.0), AreaKind::Building);
    let positions = |list: &[PolyArea]| {
        extruded_mesh(list, &[], detail(false))
            .build()
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec()
    };
    let sorted = positions(&[south.clone(), north.clone()]);
    let reversed = positions(&[north, south]);
    // порядок входа не важен: painter's sort всегда пишет дальний по подъёму
    // (северный) дом первым, поэтому буферы вершин совпадают, а первая
    // вершина — северная
    assert_eq!(sorted, reversed);
    assert!(
        sorted[0][1] >= 100.0,
        "north building must be written first"
    );
}

#[test]
fn every_mode_builds_geometry_for_mixed_input() {
    let _sun = crate::map::default_sun();
    let mut with_hole = building(square(), Some(20.0), AreaKind::Building);
    with_hole.holes.push(vec![
        Vec2::new(4.0, 4.0),
        Vec2::new(6.0, 4.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(4.0, 6.0),
    ]);
    let list = [
        with_hole,
        building(square(), None, AreaKind::Building),
        building(square(), Some(12.0), AreaKind::Kremlin),
    ];

    let footprints = HouseFootprint::all(&list);
    let ctx = CityContext::of(&list, &[], &footprints);
    let (facades, roofs) = facade_and_roof_builders(&list, detail(true), &ctx, &footprints);
    assert!(!facades.is_empty());
    assert!(!roofs.is_empty());
    assert_eq!(facades.skipped_polygons(), 0);

    let shadows = ground_shadows(&list, &[], false);
    assert!(!shadows.is_empty());

    let extruded = extruded_mesh(&list, &[], detail(false));
    assert!(!extruded.is_empty());
    assert_eq!(extruded.skipped_polygons(), 0);
    // комбинированный режим: рампа меняет цвета, но не геометрию
    let tinted = extruded_mesh(&list, &[], detail(true));
    assert!(!tinted.is_empty());
    assert_eq!(tinted.skipped_polygons(), 0);
}

#[test]
fn roof_tint_darkens_tall_buildings_and_spares_the_kremlin() {
    let _sun = crate::map::default_sun();
    // назначение задано: нетегированная коробка — частный сектор, тегированная
    // — корпус, и без него база и «высотка» взяли бы разные таблицы кровли
    let color = |height, tinted| {
        let mut b = building(square(), height, AreaKind::Building);
        b.building_use = BuildingUse::Apartments;
        roof_color(&b, &roof_look(&b, None), tinted)
    };
    let base = color(None, true);
    let tall = color(Some(60.0), true);
    // именно темнее в сумме, а не в каждом канале: рампа ведёт к нейтральному
    // тёмному, и у зеленоватой кровли её зелёный канал почти не двигается
    let luminance = |color: Srgba| color.red + color.green + color.blue;
    assert!(luminance(tall) < luminance(base), "{tall:?} vs {base:?}");

    let kremlin = |height, tinted| {
        let b = building(square(), height, AreaKind::Kremlin);
        roof_color(&b, &roof_look(&b, None), tinted)
    };
    assert_eq!(kremlin(Some(60.0), true), kremlin(None, false));
}

#[test]
fn the_tint_ramp_darkens_every_palette_colour() {
    let _sun = crate::map::default_sun();
    // Цель рампы обязана быть темнее любого цвета любой палитры, иначе на
    // высоком доме рампа переворачивается и осветляет: у насыщенной красной
    // черепицы сумма каналов ниже, чем у среднего серого. Проверяется на
    // каждом цвете — палитры теперь не одни серые.
    let tall = building(square(), Some(60.0), AreaKind::Building);
    let luminance = |color: Srgba| color.red + color.green + color.blue;
    let palettes = RoofKind::ALL
        .iter()
        .map(|kind| kind.palette())
        .chain(std::iter::once(&CHURCH_ROOF_COLORS as &[Color]));
    for palette in palettes {
        for color in palette {
            let look = RoofLook::new(RoofKind::Tile, color.to_srgba(), Vec2::X, 0.0);
            let ramped = roof_color(&tall, &look, true);
            let flat = roof_color(&tall, &look, false);
            assert!(
                luminance(ramped) < luminance(flat),
                "{color:?}: ramped {ramped:?} vs flat {flat:?}"
            );
        }
    }
}

// --- слои целиком ------------------------------------------------------
//
// Тесты на `mesh_buildings`. До шва слои собирались внутри системы Bevy, и
// достать сборку из теста было нечем: проверять можно было только билдеры под
// ней. Здания — единственный модуль карты, отдающий **два** списка
// (`BuildingMeshes { layers, shadows }`) под разными метками и с разным
// расписанием пересборки, и держит это разделение только здешняя секция.

#[test]
fn extrusion_builds_one_layer_and_the_flat_modes_split_facade_from_roof() {
    let _sun = crate::map::default_sun();
    let buildings = [one_building()];

    // 2.5D: стены и кровля в одном меше, и весь он идёт кровельным материалом
    let (extruded, _) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &buildings,
        &[],
    );
    assert_eq!(layer_names(&extruded.layers), ["building_extruded"]);
    assert_eq!(extruded.layers[0].material, MaterialSpec::Roof);

    // плоские режимы: полоса фасада отдельным слоем и **ниже** кровли — крыша
    // соседа сверху прикрывает полосу
    let (flat, _) = mesh_buildings(
        building_plan(BuildingHeightMode::Facade, false),
        &buildings,
        &[],
    );
    assert_eq!(
        layer_names(&flat.layers),
        ["building_facades", "building_roofs"]
    );
    assert_eq!(flat.layers[0].material, MaterialSpec::Flat);
    assert_eq!(flat.layers[1].material, MaterialSpec::Roof);
    assert!(
        flat.layers[0].z < flat.layers[1].z,
        "фасад лежит ниже крыши"
    );
    assert!(flat.layers.iter().all(|layer| !layer.builder.is_empty()));
}

/// Слой экструзии режется домами по клеткам карты: дом через границу клетки
/// уходит в один кусок целиком — его собственный порядок (крыша поверх своих
/// стен) переживает разрез, каким бы косым ни был дом.
#[test]
fn a_building_straddling_the_border_lives_in_one_chunk() {
    let _sun = crate::map::default_sun();
    // квадрат с центром ровно на линии сетки x = 1000
    let ring = rect(Vec2::new(995.0, 995.0), Vec2::new(1005.0, 1005.0));
    let (built, _) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &[building(ring, Some(9.0), AreaKind::Building)],
        &[],
    );
    assert_eq!(
        layer_names(&built.layers),
        ["building_extruded"],
        "один дом — один кусок"
    );
    // крыша смещена вправо-вверх за границу — её вершины в том же куске
    let builder = &built.layers[0].builder;
    assert!(
        builder
            .positions_for_test()
            .iter()
            .any(|point| point[0] > 1005.0 && point[1] > 1005.0),
        "геометрия за границей клетки не потерялась"
    );
}

/// Клетки экструзии идут от дальней к ближней, и z растёт вместе с ними:
/// стек кусков держится на глубинном буфере, а не на порядке рендер-бина.
#[test]
fn building_chunks_stack_by_depth() {
    let _sun = crate::map::default_sun();
    let far = building(
        rect(Vec2::new(2500.0, 2500.0), Vec2::new(2510.0, 2510.0)),
        Some(9.0),
        AreaKind::Building,
    );
    // квадрат у начала координат — клетка (0, 0), ближняя
    let (built, _) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &[far, one_building()],
        &[],
    );
    assert_eq!(built.layers.len(), 2, "две клетки — два куска");
    assert!(
        built.layers[0].z < built.layers[1].z,
        "дальняя клетка стоит ниже ближней"
    );
    for layer in &built.layers {
        assert!(
            crate::map::z::Z_BUILDING - layer.z <= 0.05,
            "куски живут в полосе под полным зумом слоя, а не под тенями: {:?}",
            layer.z
        );
    }
}

#[test]
fn both_shadow_layers_ride_their_own_list_around_the_building_ones() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, true),
        &[one_building()],
        &[],
    );

    assert_eq!(
        layer_names(&built.shadows),
        ["building_shadows", "roof_shadows"]
    );
    // обе тени полупрозрачны, отсюда `Blend`
    assert!(
        built
            .shadows
            .iter()
            .all(|layer| layer.material == MaterialSpec::Blend)
    );
    // наземная тень — под всеми зданиевыми слоями (её маскирует стена соседа),
    // тень на кровле — над ними: это единственная часть тени, которая обязана
    // лежать поверх крыши
    let ground = built.shadows[0].z;
    let on_roofs = built.shadows[1].z;
    assert!(
        built
            .layers
            .iter()
            .all(|layer| ground < layer.z && layer.z < on_roofs),
        "теневые рунги обязаны обнимать зданиевые"
    );
    assert!(report.vertices > 0);
}

#[test]
fn the_shadow_flag_leaves_the_list_empty_without_touching_the_building_layers() {
    let _sun = crate::map::default_sun();
    let buildings = [one_building()];
    let (kept, quiet) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &buildings,
        &[],
    );
    let (whole, full) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, true),
        &buildings,
        &[],
    );

    // `shadows: false` — «теневой слой оставить как есть»: список пуст, а не с
    // пустыми мешами, потому что адаптер в этом случае старые теневые сущности
    // не деспавнит, и класть поверх них нечего
    assert!(kept.shadows.is_empty());
    // а сами зданиевые слои от тумблера не зависят
    assert_eq!(layer_names(&kept.layers), layer_names(&whole.layers));
    // и вершины теней считаются в общий отчёт, а не мимо него
    assert!(full.vertices > quiet.vertices);
}

#[test]
fn a_mode_without_long_shadows_builds_none_however_the_plan_asks() {
    let _sun = crate::map::default_sun();
    let (built, _) = mesh_buildings(
        building_plan(BuildingHeightMode::Facade, true),
        &[one_building()],
        &[],
    );

    // длинные тени рисуют не все режимы (`casts_shadows`), и план их не
    // переспорит
    assert!(built.shadows.is_empty());
}
