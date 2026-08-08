use super::*;
use crate::map::osm::model::{AreaKind, RoadClass, RoadLine, TreeCompose, TreeNode, TreeRow};
use crate::settings::TREE_DENSITY_MIN;

/// Прямой ряд длиной `length` вдоль оси X, на высоте 1000 — подальше от
/// края карты и от чего бы то ни было ещё.
fn straight_row(length: f32, spacing: Option<f32>) -> TreeRow {
    TreeRow {
        points: vec![
            Vec2::new(1000.0, 1000.0),
            Vec2::new(1000.0 + length, 1000.0),
        ],
        spacing,
        radius: None,
    }
}

fn map_with_row(row: TreeRow) -> MapData {
    MapData {
        tree_rows: vec![row],
        ..MapData::default()
    }
}

/// Посадка рядов при раскладке «шаг из OSM, если он есть» — то, что и
/// работает по умолчанию.
fn plant(map: &MapData, placement: TreeRowPlacement) -> Vec<PlantedTree> {
    plant_as(
        map,
        TreeRowLayout {
            placement,
            osm_spacing: true,
        },
    )
}

fn plant_as(map: &MapData, layout: TreeRowLayout) -> Vec<PlantedTree> {
    let obstacles = Obstacles::build(map);
    plant_rows(map, &obstacles, Occupied::default(), layout)
}

/// Сколько деревьев видно на такой плотности — то же, что делает
/// `map::trees::visible_count`, но без ползунка.
fn visible(planted: &[PlantedTree], density: f32) -> Vec<Vec2> {
    planted
        .iter()
        .filter(|&&(.., at)| at <= density)
        .map(|&(pos, ..)| pos)
        .collect()
}

#[test]
fn polyline_length_sums_segments() {
    let points = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
    assert!((polyline_length(&points) - 7.0).abs() < 1e-3);
}

#[test]
fn point_at_arc_length_walks_the_corner() {
    let points = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
    assert!(point_at_arc_length(&points, 4.0).distance(Vec2::new(4.0, 0.0)) < 1e-3);
    // за угол: 6 м по первому сегменту и ещё 4 по второму
    assert!(point_at_arc_length(&points, 14.0).distance(Vec2::new(10.0, 4.0)) < 1e-3);
    // за концом ряда — последняя точка, а не экстраполяция
    assert!(point_at_arc_length(&points, 99.0).distance(Vec2::new(10.0, 10.0)) < 1e-3);
}

/// Смысл обратных битов: префикс рангов рассыпан по всему ряду. Натуральный
/// порядок дал бы четыре первых слота подряд — то есть четверть аллеи и три
/// четверти пустоты.
#[test]
fn scattered_ranks_spread_the_prefix() {
    let ranks = scattered_ranks(16);
    let mut prefix: Vec<usize> = (0..16).filter(|&slot| ranks[slot] < 4).collect();
    prefix.sort_unstable();
    assert_eq!(prefix.len(), 4);
    // равномерно — значит крайний слот префикса лежит во второй половине
    assert!(
        *prefix.last().unwrap() >= 8,
        "префикс сгружен в начало ряда: {prefix:?}"
    );
    // ранги — перестановка, иначе пороги начнут совпадать или дублироваться
    let mut all = ranks.clone();
    all.sort_unstable();
    assert_eq!(all, (0..16).collect::<Vec<_>>());
}

/// Аллея на любой плотности стоит так же редко, как лес той же плотности.
/// Ради этого порог и квадратичный: с линейным ряд выходил вдвое гуще
/// соседнего парка, и на карте это било в глаза.
#[test]
fn row_is_as_dense_as_the_forest_beside_it() {
    let length = 300.0;
    let planted = plant(
        &map_with_row(straight_row(length, None)),
        TreeRowPlacement::Keep,
    );
    for density in [0.5, 1.0, 3.0, 6.0] {
        let seen = visible(&planted, density);
        let expected = (length / row_spacing_at(density)) as usize;
        assert!(
            seen.len().abs_diff(expected) <= 1,
            "плотность {density}: видно {}, ожидалось ~{expected}",
            seen.len()
        );
    }
}

/// Тот же пол минимального зазора, что у леса, и он **не** должен упираться
/// на рабочих плотностях: ряд, стоящий ровно по минимуму, и читается как
/// более густой, чем лес.
#[test]
fn visible_row_neighbours_keep_the_forest_gap() {
    let planted = plant(
        &map_with_row(straight_row(300.0, None)),
        TreeRowPlacement::Keep,
    );
    for density in [1.0, 3.0] {
        let mut xs: Vec<f32> = visible(&planted, density).iter().map(|p| p.x).collect();
        xs.sort_by(f32::total_cmp);
        let closest = xs
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(f32::INFINITY, f32::min);
        assert!(
            closest > TREE_MIN_SPACING,
            "плотность {density}: соседи через {closest} м, это пол зазора"
        );
    }
}

/// Прореживание монотонно: шаг ползунка вверх только добавляет деревья, уже
/// стоящие не переезжают.
#[test]
fn thinning_is_monotone() {
    let planted = plant(
        &map_with_row(straight_row(200.0, None)),
        TreeRowPlacement::Keep,
    );
    let sparse = visible(&planted, 0.5);
    let dense = visible(&planted, 2.0);
    assert!(sparse.len() < dense.len());
    for position in &sparse {
        assert!(
            dense.contains(position),
            "дерево {position:?} исчезло с ростом плотности"
        );
    }
}

/// Шаг из данных OSM — ползунок такой ряд не прореживает: пороги нулевые,
/// а расстояние между стволами равно тегу.
#[test]
fn data_spacing_ignores_the_density_slider() {
    let planted = plant(
        &map_with_row(straight_row(100.0, Some(20.0))),
        TreeRowPlacement::Keep,
    );
    assert!(planted.iter().all(|&(.., at)| at == 0.0));
    assert_eq!(planted.len(), 6);
    // на минимуме ползунка ряд стоит целиком
    assert_eq!(visible(&planted, TREE_DENSITY_MIN).len(), planted.len());

    let mut xs: Vec<f32> = planted.iter().map(|&(pos, ..)| pos.x).collect();
    xs.sort_by(f32::total_cmp);
    for pair in xs.windows(2) {
        assert!((pair[1] - pair[0] - 20.0).abs() < 1e-2);
    }
}

fn building_on_the_row() -> PolyArea {
    PolyArea {
        outer: vec![
            Vec2::new(1040.0, 990.0),
            Vec2::new(1060.0, 990.0),
            Vec2::new(1060.0, 1010.0),
            Vec2::new(1040.0, 1010.0),
        ],
        holes: Vec::new(),
        kind: AreaKind::Building,
        height: None,
        entrances: Vec::new(),
    }
}

/// `Keep` доверяет позиции из OSM, но не настолько, чтобы сажать дерево в
/// доме: крона из крыши читается как баг рендера.
#[test]
fn keep_still_drops_trees_inside_buildings() {
    let mut map = map_with_row(straight_row(100.0, None));
    map.buildings.push(building_on_the_row());
    let planted = plant(&map, TreeRowPlacement::Keep);

    assert!(!planted.is_empty());
    for &(pos, ..) in &planted {
        assert!(
            !point_in_area(pos, &map.buildings[0]),
            "дерево {pos:?} стоит внутри дома"
        );
    }
}

fn road_across_the_row() -> RoadLine {
    RoadLine {
        points: vec![Vec2::new(1050.0, 900.0), Vec2::new(1050.0, 1100.0)],
        width: 12.0,
        class: RoadClass::Street,
        bridge: false,
        passage: false,
    }
}

/// Разница двух политик: `Keep` оставляет дерево на асфальте, `Slide`
/// уводит его с дороги. Ради этого тумблер и заведён.
#[test]
fn slide_moves_trees_off_a_road_where_keep_leaves_them() {
    let mut map = map_with_row(straight_row(100.0, None));
    map.roads.push(road_across_the_row());
    let road = &map.roads[0];

    let on_road = |planted: &[PlantedTree]| {
        planted.iter().any(|&(pos, radius, _)| {
            distance_to_segment(pos, road.points[0], road.points[1])
                <= road.width / 2.0 + radius + TREE_KERB_CLEARANCE
        })
    };

    assert!(on_road(&plant(&map, TreeRowPlacement::Keep)));
    let slid = plant(&map, TreeRowPlacement::Slide);
    assert!(!on_road(&slid));
    // ряд не должен при этом развалиться: дорога съедает пару слотов, а не всё
    assert!(
        slid.len() >= 10,
        "после сдвига осталось всего {}",
        slid.len()
    );
}

/// Обе политики видят один и тот же лес и не видят друг друга — иначе
/// переключение тумблера меняло бы ряд целиком, а не пару деревьев.
#[test]
fn both_policies_start_from_the_same_wood() {
    let map = map_with_row(straight_row(100.0, None));
    let obstacles = Obstacles::build(&map);
    let mut occupied = Occupied::default();
    occupied.insert(Vec2::new(1000.0, 1000.0));

    for layout in TreeRowLayout::ALL {
        let planted = plant_rows(&map, &obstacles, occupied.clone(), layout);
        assert!(
            planted
                .iter()
                .all(|&(pos, ..)| pos.distance(Vec2::new(1000.0, 1000.0)) >= TREE_MIN_SPACING),
            "{layout:?} поставил дерево поверх лесного"
        );
    }
}

/// Настройка `osm_spacing = false` выключает шаг из тегов: ряд с
/// `spacing` начинает жить по ползунку плотности наравне с лесом.
#[test]
fn disabling_osm_spacing_hands_the_row_back_to_the_slider() {
    let map = map_with_row(straight_row(100.0, Some(20.0)));
    let from_data = plant_as(
        &map,
        TreeRowLayout {
            placement: TreeRowPlacement::Keep,
            osm_spacing: true,
        },
    );
    let from_slider = plant_as(
        &map,
        TreeRowLayout {
            placement: TreeRowPlacement::Keep,
            osm_spacing: false,
        },
    );

    assert!(from_data.iter().all(|&(.., at)| at == 0.0));
    assert!(from_slider.iter().all(|&(.., at)| at > 0.0));
    // теги игнорируются — ряд засажен по своему полу, а не через 20 м
    assert!(from_slider.len() > from_data.len());
    assert_eq!(
        visible(&from_slider, 1.0).len(),
        (100.0 / row_spacing_at(1.0)) as usize
    );
}

/// Ряд длиннее карты не должен превращаться в бесконечную посадку.
#[test]
fn a_dense_tag_cannot_blow_the_row_up() {
    let planted = plant(
        &map_with_row(straight_row(4000.0, Some(2.0))),
        TreeRowPlacement::Keep,
    );
    assert!(planted.len() <= TREE_ROW_MAX_TREES);
}

fn node_at(x: f32, y: f32) -> TreeNode {
    TreeNode {
        pos: Vec2::new(x, y),
        radius: None,
    }
}

fn wood_square() -> PolyArea {
    PolyArea {
        outer: vec![
            Vec2::new(1000.0, 1000.0),
            Vec2::new(1100.0, 1000.0),
            Vec2::new(1100.0, 1100.0),
            Vec2::new(1000.0, 1100.0),
        ],
        holes: Vec::new(),
        kind: AreaKind::Wood,
        height: None,
        entrances: Vec::new(),
    }
}

fn plant_nodes(map: &MapData) -> Vec<PlantedTree> {
    let obstacles = Obstacles::build(map);
    plant_standalone(map, &obstacles, &mut Occupied::default())
}

/// Нода в лесном полигоне пропускается: там дерево уже сажает
/// [`plant_woods`], и своё сверху дало бы двойную крону.
#[test]
fn standalone_skips_nodes_inside_woods() {
    let mut map = MapData {
        woods: vec![wood_square()],
        ..MapData::default()
    };
    map.tree_nodes.push(node_at(1050.0, 1050.0)); // в лесу
    map.tree_nodes.push(node_at(1200.0, 1050.0)); // в чистом поле
    let planted = plant_nodes(&map);
    assert_eq!(planted.len(), 1);
    assert_eq!(planted[0].0, Vec2::new(1200.0, 1050.0));
}

/// Нода на аллее пропускается: ряд по этому way уже посажен, а нода ближе
/// минимального зазора к оси — то же дерево, размеченное дважды.
#[test]
fn standalone_skips_nodes_hugging_a_tree_row() {
    let mut map = map_with_row(straight_row(100.0, None));
    map.tree_nodes.push(node_at(1050.0, 1004.0)); // 4 м от оси — это сама аллея
    map.tree_nodes.push(node_at(1050.0, 1020.0)); // 20 м — отдельное дерево
    let planted = plant_nodes(&map);
    assert_eq!(planted.len(), 1);
    assert_eq!(planted[0].0, Vec2::new(1050.0, 1020.0));
}

/// В доме и на полотне (или канте) дороги дерево не сажается даже по
/// данным, а у самой кромки — живёт: зазора кроны у одиночных нет
/// (та же логика, что у [`TreeRowPlacement::Keep`]).
#[test]
fn standalone_avoids_buildings_and_roadbed_but_keeps_kerbside() {
    let mut map = MapData::default();
    map.buildings.push(building_on_the_row());
    map.roads.push(road_across_the_row());
    map.tree_nodes.push(node_at(1050.0, 1000.0)); // внутри дома
    map.tree_nodes.push(node_at(1050.0, 950.0)); // на полотне (ось дороги)
    map.tree_nodes.push(node_at(1058.0, 950.0)); // за кантом, на тротуаре
    let planted = plant_nodes(&map);
    assert_eq!(planted.len(), 1);
    assert_eq!(planted[0].0, Vec2::new(1058.0, 950.0));
}

/// Две ноды ближе минимального зазора — одно дерево: в OSM встречаются
/// продублированные ноды, и обе кроны рисовать незачем.
#[test]
fn duplicate_standalone_nodes_collapse() {
    let mut map = MapData::default();
    map.tree_nodes.push(node_at(1000.0, 1000.0));
    map.tree_nodes.push(node_at(1003.0, 1000.0));
    assert_eq!(plant_nodes(&map).len(), 1);
}

/// Порог одиночных — 0 (дерево из данных видно на любой плотности), радиус
/// из `diameter_crown` доезжает как есть, без тега — разыгрывается в
/// лесной вилке.
#[test]
fn standalone_trees_are_always_visible_and_keep_tagged_radius() {
    let mut map = MapData::default();
    map.tree_nodes.push(TreeNode {
        pos: Vec2::new(1000.0, 1000.0),
        radius: Some(5.0),
    });
    map.tree_nodes.push(node_at(1000.0, 1050.0));
    let planted = plant_nodes(&map);
    assert_eq!(planted.len(), 2);
    assert!(planted.iter().all(|&(.., at)| at == 0.0));
    assert_eq!(visible(&planted, TREE_DENSITY_MIN).len(), 2);
    assert_eq!(planted[0].1, 5.0);
    assert!((TREE_MIN_RADIUS..=TREE_MAX_RADIUS).contains(&planted[1].1));
}

/// Лес держит минимальный зазор от уже занятых мест — так одиночные
/// деревья, посаженные первыми, не получают лесного соседа впритык.
#[test]
fn woods_keep_clear_of_preoccupied_spots() {
    let map = MapData {
        woods: vec![wood_square()],
        ..MapData::default()
    };
    let obstacles = Obstacles::build(&map);
    let mut occupied = Occupied::default();
    let spot = Vec2::new(1050.0, 1050.0);
    occupied.insert(spot);
    let (woods, _) = plant_woods(&map, &obstacles, &mut occupied);
    assert!(!woods.is_empty());
    assert!(
        woods
            .iter()
            .all(|&(pos, ..)| pos.distance(spot) >= TREE_MIN_SPACING),
        "лесное дерево встало поверх занятого места"
    );
}

/// Одиночные деревья идут в общем массиве первыми, и массив остаётся
/// отсортированным по порогу — на этом стоит `compose_trees`.
#[test]
fn standalone_trees_lead_the_planted_array() {
    let mut map = MapData {
        woods: vec![wood_square()],
        ..MapData::default()
    };
    map.tree_nodes.push(node_at(2000.0, 2000.0));
    let (standalone, woods, rows, _) = plant_trees(&map);
    assert_eq!(standalone.len(), 1);
    assert!(
        !woods.is_empty(),
        "лес не посадился рядом с одиночным деревом"
    );
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;

    map.compose_trees(TreeCompose::default());
    assert_eq!(map.trees[0].0, Vec2::new(2000.0, 2000.0));
    assert_eq!(map.tree_appears_at[0], 0.0);
    assert!(
        map.tree_appears_at
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
}

/// Тумблер источника выключает своё слагаемое и не трогает чужие.
#[test]
fn compose_toggles_exclude_sources() {
    let mut map = MapData {
        woods: vec![wood_square()],
        ..MapData::default()
    };
    map.tree_nodes.push(node_at(2000.0, 2000.0));
    let (standalone, woods, rows, _) = plant_trees(&map);
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;

    map.compose_trees(TreeCompose {
        standalone: false,
        ..TreeCompose::default()
    });
    assert_eq!(map.trees.len(), map.wood_trees.len());

    map.compose_trees(TreeCompose {
        woods: false,
        ..TreeCompose::default()
    });
    assert_eq!(map.trees.len(), map.standalone_trees.len());

    map.compose_trees(TreeCompose {
        woods: false,
        standalone: false,
        ..TreeCompose::default()
    });
    assert!(map.trees.is_empty());
}
