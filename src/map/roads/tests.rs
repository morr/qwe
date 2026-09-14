use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::fixture;

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
fn chaikin_keeps_endpoints() {
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, 3.5);
    assert_eq!(smoothed[0], original[0]);
    assert_eq!(smoothed[smoothed.len() - 1], original[original.len() - 1]);
}

#[test]
fn chaikin_deviation_is_bounded_by_width() {
    // длинные сегменты: без ограничения шириной срез ушёл бы на 5 м от угла
    let width = 3.5;
    let original = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let smoothed = chaikin(&original, width);
    // сами точки среза лежат на исходных сегментах
    for point in &smoothed {
        assert!(distance_to_path(*point, &original) < 1e-4);
    }
    // а хорда, заменившая угол, отходит от него не дальше ширины дороги
    let deviation = smoothed
        .windows(2)
        .map(|segment| distance_to_path(segment[0].midpoint(segment[1]), &original))
        .fold(0.0_f32, f32::max);
    assert!(deviation <= width, "chaikin drifted {deviation} m");
}

#[test]
fn chaikin_leaves_straight_runs_alone() {
    // изломы по 2° мельче MIN_SMOOTH_ANGLE — ломаная возвращается как есть
    let step = 10.0 * 2.0_f32.to_radians().tan();
    let original = vec![
        Vec2::ZERO,
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, step),
        Vec2::new(30.0, step * 2.0),
    ];
    assert_eq!(chaikin(&original, 3.5), original);
}

#[test]
fn passage_roads_are_not_smoothed() {
    // концы арки приколоты к вершинам контура здания — сглаживать её нельзя
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    let arch = road(points.clone(), 5.0, true);
    assert_eq!(
        centerline(&arch, RoadSmoothing::Strong).as_ref(),
        points.as_slice()
    );
    let ordinary = road(points, 5.0, false);
    assert!(centerline(&ordinary, RoadSmoothing::Strong).len() > 3);
}

#[test]
fn smoothing_off_borrows_the_osm_centerline() {
    let ordinary = road(vec![Vec2::ZERO, Vec2::new(20.0, 0.0)], 5.0, false);
    assert!(matches!(
        centerline(&ordinary, RoadSmoothing::Off),
        Cow::Borrowed(_)
    ));
}

#[test]
fn smooth_path_applies_where_nothing_is_pinned() {
    // `smooth_path` — общий вход для рельсов, трамвая и зелёной полосы: в
    // отличие от `centerline` ему нечего закреплять, и сглаживание он
    // применяет всегда
    let points = vec![Vec2::ZERO, Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)];
    assert!(smooth_path(&points, 5.0, RoadSmoothing::Strong).len() > 3);
    assert!(matches!(
        smooth_path(&points, 5.0, RoadSmoothing::Off),
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
    // проезд (`service`, 5 м) — без тротуара; жилая улица и магистраль — с ним,
    // в пределах диапазона
    assert_eq!(sidewalk_width(5.0), None);
    let residential = sidewalk_width(8.0).unwrap();
    let primary = sidewalk_width(16.0).unwrap();
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
    assert_eq!(lane_count(&street), 1, "на кольце линий нет");
}

#[test]
fn markings_need_a_carriageway_with_two_lanes() {
    let line = vec![Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut street = fixture::street(line.clone(), 8.0);
    assert_eq!(
        road_markings(&street),
        Some(Markings {
            lanes: 2,
            oneway: false
        })
    );
    street.oneway = true;
    assert_eq!(road_markings(&street), None, "одна полоса — делить нечего");
    street.width = 12.0;
    assert_eq!(
        road_markings(&street),
        Some(Markings {
            lanes: 2,
            oneway: true
        })
    );
    assert_eq!(road_markings(&fixture::street(line.clone(), 5.0)), None);
    assert_eq!(road_markings(&fixture::passage(line.clone(), 8.0)), None);
    assert!(
        road_markings(&fixture::bridge(line, 8.0)).is_some(),
        "мост несёт разметку своей улицы"
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
        outer,
        holes: Vec::new(),
        kind: AreaKind::Kremlin,
        building_use: crate::map::osm::BuildingUse::Other,
        height: Some(12.0),
        entrances: Vec::new(),
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
