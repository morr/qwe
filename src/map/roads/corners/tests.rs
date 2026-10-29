use super::super::VergeZoomBucket;
use super::super::network::pairs::PairRun;
use super::super::shape::RoadShape;
use super::super::tapers::Taper;
use super::*;
use crate::map::RoadStyle;
use crate::map::osm::Highway;
use crate::map::osm::fixture::street;
use crate::map::osm::model::point_in_polygon;
use crate::map::osm::{MapData, RoadLine};

fn returns_of(roads: &[RoadLine]) -> Vec<(RoadClass, Vec<Vec2>)> {
    walked_returns_of(roads, false).roads
}

/// Карта из одних дорог — вход [`Drawn::for_test`].
fn map_of(roads: &[RoadLine]) -> MapData {
    MapData {
        roads: roads.to_vec(),
        ..default()
    }
}

/// Скругления с тротуарами по карте (`sidewalks`) или без них.
fn walked_returns_of(roads: &[RoadLine], sidewalks: bool) -> KerbReturns {
    let map = map_of(roads);
    kerb_returns(&Drawn::for_test(&map).with_sidewalks(sidewalks), 1.0)
}

/// Мощёная дорожка 3.5 м — как [`fixture::footway`], но с покрытием:
/// наезд плитки спрашивает именно мощёные.
fn paved_path(points: Vec<Vec2>) -> RoadLine {
    RoadLine {
        pavement: Some(crate::map::osm::model::Pavement::Paved),
        ..crate::map::osm::fixture::footway(points)
    }
}

/// Слияние трёх мощёных дорожек: сквозная запад–восток и примыкающая
/// сверху, двор — клином под кромкой восточной ветви. Наезд
/// ([`tile_overlaps`]) закрывает клин полосой плитки вдоль кромки.
#[test]
fn a_tile_overlap_covers_the_yard_wedge_at_a_paved_merge() {
    let roads = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![Vec2::new(0.0, 50.0), Vec2::ZERO]),
    ];
    let mut map = map_of(&roads);
    map.landuse = vec![crate::map::osm::fixture::area(
        crate::map::osm::model::AreaKind::Residential,
        vec![
            Vec2::new(0.5, -1.8),
            Vec2::new(12.0, -2.5),
            Vec2::new(2.0, -9.0),
        ],
    )];
    let found = kerb_returns(&Drawn::for_test(&map).with_sidewalks(false), 1.0);
    // наезд один и накрывает щель двора: полосу за кромкой восточной
    // ветви; скругления туда не доходят
    let wedge = Vec2::new(TILE_OVERLAP_REACH * 0.5, -(1.75 + TILE_OVERLAP_WIDTH * 0.5));
    let overlaps: Vec<&Vec<Vec2>> = found
        .overlaps
        .iter()
        .filter(|outline| point_in_polygon(wedge, outline))
        .collect();
    assert_eq!(overlaps.len(), 1, "наезд один и накрывает щель двора");
    let outline = overlaps[0];
    // от кромки (с заходом под ленту) на ширину наезда, длиной REACH
    assert!(outline.contains(&Vec2::new(0.0, -(1.75 - OUTER_OVERLAP))));
    assert!(outline.contains(&Vec2::new(0.0, -(1.75 + TILE_OVERLAP_WIDTH))));
    assert!(outline.contains(&Vec2::new(TILE_OVERLAP_REACH, -(1.75 + TILE_OVERLAP_WIDTH))));
    assert!(outline.contains(&Vec2::new(TILE_OVERLAP_REACH, -(1.75 - OUTER_OVERLAP))));
    // без двора наезда нет: обе кромки улицы свободны, полосы не
    // раздулись
    let bare = kerb_returns(&Drawn::for_test(&map_of(&roads)).with_sidewalks(false), 1.0);
    assert!(bare.overlaps.is_empty());
    assert_eq!(bare.sidewalks.len(), found.sidewalks.len());
}

/// Сумма угловых кусков узла по всем слоям: проверка «ни одного куска» —
/// это проверка её нуля.
fn corner_pieces(found: &KerbReturns) -> usize {
    found.sidewalks.len() + found.roads.len() + found.bends.len() + found.noses.len()
}

/// Крест двух мощёных дорожек: оба луча проходят узел насквозь, ленты
/// аллей на узлах не режутся, и стык кроют они сами — ни уголков, ни
/// наездов (Тула, Первомайская × проспект Ленина).
#[test]
fn a_crossing_of_paved_paths_draws_no_corner_pieces() {
    let roads = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![
            Vec2::new(0.0, -50.0),
            Vec2::ZERO,
            Vec2::new(0.0, 50.0),
        ]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&roads)).with_sidewalks(false), 1.0);
    assert_eq!(corner_pieces(&found), 0);
    assert!(found.overlaps.is_empty());
}

/// Тот же крест из четырёх ways, разбитых в узле: каждый луч «кончается»
/// ([`Arm::end`]), но продолжение видно по встречному лучу — кусков
/// по-прежнему нет.
#[test]
fn a_crossing_of_split_paths_still_draws_nothing() {
    let roads = [
        paved_path(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)]),
        paved_path(vec![Vec2::new(0.0, -50.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&roads)).with_sidewalks(false), 1.0);
    assert_eq!(corner_pieces(&found), 0);
    assert!(found.overlaps.is_empty());
}

/// Примыкание дорожки к сквозной (Т): устье кроют ленты, уголков нет, а
/// наезд у двора остаётся — щель между кромкой и двором настоящая.
#[test]
fn a_path_joining_a_through_path_draws_no_corner_pieces() {
    let roads = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![Vec2::new(0.0, 50.0), Vec2::ZERO]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&roads)).with_sidewalks(false), 1.0);
    assert_eq!(corner_pieces(&found), 0);
}

/// L-угол из двух кончающихся дорожек — вырез у торцов настоящий, кусок
/// рисуется: у двух мощёных — плиткой в слой тротуаров, у мощёной с
/// грунтовой — песком в слой аллей (плиточное пятно на грунтовой читалось
/// бы пятном).
#[test]
fn an_l_of_two_ending_paths_keeps_one_corner() {
    let paved = [
        paved_path(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&paved)).with_sidewalks(false), 1.0);
    // пара из двух лучей обходит круг в обе стороны: филет изнутри и
    // наружный веер
    assert_eq!(corner_pieces(&found), 2);
    assert_eq!(found.sidewalks.len(), 2, "обе мощёные — плиткой");

    let mixed = [
        paved_path(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO]),
        crate::map::osm::fixture::footway(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&mixed)).with_sidewalks(false), 1.0);
    assert_eq!(corner_pieces(&found), 2);
    assert!(found.sidewalks.is_empty(), "смешанная пара — не плиткой");
    assert_eq!(found.roads.len(), 2);
    assert_eq!(
        found.roads[0].0,
        RoadClass::Alley,
        "смешанная пара — песком"
    );
}

/// Слияние дорожек, разрезанных OSM в узле: у южного и северного плеча
/// есть встречный луч, у зебры, входящей в их слияние, все соседи
/// проходные — уголковые куски их пар пропущены, срезу нечем прикрыться,
/// и ленты кончаются круглыми шапками: у слияния срез торчал клином и
/// ступенькой на стыке лент разной ширины (Тула, Фрунзе ×
/// Красноармейский). У L-поворота двух кончающихся дорожек кусок ляжет —
/// торцы остаются прямыми.
#[test]
fn a_path_arm_with_an_opposite_keeps_its_round_cap() {
    let merge = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![Vec2::new(0.0, -50.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&merge)).with_sidewalks(false), 1.0);
    assert!(!found.butt(1)[1], "у южного плеча есть встречный — шапка");
    assert!(
        !found.butt(2)[0],
        "у северного плеча есть встречный — шапка"
    );

    // зебра входит в слияние двух продолжающихся дорожек: соседей без
    // встречного у неё нет, куска не будет и у неё — шапка
    let joined = [
        paved_path(vec![Vec2::new(50.0, 0.0), Vec2::ZERO]),
        paved_path(vec![Vec2::new(0.0, -50.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(
        &Drawn::for_test(&map_of(&joined)).with_sidewalks(false),
        1.0,
    );
    assert!(!found.butt(0)[0], "у зебры все соседи проходные — шапка");

    let corner = [
        paved_path(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO]),
        paved_path(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)]),
    ];
    let found = kerb_returns(
        &Drawn::for_test(&map_of(&corner)).with_sidewalks(false),
        1.0,
    );
    assert!(
        found.butt(0)[1] && found.butt(1)[0],
        "L-поворот без встречных — прямые торцы"
    );
}

/// Мощёная дорожка заданной ширины — как `paved_path`, но ширина своя:
/// широкая зебра против узкого тротуара.
fn paved_path_wide(points: Vec<Vec2>, width: f32) -> RoadLine {
    RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        pavement: Some(crate::map::osm::model::Pavement::Paved),
        ..crate::map::osm::fixture::street(points, width)
    }
}

/// Широкое плечо, кончающееся в узле поверх чужой узкой ленты: полудиск
/// шапки радиусом в свою полуширину ложится на ленту и выступает за её
/// кромку клином на газоне (Тула, Первомайская × проспект Ленина, юг —
/// зебра шире изгибаемого тротуара; витрина 02, СЗ — зебра шире тротуаров).
/// Торец режется прямо, в плоскость узла.
#[test]
fn a_wider_ending_path_is_cut_square_at_the_node() {
    // изгибаемый узкий тротуар (запад–юг–запад → восток) и широкая зебра
    // с северо-запада, идущая вдоль его восточной половины
    let bent = paved_path_wide(
        vec![Vec2::new(-40.0, -30.0), Vec2::ZERO, Vec2::new(40.0, -20.0)],
        2.5,
    );
    let zebra = paved_path_wide(vec![Vec2::new(-30.0, 20.0), Vec2::ZERO], 4.0);
    let found = kerb_returns(
        &Drawn::for_test(&map_of(&[bent, zebra])).with_sidewalks(false),
        1.0,
    );
    assert!(found.butt(1)[1], "широкий торец в узел — срез");
}

/// То же в узле из трёх торцов (витрина 02, СЗ): широкая зебра режется
/// прямо, узкие тротуары оставляют круглые шапки — их накрывают ленты
/// друг друга и зебры.
#[test]
fn at_a_node_of_three_ends_only_the_widest_gets_a_square_cut() {
    let roads = [
        paved_path_wide(vec![Vec2::new(-4.5, -9.0), Vec2::ZERO], 2.5),
        paved_path_wide(vec![Vec2::ZERO, Vec2::new(3.0, 10.75)], 2.5),
        paved_path_wide(vec![Vec2::ZERO, Vec2::new(40.0, -9.0)], 3.0),
    ];
    let found = kerb_returns(&Drawn::for_test(&map_of(&roads)).with_sidewalks(false), 1.0);
    assert!(found.butt(2)[0], "широкая зебра — срез");
    assert!(!found.butt(0)[1], "узкий тротуар — шапка");
    assert!(!found.butt(1)[0], "узкий тротуар — шапка");
}

/// Двор, подступивший к кромке у самого узла, — не щель: наезд закрывает
/// щель, раскрывающуюся от узла (голая земля, дальше двор), а здесь лёг
/// бы прямоугольной плитой на газон (Тула, Первомайская × проспект
/// Ленина — угол двора у перехода).
#[test]
fn no_tile_overlap_where_the_yard_touches_the_node() {
    let roads = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![Vec2::new(0.0, 50.0), Vec2::ZERO]),
    ];
    let mut map = map_of(&roads);
    map.landuse = vec![crate::map::osm::fixture::area(
        crate::map::osm::model::AreaKind::Residential,
        vec![
            Vec2::new(0.0, -1.8),
            Vec2::new(12.0, -2.5),
            Vec2::new(8.0, -9.0),
            Vec2::new(0.0, -9.0),
        ],
    )];
    let found = kerb_returns(&Drawn::for_test(&map).with_sidewalks(false), 1.0);
    assert!(
        found.overlaps.is_empty(),
        "двор у самой кромки у узла — наезда нет"
    );
}

/// Клин двора, вошедший в кромку остриём мельче ширины наезда: у ближнего
/// конца полосы двор от кромки не уходит вглубь, угол слияния — газонный,
/// и полоса ложилась бы на него плитой (Тула, Первомайская × проспект
/// Ленина, западный угол: клин кармана земли — в угол слияния зебры,
/// двор-дорожки и тротуара).
#[test]
fn no_tile_overlap_where_the_yard_wedge_is_shallower_than_the_strip() {
    let roads = [
        paved_path(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        paved_path(vec![Vec2::new(0.0, 50.0), Vec2::ZERO]),
    ];
    let mut map = map_of(&roads);
    map.landuse = vec![crate::map::osm::fixture::area(
        crate::map::osm::model::AreaKind::Residential,
        vec![
            Vec2::new(0.5, -1.8),
            Vec2::new(12.0, -2.5),
            Vec2::new(4.0, -6.0),
        ],
    )];
    let found = kerb_returns(&Drawn::for_test(&map).with_sidewalks(false), 1.0);
    assert!(found.overlaps.is_empty(), "клин мельче полосы — наезда нет");
}

/// Тротуар улицы 8 м — как его считает `osm::model::sidewalk_band`.
const SIDEWALK: f32 = 1.76;

/// Перекрёсток двух улиц 8 м в начале координат.
fn crossing() -> [RoadLine; 2] {
    [
        east_west(),
        street(
            vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
            8.0,
        ),
    ]
}

/// Сквозная улица 8 м по оси x через узел в начале координат.
fn east_west() -> RoadLine {
    street(
        vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
        8.0,
    )
}

#[test]
fn a_crossing_gets_four_rounded_corners_outside_both_ribbons() {
    let found = returns_of(&crossing());
    assert_eq!(found.len(), 4);
    for (_, outline) in &found {
        // угол — на пересечении краёв, чуть под лентами
        assert!(
            (outline[0].abs() - Vec2::splat(4.0 - OVERLAP)).length() < 1e-3,
            "{:?}",
            outline[0]
        );
        let corner = Vec2::splat(4.0) * outline[0].signum();
        // и сама дуга за краями обеих лент
        for point in &outline[1..] {
            assert!(point.x.abs() >= 4.0 - OVERLAP - 1e-3 && point.y.abs() >= 4.0 - OVERLAP - 1e-3);
        }
        // радиус двух жилых улиц: все точки дуги на нём от центра
        let centre = corner + Vec2::splat(STREET_RADIUS) * outline[0].signum();
        for point in &outline[2..outline.len() - 1] {
            let radius = point.distance(centre);
            assert!((radius - STREET_RADIUS).abs() < 1e-2, "{radius}");
        }
    }
}

#[test]
fn a_road_corner_arc_is_cut_at_the_kerb_step() {
    // Дорожная группа резала дугу общим шагом: у обычного перекрёстка её
    // кромка — сама линия бордюра между прямыми краями двух улиц, и стыки
    // хорд общего шага (~13° на угол 6 м) читались гранями (Тула, Фёдора
    // Смирнова × Красноармейский, 3399 3835).
    for (_, outline) in returns_of(&crossing()) {
        for triple in outline[2..outline.len() - 1].windows(3) {
            let incoming = (triple[1] - triple[0]).normalize();
            let outgoing = (triple[2] - triple[1]).normalize();
            let turn = incoming.angle_to(outgoing).abs().to_degrees();
            // втрое мельче общего шага: ~4° на радиусе 6 м
            assert!(turn < 6.0, "стык дуги {turn:.1}°: {outline:?}");
        }
    }
}

/// Радиус скругления прямого угла: касательная от угла краёв до дуги
/// равна ему. Третья вершина контура — точка касания на первом луче.
fn right_angle_radius(outline: &[Vec2]) -> f32 {
    let corner = outline[0] + (outline[0].signum() * OVERLAP);
    outline[2].distance(corner)
}

#[test]
fn a_t_junction_rounds_only_the_two_turning_corners() {
    let through = east_west();
    let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
    let found = returns_of(&[through, side]);
    assert_eq!(found.len(), 2);
    // обе дуги со стороны примыкания
    for (_, outline) in &found {
        assert!(outline.iter().all(|point| point.y >= 4.0 - OVERLAP - 1e-3));
    }
    let corner = &found[0].1;
    assert!(!point_in_polygon(Vec2::new(0.0, 10.0), corner));
}

#[test]
fn a_street_and_a_footway_are_not_rounded_together() {
    let through = east_west();
    let path = RoadLine {
        class: RoadClass::Alley,
        ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 3.5)
    };
    assert!(returns_of(&[through, path]).is_empty());
}

#[test]
fn a_vertex_on_a_straight_arm_does_not_cut_the_corner() {
    // проезд пересекает тротуар в двух метрах от улицы — узел на прямой,
    // но скругление по-прежнему ложится полным радиусом
    let through = east_west();
    let drive = street(
        vec![
            Vec2::new(0.0, 40.0),
            Vec2::new(0.05, 6.0),
            Vec2::new(0.0, 2.0),
            Vec2::ZERO,
        ],
        5.0,
    );
    let found = returns_of(&[through, drive]);
    assert_eq!(found.len(), 2);
    for (_, outline) in &found {
        // с проездом радиус проезда: дуга уходит вдоль проезда на него от
        // края улицы
        let reach = outline.iter().map(|point| point.y).fold(0.0, f32::max);
        assert!((reach - (4.0 + DRIVE_RADIUS)).abs() < 0.1, "{reach}");
    }
}

#[test]
fn a_short_arm_limits_the_radius() {
    // вторая вершина поперечной улицы в двух метрах за краем — дуга не
    // длиннее этого
    let through = east_west();
    let side = street(
        vec![Vec2::new(30.0, 30.0), Vec2::new(0.0, 6.0), Vec2::ZERO],
        8.0,
    );
    for (_, outline) in returns_of(&[through, side]) {
        for point in &outline {
            assert!(point.y <= 6.0 + 1e-3, "{point:?}");
        }
    }
}

/// Широкая улица сливает вершину оси ближе четверти своей ширины, и её
/// край идёт прямо на следующую оставленную: дуга скругления кончается на
/// этом крае, а не на прямой к слитой вершине — та уводила её конец за
/// кромку в тротуар (Тула, Халтурина × Гоголевская, Сойфера × Лейтейзена).
#[test]
fn the_corner_ends_on_the_edge_the_ribbon_draws() {
    let wide = street(
        vec![
            Vec2::new(0.0, 60.0),
            Vec2::ZERO,
            Vec2::new(-0.45, -4.5),
            Vec2::new(0.0, -40.0),
        ],
        20.0,
    );
    let drive = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 5.0);
    let found = returns_of(&[wide, drive]);
    let (_, south_west) = found
        .iter()
        .find(|(_, outline)| outline[0].x < 0.0 && outline[0].y < 0.0)
        .expect("скругление юго-западного угла");
    // лента идёт от узла прямо на (0, −40): её западный край — x = −10
    let on_edge = south_west[south_west.len() - 2];
    assert!((on_edge.x + 10.0).abs() < 1e-3, "{on_edge:?}");
    assert!(
        south_west
            .iter()
            .all(|point| point.x <= -10.0 + OVERLAP + 1e-3),
        "{south_west:?}"
    );
}

/// Плитка у бордюра угла с газоном обочины кончается на дорожке,
/// пересекающей улицу сразу за углом: за её лентой снова газон, и хвост
/// площадки торчал в него квадратом плитки (Тула, Гоголевская у Халтурина).
#[test]
fn the_kerb_pad_stops_at_a_path_crossing_the_street() {
    use crate::map::osm::model::SidewalkSide;
    let verged = |points: Vec<Vec2>| RoadLine {
        sidewalks: [SidewalkSide::None; 2],
        verges: [12.0; 2],
        ..street(points, 12.0)
    };
    let path = RoadLine {
        class: RoadClass::Alley,
        ..street(
            vec![
                Vec2::new(14.0, -30.0),
                Vec2::new(14.0, 0.0),
                Vec2::new(14.0, 30.0),
            ],
            3.0,
        )
    };
    let roads = [
        verged(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(14.0, 0.0),
            Vec2::new(50.0, 0.0),
        ]),
        verged(vec![
            Vec2::new(0.0, -50.0),
            Vec2::ZERO,
            Vec2::new(0.0, 50.0),
        ]),
        path,
    ];
    let found = walked_returns_of(&roads, true);
    let north_east: Vec<Vec2> = found
        .verges
        .iter()
        .flatten()
        .copied()
        .filter(|point| point.x > 0.0 && point.y > 0.0)
        .collect();
    assert!(!north_east.is_empty(), "у угла нет площадки");
    // касание дуги в (12, 6), дорожка — на x = 14
    let reach = north_east.iter().map(|point| point.x).fold(0.0, f32::max);
    assert!(reach <= 14.0 + 1e-3, "{reach}");
}

/// Площадка у угла, где за бордюром полоса тротуара уже площадки, а за ней
/// газон обочины: площадка — не шире тротуара, иначе у конца хвоста и у
/// начала дуги кромка газона шла уступом (Тула, Фёдора Смирнова, R1).
#[test]
fn a_kerb_pad_is_no_wider_than_the_sidewalk_band() {
    let verged = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Residential,
        verges: [12.0; 2],
        ..street(points, 12.0)
    };
    let roads = [
        verged(vec![
            Vec2::new(-50.0, 0.0),
            Vec2::ZERO,
            Vec2::new(50.0, 0.0),
        ]),
        verged(vec![
            Vec2::new(0.0, -50.0),
            Vec2::ZERO,
            Vec2::new(0.0, 50.0),
        ]),
    ];
    let map = map_of(&roads);
    let drawn = Drawn::for_test(&map).with_sidewalks(true);
    let sidewalk = drawn.sidewalk_on(0, 0).expect("у улицы тротуар");
    assert!(sidewalk < KERB_PAD_WIDTH - 0.1, "{sidewalk}");
    let found = kerb_returns(&drawn, 1.0);
    let pad: Vec<Vec2> = found
        .verges
        .iter()
        .flatten()
        .copied()
        .filter(|point| point.x > 0.0 && point.y > 0.0)
        .collect();
    assert!(!pad.is_empty(), "у угла нет площадки");
    // от кромки — бордюрной дуги радиуса 6 м или прямых краёв — не дальше
    // полосы тротуара
    let centre = Vec2::splat(6.0 + STREET_RADIUS);
    for point in pad {
        let off = if point.x >= centre.x {
            point.y - 6.0
        } else if point.y >= centre.y {
            point.x - 6.0
        } else {
            STREET_RADIUS - point.distance(centre)
        };
        assert!(off <= sidewalk + 0.01, "{point:?} за кромкой на {off}");
    }
}

/// Площадка за дугой уже своей ширины лежит на всю ширину от обеих
/// кромок: внутренняя дуга сходилась в центр, и за ним в угол площадки
/// проглядывал газон зубцом (Тула, Халтурина у Гоголевской).
#[test]
fn a_kerb_pad_behind_a_tight_arc_keeps_its_depth() {
    use crate::map::osm::model::SidewalkSide;
    let verged = |road: RoadLine| RoadLine {
        sidewalks: [SidewalkSide::None; 2],
        verges: [12.0; 2],
        ..road
    };
    // оба прямых края кончаются у касаний дуги — хвостов у площадки нет
    let roads = [
        verged(street(
            vec![
                Vec2::new(-50.0, 0.0),
                Vec2::ZERO,
                Vec2::new(5.0, 0.0),
                Vec2::new(20.0, -20.0),
            ],
            12.0,
        )),
        verged(with_highway(
            street(
                vec![Vec2::ZERO, Vec2::new(0.0, 8.5), Vec2::new(20.0, 30.0)],
                5.0,
            ),
            Highway::Service,
        )),
    ];
    let found = walked_returns_of(&roads, true);
    // угол кромок (2.5, 6), дуга проезда в 2.5 м — центр (5, 8.5); за ним
    // площадка идёт до 3 м от обеих кромок
    let behind = Vec2::new(5.25, 8.75);
    assert!(
        found
            .verges
            .iter()
            .any(|outline| point_in_polygon(behind, outline)),
        "{:?}",
        found.verges
    );
}

/// Асфальт двумя way с изломом в узле, где его пересекает грунтовка:
/// торцы прямые, скруглений к грунту нет — щель с наружной стороны
/// излома закрыта асфальтом, а не светится грунтом (Калуга, 06).
#[test]
fn asphalt_kinked_across_a_dirt_road_leaves_no_gap_at_the_seam() {
    let mut dirt = street(
        vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
        6.0,
    );
    dirt.pavement = Some(crate::map::osm::model::Pavement::Unpaved);
    let roads = [
        street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0),
        street(vec![Vec2::ZERO, Vec2::new(50.0, 5.0)], 8.0),
        dirt,
    ];
    let found = walked_returns_of(&roads, false);
    assert!(found.butt(0)[1] && found.butt(1)[0], "торцы в узле прямые");
    // между торцами с южной стороны: левее торца восточного и правее
    // западного — ни одна лента сюда не доходит
    let gap = Vec2::new(0.15, -3.5);
    assert!(
        found
            .roads
            .iter()
            .any(|(_, outline)| point_in_polygon(gap, outline)),
        "щель у излома открыта: {:?}",
        found.roads
    );
    // и только она: к грунтовке асфальт не скругляется
    assert_eq!(found.roads.len(), 1, "{:?}", found.roads);
}

/// Угол двух грунтовок — грунтом и малым радиусом: уличные 6 м ложились
/// бордюрной дугой шире самого проезда (Тула, проезды Мясново). Та же
/// крестовина в асфальте свой радиус улиц сохраняет.
#[test]
fn two_dirt_roads_meet_with_a_small_corner() {
    let mut dirt = crossing();
    for road in &mut dirt {
        road.pavement = Some(crate::map::osm::model::Pavement::Unpaved);
    }
    let found = walked_returns_of(&dirt, false);
    assert!(found.roads.is_empty(), "асфальта нет: {:?}", found.roads);
    assert_eq!(found.unpaved.len(), 4);
    for outline in &found.unpaved {
        let radius = right_angle_radius(outline);
        assert!((radius - DIRT_RADIUS).abs() < 1e-2, "{radius}");
    }
    let paved = returns_of(&crossing());
    assert!((right_angle_radius(&paved[0].1) - STREET_RADIUS).abs() < 1e-2);
}

/// Поперечная в 12 м с клином на дальнем конце в 14 м: подогнанная длина
/// клина — 5.4 м (доля пути), кромка прямая только 6.6 м от узла, и дуга
/// дальше не заходит — в клине лента уже, и касательная торчала из-под
/// него шипом (витрина 08). Капа по сырой длине здесь держала бы кромку
/// на 12 − 14 < 0 — вовсе без пробега.
#[test]
fn a_taper_on_the_arm_limits_the_radius() {
    let map = map_of(&[
        east_west(),
        street(vec![Vec2::ZERO, Vec2::new(0.0, 12.0)], 8.0),
    ]);
    let drawn = Drawn::for_test(&map).with_sidewalks(false).with_taper(
        1,
        1,
        Taper {
            length: 14.0,
            seam: 0,
            sides: [true; 2],
        },
    );
    let found = kerb_returns(&drawn, 1.0);
    assert!(!found.roads.is_empty());
    for (_, outline) in &found.roads {
        for point in outline {
            assert!(point.y <= 6.6 + 1e-3, "{point:?}");
        }
    }
}

/// Клин длиннее дороги: сырая длина клина бывает больше пути, и капа
/// прямого пробега по ней уходила в минус — все лучи дороги вставали
/// с нулевым пробегом, дорожный угол у устья переулка не строился,
/// и асфальт торчал шипом в чужой угол (Тула, съезд с Фрунзе
/// в стоянку, N2). Капа — по подогнанной длине клина.
#[test]
fn a_taper_longer_than_the_road_keeps_the_mouth_corner() {
    let map = map_of(&[
        street(vec![Vec2::ZERO, Vec2::new(30.0, 0.0)], 8.0),
        street(vec![Vec2::ZERO, Vec2::new(0.0, -20.0)], 4.0),
    ]);
    let drawn = Drawn::for_test(&map).with_sidewalks(false).with_taper(
        0,
        1,
        Taper {
            length: 80.0,
            seam: 1,
            sides: [true; 2],
        },
    );
    let found = kerb_returns(&drawn, 1.0);
    // дуга устья в квадранте между улицей и переездом. Без подогнанной
    // капы пробег луча улицы нулевой, дуга проваливается в bend, и среди
    // дорожных углов остаётся только веер противоположного квадранта
    let mouth = found
        .roads
        .iter()
        .any(|(_, outline)| outline.iter().any(|point| point.x > 1.5 && point.y < -5.0));
    assert!(mouth, "{:?}", found.roads);
}

/// Узкая улица 8 м переходит в широкую 11 м клином в 30 м, и в 20 м от шва
/// в широкую примыкает поперечная: кромка там — на 5.0 м от оси, между
/// соседской 4 и своей 5.5, и идёт наклонно (Тула, Сойфера у Фёдора
/// Смирнова).
fn junction_inside_a_taper() -> KerbReturns {
    let map = map_of(&[
        street(vec![Vec2::new(0.0, 60.0), Vec2::ZERO], 8.0),
        street(
            vec![Vec2::ZERO, Vec2::new(0.0, -20.0), Vec2::new(0.0, -80.0)],
            11.0,
        ),
        street(vec![Vec2::new(0.0, -20.0), Vec2::new(40.0, -20.0)], 8.0),
    ]);
    let drawn = Drawn::for_test(&map).with_sidewalks(false).with_taper(
        1,
        0,
        Taper {
            length: 30.0,
            seam: 0,
            sides: [true; 2],
        },
    );
    kerb_returns(&drawn, 1.0)
}

/// Кромка клина широкой улицы: полуширина на `y` (от шва в 0 вниз).
fn wedge_edge(y: f32) -> f32 {
    4.0 + 1.5 * (-y / 30.0).clamp(0.0, 1.0)
}

#[test]
fn a_junction_inside_a_taper_wedge_stands_on_the_wedge_edge() {
    let found = junction_inside_a_taper();
    let east: Vec<&Vec<Vec2>> = found
        .roads
        .iter()
        .map(|(_, outline)| outline)
        .filter(|outline| outline[0].x > 0.0)
        .collect();
    assert!(!east.is_empty());
    for outline in east {
        // угол — на наклонной кромке клина, чуть под лентой
        let corner = outline[0];
        assert!(
            (corner.x - (wedge_edge(corner.y) - OVERLAP)).abs() < 0.05,
            "{corner:?}"
        );
        // и касание дуги с широкой улицей — на ней же: из двух концов
        // дуги тот, что дальше от оси поперечной
        let ends = [outline[2], outline[outline.len() - 2]];
        let on_wide = if (ends[0].y + 20.0).abs() > (ends[1].y + 20.0).abs() {
            ends[0]
        } else {
            ends[1]
        };
        assert!(
            (on_wide.x - wedge_edge(on_wide.y)).abs() < 0.02,
            "{on_wide:?}"
        );
    }
}

#[test]
fn an_arm_toward_the_seam_still_gets_its_corner() {
    let found = junction_inside_a_taper();
    // северо-восточный угол — между поперечной и лучом к шву
    assert!(
        found
            .roads
            .iter()
            .any(|(_, outline)| outline[0].x > 0.0 && outline[0].y > -20.0),
        "{:?}",
        found.roads
    );
}

#[test]
fn the_sidewalk_turns_the_corner_on_the_kerb_arc() {
    let found = walked_returns_of(&crossing(), true);
    assert_eq!(found.sidewalks.len(), 4);
    // центр бордюрной дуги в северо-восточном углу: угол краёв (4, 4) плюс
    // биссектриса на r/sin 45°
    let centre = Vec2::splat(4.0 + STREET_RADIUS);
    for outline in &found.sidewalks {
        let corner = outline[0].signum() * Vec2::splat(4.0 + SIDEWALK - OVERLAP);
        assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
        // дуга — того же центра, радиусом на тротуар меньше бордюрной, так
        // что за бордюром идёт ровная полоса в ширину тротуара
        let quadrant = outline[0].signum() * centre;
        for point in &outline[2..outline.len() - 1] {
            let radius = point.distance(quadrant);
            assert!(
                (radius - (STREET_RADIUS - SIDEWALK)).abs() < 1e-2,
                "{radius}"
            );
        }
    }
    // и асфальтовый клин по-прежнему целиком на тротуаре: где не на полосе,
    // там на её скруглении
    for (_, outline) in &found.roads {
        let quadrant = outline[0].signum() * centre;
        for point in outline {
            let band = 4.0 + SIDEWALK + 1e-3;
            let on_band = point.x.abs() <= band || point.y.abs() <= band;
            assert!(
                on_band || point.distance(quadrant) >= STREET_RADIUS - SIDEWALK - 1e-3,
                "{point:?}"
            );
        }
    }
}

#[test]
fn a_kerb_radius_under_the_sidewalk_width_leaves_the_corner_square() {
    // жилая зона с тротуаром выходит в магистраль: радиус жилой зоны
    // 2.5 м меньше трёхметрового тротуара (16 м — `sidewalk_band` в
    // потолке), и внешний угол полосы на месте такой же прямой
    let avenue = street(
        vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
        16.0,
    );
    let side = RoadLine {
        highway: Highway::LivingStreet,
        ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 16.0)
    };
    let found = walked_returns_of(&[avenue, side], true);
    assert_eq!(found.roads.len(), 2);
    assert!(found.sidewalks.is_empty());
}

#[test]
fn a_drive_without_a_sidewalk_does_not_break_the_band() {
    // проезд 5 м выходит в улицу: асфальт скругляется с обеих сторон,
    // а полоса тротуара идёт мимо него насквозь — скруглять нечего
    let through = east_west();
    let drive = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 5.0);
    let found = walked_returns_of(&[through, drive], true);
    assert_eq!(found.roads.len(), 2);
    assert!(found.sidewalks.is_empty());
}

/// Улица из двух way с изломом в узле, где к ней примыкает проезд без
/// тротуара: торцы её полос прямые (узел улиц — перекрёсток), и наружный
/// угол между ними закрывает щель; без него через тротуар шла нить
/// (Орёл, витрина 04, север).
#[test]
fn a_band_butted_at_a_drive_gets_its_outer_corner() {
    let west = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
    let east = street(vec![Vec2::ZERO, Vec2::new(50.0, -1.5)], 8.0);
    let drive = street(vec![Vec2::new(0.0, -50.0), Vec2::ZERO], 5.0);
    let found = walked_returns_of(&[west, east, drive], true);
    assert!(found.butt(0)[1] && found.butt(1)[0], "торцы прямые");
    assert_eq!(found.outer[1], 1, "{:?}", found.sidewalks);
}

#[test]
fn a_drive_between_two_streets_leaves_their_sidewalk_corner_alone() {
    // тот же проезд, но улицы сходятся углом: тротуарный угол считается
    // между ними, поверх устья проезда — его асфальт ляжет сверху
    let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
    let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
    let drive = street(vec![Vec2::ZERO, Vec2::new(35.0, 35.0)], 5.0);
    let found = walked_returns_of(&[east, north, drive], true);
    // скругление внутри угла и наружный угол по другую сторону узла
    assert_eq!(found.sidewalks.len(), 2);
    assert_eq!(found.outer[1], 1);
    let outline = &found.sidewalks[0];
    let corner = Vec2::splat(4.0 + SIDEWALK - OVERLAP);
    assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
}

#[test]
fn no_sidewalk_corner_on_the_side_of_the_paired_half() {
    // сквозная — половина разделённой улицы, вторая половина слева (к
    // северу): с той стороны тротуара нет, и углов по нему тоже
    let map = map_of(&crossing());
    let drawn = Drawn::for_test(&map)
        .with_pairs(0, vec![PairRun::for_test(0.0, 100.0, 1, true, 0.0, true)]);
    let found = kerb_returns(&drawn, 1.0);
    assert_eq!(found.roads.len(), 4, "асфальт скругляется, как был");
    assert_eq!(found.sidewalks.len(), 2);
    for outline in &found.sidewalks {
        assert!(outline[0].y < 0.0, "{:?}", outline[0]);
    }
}

fn with_highway(road: RoadLine, highway: Highway) -> RoadLine {
    RoadLine { highway, ..road }
}

#[test]
fn the_radius_follows_the_minor_class_of_the_pair() {
    let nearest_to_corner = |found: &[(RoadClass, Vec<Vec2>)]| right_angle_radius(&found[0].1);
    let [through, side] = crossing();
    let avenues = returns_of(&[
        with_highway(through.clone(), Highway::Primary),
        with_highway(side.clone(), Highway::Secondary),
    ]);
    assert!((nearest_to_corner(&avenues) - MAJOR_RADIUS).abs() < 0.1);
    let street_into_avenue = returns_of(&[
        with_highway(through.clone(), Highway::Primary),
        with_highway(side.clone(), Highway::Residential),
    ]);
    assert!((nearest_to_corner(&street_into_avenue) - STREET_RADIUS).abs() < 0.1);
    let drive = returns_of(&[
        with_highway(through, Highway::Primary),
        with_highway(side, Highway::Service),
    ]);
    assert!((nearest_to_corner(&drive) - DRIVE_RADIUS).abs() < 0.1);
}

#[test]
fn an_arm_ending_in_a_junction_ends_square_and_a_seam_stays_round() {
    // Т-образный узел: торец примыкающей улицы в узле — прямой, у
    // сквозной там не торец
    let through = east_west();
    let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
    let found = walked_returns_of(&[through, side], false);
    assert_eq!(found.butt, vec![[false; 2], [false, true]]);
    // шов двух ways одной улицы — не узел, торцы круглые
    let first = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
    let second = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
    let found = walked_returns_of(&[first, second], false);
    assert_eq!(found.butt, vec![[false; 2]; 2]);
    assert!(found.roads.is_empty());
}

#[test]
fn two_streets_meeting_at_a_corner_get_a_rounded_outer_side() {
    // угол двух улиц без сквозной: скругление внутри, веер снаружи — тот
    // же, что прежде давали круглые торцы
    let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
    let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
    let found = walked_returns_of(&[east, north], false);
    assert_eq!(found.roads.len(), 2);
    assert_eq!(found.outer[0], 1);
    assert_eq!(found.butt, vec![[true, false]; 2]);
    let outer = &found.roads[1].1;
    // веер — в третьем квадранте, радиус полуширины
    for point in &outer[2..outer.len() - 1] {
        assert!(point.x < 0.0 && point.y < 0.0, "{point:?}");
        assert!((point.length() - 4.0).abs() < 1e-3, "{point:?}");
    }
    assert!(point_in_polygon(Vec2::new(-2.0, -2.0), outer));
}

#[test]
fn a_small_triangle_of_streets_is_paved_and_a_large_one_is_not() {
    // развилка с островом (Тула, витрина 06): три улицы по 7.6 м между
    // тремя узлами; вписанный радиус 5.2 м — острова за полотнами почти нет
    let triangle = |scale: f32| {
        let [a, b, c] =
            [Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(10.0, 12.0)].map(|point| point * scale);
        [(a, b), (b, c), (c, a)].map(|(from, to)| street(vec![from, to], 7.6))
    };
    let islands = |roads: &[RoadLine]| small_islands(&Drawn::for_test(&map_of(roads)));
    let small = islands(&triangle(1.0));
    assert_eq!(small.len(), 1);
    assert_eq!(small[0].len(), 3, "контур — три узла: {:?}", small[0]);
    assert!(point_in_polygon(Vec2::new(13.0, 4.0), &small[0]));
    assert!(
        islands(&triangle(2.0)).is_empty(),
        "большой остров — настоящий"
    );
}

#[test]
fn a_sharp_fork_gets_a_small_nose_instead_of_a_spike() {
    // ветка уходит от сквозной улицы под 15°: кромки сходятся в 30.6 м от
    // узла по биссектрисе, и нос радиуса NOSE_RADIUS закрывает остриё до
    // дуги в 40.6 м (Тула, витрины 04, 06, 14)
    let angle = 15_f32.to_radians();
    let branch = street(
        vec![Vec2::ZERO, Vec2::from_angle(angle).rotate(Vec2::X) * 80.0],
        8.0,
    );
    let through = street(
        vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
        8.0,
    );
    let found = walked_returns_of(&[through, branch], true);
    let bisector = Vec2::from_angle(angle / 2.0).rotate(Vec2::X);
    let half_angle = angle / 2.0;
    let apex = |half: f32| half / half_angle.sin() + NOSE_RADIUS / half_angle.sin() - NOSE_RADIUS;
    let covered = |fill: Fill, at: f32| {
        found
            .noses
            .iter()
            .any(|(layer, outline)| *layer == fill && point_in_polygon(bisector * at, outline))
    };
    let road = Fill::Road(RoadClass::Street);
    // шаг поиска 0.5 м — центр носа может уйти на четверть метра
    assert!(covered(road, apex(4.0) - 1.0), "остриё до дуги — асфальт");
    assert!(!covered(road, apex(4.0) + 0.5), "за дугой асфальта нет");
    // дуга носа — на радиусе от центра и не ближе к нему
    let (_, outline) = found.noses.iter().find(|(fill, _)| *fill == road).unwrap();
    let centre = bisector * (apex(4.0) + NOSE_RADIUS);
    let on_arc = outline
        .iter()
        .filter(|point| (point.distance(centre) - NOSE_RADIUS).abs() < 0.3)
        .count();
    assert!(on_arc >= 3, "{outline:?}");
    assert!(
        outline
            .iter()
            .all(|point| point.distance(centre) > NOSE_RADIUS - 0.3)
    );
    // тротуар — своим носом по краям полос, дальше асфальтового
    let walked = apex(4.0 + SIDEWALK);
    assert!(
        covered(Fill::Sidewalk, walked - 1.0),
        "остриё тротуара закрыто"
    );
    assert!(!covered(Fill::Sidewalk, walked + 0.5));
}

#[test]
fn a_sharp_street_fork_hatches_a_gore_ahead_of_its_nose() {
    // та же развилка под 15°: ленты по 8 м перекрываются до острия носа
    // в 30.6 м по биссектрисе. Клин начинается, где ось ветки вышла из
    // полотна сквозной (оси разошлись на полуширину, 4 м: ~15.5 м от
    // узла), и кончается у вершины дуги носа (Тула, витрина 14)
    let angle = 15_f32.to_radians();
    let branch = street(
        vec![Vec2::ZERO, Vec2::from_angle(angle).rotate(Vec2::X) * 80.0],
        8.0,
    );
    let through = street(
        vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
        8.0,
    );
    let found = walked_returns_of(&[through.clone(), branch.clone()], false);
    assert_eq!(found.fork_gores.len(), 1, "{:?}", found.fork_gores);
    let gore = &found.fork_gores[0];
    let bisector = Vec2::from_angle(angle / 2.0).rotate(Vec2::X);
    let half_angle = angle / 2.0;
    let apex = 4.0 / half_angle.sin() + NOSE_RADIUS / half_angle.sin() - NOSE_RADIUS;
    let start = 4.0 / angle.sin();
    let inside = |at: f32| point_in_polygon(bisector * at, gore);
    assert!(inside(start + 3.0), "клин начался у выхода оси ветки");
    assert!(inside(apex - 1.0), "и дошёл до носа");
    assert!(!inside(start - 3.0), "у самого узла клина нет");
    assert!(
        !inside(apex + 1.0),
        "за вершиной дуги — остров, не штриховка"
    );
    // клин лежит на асфальте лент, не на земле
    for point in gore {
        let under = [&through, &branch].iter().any(|road| {
            crate::map::footprint::distance_to_polyline(*point, &road.points) <= 4.0 + 0.05
        });
        assert!(under, "{point:?} вне лент");
    }
    // у развилки с проездом клина нет: штриховка — между полотнами улиц
    let drive = street(branch.points.clone(), 5.0);
    assert!(
        walked_returns_of(&[through, drive], false)
            .fork_gores
            .is_empty()
    );
}

#[test]
fn a_nose_and_its_gore_keep_to_a_kerb_narrowed_by_a_far_taper() {
    // Тула, устье съезда с Фрунзе (S7): широкая улица сужена клином к
    // дальнему узкому продолжению, острое плечо уходит рядом. Нос по
    // номинальной полуширине вставал за настоящее расхождение кромок, и
    // штрихуемый клин ложился на плитку обочины мимо сузившейся кромки
    let mut through = street(vec![Vec2::ZERO, Vec2::new(70.0, 0.0)], 14.0);
    through.highway = Highway::Secondary;
    through.lanes = Some(2);
    let mut next = street(vec![Vec2::new(70.0, 0.0), Vec2::new(130.0, 0.0)], 8.0);
    next.highway = Highway::Secondary;
    next.lanes = Some(1);
    let mut west = street(vec![Vec2::new(-100.0, 0.0), Vec2::ZERO], 14.0);
    west.highway = Highway::Secondary;
    west.lanes = Some(2);
    let mut branch = street(
        vec![
            Vec2::ZERO,
            Vec2::from_angle(15_f32.to_radians()).rotate(Vec2::X) * 120.0,
        ],
        8.0,
    );
    branch.highway = Highway::Tertiary;
    branch.lanes = Some(1);
    let roads = vec![west, through, next, branch];
    let map = MapData {
        network: crate::map::roads::network::RoadNetwork::new(&roads),
        roads: roads.clone(),
        ..default()
    };
    let found = kerb_returns(
        &Drawn::new(
            &map,
            &crate::map::roads::RoadStyle::default(),
            &crate::map::roads::shape::RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        ),
        1.0,
    );
    assert_eq!(found.fork_gores.len(), 1, "{:?}", found.fork_gores);
    // клин лежит на асфальте, каким его положила лента: в клине сечений
    // (14 → 8 на длине 0.45 пути, от x = 38.5) кромка сквозной уже
    // номинальной полуширины
    let laid_half = |x: f32| 7.0 - 3.0 * ((x - 38.5) / 31.5).clamp(0.0, 1.0);
    assert!(
        found.fork_gores[0].iter().any(|point| point.x > 38.5),
        "клин доходит до клина сечений: {:?}",
        found.fork_gores
    );
    for point in &found.fork_gores[0] {
        let on_branch =
            crate::map::footprint::distance_to_polyline(*point, &roads[3].points) <= 4.0 + 0.1;
        assert!(
            on_branch || point.y.abs() <= laid_half(point.x) + 0.1,
            "{point:?} вне сузившейся кромки"
        );
    }
}

#[test]
fn a_nose_follows_a_bent_branch() {
    // ветка уходит под 15° и через 20 м загибается до 30°: прямая от
    // узла ушла бы с её кромки, нос же лежит на кромках лент — ни одна его
    // вершина, кроме острия, не заходит под ленты глубже OVERLAP
    let first = Vec2::from_angle(15_f32.to_radians()).rotate(Vec2::X) * 20.0;
    let branch = street(
        vec![
            Vec2::ZERO,
            first,
            first + Vec2::from_angle(30_f32.to_radians()).rotate(Vec2::X) * 60.0,
        ],
        8.0,
    );
    let through = street(
        vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
        8.0,
    );
    let found = walked_returns_of(&[through.clone(), branch.clone()], false);
    assert_eq!(found.noses.len(), 1);
    let (_, outline) = &found.noses[0];
    for point in &outline[1..] {
        for road in [&through, &branch] {
            let inside = 4.0 - crate::map::footprint::distance_to_polyline(*point, &road.points);
            assert!(inside <= OVERLAP + 0.02, "{point:?} под лентой на {inside}");
        }
    }
}

#[test]
fn nearly_parallel_arms_get_no_nose() {
    // почти параллельные полотна — одна дорога из двух way
    let branch = street(
        vec![
            Vec2::ZERO,
            Vec2::from_angle(1_f32.to_radians()).rotate(Vec2::X) * 80.0,
        ],
        8.0,
    );
    assert!(
        walked_returns_of(&[east_west(), branch], false)
            .noses
            .is_empty()
    );
}

/// Кольцо R = 15 м хордами по 2.4 м и улица, входящая в него по радиусу:
/// прямой пробег луча кольца — одна хорда, короче касательной, и прямое
/// скругление не ложилось — угол въезда оставался ступенькой, а из-под
/// неё торчал торец полосы тротуара (Белгород, Чапаева у кольца, R26).
#[test]
fn a_ring_entry_gets_a_kerb_return_on_the_ring_kerb() {
    let ring_points: Vec<Vec2> = (0..=40)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 40.0) * 15.0)
        .collect();
    let ring = RoadLine {
        oneway: true,
        roundabout: true,
        ..street(ring_points, 8.0)
    };
    let entry = street(vec![Vec2::new(60.0, 0.0), Vec2::new(15.0, 0.0)], 8.0);
    for sidewalks in [false, true] {
        let found = walked_returns_of(&[ring.clone(), entry.clone()], sidewalks);
        // за углом кромок въезда (y = ±4) и кольца (r = 19) — асфальт
        // скругления, по обе стороны въезда
        for y in [4.3, -4.3] {
            let outside = Vec2::new(18.8, y);
            assert!(
                found
                    .roads
                    .iter()
                    .map(|(_, outline)| outline)
                    .chain(found.bends.iter().map(|(_, outline)| outline))
                    .any(|outline| point_in_polygon(outside, outline)),
                "угол въезда у {outside} открыт ({sidewalks})"
            );
        }
        // и дуга не заходит на полотна глубже нахлёста
        for (_, outline) in &found.bends {
            for point in outline {
                let from_ring = point.length() - 15.0;
                let from_entry = point.y.abs();
                assert!(
                    from_ring.abs() >= 4.0 - BEND_OVERLAP - 0.05
                        || from_entry >= 4.0 - BEND_OVERLAP - 0.05,
                    "{point} под обеими лентами"
                );
            }
        }
        if sidewalks {
            // тротуар поворачивает вместе с бордюром — той же гнутой дугой
            assert!(
                found
                    .bends
                    .iter()
                    .filter(|(fill, _)| *fill == Fill::Sidewalk)
                    .count()
                    >= 2,
                "{:?}",
                found.bends.iter().map(|(fill, _)| fill).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn a_corner_past_a_bent_arm_gets_a_curved_kerb_return() {
    // Сквозная улица поворачивает в узле примыкания, и прямой край её
    // плеча кончается дугой сквозного отрезка задолго до угла краёв
    // (Путейская × Демонстрации, Тула: край 3.2 м, угол краёв 6.3 м от
    // узла). Прямое скругление не ложится, гнутому класс-радиус не
    // помещается в карман — угол оставался открытой землёй у проезжей.
    let through = street(
        vec![
            Vec2::new(5.0, 9.2),
            Vec2::ZERO,
            Vec2::new(-6.2, -6.0),
            Vec2::new(-12.4, -12.0),
        ],
        7.6,
    );
    let side = street(vec![Vec2::ZERO, Vec2::new(40.0, -6.0)], 7.6);
    // ось со сглаживанием (`for_test` рисует без него): прямой отрезок
    // через узел короче угла краёв
    let map = map_of(&[through, side]);
    let drawn = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let found = kerb_returns(&drawn, 1.0);
    // южный угол — прямым скруглением, северо-восточный — гнутым: дуга
    // асфальта и поворот тротуара за ней
    assert_eq!(found.roads.len(), 1, "{:?}", found.roads);
    assert_eq!(
        found
            .bends
            .iter()
            .map(|(fill, _)| *fill)
            .collect::<Vec<_>>(),
        [Fill::Road(RoadClass::Street), Fill::Sidewalk],
        "{:?}",
        found.bends
    );
    // северо-восточный угол закрыт: у контура есть точки за кромками
    assert!(
        found
            .roads
            .iter()
            .map(|(_, outline)| outline)
            .chain(found.bends.iter().map(|(_, outline)| outline))
            .any(|outline| outline.iter().any(|point| point.y > 3.0 && point.x > 5.0)),
        "{:?} {:?}",
        found.roads,
        found.bends
    );
}

#[test]
fn a_slight_kink_between_two_ways_at_a_junction_is_closed() {
    // улица из двух ways, излом в узле 0.7° наружу от примыкания (Тула,
    // витрина 08): торцы обоих плеч прямые, и клин между ними со стороны
    // без примыкания закрывает наружный угол
    let kink = (-0.7_f32).to_radians();
    let west = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
    let east = street(
        vec![Vec2::ZERO, Vec2::from_angle(kink).rotate(Vec2::X) * 50.0],
        8.0,
    );
    let side = street(vec![Vec2::new(0.0, -50.0), Vec2::ZERO], 8.0);
    let found = walked_returns_of(&[west, east, side], false);
    assert_eq!(found.outer[0], 1, "{:?}", found.roads);
    // клин — у кромки над узлом, между торцами
    let wedge = Vec2::new(0.02, 3.9);
    assert!(
        found
            .roads
            .iter()
            .any(|(_, outline)| point_in_polygon(wedge, outline)),
        "клин открыт"
    );
}

/// Узел R13 (Тула, Одоевский путепровод): широкая 14.2 м уходит на 150°,
/// улица 7.6 м — на −12° (между ними 162°), третья 7.6 м — на юг под −54°.
/// Направление узкой на `kink` градусов от −12°.
fn obtuse_junction(kink: f32) -> [RoadLine; 3] {
    let ray = |degrees: f32, length: f32| Vec2::from_angle(degrees.to_radians()) * length;
    [
        street(vec![Vec2::ZERO, ray(150.0, 40.0)], 14.2),
        street(vec![Vec2::ZERO, ray(-12.0 + kink, 50.0)], 7.6),
        street(vec![Vec2::ZERO, ray(-54.0, 40.0)], 7.6),
    ]
}

#[test]
fn an_obtuse_junction_pair_gets_its_corner() {
    // кромка широкой, продлённая за узел, сходится с кромкой узкой в
    // десяти метрах по узкой — между прямым торцом широкой и кромкой узкой
    // клин земли, пока его не закроет клин широкой кромки
    let found = walked_returns_of(&obtuse_junction(0.0), false);
    let along = Vec2::from_angle((-12.0_f32).to_radians());
    let wedge = along * 6.0 + along.perp() * 4.3;
    assert!(
        found
            .roads
            .iter()
            .any(|(_, outline)| point_in_polygon(wedge, outline)),
        "клин у торца широкой открыт: {:?}",
        found.roads
    );
    // и за кромкой широкой, продлённой за узел, асфальта нет
    let wide = Vec2::from_angle(150.0_f32.to_radians());
    let beyond = -wide.perp() * 7.6 - wide * 3.0;
    assert!(
        !found
            .roads
            .iter()
            .any(|(_, outline)| point_in_polygon(beyond, outline))
    );
}

#[test]
fn a_street_split_at_a_junction_with_a_slight_kink_gets_no_obtuse_corner() {
    // 179° между широкой и узкой — продолжение, уступ там — дело клина
    // сечений, а не угла узла
    let found = walked_returns_of(&obtuse_junction(-17.0), false);
    let along = Vec2::from_angle((-29.0_f32).to_radians());
    let wedge = along * 6.0 + along.perp() * 4.3;
    assert!(
        !found
            .roads
            .iter()
            .any(|(_, outline)| point_in_polygon(wedge, outline))
    );
}

/// Кромка тротуара и газона за скруглением не ломается на стыках кусков
/// дуги: дуги бордюрных поворотов режутся в [`KERB_ARC_SUBDIVISION`] раз
/// мельче общего шага, иначе 6 хорд общего шага выходили на кромку
/// гранями с изломами в 12–17° (Тула, Фёдора Смирнова ×
/// Красноармейский, 3399 3835).
#[test]
fn kerb_return_arcs_turn_smoothly_at_the_lawn() {
    // юго-восточный угол того перекрёстка: Фёдора Смирнова с газонной
    // обочиной 8.3 м на север, Красноармейский со своей узкой на восток,
    // переулок на запад, проспект на юг
    let mut smirnova = street(
        vec![
            Vec2::new(-13.5, 104.1),
            Vec2::new(-1.8, 12.3),
            Vec2::new(-1.1, 6.5),
            Vec2::ZERO,
        ],
        10.9,
    );
    smirnova.highway = Highway::Secondary;
    smirnova.verges = [8.3, 0.0];

    let mut east = street(
        vec![
            Vec2::ZERO,
            Vec2::new(23.4, -3.4),
            Vec2::new(26.0, -3.8),
            Vec2::new(58.4, -8.5),
            Vec2::new(107.1, -15.7),
        ],
        10.9,
    );
    east.highway = Highway::Primary;
    east.oneway = true;

    let mut west = street(
        vec![Vec2::new(-102.1, 19.9), Vec2::new(-19.2, 4.0), Vec2::ZERO],
        10.9,
    );
    west.highway = Highway::Tertiary;

    let mut south = street(
        vec![Vec2::ZERO, Vec2::new(-1.4, -9.5), Vec2::new(-2.8, -21.0)],
        20.8,
    );
    south.highway = Highway::Primary;

    let map = map_of(&[smirnova, east, west, south]);
    let found = kerb_returns(
        &Drawn::new(
            &map,
            &RoadStyle::default(),
            &RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        ),
        1.0,
    );
    // у контуров тротуаров излом между соседними звеньями дуги — не
    // грань. Звено дуги короче прямого края полосы и длиннее
    // пятисантиметровых заходов под ленты; излом круче 45° — это угол
    // самой фигуры (кончик линзы, смычка), а не стык кусков, и не
    // меряется
    let mut joints = 0;
    let mut worst = 0.0_f32;
    for outline in &found.sidewalks {
        let edges: Vec<_> = outline
            .windows(2)
            .map(|pair| (pair[0].distance(pair[1]), pair[1] - pair[0]))
            .collect();
        for pair in edges.windows(2) {
            let (a, b) = (pair[0].0, pair[1].0);
            if !(0.3..=3.0).contains(&a) || !(0.3..=3.0).contains(&b) {
                continue;
            }
            let turn = pair[0]
                .1
                .perp_dot(pair[1].1)
                .atan2(pair[0].1.dot(pair[1].1));
            let turn = turn.abs().to_degrees();
            if turn <= 45.0 {
                joints += 1;
                worst = worst.max(turn);
            }
        }
    }
    assert!(
        joints >= 6,
        "дуги не нашлись: {joints} стыков коротких звеньев"
    );
    assert!(
        worst <= 10.0,
        "кромка ломается на стыках кусков дуги: {worst:.1}°"
    );
}

/// Контур-квадрат от `at` стороной `side` — свой для каждой группы,
/// чтобы представления различались срезами.
fn square(at: Vec2, side: f32) -> Vec<Vec2> {
    vec![
        at,
        at + Vec2::new(side, 0.0),
        at + Vec2::new(side, side),
        at + Vec2::new(0.0, side),
    ]
}

/// `KerbReturns` с одной фигурой в каждой группе, разложенной по углам.
fn every_group() -> KerbReturns {
    KerbReturns {
        roads: vec![
            (RoadClass::Street, square(Vec2::ZERO, 2.0)),
            (RoadClass::Alley, square(Vec2::new(10.0, 0.0), 2.0)),
        ],
        unpaved: vec![square(Vec2::new(20.0, 0.0), 2.0)],
        sidewalks: vec![square(Vec2::new(30.0, 0.0), 2.0)],
        overlaps: vec![square(Vec2::new(40.0, 0.0), 2.0)],
        verges: vec![square(Vec2::new(50.0, 0.0), 2.0)],
        verge_lawns: vec![square(Vec2::new(60.0, 0.0), 2.0)],
        noses: vec![
            (
                Fill::Road(RoadClass::Street),
                square(Vec2::new(70.0, 0.0), 2.0),
            ),
            (Fill::Unpaved, square(Vec2::new(80.0, 0.0), 2.0)),
        ],
        bends: vec![(Fill::Sidewalk, square(Vec2::new(90.0, 0.0), 2.0))],
        ..default()
    }
}

/// Семь свежих билдеров слоёв — как в `build_roads`.
fn seven_builders() -> [MeshBuilder; 7] {
    std::array::from_fn(|_| MeshBuilder::with_surface_coords())
}

/// Уложить `returns` по свежим билдерам — `[улицы, аллеи, грунт,
/// тротуары, обочины, луга, дворы]`.
fn laid(returns: &KerbReturns, meadows: &Meadows, scraps: &mut Scraps) -> [MeshBuilder; 7] {
    let mut builders = seven_builders();
    let [streets, alleys, unpaved, sidewalks, verges, lawns, yards] = &mut builders;
    returns.lay(
        KerbLayers {
            streets,
            alleys,
            unpaved,
            sidewalks,
            verges,
            lawns,
            yards,
        },
        meadows,
        scraps,
    );
    builders
}

#[test]
fn lay_routes_each_kerb_group_to_its_layer() {
    let mut scraps = Scraps::default();
    // Meadows пуст: газон любого угла — травой двора
    let [streets, alleys, unpaved, sidewalks, verges, lawns, yards] =
        laid(&every_group(), &Meadows::of(&[], &[]), &mut scraps);
    // плоские группы: улицы и аллеи по классу, грунт, тротуары с
    // наездом, обочины, газон двора; слой луга пуст — Meadows пуст
    assert!(streets.vertex_count() > 0, "скругление улицы не легло");
    assert!(alleys.vertex_count() > 0, "скругление аллеи не легло");
    assert!(unpaved.vertex_count() > 0, "скругление грунтовки не легло");
    assert!(sidewalks.vertex_count() > 0, "тротуар с наездом не легли");
    assert!(verges.vertex_count() > 0, "обочина не легла");
    assert!(yards.vertex_count() > 0, "газон двора не лёг");
    assert_eq!(lawns.vertex_count(), 0, "без лугов газон ушёл в луг");
    // угол газона — лоскутам, до всего мощения
    assert!(
        scraps.near_lawn(&square(Vec2::new(60.0, 0.0), 2.0), 0.0),
        "угол газона не ушёл в лоскуты"
    );
    // нос улицы и дуга тротуара легли в те же слои поверх плоских групп:
    // без них слои тоньше
    let mut bare = every_group();
    bare.noses.clear();
    bare.bends.clear();
    let mut no_scraps = Scraps::default();
    let [bare_streets, _, _, bare_sidewalks, ..] =
        laid(&bare, &Meadows::of(&[], &[]), &mut no_scraps);
    assert!(
        streets.vertex_count() > bare_streets.vertex_count(),
        "нос улицы не лёг в слой улиц"
    );
    assert!(
        sidewalks.vertex_count() > bare_sidewalks.vertex_count(),
        "дуга тротуара не легла в слой тротуаров"
    );
}

#[test]
fn a_meadow_lawn_takes_the_lawn_layer_over_the_yard() {
    let meadow = crate::map::osm::fixture::area(
        crate::map::osm::model::AreaKind::Park,
        square(Vec2::new(59.0, -1.0), 4.0),
    );
    let mut scraps = Scraps::default();
    let [.., lawns, yards] = laid(&every_group(), &Meadows::of(&[meadow], &[]), &mut scraps);
    assert!(lawns.vertex_count() > 0, "угол у луга не стал лугом");
    assert_eq!(yards.vertex_count(), 0, "у луга угол остался двором");
}

#[test]
fn paving_cuts_lawns_with_street_asphalt_but_not_tile_overlaps() {
    let returns = every_group();
    let flat: Vec<&[Vec2]> = returns.paving().flat().collect();
    assert_eq!(
        flat,
        vec![
            returns.roads[0].1.as_slice(),
            returns.sidewalks[0].as_slice(),
            returns.verges[0].as_slice(),
        ],
        "плоское мощение: улица, тротуар, обочина"
    );
    let curved: Vec<&[Vec2]> = returns.paving().curved().collect();
    assert_eq!(
        curved,
        vec![returns.noses[0].1.as_slice(), returns.bends[0].1.as_slice()],
        "из гнутых мощёны только улица и тротуар"
    );
    // наезд плитки — не мощение, аллея и грунт — тоже: их контуров нет
    let overlap = returns.overlaps[0].as_slice();
    let alley = returns.roads[1].1.as_slice();
    let dirt = returns.noses[1].1.as_slice();
    assert!(!flat.contains(&overlap) && !curved.contains(&overlap));
    assert!(!flat.contains(&alley) && !curved.contains(&alley));
    assert!(!flat.contains(&dirt) && !curved.contains(&dirt));
}

#[test]
fn streets_view_yields_only_street_class_asphalt() {
    let returns = every_group();
    let found: Vec<&[Vec2]> = returns.streets().outlines().collect();
    assert_eq!(
        found,
        vec![returns.roads[0].1.as_slice(), returns.noses[0].1.as_slice()],
        "асфальт улиц — скругление и нос, без аллеи, грунта и тротуара"
    );
    let sidewalk_bend = returns.bends[0].1.as_slice();
    assert!(!found.contains(&sidewalk_bend));
}
