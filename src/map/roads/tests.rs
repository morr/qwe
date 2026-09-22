use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::model::RoadNode;
use crate::map::osm::{Highway, fixture};
use crate::map::shadow_dir;

fn road(points: Vec<Vec2>, width: f32, passage: bool) -> RoadLine {
    RoadLine {
        class: RoadClass::Alley,
        passage,
        ..fixture::street(points, width)
    }
}

/// Одинокий мост из одного way: оба торца свободны, пролёт — своя длина.
/// Разбор про склейку — у [`Bridges`], здесь она не при чём.
fn lone(points: &[Vec2]) -> BridgeSpan {
    BridgeSpan {
        span: polyline_length(points),
        from_start: 0.0,
        from_end: 0.0,
        casts: true,
    }
}

/// Готовая теневая лента одинокого моста.
fn band(points: &[Vec2], reach: f32) -> ShadowBand {
    let deck = lone(points);
    ShadowBand {
        path: bridge_shadow_path(points, &deck),
        reach,
        penumbra: bridge_penumbra(deck.span),
    }
}

/// Точка на оси x — нарезанные мосты тестов лежат вдоль неё.
fn on_x(x: f32) -> Vec2 {
    Vec2::new(x, 0.0)
}

/// Карта из одних мостовых ways — вход [`Bridges`].
fn bridge_map(decks: Vec<RoadLine>) -> MapData {
    MapData {
        roads: decks,
        ..default()
    }
}

#[test]
fn passage_roads_are_not_smoothed() {
    // концы арки приколоты к вершинам контура здания — сглаживать её нельзя
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let nodes = RoadNodes::new(&[]);
    let arch = road(points.clone(), 5.0, true);
    assert_eq!(
        centerline(&arch, Smoothing::Strong, &nodes).as_ref(),
        points.as_slice()
    );
    let ordinary = road(points, 5.0, false);
    assert!(centerline(&ordinary, Smoothing::Strong, &nodes).len() > 3);
}

#[test]
fn a_shared_node_survives_smoothing() {
    // на изломе сквозной улицы кончается поперечная: хорда Chaikin сдвинула
    // бы узел, и торец поперечной повис бы мимо асфальта
    let corner = Vec2::new(20.0, 0.0);
    let through = road(vec![Vec2::ZERO, corner, Vec2::new(40.0, 20.0)], 8.0, false);
    let side = road(vec![Vec2::new(20.0, -30.0), corner], 8.0, false);
    let roads = [through, side];
    let nodes = RoadNodes::new(&roads);
    let drawn = centerline(&roads[0], Smoothing::Strong, &nodes);
    assert!(drawn.contains(&corner), "{drawn:?}");
}

#[test]
fn smoothing_off_borrows_the_osm_centerline() {
    let ordinary = road(vec![Vec2::ZERO, Vec2::new(20.0, 0.0)], 5.0, false);
    assert!(matches!(
        centerline(&ordinary, Smoothing::Off, &RoadNodes::new(&[])),
        Cow::Borrowed(_)
    ));
}

#[test]
fn casing_is_wider_than_the_fill() {
    // диапазоны самих ширин — тесты `map::footprint`; здесь — что кант
    // геометрически торчит из-под заливки
    let points = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let extent = |width: f32| {
        let mut builder = MeshBuilder::default();
        push_ribbon(
            &mut builder,
            &points,
            width,
            LinearRgba::WHITE,
            RoadJoin::Round,
        );
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let fill = 3.5;
    assert!(extent(fill + 2.0 * casing_width(fill)) > extent(fill));
}

#[test]
fn bridge_curb_ends_are_square_under_every_join() {
    let points = [Vec2::ZERO, Vec2::new(20.0, 0.0)];
    let max_x = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[0])
            .fold(f32::NEG_INFINITY, f32::max)
    };

    // ровный срез: бордюр кончается ровно на конце осевой при любом стиле стыка
    for join in RoadJoin::ALL {
        let mut curb = MeshBuilder::default();
        push_bridge_curb(&mut curb, &points, 5.0, join);
        assert!(!curb.is_empty());
        assert!(max_x(&curb) <= 20.0 + 1e-4, "curb pokes past the deck end");
    }

    // а заливка со стилем Round — полудиск за концом, для контраста
    let mut fill = MeshBuilder::default();
    push_ribbon(&mut fill, &points, 5.0, LinearRgba::WHITE, RoadJoin::Round);
    assert!(max_x(&fill) > 20.0);
}

/// Подъём настила живёт в вершинах осевой, а прямой мост в OSM — это ровно
/// две точки, и обе торцы (42 из 61 моста Тулы, включая мост через Упу). Пока
/// путь тени не догущался, `rise` в обеих был нулём, тень ложилась точь-в-точь
/// под настил и пропадала целиком: мостики через пруд не отбрасывали тени
/// вовсе.
#[test]
fn a_straight_two_point_bridge_still_casts_a_shadow() {
    let deck = [Vec2::ZERO, Vec2::new(60.0, 0.0)];
    let shadow = bridge_shadow_path(&deck, &lone(&deck));

    // у береговой опоры настил лежит на земле — торцы теневого пути на месте
    assert_eq!(shadow[0].rise, 0.0);
    assert!(shadow[0].at.distance(deck[0]) < 1e-3);
    let end = &shadow[shadow.len() - 1];
    assert_eq!(end.rise, 0.0);
    assert!(end.at.distance(deck[1]) < 1e-3);

    // а середина поднялась на полную высоту и отъехала по свету: шесть метров
    // при любом разумном солнце дают больше метра тени (порог, а не точное
    // число, — высота солнца это глобаль, которую крутят соседние тесты)
    let middle = &shadow[shadow.len() / 2];
    assert_eq!(middle.rise, 1.0);
    let drift = middle.at.distance(deck[0].midpoint(deck[1]));
    assert!(
        drift > 1.0,
        "the deck shadow stayed under the deck ({drift} m)"
    );
}

/// Мостик, идущий ровно по азимуту солнца: поперечной части у сдвига нет, и
/// тень-силуэт целиком прячется под настилом. Видимой её делает кайма — и
/// кайма же обязана сойти к нулю у торцов, где настил лежит на земле.
#[test]
fn a_bridge_along_the_sun_is_still_outlined() {
    let along = shadow_dir() * 60.0;
    let deck = [Vec2::ZERO, along];
    let reach = 2.55;
    let mut builder = MeshBuilder::default();
    push_bridge_shadows(&mut builder, &[band(&deck, reach)]);

    // ширину меряем поперёк моста — по проекции на нормаль его направления
    let across = along.normalize().perp();
    let (mut inside, mut ends) = (0.0_f32, 0.0_f32);
    for position in builder.positions_for_test() {
        let point = Vec2::new(position[0], position[1]);
        let side = across.dot(point).abs();
        // торцы — первые и последние два метра ленты
        let at = along.normalize().dot(point);
        if at < 2.0 || at > along.length() - 2.0 {
            ends = ends.max(side);
        } else {
            inside = inside.max(side);
        }
    }
    assert!(
        inside > reach + 0.5 * SHADOW_SPREAD,
        "the shadow band is no wider than the deck ({inside} m)"
    );
    assert!(
        ends < reach + 0.5,
        "the band still flares where the deck sits on the ground ({ends} m)"
    );
}

/// Тень кончается там же, где кончается настил: за створом торца её быть не
/// должно. Бордюр режется `RibbonCap::Butt` ровно по последней точке, а лента
/// у торца лежит точь-в-точь под ним — значит ни одна вершина слоя не имеет
/// права уехать за торец. Язычок тени, торчащий из-под конца бортика, —
/// репорт с карты.
#[test]
fn the_shadow_never_runs_past_the_abutment() {
    let deck = [Vec2::ZERO, Vec2::new(22.107, 0.0)];
    let reach = 3.3;
    let mut builder = MeshBuilder::default();
    push_bridge_shadows(&mut builder, &[band(&deck, reach)]);

    let (mut behind, mut ahead) = (0.0_f32, 0.0_f32);
    for position in builder.positions_for_test() {
        behind = behind.max(-position[0]);
        ahead = ahead.max(position[0] - deck[1].x);
    }
    assert!(
        behind < 1e-3,
        "the band runs {behind} m past the near abutment"
    );
    assert!(
        ahead < 1e-3,
        "the band runs {ahead} m past the far abutment"
    );
}

/// Край тени у всех, кто её отбрасывает, мягкий — у домов, машин и оград, — а
/// у настила был жёстким. И кайма ему нужна своя: мост из них самый высокий,
/// а правило карты (`cars/body.rs::SHADOW_BLUR`) — «кайма тем шире, чем
/// длиннее сама тень».
#[test]
fn the_shadow_edge_fades_and_dies_at_the_abutment() {
    let deck = [Vec2::ZERO, Vec2::new(0.0, 80.0)];
    let reach = 2.55;
    let shadow = band(&deck, reach);
    let mut builder = MeshBuilder::default();
    push_bridge_shadows(&mut builder, std::slice::from_ref(&shadow));

    let penumbra = shadow.penumbra;
    // ширину меряем от самой ленты: она вся сдвинута по свету вбок, и ось
    // моста ей уже не центр
    let centers: Vec<Vec2> = shadow.path.iter().map(|point| point.at).collect();
    let tips = [centers[0], centers[centers.len() - 1]];
    let (mut faded, mut ends) = (0.0_f32, 0.0_f32);
    for (position, color) in builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
    {
        let point = Vec2::new(position[0], position[1]);
        let side = distance_to_path(point, &centers);
        if tips.iter().any(|tip| point.distance(*tip) < 2.0) {
            ends = ends.max(side);
        } else if color[3] == 0.0 {
            // прозрачная вершина бывает только на внешнем крае каймы
            faded = faded.max(side);
        }
    }
    // кайма ушла за ядро ленты, и её внешний край прозрачен
    assert!(
        faded > reach + SHADOW_SPREAD,
        "the band has no faded outer edge ({faded} m)"
    );
    assert!(
        faded < reach + SHADOW_SPREAD + penumbra + 0.05,
        "the faded edge runs past the penumbra ({faded} m)"
    );
    // а у устоя, где настил лежит на земле, каймы нет вовсе: мягкий ореол
    // вокруг торца — это та самая контактная юбка, которую убирали у зданий
    assert!(
        ends < reach + 0.5,
        "the penumbra flares where the deck sits on the ground ({ends} m)"
    );
}

/// Ширина каймы — доля длины собственной тени, зажатая между каймой машины и
/// каймой дома: мостик через пруд и путепровод размыты по-разному.
#[test]
fn the_penumbra_follows_the_span() {
    let (short, long) = (bridge_penumbra(16.0), bridge_penumbra(40.0));
    assert!(
        short < long,
        "a 16 m footbridge is blurred like a 40 m one ({short} vs {long} m)"
    );
    // концы — константы соседних слоёв, и за них она не выходит
    assert_eq!(bridge_penumbra(1.0), PENUMBRA_MIN);
    assert_eq!(bridge_penumbra(600.0), PENUMBRA_MAX);
    assert!((PENUMBRA_MIN..=PENUMBRA_MAX).contains(&short));
}

/// Западный подход к мосту через Упу — это четыре way по 23–30 м с
/// `bridge=yes` и `layer=1`, а на месте под ними ровная земля: насыпь, а не
/// эстакада. По тегам их от пролёта не отличить, поэтому у короткого моста
/// спрашивают, есть ли под ним разрыв.
#[test]
fn a_short_bridge_needs_a_gap_under_it() {
    let across = |x: f32, half: f32| vec![Vec2::new(x - half, 0.0), Vec2::new(x + half, 0.0)];
    let map = MapData {
        water: vec![fixture::water_area(
            fixture::square(Vec2::ZERO, 20.0),
            vec![],
        )],
        rails: vec![fixture::rail(
            vec![Vec2::new(200.0, -20.0), Vec2::new(200.0, 20.0)],
            5.0,
        )],
        roads: vec![
            fixture::bridge(across(0.0, 10.0), 8.0),
            fixture::bridge(across(200.0, 10.0), 8.0),
            fixture::bridge(across(500.0, 10.0), 8.0),
            fixture::bridge(across(800.0, 30.0), 8.0),
        ],
        ..default()
    };
    let bridges = Bridges::new(&map);
    let casts = |deck: usize| bridges.span(deck).unwrap().casts;

    // мостик через пруд короток, но под ним вода
    assert!(casts(0));
    // переход над путями — тоже разрыв
    assert!(casts(1));
    // тот же пролёт по сухой земле — насыпь, тени нет
    assert!(!casts(2));
    // а длинный не спрашивают вовсе: на шестидесяти метрах насыпи не бывает
    assert!(casts(3));
}

/// Мост в OSM нарезан: переход через Упу — три way (424 + 95 + 299 м). Рампа
/// обязана отработать только на **внешних** торцах цепочки, иначе тень дважды
/// проваливается под настил посреди восьмисотметрового моста.
#[test]
fn a_glued_bridge_ramps_only_at_its_outer_ends() {
    let map = bridge_map(vec![
        fixture::bridge(vec![on_x(0.0), on_x(60.0)], 12.0),
        fixture::bridge(vec![on_x(60.0), on_x(90.0)], 12.0),
        fixture::bridge(vec![on_x(90.0), on_x(150.0)], 12.0),
    ]);
    let bridges = Bridges::new(&map);

    // середина цепочки не знает торцов вовсе: от её концов до свободного — 60 м
    let middle = *bridges.span(1).unwrap();
    assert_eq!((middle.from_start, middle.from_end), (60.0, 60.0));
    let raised = bridge_shadow_path(&map.roads[1].points, &middle);
    assert!(
        raised.iter().all(|point| point.rise == 1.0),
        "the deck dipped to the ground at an internal joint"
    );

    // а у крайнего куска садится на землю только его внешний торец. От его
    // дальнего узла до земли 60 м — назад по нему же самому, а не 90 вперёд:
    // путь до свободного торца кратчайший, и вернуться по своему настилу
    // никто не запрещает. `min(behind, ahead)` берёт ту же величину с обеих
    // сторон, так что двойного счёта из этого не выходит
    let first = *bridges.span(0).unwrap();
    assert_eq!((first.from_start, first.from_end), (0.0, 60.0));
    let path = bridge_shadow_path(&map.roads[0].points, &first);
    assert_eq!(path[0].rise, 0.0);
    assert_eq!(path[path.len() - 1].rise, 1.0);
}

/// Высота — от пролёта, и пролёт у куска тот же, что у всего моста: иначе
/// 30-метровая середина 150-метрового моста поднялась бы на 3.75 м вместо
/// шести и дала бы вдвое более короткую тень, чем её же соседи.
#[test]
fn a_glued_bridge_takes_its_height_from_the_whole_span() {
    let piece = vec![on_x(60.0), on_x(90.0)];
    let map = bridge_map(vec![
        fixture::bridge(vec![on_x(0.0), on_x(60.0)], 12.0),
        fixture::bridge(piece.clone(), 12.0),
        fixture::bridge(vec![on_x(90.0), on_x(150.0)], 12.0),
    ]);
    let glued = *Bridges::new(&map).span(1).unwrap();
    assert_eq!(glued.span, 150.0);

    let middle_of = |deck: &BridgeSpan| {
        let path = bridge_shadow_path(&piece, deck);
        let point = &path[path.len() / 2];
        point.at.distance(piece[0].midpoint(piece[1]))
    };
    assert!(
        middle_of(&glued) > middle_of(&lone(&piece)) + 1.0,
        "the middle piece kept the height of its own 30 m"
    );
}

/// Тот же нарез бьёт и по [`SHORT_SPAN`]: 22-метровая середина 185-метрового
/// моста — не мостик, и спрашивать у неё про разрыв под настилом нельзя.
#[test]
fn a_short_piece_of_a_long_bridge_keeps_its_shadow() {
    let stub = vec![on_x(0.0), on_x(20.0)];
    // по сухой земле: разрыва под настилом нет ни у куска, ни у моста
    let glued = bridge_map(vec![
        fixture::bridge(stub.clone(), 3.5),
        fixture::bridge(vec![on_x(20.0), on_x(80.0)], 3.5),
    ]);
    assert!(Bridges::new(&glued).span(0).unwrap().casts);

    // он же сам по себе — насыпь, и тени у него нет
    let alone = bridge_map(vec![fixture::bridge(stub, 3.5)]);
    assert!(!Bridges::new(&alone).span(0).unwrap().casts);
}

/// В узле сходятся и три конца сразу — на Туле ровно один такой, съезд
/// развязки у моста через Упу (424 + 95 + 299 м в одной точке). Геометрия
/// поэтому и не склеивается: склеивается счёт, и развилке он ничего не стоит —
/// узел не свободный торец, настил на нём поднят, а пролёт у всех трёх веток
/// общий.
#[test]
fn a_fork_is_one_bridge_and_stays_up() {
    let fork = Vec2::new(100.0, 0.0);
    let map = bridge_map(vec![
        fixture::bridge(vec![Vec2::ZERO, fork], 12.0),
        fixture::bridge(vec![fork, Vec2::new(150.0, 0.0)], 12.0),
        fixture::bridge(vec![fork, Vec2::new(100.0, 40.0)], 12.0),
    ]);
    let bridges = Bridges::new(&map);

    for deck in 0..3 {
        assert_eq!(bridges.span(deck).unwrap().span, 190.0);
    }
    // до свободного торца от развилки — по самой короткой ветке (40 м),
    // и это много больше рампы: настил на развилке стоит на полной высоте
    let branch = *bridges.span(0).unwrap();
    assert_eq!((branch.from_start, branch.from_end), (0.0, 40.0));
    let path = bridge_shadow_path(&map.roads[0].points, &branch);
    assert_eq!(path[path.len() - 1].rise, 1.0);
}

#[test]
fn sidewalks_belong_to_streets_not_service_roads() {
    // проезд — без тротуара, как бы широк он ни был: решает класс, не ширина;
    // жилая улица и магистраль — с ним, в пределах диапазона
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut service = fixture::street(line.clone(), 8.0);
    service.highway = Highway::Service;
    assert_eq!(sidewalk_width(&service), None);
    let residential = sidewalk_width(&fixture::street(line.clone(), 8.0)).unwrap();
    let mut primary = fixture::street(line, 16.0);
    primary.highway = Highway::Primary;
    let primary = sidewalk_width(&primary).unwrap();
    assert!(residential < primary);
    assert!(SIDEWALK_WIDTH_RANGE.contains(&residential));
    assert!(SIDEWALK_WIDTH_RANGE.contains(&primary));
}

#[test]
fn lanes_come_from_the_tag_and_fall_back_to_the_width() {
    let mut street = fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0);
    assert_eq!(
        lane_count(&street),
        2,
        "жилая улица без тега — по полосе в каждую сторону"
    );
    street.width = 12.0;
    assert_eq!(lane_count(&street), 4);
    street.lanes = Some(3);
    assert_eq!(lane_count(&street), 3, "тег важнее ширины");
    street.lanes = Some(8);
    assert_eq!(lane_count(&street), 4, "полоса у́же 2.5 м не бывает");
    street.lanes = None;
    street.oneway = true;
    street.width = 8.0;
    assert_eq!(lane_count(&street), 1, "односторонняя жилая — одна полоса");
    street.width = 16.0;
    assert_eq!(lane_count(&street), 3);
    street.roundabout = true;
    street.lanes = Some(2);
    assert_eq!(lane_count(&street), 2, "кольцо — как любая улица");
}

/// Кольцо узнаётся и **без тега** — по форме: замкнутое одностороннее полотно.
/// Большое кольцо у ТРЦ «Макси» (way 397005605) в OSM просто `oneway=yes`,
/// замкнутый сам на себя, и по одному тегу ни гладкой фигуры, ни островков
/// на подходах не получало бы.
#[test]
fn a_closed_oneway_way_is_a_roundabout_without_the_tag() {
    let corner = Vec2::new(-10.0, -10.0);
    let ring = vec![
        corner,
        Vec2::new(10.0, -10.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(-10.0, 10.0),
        corner,
    ];
    let mut road = fixture::street(ring.clone(), 12.0);
    road.oneway = true;
    assert!(road.is_roundabout());

    road.oneway = false;
    assert!(
        !road.is_roundabout(),
        "двусторонняя петля — не кольцевая развязка"
    );
    assert_eq!(lane_count(&road), 4);

    let mut open = fixture::street(ring[..4].to_vec(), 12.0);
    open.oneway = true;
    assert!(
        !open.is_roundabout(),
        "дуга кольца без тега — обычная улица"
    );
}

#[test]
fn lanes_come_with_the_carriageway() {
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut street = fixture::street(line.clone(), 8.0);
    assert_eq!(road_lanes(&street), Some(paint::lane_frame(2)));
    street.oneway = true;
    assert_eq!(
        road_lanes(&street),
        Some(paint::lane_frame(1)),
        "одна полоса: линий нет, а колея есть"
    );
    assert_eq!(road_lanes(&fixture::passage(line.clone(), 8.0)), None);
    let mut drive = fixture::street(line.clone(), 8.0);
    drive.highway = Highway::Service;
    assert_eq!(road_lanes(&drive), None, "у проезда полос нет");
    assert!(
        road_lanes(&fixture::bridge(line, 8.0)).is_some(),
        "мост несёт полосы своей улицы"
    );
}

#[test]
fn road_style_defaults_draw_sidewalks_and_markings() {
    let style = RoadStyle::default();
    assert!(style.sidewalks);
    assert!(style.markings);
}

/// Мост с тротуаром — два параллельных way, и ядра их теней перекрываются:
/// каждое на [`SHADOW_SPREAD`] шире своего настила. Ядра объединены, но кайма
/// кладётся от рельсов своей ленты — и рельс одного моста лежит внутри ядра
/// другого. Кайма оттуда легла бы поверх уже закрашенного союза полосой
/// двойной темноты с жёсткой линией по рельсу.
#[test]
fn a_bridge_penumbra_never_lies_over_a_neighbours_core() {
    let reach = 2.5;
    // три метра между осями: ядра по 3.5 м в полуширину накрывают друг друга
    let decks = [
        [Vec2::ZERO, Vec2::new(80.0, 0.0)],
        [Vec2::new(0.0, 3.0), Vec2::new(80.0, 3.0)],
    ];
    let bands: Vec<ShadowBand> = decks.iter().map(|deck| band(deck, reach)).collect();
    let cores: Vec<Vec<Vec2>> = bands
        .iter()
        .map(|shadow| {
            let edges = shadow_edges(shadow);
            edges
                .iter()
                .map(|edge| edge.left)
                .chain(edges.iter().rev().map(|edge| edge.right))
                .collect()
        })
        .collect();
    // сцена честная: посреди моста рельс каждой ленты и правда в ядре соседа
    for (own, shadow) in bands.iter().enumerate() {
        let edges = shadow_edges(shadow);
        let middle = &edges[edges.len() / 2];
        assert!(
            [middle.left, middle.right]
                .iter()
                .any(|rail| point_in_polygon(*rail, &cores[1 - own])),
            "the cores of the two bridges do not overlap"
        );
    }

    let mut builder = MeshBuilder::default();
    push_bridge_shadows(&mut builder, &bands);

    let mut faded = 0;
    for (position, color) in builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
    {
        // прозрачная вершина бывает только на внешнем крае каймы
        if color[3] != 0.0 {
            continue;
        }
        faded += 1;
        let point = Vec2::new(position[0], position[1]);
        // у устоя кайма схлопнута на свой же рельс — граница своего ядра не
        // в счёт, в счёт только глубина
        let buried = cores.iter().any(|core| {
            let ring: Vec<Vec2> = core.iter().chain(core.first()).copied().collect();
            point_in_polygon(point, core) && distance_to_path(point, &ring) > 0.01
        });
        assert!(
            !buried,
            "a penumbra lip lies inside a shadow core at {point}"
        );
    }
    // наружные каймы пары остались: пропускается только погребённая
    assert!(faded > 0, "the pair lost its outer penumbra too");
}

fn fortress(outer: Vec<Vec2>) -> PolyArea {
    PolyArea {
        height: Some(12.0),
        ..fixture::area(AreaKind::Kremlin, outer)
    }
}

/// Лента кремлёвской стены не ложится поверх крепостных зданий: остаётся
/// только кусок в стороне от них, а огрызок между двумя зданиями пропадает.
#[test]
fn the_city_wall_ribbon_stays_off_fortress_buildings() {
    let wall = vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)];
    // стена-здание на первых ста метрах, башня на 104…114 — между ними 4 м
    let buildings = [
        fortress(fixture::rect(Vec2::new(-1.0, -2.0), Vec2::new(100.0, 2.0))),
        fortress(fixture::rect(Vec2::new(104.0, -5.0), Vec2::new(114.0, 5.0))),
    ];
    let runs = Fortresses::of(&buildings).bare_runs(&wall);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run = &runs[0];
    assert!(
        // точка на самой кромке башни ничья — кусок начинается не раньше неё
        run[0].x >= 114.0 && run.last().unwrap().x == 200.0,
        "{run:?}"
    );

    // без крепостных зданий лента целиком — единственный рисунок стены
    let alone = Fortresses::of(&[]).bare_runs(&wall);
    assert_eq!(alone, vec![wall.clone()]);
    // и короткая неразрезанная лента не пропадает
    let short = vec![Vec2::new(500.0, 0.0), Vec2::new(505.0, 0.0)];
    assert_eq!(Fortresses::of(&buildings).bare_runs(&short).len(), 1);
}

// --- слои целиком ------------------------------------------------------
//
// Тесты на `mesh_roads`. До шва девять слоёв, три вида материала и вся
// телеметрия области жили внутри `spawn_roads` — 275 строк, взять которые из
// теста было нечем: проверять можно было только хелперы под ними.

/// Двадцать дорожных слоёв снизу вверх, ровно в том порядке, в каком они
/// уходят в мир: двенадцать лент и восемь слоёв краски над своим асфальтом —
/// колея траекторий узла (маска, потом наложение) ниже линий, островки колец
/// над асфальтом стоянок.
const LAYERS: [&str; 20] = [
    "alley_casings",
    "alleys",
    "sidewalks",
    "road_medians",
    "road_casings",
    "roads",
    paint::PAINT_WEAR_MASK,
    paint::PAINT_WEAR,
    paint::PAINT_ZEBRAS,
    paint::PAINT_LANES,
    paint::PAINT_AXES,
    "lot_sidewalks",
    "lot_lines",
    paint::PAINT_ISLANDS,
    "bridge_shadows",
    "bridge_casings",
    "bridges",
    paint::BRIDGE_PAINT_LANES,
    paint::BRIDGE_PAINT_AXES,
    "walls",
];

fn one_street() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    map
}

fn layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a LayerMesh {
    layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("слой {name} описан на любом стиле"))
}

#[test]
fn a_street_builds_seventeen_layers_bottom_up() {
    let (layers, report) = mesh_roads(&one_street(), RoadStyle::default());

    let names: Vec<&str> = layers.iter().map(|layer| layer.name).collect();
    assert_eq!(names, LAYERS);
    for pair in layers.windows(2) {
        // линии полос и осевые лежат на одной высоте: их полосы не
        // перекрываются, а прячет их ступень зума, не порядок
        let paint_pair = [pair[0].name, pair[1].name]
            .iter()
            .all(|name| paint::PaintTag::of(name).is_some());
        assert!(
            pair[0].z < pair[1].z || paint_pair && pair[0].z == pair[1].z,
            "{} лежит не ниже {}",
            pair[0].name,
            pair[1].name
        );
    }
    assert!(report.vertices > 0);
}

#[test]
fn only_the_bridge_shadow_is_blended() {
    let (layers, _) = mesh_roads(&one_street(), RoadStyle::default());

    // фактурный материал — у всего, что асфальт, тротуар или дорожка;
    // блендинг — ровно у полупрозрачной тени настила
    for layer in &layers {
        let expected = match layer.name {
            "bridge_shadows" => MaterialSpec::Blend,
            "sidewalks" | "lot_sidewalks" => MaterialSpec::Surface(SurfaceKind::Sidewalk),
            "alleys" => MaterialSpec::Surface(SurfaceKind::Alley),
            "road_medians" => MaterialSpec::Surface(SurfaceKind::Grass),
            "roads" | "bridges" => MaterialSpec::Surface(SurfaceKind::Street),
            paint::PAINT_WEAR_MASK => MaterialSpec::Paint(paint::PaintPass::WearMask),
            paint::PAINT_WEAR => MaterialSpec::Paint(paint::PaintPass::Wear),
            name if paint::PaintTag::of(name).is_some() => {
                MaterialSpec::Paint(paint::PaintPass::Lines)
            }
            _ => MaterialSpec::Flat,
        };
        assert_eq!(layer.material, expected, "{}", layer.name);
    }
}

#[test]
fn the_casing_knob_fills_the_casing_layers() {
    let map = one_street();
    let casing_verts = |casing| {
        let style = RoadStyle {
            casing,
            ..RoadStyle::default()
        };
        let (layers, _) = mesh_roads(&map, style);
        layer(&layers, "road_casings").builder.vertex_count()
    };

    // кант — отдельный слой, и выключенный он пуст, а не отсутствует
    assert_eq!(casing_verts(false), 0);
    assert!(casing_verts(true) > 0);
}

#[test]
fn the_sidewalk_knob_fills_the_sidewalk_layer() {
    let map = one_street();
    let sidewalk_verts = |sidewalks| {
        let style = RoadStyle {
            sidewalks,
            ..RoadStyle::default()
        };
        let (layers, _) = mesh_roads(&map, style);
        layer(&layers, "sidewalks").builder.vertex_count()
    };

    assert_eq!(sidewalk_verts(false), 0);
    assert!(sidewalk_verts(true) > 0);
}

/// Улица с вершиной посередине и вторая, выходящая из неё **этой же** точкой:
/// у Overpass нет id нод, и перекрёсток восстанавливается по совпадению
/// координат, так что общая вершина обязана быть у обеих.
fn a_tee() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(350.0, 100.0),
            Vec2::new(600.0, 100.0),
        ],
        12.0,
    ));
    map.roads.push(fixture::street(
        vec![Vec2::new(350.0, 100.0), Vec2::new(350.0, 400.0)],
        10.0,
    ));
    map
}

fn junctions_with(map: &MapData, markings: bool) -> usize {
    let style = RoadStyle {
        markings,
        ..RoadStyle::default()
    };
    mesh_roads(map, style).1.junctions
}

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

#[test]
fn a_bridge_leaves_the_street_layers_for_the_deck_ones() {
    let mut map = MapData::default();
    map.roads.push(fixture::bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    let (layers, _) = mesh_roads(&map, RoadStyle::default());

    // настил уходит из уличных слоёв в мостовые целиком, и тротуара у него нет
    // никогда: полоса свисала бы с настила над водой
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(layer(&layers, "sidewalks").builder.is_empty());
    assert!(!layer(&layers, "bridges").builder.is_empty());
    // бордюр настила рисуется всегда, независимо от ручки канта
    assert!(!layer(&layers, "bridge_casings").builder.is_empty());
}

#[test]
fn an_empty_map_still_describes_every_layer() {
    let (layers, report) = mesh_roads(&MapData::default(), RoadStyle::default());

    assert_eq!(layers.len(), LAYERS.len());
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
    assert_eq!(report.vertices, 0);
}

/// Большая стоянка с дорогой сквозь неё: `through` — односторонняя, уходит за
/// оба края площадки; проезд ряда лежит внутри целиком.
fn ground_with_roads(lot_side: f32) -> MapData {
    let mut map = MapData::default();
    map.parking.push(fixture::area(
        AreaKind::Parking,
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0 + lot_side, 100.0),
            Vec2::new(100.0 + lot_side, 100.0 + lot_side),
            Vec2::new(100.0, 100.0 + lot_side),
        ],
    ));
    let middle = 100.0 + lot_side / 2.0;
    map.roads.push(RoadLine {
        oneway: true,
        ..fixture::street(
            vec![Vec2::new(0.0, middle), Vec2::new(200.0 + lot_side, middle)],
            5.0,
        )
    });
    map.roads.push(fixture::parking_aisle(vec![
        Vec2::new(middle, 110.0),
        Vec2::new(middle, 90.0 + lot_side),
    ]));
    map
}

fn extent_x(builder: &MeshBuilder) -> (f32, f32) {
    builder
        .positions_for_test()
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), at| {
            (low.min(at[0]), high.max(at[0]))
        })
}

#[test]
fn a_road_through_a_big_lot_is_drawn_over_it_with_a_kerb() {
    let (layers, _) = mesh_roads(&ground_with_roads(100.0), RoadStyle::default());

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
    let (layers, _) = mesh_roads(&ground_with_roads(60.0), RoadStyle::default());
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
    assert!(layer(&layers, "lot_lines").builder.is_empty());
}

/// Кольцо радиусом 12 м в начале координат и два односторонних подхода к нему,
/// сходящиеся в одну дорогу в полусотне метров, — въезд и съезд одной улицы.
///
/// `tagged` — несёт ли кольцо `junction=roundabout`, `oneway` — одностороннее ли
/// оно: кольцо без тега в OSM обычное дело.
fn roundabout_with_an_approach(tagged: bool, oneway: bool) -> MapData {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 12.0)
        .collect();
    let arm = |turn: f32, side: f32| RoadLine {
        oneway: true,
        ..fixture::street(
            vec![circle[(24.0 + turn) as usize % 24], Vec2::new(55.0, side)],
            5.0,
        )
    };
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway,
        roundabout: tagged,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads.push(arm(2.0, 1.5));
    map.roads.push(arm(-2.0, -1.5));
    map
}

/// Клин между въездом, съездом и кольцом — направляющий островок: асфальт со
/// штриховкой, как рисует Яндекс, — и стоянка для этого не нужна.
#[test]
fn the_wedge_between_the_two_arms_of_a_roundabout_is_hatched() {
    // с тегом — и **без него**: большое кольцо у ТРЦ «Макси» в OSM просто
    // замкнутое одностороннее полотно, и по одному тегу его островки не
    // находились вовсе
    for tagged in [true, false] {
        let map = roundabout_with_an_approach(tagged, true);
        let (layers, _) = mesh_roads(&map, RoadStyle::default());
        // клин — правее кольца, между подходами, у оси x
        let wedged = |at: &&[f32; 3]| (17.0..40.0).contains(&at[0]) && at[1].abs() < 2.5;
        let lines = layer(&layers, paint::PAINT_ISLANDS)
            .builder
            .positions_for_test();
        assert!(
            lines.iter().any(|at| wedged(&at)),
            "клин не заштрихован ({tagged})"
        );
        // и ничего не заштриховано с внешней стороны подходов
        assert!(lines.iter().all(|at| at[0] > 10.0 && at[1].abs() < 9.0));
    }
}

/// Треугольник тротуара между въездом, съездом и кольцом кроет **асфальт**
/// островка, а не его штриховка: он идёт в слой улиц (выше тротуаров) и выпущен
/// на `ASPHALT_PAD` под кромки полотен, чтобы между ним и лентой дороги не
/// оставалось волоска земли. Ни одной ручки `RoadStyle` у этой фигуры нет —
/// держит её только тест.
#[test]
fn the_gore_is_paved_under_the_edges_of_both_arms() {
    let map = roundabout_with_an_approach(true, true);
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.gores, 1, "островок у кольца один");

    // клин целиком: от кромки кольца (радиус 16 м) до острия, где полотна
    // сходятся (x ≈ 45, и асфальт выпущен ещё дальше); подходы — дороги 1 и 2
    // сцены. Своих вершин у лент подходов тут нет — они прямые, и вершины у них
    // только на торцах
    let arms = [
        map.roads[1].points.as_slice(),
        map.roads[2].points.as_slice(),
    ];
    let wedged = |at: &[f32; 3]| (14.0..50.0).contains(&at[0]) && at[1].abs() < 5.0;
    // кромка полотна — в 2.5 м от его оси, так что ближе неё лежит только то,
    // что зашло **под** ленту; полоса под обводку штриховки вылезает из клина
    // на `EDGE_STRIP` — место шейдеру, сама линия в ней тоньше
    let depth = |at: &[f32; 3]| {
        let at = Vec2::new(at[0], at[1]);
        arms.iter()
            .map(|arm| distance_to_path(at, arm))
            .fold(f32::INFINITY, f32::min)
    };
    let deepest = |name: &str| {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|at| wedged(at))
            .map(&depth)
            .fold(f32::INFINITY, f32::min)
    };
    const UNDER: f32 = 2.3;
    let (asphalt, hatching) = (deepest("roads"), deepest(paint::PAINT_ISLANDS));
    assert!(
        asphalt < UNDER,
        "асфальт островка не зашёл под кромки полотен: {asphalt}"
    );
    // ...а штриховка у́же его ровно на этот заход и на полотно не лезет
    let hatching = hatching + paint::EDGE_STRIP;
    assert!(
        hatching.is_finite() && hatching > UNDER,
        "штриховка островка вылезла на полотно: {hatching}"
    );
}

/// Двусторонний подход одним way — веера из въезда и съезда нет, и клин
/// [`gores::Gores::of`] не находит. Островок ставится по правилу: капля на оси
/// подхода от кромки кольца, подход вокруг неё раздвинут асфальтом.
#[test]
fn a_two_way_approach_gets_a_splitter_island() {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 25.0)
        .collect();
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway: true,
        roundabout: true,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads
        .push(fixture::street(vec![circle[0], Vec2::new(90.0, 0.0)], 7.6));
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.rings[0], 1);
    assert_eq!(report.gores, 1, "островок на подходе один");
    // капля — на оси подхода за кромкой кольца (25 + 4 м), не дальше острия
    let island: Vec<&[f32; 3]> = layer(&layers, paint::PAINT_ISLANDS)
        .builder
        .positions_for_test()
        .iter()
        .collect();
    assert!(!island.is_empty());
    assert!(
        island
            .iter()
            .all(|at| (28.0..50.0).contains(&at[0]) && at[1].abs() < 3.0),
        "островок не на подходе"
    );
    // и подход раздвинут: асфальт шире полотна по сторонам островка
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (30.0..35.0).contains(&at[0]) && at[1].abs() > 7.6 / 2.0 + 0.5),
        "подход у островка не расширен"
    );
}

#[test]
fn a_two_way_loop_without_the_tag_is_not_a_roundabout() {
    let map = roundabout_with_an_approach(false, false);
    let (layers, _) = mesh_roads(&map, RoadStyle::default());
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

#[test]
fn the_markings_knob_takes_the_hatching_off() {
    let style = RoadStyle {
        markings: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(&roundabout_with_an_approach(true, true), style);
    assert!(layer(&layers, paint::PAINT_ISLANDS).builder.is_empty());
}

/// Два встречных полотна бок о бок — бульвар: между ними не бордюр, а двойная
/// сплошная. Пару находит сеть (`roads/network/pairs.rs`); бульвар у ТРЦ
/// «Макси» — два встречных проезда, как и здесь.
#[test]
fn two_carriageways_side_by_side_get_a_double_line_and_no_kerb_between() {
    let mut map = ground_with_roads(100.0);
    let middle = 150.0;
    map.roads.push(RoadLine {
        oneway: true,
        ..fixture::street(
            vec![Vec2::new(300.0, middle + 5.5), Vec2::new(0.0, middle + 5.5)],
            5.0,
        )
    });
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.medians, [1, 0]);

    let lines = &layer(&layers, "lot_lines").builder;
    assert!(!lines.is_empty());
    for at in lines.positions_for_test() {
        assert!(
            (at[1] - (middle + 2.75)).abs() < 0.5,
            "линия не по оси: {at:?}"
        );
        assert!(
            (100.0..=200.0).contains(&at[0]),
            "линия за площадкой: {at:?}"
        );
    }
    // между осями полотен бордюра нет, снаружи от них он есть
    let kerb = layer(&layers, "lot_sidewalks").builder.positions_for_test();
    assert!(kerb.iter().any(|at| at[1] < middle - 2.5));
    assert!(kerb.iter().any(|at| at[1] > middle + 8.0));
    for at in kerb {
        assert!(
            at[1] < middle + 0.1 || at[1] > middle + 5.4,
            "бордюр на разделительной: {at:?}"
        );
    }
}

#[test]
fn the_sidewalk_knob_takes_the_kerb_off_the_lot_road_too() {
    let style = RoadStyle {
        sidewalks: false,
        ..RoadStyle::default()
    };
    let (layers, _) = mesh_roads(&ground_with_roads(100.0), style);
    assert!(layer(&layers, "lot_sidewalks").builder.is_empty());
}

/// Разделённый проспект вдоль x: две встречные половины в три полосы с
/// `gap` метров между кромками. Возвращает карту и расстояние между осями.
fn divided_avenue(gap: f32) -> (MapData, f32) {
    let width = 3.0 * 3.3 + 1.0;
    let apart = width + gap;
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(3),
        ..fixture::street(points, width)
    };
    let mut map = MapData::default();
    map.roads
        .push(half(vec![Vec2::new(100.0, 100.0), Vec2::new(500.0, 100.0)]));
    map.roads.push(half(vec![
        Vec2::new(500.0, 100.0 + apart),
        Vec2::new(100.0, 100.0 + apart),
    ]));
    (map, apart)
}

#[test]
fn paired_halves_share_a_paved_median_and_keep_sidewalks_outside() {
    let (map, apart) = divided_avenue(0.6);
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.medians, [1, 0]);
    let middle = 100.0 + apart / 2.0;
    // двойная сплошная — по середине между половинами
    let axes = layer(&layers, paint::PAINT_AXES)
        .builder
        .positions_for_test();
    assert!(!axes.is_empty(), "двойная сплошная есть");
    assert!(
        axes.iter().all(|at| (at[1] - middle).abs() < 1.5),
        "осевые только по середине"
    );
    // под зазором — асфальт, а не тротуар
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    let half = (3.0 * 3.3 + 1.0) / 2.0;
    let (low, high) = (100.0 + half, 100.0 + apart - half);
    assert!(
        sidewalks
            .iter()
            .all(|at| at[1] <= low + 0.01 || at[1] >= high - 0.01),
        "тротуар со стороны пары"
    );
    assert!(
        sidewalks.iter().any(|at| at[1] < 100.0 - half - 1.0),
        "а с внешней стороны он есть"
    );
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(
        roads
            .iter()
            .any(|at| (at[1] - middle).abs() < 0.01 && at[0] > 150.0)
    );
    assert!(layer(&layers, "road_medians").builder.is_empty());
}

/// `sidewalk=right` — полоса только справа по ходу (к югу у улицы на восток),
/// `separate` с обеих сторон — никакой.
#[test]
fn the_sidewalk_tag_picks_the_side() {
    let sidewalks_with = |sides: [bool; 2]| {
        let mut map = one_street();
        map.roads[0].sidewalks = sides;
        let (layers, _) = mesh_roads(&map, RoadStyle::default());
        layer(&layers, "sidewalks")
            .builder
            .positions_for_test()
            .to_vec()
    };
    let half = 12.0 / 2.0;
    let right = sidewalks_with([false, true]);
    assert!(!right.is_empty());
    assert!(
        right.iter().all(|at| at[1] <= 100.0 + half + 0.01),
        "слева по ходу тротуара нет"
    );
    assert!(right.iter().any(|at| at[1] < 100.0 - half - 1.0));
    assert!(sidewalks_with([false, false]).is_empty());
}

/// Магистраль получает карманы с обеих сторон: асфальт за кромкой, тротуар
/// отодвинут за карман.
#[test]
fn a_primary_gets_pockets_in_its_sidewalks() {
    let mut map = one_street();
    map.roads[0].highway = Highway::Primary;
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.kerb_pockets, 2);
    let edge = 100.0 + 6.0 + pockets::POCKET_WIDTH;
    let roads = layer(&layers, "roads").builder.positions_for_test();
    assert!(roads.iter().any(|at| (at[1] - edge).abs() < 0.01));
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(sidewalks.iter().any(|at| at[1] > edge + 1.0));
}

/// `highway=turning_circle` на торце тупика — круг асфальта шире дороги.
#[test]
fn a_turning_circle_widens_the_dead_end() {
    let mut map = one_street();
    map.road_nodes.push(RoadNode {
        pos: Vec2::new(600.0, 100.0),
        kind: RoadNodeKind::TurningCircle,
    });
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
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

/// Двойная сплошная доходит до перекрёстка так же, как линии полос: пробы пары
/// теряют соседа за несколько метров до узла, и середина кончалась там (отчёт
/// автора, пример 2 витрины).
#[test]
fn the_double_line_reaches_the_junction_like_the_lane_lines() {
    let (mut map, apart) = divided_avenue(0.6);
    for road in &mut map.roads {
        road.points.insert(1, Vec2::new(300.0, road.points[0].y));
    }
    map.roads.push(fixture::street(
        vec![
            Vec2::new(300.0, 40.0),
            Vec2::new(300.0, 100.0),
            Vec2::new(300.0, 100.0 + apart),
            Vec2::new(300.0, 170.0),
        ],
        12.0,
    ));
    let (layers, _) = mesh_roads(&map, RoadStyle::default());
    let nearest_before = |name: &str| {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|at| at[0] < 300.0 && (at[1] - 100.0 - apart / 2.0).abs() < apart)
            .map(|at| at[0])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let (axis, lanes) = (
        nearest_before(paint::PAINT_AXES),
        nearest_before(paint::PAINT_LANES),
    );
    assert!(axis.is_finite() && lanes.is_finite());
    // полоса краски выходит за видимый конец линии — гасит её шейдер по
    // «до разрыва», — так что двойная обязана доходить не меньше линий полос
    assert!(
        axis > lanes - 0.5,
        "двойная кончается у x = {axis}, линии полос — у x = {lanes}"
    );
}

/// Улица, примыкающая только к ближней половине, разделительную не открывает:
/// двойная сплошная идёт мимо узла без разрыва (пример 12 витрины).
#[test]
fn a_street_into_one_half_does_not_open_the_median() {
    let (mut map, _) = divided_avenue(0.6);
    map.roads[0].points.insert(1, Vec2::new(300.0, 100.0));
    map.roads.push(fixture::street(
        vec![Vec2::new(300.0, 40.0), Vec2::new(300.0, 100.0)],
        7.6,
    ));
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert!(report.junctions > 0, "узел у ближней половины есть");
    let axes = &layer(&layers, paint::PAINT_AXES).builder;
    let positions = axes.positions_for_test();
    assert!(positions.iter().any(|at| at[0] < 200.0) && positions.iter().any(|at| at[0] > 400.0));
    // разрыв — в «до разрыва» полосы краски: внутри него оно отрицательно
    // осевая самой примыкающей улицы лежит ниже половин — её не считаем
    assert!(
        axes.ribbon_coords_for_test()
            .expect("у краски атрибут есть")
            .iter()
            .zip(positions)
            .filter(|(_, at)| at[1] > 100.0)
            .all(|(coords, _)| coords[2] > 0.0),
        "двойная сплошная рвётся у узла одной половины"
    );
}

#[test]
fn a_wide_gap_between_halves_is_a_lawn_with_a_kerb() {
    let (map, apart) = divided_avenue(8.0);
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert_eq!(report.medians, [0, 1]);
    let inner = (3.0 * 3.3 + 1.0) / 2.0;
    let grass = layer(&layers, "road_medians").builder.positions_for_test();
    assert!(!grass.is_empty(), "газон есть");
    for at in grass {
        assert!(
            at[1] >= 100.0 + inner + medians::MEDIAN_KERB - 0.05
                && at[1] <= 100.0 + apart - inner - medians::MEDIAN_KERB + 0.05,
            "газон заходит на бордюр или полотно: {at:?}"
        );
    }
    // осевой краски у газона нет
    assert!(layer(&layers, paint::PAINT_AXES).builder.is_empty());
}
