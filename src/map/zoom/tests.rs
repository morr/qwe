use super::*;

/// Таблица из трёх ступеней — достаточно, чтобы увидеть все три исхода
/// выбора: до первой границы, между границами и за последней.
enum Three {}

impl ZoomLods for Three {
    fn max_zooms() -> impl Iterator<Item = f32> {
        [1.0, 2.0, f32::INFINITY].into_iter()
    }
}

/// Зум ровно на границе попадает в верхнюю ступень, а за последней границей
/// остаётся последняя.
#[test]
fn bucket_is_the_first_lod_above_the_zoom() {
    assert_eq!(ZoomBucket::<Three>::for_zoom(0.0).index, 0);
    assert_eq!(ZoomBucket::<Three>::for_zoom(0.5).index, 0);
    assert_eq!(ZoomBucket::<Three>::for_zoom(1.0).index, 1);
    assert_eq!(ZoomBucket::<Three>::for_zoom(1.5).index, 1);
    assert_eq!(ZoomBucket::<Three>::for_zoom(2.0).index, 2);
    assert_eq!(ZoomBucket::<Three>::for_zoom(1e9).index, 2);
}

/// Ступени равны по индексу — `set_if_neq` в [`update_zoom_bucket`] держится
/// на этом.
#[test]
fn buckets_compare_by_index() {
    assert_eq!(
        ZoomBucket::<Three>::for_zoom(0.5),
        ZoomBucket::<Three>::for_zoom(0.9)
    );
    assert_ne!(
        ZoomBucket::<Three>::for_zoom(0.5),
        ZoomBucket::<Three>::for_zoom(1.0)
    );
}
