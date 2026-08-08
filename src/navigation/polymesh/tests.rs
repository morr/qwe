use super::*;
use crate::map::osm::model::{RoadClass, WaterKind};
use crate::settings::MAP_SIZE;

/// Сторона чанка для тестов иерархии: по умолчанию она больше карты
/// (`CHUNK_TARGET_METERS`), то есть швов нет вовсе и проверять было бы
/// нечего.
const CHUNK_METERS: f32 = 400.0;

/// Водоток через всю ширину карты — без моста он обязан рассечь её
/// надвое даже у самой кромки (клип расширен наружу, втянутый оставлял
/// бы обходную щель вдоль границы).
fn input_with(roads: Vec<RoadLine>) -> PolymeshInput {
    PolymeshInput {
        buildings: vec![],
        water: vec![],
        water_lines: vec![WaterLine {
            points: vec![Vec2::new(0.0, 500.0), Vec2::new(MAP_SIZE.x, 500.0)],
            width: 4.0,
            kind: WaterKind::Stream,
            tunnel: false,
        }],
        walls: vec![],
        roads,
    }
}

/// Пиннит порядок boolean: бордюры и русло блокируют, настил вычитается
/// после них — иначе моста либо нет, либо он перегорожен своим бордюром.
#[test]
fn bridge_deck_opens_a_crossing_over_a_waterway() {
    let from = Vec2::new(300.0, 450.0);
    let to = Vec2::new(300.0, 550.0);

    let severed = build_polymesh(&input_with(vec![]), 0.0, None, None).expect("not cancelled");
    assert!(
        find_path_polymesh(&severed, from, to).is_none(),
        "waterway without a bridge must sever the banks"
    );
    assert!(
        !severed.obstacles.is_empty(),
        "the waterway must survive as a filled obstacle for the overlay"
    );

    let bridge = RoadLine {
        points: vec![Vec2::new(300.0, 460.0), Vec2::new(300.0, 540.0)],
        width: 5.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    };
    let bridged =
        build_polymesh(&input_with(vec![bridge]), 0.0, None, None).expect("not cancelled");
    let path = find_path_polymesh(&bridged, from, to)
        .expect("bridge deck must carry a path over the waterway");

    // контракт для `listen_for_pathfinding_tasks`: стартовая точка входит
    // в путь (её там срежут), последняя — цель
    assert_eq!(path.first(), Some(&from), "path must start at `from`");
    assert!(
        path.last().expect("path is non-empty").distance(to) < 0.5,
        "path must end at `to`, got {:?}",
        path.last()
    );
}

/// Маршрут длиной в несколько чанков через единственный проход в стене.
/// Пиннит сшивку швов и коридор разом: без сшивки соседние чанки не
/// связаны и пути нет вовсе, а без прохода в стене граф компонент обязан
/// отказать, не запуская polyanya.
#[test]
fn a_route_crosses_chunks_through_the_only_gap_in_a_wall() {
    let wall = |gap: bool| {
        let x = 2000.0;
        let mut walls = vec![WallLine {
            points: vec![Vec2::new(x, 0.0), Vec2::new(x, 1500.0)],
            width: 6.0,
        }];
        // верхний кусок стены начинается выше прохода — или вплотную,
        // если прохода нет
        walls.push(WallLine {
            points: vec![
                Vec2::new(x, if gap { 1560.0 } else { 1500.0 }),
                Vec2::new(x, MAP_SIZE.y),
            ],
            width: 6.0,
        });
        PolymeshInput {
            buildings: vec![],
            water: vec![],
            water_lines: vec![],
            walls,
            roads: vec![],
        }
    };

    let from = Vec2::new(500.0, 1000.0);
    let to = Vec2::new(3500.0, 1000.0);

    let open = build_polymesh(&wall(true), 0.2, None, Some(CHUNK_METERS)).expect("not cancelled");
    let path = find_path_polymesh(&open, from, to).expect("the gap must carry a path");
    assert_eq!(path.first(), Some(&from));
    assert!(path.last().expect("non-empty").distance(to) < 1.0);
    // путь обязан свернуть к проходу, а не идти напрямик сквозь стену
    assert!(
        path.iter().any(|point| point.y > 1400.0),
        "path must detour to the gap at y=1530: {path:?}"
    );

    let closed =
        build_polymesh(&wall(false), 0.2, None, Some(CHUNK_METERS)).expect("not cancelled");
    assert!(
        find_path_polymesh(&closed, from, to).is_none(),
        "a wall without a gap must sever the map"
    );
}

/// `contains` строже сеточной проходимости: контур раздут на радиус
/// агента, и точка вплотную к стене на меше уже внутри препятствия. На
/// этом держится спасение застрявших после постройки меша с новым
/// радиусом — сетка о нём ничего не знает.
#[test]
fn a_point_within_the_agent_radius_of_a_wall_is_off_the_mesh() {
    let building = PolyArea {
        outer: vec![
            Vec2::new(1000.0, 1000.0),
            Vec2::new(1040.0, 1000.0),
            Vec2::new(1040.0, 1030.0),
            Vec2::new(1000.0, 1030.0),
        ],
        holes: vec![],
        kind: crate::map::osm::model::AreaKind::Building,
        height: None,
        entrances: vec![],
    };
    let input = PolymeshInput {
        buildings: vec![building],
        water: vec![],
        water_lines: vec![],
        walls: vec![],
        roads: vec![],
    };
    let mesh = build_polymesh(&input, 1.0, None, None).expect("not cancelled");

    assert!(!mesh.contains(Vec2::new(1020.0, 1015.0)), "внутри дома");
    assert!(
        !mesh.contains(Vec2::new(999.5, 1015.0)),
        "в полуметре от стены при радиусе агента 1 м"
    );
    assert!(
        mesh.contains(Vec2::new(995.0, 1015.0)),
        "в пяти метрах от стены"
    );
}
