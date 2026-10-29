use super::fixtures::{CENTER, CITY, WOOD_HALF, corners, tree_row, wood_scene};
use super::*;
use crate::map::osm::fixture::{Overpass, rect, square};
use crate::map::osm::model::distance_to_segment;
use crate::map::osm::planting::{
    TREE_CROWN_REACH, TREE_MIN_SPACING, TREE_SHORE_CLEARANCE, TREE_WALL_CLEARANCE, near_area_edge,
};
use bevy::math::Vec2;

#[test]
fn trees_are_deterministic_and_inside_the_wood() {
    let scene = wood_scene();
    let first = scene.parse();
    let second = scene.parse();

    assert!(!first.trees.is_empty());
    assert_eq!(first.trees, second.trees);
    for &(pos, radius) in first.trees.positions() {
        assert!(point_in_area(pos, &first.woods[0]), "{pos:?}");
        assert!((2.5..=4.0).contains(&radius));
    }
    for (index, &(pos, _)) in first.trees.positions().iter().enumerate() {
        for &(other, _) in &first.trees.positions()[index + 1..] {
            assert!(
                pos.distance(other) >= TREE_MIN_SPACING,
                "trees too close: {pos:?} vs {other:?}"
            );
        }
    }
}

#[test]
fn trees_avoid_a_pond_inside_the_wood() {
    let (.., ne, _) = corners(WOOD_HALF);
    // пруд занимает северо-восточную четверть массива
    let map = wood_scene()
        .area(&[("natural", "water")], rect(CENTER, ne))
        .parse();

    assert_eq!(map.water.len(), 1);
    assert!(!map.trees.is_empty());
    let pond = &map.water[0];
    for &(pos, _) in map.trees.positions() {
        assert!(!point_in_area(pos, pond), "tree in the pond at {pos:?}");
        assert!(
            !near_area_edge(pos, pond, TREE_SHORE_CLEARANCE),
            "tree on the shoreline at {pos:?}"
        );
    }
}

#[test]
fn trees_avoid_grass_and_sand_inside_the_wood() {
    let (sw, _, ne, nw) = corners(WOOD_HALF);
    // луг — восточная половина массива, песок — северо-западная четверть
    let map = wood_scene()
        .area(
            &[("landuse", "meadow")],
            rect(Vec2::new(CENTER.x, sw.y), ne),
        )
        .area(
            &[("natural", "beach")],
            rect(Vec2::new(nw.x, CENTER.y), Vec2::new(CENTER.x, ne.y)),
        )
        .parse();

    assert_eq!(map.woods.len(), 1);
    assert_eq!(map.grass.len(), 1);
    assert_eq!(map.sand.len(), 1);
    assert!(!map.trees.is_empty());
    for &(pos, _) in map.trees.positions() {
        assert!(
            !point_in_area(pos, &map.grass[0]),
            "tree on grass at {pos:?}"
        );
        assert!(!point_in_area(pos, &map.sand[0]), "tree on sand at {pos:?}");
    }
}

/// Дом посреди массива: крона не должна наползать на стену — раньше
/// проверка была только «центр внутри полигона», и дерево вырастало
/// впритык к фасаду (Тула, павильон в парке).
#[test]
fn trees_keep_the_crown_off_walls_and_kerbs() {
    let (sw, _, ne, _) = corners(WOOD_HALF);
    // дом — центральная четверть массива, дорожка режет массив по диагонали
    let map = wood_scene()
        .area(&[("building", "yes")], square(CENTER, WOOD_HALF / 4.0))
        .way(&[("highway", "footway")], vec![sw, ne])
        .parse();

    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.roads.len(), 1);
    assert!(!map.trees.is_empty());
    let house = &map.buildings[0];
    let path = &map.roads[0];
    for &(pos, radius) in map.trees.positions() {
        assert!(
            !point_in_area(pos, house),
            "tree inside the house at {pos:?}"
        );
        assert!(
            !near_area_edge(pos, house, radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE),
            "crown on the wall at {pos:?}"
        );
        let kerb = path
            .points
            .windows(2)
            .map(|segment| distance_to_segment(pos, segment[0], segment[1]))
            .fold(f32::INFINITY, f32::min)
            - path.width / 2.0;
        assert!(kerb > radius, "tree on the kerb at {pos:?}, gap {kerb}");
    }
}

/// Открытая часть парка — поле: деревья растут только в лесных полигонах,
/// парк без `natural=wood` остаётся пустым.
#[test]
fn a_park_without_wood_grows_no_trees() {
    let map = Overpass::new(CITY)
        .area(&[("leisure", "park")], square(CENTER, WOOD_HALF))
        .parse();

    assert_eq!(map.parks.len(), 1);
    assert!(map.woods.is_empty());
    assert!(map.trees.is_empty());
}

/// `natural=tree_row` доезжает до `MapData::tree_rows` и даёт деревья вдоль
/// полилинии. Тег в OSM всегда на way, кольцом почти не бывает — ряд разбирается
/// как открытая полилиния, а не как площадь.
#[test]
fn parses_tree_rows_and_plants_along_them() {
    let map = tree_row(&[("natural", "tree_row")]);

    assert_eq!(map.tree_rows.len(), 1);
    // шага в тегах нет — плотность берётся из ползунка, порог ненулевой
    assert!(map.tree_rows[0].spacing.is_none());
    let rows = map.row_trees.get(TreeRowLayout::default());
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|&(.., at)| at > 0.0));

    // деревья лежат на самом ряду, а не где придётся
    let row = &map.tree_rows[0];
    for &(pos, ..) in rows {
        assert!(distance_to_segment(pos, row.points[0], row.points[1]) < 1.0);
    }

    // и они же — в собранном наборе, который читает рендер
    assert_eq!(map.trees.len(), rows.len());
    assert_eq!(map.composed_for, Some(TreeCompose::default()));
}

/// Шаг из данных: `count` растягивается на длину ряда, а ползунок такой ряд не
/// прореживает — порог нулевой у всех его деревьев.
#[test]
fn tree_row_count_tag_fixes_the_spacing() {
    let map = tree_row(&[("natural", "tree_row"), ("count", "5")]);

    let row = &map.tree_rows[0];
    let length = row.points[0].distance(row.points[1]);
    let spacing = row.spacing.expect("count даёт шаг");
    assert!((spacing - length / 4.0).abs() < 1e-2);

    let rows = map.row_trees.get(TreeRowLayout::default());
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|&(.., at)| at == 0.0));
}

/// Мусорные значения тегов не должны становиться посадкой: `spacing=0.1`
/// смыкает кроны в сплошную кляксу, `count=1` не задаёт шага вовсе.
#[test]
fn implausible_tree_row_tags_fall_back_to_the_slider() {
    for tags in [
        &[("natural", "tree_row"), ("spacing", "0.1")],
        &[("natural", "tree_row"), ("count", "1")],
        &[("natural", "tree_row"), ("diameter_crown", "50")],
    ] {
        let map = tree_row(tags);
        assert!(map.tree_rows[0].spacing.is_none(), "{tags:?}");
        assert!(map.tree_rows[0].radius.is_none(), "{tags:?}");
    }
}

/// Нода `natural=tree` доезжает до `MapData::tree_nodes` и до собранного
/// набора, который читает рендер; `diameter_crown` задаёт радиус кроны, порог
/// нулевой — дерево из данных видно на любой плотности.
#[test]
fn parses_standalone_tree_nodes() {
    let map = Overpass::new(CITY)
        .node(&[("natural", "tree"), ("diameter_crown", "10")], CENTER)
        .parse();

    assert_eq!(map.tree_nodes.len(), 1);
    assert_eq!(map.tree_nodes[0].radius, Some(5.0));
    assert_eq!(map.trees.len(), 1);
    assert_eq!(map.trees.positions()[0].1, 5.0);
    assert_eq!(map.trees.appears_at(0), 0.0);
}
