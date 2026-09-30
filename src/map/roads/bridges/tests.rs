use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::{RoadLine, fixture};
use crate::map::roads::push_ribbon;
use crate::map::shadow_dir;

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
        reach: [reach; 2],
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

/// Склейка — по торцам: два мостика, которые всего лишь пересекаются
/// серединами (на Туле таких пар две), остаются двумя мостами, каждый со
/// своим пролётом. `footprint::ways_joined` склеил бы их в один.
#[test]
fn crossing_decks_are_two_bridges_not_one() {
    let map = bridge_map(vec![
        fixture::bridge(vec![on_x(0.0), on_x(40.0)], 3.0),
        fixture::bridge(vec![Vec2::new(20.0, -20.0), Vec2::new(20.0, 20.0)], 3.0),
    ]);
    let bridges = Bridges::new(&map);
    for deck in 0..2 {
        let span = bridges.span(deck).unwrap();
        assert_eq!(span.span, 40.0, "deck {deck} was glued to its neighbour");
        assert_eq!((span.from_start, span.from_end), (0.0, 0.0));
    }
}

/// Мост — цепочка ways, и считается мостом цепочка: на Туле (кеш v15) 91
/// мостовой way дают 86 мостов. Здесь — развилка из трёх, пара из двух и
/// одинокий мостик по сухой земле, который тени не отбрасывает.
#[test]
fn a_bridge_report_counts_chains_not_ways() {
    let fork = Vec2::new(100.0, 0.0);
    let map = bridge_map(vec![
        fixture::bridge(vec![Vec2::ZERO, fork], 12.0),
        fixture::bridge(vec![fork, Vec2::new(150.0, 0.0)], 12.0),
        fixture::bridge(vec![fork, Vec2::new(100.0, 40.0)], 12.0),
        fixture::bridge(vec![Vec2::new(0.0, 300.0), Vec2::new(60.0, 300.0)], 8.0),
        fixture::bridge(vec![Vec2::new(60.0, 300.0), Vec2::new(90.0, 300.0)], 8.0),
        fixture::street(vec![Vec2::new(0.0, 500.0), Vec2::new(60.0, 500.0)], 8.0),
        fixture::bridge(vec![Vec2::new(0.0, 600.0), Vec2::new(20.0, 600.0)], 3.0),
    ]);
    assert_eq!(
        Bridges::new(&map).count(),
        BridgeReport {
            ways: 6,
            bridges: 3,
            casting: 2,
        }
    );
    assert_eq!(
        Bridges::new(&MapData::default()).count(),
        BridgeReport::default()
    );
}

/// Не мост — не пролёт: улица и мост без двух точек в [`Bridges`] не входят.
#[test]
fn only_bridge_ways_get_a_span() {
    let map = bridge_map(vec![
        fixture::street(vec![on_x(0.0), on_x(40.0)], 8.0),
        fixture::bridge(vec![on_x(40.0)], 8.0),
        fixture::bridge(vec![on_x(40.0), on_x(80.0)], 8.0),
    ]);
    let bridges = Bridges::new(&map);
    assert!(bridges.span(0).is_none());
    assert!(bridges.span(1).is_none());
    assert_eq!(bridges.span(2).unwrap().span, 40.0);
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

/// Путепровод (R35, Орёл, 7245, 728): путь с `bridge=yes` над дорогой —
/// мост, как у улицы: плита шириной в подошву балласта, парапет по её краям
/// и тень. Раньше мостовых слоёв у пути не было вовсе, и он лежал на
/// асфальте, как на переезде.
#[test]
fn a_track_bridge_gets_a_deck_a_parapet_and_a_shadow() {
    let mut rail = fixture::rail(vec![Vec2::ZERO, Vec2::new(64.0, 0.0)], 5.0);
    rail.bridge = true;
    let mut bridges = Bridges::new(&MapData::default());
    bridges.push_track(&rail);
    let [shadows, casings, fills] = bridges.layers();

    for layer in [&shadows, &casings, &fills] {
        assert!(!layer.builder.is_empty(), "{}", layer.name);
    }
    let half = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1].abs())
            .fold(0.0_f32, f32::max)
    };
    let deck = deck_width(&rail);
    assert!((half(&fills.builder) - deck / 2.0).abs() < 1e-3);
    // парапет торчит из-под плиты с обеих сторон
    assert!(half(&casings.builder) > deck / 2.0 + 0.5);
}
