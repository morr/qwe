use super::arches::*;
use super::layers::*;
use super::*;
use crate::map::SHADOW_DIR;
use crate::map::osm::RoadClass;
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
fn extrusion_walls_face_south_only() {
    // у квадрата при подъёме строго вверх видима одна южная стена
    let edges = silhouette_edges(&square(), Vec2::NEG_Y);
    assert_eq!(edges.len(), 1);
    let (a, b) = edges[0];
    assert_eq!(a.y, 0.0);
    assert_eq!(b.y, 0.0);
}

#[test]
fn extrusion_sorts_north_first() {
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
    // порядок входа не важен: painter's sort всегда пишет север первым,
    // поэтому буферы вершин совпадают, а первая вершина — северная
    assert_eq!(sorted, reversed);
    assert!(
        sorted[0][1] >= 100.0,
        "north building must be written first"
    );
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
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    // дорога идёт наискось и коротка: до стены дотягивается один конец
    let slanted = passage(vec![Vec2::new(4.0, -2.0), Vec2::new(9.0, 20.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&slanted], lift);

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
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    // дорога упирается в южную грань в метре от юго-западного угла
    let road = passage(vec![Vec2::new(1.0, 0.0), Vec2::new(1.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], lift);
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
