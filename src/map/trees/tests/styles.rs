//! Ручки стилей: выбор породы формой кроны и квантование яркости по колоколу.

use super::super::*;

#[test]
fn only_mixed_listens_to_the_conifer_field() {
    let _sun = crate::map::default_sun();
    assert_eq!(TreeShape::Mixed.resolve(true), TreeShape::Conifer);
    assert_eq!(TreeShape::Mixed.resolve(false), TreeShape::Cotton);
    for shape in TreeShape::CONCRETE {
        assert_eq!(shape.resolve(true), shape);
        assert_eq!(shape.resolve(false), shape);
    }
}

/// Шаг 7 по пяти оттенкам перебирает их все: иначе лес пошёл бы полосами по
/// яркости вместо перемешанных крон.
#[test]
fn tint_slots_cover_every_shade() {
    let _sun = crate::map::default_sun();
    let slots: Vec<usize> = (0..5).map(TreeStyle::tint_slot).collect();
    let mut distinct = slots.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        5,
        "шаг 7 не перебирает все оттенки: {slots:?}"
    );
}

/// Нулевой разброс — лес одного цвета; игровой дефолт даёт растущий по яркости
/// ряд множителей.
#[test]
fn zero_variance_flattens_the_tints() {
    let _sun = crate::map::default_sun();
    let flat = TreeStyle {
        variance: 0.0,
        ..default()
    };
    assert_eq!(flat.tint_factors(), [1.0; 5]);
    let spread = TreeStyle::default().tint_factors();
    assert!(
        spread.windows(2).all(|pair| pair[0] < pair[1]),
        "множители должны расти от тёмного к светлому: {spread:?}"
    );
}
