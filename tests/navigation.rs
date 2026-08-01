//! Юнит-тесты навигации: A* по синтетическому navmesh и заполнение
//! navmesh из рукотворной `MapData` (без сети).

use bevy::math::{IVec2, Vec2};

use qwe::grid::{tile_center, world_to_tile};
use qwe::map::osm::{
    AreaKind, MapData, PolyArea, RailKind, RailLine, RoadClass, RoadLine, WallLine, WaterKind,
    WaterLine,
};
use qwe::navigation::{Navmesh, PathfindingAlgorithm, find_path, line_of_sight};

fn astar_pathfinding(navmesh: &Navmesh, start: IVec2, end: IVec2) -> Option<Vec<IVec2>> {
    find_path(navmesh, start, end, PathfindingAlgorithm::Astar)
}
use qwe::settings::GRID_SIZE;

/// Navmesh с одним прямоугольным препятствием (в тайлах, включительно).
fn navmesh_with_block(min: IVec2, max: IVec2) -> Navmesh {
    let mut navmesh = Navmesh::default();
    for x in min.x..=max.x {
        for y in min.y..=max.y {
            navmesh.set_passable(x, y, false);
        }
    }
    navmesh
}

fn rect_ring(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

#[test]
fn out_of_bounds_is_impassable() {
    let navmesh = Navmesh::default();
    assert!(!navmesh.is_passable(-1, 0));
    assert!(!navmesh.is_passable(0, -1));
    assert!(!navmesh.is_passable(GRID_SIZE.x, 0));
    assert!(!navmesh.is_passable(0, GRID_SIZE.y));
}

#[test]
fn line_of_sight_is_blocked_by_a_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let west = tile_center(IVec2::new(95, 105));
    let east = tile_center(IVec2::new(115, 105));
    let north = tile_center(IVec2::new(105, 115));

    // сквозь здание — нет; вдоль его южной стороны и по вертикали — да
    assert!(!line_of_sight(&navmesh, west, east));
    assert!(!line_of_sight(&navmesh, west, north));
    assert!(line_of_sight(
        &navmesh,
        tile_center(IVec2::new(95, 99)),
        tile_center(IVec2::new(115, 99))
    ));
    assert!(line_of_sight(&navmesh, west, west));
}

/// Косое узкое русло обязано лечь в сетку **четырёхсвязной** преградой, а не
/// цепочкой тайлов, соприкасающихся углами.
///
/// Растеризация метит тайлы по «центр ближе полуширины», и лента у́же
/// `NAVTILE_SIZE · √2` (2.83 м) на косой линии вырождается в шахматку. Своим
/// A* такую преграду не перейти — он не срезает углы, — но перейдут все
/// остальные потребители сетки: `OrdinalGrid` из `bevy_northstar` (HPA*,
/// Theta*) собирается без фильтра срезания углов и делает диагональный шаг
/// прямо между двумя заблокированными тайлами, а `line_of_sight` сэмплирует
/// точки и проходит через место касания. На реальном ручье Тулы (2.5 м) так и
/// вышло: навмеш-оверлей рисовал вдоль русла шахматку, и на HPA* люди ходили
/// через воду.
///
/// Проверяется тем же обходом, каким ходит HPA*: восемь направлений и углы
/// срезать можно. Дошёл до другого берега — русло дырявое.
#[test]
fn a_narrow_diagonal_waterway_leaves_no_corner_squeeze() {
    // 18° — один из худших углов: при ширине 2.5 м и тайле 2 м лента как раз
    // вырождается в цепочку по углам
    let (origin, direction) = (Vec2::new(200.0, 200.0), Vec2::new(3000.0, 970.0));
    let map = MapData {
        water_lines: vec![WaterLine {
            points: vec![origin, origin + direction],
            width: 2.5,
            kind: WaterKind::Stream,
            tunnel: false,
        }],
        ..MapData::default()
    };
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    // Обход берега по правилам northstar: восемь направлений и **углы срезать
    // можно** — именно так ходит HPA*. Окно берётся так, что русло пересекает
    // его насквозь, и за края обход не выпускается: иначе он обошёл бы русло
    // с торца, где никакой преграды и нет.
    // окно у́же русла по обеим осям: русло входит слева и выходит справа
    let lo = world_to_tile(Vec2::new(400.0, 150.0));
    let hi = world_to_tile(Vec2::new(1400.0, 700.0));
    // сторона относительно осевой: знак векторного произведения
    let side = |tile: IVec2| {
        let point = tile_center(tile) - origin;
        point.x * direction.y - point.y * direction.x
    };
    let start = world_to_tile(Vec2::new(900.0, 200.0));
    assert!(side(start) > 0.0 && navmesh.is_passable(start.x, start.y));

    let mut seen = std::collections::HashSet::from([start]);
    let mut queue = std::collections::VecDeque::from([start]);
    let mut crossed = None;
    while let Some(tile) = queue.pop_front() {
        for dx in -1..=1 {
            for dy in -1..=1 {
                let next = tile + IVec2::new(dx, dy);
                if next.x < lo.x || next.x > hi.x || next.y < lo.y || next.y > hi.y {
                    continue;
                }
                if !navmesh.is_passable(next.x, next.y) || !seen.insert(next) {
                    continue;
                }
                if side(next) < 0.0 {
                    crossed = crossed.or(Some((tile, next)));
                }
                queue.push_back(next);
            }
        }
    }
    assert!(
        crossed.is_none(),
        "HPA* перешёл русло: {:?}",
        crossed.unwrap()
    );

    // и оно по-прежнему непрозрачно для луча, но не залило пол-округи
    let mid = origin + direction * 0.3;
    let normal = Vec2::new(-direction.y, direction.x).normalize();
    assert!(!line_of_sight(
        &navmesh,
        mid - normal * 8.0,
        mid + normal * 8.0
    ));
    assert!(line_of_sight(
        &navmesh,
        mid - normal * 8.0,
        mid - normal * 8.0 + direction * 0.1
    ));
}

#[test]
fn astar_finds_path_around_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let start = IVec2::new(105, 95);
    let end = IVec2::new(105, 115);

    let path = astar_pathfinding(&navmesh, start, end).expect("path should exist");
    assert!(
        path.len() > 20,
        "path must detour, got {} tiles",
        path.len()
    );
    assert_eq!(*path.first().unwrap(), start);
    assert_eq!(*path.last().unwrap(), end);
    for tile in &path {
        assert!(
            navmesh.is_passable(tile.x, tile.y),
            "path goes through impassable tile {tile:?}"
        );
    }
}

#[test]
fn astar_to_impassable_target_returns_none() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    assert!(astar_pathfinding(&navmesh, IVec2::new(90, 90), IVec2::new(105, 105)).is_none());
}

#[test]
fn astar_does_not_cut_corners() {
    let mut navmesh = Navmesh::default();
    // одиночный блок: диагональ вокруг угла запрещена
    navmesh.set_passable(10, 10, false);
    let path = astar_pathfinding(&navmesh, IVec2::new(9, 10), IVec2::new(11, 10))
        .expect("path should exist");
    for pair in path.windows(2) {
        let step = (pair[1] - pair[0]).abs();
        if step == IVec2::ONE {
            assert!(
                navmesh.is_passable(pair[0].x, pair[1].y)
                    && navmesh.is_passable(pair[1].x, pair[0].y),
                "diagonal step cuts corner at {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}

/// Каждый алгоритм обходит препятствие валидным путём с теми же концами.
#[test]
fn every_algorithm_finds_valid_path_around_building() {
    let navmesh = navmesh_with_block(IVec2::new(100, 100), IVec2::new(110, 110));
    let start = IVec2::new(105, 95);
    let end = IVec2::new(105, 115);

    for algorithm in [
        PathfindingAlgorithm::Astar,
        PathfindingAlgorithm::Dijkstra,
        PathfindingAlgorithm::Fringe,
        PathfindingAlgorithm::Bfs,
    ] {
        let path = find_path(&navmesh, start, end, algorithm)
            .unwrap_or_else(|| panic!("{algorithm:?} should find a path"));
        assert_eq!(*path.first().unwrap(), start, "{algorithm:?}");
        assert_eq!(*path.last().unwrap(), end, "{algorithm:?}");
        for pair in path.windows(2) {
            let step = (pair[1] - pair[0]).abs();
            assert!(
                step.max_element() <= 1,
                "{algorithm:?} makes a non-adjacent step {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
        for tile in &path {
            assert!(
                navmesh.is_passable(tile.x, tile.y),
                "{algorithm:?} path goes through impassable tile {tile:?}"
            );
        }
    }
}

#[test]
fn grid_roundtrip() {
    let tile = IVec2::new(123, 45);
    assert_eq!(world_to_tile(tile_center(tile)), tile);
}

/// Рукотворная карта: здание с двором-дыркой, вода с мостом, стена.
#[test]
fn fill_from_mapdata_blocks_and_carves() {
    let map = MapData {
        buildings: vec![PolyArea {
            outer: rect_ring(Vec2::new(100.0, 100.0), Vec2::new(160.0, 160.0)),
            holes: vec![rect_ring(Vec2::new(120.0, 120.0), Vec2::new(140.0, 140.0))],
            kind: AreaKind::Building,
            height: Some(15.0),
            entrances: Vec::new(),
        }],
        water: vec![PolyArea {
            outer: rect_ring(Vec2::new(300.0, 0.0), Vec2::new(340.0, 400.0)),
            holes: Vec::new(),
            kind: AreaKind::Water,
            height: None,
            entrances: Vec::new(),
        }],
        parks: Vec::new(),
        woods: Vec::new(),
        grass: Vec::new(),
        sand: Vec::new(),
        roads: vec![
            RoadLine {
                points: vec![Vec2::new(280.0, 200.0), Vec2::new(360.0, 200.0)],
                width: 8.0,
                class: RoadClass::Street,
                bridge: true,
                passage: false,
            },
            // мост через линейное русло — прорезка обязана работать и по нему,
            // иначе ручей рассекал бы город без единого перехода
            RoadLine {
                points: vec![Vec2::new(680.0, 200.0), Vec2::new(720.0, 200.0)],
                width: 8.0,
                class: RoadClass::Street,
                bridge: true,
                passage: false,
            },
        ],
        rails: vec![RailLine {
            points: vec![Vec2::new(600.0, 100.0), Vec2::new(600.0, 200.0)],
            width: 5.0,
            kind: RailKind::Active,
        }],
        walls: vec![WallLine {
            points: vec![Vec2::new(500.0, 100.0), Vec2::new(500.0, 200.0)],
            width: 3.0,
        }],
        water_lines: vec![
            // открытое русло — вода, как пруд: вброд не переходят
            WaterLine {
                points: vec![Vec2::new(700.0, 100.0), Vec2::new(700.0, 300.0)],
                width: 6.0,
                kind: WaterKind::Stream,
                tunnel: false,
            },
            // тот же ручей в трубе — под землёй, значит поверху ходят
            WaterLine {
                points: vec![Vec2::new(800.0, 100.0), Vec2::new(800.0, 300.0)],
                width: 6.0,
                kind: WaterKind::Stream,
                tunnel: true,
            },
        ],
        // деревья навмеша не касаются — они и лесные, и аллейные чисто
        // визуальные, так что перечислять их поля тут нечего
        ..MapData::default()
    };

    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);

    // здание непроходимо, двор-дырка — проходим
    let building_tile = world_to_tile(Vec2::new(110.0, 110.0));
    assert!(!navmesh.is_passable(building_tile.x, building_tile.y));
    let courtyard_tile = world_to_tile(Vec2::new(130.0, 130.0));
    assert!(navmesh.is_passable(courtyard_tile.x, courtyard_tile.y));

    // вода непроходима, мост через неё — проходим
    let water_tile = world_to_tile(Vec2::new(320.0, 100.0));
    assert!(!navmesh.is_passable(water_tile.x, water_tile.y));
    let bridge_tile = world_to_tile(Vec2::new(320.0, 200.0));
    assert!(navmesh.is_passable(bridge_tile.x, bridge_tile.y));

    // стена непроходима
    let wall_tile = world_to_tile(Vec2::new(500.0, 150.0));
    assert!(!navmesh.is_passable(wall_tile.x, wall_tile.y));

    // а рельсы — слой чисто визуальный: через путь ходят как по земле
    let rail_tile = world_to_tile(Vec2::new(600.0, 150.0));
    assert!(navmesh.is_passable(rail_tile.x, rail_tile.y));

    // линейное русло, в отличие от рельсов, блокирует — и мост его прорезает
    let stream_tile = world_to_tile(Vec2::new(700.0, 150.0));
    assert!(!navmesh.is_passable(stream_tile.x, stream_tile.y));
    let stream_bridge_tile = world_to_tile(Vec2::new(700.0, 200.0));
    assert!(navmesh.is_passable(stream_bridge_tile.x, stream_bridge_tile.y));

    // а труба не блокирует: вода под землёй, человек идёт поверху
    let culvert_tile = world_to_tile(Vec2::new(800.0, 150.0));
    assert!(navmesh.is_passable(culvert_tile.x, culvert_tile.y));

    // и путь через реку существует и идёт по мосту
    let path = astar_pathfinding(
        &navmesh,
        world_to_tile(Vec2::new(280.0, 200.0)),
        world_to_tile(Vec2::new(360.0, 200.0)),
    )
    .expect("path across the bridge should exist");
    for tile in &path {
        assert!(navmesh.is_passable(tile.x, tile.y));
    }
}
