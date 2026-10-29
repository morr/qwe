use super::fixtures::{CITY, HALF, corners, tags};
use super::*;
use crate::map::osm::fixture::Overpass;
use crate::map::osm::model::{Highway, LaneTurn};
use bevy::math::Vec2;

/// Арки в Туле размечены двумя разными тегами, а подземный туннель —
/// третьим, и он аркой быть не должен.
#[test]
fn arches_are_recognised_by_tunnel_and_covered_but_not_by_a_real_tunnel() {
    assert!(is_building_passage(&tags(&[(
        "tunnel",
        "building_passage"
    )])));
    assert!(is_building_passage(&tags(&[(
        "covered",
        "building_passage"
    )])));
    assert!(is_building_passage(&tags(&[("covered", "yes")])));
    assert!(!is_building_passage(&tags(&[("tunnel", "yes")])));
    assert!(!is_building_passage(&tags(&[("bridge", "yes")])));
    assert!(!is_building_passage(&tags(&[])));
}

/// Один way — три фичи. Ровно то, что в `parse_way` закодировано
/// **расположением** `return`'ов: рельсы, аллея и водоток разбираются до дорог
/// и не прерывают разбор, а `highway` прерывает. Конвенция держалась на трёх
/// комментариях; здесь она перестаёт быть конвенцией.
///
/// Сцена не выдуманная: трамвайные пути в OSM сплошь висят на том же way, что
/// и улица, а водоток вдоль неё размечен на нём же.
#[test]
fn one_way_becomes_a_street_a_tramway_and_a_watercourse_at_once() {
    let (sw, se, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[
                ("highway", "secondary"),
                ("railway", "tram"),
                ("waterway", "ditch"),
                ("natural", "tree_row"),
            ],
            vec![sw, se],
        )
        .parse();

    assert_eq!(map.roads.len(), 1, "улица");
    assert_eq!(map.rails.len(), 1, "трамвайный путь");
    assert_eq!(map.water_lines.len(), 1, "водоток");
    assert_eq!(map.tree_rows.len(), 1, "аллея");
}

/// `tunnel=culvert` на общем way описывает ручей, а не улицу над ним.
///
/// Труба — обычный способ пустить ручей под дорогой, куда более частый, чем
/// мост; распространив правило подземного на дороги «в лоб», эту улицу с
/// карты сносило (поймано существующим
/// [`parses_linear_waterways_and_keeps_riverbank_an_area`]). Для водотока при
/// этом `culvert` обязан остаться подземным: незапомненная труба перегородит
/// навмеш и отрежет квартал.
#[test]
fn a_culvert_hides_the_stream_and_leaves_the_street_above_it() {
    let (sw, se, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[
                ("highway", "residential"),
                ("waterway", "stream"),
                ("tunnel", "culvert"),
            ],
            vec![sw, se],
        )
        .parse();

    assert_eq!(map.roads.len(), 1, "улица над трубой — на поверхности");
    assert!(map.water_lines[0].tunnel, "а ручей в ней — под землёй");

    // труба, которая И правда под землёй, помечена явно — тогда улицы нет
    let deep = Overpass::new(CITY)
        .way(
            &[
                ("highway", "residential"),
                ("waterway", "stream"),
                ("tunnel", "culvert"),
                ("layer", "-1"),
            ],
            vec![sw, se],
        )
        .parse();
    assert_eq!(deep.roads.len(), 0);
}

/// Подземное не выходит на поверхность — и у дороги тоже.
///
/// Правило было применено к рельсам и водотокам, но не к `highway`, и
/// подземный переход рисовался обычной дорожкой: в Токио 1399 way из 12 859.
#[test]
fn an_underground_road_never_reaches_the_map() {
    let (sw, se, ..) = corners(HALF);
    let surface = |tags: &[(&str, &str)]| {
        Overpass::new(CITY)
            .way(tags, vec![sw, se])
            .parse()
            .roads
            .len()
    };

    assert_eq!(surface(&[("highway", "footway")]), 1, "обычная дорожка");
    assert_eq!(surface(&[("highway", "footway"), ("tunnel", "yes")]), 0);
    assert_eq!(surface(&[("highway", "footway"), ("layer", "-1")]), 0);
    // `layer` выше нуля — эстакада, она как раз видна
    assert_eq!(surface(&[("highway", "footway"), ("layer", "1")]), 1);
    assert_eq!(surface(&[("highway", "footway"), ("tunnel", "no")]), 1);
}

/// Мост и арка правилу подземного не подчиняются: обе роли существуют на
/// уровне ходьбы по определению, а `layer` у них говорит «ниже того, что
/// сверху пересекает».
///
/// Риск здесь несимметричен, и это главное. Лишняя лента — косметика; лишний
/// снос — дыра в навмеше: аркой закрывается двор, в который другого входа нет,
/// а мостом — единственная переправа через реку. Правило «в лоб» уносило в
/// Токио 331 арку и 17 мостов, в Лондоне 177 арок.
#[test]
fn a_bridge_and_an_arch_outrank_the_underground_rule() {
    let (sw, se, ..) = corners(HALF);
    let road = |tags: &[(&str, &str)]| Overpass::new(CITY).way(tags, vec![sw, se]).parse().roads;

    // арка обоих начертаний, вместе с `layer=-1` — проезд под домом
    for arch in [
        &[("tunnel", "building_passage")][..],
        &[("covered", "yes"), ("layer", "-1")][..],
    ] {
        let tags: Vec<(&str, &str)> = [("highway", "service")]
            .into_iter()
            .chain(arch.iter().copied())
            .collect();
        let roads = road(&tags);
        assert_eq!(roads.len(), 1, "арка {arch:?} обязана дожить до карты");
        assert!(roads[0].passage, "и остаться проездом сквозь дом");
    }

    // мост с противоречивой разметкой: сносить переправу нельзя
    let roads = road(&[
        ("highway", "residential"),
        ("bridge", "yes"),
        ("layer", "-1"),
    ]);
    assert_eq!(roads.len(), 1, "мост обязан дожить до карты");
    assert!(
        roads[0].bridge,
        "и остаться мостом — по нему режется навмеш"
    );
}

/// Односторонность, кольцо и число полос — то, по чему рисуется разметка.
#[test]
fn oneway_roundabout_and_lanes_reach_the_road() {
    let (sw, se, ..) = corners(HALF);
    let road = |extra: &[(&str, &str)]| {
        let tags: Vec<(&str, &str)> = [("highway", "residential")]
            .into_iter()
            .chain(extra.iter().copied())
            .collect();
        Overpass::new(CITY)
            .way(&tags, vec![sw, se])
            .parse()
            .roads
            .remove(0)
    };

    let plain = road(&[]);
    assert!(!plain.oneway && !plain.roundabout);
    // без тега и без соседей по улице — полосы по классу, ширина из них
    assert_eq!(plain.lanes, Some(2));
    assert!((plain.width - 7.6).abs() < 1e-4, "{}", plain.width);

    assert!(road(&[("oneway", "yes")]).oneway);
    assert!(road(&[("oneway", "-1")]).oneway);
    assert!(!road(&[("oneway", "no")]).oneway);
    assert!(!road(&[("oneway", "reversible")]).oneway);

    let ring = road(&[("junction", "roundabout")]);
    assert!(
        ring.roundabout && ring.oneway,
        "кольцо одностороннее по определению"
    );
    assert!(road(&[("junction", "circular")]).roundabout);

    assert!(plain.lane_markings, "без тега разметку решает число полос");
    assert!(!road(&[("lane_markings", "no")]).lane_markings);
    assert!(road(&[("lane_markings", "yes")]).lane_markings);

    let four = road(&[("lanes", "4")]);
    assert_eq!(four.lanes, Some(4));
    assert!((four.width - 14.2).abs() < 1e-4, "{}", four.width);

    // сам тег — до сечений, которые его неправдоподобное значение заменят
    // дефолтом по классу
    let tagged = |pairs: &[(&str, &str)]| tagged_lanes(&tags(pairs));
    assert_eq!(tagged(&[("lanes", "2;3")]), Some(2));
    assert_eq!(tagged(&[("lanes", "2.5")]), Some(2));
    assert_eq!(tagged(&[("lanes", "0")]), None);
    assert_eq!(
        tagged(&[("lanes", "12")]),
        None,
        "за восемью полосами — не лента, а вся развязка"
    );
    assert_eq!(
        tagged(&[("lanes:forward", "2"), ("lanes:backward", "1")]),
        Some(3),
        "без общего `lanes` — сумма по направлениям"
    );
    assert_eq!(
        tagged(&[("lanes:forward", "2")]),
        None,
        "одно направление без второго — не сумма"
    );
    assert_eq!(road(&[("lanes", "0")]).lanes, Some(2));

    // деление по потокам: назад — `lanes:backward`, иначе `lanes` без
    // `lanes:forward`; у общей средней полосы и у односторонней его нет
    let backward = |pairs: &[(&str, &str)]| tagged_lanes_backward(&tags(pairs));
    assert_eq!(backward(&[("lanes", "5"), ("lanes:forward", "3")]), Some(2));
    assert_eq!(backward(&[("lanes:backward", "1")]), Some(1));
    assert_eq!(backward(&[("lanes", "5")]), None);
    assert_eq!(
        backward(&[
            ("lanes", "5"),
            ("lanes:forward", "2"),
            ("lanes:both_ways", "1")
        ]),
        None
    );
    assert_eq!(
        backward(&[("lanes", "3"), ("lanes:forward", "2"), ("oneway", "yes")]),
        None
    );
}

/// `turn:lanes` — по направлению потока: у односторонней `oneway=-1` тег
/// `:backward` становится первым, у двусторонней общий тег не читается.
#[test]
fn turn_lanes_follow_the_flow() {
    let turns = |pairs: &[(&str, &str)]| tagged_turns(&tags(pairs));
    let lane = |left, through, right| LaneTurn {
        left,
        through,
        right,
    };
    let (left, through, right) = (
        lane(true, false, false),
        lane(false, true, false),
        lane(false, true, true),
    );
    assert_eq!(
        turns(&[
            ("oneway", "yes"),
            ("turn:lanes", "left|through|through;right")
        ]),
        [vec![left, through, right], Vec::new()]
    );
    assert_eq!(
        turns(&[("oneway", "-1"), ("turn:lanes:backward", "left|throught")]),
        [vec![left, through], Vec::new()],
        "опечатка — прямо"
    );
    assert_eq!(
        turns(&[("turn:lanes", "left|through")]),
        [Vec::new(), Vec::new()],
        "у двусторонней чьи это полосы, не сказано"
    );
    assert_eq!(
        turns(&[("turn:lanes:backward", "|right")]),
        [Vec::new(), vec![through, lane(false, false, true)]]
    );
}

/// Съезды развязок (`*_link`) — дороги своего класса, а не мусор словаря.
#[test]
fn a_link_road_reaches_the_map() {
    let (sw, se, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[
                ("highway", "primary_link"),
                ("oneway", "yes"),
                ("lanes", "1"),
            ],
            vec![sw, se],
        )
        .parse();
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].highway, Highway::PrimaryLink);
    assert!((map.roads[0].width - 4.3).abs() < 1e-4);
}

/// `oneway=-1` — поток против порядка точек; way разворачивается при разборе,
/// чтобы «направление way» ниже по конвейеру значило «направление движения».
#[test]
fn oneway_backward_reverses_the_way() {
    let (sw, se, ..) = corners(HALF);
    let road = |extra: &[(&str, &str)]| {
        let tags: Vec<(&str, &str)> = [("highway", "residential")]
            .into_iter()
            .chain(extra.iter().copied())
            .collect();
        Overpass::new(CITY)
            .way(&tags, vec![sw, se])
            .parse()
            .roads
            .remove(0)
    };

    let forward = road(&[("oneway", "yes")]);
    let backward = road(&[("oneway", "-1")]);
    assert!(backward.oneway);
    let reversed: Vec<Vec2> = forward.points.iter().rev().copied().collect();
    for (point, twin) in backward.points.iter().zip(&reversed) {
        assert!(point.distance(*twin) < 0.01, "{point} / {twin}");
    }
}
