use super::super::fixtures::{a_tee, junctions_with, layer, one_street, with_network};
use super::super::*;
use crate::map::osm::model::RoadNode;
use crate::map::osm::{Highway, fixture};

#[test]
fn junctions_are_counted_with_markings_off_too() {
    // перекрёсток гасит и колею асфальта, а она есть без разметки
    assert_eq!(junctions_with(&a_tee(), false), 1);
    assert_eq!(junctions_with(&a_tee(), true), 1);
}

#[test]
fn a_crossing_without_a_shared_node_is_not_a_junction() {
    let mut map = one_street();
    // улица пересекает первую геометрически, но общей вершины у них нет —
    // так в OSM выглядит мост над улицей, и рвать линии он не должен
    map.roads.push(fixture::street(
        vec![Vec2::new(350.0, -100.0), Vec2::new(350.0, 400.0)],
        10.0,
    ));

    assert_eq!(
        junctions_with(&map, true),
        0,
        "перекрёсток восстанавливается по общей ноде, а не по пересечению"
    );
}

/// Зебра по правилу — на плече узла двух улиц с тротуарами **по тегу**
/// (`RoadLine::sidewalks`, как карман у `pockets::kerb_parking`), а не по
/// ручке «Sidewalks»: та лишь прячет ленту, у зебры своя ручка `crossings`.
#[test]
fn rule_zebras_do_not_follow_the_sidewalk_knob() {
    let mut map = a_tee();
    // зебра по правилу — только у улицы не ниже `tertiary`
    map.roads[0].highway = Highway::Tertiary;
    let zebras = |sidewalks| {
        let style = RoadStyle {
            sidewalks,
            ..RoadStyle::default()
        };
        mesh_roads(
            &map,
            style,
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        )
        .1
        .junctions
        .zebras[0]
    };

    assert!(zebras(true) > 0, "у тройника есть зебра по правилу");
    assert_eq!(
        zebras(false),
        zebras(true),
        "ручка тротуаров зебру не трогает"
    );
}

/// `highway=turning_circle` на торце тупика — круг асфальта шире дороги.
#[test]
fn a_turning_circle_widens_the_dead_end() {
    let mut map = one_street();
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(600.0, 100.0),
        kind: RoadNodeKind::TurningCircle,
        osm_id: None,
    });
    let (layers, report) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert_eq!(report.turning_circles, 1);
    let radius = turning_radius(12.0);
    assert!(radius > 6.0);
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (at[1] - (100.0 + radius)).abs() < 0.01 && (at[0] - 600.0).abs() < 1.0)
    );
}

/// Планка зебры рвёт и **чужие** линии: разрыв получала только своя дорога,
/// и у Y-слияния двух односторонних в двустороннюю (Тула, у кольца, N6) ветки
/// начинались короткими косыми обрывками прямо на планке, стоящей в метрах
/// от узла слияния. Углы веток и главной — как у той развилки: въезд уходит
/// из узла на 121°, выезд на 72°, главная на 283°; переход стоит в узле
/// слияния, и зебра вытягивается на продолжение в 3 м от узла
#[test]
fn a_zebra_plank_cuts_the_lines_of_crossing_carriageways() {
    use crate::map::osm::model::{RoadNode, RoadNodeKind};
    let one_way = |points: Vec<Vec2>| RoadLine {
        oneway: true,
        lanes: Some(2),
        highway: Highway::Primary,
        ..fixture::street(points, 7.6)
    };
    let mut map = with_network(vec![
        RoadLine {
            highway: Highway::Primary,
            lanes: Some(4),
            ..fixture::street(vec![Vec2::new(18.0, -77.9), Vec2::new(0.0, 0.0)], 14.2)
        },
        one_way(vec![Vec2::new(-17.2, 28.5), Vec2::new(0.0, 0.0)]),
        one_way(vec![Vec2::new(0.0, 0.0), Vec2::new(12.4, 38.0)]),
    ]);
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(0.0, 0.0),
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
        osm_id: None,
    });
    let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    let junctions = junctions::Junctions::new(
        &drawn,
        &map,
        &[],
        node_paint::NodePaintStyle {
            crossings: CrossingMode::Osm,
            stop_lines: false,
        },
    );
    let paint = junctions.node_paint();
    // своя дорога планку режет
    let own = paint.lines().of(0).cut.len();
    assert!(own > 0, "у главной с зеброй разрыва нет");
    // узел веток — слияние: разрывов асфальта в узле нет (тупиковой с нулевой
    // досягаемостью на другом конце — можно), линии веток идут сквозь, и рвать
    // их могло только полотно главной через планку
    for road in [1, 2] {
        let axis = drawn.axis(road, Axis::Ribbon);
        assert!(
            !paint
                .asphalt()
                .of(road)
                .iter()
                .any(|found| { found.reach > 0.0 && found.at.distance(Vec2::ZERO) < 10.0 }),
            "узел ветки {road} — слияние, разрыва асфальта в нём быть не должно"
        );
        // разрыв от планки: недалеко от неё, короткий (узловой был бы в
        // полуширины главной) — и накрывает планку интервалом. Планка в узле
        // слияния вытягивается на продолжение: центр в 3 м от узла по главной
        let plank = Vec2::new(0.67, -2.92);
        let cut = paint
            .lines()
            .of(road)
            .cut
            .iter()
            .find(|found| found.reach < 5.0 && found.at.distance(plank) < 12.0)
            .unwrap_or_else(|| {
                panic!(
                    "у ветки {road} нет разрыва на планке: cuts {:?}",
                    paint.lines().of(road).cut
                )
            });
        let along = |point: Vec2| crate::map::along::nearest_on_path(axis, point).unwrap().1;
        let plank_along = along(plank);
        let cut_along = along(cut.at);
        assert!(
            cut_along - cut.reach <= plank_along + 0.05
                && plank_along <= cut_along + cut.reach + 0.05,
            "разрыв ветки {road} не накрывает планку: {cut:?}, планка на {plank_along}"
        );
    }
}

#[test]
fn an_arm_is_filled_before_its_leader_even_when_it_leads_elsewhere() {
    use node_paint::{Junction, JunctionArm, PaintJunctions};
    // 0 — главная, ведёт узел; 1 — примыкание шире неё, что само ведёт
    // другой узел дальше: по одному ключу «ведущие последними, узкие раньше»
    // оно ложилось бы на главную
    let widths = [5.0, 10.0, 8.0];
    let leading = [true, true, false];
    let arm = |road| JunctionArm {
        road,
        edge: 0.0,
        dir: 1.0,
        link: false,
    };
    let junctions = [Junction {
        arms: vec![arm(0), arm(0), arm(1)],
        leading: vec![0],
    }];
    assert_eq!(
        fill_order(&widths, &leading, PaintJunctions::of(&junctions)),
        vec![2, 1, 0]
    );
    // без узла — прежний ключ
    assert_eq!(
        fill_order(&widths, &leading, PaintJunctions::of(&[])),
        vec![2, 0, 1]
    );
}
