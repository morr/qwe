use super::*;
// посадка деревьев переехала в соседний модуль, но проверяется она через
// весь конвейер — от JSON Overpass до `map.trees`
use crate::map::osm::fixture::{Overpass, closed, rect, square};
use crate::map::osm::model::distance_to_segment;
use crate::map::osm::planting::{
    TREE_CROWN_REACH, TREE_MIN_SPACING, TREE_SHORE_CLEARANCE, TREE_WALL_CLEARANCE, near_area_edge,
};
use crate::settings::MAP_SIZE;

/// Фикстуры строятся вокруг гео-центра Тулы — города по умолчанию.
const CITY: City = City::Tula;

/// Центр карты: сцены собираются вокруг него, и в тех же метрах пишутся
/// проверки — фикстуре незачем говорить в градусах.
const CENTER: Vec2 = Vec2::new(MAP_SIZE.x / 2.0, MAP_SIZE.y / 2.0);

/// Половина стороны обычной сцены: дом, пруд, отрезок дороги.
const HALF: f32 = 55.0;
/// Половина стороны лесной сцены — посадке нужно место.
const WOOD_HALF: f32 = 110.0;

/// Углы квадратной сцены вокруг центра карты: юго-западный, юго-восточный,
/// северо-восточный, северо-западный.
fn corners(half: f32) -> (Vec2, Vec2, Vec2, Vec2) {
    let (min, max) = (CENTER - Vec2::splat(half), CENTER + Vec2::splat(half));
    (min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y))
}

/// Мини-ответ Overpass: way-здание, дорога-мост, relation-вода из двух
/// половинок с дыркой-островом.
fn fixture() -> Overpass {
    let (sw, se, ne, nw) = corners(HALF);
    Overpass::new(CITY)
        .area(&[("building", "yes")], square(CENTER, HALF))
        .way(&[("highway", "secondary"), ("bridge", "yes")], vec![sw, ne])
        .way(&[("highway", "proposed")], vec![sw, ne])
        .relation(
            &[("natural", "water")],
            &[
                ("outer", vec![sw, se, ne]),
                ("outer", vec![ne, nw, sw]),
                ("inner", closed(square(CENTER, HALF / 4.0))),
            ],
        )
}

/// Лес во всю сцену — фон тестов посадки: деревья растут только в лесных
/// полигонах, поэтому каждое правило проверяется как вычитание из него.
fn wood_scene() -> Overpass {
    Overpass::new(CITY).area(&[("natural", "wood")], square(CENTER, WOOD_HALF))
}

/// Ряд деревьев поперёк сцены — общее начало тестов `natural=tree_row`.
fn tree_row(tags: &[(&str, &str)]) -> MapData {
    let (sw, se, ..) = corners(HALF);
    Overpass::new(CITY).way(tags, vec![sw, se]).parse()
}

#[test]
fn parses_building_road_and_multipolygon() {
    let map = fixture().parse();

    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.buildings[0].outer.len(), 4);

    // proposed отброшен, secondary-мост остался
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].width, 12.0);
    assert!(map.roads[0].bridge);

    assert_eq!(map.water.len(), 1);
    let water = &map.water[0];
    assert_eq!(water.holes.len(), 1);
    // центр — в дырке-острове: суша
    assert!(!point_in_area(CENTER, water));
    // точка между границей и островом — вода
    assert!(point_in_area(CENTER + Vec2::new(0.0, 40.0), water));
}

#[test]
fn parses_rails_and_drops_station_furniture() {
    let (sw, se, ne, nw) = corners(HALF);
    // путь, платформа (отбрасывается), заброшенная ветка и трамвай на улице
    let map = Overpass::new(CITY)
        .way(&[("railway", "rail")], vec![sw, ne])
        .way(&[("railway", "platform")], vec![sw, ne])
        .way(&[("railway", "abandoned")], vec![se, nw])
        .way(
            &[("railway", "tram"), ("highway", "residential")],
            vec![sw, se],
        )
        .parse();

    assert_eq!(map.rails.len(), 3, "platform must not become a rail");
    assert_eq!(map.rails[0].width, 5.0);
    assert_eq!(map.rails[0].kind, RailKind::Active);
    assert_eq!(map.rails[1].kind, RailKind::Disused);

    // трамвайный путь на улице — это и улица, и путь: way попадает в оба списка
    assert_eq!(map.rails[2].kind, RailKind::Tram);
    // и он тоньше улицы, по которой идёт, — иначе линия закрыла бы саму улицу
    assert!(map.rails[2].width < map.roads[0].width);
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].width, 8.0);
    assert_eq!(map.roads[0].points, map.rails[2].points);

    // рельсы существуют только для картинки — навмеша они не касаются
    assert!(map.walls.is_empty());
    assert!(map.buildings.is_empty());
}

#[test]
fn underground_tracks_are_not_drawn() {
    let (sw, se, ne, nw) = corners(HALF);
    // подземные размечены по-разному: тоннелем, отрицательным слоем или обоими
    let map = Overpass::new(CITY)
        .way(
            &[("railway", "subway"), ("tunnel", "yes"), ("layer", "-1")],
            vec![sw, ne],
        )
        .way(&[("railway", "rail"), ("layer", "-1")], vec![sw, ne])
        .way(&[("railway", "rail"), ("tunnel", "yes")], vec![sw, ne])
        .way(&[("railway", "subway"), ("layer", "1")], vec![se, nw])
        .way(&[("railway", "rail"), ("tunnel", "no")], vec![sw, se])
        .parse();

    // остаются только надземные: эстакадное метро и путь с явным `tunnel=no`
    assert_eq!(map.rails.len(), 2);
    assert_eq!(map.rails[0].width, 4.0, "elevated subway must survive");
    assert_eq!(map.rails[1].width, 5.0);
}

#[test]
fn parses_linear_waterways_and_keeps_riverbank_an_area() {
    let (sw, se, ne, nw) = corners(HALF);
    // русло, ручей с шириной из тегов, канава, замкнутый riverbank (площадь),
    // плотина (не линия) и ручей под улицей в трубе
    let map = Overpass::new(CITY)
        .way(&[("waterway", "river")], vec![sw, ne])
        .way(&[("waterway", "stream"), ("width", "3,5")], vec![se, nw])
        .way(&[("waterway", "ditch")], vec![sw, se])
        .area(&[("waterway", "riverbank")], square(CENTER, HALF))
        .way(&[("waterway", "dam")], vec![sw, ne])
        .way(
            &[
                ("waterway", "stream"),
                ("tunnel", "culvert"),
                ("highway", "residential"),
            ],
            vec![nw, ne],
        )
        .parse();

    // `dam` линией не становится — белый список, а не «всё, что waterway»
    assert_eq!(map.water_lines.len(), 4);
    assert_eq!(map.water_lines[0].kind, WaterKind::River);
    assert_eq!(map.water_lines[0].width, 8.0);
    // ширина из тегов бьёт дефолт класса, и запятая как разделитель разбирается
    assert_eq!(map.water_lines[1].kind, WaterKind::Stream);
    assert_eq!(map.water_lines[1].width, 3.5);
    assert_eq!(map.water_lines[2].kind, WaterKind::Ditch);
    assert_eq!(map.water_lines[2].width, 1.5);

    // труба помечена, и way остался при этом улицей: ветка водотока не должна
    // затыкать разбор чужих тегов на том же way
    assert!(map.water_lines[3].tunnel);
    assert!(!map.water_lines[0].tunnel);
    assert_eq!(map.roads.len(), 1);
    assert_eq!(map.roads[0].points, map.water_lines[3].points);

    // замкнутый riverbank — по-прежнему площадь, а не лента
    assert_eq!(map.water.len(), 1);
    assert_eq!(map.water[0].outer.len(), 4);
}

#[test]
fn implausible_waterway_width_falls_back_to_the_class_default() {
    let (sw, se, ne, nw) = corners(HALF);
    // `width=200` на ручье — это пойма или опечатка; лента такой ширины, раз
    // водотоки блокируют навмеш, отрезала бы полгорода
    let map = Overpass::new(CITY)
        .way(&[("waterway", "stream"), ("width", "200")], vec![sw, ne])
        .way(&[("waterway", "canal"), ("width", "0.1")], vec![se, nw])
        .parse();

    assert_eq!(map.water_lines.len(), 2);
    assert_eq!(map.water_lines[0].width, 2.5);
    assert_eq!(map.water_lines[1].width, 6.0);
}

#[test]
fn trees_are_deterministic_and_inside_the_wood() {
    let scene = wood_scene();
    let first = scene.parse();
    let second = scene.parse();

    assert!(!first.trees.is_empty());
    assert_eq!(first.trees, second.trees);
    for &(pos, radius) in &first.trees {
        assert!(point_in_area(pos, &first.woods[0]), "{pos:?}");
        assert!((2.5..=4.0).contains(&radius));
    }
    for (index, &(pos, _)) in first.trees.iter().enumerate() {
        for &(other, _) in &first.trees[index + 1..] {
            assert!(
                pos.distance(other) >= TREE_MIN_SPACING,
                "trees too close: {pos:?} vs {other:?}"
            );
        }
    }
}

#[test]
fn trees_avoid_a_pond_inside_the_wood() {
    let (.., ne, _) = corners(WOOD_HALF);
    // пруд занимает северо-восточную четверть массива
    let map = wood_scene()
        .area(&[("natural", "water")], rect(CENTER, ne))
        .parse();

    assert_eq!(map.water.len(), 1);
    assert!(!map.trees.is_empty());
    let pond = &map.water[0];
    for &(pos, _) in &map.trees {
        assert!(!point_in_area(pos, pond), "tree in the pond at {pos:?}");
        assert!(
            !near_area_edge(pos, pond, TREE_SHORE_CLEARANCE),
            "tree on the shoreline at {pos:?}"
        );
    }
}

/// Дом целиком в пруду выбрасывается, дом на берегу с одним углом в воде —
/// остаётся (пирс, набережная).
#[test]
fn buildings_standing_in_water_are_dropped() {
    // пруд — квадрат вокруг центра; первый дом внутри него, второй сидит на
    // южном берегу и заходит в воду только верхней парой углов
    let north_east = CENTER + Vec2::new(HALF, 0.0);
    let south_east = CENTER + Vec2::new(HALF, -HALF);
    let map = Overpass::new(CITY)
        .area(&[("natural", "water")], square(CENTER, WOOD_HALF))
        .area(
            &[("building", "yes")],
            rect(CENTER - Vec2::new(0.0, HALF), north_east),
        )
        .area(
            &[("building", "yes")],
            rect(CENTER - Vec2::new(0.0, WOOD_HALF * 2.0), south_east),
        )
        .parse();

    assert_eq!(map.water.len(), 1);
    assert_eq!(map.buildings.len(), 1, "only the shore building survives");
    // у выжившего есть угол вне воды
    let survivor = &map.buildings[0];
    assert!(
        survivor
            .outer
            .iter()
            .any(|&point| !point_in_area(point, &map.water[0]))
    );
}

#[test]
fn trees_avoid_grass_and_sand_inside_the_wood() {
    let (sw, _, ne, nw) = corners(WOOD_HALF);
    // луг — восточная половина массива, песок — северо-западная четверть
    let map = wood_scene()
        .area(
            &[("landuse", "meadow")],
            rect(Vec2::new(CENTER.x, sw.y), ne),
        )
        .area(
            &[("natural", "beach")],
            rect(Vec2::new(nw.x, CENTER.y), Vec2::new(CENTER.x, ne.y)),
        )
        .parse();

    assert_eq!(map.woods.len(), 1);
    assert_eq!(map.grass.len(), 1);
    assert_eq!(map.sand.len(), 1);
    assert!(!map.trees.is_empty());
    for &(pos, _) in &map.trees {
        assert!(
            !point_in_area(pos, &map.grass[0]),
            "tree on grass at {pos:?}"
        );
        assert!(!point_in_area(pos, &map.sand[0]), "tree on sand at {pos:?}");
    }
}

/// Дом посреди массива: крона не должна наползать на стену — раньше
/// проверка была только «центр внутри полигона», и дерево вырастало
/// впритык к фасаду (Тула, павильон в парке).
#[test]
fn trees_keep_the_crown_off_walls_and_kerbs() {
    let (sw, _, ne, _) = corners(WOOD_HALF);
    // дом — центральная четверть массива, дорожка режет массив по диагонали
    let map = wood_scene()
        .area(&[("building", "yes")], square(CENTER, WOOD_HALF / 4.0))
        .way(&[("highway", "footway")], vec![sw, ne])
        .parse();

    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.roads.len(), 1);
    assert!(!map.trees.is_empty());
    let house = &map.buildings[0];
    let path = &map.roads[0];
    for &(pos, radius) in &map.trees {
        assert!(
            !point_in_area(pos, house),
            "tree inside the house at {pos:?}"
        );
        assert!(
            !near_area_edge(pos, house, radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE),
            "crown on the wall at {pos:?}"
        );
        let kerb = path
            .points
            .windows(2)
            .map(|segment| distance_to_segment(pos, segment[0], segment[1]))
            .fold(f32::INFINITY, f32::min)
            - path.width / 2.0;
        assert!(kerb > radius, "tree on the kerb at {pos:?}, gap {kerb}");
    }
}

/// Открытая часть парка — поле: деревья растут только в лесных полигонах,
/// парк без `natural=wood` остаётся пустым.
#[test]
fn a_park_without_wood_grows_no_trees() {
    let map = Overpass::new(CITY)
        .area(&[("leisure", "park")], square(CENTER, WOOD_HALF))
        .parse();

    assert_eq!(map.parks.len(), 1);
    assert!(map.woods.is_empty());
    assert!(map.trees.is_empty());
}

#[test]
fn measures_parse_with_units_commas_and_feet() {
    assert_eq!(parse_measure("12"), Some(12.0));
    assert_eq!(parse_measure("12.5"), Some(12.5));
    assert_eq!(parse_measure("12,5"), Some(12.5));
    assert_eq!(parse_measure("12 m"), Some(12.0));
    assert_eq!(parse_measure("12.5 metres"), Some(12.5));
    // несколько значений — берём первое
    assert_eq!(parse_measure("3;4"), Some(3.0));
    assert_eq!(parse_measure("40'"), Some(12.192));
    let inches = parse_measure("40'6\"").unwrap();
    assert!((inches - 12.3444).abs() < 1e-3, "{inches}");
    assert_eq!(parse_measure("tall"), None);
    assert_eq!(parse_measure(""), None);
}

fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

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

#[test]
fn height_prefers_the_metric_tag_then_falls_back_to_levels() {
    // ветка Нью-Йорка: метры из LiDAR-импорта
    assert_eq!(building_height(&tags(&[("height", "31.4")])), Some(31.4));
    // ветка Европы: этажи
    assert_eq!(
        building_height(&tags(&[("building:levels", "5")])),
        Some(15.0)
    );
    // roof:levels по схеме S3DB в building:levels не входит
    assert_eq!(
        building_height(&tags(&[("building:levels", "5"), ("roof:levels", "1")])),
        Some(18.0)
    );
    // проставлены оба — верим метрам, а не пересчёту
    assert_eq!(
        building_height(&tags(&[("height", "20"), ("building:levels", "5")])),
        Some(20.0)
    );
    assert_eq!(building_height(&tags(&[("building", "yes")])), None);
}

#[test]
fn implausible_heights_are_treated_as_missing() {
    assert_eq!(building_height(&tags(&[("height", "0")])), None);
    assert_eq!(building_height(&tags(&[("height", "12000")])), None);
    assert_eq!(building_height(&tags(&[("building:levels", "0")])), None);
    assert_eq!(building_height(&tags(&[("building:levels", "-1")])), None);
    // мусор в метрах не должен глушить этажи
    assert_eq!(
        building_height(&tags(&[("height", "9999"), ("building:levels", "4")])),
        Some(12.0)
    );
}

/// Высота доезжает до `MapData` и из way, и из relation, а на воде её нет.
#[test]
fn parsed_areas_carry_building_height_only() {
    let (sw, se, ne, _) = corners(HALF);
    let map = Overpass::new(CITY)
        .area(
            &[("building", "yes"), ("building:levels", "9")],
            square(CENTER, HALF),
        )
        .relation(
            &[("building", "yes"), ("height", "42 m")],
            &[("outer", closed(vec![sw, se, ne]))],
        )
        .area(
            &[("natural", "water"), ("height", "5")],
            square(CENTER, HALF),
        )
        .parse();

    assert_eq!(map.buildings.len(), 2);
    let heights: Vec<Option<f32>> = map
        .buildings
        .iter()
        .map(|building| building.height)
        .collect();
    assert!(heights.contains(&Some(27.0)), "{heights:?}");
    assert!(heights.contains(&Some(42.0)), "{heights:?}");
    assert_eq!(map.water.len(), 1);
    assert_eq!(map.water[0].height, None);
}

/// Вход-узел контура достаётся своему зданию; `entrance=no` и ворота
/// гаража отбрасываются, а вход в стороне от домов остаётся сиротой.
#[test]
fn entrances_attach_to_the_building_whose_outline_they_sit_on() {
    let (sw, se, ne, nw) = corners(HALF);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], sw)
        .node(&[("entrance", "staircase")], se)
        .node(&[("entrance", "no")], ne)
        .node(&[("entrance", "garage")], nw)
        .node(&[("entrance", "yes")], CENTER)
        .area(&[("building", "yes")], square(CENTER, HALF))
        .parse();

    assert_eq!(map.buildings.len(), 1);
    // main и staircase — на контуре; no и garage отброшены как значения,
    // а «yes» в центре квартала ни одной вершине не соответствует
    let entrances = &map.buildings[0].entrances;
    assert_eq!(entrances.len(), 2, "{entrances:?}");
    for entrance in entrances {
        assert!(
            map.buildings[0]
                .outer
                .iter()
                .any(|vertex| vertex.distance(*entrance) < 0.01),
            "entrance off the outline: {entrance:?}"
        );
    }
}

/// Две ноды `entrance` в одной точке — обычное дело в Париже; на карте
/// они обязаны стать одной дверью, а не двумя одинаковыми целями.
#[test]
fn entrances_at_the_same_point_collapse_into_one() {
    let (sw, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], sw)
        .node(&[("entrance", "yes")], sw)
        .area(&[("building", "yes")], square(CENTER, HALF))
        .parse();

    assert_eq!(map.buildings[0].entrances.len(), 1);
}

/// Здание без размеченных в OSM входов не остаётся без двери: их
/// досочиняет генератор (`entrances/`), иначе в Токио, где размечено
/// 0.9% домов, целей у населения почти не было бы.
#[test]
fn a_building_without_osm_entrances_gets_generated_ones() {
    let map = fixture().parse();
    assert_eq!(map.buildings.len(), 1);
    let building = &map.buildings[0];
    assert!(!building.entrances.is_empty());
    for entrance in &building.entrances {
        let on_outline = (0..building.outer.len()).any(|index| {
            distance_to_segment(
                *entrance,
                building.outer[index],
                building.outer[(index + 1) % building.outer.len()],
            ) < 0.01
        });
        assert!(
            on_outline,
            "generated entrance off the outline: {entrance:?}"
        );
    }
}

/// Кремлёвские постройки — те же здания, но своего вида: их красят иначе.
#[test]
fn kremlin_buildings_classified_by_historic_tag() {
    let map = Overpass::new(CITY)
        .area(
            &[("building", "yes"), ("historic", "citywalls")],
            square(CENTER, HALF),
        )
        .parse();

    assert_eq!(map.buildings[0].kind, AreaKind::Kremlin);
}

/// `natural=tree_row` доезжает до `MapData::tree_rows` и даёт деревья вдоль
/// полилинии. Тег в OSM всегда на way, кольцом почти не бывает — ряд разбирается
/// как открытая полилиния, а не как площадь.
#[test]
fn parses_tree_rows_and_plants_along_them() {
    let map = tree_row(&[("natural", "tree_row")]);

    assert_eq!(map.tree_rows.len(), 1);
    // шага в тегах нет — плотность берётся из ползунка, порог ненулевой
    assert!(map.tree_rows[0].spacing.is_none());
    let rows = map.row_trees.get(TreeRowLayout::default());
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|&(.., at)| at > 0.0));

    // деревья лежат на самом ряду, а не где придётся
    let row = &map.tree_rows[0];
    for &(pos, ..) in rows {
        assert!(distance_to_segment(pos, row.points[0], row.points[1]) < 1.0);
    }

    // и они же — в собранном наборе, который читает рендер
    assert_eq!(map.trees.len(), rows.len());
    assert_eq!(map.composed_for, Some(TreeCompose::default()));
}

/// Шаг из данных: `count` растягивается на длину ряда, а ползунок такой ряд не
/// прореживает — порог нулевой у всех его деревьев.
#[test]
fn tree_row_count_tag_fixes_the_spacing() {
    let map = tree_row(&[("natural", "tree_row"), ("count", "5")]);

    let row = &map.tree_rows[0];
    let length = row.points[0].distance(row.points[1]);
    let spacing = row.spacing.expect("count даёт шаг");
    assert!((spacing - length / 4.0).abs() < 1e-2);

    let rows = map.row_trees.get(TreeRowLayout::default());
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|&(.., at)| at == 0.0));
}

/// Мусорные значения тегов не должны становиться посадкой: `spacing=0.1`
/// смыкает кроны в сплошную кляксу, `count=1` не задаёт шага вовсе.
#[test]
fn implausible_tree_row_tags_fall_back_to_the_slider() {
    for tags in [
        &[("natural", "tree_row"), ("spacing", "0.1")],
        &[("natural", "tree_row"), ("count", "1")],
        &[("natural", "tree_row"), ("diameter_crown", "50")],
    ] {
        let map = tree_row(tags);
        assert!(map.tree_rows[0].spacing.is_none(), "{tags:?}");
        assert!(map.tree_rows[0].radius.is_none(), "{tags:?}");
    }
}

/// Нода `natural=tree` доезжает до `MapData::tree_nodes` и до собранного
/// набора, который читает рендер; `diameter_crown` задаёт радиус кроны, порог
/// нулевой — дерево из данных видно на любой плотности.
#[test]
fn parses_standalone_tree_nodes() {
    let map = Overpass::new(CITY)
        .node(&[("natural", "tree"), ("diameter_crown", "10")], CENTER)
        .parse();

    assert_eq!(map.tree_nodes.len(), 1);
    assert_eq!(map.tree_nodes[0].radius, Some(5.0));
    assert_eq!(map.trees.len(), 1);
    assert_eq!(map.trees[0].1, 5.0);
    assert_eq!(map.tree_appears_at[0], 0.0);
}
