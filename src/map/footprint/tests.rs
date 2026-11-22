use super::*;
use crate::map::osm::fixture;

/// Общий узел посреди одной из ways ловится с любой стороны — ровно ради
/// этого случая предикат симметричен.
#[test]
fn a_way_ending_in_the_middle_of_another_still_joins_it() {
    let through = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
    let stub = [Vec2::new(50.0, 0.0), Vec2::new(50.0, 40.0)];
    assert!(ways_joined(&through, &stub));
    assert!(ways_joined(&stub, &through));
}

/// Тропа, прошедшая под пролётом, узла не делит: примыкание — это общая
/// точка, а не близость в плане.
#[test]
fn a_way_passing_by_does_not_join() {
    let bridge = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
    let under = [Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)];
    assert!(!ways_joined(&bridge, &under));
}

/// Отбор примыкающих: общий узел — да; проход ПОД пролётом — нет, хотя
/// коробки у него и у моста совпадают (решает по-прежнему `ways_joined`,
/// а не префильтр); дальняя дорога — нет.
#[test]
fn coverage_takes_only_ways_sharing_a_node_with_a_bridge() {
    let roads = vec![
        fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
        fixture::street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 60.0)], 8.0),
        fixture::street(vec![Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)], 3.5),
        fixture::street(vec![Vec2::new(500.0, 500.0), Vec2::new(600.0, 500.0)], 8.0),
    ];
    let coverage = CurbCoverage::build(&roads);
    assert_eq!(coverage.bridges().len(), 1);
    assert_eq!(coverage.joining().len(), 1);
    assert_eq!(coverage.joining()[0].points[0], Vec2::new(100.0, 0.0));
}

/// Отбор идёт с запасом [`JOIN_EPSILON`] на обеих сторонах: дорога,
/// начинающаяся в 0.4 м от торца моста, примыкает — хотя её коробка
/// коробку моста не пересекает, и любой отбор по голому AABB её потеряет.
#[test]
fn a_way_ending_just_short_of_a_bridge_still_joins_it() {
    let roads = vec![
        fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
        fixture::street(vec![Vec2::new(100.4, 0.0), Vec2::new(200.0, 0.0)], 8.0),
    ];
    assert_eq!(CurbCoverage::build(&roads).joining().len(), 1);
}

#[test]
fn casing_stays_within_its_range_on_every_road_class() {
    for width in [3.5_f32, 5.0, 8.0, 16.0] {
        assert!(casing_width(width) >= *CASING_RANGE.start());
        assert!(casing_width(width) <= *CASING_RANGE.end());
    }
}

/// Бордюр обязан торчать из-под канта на любом классе — иначе при
/// включённом канте мост неотличим от окантованной дороги.
#[test]
fn bridge_curb_is_thicker_than_a_casing() {
    for width in [3.5_f32, 5.0, 8.0, 16.0] {
        assert!(bridge_curb_width(width) > casing_width(width));
        assert!(bridge_curb_width(width) >= *BRIDGE_CURB_RANGE.start());
        assert!(bridge_curb_width(width) <= *BRIDGE_CURB_RANGE.end());
    }
}

/// Кромки бордюрных полос отстоят от осевой ровно на полширины настила
/// плюс полбордюра — та самая конструкция, которой пользуются обе заливки
/// и рендер.
#[test]
fn curb_bands_sit_on_both_deck_edges() {
    let road = fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0);
    let [left, right] = road.curb_bands();
    let offset = (road.width + road.curb_width()) / 2.0;
    assert_eq!(left.width, road.curb_width());
    assert_eq!(left.line[0].y, -offset);
    assert_eq!(right.line[0].y, offset);
    assert_eq!(
        road.curb_reach(),
        road.width / 2.0 + road.curb_width(),
        "щуп сетки и разность меша меряют одну и ту же полуширину"
    );
}
