use super::build::{inflate_ring, ribbon_outline};
use super::*;
use crate::map::osm::model::{RoadClass, WaterKind};
use crate::settings::{
    MAP_SIZE, POLYMESH_CHUNK_TARGET_METERS, POLYMESH_FLAT_CHUNK_METERS, POLYMESH_SEARCH_DELTA,
    POLYMESH_SEARCH_STEPS,
};

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

/// Остров в реке — это дыра водного мультиполигона (Сите и Сен-Луи в Париже
/// суть inner-кольца «La Seine», и точка старта города стоит на первом из
/// них). Дыра обязана вычитаться из препятствия, как её вычитает сеточный
/// `row_spans`; открывает же остров прорез настила моста, разрезающий водное
/// кольцо. Без моста остров остаётся дырой результата и отбрасывается —
/// недостижимый карман, ровно то, что убивает сеточный `prune_unreachable`.
#[test]
fn an_island_is_walkable_once_a_bridge_reaches_it() {
    // квадратное «русло» с островом-дырой и сараем во дворе острова
    let river = PolyArea {
        outer: vec![
            Vec2::new(1000.0, 1000.0),
            Vec2::new(1600.0, 1000.0),
            Vec2::new(1600.0, 1600.0),
            Vec2::new(1000.0, 1600.0),
        ],
        holes: vec![vec![
            Vec2::new(1200.0, 1200.0),
            Vec2::new(1400.0, 1200.0),
            Vec2::new(1400.0, 1400.0),
            Vec2::new(1200.0, 1400.0),
        ]],
        kind: crate::map::osm::model::AreaKind::Water,
        height: None,
        entrances: vec![],
    };
    let shed = PolyArea {
        outer: vec![
            Vec2::new(1240.0, 1240.0),
            Vec2::new(1270.0, 1240.0),
            Vec2::new(1270.0, 1270.0),
            Vec2::new(1240.0, 1270.0),
        ],
        holes: vec![],
        kind: crate::map::osm::model::AreaKind::Building,
        height: None,
        entrances: vec![],
    };
    let input = |roads: Vec<RoadLine>| PolymeshInput {
        buildings: vec![shed.clone()],
        water: vec![river.clone()],
        water_lines: vec![],
        walls: vec![],
        roads,
    };

    let bank = Vec2::new(1300.0, 900.0);
    let island = Vec2::new(1330.0, 1330.0);

    let severed = build_polymesh(&input(vec![]), 0.4, None, None).expect("not cancelled");
    assert!(
        !severed.contains(island),
        "остров без моста недостижим — дыра результата отбрасывается"
    );

    let bridge = RoadLine {
        points: vec![Vec2::new(1300.0, 950.0), Vec2::new(1300.0, 1300.0)],
        width: 20.0,
        class: RoadClass::Street,
        bridge: true,
        passage: false,
    };
    let bridged = build_polymesh(&input(vec![bridge]), 0.4, None, None).expect("not cancelled");
    assert!(
        bridged.contains(island),
        "остров под мостом обязан быть на меше"
    );
    assert!(
        !bridged.contains(Vec2::new(1255.0, 1255.0)),
        "сарай внутри дыры остаётся сплошным: NonZero поднимает обмотку обратно"
    );
    let path = find_path_polymesh(&bridged, bank, island).expect("мост обязан довести до острова");
    assert_eq!(path.first(), Some(&bank));
    assert!(path.last().expect("непустой путь").distance(island) < 1.0);
}
/// Раскладка упакованного индекса полигона (`layer_of`/`polygon_of`): старшие
/// 8 бит — слой, младшие 24 — номер полигона внутри слоя. На ней стоит потолок
/// `MAX_CHUNKS`, и её же читают `locate`, `segment_clear` и `verify_seams`.
#[test]
fn a_packed_polygon_index_splits_into_layer_and_number() {
    let packed = ((MAX_CHUNKS - 1) << 24) | 0x00AB_CDEF;
    assert_eq!(layer_of(packed), (MAX_CHUNKS - 1) as usize);
    assert_eq!(polygon_of(packed), 0x00AB_CDEF);
    // плоский меш — слой 0, номер полигона идёт как есть
    assert_eq!(layer_of(7), 0);
    assert_eq!(polygon_of(7), 7);
}

/// Чанк, целиком накрытый препятствием, даёт слой без единого полигона — на
/// Нью-Йорке таких четыре (река, сплошная застройка). `Layer::bake` на нём
/// уходил в бесконечную рекурсию `BVH2d::build`, и процесс умирал с `stack
/// overflow` в воркере `AsyncComputeTaskPool` (см. `bake_polygon_finder` в
/// `vendor/polyanya`). Провал этого теста выглядит не как assert, а как
/// падение всего тестового бинаря — так и задумано.
#[test]
fn a_chunk_fully_covered_by_an_obstacle_builds_an_empty_layer() {
    let blocked = PolyArea {
        // с запасом за края чанка (0,0)-(400,400), чтобы после инфляции
        // радиусом агента не осталось щели вдоль кромки
        outer: vec![
            Vec2::new(-50.0, -50.0),
            Vec2::new(CHUNK_METERS + 50.0, -50.0),
            Vec2::new(CHUNK_METERS + 50.0, CHUNK_METERS + 50.0),
            Vec2::new(-50.0, CHUNK_METERS + 50.0),
        ],
        holes: vec![],
        kind: crate::map::osm::model::AreaKind::Building,
        height: None,
        entrances: vec![],
    };
    let input = PolymeshInput {
        buildings: vec![blocked],
        water: vec![],
        water_lines: vec![],
        walls: vec![],
        roads: vec![],
    };
    let mesh = build_polymesh(&input, 0.2, None, Some(CHUNK_METERS)).expect("not cancelled");

    assert!(
        mesh.mesh.layers[0].polygons.is_empty(),
        "чанк под сплошным зданием не даёт свободных полигонов"
    );
    assert!(!mesh.contains(Vec2::new(200.0, 200.0)), "внутри перекрытия");
    // остальная карта не пострадала: соседние чанки строятся и ходятся
    let from = Vec2::new(1000.0, 1000.0);
    let to = Vec2::new(1400.0, 1400.0);
    assert!(
        find_path_polymesh(&mesh, from, to).is_some(),
        "пустой слой не должен ломать поиск в остальной карте"
    );
}

/// Обе стороны чанка означают ровно то, что про них написано в `settings.rs`:
/// «чанк размером с мир» — один слой без швов, штатная сторона — иерархия,
/// влезающая в 8-битный индекс слоя (`MAX_CHUNKS`, он остался здесь).
#[test]
fn chunk_sides_mean_what_the_settings_say() {
    assert_eq!(
        chunk_grid(MAP_SIZE, Some(POLYMESH_FLAT_CHUNK_METERS)),
        UVec2::ONE,
        "плоская сторона обязана давать один слой"
    );
    let grid = chunk_grid(MAP_SIZE, Some(POLYMESH_CHUNK_TARGET_METERS));
    assert!(grid.element_product() > 1, "штатная сторона даёт иерархию");
    assert!(
        grid.element_product() <= MAX_CHUNKS,
        "сетка чанков обязана влезать в потолок слоёв"
    );
}

/// Сторона чанка приходит снаружи — из `QWE_POLYMESH_CHUNK_M` для офлайн-прогонов
/// и параметром из тестов, — и вырожденное значение обязано откатываться к
/// дефолту, а не выдавать сетку в `u32::MAX` чанков: `map / 0` даёт `inf`,
/// `as_uvec2` насыщается, а произведение `u32` заворачивается ровно под потолок
/// `MAX_CHUNKS` и объявляет, что «уложились».
#[test]
fn a_degenerate_chunk_side_falls_back_to_the_default_grid() {
    let default = chunk_grid(MAP_SIZE, Some(POLYMESH_CHUNK_TARGET_METERS));
    assert!(
        default.element_product() <= MAX_CHUNKS,
        "дефолтная сторона обязана укладываться в потолок слоёв, вышло {default}"
    );

    for bad in [0.0, -400.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            chunk_grid(MAP_SIZE, Some(bad)),
            default,
            "сторона {bad} — не длина, берётся {POLYMESH_CHUNK_TARGET_METERS} м"
        );
    }

    // положительная, но мельче пола — это всё-таки просьба «мельче»: сторона
    // поднимается до CHUNK_MIN_METERS, и потолок слоёв всё равно держится
    for tiny in [f32::MIN_POSITIVE, 1e-30, 0.001, 0.5] {
        let grid = chunk_grid(MAP_SIZE, Some(tiny));
        assert!(
            (1..=MAX_CHUNKS).contains(&grid.element_product()),
            "сторона {tiny} м дала сетку {grid}"
        );
    }

    // верхний конец не зажат: плоский слой строится тем же путём
    assert_eq!(
        chunk_grid(MAP_SIZE, Some(POLYMESH_FLAT_CHUNK_METERS)),
        UVec2::ONE,
        "сторона больше карты обязана давать один слой без швов"
    );
}

/// Продублированный узел не имеет права двигать кромку ленты. Без схлопывания
/// нулевой сегмент подставляет в `miter_offsets` запасную ось `Vec2::X`: у
/// ленты, идущей на запад, офсет на этой вершине разворачивается на 180°,
/// кромки меняются местами и кольцо складывается бабочкой.
#[test]
fn a_duplicated_node_does_not_flip_the_ribbon_edge() {
    let start = Vec2::new(1000.0, 750.0);
    let node = Vec2::new(500.0, 750.0);
    let end = Vec2::new(0.0, 750.0);
    assert_eq!(
        ribbon_outline(&[start, node, node, end], 6.0),
        ribbon_outline(&[start, node, end], 6.0),
        "дубль узла обязан схлопнуться до исходной ломаной"
    );
}

/// Путь, весь уместившийся в одну точку, ленты не даёт: после схлопывания в
/// нём меньше двух вершин, и офсеты считать не по чему.
#[test]
fn a_path_collapsing_to_one_point_gives_no_ribbon() {
    let point = Vec2::new(10.0, 10.0);
    assert!(ribbon_outline(&[point, point], 6.0).is_none());
}

/// То же для кольца из boolean: i_overlay считает в целочисленной сетке мельче
/// шага f32 на масштабе карты, поэтому микроребро приходит парой одинаковых
/// вершин. На такой паре угол не раздувается биссектрисой, а срезается — ровно
/// на радиус агента внутрь барьера.
#[test]
fn a_repeated_ring_vertex_does_not_dent_the_inflated_contour() {
    let corner = Vec2::new(100.0, 0.0);
    let far = Vec2::new(100.0, 100.0);
    let near = Vec2::new(0.0, 100.0);
    assert_eq!(
        inflate_ring(&[Vec2::ZERO, corner, corner, far, near], 0.2),
        inflate_ring(&[Vec2::ZERO, corner, far, near], 0.2),
        "повторённая вершина обязана схлопнуться до исходного кольца"
    );
}

/// Старт, не севший на меш, остаётся в полилинии **своей точкой**. Иначе второй
/// точкой пути идёт первый waypoint воронки, посчитанный от посадки, и пешка
/// шагает к нему по отрезку, который не проверял никто: воронка гарантирует
/// свои звенья, `smoothed` — только срезы.
#[test]
fn a_snapped_start_stays_in_the_path_as_its_own_waypoint() {
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

    // в полуметре от стены при радиусе агента 1 м — внутри инфляции
    let from = Vec2::new(999.5, 1015.0);
    let to = Vec2::new(960.0, 1015.0);
    assert!(!mesh.contains(from), "старт обязан быть вне меша");

    let path = find_path_polymesh(&mesh, from, to).expect("на запад путь свободен");
    assert_eq!(
        path.first(),
        Some(&from),
        "контракт: путь начинается там, где пешка стоит"
    );
    let snapped = path[1];
    assert!(
        mesh.contains(snapped),
        "вторая точка обязана лежать на меше: {snapped:?}"
    );
    let tolerance = POLYMESH_SEARCH_DELTA * (POLYMESH_SEARCH_STEPS - 1) as f32;
    assert!(
        snapped.distance(from) <= tolerance + 1.0e-3,
        "второй точкой идёт посадка старта, а не waypoint воронки: {} м",
        snapped.distance(from)
    );

    // старт на меше лишней точки не порождает: посадка вернула его же
    let free = Vec2::new(990.0, 1015.0);
    assert!(mesh.contains(free));
    let straight = find_path_polymesh(&mesh, free, to).expect("на запад путь свободен");
    assert_eq!(straight.first(), Some(&free));
    assert!(
        straight[1].distance(free) > tolerance,
        "дубля стартовой точки быть не должно: {straight:?}"
    );
}
