use super::super::fixtures::{
    a_tee, ground_with_roads, layer, one_street, roundabout_with_an_approach,
};
use super::super::*;
use super::support::extent_x;

/// Ручки формы доходят до геометрии: радиус угла растягивает скругления
/// бордюра, нулевой допуск оставляет ось по точкам OSM.
#[test]
fn the_shape_knobs_move_the_geometry() {
    let map = a_tee();
    let verts = |shape: RoadShape| {
        let (layers, report) = mesh_roads(
            &map,
            RoadStyle::default(),
            shape,
            VergeZoomBucket::for_zoom(0.0),
        );
        (layer(&layers, "roads").builder.vertex_count(), report)
    };
    let (_, base) = verts(RoadShape::default());
    assert!(base.kerb_returns > 0, "{base}");
    let (tight, _) = verts(RoadShape {
        corner_radius: 0.5,
        ..default()
    });
    let (wide, _) = verts(RoadShape {
        corner_radius: 2.0,
        ..default()
    });
    assert_ne!(tight, wide, "радиус угла меняет дуги скруглений");
}

#[test]
fn the_sidewalk_knob_fills_the_sidewalk_layer() {
    let map = one_street();
    let sidewalk_verts = |sidewalks| {
        let style = RoadStyle {
            sidewalks,
            ..RoadStyle::default()
        };
        let (layers, _) = mesh_roads(
            &map,
            style,
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        );
        layer(&layers, "sidewalks").builder.vertex_count()
    };

    assert_eq!(sidewalk_verts(false), 0);
    assert!(sidewalk_verts(true) > 0);
}

#[test]
fn a_road_through_a_big_lot_is_drawn_over_it_with_a_kerb() {
    let (layers, _) = mesh_roads(
        &ground_with_roads(100.0),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    // бордюр — только у сквозной дороги и только у площадки: за её контур он
    // выпущен на два метра, до тротуара улицы, и не дальше
    let kerb = &layer(&layers, "lot_sidewalks").builder;
    let (low, high) = extent_x(kerb);
    assert!(
        (97.9..98.5).contains(&low) && (201.5..=202.1).contains(&high),
        "{low}..{high}"
    );
    // проезд ряда прорезал в бордюре устье, асфальт самой дороги — полосу
    let middle = 150.0;
    for at in kerb.positions_for_test() {
        assert!(
            (at[0] - middle).abs() > 1.5,
            "бордюр в устье проезда: {at:?}"
        );
        assert!((at[1] - middle).abs() > 2.0, "бордюр на полотне: {at:?}");
    }
    // дорога одна — разделительной нет
    assert!(layer(&layers, "lot_lines").builder.is_empty());
    // сама улица в своём слое осталась целой: за площадкой её рисует он
    let (low, high) = extent_x(&layer(&layers, "roads").builder);
    assert!(low < 1.0 && high > 299.0, "{low}..{high}");
}

#[test]
fn a_small_lot_still_hides_every_road_on_it() {
    // 60 × 60 — двор: его асфальт и есть проезд, поверх него ничего не кладётся
    let (layers, _) = mesh_roads(
        &ground_with_roads(60.0),
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
    assert!(layer(&layers, "lot_lines").builder.is_empty());
}

#[test]
fn the_markings_knob_takes_the_hatching_off() {
    let style = RoadStyle {
        markings: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(
        &roundabout_with_an_approach(true, true),
        style,
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

#[test]
fn the_sidewalk_knob_takes_the_kerb_off_the_lot_road_too() {
    let style = RoadStyle {
        sidewalks: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(
        &ground_with_roads(100.0),
        style,
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
}
