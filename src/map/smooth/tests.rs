use super::*;
use crate::map::meshing::distance_to_path;

#[test]
fn chaikin_keeps_endpoints() {
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, 3.5, |_| false);
    assert_eq!(smoothed[0], original[0]);
    assert_eq!(smoothed[smoothed.len() - 1], original[original.len() - 1]);
}

#[test]
fn chaikin_deviation_is_bounded_by_width() {
    // длинные сегменты: без ограничения шириной срез ушёл бы на 5 м от угла
    let width = 3.5;
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, width, |_| false);
    // сами точки среза лежат на исходных сегментах
    for point in &smoothed {
        assert!(distance_to_path(*point, &original) < 1e-4);
    }
    // а хорда, заменившая угол, отходит от него не дальше ширины дороги
    let deviation = smoothed
        .windows(2)
        .map(|segment| distance_to_path(segment[0].midpoint(segment[1]), &original))
        .fold(0.0_f32, f32::max);
    assert!(deviation <= width, "chaikin drifted {deviation} m");
}

#[test]
fn chaikin_leaves_straight_runs_alone() {
    // изломы по 2° мельче MIN_SMOOTH_ANGLE — ломаная возвращается как есть
    let step = 10.0 * 2.0_f32.to_radians().tan();
    let original = vec![
        Vec2::ZERO,
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, step),
        Vec2::new(30.0, step * 2.0),
    ];
    assert_eq!(chaikin(&original, 3.5, |_| false), original);
}

#[test]
fn smooth_path_applies_where_nothing_is_pinned() {
    // `smooth_path` — общий вход для рельсов, трамвая и зелёной полосы: в
    // отличие от `roads::centerline` ему нечего закреплять, и сглаживание он
    // применяет всегда
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    assert!(smooth_path(&points, 5.0, Smoothing::Strong).len() > 3);
    assert!(matches!(
        smooth_path(&points, 5.0, Smoothing::Off),
        Cow::Borrowed(_)
    ));
}
