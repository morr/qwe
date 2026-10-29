use super::super::fixtures::{layer, one_street};
use super::super::network::RoadNetwork;
use super::super::ribbons::{BAND_SEAM_OVERLAP, BAND_SEAM_TAPER};
use super::super::{
    Axis, Drawn, LayerMesh, MeshBuilder, RoadClass, RoadShape, RoadStyle, VergeZoomBucket,
    mesh_roads,
};
use super::*;
use crate::map::meshing::distance_to_path;
use crate::map::osm::model::Pavement;
use crate::map::osm::{Highway, MapData, RoadLine, fixture};
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
    let path = bridge_shadow_path(points, &deck);
    ShadowBand {
        reach: vec![[reach; 2]; path.len()],
        path,
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

/// Настил-половина над смешанной разделительной: первые 20 м она мощёная
/// (бортика справа нет), дальше газон (бортик есть). Тень справа — по кромку
/// асфальта только над мощёным куском; над газоном под бортиком она остаётся,
/// а не сужается по всей длине из-за одного куска без бортика (R30).
#[test]
fn a_half_deck_keeps_its_curb_shadow_over_the_lawn() {
    let line = fixture::bridge(vec![on_x(0.0), on_x(40.0)], 12.0);
    let pieces: [BandPiece; 2] = [(0.0, 20.0, [true, false]), (20.0, 40.0, [true, true])];
    let (curb, bare) = (line.curb_reach(), line.width / 2.0);
    let head = 3.0;
    let reach = |along: f32| deck_shadow_reach(&line, Some(&pieces), along - head);
    assert_eq!(reach(head + 10.0), [curb, bare], "over the paved median");
    assert_eq!(reach(head + 30.0), [curb, curb], "over the lawn");
    // клин перед телом берёт крайний кусок
    assert_eq!(reach(1.0), [curb, bare], "the head wedge");
    assert_eq!(reach(head + 45.0), [curb, curb], "the tail wedge");
    assert_eq!(deck_shadow_reach(&line, None, 5.0), [curb; 2]);
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

#[test]
fn a_bridge_leaves_the_street_layers_for_the_deck_ones() {
    let mut map = MapData::default();
    map.roads.push(fixture::bridge(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );

    // настил уходит из уличных слоёв в мостовые целиком, и тротуара у него нет
    // никогда: полоса свисала бы с настила над водой
    assert!(layer(&layers, "roads").builder.is_empty());
    assert!(layer(&layers, "sidewalks").builder.is_empty());
    assert!(!layer(&layers, "bridges").builder.is_empty());
    // бордюр настила рисуется всегда, независимо от ручки канта
    assert!(!layer(&layers, "bridge_casings").builder.is_empty());
}

/// Переезд в одном уровне (R35) кладёт слой дорог — по улице как она
/// нарисована; путь на мосту переезда не даёт, зато даёт путепровод: плиту,
/// парапет и тень в мостовых слоях.
#[test]
fn a_track_across_a_street_is_a_level_crossing_unless_it_is_on_a_bridge() {
    let with_track = |bridge: bool| {
        let mut map = one_street();
        let mut track = fixture::rail(vec![Vec2::new(300.0, 0.0), Vec2::new(300.0, 200.0)], 5.0);
        track.bridge = bridge;
        map.rails.push(track);
        mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        )
    };

    let (layers, report) = with_track(false);
    assert!(!layer(&layers, "rail_crossings").builder.is_empty());
    assert_eq!(report.level_crossings, 1);
    assert_eq!(report.track_bridges, 0);
    assert!(layer(&layers, "bridges").builder.is_empty());

    let (layers, report) = with_track(true);
    assert!(layer(&layers, "rail_crossings").builder.is_empty());
    assert_eq!(report.level_crossings, 0);
    assert_eq!(report.track_bridges, 1);
    for name in ["bridge_shadows", "bridge_casings", "bridges"] {
        assert!(!layer(&layers, name).builder.is_empty(), "{name}");
    }
}

/// Заливка настила кладётся в порядке заливки улиц и несёт раму полос и
/// разрывы асфальта своей улицы: колея шейдера на мосту та же, что на
/// подходе, и гаснет на узле, где мост — не ведущий. Пешеходный мостик в том
/// же меше рамы не получает.
#[test]
fn deck_fill_carries_its_streets_lane_frame() {
    let mut map = MapData::default();
    // широкая улица и мост у́же её, торцом в её средний узел: у моста там
    // разрыв асфальта
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(350.0, 100.0),
            Vec2::new(600.0, 100.0),
        ],
        16.0,
    ));
    map.roads.push(fixture::bridge(
        vec![Vec2::new(350.0, 100.0), Vec2::new(350.0, 400.0)],
        8.0,
    ));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let coords = layer(&layers, "bridges")
        .builder
        .ribbon_coords_for_test()
        .expect("настил — фактурный слой");
    assert!(!coords.is_empty());
    // рама полос: `[поперёк, до разрыва, low, high]` от узла сетки, у ленты
    // без полос — `[поперёк, до разрыва, полуширина, 0]`
    assert!(
        coords.iter().all(|c| c[2] < 0.0 && c[3] > 0.0),
        "the deck fill lost its street's lane frame"
    );
    let nearest = coords.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
    assert!(
        nearest < 10.0,
        "the deck fill ignores the asphalt break at its junction ({nearest} m)"
    );

    // пешеходный мостик — в том же меше, но без полос
    let mut footbridge = MapData::default();
    footbridge.roads.push(RoadLine {
        class: RoadClass::Alley,
        ..fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 40.0)], 3.0)
    });
    let (layers, _) = mesh_roads(
        &footbridge,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let coords = layer(&layers, "bridges")
        .builder
        .ribbon_coords_for_test()
        .unwrap();
    assert!(!coords.is_empty());
    assert!(coords.iter().all(|c| c[3] == 0.0), "a footbridge got lanes");
}

/// Настил, продолжающий наземный подход, кончается у головы моста ровным
/// срезом, как его бортик: полудиск торца ложился поверх асфальта подхода, а
/// у мостика у кромки — светлым полукруглым вырезом на краю проезжей части
/// (R14, Тула, 49777488 → мост 49777171 и тротуар 1506675089 → 1506675090).
/// Свободный торец моста остаётся круглым.
#[test]
fn a_deck_landing_on_its_approach_ends_square() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    map.roads
        .push(fixture::street(vec![Vec2::new(0.0, 0.0), head], 8.0));
    map.roads
        .push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 8.0));
    // тротуар: наземный кусок и мостик — продолжением через свой узел
    let foot = Vec2::new(30.0, 100.0);
    map.roads
        .push(fixture::footway(vec![Vec2::new(30.0, 0.0), foot]));
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::bridge(vec![foot, Vec2::new(30.0, 160.0)], 3.0)
    });
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let positions = layer(&layers, "bridges").builder.positions_for_test();
    assert!(!positions.is_empty());
    let behind = positions
        .iter()
        .map(|position| head.y - position[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        behind < 1e-3,
        "the deck fill runs {behind} m past the bridge head onto its approach"
    );
    // свободный конец — скруглён: заливка выходит за последнюю точку
    let ahead = positions
        .iter()
        .map(|position| position[1] - 160.0)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        ahead > 1.0,
        "the free deck end lost its round cap ({ahead} m)"
    );
}

/// Полоса тротуара подхода кончается на голове моста ровным срезом: её
/// полудиск светился по сторонам настила за его бортиком, как тротуар на
/// самом мосту (R32, Орёл, путепровод 1-й Курской).
#[test]
fn an_approach_sidewalk_ends_square_at_the_bridge_head() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    map.roads
        .push(fixture::street(vec![Vec2::new(0.0, 0.0), head], 14.2));
    map.roads
        .push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 14.2));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    assert!(!sidewalks.is_empty());
    let past = sidewalks
        .iter()
        .map(|position| position[1] - head.y)
        .fold(f32::NEG_INFINITY, f32::max);
    // за голову — только губа под настил (S3), и та не шире бортика: рядом с
    // настилом полосы не видно
    assert!(
        past < BAND_SEAM_OVERLAP + 1e-3,
        "the approach sidewalk runs {past} m past the bridge head"
    );
    let curb = map.roads[1].curb_reach();
    let beside = sidewalks
        .iter()
        .filter(|position| position[1] > head.y + 1e-3)
        .map(|position| position[0].abs())
        .fold(0.0, f32::max);
    assert!(
        beside < curb + 1e-3,
        "past the head the sidewalk reaches {beside} m, beside the deck curb at {curb} m"
    );
}

/// Мостик, замапленный своим way в паре метров от настила (Орёл, мост 1-й
/// Курской: 2.15 м между бортиками), — щель между ними закрыта фартуком в
/// слое настилов, а её продолжение на земле, между полосой тротуара подхода и
/// дорожкой на мостик, — плиткой (S2). Мостик в шести метрах — отдельный
/// мост, фартука нет.
#[test]
fn a_footbridge_beside_a_deck_gets_a_slit_apron() {
    let covered = |layers: &[LayerMesh], name: &str, at: Vec2| {
        layer(layers, name)
            .builder
            .triangles()
            .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
    };
    let scene = |x: f32| {
        let mut map = MapData::default();
        map.roads.push(fixture::street(
            vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)],
            14.2,
        ));
        map.roads.push(fixture::bridge(
            vec![Vec2::new(0.0, 100.0), Vec2::new(0.0, 160.0)],
            14.2,
        ));
        map.roads.push(RoadLine {
            pavement: Some(Pavement::Paved),
            ..fixture::footway(vec![Vec2::new(x, 40.0), Vec2::new(x, 100.0)])
        });
        map.roads.push(RoadLine {
            class: RoadClass::Alley,
            highway: Highway::Path,
            ..fixture::bridge(vec![Vec2::new(x, 100.0), Vec2::new(x, 160.0)], 3.5)
        });
        map.network = RoadNetwork::new(&map.roads);
        mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        )
        .0
    };
    let near = scene(-13.5);
    assert!(
        covered(&near, "bridges", Vec2::new(-10.0, 130.0)),
        "the slit between the deck curb and the footbridge is open"
    );
    assert!(
        covered(&near, "sidewalks", Vec2::new(-11.0, 95.0)),
        "the ground between the approach sidewalk and the path is open"
    );
    let far = scene(-17.0);
    assert!(!covered(&far, "bridges", Vec2::new(-12.0, 130.0)));
}

/// Мостик, идущий берегом улицы без настила (R32, Орёл, 1-я Курская): берег
/// моста длиннее настила, и вдоль подхода читалась тройка «тротуар, щель
/// грунта, мостик». Щель между полосой тротуара улицы и настилом мостика
/// мостится плиткой; мостик в шести метрах — отдельный мост, фартука нет.
#[test]
fn a_footbridge_along_a_street_approach_gets_a_tile_apron() {
    let covered = |layers: &[LayerMesh], at: Vec2| {
        layer(layers, "sidewalks")
            .builder
            .triangles()
            .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
    };
    let scene = |x: f32| {
        let mut map = MapData::default();
        map.roads.push(fixture::street(
            vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)],
            14.2,
        ));
        map.roads.push(fixture::bridge(
            vec![Vec2::new(0.0, 100.0), Vec2::new(0.0, 160.0)],
            14.2,
        ));
        // мостик начинается задолго до настила — берегом, вдоль улицы
        map.roads.push(RoadLine {
            class: RoadClass::Alley,
            highway: Highway::Path,
            pavement: Some(Pavement::Paved),
            ..fixture::bridge(vec![Vec2::new(x, -20.0), Vec2::new(x, 160.0)], 3.5)
        });
        map.network = RoadNetwork::new(&map.roads);
        mesh_roads(
            &map,
            RoadStyle::default(),
            RoadShape::default(),
            VergeZoomBucket::for_zoom(0.0),
        )
        .0
    };
    let near = scene(-13.5);
    let strip = [-11.5, -10.5, -9.5, -8.5]
        .iter()
        .any(|&x| covered(&near, Vec2::new(x, 80.0)));
    assert!(strip, "щель между тротуаром подхода и мостиком не закрыта");
    let far = scene(-17.0);
    assert!(
        !covered(&far, Vec2::new(-13.0, 80.0)),
        "далеко — без фартука"
    );
}

/// Мостик, вынесенный из настила к его кромке (R14), тянет за собой дорожку,
/// и та, возвращаясь на своё место, расходится с полосой тротуара подхода
/// клином голой земли (S5, Тула, Одоевский путепровод, 2411 3020): щели на
/// настиле нет — бортики внахлёст, — а клин на земле мостится. И дорожка у
/// головы расширяется в мостик: без клина её торец стоял уступом внутри его
/// бортика.
#[test]
fn a_moved_footbridges_path_meets_the_band_without_a_wedge() {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)],
        7.6,
    ));
    map.roads.push(fixture::bridge(
        vec![Vec2::new(0.0, 100.0), Vec2::new(0.0, 160.0)],
        7.6,
    ));
    let foot = Vec2::new(-3.6, 100.0);
    map.roads.push(RoadLine {
        pavement: Some(Pavement::Paved),
        ..fixture::footway(vec![Vec2::new(-12.0, 40.0), foot])
    });
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::bridge(vec![foot, Vec2::new(-3.6, 160.0)], 3.5)
    });
    map.network = RoadNetwork::new(&map.roads);
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let covered = |name: &str, at: Vec2| {
        layer(&layers, name)
            .builder
            .triangles()
            .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
    };
    // от асфальта подхода до оси дорожки — ни пятна голой земли
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let path = prepared.axis(2, Axis::Ribbon);
    let axis_at = |y: f32| {
        path.windows(2).find_map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            ((a.y - y) * (b.y - y) <= 0.0 && a.y != b.y)
                .then(|| a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y))
        })
    };
    for y in [86.0, 90.0, 94.0, 98.0] {
        let axis = axis_at(y).expect("the path reaches the head");
        let mut x = -3.9;
        while x > axis {
            let at = Vec2::new(x, y);
            assert!(
                covered("roads", at) || covered("sidewalks", at),
                "bare ground at {at} between the band and the path (axis at {axis})"
            );
            x -= 0.1;
        }
    }
    // клин дорожки в мостик: у головы — до кромки его бортика
    let reach = map.roads[3].curb_reach();
    let axis = prepared.axis(3, Axis::Nodal)[0].x;
    let at = Vec2::new(axis - reach + 0.1, 99.9);
    assert!(
        covered("sidewalks", at),
        "the path stops short of its footbridge's curb at {at}"
    );
}

/// Асфальт подхода, срезанный в узле шва прямо (в узле примыкает третья
/// улица), стоит поперёк своей оси, торец настила — поперёк своей: на изломе
/// шва между ними светилась нитка земли наискось через полосы (S3, Орёл,
/// конец моста Р-119, где в узле шва сходятся половины). Под торцом настила
/// лежит фартук асфальта — нитки нет.
#[test]
fn no_ground_shows_between_a_kinked_approach_and_the_deck_head() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    let kink = 3.0f32.to_radians();
    map.roads.push(fixture::street(
        vec![head - Vec2::new(kink.sin(), kink.cos()) * 100.0, head],
        14.2,
    ));
    map.roads
        .push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 14.2));
    map.roads
        .push(fixture::street(vec![head, Vec2::new(-80.0, 100.0)], 7.6));
    map.network = RoadNetwork::new(&map.roads);
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let covered = |at: Vec2| {
        ["roads", "bridges"].iter().any(|name| {
            layer(&layers, name)
                .builder
                .triangles()
                .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
        })
    };
    for step in 0..=20 {
        let x = -6.9 + 13.8 * step as f32 / 20.0;
        let at = Vec2::new(x, head.y - 0.03);
        assert!(covered(at), "bare ground at {at} behind the deck head");
    }
}

/// Прямой торец полосы тротуара подхода стоит поперёк своей оси, торец
/// настила — поперёк своей: на изломе шва между ними светилась нитка земли,
/// наискось от кромки через крайнюю полосу (S3, Орёл, конец моста Р-119).
/// Полоса заходит под настил губой — нитки нет.
#[test]
fn an_approach_sidewalk_runs_under_a_kinked_deck_head() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    // подход под 2° к мосту
    let kink = 2.0f32.to_radians();
    map.roads.push(fixture::street(
        vec![head - Vec2::new(kink.sin(), kink.cos()) * 100.0, head],
        14.2,
    ));
    map.roads
        .push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 14.2));
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let sidewalks = &layer(&layers, "sidewalks").builder;
    let deck = &layer(&layers, "bridges").builder;
    let curb = map.roads[1].curb_reach();
    // вдоль торца настила, от кромки его асфальта до кромки бортика
    for step in 0..=10 {
        let lateral = 7.1 + (curb - 7.1 - 0.1) * step as f32 / 10.0;
        for x in [-lateral, lateral] {
            let at = Vec2::new(x, head.y - 0.02);
            let covered = |builder: &MeshBuilder| {
                builder
                    .triangles()
                    .any(|[a, b, c]| crate::map::osm::model::point_in_polygon(at, &[a, b, c]))
            };
            assert!(
                covered(sidewalks) || covered(deck),
                "bare ground at {at} behind the deck head"
            );
        }
    }
}

/// Полоса тротуара подхода сходит у головы моста клином в бортик настила:
/// полоса шире бортика по построению, и её прямой торец стоял уступом наружу
/// от бортика (S1, Орёл, Красный мост; S5, Тула, Одоевский путепровод). Дальше
/// клина полоса — во всю ширину.
#[test]
fn an_approach_sidewalk_tapers_into_the_deck_curb() {
    let head = Vec2::new(0.0, 100.0);
    let mut map = MapData::default();
    map.roads
        .push(fixture::street(vec![Vec2::new(0.0, 0.0), head], 14.2));
    map.roads
        .push(fixture::bridge(vec![head, Vec2::new(0.0, 160.0)], 14.2));
    let curb = map.roads[1].curb_reach();
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let sidewalks = layer(&layers, "sidewalks").builder.positions_for_test();
    let reach_near = |from: f32, to: f32| {
        sidewalks
            .iter()
            .filter(|position| (from..=to).contains(&(position[1] - head.y)))
            .map(|position| position[0].abs())
            .fold(0.0, f32::max)
    };
    let at_head = reach_near(-0.05, 0.05);
    assert!(
        (at_head - curb).abs() < 0.05,
        "at the head the sidewalk reaches {at_head} m, the deck curb {curb} m"
    );
    let band = reach_near(-60.0, -BAND_SEAM_TAPER - 0.1);
    assert!(
        band > curb + 0.3,
        "past the taper the band keeps its width: {band} m"
    );
}

/// Мостик, замапленный внутри настила моста (Тула: 3.6 м от оси при
/// полуширине 3.8), выносится к его кромке, а наземная дорожка, продолжающая
/// его, идёт следом: иначе мостик полосой закрывал край проезжей части, а за
/// головой моста его торец лежал светлым прямоугольником на асфальте подхода
/// (R14).
#[test]
fn a_footbridge_inside_the_deck_moves_to_its_edge() {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)],
        7.6,
    ));
    map.roads.push(fixture::bridge(
        vec![Vec2::new(0.0, 100.0), Vec2::new(0.0, 160.0)],
        7.6,
    ));
    let foot = Vec2::new(-3.6, 101.0);
    map.roads
        .push(fixture::footway(vec![Vec2::new(-20.0, 80.0), foot]));
    map.roads.push(RoadLine {
        class: RoadClass::Alley,
        highway: Highway::Path,
        ..fixture::bridge(vec![foot, Vec2::new(-3.6, 160.0)], 3.5)
    });
    map.network = RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    // и бортик мостика — за кромкой: на подходе он торчал бы на асфальт
    let need = 7.6 / 2.0 + map.roads[3].curb_reach();
    let footbridge = prepared.axis(3, Axis::Nodal);
    for point in footbridge {
        assert!(
            point.x <= -need + 1e-3,
            "the footbridge stays {} m off the deck axis, inside its edge",
            -point.x
        );
    }
    // наземная дорожка сходится в тот же торец
    let ground = prepared.axis(2, Axis::Nodal);
    let joint = ground[ground.len() - 1];
    assert!(
        joint.distance(footbridge[0]) < 1e-3,
        "the ground footway ends at {joint}, the footbridge starts at {}",
        footbridge[0]
    );
    assert_eq!(ground[0], Vec2::new(-20.0, 80.0), "the far end moved too");
}

/// Мощёная разделительная двух настилов-половин лежит в слое настилов: в слое
/// улиц её закрывала тень моста, и между половинами шла тёмная щель с
/// бортиками по краям — светлый разделитель вместо двойной сплошной подхода
/// (R30).
#[test]
fn a_paved_median_between_two_decks_lies_on_the_deck() {
    let mut map = MapData::default();
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(2),
        ..fixture::bridge(points, 7.6)
    };
    map.roads
        .push(half(vec![Vec2::new(-4.5, 0.0), Vec2::new(-4.5, 200.0)]));
    map.roads
        .push(half(vec![Vec2::new(4.5, 200.0), Vec2::new(4.5, 0.0)]));
    map.network = RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(
        prepared
            .pairs()
            .medians()
            .iter()
            .any(|median| median.is_paved()),
        "the two decks make no paved median"
    );
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let middle = Vec2::new(0.0, 100.0);
    assert!(
        layer(&layers, "bridges").builder.covers_for_test(middle),
        "the median between the decks is not on the deck"
    );
}

/// Над газоном между настилами-половинами бортик со стороны пары остаётся:
/// там он край моста, а не разделитель (Тула, витрина 20 — половины развязки
/// в десятке метров друг от друга потеряли внутренний бортик).
#[test]
fn decks_parted_over_a_lawn_keep_their_inner_curbs() {
    let mut map = MapData::default();
    let half = |points: Vec<Vec2>| RoadLine {
        highway: Highway::Primary,
        oneway: true,
        lanes: Some(2),
        ..fixture::bridge(points, 7.6)
    };
    map.roads
        .push(half(vec![Vec2::new(-10.0, 0.0), Vec2::new(-10.0, 200.0)]));
    map.roads
        .push(half(vec![Vec2::new(10.0, 200.0), Vec2::new(10.0, 0.0)]));
    map.network = RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(
        prepared
            .pairs()
            .medians()
            .iter()
            .any(|median| !median.is_paved()),
        "the two decks make no lawn median"
    );
    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let curb = map.roads[0].curb_reach() - map.roads[0].width / 2.0;
    let inner = Vec2::new(-10.0 + 3.8 + curb / 2.0, 100.0);
    assert!(
        layer(&layers, "bridge_casings")
            .builder
            .covers_for_test(inner),
        "the deck lost its inner curb over the lawn"
    );
}

/// На шве земля/мост со сменой числа полос настил берёт клин, как шов улицы:
/// у головы моста он шириной в подход и расходится до своей, бортик сужается
/// вместе с ним. Без клина ширина прыгала ступенькой ровно на голове, а шов
/// закрывали полудиски торцов (R30, Красный мост в Орле: земля 2 полосы, мост
/// 3).
#[test]
fn a_wider_deck_tapers_from_its_approach() {
    let head = Vec2::new(0.0, 100.0);
    let lanes = |lanes: u8| 3.3 * f32::from(lanes) + 1.0;
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        highway: Highway::Primary,
        lanes: Some(2),
        ..fixture::street(vec![Vec2::new(0.0, 0.0), head], lanes(2))
    });
    map.roads.push(RoadLine {
        highway: Highway::Primary,
        lanes: Some(3),
        ..fixture::bridge(vec![head, Vec2::new(0.0, 200.0)], lanes(3))
    });
    map.network = RoadNetwork::new(&map.roads);
    let prepared = Drawn::new(
        &map,
        &RoadStyle::default(),
        &RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    assert!(
        prepared.taper_ends(1)[0].is_some(),
        "the deck takes no taper at its head"
    );

    let (layers, _) = mesh_roads(
        &map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    let near_head = |name: &str| -> f32 {
        layer(&layers, name)
            .builder
            .positions_for_test()
            .iter()
            .filter(|position| (position[1] - head.y).abs() < 0.5)
            .map(|position| position[0].abs())
            .fold(0.0, f32::max)
    };
    let narrow = lanes(2) / 2.0;
    let fill = near_head("bridges");
    assert!(
        (fill - narrow).abs() < 0.05,
        "the deck starts {fill} m wide at its head, the approach is {narrow}"
    );
    let curb = map.roads[1].curb_reach() - map.roads[1].width / 2.0;
    let casing = near_head("bridge_casings");
    assert!(
        casing < narrow + curb + 0.05,
        "the curb stands {casing} m out at the head, past the tapered deck"
    );
}
