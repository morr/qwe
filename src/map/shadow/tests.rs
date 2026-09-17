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

/// Сдвиг — та же длина, но в сторону солнца, и модуль азимута он держит.
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
    // боковая сторона — ровно посередине между ними
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
        vec![square(Vec2::ZERO, 5.0), square(Vec2::new(100.0, 0.0), 5.0)],
        1.0,
    );
    let mut overlapping = MeshBuilder::default();
    push_union(
        &mut overlapping,
        vec![square(Vec2::ZERO, 5.0), square(Vec2::new(5.0, 0.0), 5.0)],
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
    push_union(&mut builder, Vec::new(), 1.0);
    assert!(builder.is_empty());
}
