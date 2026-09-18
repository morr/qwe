//! Оба теневых слоя зданий — наземный и тот, что ложится на кровли соседей.
//!
//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе: каждый тест здесь строит освещённую геометрию, поэтому каждый
//! берёт `default_sun()` — гард, который держит солнце на дефолте и не пускает
//! к нему соседа (`map/sun.rs`).

use super::*;
use crate::map::buildings::fixtures::{building, ground_shadows, mesh_points, rect, square};
use crate::map::osm::AreaKind;

/// Тени на кровлях. `extruded` — 2.5D, и порядок отрисовки строится здесь
/// ровно потому, что в игре его делят меш экструзии и этот слой.
fn roof_shadows(list: &[PolyArea], extruded: bool) -> MeshBuilder {
    use crate::map::buildings::order::draw_order;

    let order = extruded.then(|| draw_order(list, Lean::of()));
    roof_shadow_builder(list, &ShadowSweeps::of(list), order.as_deref())
}

/// Тень высокого соседа ложится **на кровлю** низкого, и только на неё: весь
/// меш обязан лежать внутри контура низкого дома.
#[test]
fn a_tall_neighbour_shades_the_lower_roof() {
    let caster = building(
        vec![
            Vec2::new(100.0, 200.0),
            Vec2::new(130.0, 200.0),
            Vec2::new(130.0, 230.0),
            Vec2::new(100.0, 230.0),
        ],
        Some(40.0),
        AreaKind::Building,
    );
    // низкий сосед — по ходу тени от высокого (солнце с северо-запада)
    let along = shadow_dir() * 24.0;
    let low = building(
        vec![
            Vec2::new(100.0, 200.0) + along,
            Vec2::new(140.0, 200.0) + along,
            Vec2::new(140.0, 240.0) + along,
            Vec2::new(100.0, 240.0) + along,
        ],
        Some(4.0),
        AreaKind::Building,
    );
    let shaded = roof_shadows(&[caster.clone(), low.clone()], false);
    assert!(
        !shaded.is_empty(),
        "тень высокого соседа не легла на кровлю"
    );
    // по рамке контура, а не `point_in_area`: вершины пересечения ложатся
    // ровно **на** границу, а строгая проверка «внутри» их отвергает
    let min = low.outer.iter().copied().reduce(Vec2::min).expect("ring");
    let max = low.outer.iter().copied().reduce(Vec2::max).expect("ring");
    for point in shaded.positions_for_test() {
        let at = Vec2::new(point[0], point[1]);
        assert!(
            at.cmpge(min - 1e-3).all() && at.cmple(max + 1e-3).all(),
            "тень вышла за пятно низкого дома: {at} вне {min}..{max}"
        );
    }

    // а на кровлю самого высокого — не ложится ничья
    let alone = roof_shadows(&[caster], false);
    assert!(alone.is_empty());
}

/// Сосед той же высоты кровлю не темнит: иначе пара считалась бы для почти
/// каждой пары домов в городе.
#[test]
fn an_equal_neighbour_shades_nothing() {
    let a = building(square(), Some(20.0), AreaKind::Building);
    let mut b = a.clone();
    b.outer = a
        .outer
        .iter()
        .map(|point| *point + shadow_dir() * 12.0)
        .collect();
    assert!(roof_shadows(&[a, b], false).is_empty());
}

/// Прямоугольник в осях тени: `across` — поперёк `shadow_dir()`, `along` —
/// вдоль неё, обе пары в метрах от начала координат. Закрутка задаётся явно:
/// OSM обход колец не нормализует, а тень на кровле обязана выходить
/// одинаковой при любом.
fn shadow_rect(across: (f32, f32), along: (f32, f32), ccw: bool) -> Vec<Vec2> {
    let (side, ahead) = (shadow_dir().perp(), shadow_dir());
    let corner = |a: f32, b: f32| side * a + ahead * b;
    let mut ring = vec![
        corner(across.0, along.0),
        corner(across.1, along.0),
        corner(across.1, along.1),
        corner(across.0, along.1),
    ];
    if (signed_ring_area(&ring) > 0.0) != ccw {
        ring.reverse();
    }
    ring
}

/// Площадь тени на кровлях — тем же счётом по треугольникам, что у наземного
/// слоя.
fn roof_shadow_area(list: &[PolyArea]) -> f32 {
    shadow_area(&roof_shadows(list, false).build())
}

/// Два каста, чьи тени накрывают одну и ту же часть кровли, обязаны склеиться,
/// а не погасить друг друга: обход свипа наследует закрутку кольца OSM, и без
/// нормализации NonZero дал бы на перекрытии обмотку 0 — светлую дыру ровно
/// там, где тень должна быть сплошной.
#[test]
fn casters_of_opposite_winding_merge_instead_of_cancelling() {
    let _sun = crate::map::default_sun();
    // два дома по 60 м стоят друг за другом против солнца, их развёртки
    // накрывают одну полосу кровли низкого соседа: 12 м поперёк × 10 вдоль
    let near = |ccw| {
        building(
            shadow_rect((-6.0, 6.0), (-8.0, 0.0), ccw),
            Some(60.0),
            AreaKind::Building,
        )
    };
    let far = |ccw| {
        building(
            shadow_rect((-6.0, 6.0), (-30.0, -22.0), ccw),
            Some(60.0),
            AreaKind::Building,
        )
    };
    // низкий сосед — корпус с плоской кровлей: на скатную тень не кладётся
    let mut low = building(
        shadow_rect((-10.0, 10.0), (2.0, 12.0), true),
        Some(4.0),
        AreaKind::Building,
    );
    low.building_use = BuildingUse::Apartments;

    let same = roof_shadow_area(&[near(true), far(true), low.clone()]);
    let opposite = roof_shadow_area(&[near(true), far(false), low]);
    assert!(
        (same - 120.0).abs() < 0.5,
        "две тени на одной кровле обязаны дать её полосу целиком: {same} вместо 120"
    );
    assert!(
        (opposite - same).abs() < 0.5,
        "закрутка кольца OSM не смеет менять тень: {opposite} против {same}"
    );
}

/// Двор в контур дома не входит, и тень туда не ложится — даже когда кольцо
/// двора закручено так же, как внешнее (в OSM это половина случаев). Слой
/// лежит выше всех зданиевых, так что залитый двор был бы пятном поверх всего.
#[test]
fn a_courtyard_takes_no_roof_shadow() {
    let _sun = crate::map::default_sun();
    // широкий высокий сосед накрывает низкую кровлю 20 × 16 целиком
    let caster = building(
        shadow_rect((-25.0, 25.0), (-18.0, -12.0), true),
        Some(60.0),
        AreaKind::Building,
    );
    let with_courtyard = |ccw| {
        let mut low = building(
            shadow_rect((-10.0, 10.0), (0.0, 16.0), true),
            Some(4.0),
            AreaKind::Building,
        );
        low.holes.push(shadow_rect((-3.0, 3.0), (6.0, 12.0), ccw));
        low
    };
    let solid = building(
        shadow_rect((-10.0, 10.0), (0.0, 16.0), true),
        Some(4.0),
        AreaKind::Building,
    );

    let whole = roof_shadow_area(&[caster.clone(), solid]);
    assert!(
        (whole - 320.0).abs() < 0.5,
        "кровля затенена целиком: {whole}"
    );
    // двор 6 × 6 вычитается при любой закрутке своего кольца
    for ccw in [true, false] {
        let shaded = roof_shadow_area(&[caster.clone(), with_courtyard(ccw)]);
        assert!(
            (shaded - (whole - 36.0)).abs() < 0.5,
            "тень легла на двор (кольцо ccw = {ccw}): {shaded} вместо {}",
            whole - 36.0
        );
    }
}

/// Лежит ли точка внутри **нарисованного тела** прямоугольного дома — суммы
/// Минковского его контура с отрезком подъёма: есть ли доля подъёма
/// `t ∈ [0, 1]`, при которой точка попадает в сам контур. Обе координаты
/// подъёма в 2.5D строго положительны (`EXTRUDE_SKEW`, 1), поэтому деление
/// безопасно. Прямоугольник вызывающий сжимает сам: вершины разности ложатся
/// ровно **на** границу тела, и строгая проверка обязана их пропустить.
fn inside_drawn_body(point: Vec2, min: Vec2, max: Vec2, lift: Vec2) -> bool {
    let low = ((point - max) / lift).max_element().max(0.0);
    let high = ((point - min) / lift).min_element().min(1.0);
    low <= high
}

/// Слой теней на кровлях лежит над всеми зданиевыми, а painter's порядок 2.5D
/// живёт **внутри одного меша**: тень, посчитанная для дальней кровли, не
/// смеет лечь на нарисованное тело дома, который эту кровлю закрывает.
#[test]
fn a_nearer_body_eats_the_shadow_it_covers() {
    let _sun = crate::map::default_sun();
    let target = building(
        rect(Vec2::ZERO, Vec2::new(30.0, 30.0)),
        Some(4.0),
        AreaKind::Building,
    );
    // высокий каст стоит против солнца от цели и затеняет её кровлю почти
    // целиком. Своё нарисованное тело он уносит на северо-восток, кровли не
    // задевает, и в `draw_order` остаётся раньше цели: тень видна вся
    let caster = building(
        rect(Vec2::new(-40.0, 15.0), Vec2::new(-10.0, 45.0)),
        Some(60.0),
        AreaKind::Building,
    );
    // а третий дом стоит юго-западнее цели: в `draw_order` он позже неё, и
    // его поднятая кровля накрывает затенённую
    let cover = building(
        rect(Vec2::new(0.0, -30.0), Vec2::new(30.0, -2.0)),
        Some(60.0),
        AreaKind::Building,
    );

    let lit = roof_shadows(&[caster.clone(), target.clone()], true).build();
    let whole = shadow_area(&lit);
    assert!(
        (whole - 628.7).abs() < 0.5,
        "каст затеняет полосу кровли в 21 м шириной, срезанную наискось \
         дальним краем развёртки, и его собственное тело (рисуется раньше \
         цели) из неё не вычитается: {whole} вместо 628.7"
    );

    let shaded = roof_shadows(&[caster, target, cover.clone()], true).build();
    let left = shadow_area(&shaded);
    assert!(
        (left - 331.3).abs() < 1.0,
        "тело ближнего дома обязано съесть 297 м² тени: {left} из {whole}"
    );

    let lift = extrusion_lift(&cover, BuildingHeightMode::Extrusion);
    let (min, max) = (Vec2::new(0.0, -30.0), Vec2::new(30.0, -2.0));
    for point in mesh_points(&shaded) {
        assert!(
            !inside_drawn_body(point, min + 1e-2, max - 1e-2, lift),
            "тень легла на нарисованное тело ближнего дома: {point}"
        );
    }
}

#[test]
fn shadow_length_scales_with_height() {
    let _sun = crate::map::default_sun();
    let low = building(square(), Some(6.0), AreaKind::Building);
    let high = building(square(), Some(60.0), AreaKind::Building);
    let reach = |list: &[PolyArea]| {
        let mesh = ground_shadows(list, &[], false).build();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        positions
            .iter()
            .map(|p| Vec2::new(p[0], p[1]).dot(shadow_dir()))
            .fold(f32::NEG_INFINITY, f32::max)
    };
    assert!(reach(std::slice::from_ref(&high)) > reach(std::slice::from_ref(&low)) + 10.0);
}

#[test]
fn the_shadow_length_follows_the_sun_elevation() {
    let _sun = crate::map::default_sun();
    // 1 / tan 59° — то самое «0.6 метра тени на метр высоты», которое раньше
    // стояло константой без вывода
    let scale = crate::map::shadow_length_scale();
    assert!((scale - 0.6).abs() < 0.01, "{scale}");
}

/// Сумма площадей **непрозрачных** треугольников меша — тела тени, без
/// мягкого края: у каймы внутренние вершины сходят в нулевую альфу, и
/// треугольник каймы всегда несёт хотя бы одну такую. Двойное наложение
/// внутри тела дало бы сумму больше площади самой фигуры.
fn shadow_area(mesh: &Mesh) -> f32 {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .to_vec();
    let bevy::mesh::VertexAttributeValues::Float32x4(colors) =
        mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
    else {
        panic!("цвет вершины — Float32x4");
    };
    let indices: Vec<usize> = match mesh.indices().unwrap() {
        bevy::mesh::Indices::U32(list) => list.iter().map(|&i| i as usize).collect(),
        bevy::mesh::Indices::U16(list) => list.iter().map(|&i| i as usize).collect(),
    };
    indices
        .chunks_exact(3)
        .filter(|triangle| triangle.iter().all(|&i| colors[i][3] > 0.0))
        .map(|triangle| {
            let point = |i: usize| Vec2::new(positions[triangle[i]][0], positions[triangle[i]][1]);
            (point(1) - point(0)).perp_dot(point(2) - point(0)).abs() / 2.0
        })
        .sum()
}

/// Свип цепочки силуэта — то, из чего union собирает тело тени.
fn sweep_of(chain: &[Vec2], height: f32) -> Vec<Vec2> {
    let offset = shadow_dir() * height * crate::map::shadow_length_scale();
    let mut sweep = chain.to_vec();
    sweep.extend(chain.iter().rev().map(|point| *point + offset));
    sweep
}

#[test]
fn square_shadow_is_one_swept_polygon() {
    let _sun = crate::map::default_sun();
    // одна цепочка низ+право даёт свип из шести вершин — по вершине на угол
    // цепочки и столько же на сдвинутую копию, без квадов на ребро
    let chains = silhouette_chains(&square(), shadow_dir());
    assert_eq!(chains.len(), 1);
    assert_eq!(chains[0].len() * 2, 6);

    // и в самом слое тело тени — ровно этот свип: у одного дома объединять
    // нечего, а мягкий край альфу тела не трогает
    let list = [building(square(), Some(15.0), AreaKind::Building)];
    let mesh = ground_shadows(&list, &[], false).build();
    let expected = signed_ring_area(&sweep_of(&chains[0], 15.0)).abs();
    assert!((shadow_area(&mesh) - expected).abs() < 0.5, "{expected}");
}

#[test]
fn the_penumbra_stays_off_the_lit_side_and_softens_the_far_edge() {
    // тень примыкает к дому жёстко: метровая кайма по контуру примыкания
    // обводила дом мягким пятном с солнечной стороны — тем самым контактным
    // затенением, которое из объединения убрали
    let list = [building(square(), Some(15.0), AreaKind::Building)];
    let mesh = ground_shadows(&list, &[], false).build();
    let along = |points: &[Vec2]| {
        points
            .iter()
            .map(|point| point.dot(shadow_dir()))
            .fold((f32::MAX, f32::MIN), |(low, high), value| {
                (low.min(value), high.max(value))
            })
    };
    let (footprint_near, footprint_far) = along(&square());
    let (mesh_near, mesh_far) = along(&mesh_points(&mesh));
    assert!(
        mesh_near > footprint_near - 0.01,
        "кайма не заходит против света за контур дома: {mesh_near} против {footprint_near}"
    );

    // а дальний край, наоборот, размыт на всю ширину — с запасом на miter:
    // на прямом углу свипа офсет вершины длиннее ширины каймы в корень из двух
    let offset = 15.0 * crate::map::shadow_length_scale();
    let soft = mesh_far - (footprint_far + offset);
    assert!(
        (PENUMBRA_WIDTH..=PENUMBRA_WIDTH * 1.5).contains(&soft),
        "дальний край размыт на {soft} м вместо {PENUMBRA_WIDTH}"
    );
}

#[test]
fn staircase_shadow_has_no_double_darkening() {
    let _sun = crate::map::default_sun();
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
    let chains = silhouette_chains(&staircase, shadow_dir());
    assert_eq!(chains.len(), 1, "лестница — одна непрерывная цепочка");
    assert_eq!(chains[0].len(), 7);

    // свип цепочки самопересечься не может, поэтому его площадь — ровно
    // «длина сдвига × размах контура поперёк тени»
    let offset_length = 20.0 * crate::map::shadow_length_scale();
    let perp_span = Vec2::new(12.0, 9.0).dot(shadow_dir().perp());
    let sweep = sweep_of(&chains[0], 20.0);
    assert!((signed_ring_area(&sweep).abs() - offset_length * perp_span).abs() < 0.5);

    // и ровно столько же в слое: union не съел свип и не удвоил его
    let list = [building(staircase, Some(20.0), AreaKind::Building)];
    let mesh = ground_shadows(&list, &[], false).build();
    assert!((shadow_area(&mesh) - offset_length * perp_span).abs() < 0.5);
}

#[test]
fn neighbour_shadows_union_without_double_darkening() {
    let _sun = crate::map::default_sun();
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
        |b: &PolyArea| shadow_area(&ground_shadows(std::slice::from_ref(b), &[], false).build());
    let separate = alone(&left) + alone(&right);
    let together = shadow_area(&ground_shadows(&[left, right], &[], false).build());
    assert!(
        together < separate - 1.0,
        "union must remove the overlap: {together} vs {separate}"
    );
}
