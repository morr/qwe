//! Общие правила тени.
//!
//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе, поэтому каждый берёт гард `default_sun()` / `sun_at()`
//! (`map/sun.rs`). Это и есть записанная цена того, что свет глобаль, а не
//! аргумент.

use super::*;
use crate::map::{default_sun, sun_at};

/// Длина тени — котангенс высоты солнца, и она общая для всех слоёв: у дома,
/// забора и вагона один множитель на метр высоты.
#[test]
fn the_length_is_the_cotangent_of_the_sun() {
    let _sun = default_sun();
    // дефолт — 59°, котангенс ≈ 0.6
    assert!((length(10.0) - 6.0).abs() < 0.05, "{}", length(10.0));
    assert!(
        (length(20.0) - 2.0 * length(10.0)).abs() < 1e-3,
        "не линейна"
    );
    assert_eq!(length(0.0), 0.0);
}

/// Низкое солнце удлиняет тень всем сразу — ровно то свойство, из-за которого
/// длина, написанная мимо этого модуля, верна на дефолте и неверна на концах
/// ползунка.
#[test]
fn a_lower_sun_lengthens_every_shadow() {
    let high = {
        let _sun = sun_at(300.0, 59.0);
        length(10.0)
    };
    let low = {
        let _sun = sun_at(300.0, 15.0);
        length(10.0)
    };
    // `cot 15° / cot 59°` ≈ 6.2
    assert!(low > high * 6.0 && low < high * 6.5, "{high} -> {low}");
}

/// Сдвиг — та же длина, но вектором по свету, то есть **от** солнца, и модуль
/// азимута он держит.
#[test]
fn the_offset_points_away_from_the_sun() {
    for azimuth in [0.0, 90.0, 300.0] {
        let _sun = sun_at(azimuth, 59.0);
        let shift = offset(10.0);
        assert!((shift.length() - length(10.0)).abs() < 1e-3, "{azimuth}");
        assert!(
            shift.normalize().dot(crate::map::shadow_dir()) > 0.999,
            "{azimuth}: сдвиг не по свету"
        );
    }
}

/// Полутень: ноль там, где тень встречает предмет, полная ширина на дальнем
/// краю, и рост между ними. Это то правило, что жило тремя копиями.
#[test]
fn the_penumbra_is_zero_at_the_contact_and_full_at_the_far_edge() {
    let _sun = default_sun();
    let light = crate::map::shadow_dir();

    assert!(
        (penumbra(light) - 1.0).abs() < 1e-3,
        "дальний край не полный"
    );
    assert_eq!(penumbra(-light), 0.0, "у контакта кайма не нулевая");
    // поперёк света кайма уже нулевая: доля — проекция на свет, а не доля угла,
    // и рост от нуля к единице идёт по той половине круга, что смотрит по свету
    let across = light.perp();
    assert!(penumbra(across).abs() < 1e-3);
    // и ничего отрицательного: ширина каймы — доля, а не знак
    for step in 0..16 {
        let angle = step as f32 * std::f32::consts::TAU / 16.0;
        assert!(penumbra(Vec2::from_angle(angle)) >= 0.0);
    }
}

/// Объединение снимает двойную темноту: два наложившихся квадрата дают одну
/// фигуру, а не две залитые поверх друг друга.
#[test]
fn overlapping_sweeps_are_unioned_into_one_body() {
    let _sun = default_sun();
    let square = |at: Vec2, half: f32| -> Vec<[f32; 2]> {
        vec![
            (at + Vec2::new(-half, -half)).to_array(),
            (at + Vec2::new(half, -half)).to_array(),
            (at + Vec2::new(half, half)).to_array(),
            (at + Vec2::new(-half, half)).to_array(),
        ]
    };

    let mut apart = MeshBuilder::default();
    push_union(
        &mut apart,
        &[square(Vec2::ZERO, 5.0), square(Vec2::new(100.0, 0.0), 5.0)],
        1.0,
    );
    let mut overlapping = MeshBuilder::default();
    push_union(
        &mut overlapping,
        &[square(Vec2::ZERO, 5.0), square(Vec2::new(5.0, 0.0), 5.0)],
        1.0,
    );

    assert!(!apart.is_empty());
    assert!(
        overlapping.vertex_count() < apart.vertex_count(),
        "перекрытие не слилось: {} против {}",
        overlapping.vertex_count(),
        apart.vertex_count()
    );
}

/// Пустой список свипов — пустой меш, а не фигура нулевой площади: слой без
/// теней (выключенный, за порогом зума) проходит через ту же дверь.
#[test]
fn nothing_to_cast_draws_nothing() {
    let _sun = default_sun();
    let mut builder = MeshBuilder::default();
    push_union(&mut builder, &[], 1.0);
    assert!(builder.is_empty());
}

/// Тайловый union (вход выше порога) сшивает цепочку перекрывающихся свипов
/// в одну полосу — и два прогона по одному входу дают один и тот же меш:
/// группировка по клеткам детерминирована, порядок клеток — ключом, а не
/// хешем.
#[test]
fn the_tiled_union_merges_into_one_body_and_is_deterministic() {
    let _sun = default_sun();
    let square = |at: Vec2, half: f32| -> Vec<[f32; 2]> {
        vec![
            (at + Vec2::new(-half, -half)).to_array(),
            (at + Vec2::new(half, -half)).to_array(),
            (at + Vec2::new(half, half)).to_array(),
            (at + Vec2::new(-half, half)).to_array(),
        ]
    };
    // цепочка из 600 перекрывающихся квадратов — вход выше `TILED_MIN`,
    // объединение идёт тайлами; соседние квадраты налегают на 2 м
    let contours: Vec<Vec<[f32; 2]>> = (0..600)
        .map(|step| square(Vec2::new(step as f32 * 8.0, 0.0), 5.0))
        .collect();

    let mut first = MeshBuilder::default();
    push_union(&mut first, &contours, 1.0);
    let mut second = MeshBuilder::default();
    push_union(&mut second, &contours, 1.0);
    assert_eq!(
        first.colored_triangles_for_test().collect::<Vec<_>>(),
        second.colored_triangles_for_test().collect::<Vec<_>>(),
        "тайловый union недетерминирован"
    );
    assert!(!first.is_empty());

    // вся цепочка слилась в одну полосу: концы и стыки покрыты, в стороне —
    // ничего
    assert!(first.covers_for_test(Vec2::ZERO));
    assert!(first.covers_for_test(Vec2::new(599.0 * 8.0, 0.0)));
    assert!(first.covers_for_test(Vec2::new(4.0, 0.0)));
    assert!(!first.covers_for_test(Vec2::new(300.0, 100.0)));
}

/// Плотная группа делится на квадранты: листьев много, каждый — не больше
/// `DENSE_MIN`; вырожденный вход (все первые точки в одном квадранте на
/// каждой глубине) доходит до дна решётки и возвращается одним листом.
#[test]
fn a_dense_group_splits_into_quadrants() {
    let square_at = |at: Vec2| -> Vec<[f32; 2]> {
        vec![
            (at + Vec2::new(-1.0, -1.0)).to_array(),
            (at + Vec2::new(1.0, -1.0)).to_array(),
            (at + Vec2::new(1.0, 1.0)).to_array(),
            (at + Vec2::new(-1.0, 1.0)).to_array(),
        ]
    };
    // сетка 30×25 с шагом 6 м: 750 контуров в одной клетке, гребёнка
    // квадрантов делит её уже на первом уровне
    let cluster: Vec<Vec<[f32; 2]>> = (0..30)
        .flat_map(|x| {
            (0..25).map(move |y| square_at(Vec2::new(x as f32 * 6.0, y as f32 * 6.0)))
        })
        .collect();
    let leaves = split_dense(cluster, |contour| Vec2::from_array(contour[0]));
    assert!(leaves.len() > 1, "плотная группа не разделилась");
    assert!(leaves.iter().all(|leaf| leaf.len() <= DENSE_MIN));
    assert_eq!(leaves.iter().map(Vec::len).sum::<usize>(), 750);

    // все первые точки в одном квадранте на каждой глубине: деление доходит
    // до дна решётки и отдаёт группу целиком, не зацикливаясь
    let pile: Vec<Vec<[f32; 2]>> = (0..DENSE_MIN + 10)
        .map(|step| square_at(Vec2::new(step as f32 * 0.001, step as f32 * 0.001)))
        .collect();
    let leaves = split_dense(pile, |contour| Vec2::from_array(contour[0]));
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].len(), DENSE_MIN + 10);
}

/// Дроблёная плотная клетка сливает тот же набор, что и недроблёная:
/// покрытие совпадает, два прогона дроблёного пути дают одно и то же, а
/// вход без плотных клеток дробление не меняет вовсе — группировка и
/// порядок те же, на которых сидит полимеш.
#[test]
fn the_dense_split_unions_the_same_set_and_stays_deterministic() {
    let _sun = default_sun();
    let square = |at: Vec2, half: f32| -> Vec<[f32; 2]> {
        vec![
            (at + Vec2::new(-half, -half)).to_array(),
            (at + Vec2::new(half, -half)).to_array(),
            (at + Vec2::new(half, half)).to_array(),
            (at + Vec2::new(-half, half)).to_array(),
        ]
    };
    // 750 налегающих квадратов (шаг 6 м при размере 10 м) в одной клетке
    let cluster: Vec<Vec<[f32; 2]>> = (0..30)
        .flat_map(|x| {
            (0..25).map(move |y| square(Vec2::new(x as f32 * 6.0, y as f32 * 6.0), 5.0))
        })
        .collect();

    let mesh_of = |shapes: &[Vec<Vec<[f32; 2]>>]| {
        let mut builder = MeshBuilder::default();
        for shape in shapes {
            let mut rings = shape.iter().map(|contour| {
                contour
                    .iter()
                    .map(|point| Vec2::from_array(*point))
                    .collect::<Vec<Vec2>>()
            });
            let Some(outer) = rings.next() else {
                continue;
            };
            let holes: Vec<Vec<Vec2>> = rings.collect();
            builder.push_polygon(&outer, &holes, SHADOW_COLOR.to_linear());
        }
        builder
    };
    let whole = mesh_of(&union_of(&cluster));
    let split = mesh_of(&union_of_dense(&cluster));
    // и внутри кластера, и на его пустой окраине покрытие одно
    for point in [
        Vec2::new(3.0, 3.0),
        Vec2::new(87.0, 72.0),
        Vec2::new(174.0, 144.0),
    ] {
        assert_eq!(
            whole.covers_for_test(point),
            split.covers_for_test(point),
            "{point}: покрытие разошлось"
        );
    }
    assert!(!whole.covers_for_test(Vec2::new(300.0, 300.0)));
    assert!(!split.covers_for_test(Vec2::new(300.0, 300.0)));

    assert_eq!(
        union_of_dense(&cluster),
        union_of_dense(&cluster),
        "дроблёный union недетерминирован"
    );

    // без плотных клеток дробление не меняет путь: группировка по клеткам и
    // порядок клеток те же, объединение и его результат байт в байт
    let spread: Vec<Vec<[f32; 2]>> = (0..600)
        .map(|step| square(Vec2::new(step as f32 * 8.0, 0.0), 5.0))
        .collect();
    assert_eq!(union_of(&spread), union_of_dense(&spread));
}
