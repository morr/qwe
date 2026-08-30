use crate::map::osm::model::point_in_polygon;

use super::*;
use crate::grid::world_to_tile;
use crate::map::footprint::bridge_curb_width;
use crate::map::osm::fixture::{bridge, building, culvert, passage, rect, stream, street};
use crate::settings::{MAP_SIZE, PASSAGE_MAX_WIDTH};

/// Тайлы строки `y`, залитые построчной заливкой. Отрезки обрезаются по
/// ширине проверяемой полосы — как `set_area` обрезает их по сетке.
fn scanline_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
    let mut scratch = RowScratch::default();
    row_spans(outer, holes, y, navtile_size(), &mut scratch);
    scratch
        .spans
        .iter()
        .flat_map(|&(from, to)| from.max(0)..=to.min(width - 1))
        .collect::<Vec<_>>()
}

/// Те же тайлы точечной проверкой — эталон, который заменила заливка.
fn point_test_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
    (0..width)
        .filter(|&x| {
            let center = (Vec2::new(x as f32, y as f32) + 0.5) * navtile_size();
            point_in_polygon(center, outer)
                && !holes.iter().any(|hole| point_in_polygon(center, hole))
        })
        .collect()
}

fn assert_same_fill(outer: &[Vec2], holes: &[Vec<Vec2>], rows: i32, width: i32) {
    for y in 0..rows {
        assert_eq!(
            scanline_row(outer, holes, y, width),
            point_test_row(outer, holes, y, width),
            "row {y}"
        );
    }
}

#[test]
fn scanline_matches_point_test_for_a_rect_with_a_hole() {
    let outer = rect(Vec2::new(3.0, 5.0), Vec2::new(41.0, 33.0));
    let holes = vec![rect(Vec2::new(11.0, 13.0), Vec2::new(25.0, 27.0))];
    assert_same_fill(&outer, &holes, 20, 25);
}

/// Вогнутый контур: строка пересекает его дважды, и заливка обязана дать
/// два отрезка, а не один сплошной.
#[test]
fn scanline_matches_point_test_for_a_concave_ring() {
    let outer = vec![
        Vec2::new(2.0, 2.0),
        Vec2::new(30.0, 2.0),
        Vec2::new(30.0, 30.0),
        Vec2::new(24.0, 30.0),
        Vec2::new(24.0, 9.0),
        Vec2::new(8.0, 9.0),
        Vec2::new(8.0, 30.0),
        Vec2::new(2.0, 30.0),
    ];
    assert_same_fill(&outer, &[], 18, 18);
}

/// Дырка, наполовину вылезшая за внешнее кольцо. Если сваливать её рёбра
/// в общий even-odd список, торчащий кусок не вычтется, а зальётся —
/// именно поэтому дырки вычитаются отрезками.
#[test]
fn scanline_matches_point_test_for_a_hole_sticking_out() {
    let outer = rect(Vec2::new(6.0, 6.0), Vec2::new(30.0, 30.0));
    let holes = vec![rect(Vec2::new(20.0, 12.0), Vec2::new(44.0, 22.0))];
    assert_same_fill(&outer, &holes, 18, 25);
}

/// Косые рёбра — единственное место, где заливка могла бы разъехаться с
/// точечной проверкой на полтайла.
#[test]
fn scanline_matches_point_test_for_a_diagonal_ring() {
    let outer = vec![
        Vec2::new(1.7, 0.3),
        Vec2::new(37.4, 11.9),
        Vec2::new(21.1, 34.6),
        Vec2::new(5.2, 20.8),
    ];
    assert_same_fill(&outer, &[], 20, 22);
}

/// Арка режется последней: дом уже залит непроходимым, и проём должен
/// пробить его насквозь, не открыв при этом остального дома.
#[test]
fn a_building_passage_carves_a_corridor_through_the_building() {
    let mut map = MapData::default();
    map.buildings.push(building(
        rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
        vec![],
    ));
    map.roads.push(passage(
        vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
        5.0,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    for y in [101.0, 115.0, 129.0] {
        assert!(passable_at(Vec2::new(131.0, y)), "проём на y={y}");
    }
    assert!(!passable_at(Vec2::new(110.0, 115.0)), "стена западнее арки");
    assert!(
        !passable_at(Vec2::new(150.0, 115.0)),
        "стена восточнее арки"
    );
}

/// Бордюры моста непроходимы: сойти с настила вбок нельзя, но торцы моста
/// открыты — бордюр блокирует только две продольные кромки.
#[test]
fn bridge_curbs_block_the_deck_edges() {
    let mut map = MapData::default();
    map.roads.push(bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        8.0,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    // осевая бордюрной полосы — на полширины настила плюс полбордюра от оси
    let curb_line = (8.0 + bridge_curb_width(8.0)) / 2.0;
    assert!(passable_at(Vec2::new(130.0, 100.0)), "настил");
    for side in [-1.0, 1.0] {
        assert!(
            !passable_at(Vec2::new(130.0, 100.0 + side * curb_line)),
            "бордюр со стороны {side}"
        );
    }
    assert!(passable_at(Vec2::new(130.0, 110.0)), "земля за бордюром");
    for x in [95.0, 165.0] {
        assert!(passable_at(Vec2::new(x, 100.0)), "подход по осевой, x={x}");
    }
}

/// Дорога, подошедшая к мосту снаружи, входит на мост: её панель пробивает
/// проём в бордюре. Пробивает только бордюр — ручей, пересекающий ту же
/// дорогу, остаётся непроходимым, его переходят по мосту.
#[test]
fn a_joining_road_breaks_through_the_curb_but_not_through_water() {
    let mut map = MapData::default();
    map.roads.push(bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        8.0,
    ));
    // примыкающая с внешней стороны дорога, общий узел на осевой моста
    map.roads.push(street(
        vec![Vec2::new(130.0, 140.0), Vec2::new(130.0, 100.0)],
        5.0,
    ));
    map.water_lines.push(stream(
        vec![Vec2::new(110.0, 126.0), Vec2::new(150.0, 126.0)],
        2.0,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    let curb_line = 100.0 + (8.0 + bridge_curb_width(8.0)) / 2.0;
    assert!(
        passable_at(Vec2::new(130.0, curb_line)),
        "проём в бордюре на примыкании"
    );
    assert!(
        !passable_at(Vec2::new(115.0, curb_line)),
        "бордюр в стороне от примыкания"
    );
    assert!(
        !passable_at(Vec2::new(130.0, 126.0)),
        "ручей поперёк примыкающей дороги"
    );
}

/// Узкий косой мост: цепочка настила делит тайлы с цепочкой собственного
/// бордюра, настил отвоёвывает их себе, и без латки барьер продолжался бы со
/// сдвигом в соседнюю колонку — касанием углов, сквозь которое шагает
/// OrdinalGrid northstar. После заливки в окрестности моста не должно
/// остаться ни одной диагональной пары заблокированных тайлов с двумя
/// открытыми ортогональными соседями, а настил обязан остаться проходимым.
#[test]
fn a_narrow_slanted_bridge_leaves_no_corner_slips() {
    let (from, to) = (Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0));
    let mut map = MapData::default();
    map.roads.push(bridge(vec![from, to], 3.5));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let min_tile = world_to_tile(Vec2::new(90.0, 90.0));
    let max_tile = world_to_tile(Vec2::new(170.0, 140.0));
    for x in min_tile.x..max_tile.x {
        for y in min_tile.y..max_tile.y {
            let blocked = |dx: i32, dy: i32| !navmesh.is_passable(x + dx, y + dy);
            let slips = (blocked(0, 0) && blocked(1, 1) && !blocked(1, 0) && !blocked(0, 1))
                || (blocked(1, 0) && blocked(0, 1) && !blocked(0, 0) && !blocked(1, 1));
            assert!(!slips, "диагональная щель у тайла ({x}, {y})");
        }
    }
    for step in 0..=40 {
        let along = from.lerp(to, step as f32 / 40.0);
        let tile = world_to_tile(along);
        assert!(
            navmesh.is_passable(tile.x, tile.y),
            "осевая настила, шаг {step}"
        );
    }
}

/// Коллинеарный подход к мосту не слизывает бордюр: покрытие дороги — это её
/// тело, без торцевого выступа за общий узел. Иначе у короткого моста подходы
/// с двух концов открывали бы боковые бордюры почти целиком.
#[test]
fn a_collinear_approach_road_does_not_lick_the_curb_open() {
    let mut map = MapData::default();
    map.roads.push(bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        8.0,
    ));
    map.roads.push(street(
        vec![Vec2::new(60.0, 100.0), Vec2::new(100.0, 100.0)],
        8.0,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    let curb_line = (8.0 + bridge_curb_width(8.0)) / 2.0;
    for side in [-1.0, 1.0] {
        assert!(
            !passable_at(Vec2::new(101.0, 100.0 + side * curb_line)),
            "бордюр у торца, сторона {side}"
        );
    }
    assert!(
        passable_at(Vec2::new(95.0, 100.0)),
        "вход на мост по осевой"
    );
    assert!(passable_at(Vec2::new(105.0, 100.0)), "настил за торцом");
}

/// Мост и его тротуар — два параллельных bridge-ways одного физического
/// моста. Встречные бордюры в шве между ними накрыты лентами друг друга и
/// открыты, внешние кромки пары держат: пара ходит как один широкий мост.
#[test]
fn a_bridge_and_its_sidewalk_way_act_as_one_bridge() {
    let mut map = MapData::default();
    map.roads.push(bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        8.0,
    ));
    map.roads.push(bridge(
        vec![Vec2::new(100.0, 107.0), Vec2::new(160.0, 107.0)],
        3.5,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    assert!(
        passable_at(Vec2::new(130.0, 100.0)),
        "настил проезжей части"
    );
    assert!(passable_at(Vec2::new(130.0, 107.0)), "настил тротуара");
    assert!(
        passable_at(Vec2::new(130.0, 105.0)),
        "шов между проезжей частью и тротуаром"
    );
    let street_curb = (8.0 + bridge_curb_width(8.0)) / 2.0;
    let sidewalk_curb = (3.5 + bridge_curb_width(3.5)) / 2.0;
    assert!(
        !passable_at(Vec2::new(130.0, 100.0 - street_curb)),
        "внешний бордюр проезжей части"
    );
    assert!(
        !passable_at(Vec2::new(130.0, 107.0 + sidewalk_curb)),
        "внешний бордюр тротуара"
    );
}

/// Косой мост: тайлы цепочки бордюра гуляют внутрь настила до полудиагонали,
/// и прорезка настила полной шириной открывала их обратно — бордюр
/// превращался в пунктир, мост был проходим вбок посередине. Каждая точка
/// бордюрной осевой обязана лежать в непроходимом тайле, а осевая настила —
/// в проходимом.
#[test]
fn a_slanted_bridge_keeps_its_curbs_unbroken() {
    let (from, to) = (Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0));
    let mut map = MapData::default();
    map.roads.push(bridge(vec![from, to], 8.0));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    let normal = (to - from).normalize().perp();
    let curb_line = (8.0 + bridge_curb_width(8.0)) / 2.0;
    // торцы не проверяются: там бордюр кончается, а настил открыт
    for step in 4..=36 {
        let along = from.lerp(to, step as f32 / 40.0);
        assert!(passable_at(along), "осевая настила, шаг {step}");
        for side in [-1.0, 1.0] {
            assert!(
                !passable_at(along + normal * side * curb_line),
                "бордюр со стороны {side}, шаг {step}"
            );
        }
    }
}

/// Стык двух bridge-ways одного моста: настилы прорезаются после всех
/// бордюров, и бордюр первого way не перегораживает настил второго.
#[test]
fn a_bridge_junction_is_not_walled_by_the_other_ways_curb() {
    let mut map = MapData::default();
    for points in [
        vec![Vec2::new(100.0, 100.0), Vec2::new(130.0, 100.0)],
        vec![Vec2::new(130.0, 100.0), Vec2::new(130.0, 140.0)],
    ] {
        map.roads.push(bridge(points, 8.0));
    }

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    // бордюр первого way тянется по y = 100 + (w + curb) / 2 и пересекает
    // настил второго — тайл на пересечении обязан остаться проходимым
    let curb_line = 100.0 + (8.0 + bridge_curb_width(8.0)) / 2.0;
    assert!(passable_at(Vec2::new(130.0, curb_line)), "стык открыт");
    assert!(passable_at(Vec2::new(130.0, 120.0)), "настил второго way");
}

/// Узел, в котором сходятся три мостовых way, — не редкость, а обычная
/// пешеходная развязка: в кешах Лондона таких бордюрных тайлов 528 из 19 278
/// (до семи владельцев на тайл), в Нью-Йорке 1 064, в Токио 148. Владельцев
/// тайла надо держать всех: решение — `any` по ним, и выброшенный владелец
/// может только снять барьер. Здесь щупы первых двух way попадают в ленты
/// друг друга (внутренний шов, барьер не держат), а щуп третьего уходит в
/// пустоту — тайл на клину между ветками обязан быть непроходим. На двух
/// слотах владельцев третий терялся, и на этом месте в перилах оставалась
/// дыра.
#[test]
fn a_curb_tile_keeps_the_owner_that_holds_it_at_a_three_way_junction() {
    let node = Vec2::new(100.0, 101.0);
    let mut map = MapData::default();
    for end in [
        Vec2::new(110.0, 91.0),
        Vec2::new(110.0, 111.0),
        Vec2::new(110.0, 121.0),
    ] {
        map.roads.push(bridge(vec![node, end], 3.5));
    }

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };
    assert!(
        !passable_at(Vec2::new(103.0, 101.0)),
        "внешний бордюр третьего way на клину развязки"
    );
    assert!(passable_at(node), "узел развязки");
    for point in [
        Vec2::new(105.0, 96.0),
        Vec2::new(105.0, 106.0),
        Vec2::new(105.0, 111.0),
    ] {
        assert!(passable_at(point), "настил ветки у {point}");
    }
}

/// Ширина арки ограничена: `service` шириной 5 м не должен вырезать по
/// тайлу фасада с каждой стороны проёма.
#[test]
fn a_passage_is_no_wider_than_the_cap() {
    let mut map = MapData::default();
    map.buildings.push(building(
        rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
        vec![],
    ));
    map.roads.push(passage(
        vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
        12.0,
    ));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    let row = world_to_tile(Vec2::new(131.0, 115.0)).y;
    let open = (0..navmesh.grid_size.x)
        .filter(|&x| {
            let center = (x as f32 + 0.5) * navtile_size();
            (100.0..160.0).contains(&center) && navmesh.is_passable(x, row)
        })
        .count();
    assert!(
        open as f32 <= PASSAGE_MAX_WIDTH / navtile_size() + 1.0,
        "проём в {open} тайлов шире потолка"
    );
}

/// Портал культверта: русло уходит в трубу, и заливка обязана оборваться на
/// узле портала, а не продлить капсулу на полуширину русла за него. Полукруг
/// непроходимых тайлов за порталом глушил бы вход в трубу — единственное
/// место, где ручей вообще переходят посуху.
#[test]
fn a_culvert_portal_cuts_the_channel_flat() {
    let portal = Vec2::new(130.0, 100.0);
    let width = 8.0;
    let mut map = MapData::default();
    map.water_lines
        .push(stream(vec![Vec2::new(100.0, 100.0), portal], width));
    map.water_lines
        .push(culvert(vec![portal, Vec2::new(160.0, 100.0)], width));

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let passable_at = |point: Vec2| {
        let tile = world_to_tile(point);
        navmesh.is_passable(tile.x, tile.y)
    };

    // точка внутри полудиска, но за плоскостью торца, и не на осевой — её
    // тайлы отдельно метит `visit_segment_tiles`
    let past_the_portal = portal + Vec2::new(1.0, 3.0);
    assert!(!passable_at(Vec2::new(120.0, 100.0)), "русло до портала");
    assert!(passable_at(past_the_portal), "земля за порталом культверта");

    // то же русло, но обрывающееся ничем: торец остаётся круглым, и тот же
    // тайл — вода
    map.water_lines.pop();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let tile = world_to_tile(past_the_portal);
    assert!(
        !navmesh.is_passable(tile.x, tile.y),
        "торец русла без трубы обязан остаться круглым"
    );
}

#[test]
fn passable_from_reports_a_fully_blocked_grid() {
    let mut map = MapData::default();
    map.buildings
        .push(building(rect(Vec2::ZERO, MAP_SIZE), vec![]));
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    assert!(navmesh.passable_from(IVec2::ZERO).is_none());
}

#[test]
fn passable_from_wraps_past_the_start() {
    let mut map = MapData::default();
    let hole_center = MAP_SIZE * 0.25;
    let hole_tile_size = navtile_size();
    let hole_rect = rect(hole_center, hole_center + hole_tile_size);
    map.buildings
        .push(building(rect(Vec2::ZERO, MAP_SIZE), vec![hole_rect]));
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let hole_tile = world_to_tile(hole_center);
    let from_after_hole = IVec2::new(hole_tile.x + 1, hole_tile.y);
    assert_eq!(
        navmesh.passable_from(from_after_hole),
        Some(hole_tile),
        "скан после дырки обязан вернуть саму дырку"
    );
    assert_eq!(
        navmesh.passable_from(IVec2::ZERO),
        Some(hole_tile),
        "скан от нуля вернёт тот же тайл"
    );
}

/// Сетка с чужим размером тайла: снапшот вдвое мельче текущего атомика.
/// Заливка обязана считать по нему — иначе дом ляжет не туда, где он стоит.
fn half_scale_navmesh(side: i32) -> Navmesh {
    let grid_size = IVec2::splat(side);
    Navmesh {
        passable: vec![true; (grid_size.x * grid_size.y) as usize],
        grid_size,
        tile_size: navtile_size() / 2.0,
    }
}

/// Площадная заливка (вода и дома — основной объём) идёт по `tile_size`
/// **своего** снапшота, а не по процессному атомику: иначе `Navmesh`, залитый
/// при одном размере навтайла, растеризует дом в масштабе другого.
#[test]
fn set_area_rasterises_by_the_navmesh_own_tile_size() {
    let mut navmesh = half_scale_navmesh(64);
    let tile_size = navmesh.tile_size;
    let area = building(rect(Vec2::new(10.0, 10.0), Vec2::new(30.0, 20.0)), vec![]);

    navmesh.set_area(&area, false);

    for x in 0..navmesh.grid_size.x {
        for y in 0..navmesh.grid_size.y {
            let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
            assert_eq!(
                navmesh.is_passable(x, y),
                !point_in_polygon(center, &area.outer),
                "тайл ({x}, {y}), центр {center:?}"
            );
        }
    }
}

/// То же для ленты: границы перебора и — главное — стартовый тайл цепочки по
/// осевой (`visit_segment_tiles`) берутся из снапшота. По атомику стена уехала
/// бы в другую строку сетки целиком.
#[test]
fn set_polyline_rasterises_by_the_navmesh_own_tile_size() {
    let mut navmesh = half_scale_navmesh(64);
    let tile_size = navmesh.tile_size;
    let (from, to) = (Vec2::new(10.0, 10.0), Vec2::new(30.0, 10.0));

    navmesh.set_polyline(&[from, to], 2.0, false);

    let tile_at = |point: Vec2| (point / tile_size).floor().as_ivec2();
    for point in [Vec2::new(20.0, 9.5), Vec2::new(20.0, 10.5)] {
        let tile = tile_at(point);
        assert!(
            !navmesh.is_passable(tile.x, tile.y),
            "лента стены в точке {point:?}"
        );
    }
    for point in [Vec2::new(20.0, 5.5), Vec2::new(20.0, 14.5)] {
        let tile = tile_at(point);
        assert!(
            navmesh.is_passable(tile.x, tile.y),
            "земля в стороне от стены, {point:?}"
        );
    }
}

/// Индекс лент мостов обязан отвечать **ровно** то же, что линейный перебор
/// всех bridge-ways, который он заменил: щуп «что снаружи» решает, какой
/// бордюр останется барьером, и любое расхождение — это либо дыра в перилах,
/// либо мост, перегороженный собственным бордюром. Сцена держит все случаи,
/// на которых индекс мог бы соврать: вплотную идущий тротуар, излом осевой,
/// поперечный мост, мост в другой ячейке хеша и мост у самого начала координат
/// (щуп уходит в отрицательные координаты — там `floor` и `as_ivec2`
/// расходятся).
#[test]
fn the_bridge_band_index_answers_exactly_like_the_linear_scan() {
    let v = Vec2::new;
    let roads = [
        bridge(vec![v(100.0, 100.0), v(160.0, 100.0)], 8.0),
        bridge(vec![v(100.0, 107.0), v(160.0, 107.0)], 3.5),
        bridge(vec![v(160.0, 100.0), v(200.0, 130.0), v(240.0, 130.0)], 8.0),
        bridge(vec![v(130.0, 60.0), v(130.0, 140.0)], 5.0),
        bridge(vec![v(0.0, 3.0), v(40.0, 3.0)], 8.0),
        bridge(vec![v(900.0, 900.0), v(940.0, 980.0)], 16.0),
    ];
    let ways: Vec<(&[Vec2], f32)> = roads
        .iter()
        .map(|road| (road.points.as_slice(), road.curb_reach()))
        .collect();
    let bands = BridgeBands::build(&ways);
    // предикат до фикса, слово в слово
    let linear = |probe: Vec2, owner: usize| {
        ways.iter()
            .enumerate()
            .any(|(other, &(way, band))| other != owner && distance_to_polyline(probe, way) <= band)
    };
    let mut probes: Vec<Vec2> = Vec::new();
    for step_x in -15..130 {
        for step_y in -15..80 {
            probes.push(v(step_x as f32 * 2.0, step_y as f32 * 2.0));
        }
    }
    for step_x in 435..485 {
        for step_y in 435..495 {
            probes.push(v(step_x as f32 * 2.0, step_y as f32 * 2.0));
        }
    }
    for owner in 0..ways.len() {
        for &probe in &probes {
            assert_eq!(
                bands.covered_by_other(probe, owner),
                linear(probe, owner),
                "щуп {probe:?} от владельца {owner}"
            );
        }
    }
}
