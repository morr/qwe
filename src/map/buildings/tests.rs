use bevy::color::Luminance;

use super::arches::*;
use super::layers::*;
use super::roofs::*;
use super::*;
use crate::map::SHADOW_DIR;
use crate::map::osm::RoadClass;
use crate::map::osm::model::signed_ring_area;
use crate::settings::ARCH_HEIGHT;

fn square() -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ]
}

fn building(outer: Vec<Vec2>, height: Option<f32>, kind: AreaKind) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use: BuildingUse::Other,
        height,
        entrances: Vec::new(),
    }
}

#[test]
fn silhouette_picks_edges_facing_the_shadow() {
    // свет сверху-слева, тень вправо-вниз: силуэт — нижнее и правое рёбра
    let edges = silhouette_edges(&square(), SHADOW_DIR);
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|(a, b)| {
        let bottom = a.y == 0.0 && b.y == 0.0;
        let right = a.x == 10.0 && b.x == 10.0;
        bottom || right
    }));
}

#[test]
fn silhouette_is_winding_independent() {
    let ccw = square();
    let cw: Vec<Vec2> = square().into_iter().rev().collect();
    let mut ccw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&ccw, SHADOW_DIR);
    let mut cw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&cw, SHADOW_DIR)
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
    // подъём вверх-вправо: у квадрата видимы южная и западная стены
    let lift = extrusion_dir();
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
fn the_wall_facing_the_light_is_lighter_than_the_one_facing_away() {
    let lift = extrusion_dir();
    let facade = Color::srgb(0.6, 0.6, 0.6);
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    // свет из верхнего левого угла: западная стена (нормаль −X) освещена,
    // южная (нормаль −Y) в тени
    let (west, _) = wall_colors(facade, Vec2::new(0.0, 10.0), Vec2::ZERO, lift);
    let (south, _) = wall_colors(facade, Vec2::ZERO, Vec2::new(10.0, 0.0), lift);
    assert!(luminance(west) > luminance(facade.to_linear()));
    assert!(luminance(south) < luminance(facade.to_linear()));
    // обход ребра тон не меняет
    let (west_reversed, _) = wall_colors(facade, Vec2::ZERO, Vec2::new(0.0, 10.0), lift);
    assert_eq!(west, west_reversed);
}

#[test]
fn extrusion_sorts_the_far_end_of_the_lift_first() {
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
        extrusion_builder(list, &[], false)
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
fn the_palette_follows_the_building_use_and_spares_the_kremlin() {
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let mut church = building(square(), None, AreaKind::Building);
    church.building_use = BuildingUse::Church;
    let other = building(square(), None, AreaKind::Building);
    let mut kremlin = building(square(), None, AreaKind::Kremlin);
    kremlin.building_use = BuildingUse::Church;

    let roof = |b: &PolyArea| roof_color(b, 0, false);
    assert_ne!(roof(&house), roof(&other));
    assert_ne!(roof(&church), roof(&house));
    // Кремль красится по `kind`, назначение его не перекрашивает
    let kremlin_plain = building(square(), None, AreaKind::Kremlin);
    assert_eq!(roof(&kremlin), roof(&kremlin_plain));
    // крыша светлее стены — так стены читаются полосой под крышей. Храм —
    // намеренное исключение: белые стены под зелёной крышей
    for b in [&house, &other, &kremlin] {
        let (roof, facade) = base_colors(b);
        assert!(
            roof.luminance() > facade.luminance(),
            "{:?}",
            b.building_use
        );
    }
}

fn oblong(width: f32, length: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(length, 0.0),
        Vec2::new(length, width),
        Vec2::new(0.0, width),
    ]
}

#[test]
fn a_gable_goes_on_houses_and_small_boxes_only() {
    let mut house = building(oblong(8.0, 600.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(is_gabled(&house), "a house of any size");
    let small = building(square(), None, AreaKind::Building);
    assert!(is_gabled(&small), "an untagged small box");
    let big = building(oblong(20.0, 20.0), None, AreaKind::Building);
    assert!(!is_gabled(&big), "an untagged big box");
    let mut flats = building(square(), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(!is_gabled(&flats));
    let tower = building(square(), None, AreaKind::Kremlin);
    assert!(!is_gabled(&tower), "the kremlin keeps its flat roof");
    let mut yard = house.clone();
    yard.holes.push(vec![
        Vec2::new(4.0, 2.0),
        Vec2::new(6.0, 2.0),
        Vec2::new(6.0, 4.0),
        Vec2::new(4.0, 4.0),
    ]);
    assert!(!is_gabled(&yard), "a courtyard has no ridge");
}

#[test]
fn the_ridge_runs_along_the_long_axis_of_the_rotated_footprint() {
    // прямоугольник 20 × 6, повёрнутый на 30°, обойдённый по часовой
    let rotate = |p: Vec2| Vec2::from_angle(30f32.to_radians()).rotate(p);
    let mut ring: Vec<Vec2> = oblong(6.0, 20.0).into_iter().map(rotate).collect();
    ring.reverse();
    let rect = min_area_rect(&ring).unwrap();
    assert!(
        (rect[1] - rect[0]).length() > 19.9,
        "long side first: {rect:?}"
    );
    assert!(
        (rect[2] - rect[1]).length() < 6.1,
        "short side second: {rect:?}"
    );
    assert!(signed_ring_area(&rect) > 0.0, "CCW");
    // конёк — вдоль длинной оси
    let mut house = building(ring, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let roof = gable_roof(&house, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).unwrap();
    let ridge = roof.slopes[0].0[2] - roof.slopes[0].0[3];
    let long = rect[1] - rect[0];
    assert!(ridge.normalize().dot(long.normalize()).abs() > 0.999);
}

#[test]
fn an_l_shaped_house_keeps_a_flat_roof() {
    let l_shape = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(12.0, 0.0),
        Vec2::new(12.0, 5.0),
        Vec2::new(5.0, 5.0),
        Vec2::new(5.0, 12.0),
        Vec2::new(0.0, 12.0),
    ];
    let mut house = building(l_shape, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(is_gabled(&house));
    assert!(gable_roof(&house, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_none());
}

#[test]
fn gable_roof_itself_refuses_a_building_that_is_not_gabled() {
    let mut flats = building(oblong(8.0, 20.0), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(gable_roof(&flats, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_none());
}

#[test]
fn the_slope_facing_the_light_is_lighter_and_the_ridge_is_lifted() {
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    let mut house = building(oblong(8.0, 20.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let base = Srgba::rgb(0.5, 0.5, 0.5);
    let roof = gable_roof(&house, Vec2::ZERO, ridge_lift, base).unwrap();
    // скаты: южный (карниз y = 0) отвёрнут от света, северный повёрнут
    let (south, north) = (&roof.slopes[0], &roof.slopes[1]);
    assert_eq!(south.0[0].y, 0.0);
    assert!(luminance(south.1) < luminance(base.into()));
    assert!(luminance(north.1) > luminance(base.into()));
    // конёк поднят по вектору подъёма на масштаб стен
    let ridge = south.0[3] - Vec2::new(0.0, 4.0);
    let expected = ridge_lift(ridge_rise(8.0));
    assert!(ridge.distance(expected) < 1e-4, "{ridge:?} vs {expected:?}");
    assert!(expected.y > 0.0 && expected.x > 0.0);
    // конёк — вдоль длинной оси, оба фронтона стоят на торцах
    for ((a, b), apex) in roof.gables {
        assert!((b - a).length() < 8.1, "gable on the short side");
        assert!(apex.distance((a + b) / 2.0 + expected) < 1e-4);
    }
}

#[test]
fn a_house_without_height_is_two_storeys_not_five() {
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let other = building(square(), None, AreaKind::Building);
    assert!(height_or_default(&house) < height_or_default(&other));
    assert_eq!(height_or_default(&other), DEFAULT_BUILDING_HEIGHT);
}

#[test]
fn shadow_length_scales_with_height() {
    let low = building(square(), Some(6.0), AreaKind::Building);
    let high = building(square(), Some(60.0), AreaKind::Building);
    let reach = |list: &[PolyArea]| {
        let mesh = shadow_builder(list, &[], false).build();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        positions
            .iter()
            .map(|p| Vec2::new(p[0], p[1]).dot(SHADOW_DIR))
            .fold(f32::NEG_INFINITY, f32::max)
    };
    assert!(reach(std::slice::from_ref(&high)) > reach(std::slice::from_ref(&low)) + 10.0);
}

#[test]
fn every_mode_builds_geometry_for_mixed_input() {
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

    let (facades, roofs) = facade_and_roof_builders(&list, &[], true);
    assert!(!facades.is_empty());
    assert!(!roofs.is_empty());
    assert_eq!(facades.skipped_polygons(), 0);

    let shadows = shadow_builder(&list, &[], false);
    assert!(!shadows.is_empty());

    let extruded = extrusion_builder(&list, &[], false);
    assert!(!extruded.is_empty());
    assert_eq!(extruded.skipped_polygons(), 0);
    // комбинированный режим: рампа меняет цвета, но не геометрию
    let tinted = extrusion_builder(&list, &[], true);
    assert!(!tinted.is_empty());
    assert_eq!(tinted.skipped_polygons(), 0);
}

/// Сумма площадей треугольников меша — двойное наложение внутри тени
/// давало бы сумму больше площади самой фигуры.
fn mesh_area(mesh: &Mesh) -> f32 {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .to_vec();
    let indices: Vec<usize> = match mesh.indices().unwrap() {
        bevy::mesh::Indices::U32(list) => list.iter().map(|&i| i as usize).collect(),
        bevy::mesh::Indices::U16(list) => list.iter().map(|&i| i as usize).collect(),
    };
    indices
        .chunks_exact(3)
        .map(|triangle| {
            let point = |i: usize| Vec2::new(positions[triangle[i]][0], positions[triangle[i]][1]);
            (point(1) - point(0)).perp_dot(point(2) - point(0)).abs() / 2.0
        })
        .sum()
}

#[test]
fn square_shadow_is_one_swept_polygon() {
    let list = [building(square(), Some(15.0), AreaKind::Building)];
    let mesh = shadow_builder(&list, &[], false).build();
    // одна цепочка низ+право: свип из 6 вершин, без квадов на ребро
    assert_eq!(mesh.count_vertices(), 6);
}

#[test]
fn staircase_shadow_has_no_double_darkening() {
    // ступенчатый юго-восточный фасад: раньше квады ступеней перекрывались
    // вдоль тени и полупрозрачность складывалась в полосы. Свип монотонной
    // цепочки покрывает ровно |сдвиг| × перп-протяжённость — без нахлёстов
    let staircase = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(6.0, 0.0),
        Vec2::new(6.0, 3.0),
        Vec2::new(9.0, 3.0),
        Vec2::new(9.0, 6.0),
        Vec2::new(12.0, 6.0),
        Vec2::new(12.0, 9.0),
        Vec2::new(0.0, 9.0),
    ];
    let chains = silhouette_chains(&staircase, SHADOW_DIR);
    assert_eq!(chains.len(), 1, "лестница — одна непрерывная цепочка");
    assert_eq!(chains[0].len(), 7);

    let list = [building(staircase, Some(20.0), AreaKind::Building)];
    let mesh = shadow_builder(&list, &[], false).build();
    let offset_length = 20.0 * SHADOW_LENGTH_SCALE;
    let perp_span = Vec2::new(12.0, 9.0).dot(SHADOW_DIR.perp());
    assert!((mesh_area(&mesh) - offset_length * perp_span).abs() < 0.5);
}

#[test]
fn neighbour_shadows_union_without_double_darkening() {
    // два корпуса в ряд: тень левого дотягивается до правого, и без
    // union суммарная площадь меша была бы суммой двух свипов — с
    // перекрытием, читающимся как пятно двойной темноты
    let left = building(square(), Some(15.0), AreaKind::Building);
    let right = building(
        square().iter().map(|p| *p + Vec2::new(12.0, 0.0)).collect(),
        Some(15.0),
        AreaKind::Building,
    );
    let alone =
        |b: &PolyArea| mesh_area(&shadow_builder(std::slice::from_ref(b), &[], false).build());
    let separate = alone(&left) + alone(&right);
    let together = mesh_area(&shadow_builder(&[left, right], &[], false).build());
    assert!(
        together < separate - 1.0,
        "union must remove the overlap: {together} vs {separate}"
    );
}

#[test]
fn roof_tint_darkens_tall_buildings_and_spares_the_kremlin() {
    let base = roof_color(&building(square(), None, AreaKind::Building), 0, true);
    let tall = roof_color(&building(square(), Some(60.0), AreaKind::Building), 0, true);
    assert!(tall.red < base.red);
    assert!(tall.green < base.green);

    let kremlin_flat = roof_color(&building(square(), Some(60.0), AreaKind::Kremlin), 0, true);
    let kremlin_base = roof_color(&building(square(), None, AreaKind::Kremlin), 0, false);
    assert_eq!(kremlin_flat, kremlin_base);
}

fn passage(points: Vec<Vec2>, passage: bool) -> RoadLine {
    RoadLine {
        points,
        width: 5.0,
        class: RoadClass::Street,
        bridge: false,
        passage,
    }
}

/// Проём режется только под дорогой с флагом `passage`, и только если
/// она действительно идёт сквозь дом.
#[test]
fn only_a_building_passage_cuts_an_arch() {
    let house = vec![building(square(), Some(15.0), AreaKind::Building)];
    let through = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        true,
    )];
    let alongside = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        false,
    )];
    let elsewhere = vec![passage(
        vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 10.0)],
        true,
    )];

    let solid = extrusion_builder(&house, &[], false).vertex_count();
    assert!(extrusion_builder(&house, &through, false).vertex_count() > solid);
    assert_eq!(
        extrusion_builder(&house, &alongside, false).vertex_count(),
        solid
    );
    assert_eq!(
        extrusion_builder(&house, &elsewhere, false).vertex_count(),
        solid
    );
}

/// Высота проёма задана в настоящих метрах, а рисуется проекция:
/// трёхметровая арка обязана занять ту же долю нарисованной стены, какую
/// три метра занимают в настоящей высоте дома.
#[test]
fn an_arch_opening_is_three_real_metres_of_the_drawn_wall() {
    // 40 м высоты, подъём 14 м: арка обязана занять 14 × 3/40 = 1.05 м
    let tall = building(square(), Some(40.0), AreaKind::Building);
    let lift = extrusion_lift(&tall, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &tall, &[&road], lift);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        let low = values.iter().copied().fold(f32::INFINITY, f32::min);
        let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        high - low
    };

    let expected = lift.y * ARCH_HEIGHT / 40.0;
    assert!(
        (span(|p| p[1]) - expected).abs() < 0.01,
        "opening is {} m tall, expected {expected} m of a {} m wall",
        span(|p| p[1]),
        lift.y
    );
}

/// У низкого дома нарисованный метр стоит других настоящих метров:
/// подъём обрезан `EXTRUDE_RANGE`, и пересчёт через `EXTRUDE_SCALE` дал бы
/// не ту долю. Проверяем, что доля считается от высоты самого дома.
#[test]
fn a_clamped_wall_still_gets_a_proportional_opening() {
    // 4 м высоты: подъём 4 × 0.35 = 1.4 обрезается снизу до 2.5 м
    let low = building(square(), Some(4.0), AreaKind::Building);
    let lift = extrusion_lift(&low, BuildingHeightMode::Extrusion);
    assert_eq!(lift.y, *EXTRUDE_RANGE.start());

    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);
    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &low, &[&road], lift);

    let heights: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .collect();
    let opening = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - heights.iter().copied().fold(f32::INFINITY, f32::min);
    // арка выше самого дома (6 > 4) — проём режется по стене целиком
    assert!(
        (opening - lift.y).abs() < 0.01,
        "opening {opening} m, expected the whole {} m wall",
        lift.y
    );
}

/// Проём лежит в плоскости стены и шириной с проход, а не растянут вдоль
/// дороги: у дороги, подходящей к южной грани под углом, вырез всё равно
/// ровно по грани.
#[test]
fn an_arch_is_cut_along_the_wall_not_along_the_road() {
    let house = building(square(), Some(15.0), AreaKind::Building);
    // `push_arches` — фасадный режим: полоса сдвинута строго вниз. Косой
    // подъём 2.5D сюда не годится — его x-составляющая растянула бы проём
    let band = Vec2::new(0.0, -3.0);
    // дорога идёт наискось и коротка: до стены дотягивается один конец
    let slanted = passage(vec![Vec2::new(4.0, -2.0), Vec2::new(9.0, 20.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&slanted], band);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - values.iter().copied().fold(f32::INFINITY, f32::min)
    };
    // ширина дороги, спроецированная углом входа: наклонная дорога
    // дырявит стену уже собственной ширины
    let entry = (Vec2::new(9.0, 20.0) - Vec2::new(4.0, -2.0)).normalize();
    let expected = slanted.width * entry.perp_dot(Vec2::X).abs();
    assert!(
        (span(|p| p[0]) - expected).abs() < 0.01,
        "opening is {} m wide, expected {expected} m",
        span(|p| p[0])
    );
}

/// Регресс: конец прохода — общая вершина двух граней (как у любой
/// OSM-арки). Зажатый в одну грань проём выходил вдвое уже дороги;
/// теперь куски на обеих гранях продолжают друг друга.
#[test]
fn an_arch_at_a_shared_vertex_keeps_the_road_width() {
    // южная сторона из двух граней со стыком в (5, 0)
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ],
        Some(15.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], lift);

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let width = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - xs.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        width >= road.width * 0.9,
        "opening is {width} m wide against a {} m road",
        road.width
    );
}

/// Арка у самого угла дома: проём подрезается по концу грани, а не
/// повисает половиной квада в воздухе за углом.
#[test]
fn an_arch_near_a_corner_is_trimmed_to_the_wall() {
    let house = building(square(), Some(15.0), AreaKind::Building);
    // фасадная полоса, как в `an_arch_is_cut_along_the_wall_not_along_the_road`
    let band = Vec2::new(0.0, -3.0);
    // дорога упирается в южную грань в метре от юго-западного угла
    let road = passage(vec![Vec2::new(1.0, 0.0), Vec2::new(1.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], band);
    assert!(!builder.is_empty());

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let west = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let east = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(west >= -0.01, "opening hangs past the corner: {west}");
    // а восточный край — там, куда дотянулась полуширина
    assert!((east - 3.5).abs() < 0.01, "{east}");
}

/// Регресс на реальную арку 485488257 (Тула): проход размечен отрезком
/// **между двумя вершинами контура**, лежит внутри дома и стен касается
/// только концами. Поиск пересечения дороги с контуром здесь не находит
/// ничего — вырез обязан появиться от концов.
#[test]
fn an_arch_lying_inside_the_outline_still_cuts_an_opening() {
    // упрощённая геометрия того дома: южная грань y = 0, арка — отрезок
    // от вершины (5, 0) вглубь до вершины (5.2, 14) северной грани
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 14.0),
            Vec2::new(5.2, 14.0),
            Vec2::new(0.0, 14.0),
        ],
        Some(42.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let inner = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.2, 14.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&inner], lift);
    assert!(
        !builder.is_empty(),
        "an outline-to-outline passage cut nothing"
    );
    // и проём сидит на южной грани — начинается на y = 0
    let bottom = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .fold(f32::INFINITY, f32::min);
    assert!(bottom.abs() < 0.01, "opening floats at y = {bottom}");
}

/// Середина прохода берётся по длине, а не по числу точек: у ломаной с
/// одним длинным и одним коротким сегментом это разные точки.
#[test]
fn the_passage_middle_is_measured_along_its_length() {
    let road = passage(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(90.0, 0.0),
            Vec2::new(100.0, 0.0),
        ],
        true,
    );
    let middle = passage_middle(&road).unwrap();
    assert!((middle.x - 50.0).abs() < 0.01, "{middle:?}");
}
