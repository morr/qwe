use crate::map::osm::model::{
    AreaKind, RoadClass, RoadLine, WaterKind, WaterLine, point_in_polygon,
};

use super::*;

/// Тайлы строки `y`, залитые построчной заливкой. Отрезки обрезаются по
/// ширине проверяемой полосы — как `set_area` обрезает их по сетке.
fn scanline_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
    let mut scratch = RowScratch::default();
    row_spans(outer, holes, y, &mut scratch);
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

fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
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
    map.buildings.push(PolyArea {
        outer: rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
        holes: Vec::new(),
        kind: AreaKind::Building,
        height: None,
        entrances: Vec::new(),
    });
    map.roads.push(RoadLine {
        points: vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
        width: 5.0,
        class: RoadClass::Street,
        bridge: false,
        passage: true,
    });

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
    map.roads.push(RoadLine {
        points: vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        width: 8.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    });

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
    map.roads.push(RoadLine {
        points: vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        width: 8.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    });
    // примыкающая с внешней стороны дорога, общий узел на осевой моста
    map.roads.push(RoadLine {
        points: vec![Vec2::new(130.0, 140.0), Vec2::new(130.0, 100.0)],
        width: 5.0,
        class: RoadClass::Street,
        bridge: false,
        passage: false,
    });
    map.water_lines.push(WaterLine {
        points: vec![Vec2::new(110.0, 126.0), Vec2::new(150.0, 126.0)],
        width: 2.0,
        kind: WaterKind::Stream,
        tunnel: false,
    });

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
    map.roads.push(RoadLine {
        points: vec![from, to],
        width: 3.5,
        class: RoadClass::Alley,
        bridge: true,
        passage: false,
    });

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
    map.roads.push(RoadLine {
        points: vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        width: 8.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    });
    map.roads.push(RoadLine {
        points: vec![Vec2::new(60.0, 100.0), Vec2::new(100.0, 100.0)],
        width: 8.0,
        class: RoadClass::Street,
        bridge: false,
        passage: false,
    });

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
    map.roads.push(RoadLine {
        points: vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
        width: 8.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    });
    map.roads.push(RoadLine {
        points: vec![Vec2::new(100.0, 107.0), Vec2::new(160.0, 107.0)],
        width: 3.5,
        class: RoadClass::Alley,
        bridge: true,
        passage: false,
    });

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
    map.roads.push(RoadLine {
        points: vec![from, to],
        width: 8.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    });

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
        map.roads.push(RoadLine {
            points,
            width: 8.0,
            class: RoadClass::Street,
            bridge: true,
            passage: false,
        });
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

/// Ширина арки ограничена: `service` шириной 5 м не должен вырезать по
/// тайлу фасада с каждой стороны проёма.
#[test]
fn a_passage_is_no_wider_than_the_cap() {
    let mut map = MapData::default();
    map.buildings.push(PolyArea {
        outer: rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
        holes: Vec::new(),
        kind: AreaKind::Building,
        height: None,
        entrances: Vec::new(),
    });
    map.roads.push(RoadLine {
        points: vec![Vec2::new(131.0, 90.0), Vec2::new(131.0, 140.0)],
        width: 12.0,
        class: RoadClass::Street,
        bridge: false,
        passage: true,
    });

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
    map.water_lines.push(WaterLine {
        points: vec![Vec2::new(100.0, 100.0), portal],
        width,
        kind: WaterKind::River,
        tunnel: false,
    });
    map.water_lines.push(WaterLine {
        points: vec![portal, Vec2::new(160.0, 100.0)],
        width,
        kind: WaterKind::River,
        tunnel: true,
    });

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
