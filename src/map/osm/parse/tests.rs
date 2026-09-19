use super::*;
// посадка деревьев переехала в соседний модуль, но проверяется она через
// весь конвейер — от JSON Overpass до `map.trees`
use super::tags::{building_height, colour, parse_measure};
use crate::map::osm::fixture::{Overpass, building, closed, rect, square, street, water_area};
use crate::map::osm::model::{
    BuildingUse, Colours, FenceKind, PitchKind, RailKind, Sacred, SacredForm, ServiceTrack,
    StructureKind, WaterKind, distance_to_segment, is_big_box,
};
use crate::map::osm::planting::{
    TREE_CROWN_REACH, TREE_MIN_SPACING, TREE_SHORE_CLEARANCE, TREE_WALL_CLEARANCE, near_area_edge,
};
use crate::settings::MAP_SIZE;

/// Фикстуры строятся вокруг гео-центра Тулы — города по умолчанию.
const CITY: City = City::Tula;

/// Храм, чья вера досталась ему от города без размеченных храмов.
const WESTERN_CHURCH: BuildingUse = BuildingUse::Church(Sacred {
    faith: Faith::Western,
    form: SacredForm::Nave,
    complex: 0,
    floor_dm: 0,
});

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

/// Сторона движения — с границы, в которой лежит карта: так её отдаёт
/// `out tags` после `is_in`, relation без членов. Из нескольких границ
/// побеждает самая мелкая, без тега — правостороннее.
#[test]
fn takes_the_driving_side_from_the_innermost_boundary() {
    let country = |side| {
        [
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("driving_side", side),
        ]
    };
    let left = Overpass::new(CITY).relation(&country("left"), &[]).parse();
    assert_eq!(left.traffic_side, TrafficSide::Left);

    // регион после страны и перед ней: порядок в ответе ничего не решает
    let region = [
        ("boundary", "administrative"),
        ("admin_level", "4"),
        ("driving_side", "left"),
    ];
    let after = Overpass::new(CITY)
        .relation(&country("right"), &[])
        .relation(&region, &[])
        .parse();
    assert_eq!(after.traffic_side, TrafficSide::Left);
    let before = Overpass::new(CITY)
        .relation(&region, &[])
        .relation(&country("right"), &[])
        .parse();
    assert_eq!(before.traffic_side, TrafficSide::Left);

    let untagged = Overpass::new(CITY)
        .relation(&[("boundary", "administrative"), ("admin_level", "2")], &[])
        .parse();
    assert_eq!(untagged.traffic_side, TrafficSide::Right);
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
fn service_tracks_are_the_siding_the_yard_and_the_spur() {
    let (sw, se, ne, nw) = corners(HALF);
    // станционные пути белого списка, съезд между главными ходами и сам
    // главный ход без `service` вовсе
    let map = Overpass::new(CITY)
        .way(&[("railway", "rail"), ("service", "siding")], vec![sw, ne])
        .way(&[("railway", "rail"), ("service", "yard")], vec![sw, se])
        .way(&[("railway", "rail"), ("service", "spur")], vec![se, nw])
        .way(
            &[("railway", "rail"), ("service", "crossover")],
            vec![nw, ne],
        )
        .way(&[("railway", "rail")], vec![sw, nw])
        .parse();

    let service: Vec<Option<ServiceTrack>> = map.rails.iter().map(|rail| rail.service).collect();
    // белый список, а не «тег есть»: на съезде между главными ходами состав
    // не бросают, и стоянка вагонов туда не приходит
    assert_eq!(
        service,
        vec![
            Some(ServiceTrack::Siding),
            Some(ServiceTrack::Yard),
            Some(ServiceTrack::Spur),
            None,
            None
        ]
    );
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
    for &(pos, radius) in first.trees.positions() {
        assert!(point_in_area(pos, &first.woods[0]), "{pos:?}");
        assert!((2.5..=4.0).contains(&radius));
    }
    for (index, &(pos, _)) in first.trees.positions().iter().enumerate() {
        for &(other, _) in &first.trees.positions()[index + 1..] {
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
    for &(pos, _) in map.trees.positions() {
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
fn landuse_blocks_are_their_own_layer_and_never_win_over_green() {
    let (sw, _, ne, _) = corners(HALF);
    let map = Overpass::new(CITY)
        .area(&[("landuse", "residential")], square(CENTER, HALF))
        .area(&[("landuse", "garages")], square(CENTER, HALF / 2.0))
        .relation(
            &[("landuse", "industrial")],
            &[("outer", closed(vec![sw, Vec2::new(ne.x, sw.y), ne]))],
        )
        // зелень с тем же тегом остаётся зеленью
        .area(&[("landuse", "forest")], square(CENTER, HALF / 4.0))
        .area(&[("landuse", "grass")], square(CENTER, HALF / 8.0))
        .parse();

    let kinds: Vec<AreaKind> = map.landuse.iter().map(|area| area.kind).collect();
    assert_eq!(
        kinds,
        [
            AreaKind::Residential,
            AreaKind::Industrial,
            AreaKind::Industrial
        ]
    );
    assert_eq!(map.woods.len(), 1);
    assert_eq!(map.grass.len(), 1);
    assert!(map.buildings.is_empty() && map.parks.is_empty());
}

#[test]
fn a_parking_lot_is_its_own_layer_and_a_parking_house_stays_a_building() {
    let map = Overpass::new(CITY)
        .area(&[("amenity", "parking")], square(CENTER, HALF))
        // парковочный дом — здание: `building` проверяется раньше
        .area(
            &[("building", "yes"), ("amenity", "parking")],
            square(CENTER, HALF / 4.0),
        )
        // сквер внутри стоянки — отдельный контур и остаётся сквером
        .area(&[("leisure", "park")], square(CENTER, HALF / 2.0))
        .parse();

    assert_eq!(map.parking.len(), 1);
    assert_eq!(map.parking[0].kind, AreaKind::Parking);
    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.parks.len(), 1);
    // в кварталы стоянка не падает: до ветки `landuse` дело не доходит
    assert!(map.landuse.is_empty());
}

#[test]
fn an_underground_car_park_is_not_asphalt_over_the_lawn() {
    let map = Overpass::new(CITY)
        // подземный паркинг нарисован своим контуром под сквером, без `building`
        .area(
            &[("amenity", "parking"), ("parking", "underground")],
            square(CENTER, HALF / 2.0),
        )
        .area(&[("leisure", "park")], square(CENTER, HALF))
        // многоуровневый паркинг без `building` — тот же случай, только над двором
        .area(
            &[("amenity", "parking"), ("parking", "multi-storey")],
            square(CENTER + Vec2::splat(HALF * 2.0), HALF / 2.0),
        )
        .parse();

    assert!(map.parking.is_empty());
    assert_eq!(map.parks.len(), 1);
    // контур ушёл дальше по цепочке и без `landuse` не нарисовался вовсе
    assert!(map.landuse.is_empty());
}

/// Цилиндры промзоны приезжают и нодой, и way. Way с `building=yes` при этом
/// становится **только** цилиндром: труба, размеченная как здание, — тот же
/// самый объект, и коробка под кругом была бы им обоим сразу.
#[test]
fn a_man_made_cylinder_arrives_instead_of_a_box() {
    let map = Overpass::new(CITY)
        // нода: размера в данных нет, берётся типовой для рода
        .node(&[("man_made", "chimney")], CENTER)
        // нода с тегом высоты — тег важнее типового
        .node(&[("man_made", "water_tower"), ("height", "42")], CENTER)
        // way, размеченный ещё и зданием
        .area(
            &[("man_made", "storage_tank"), ("building", "yes")],
            square(CENTER, 10.0),
        )
        // `man_made=*` носит и всякое, что кругом на снимке не читается
        .node(&[("man_made", "surveillance")], CENTER)
        .area(&[("man_made", "works")], square(CENTER, HALF))
        .parse();

    let kinds: Vec<StructureKind> = map
        .structures
        .iter()
        .map(|structure| structure.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            StructureKind::Chimney,
            StructureKind::WaterTower,
            StructureKind::Tank
        ]
    );
    assert!(
        map.buildings.is_empty(),
        "цилиндр не должен становиться ещё и коробкой"
    );

    let chimney = map.structures[0];
    assert_eq!(chimney.height, 60.0, "типовая высота заводской трубы");
    assert_eq!(map.structures[1].height, 42.0, "тег важнее типовой высоты");
    // радиус way считается по контуру: у квадрата со стороной 20 м среднее
    // расстояние до вершин — половина диагонали
    let tank = map.structures[2];
    assert!(
        (tank.radius - 10.0 * 2.0_f32.sqrt()).abs() < 0.5,
        "{tank:?}"
    );
}

/// Радиус ноды берётся из `diameter` (он же `width`: цилиндр меряют поперёк),
/// а неправдоподобный тег считается отсутствующим — контура у ноды нет,
/// сверить размер не с чем, и типовой радиус рода честнее круга в гектар.
#[test]
fn a_node_cylinder_takes_its_radius_from_the_diameter_tag() {
    let map = Overpass::new(CITY)
        .node(&[("man_made", "water_tower"), ("diameter", "12")], CENTER)
        .node(&[("man_made", "water_tower"), ("width", "12 m")], CENTER)
        // габарит площадки, записанный в `diameter`, — мимо диапазона
        .node(&[("man_made", "water_tower"), ("diameter", "260")], CENTER)
        .parse();

    let (typical, _) = structure_size(StructureKind::WaterTower);
    let radii: Vec<f32> = map
        .structures
        .iter()
        .map(|structure| structure.radius)
        .collect();
    assert_eq!(radii, [6.0, 6.0, typical]);
}

/// До карты доезжает только **надземный** трубопровод: правило обратное тому,
/// что у путей и водотоков, потому что труба без `location` в OSM закопана, а
/// серебристая линия через весь город по закопанной трубе — враньё крупнее,
/// чем потерянная эстакада без тега.
#[test]
fn only_an_overground_pipeline_reaches_the_map() {
    let (sw, se, ne, nw) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[
                ("man_made", "pipeline"),
                ("location", "overground"),
                ("count", "4"),
            ],
            vec![sw, ne],
        )
        // эстакада над улицей: way несёт оба тега, и оба обязаны доехать
        .way(
            &[
                ("man_made", "pipeline"),
                ("location", "overhead"),
                ("highway", "residential"),
            ],
            vec![nw, ne],
        )
        // закопанные — мимо: и явно, и по умолчанию
        .way(&[("man_made", "pipeline")], vec![se, nw])
        .way(
            &[("man_made", "pipeline"), ("location", "underground")],
            vec![sw, se],
        )
        .parse();

    assert_eq!(map.pipes.len(), 2);
    assert!(
        map.pipes[0].width > map.pipes[1].width,
        "четвёрка труб шире пары"
    );
    assert_eq!(map.roads.len(), 1, "эстакада не должна съедать улицу");
}

/// Площадки разбираются по `leisure`, а поле — по `sport`/`surface`. Парк со
/// спортивной площадкой на нём остаётся парком: поле приезжает своим way.
#[test]
fn a_leisure_ground_becomes_a_pitch_of_its_own_kind() {
    let map = Overpass::new(CITY)
        .area(
            &[("leisure", "pitch"), ("sport", "soccer")],
            square(CENTER, HALF),
        )
        .area(
            &[("leisure", "pitch"), ("sport", "basketball")],
            square(CENTER, HALF),
        )
        // без вида спорта решает покрытие, а без покрытия — «коробка»
        .area(
            &[("leisure", "pitch"), ("surface", "grass")],
            square(CENTER, HALF),
        )
        .area(&[("leisure", "pitch")], square(CENTER, HALF))
        .area(&[("leisure", "playground")], square(CENTER, HALF))
        .area(&[("leisure", "track")], square(CENTER, HALF))
        // не площадка вовсе: значение вне белого списка
        .area(&[("leisure", "bandstand")], square(CENTER, HALF))
        .area(&[("leisure", "park")], square(CENTER, HALF))
        .parse();

    let kinds: Vec<AreaKind> = map.pitches.iter().map(|area| area.kind).collect();
    assert_eq!(
        kinds,
        [
            AreaKind::Pitch(PitchKind::Soccer),
            AreaKind::Pitch(PitchKind::Hard),
            AreaKind::Pitch(PitchKind::Soccer),
            AreaKind::Pitch(PitchKind::Hard),
            AreaKind::Pitch(PitchKind::Playground),
            AreaKind::Pitch(PitchKind::Track),
        ]
    );
    assert_eq!(map.parks.len(), 1);
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
    for &(pos, _) in map.trees.positions() {
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
    for &(pos, radius) in map.trees.positions() {
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

/// Ограды участков доезжают до `MapData::fences` и не смешиваются со стеной
/// Кремля: та непроходима и лежит в `walls`.
#[test]
fn a_barrier_becomes_a_fence_but_the_city_wall_stays_a_wall() {
    let (sw, _, ne, _) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(&[("barrier", "fence")], vec![sw, ne])
        .way(&[("barrier", "wall")], vec![sw, ne])
        .way(&[("barrier", "retaining_wall")], vec![sw, ne])
        .way(&[("barrier", "hedge")], vec![sw, ne])
        .way(&[("barrier", "city_wall")], vec![sw, ne])
        // не линия и не ограда: калитка и бордюр
        .way(&[("barrier", "gate")], vec![sw, ne])
        .way(&[("barrier", "kerb")], vec![sw, ne])
        .parse();

    let kinds: Vec<FenceKind> = map.fences.iter().map(|fence| fence.kind).collect();
    assert_eq!(
        kinds,
        [
            FenceKind::Fence,
            FenceKind::Wall,
            FenceKind::Wall,
            FenceKind::Hedge
        ]
    );
    assert_eq!(map.walls.len(), 1);
}

/// Ветка ограды не прерывает разбор way: обнесённый забором квартал обязан
/// стать и оградой, и кварталом — с `return` там Тула теряла площади.
#[test]
fn a_fenced_block_becomes_both_a_fence_and_a_quarter() {
    let map = Overpass::new(CITY)
        .area(
            &[("barrier", "fence"), ("landuse", "residential")],
            square(CENTER, HALF),
        )
        .parse();

    assert_eq!(map.fences.len(), 1);
    assert_eq!(map.landuse.len(), 1);
}

/// Ветка ограды стоит **выше** дорожной: забор вдоль тропы висит на том же
/// way, что и `highway=*`, а дорожная ветка разбор прерывает — ниже неё её
/// `return` съедал бы такой забор целиком (в Париже и Лондоне по одному
/// такому way). Один way — и дорожка, и ограда.
#[test]
fn a_fenced_path_becomes_both_an_alley_and_a_fence() {
    let (sw, se, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[("highway", "footway"), ("barrier", "fence")],
            vec![sw, se],
        )
        .parse();

    assert_eq!(map.roads.len(), 1, "дорожка");
    let kinds: Vec<FenceKind> = map.fences.iter().map(|fence| fence.kind).collect();
    assert_eq!(kinds, [FenceKind::Fence], "ограда");
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
    assert_eq!(plain.lanes, None, "без тега — дефолт у рендера");

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

    assert_eq!(road(&[("lanes", "4")]).lanes, Some(4));
    assert_eq!(road(&[("lanes", "2;3")]).lanes, Some(2));
    assert_eq!(road(&[("lanes", "2.5")]).lanes, Some(2));
    assert_eq!(road(&[("lanes", "0")]).lanes, None);
    assert_eq!(
        road(&[("lanes", "12")]).lanes,
        None,
        "за восемью полосами — не лента, а вся развязка"
    );
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

/// Высота по одним тегам: дом без назначения и без контура — та ветка,
/// которая у [`building_height`] была единственной до торговой коробки.
fn height(pairs: &[(&str, &str)]) -> Option<f32> {
    building_height(&tags(pairs), BuildingUse::Other, &[])
}

/// Высота торгового здания с пятном: квадрат такой площади вокруг начала
/// координат. Назначение берётся из самих тегов, как в разборе.
fn retail_height(pairs: &[(&str, &str)], area: f32) -> Option<f32> {
    let side = area.sqrt() / 2.0;
    let ring = [
        Vec2::new(-side, -side),
        Vec2::new(side, -side),
        Vec2::new(side, side),
        Vec2::new(-side, side),
    ];
    let tags = tags(pairs);
    building_height(
        &tags,
        super::tags::area_use(AreaKind::Building, &tags),
        &ring,
    )
}

#[test]
fn height_prefers_the_metric_tag_then_falls_back_to_levels() {
    // ветка Нью-Йорка: метры из LiDAR-импорта
    assert_eq!(height(&[("height", "31.4")]), Some(31.4));
    // ветка Европы: этажи
    assert_eq!(height(&[("building:levels", "5")]), Some(15.0));
    // roof:levels по схеме S3DB в building:levels не входит
    assert_eq!(
        height(&[("building:levels", "5"), ("roof:levels", "1")]),
        Some(18.0)
    );
    // проставлены оба — верим метрам, а не пересчёту
    assert_eq!(
        height(&[("height", "20"), ("building:levels", "5")]),
        Some(20.0)
    );
    assert_eq!(height(&[("building", "yes")]), None);
}

#[test]
fn implausible_heights_are_treated_as_missing() {
    assert_eq!(height(&[("height", "0")]), None);
    assert_eq!(height(&[("height", "12000")]), None);
    assert_eq!(height(&[("building:levels", "0")]), None);
    assert_eq!(height(&[("building:levels", "-1")]), None);
    // мусор в метрах не должен глушить этажи
    assert_eq!(
        height(&[("height", "9999"), ("building:levels", "4")]),
        Some(12.0)
    );
    // и у торговой коробки тоже: прибавка оболочки подняла бы ноль уровней до
    // правдоподобных 3.5 м, то есть нарисовала бы гипермаркет плитой в один
    // рост вместо того, чтобы отдать его выводу этажности
    assert_eq!(
        retail_height(&[("building", "retail"), ("building:levels", "0")], 9055.0),
        None
    );
    assert_eq!(
        retail_height(&[("building", "retail"), ("building:levels", "-1")], 9055.0),
        None
    );
}

/// Торговый уровень выше жилого этажа, и сверх него идёт техэтаж с парапетом:
/// `building:levels=1` у гипермаркета — это не трёхметровый дом. Ровно из-за
/// этого стодвадцатиметровый «Магнит» (`building=commercial`, `shop=supermarket`,
/// 9055 м², один уровень) читался гигантским одноэтажным жилым домом.
#[test]
fn a_trading_level_is_taller_than_a_dwelling_storey() {
    let magnit = &[
        ("building", "commercial"),
        ("shop", "supermarket"),
        ("building:levels", "1"),
    ];
    assert_eq!(retail_height(magnit, 9055.0), Some(8.0));
    // ТРЦ «Макси»: `building=yes` + `shop=mall`, два уровня
    let maxi = &[
        ("building", "yes"),
        ("shop", "mall"),
        ("building:levels", "2"),
    ];
    assert_eq!(retail_height(maxi, 64257.0), Some(12.5));
    // та же разметка на мелком пятне — магазин во встройке, обычный этаж
    assert_eq!(retail_height(maxi, 295.0), Some(6.0));
}

/// `shop=*` на девятиэтажке описывает не здание, а магазин на его первом
/// этаже: тульская «Пятёрочка» размечена `building=retail` + `levels=9` на
/// весь дом, и торговым уровнем это вышло бы сорокапятиметровой башней.
#[test]
fn a_shop_on_a_tall_block_is_measured_in_dwelling_storeys() {
    let pyaterochka = &[
        ("building", "retail"),
        ("shop", "supermarket"),
        ("building:levels", "9"),
    ];
    assert_eq!(retail_height(pyaterochka, 1528.0), Some(27.0));
}

/// И этим дело не кончается: дом, которому разбор отказал в торговом уровне,
/// не коробка и в отрисовке. Порог этажности стоял только в высоте, а всё
/// остальное — глухая кассета с фризом, ярус 5.5 м, решётка зенитных фонарей и
/// входные группы через 55 м — висело на одном пятне, и девятиэтажная
/// «Пятёрочка» получала их все. Два ответа на один вопрос сходятся теперь по
/// построению, и проверяются они вместе — через весь разбор, а не по тегам.
#[test]
fn a_shop_on_a_tall_block_is_not_a_big_box_either() {
    let block = |levels: &str| {
        Overpass::new(CITY)
            .area(
                &[
                    ("building", "retail"),
                    ("shop", "supermarket"),
                    ("building:levels", levels),
                ],
                square(CENTER, HALF),
            )
            .parse()
            .buildings
            .remove(0)
    };

    let tall = block("9");
    assert_eq!(tall.height, Some(27.0));
    assert!(!is_big_box(&tall), "девятиэтажка с магазином — не коробка");

    // тот же дом в два торговых уровня — коробка, и мерена она оболочкой
    let shell = block("2");
    assert_eq!(shell.height, Some(12.5));
    assert!(is_big_box(&shell));
}

/// А высотой этого не решить, и потому этажность доезжает до модели отдельным
/// полем. ТЦ «Империя» (`building=yes` + `shop=mall`, 1781 м², четыре этажа)
/// меряется жилым этажом и стоит в двенадцати метрах; ТРЦ «Макси»
/// (`building=yes` + `shop=mall`, 64 257 м², два торговых уровня) — в
/// двенадцати с половиной. Любой потолок по `area.height` либо оставит
/// коробкой четырёхэтажный ТЦ, либо отнимет коробку у настоящей: между ними
/// полметра, и различает их только `building:levels`.
#[test]
fn four_storeys_of_mall_are_not_a_two_level_box() {
    // 42 × 42 м — 1781 м² «Империи», пятно крупноформата с запасом над 1200
    let mall = |levels: &str| {
        Overpass::new(CITY)
            .area(
                &[
                    ("building", "yes"),
                    ("shop", "mall"),
                    ("building:levels", levels),
                ],
                square(CENTER, 21.1),
            )
            .parse()
            .buildings
            .remove(0)
    };

    let imperia = mall("4");
    assert_eq!(imperia.storeys, Some(4.0));
    assert_eq!(imperia.height, Some(12.0));
    assert!(!is_big_box(&imperia), "четырёхэтажный ТЦ — не коробка");

    // «Макси»: те же двенадцать метров с небольшим, но это два торговых уровня
    let maxi = mall("2");
    assert_eq!(maxi.storeys, Some(2.0));
    assert_eq!(maxi.height, Some(12.5));
    assert!(is_big_box(&maxi), "двухуровневая коробка — коробка");

    // этажей не разметили — ответ прежний, по потолку высоты («Верный»,
    // ТЦ «Перспектива»: пятно есть, тегов высоты нет вовсе)
    let untagged = Overpass::new(CITY)
        .area(
            &[("building", "retail"), ("shop", "supermarket")],
            square(CENTER, 21.1),
        )
        .parse()
        .buildings
        .remove(0);
    assert_eq!(untagged.storeys, None);
    assert_eq!(untagged.height, None);
    assert!(is_big_box(&untagged));
}

/// Высота по одним тегам приходит без контура (`tagged_height` для
/// `is_fortification`), и пустое кольцо не должно ронять разбор: площадь
/// кольца считает `len() - 1` по `usize`. Раньше срез спасал только порядок
/// сомножителей в `&&`.
#[test]
fn an_empty_ring_is_not_a_big_box_and_does_not_panic() {
    let retail = tags(&[("building", "retail"), ("building:levels", "2")]);
    assert_eq!(
        building_height(&retail, BuildingUse::Retail, &[]),
        Some(6.0)
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
    // а «yes» в центре квартала ни одной вершине не соответствует.
    //
    // Считается тут **разбор**, а не итоговое число дверей: к размеченным
    // генератор дописывает недостающие по когорте (`entrances/`), и на
    // отброшенных значениях это не сказывается никак.
    let entrances = &map.buildings[0].entrances;
    let at = |point: Vec2| entrances.iter().any(|door| door.distance(point) < 0.01);
    assert!(at(sw) && at(se), "{entrances:?}");
    assert!(!at(ne) && !at(nw), "отброшенное значение стало дверью");
    assert!(
        !entrances.iter().any(|door| door.distance(CENTER) < 0.01),
        "вход в стороне от контура достался зданию"
    );
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

    let doubled = map.buildings[0]
        .entrances
        .iter()
        .filter(|door| door.distance(sw) < 0.01)
        .count();
    assert_eq!(doubled, 1, "{:?}", map.buildings[0].entrances);
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

/// Назначение здания — класс отрисовки: `building=*` напрямую, а когда его
/// значение вне словаря (безликое `yes`, но и `construction`) — по
/// `amenity=*` того же контура. Вода назначения не имеет, даже если тег
/// на ней стоит.
#[test]
fn building_use_comes_from_the_building_tag_or_amenity_outside_the_vocabulary() {
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], square(CENTER, HALF))
        .area(&[("building", "apartments")], square(CENTER, HALF))
        .area(&[("building", "garages")], square(CENTER, HALF))
        .area(&[("building", "garage")], square(CENTER, HALF))
        .area(
            &[("building", "yes"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "construction"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("amenity", "place_of_worship")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "church"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        // в стороне: лёжа на храме, контур без назначения стал бы его пристройкой
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(400.0, 0.0), HALF),
        )
        .area(
            &[("natural", "water"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .parse();

    // посев храма зависит от вершин контура — здесь сравниваются только классы
    let uses: Vec<BuildingUse> = map
        .buildings
        .iter()
        .map(|b| match b.building_use {
            BuildingUse::Church(sacred) => BuildingUse::Church(Sacred {
                complex: 0,
                ..sacred
            }),
            other => other,
        })
        .collect();
    assert_eq!(
        uses,
        [
            BuildingUse::House,
            BuildingUse::Apartments,
            // множественное число — кооператив целиком, единственное — бокс
            BuildingUse::GarageBlock,
            BuildingUse::Garage,
            BuildingUse::Public,
            // значение вне словаря отдаёт слово `amenity` так же, как `yes`
            BuildingUse::Public,
            // вера не размечена ни у одного храма города — западная
            // (`parse::resolve_faiths`)
            WESTERN_CHURCH,
            // `building=*` со смыслом сильнее `amenity`
            WESTERN_CHURCH,
            BuildingUse::Other,
        ]
    );
    assert_eq!(map.water[0].building_use, BuildingUse::Other);
}

/// Здание, которое **и есть магазин**, узнаётся по `building=*` и, когда там
/// безликое `yes`, по крупноформатному `shop=*` того же контура. Мелкая
/// торговля по белому списку не проходит: булочная и «продукты» в OSM стоят
/// на чужом доме, и назначение у него своё.
///
/// Без ветки `shop` половина тульских ТЦ (ТРЦ «Макси» — `building=yes` +
/// `shop=mall`, 64 тысячи м²) разбиралась как «полгорода без назначения».
///
/// `building=shop` стоит в торговой ветке, а не в конторской: в OSM это прямой
/// синоним `building=retail`. Значение конкретное, поэтому мелкий `shop=*` на
/// том же контуре его уже не перебивает — уточнять белому списку нечего.
#[test]
fn a_shop_building_is_read_from_the_shop_tag_too() {
    let map = Overpass::new(CITY)
        .area(&[("building", "retail")], square(CENTER, HALF))
        .area(&[("building", "supermarket")], square(CENTER, HALF))
        // `building=shop` — то же самое здание-магазин, что и `retail`
        .area(&[("building", "shop")], square(CENTER, HALF))
        .area(
            &[("building", "shop"), ("shop", "convenience")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "mall")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "doityourself")],
            square(CENTER, HALF),
        )
        // `commercial` — «коммерческое здание вообще», и `shop` его уточняет:
        // так размечен «Магнит»
        .area(
            &[("building", "commercial"), ("shop", "supermarket")],
            square(CENTER, HALF),
        )
        // …а `apartments` называет другой дом, и супермаркет на его первом
        // этаже этого не отменяет: так размечена тульская «Пятёрочка»
        .area(
            &[("building", "apartments"), ("shop", "supermarket")],
            square(CENTER, HALF),
        )
        // булочная на чужом доме магазином его не делает
        .area(
            &[("building", "apartments"), ("shop", "bakery")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "convenience")],
            square(CENTER, HALF),
        )
        // контора и павильон остаются торговлей вообще, а не магазином
        .area(&[("building", "commercial")], square(CENTER, HALF))
        .area(&[("building", "office")], square(CENTER, HALF))
        .parse();

    let uses: Vec<BuildingUse> = map.buildings.iter().map(|b| b.building_use).collect();
    assert_eq!(
        uses,
        [
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Apartments,
            BuildingUse::Apartments,
            BuildingUse::Other,
            BuildingUse::Commercial,
            BuildingUse::Commercial,
        ]
    );
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
    assert_eq!(map.trees.positions()[0].1, 5.0);
    assert_eq!(map.trees.appears_at(0), 0.0);
}

/// Вера храма: по `religion` с `denomination`, а без них — по тегу здания.
/// Колокольня — башня храма, барабан с `roof:shape=onion` — глава.
#[test]
fn a_place_of_worship_carries_its_faith_and_its_form() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "russian_orthodox"),
            ],
            square(CENTER, HALF),
        )
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "catholic"),
            ],
            square(CENTER + Vec2::new(300.0, 0.0), HALF),
        )
        .area(
            &[("building", "mosque")],
            square(CENTER + Vec2::new(600.0, 0.0), HALF),
        )
        .area(
            &[
                ("building", "yes"),
                ("amenity", "place_of_worship"),
                ("religion", "jewish"),
            ],
            square(CENTER + Vec2::new(900.0, 0.0), HALF),
        )
        .area(
            &[("building", "temple"), ("religion", "buddhist")],
            square(CENTER + Vec2::new(1200.0, 0.0), HALF),
        )
        .area(
            &[
                ("building", "yes"),
                ("man_made", "tower"),
                ("tower:type", "bell_tower"),
            ],
            square(CENTER + Vec2::new(0.0, 300.0), 5.0),
        )
        .area(
            &[("building", "cathedral"), ("roof:shape", "onion")],
            square(CENTER + Vec2::new(0.0, -300.0), 5.0),
        )
        .parse();

    let sacred: Vec<Sacred> = map
        .buildings
        .iter()
        .map(|building| match building.building_use {
            BuildingUse::Church(sacred) => sacred,
            other => panic!("not a church: {other:?}"),
        })
        .collect();
    let faiths: Vec<Faith> = sacred.iter().map(|s| s.faith).collect();
    // башня и глава своей веры не несут, храма вокруг них нет — их вера это
    // большинство города; православный и католический тут поровну, и при
    // равенстве берётся западная
    assert_eq!(
        faiths,
        [
            Faith::Orthodox,
            Faith::Western,
            Faith::Muslim,
            Faith::Jewish,
            Faith::Eastern,
            Faith::Western,
            Faith::Western,
        ]
    );
    assert_eq!(sacred[0].form, SacredForm::Nave);
    assert_eq!(sacred[5].form, SacredForm::Tower);
    assert_eq!(sacred[6].form, SacredForm::Dome);
}

/// Часть храма без веры берёт её у храма, в котором стоит, — а не у города.
#[test]
fn a_part_of_a_church_takes_the_faith_of_its_church() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "cathedral"),
                ("amenity", "place_of_worship"),
                ("religion", "christian"),
                ("denomination", "russian_orthodox"),
            ],
            square(CENTER, 20.0),
        )
        .area(
            &[("building", "cathedral"), ("roof:shape", "onion")],
            square(CENTER, 5.0),
        )
        // два западных храма в городе: большинство — не православное
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "catholic"),
            ],
            square(CENTER + Vec2::new(400.0, 0.0), 20.0),
        )
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "lutheran"),
            ],
            square(CENTER + Vec2::new(800.0, 0.0), 20.0),
        )
        .parse();
    let (BuildingUse::Church(host), BuildingUse::Church(part)) =
        (map.buildings[0].building_use, map.buildings[1].building_use)
    else {
        panic!("both are churches");
    };
    assert_eq!(part.faith, Faith::Orthodox);
    assert_eq!(part.form, SacredForm::Dome);
    // и красится часть вместе со своим храмом
    assert_eq!(part.complex, host.complex);
    let BuildingUse::Church(stranger) = map.buildings[2].building_use else {
        panic!("a church");
    };
    assert_ne!(stranger.complex, host.complex);
}

/// Колокольня рядом с храмом, а не внутри него, — тоже его часть.
#[test]
fn a_bell_tower_beside_a_church_is_painted_with_it() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "russian_orthodox"),
            ],
            square(CENTER, 20.0),
        )
        .area(
            &[("building", "yes"), ("tower:type", "bell_tower")],
            square(CENTER + Vec2::new(0.0, 30.0), 5.0),
        )
        .parse();
    let (BuildingUse::Church(church), BuildingUse::Church(tower)) =
        (map.buildings[0].building_use, map.buildings[1].building_use)
    else {
        panic!("both are churches");
    };
    assert_eq!(tower.complex, church.complex);
    assert_eq!(tower.faith, Faith::Orthodox);
}

/// Колокольня, ближе всего стоящая к части собора, а не к нему самому, всё
/// равно красится с собором: хозяин части — это хозяин её хозяина.
#[test]
fn a_bell_tower_beside_a_part_of_a_cathedral_is_painted_with_the_cathedral() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "cathedral"),
                ("religion", "christian"),
                ("denomination", "russian_orthodox"),
            ],
            square(CENTER, 20.0),
        )
        // часть собора: центр внутри, край выходит за его стену на 6 м
        .area(
            &[("building", "cathedral")],
            square(CENTER + Vec2::new(0.0, 18.0), 8.0),
        )
        // колокольня: до части 14 м, до собора 20
        .area(
            &[("building", "yes"), ("tower:type", "bell_tower")],
            square(CENTER + Vec2::new(0.0, 40.0), 3.0),
        )
        // два западных храма в городе: вера большинства — не православная
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "catholic"),
            ],
            square(CENTER + Vec2::new(400.0, 0.0), 20.0),
        )
        .area(
            &[
                ("building", "church"),
                ("religion", "christian"),
                ("denomination", "lutheran"),
            ],
            square(CENTER + Vec2::new(800.0, 0.0), 20.0),
        )
        .parse();
    let church = |index: usize| match map.buildings[index].building_use {
        BuildingUse::Church(sacred) => sacred,
        _ => panic!("a church"),
    };
    let (cathedral, part, tower) = (church(0), church(1), church(2));
    assert_eq!(part.complex, cathedral.complex);
    assert_eq!(tower.complex, cathedral.complex);
    assert_eq!(tower.faith, Faith::Orthodox);
}

/// Обычный контур, лежащий на храме, — его пристройка; квартал вокруг храма
/// во дворе ею не становится, и соседний дом тоже. Высота начала части — из
/// `min_height`, а без него из `building:min_level`.
#[test]
fn a_building_lying_on_a_church_becomes_its_annex() {
    let church_tags = [
        ("building", "cathedral"),
        ("religion", "christian"),
        ("denomination", "russian_orthodox"),
    ];
    let map = Overpass::new(CITY)
        .area(&church_tags, square(CENTER, 18.0))
        // музей в здании собора: чуть больше и тот же центр
        .area(
            &[("building", "yes"), ("tourism", "museum")],
            square(CENTER, 21.0),
        )
        // пристройка, чей центр внутри собора
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(15.0, 0.0), 8.0),
        )
        // квартал с храмом во дворе — в десятки раз крупнее
        .area(&[("building", "apartments")], square(CENTER, 120.0))
        // соседний дом через дорогу
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(60.0, 0.0), 10.0),
        )
        .area(
            &[
                ("building", "cathedral"),
                ("roof:shape", "onion"),
                ("min_height", "20"),
            ],
            square(CENTER, 4.0),
        )
        .area(
            &[
                ("building", "cathedral"),
                ("roof:shape", "dome"),
                ("building:min_level", "2"),
            ],
            square(CENTER + Vec2::new(8.0, 8.0), 4.0),
        )
        // алтарная часть: заходит в храм на 40 % своего пятна, но оба центра
        // снаружи друг друга
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(0.0, 20.0), 10.0),
        )
        .parse();

    let form = |index: usize| match map.buildings[index].building_use {
        BuildingUse::Church(sacred) => Some(sacred.form),
        _ => None,
    };
    assert_eq!(form(1), Some(SacredForm::Annex));
    assert_eq!(form(2), Some(SacredForm::Annex));
    assert_eq!(form(7), Some(SacredForm::Annex));
    assert_eq!(form(3), None);
    assert_eq!(form(4), None);
    let BuildingUse::Church(museum) = map.buildings[1].building_use else {
        panic!("an annex");
    };
    let BuildingUse::Church(cathedral) = map.buildings[0].building_use else {
        panic!("a church");
    };
    assert_eq!(museum.complex, cathedral.complex);
    assert_eq!(museum.faith, Faith::Orthodox);

    let floor = |index: usize| match map.buildings[index].building_use {
        BuildingUse::Church(sacred) => sacred.floor(),
        _ => f32::NAN,
    };
    assert_eq!(floor(5), 20.0);
    assert_eq!(floor(6), 6.0);
}

/// Крепость узнаётся не только по `historic`: у Тульского кремля стена —
/// `building=wall`, башни — `man_made=tower` с `tower:type=defensive`. Низкая
/// `building=wall` — ограда, и кремлём не становится.
#[test]
fn a_fortress_is_told_by_its_wall_and_its_defensive_towers() {
    let map = Overpass::new(CITY)
        .area(
            &[("building", "wall"), ("height", "12.7")],
            square(CENTER, HALF),
        )
        .area(
            &[
                ("building", "yes"),
                ("man_made", "tower"),
                ("tower:type", "defensive"),
            ],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("historic", "citywalls")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "wall"), ("height", "2")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("man_made", "tower")],
            square(CENTER, HALF),
        )
        .parse();
    let kinds: Vec<AreaKind> = map.buildings.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            AreaKind::Kremlin,
            AreaKind::Kremlin,
            AreaKind::Kremlin,
            AreaKind::Building,
            AreaKind::Building,
        ]
    );
}

/// Ромб частного дома из Тулы (way 968419942, углы 79°–100°) вокруг `at`.
fn skewed_house(at: Vec2) -> Vec<Vec2> {
    [
        Vec2::new(412.56, 3158.32),
        Vec2::new(426.85, 3167.85),
        Vec2::new(420.76, 3178.18),
        Vec2::new(407.79, 3169.97),
    ]
    .map(|point| point - Vec2::new(416.99, 3168.58) + at)
    .to_vec()
}

fn right_angles(ring: &[Vec2]) -> bool {
    (0..ring.len()).all(|index| {
        let (prev, at, next) = (
            ring[(index + ring.len() - 1) % ring.len()],
            ring[index],
            ring[(index + 1) % ring.len()],
        );
        (prev - at).normalize().dot((next - at).normalize()).abs() < 0.01
    })
}

/// Косо обведённый маленький дом выпрямляется в прямоугольник той же площади
/// на том же месте, и вход с его вершины переезжает на угол прямоугольника.
#[test]
fn a_skewed_small_house_is_squared_into_a_rectangle() {
    let ring = skewed_house(CENTER);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ring[1])
        .area(&[("building", "house")], ring.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.outer.len(), 4);
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    let area =
        |ring: &[Vec2]| signed_ring_area(&ring.iter().map(|p| *p - CENTER).collect::<Vec<_>>());
    assert!(
        (area(&house.outer) - area(&ring)).abs() < 0.5,
        "площадь и обход сохраняются"
    );
    let centre = |ring: &[Vec2]| ring.iter().map(|p| *p - CENTER).sum::<Vec2>() / 4.0;
    assert!(centre(&house.outer).distance(centre(&ring)) < 0.5);
    for (from, to) in ring.iter().zip(&house.outer) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
    assert!(
        house
            .entrances
            .iter()
            .any(|door| door.distance(house.outer[1]) < 0.01),
        "вход остался на старой вершине: {:?}",
        house.entrances
    );
}

/// Вход привязывается к дому по сантиметровому ключу, а не по точному
/// совпадению координат, — и по нему же переезжает на выпрямленный контур.
/// Нода входа в паре миллиметров от вершины (шум проекции) не должна оставлять
/// дверь на старом месте, пока дом уезжает на свои метры.
#[test]
fn an_entrance_a_hair_off_its_vertex_moves_with_the_squared_house() {
    let ring = skewed_house(CENTER);
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ring[1] + Vec2::splat(0.002))
        .area(&[("building", "house")], ring.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.entrances.len(), 1, "вход привязался по ключу");
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    assert!(
        house.entrances[0].distance(house.outer[1]) < 0.01,
        "вход остался на старой вершине: {} против {}",
        house.entrances[0],
        house.outer[1]
    );
}

/// Не выпрямляются: ровный дом (посев по первой вершине не должен сдвинуться),
/// дом, делящий вершину с соседом (разошлись бы щелью), трапеция и крупное
/// здание.
#[test]
fn only_a_lone_skewed_small_house_is_squared() {
    let exact = square(CENTER, 5.0);
    let lone = skewed_house(CENTER + Vec2::new(60.0, 0.0));
    let terraced = skewed_house(CENTER + Vec2::new(0.0, 60.0));
    let neighbour = vec![
        terraced[0],
        terraced[0] + Vec2::new(0.0, -8.0),
        terraced[0] + Vec2::new(-8.0, -8.0),
        terraced[0] + Vec2::new(-8.0, 0.0),
    ];
    let trapezoid_at = CENTER + Vec2::new(-60.0, 0.0);
    let trapezoid = vec![
        // углы 51° и 129°: перекос 39°, за `SQUARE_SKEW_MAX`
        trapezoid_at + Vec2::new(-10.0, -5.0),
        trapezoid_at + Vec2::new(10.0, -5.0),
        trapezoid_at + Vec2::new(2.0, 5.0),
        trapezoid_at + Vec2::new(-2.0, 5.0),
    ];
    let big = skewed_house(CENTER + Vec2::new(0.0, -60.0))
        .iter()
        .map(|p| (*p - (CENTER + Vec2::new(0.0, -60.0))) * 2.0 + CENTER + Vec2::new(0.0, -60.0))
        .collect::<Vec<_>>();
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], exact.clone())
        .area(&[("building", "house")], lone.clone())
        .area(&[("building", "house")], terraced.clone())
        .area(&[("building", "house")], neighbour)
        .area(&[("building", "house")], trapezoid.clone())
        .area(&[("building", "house")], big.clone())
        .parse();

    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(same(&exact, &map.buildings[0].outer), "ровный дом тронут");
    assert!(
        !same(&lone, &map.buildings[1].outer),
        "одинокий косой дом не выпрямлен"
    );
    assert!(
        same(&terraced, &map.buildings[2].outer),
        "дом с общей вершиной выпрямлен"
    );
    assert!(
        same(&trapezoid, &map.buildings[4].outer),
        "трапеция выпрямлена"
    );
    assert!(
        same(&big, &map.buildings[5].outer),
        "крупное здание выпрямлено"
    );
}

/// Площадные слои в общей вершине считаются **не все**: край стоянки, площадки
/// и водоёма нарисован сам по себе, с разметкой, а на стоянке ещё и машинами,
/// так что уехавший от него дом виден на кадре, — а квартал `landuse` лежит
/// под всем, частные дома сплошь и рядом обведены по его границе, и щели там
/// не видно (решение коммита 174ab8a).
#[test]
fn a_house_sharing_a_vertex_with_a_parking_lot_stays_put_but_one_on_a_block_is_squared() {
    let corner = |ring: &[Vec2]| {
        vec![
            ring[0],
            ring[0] + Vec2::new(0.0, -20.0),
            ring[0] + Vec2::new(-20.0, -20.0),
            ring[0] + Vec2::new(-20.0, 0.0),
        ]
    };
    let by_lot = skewed_house(CENTER + Vec2::new(60.0, 0.0));
    let by_block = skewed_house(CENTER + Vec2::new(-60.0, 0.0));
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], by_lot.clone())
        .area(&[("building", "house")], by_block.clone())
        .area(&[("amenity", "parking")], corner(&by_lot))
        .area(&[("landuse", "residential")], corner(&by_block))
        .parse();

    assert_eq!(map.parking.len(), 1, "стоянка разобрана");
    assert_eq!(map.landuse.len(), 1, "квартал разобран");
    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(
        same(&by_lot, &map.buildings[0].outer),
        "дом с общей вершиной со стоянкой выпрямлен"
    );
    assert!(
        !same(&by_block, &map.buildings[1].outer),
        "дом на границе квартала не выпрямлен"
    );
}

/// Дом с пристройкой, обведённый кривой буквой Г, выпрямляется в Г из прямых
/// углов с тем же числом вершин и почти той же площадью, и вход с вершины
/// переезжает на её угол. Ровная Г и выпуклый шестиугольник не трогаются.
#[test]
fn a_skewed_ell_house_is_squared_into_an_ell() {
    // Тула, way 968378349: углы до 17° мимо прямого, 99 м²
    let ell = [
        (629.8, 3563.2),
        (640.4, 3569.9),
        (645.9, 3561.8),
        (642.2, 3557.9),
        (637.8, 3562.0),
        (631.6, 3558.6),
    ]
    .map(|(x, y)| Vec2::new(x - 638.0, y - 3563.0) + CENTER)
    .to_vec();
    let straight_at = CENTER + Vec2::new(60.0, 0.0);
    let straight = [
        (0.0, 0.0),
        (12.0, 0.0),
        (12.0, 6.0),
        (6.0, 6.0),
        (6.0, 12.0),
        (0.0, 12.0),
    ]
    .map(|(x, y)| straight_at + Vec2::new(x, y))
    .to_vec();
    let hexagon_at = CENTER + Vec2::new(-60.0, 0.0);
    let hexagon = (0..6)
        .map(|index| {
            hexagon_at + Vec2::from_angle(index as f32 * std::f32::consts::TAU / 6.0) * 5.0
        })
        .collect::<Vec<_>>();
    let map = Overpass::new(CITY)
        .node(&[("entrance", "main")], ell[2])
        .area(&[("building", "house")], ell.clone())
        .area(&[("building", "house")], straight.clone())
        .area(&[("building", "house")], hexagon.clone())
        .parse();

    let house = &map.buildings[0];
    assert_eq!(house.outer.len(), 6);
    assert!(right_angles(&house.outer), "{:?}", house.outer);
    let area =
        |ring: &[Vec2]| signed_ring_area(&ring.iter().map(|p| *p - CENTER).collect::<Vec<_>>());
    assert!(
        (area(&house.outer) / area(&ell) - 1.0).abs() < 0.05,
        "площадь и обход сохраняются"
    );
    for (from, to) in ell.iter().zip(&house.outer) {
        assert!(from.distance(*to) < 1.0, "вершина {from} уехала в {to}");
    }
    assert!(
        house
            .entrances
            .iter()
            .any(|door| door.distance(house.outer[2]) < 0.01),
        "вход остался на старой вершине: {:?}",
        house.entrances
    );
    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    assert!(same(&straight, &map.buildings[1].outer), "ровная Г тронута");
    assert!(
        same(&hexagon, &map.buildings[2].outer),
        "шестиугольник выпрямлен"
    );
}

/// Частный сектор Тулы обводят и на 20–32° мимо прямого угла, и крупный
/// `building=house` тоже: оба выпрямляются. Здание без класса той же площади,
/// что крупный дом, остаётся как есть — у него порог скатной когорты.
#[test]
fn a_steeply_skewed_house_and_a_large_house_are_squared() {
    let around = |points: &[(f32, f32)], centre: (f32, f32), at: Vec2| {
        points
            .iter()
            .map(|(x, y)| Vec2::new(x - centre.0, y - centre.1) + at)
            .collect::<Vec<_>>()
    };
    // way 968378337: углы 65°–122°, 49 м²
    let steep = around(
        &[
            (580.8, 3528.2),
            (589.6, 3533.8),
            (592.8, 3529.0),
            (585.2, 3525.4),
        ],
        (586.0, 3529.0),
        CENTER,
    );
    // way 968378335: перекос 21°, 348 м²
    let large_ring = [
        (598.0, 3505.7),
        (626.5, 3521.8),
        (621.3, 3532.2),
        (596.0, 3517.0),
    ];
    let large = around(&large_ring, (610.0, 3519.0), CENTER + Vec2::new(80.0, 0.0));
    let untagged = around(&large_ring, (610.0, 3519.0), CENTER + Vec2::new(-80.0, 0.0));
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], steep.clone())
        .area(&[("building", "house")], large.clone())
        .area(&[("building", "yes")], untagged.clone())
        .parse();

    assert!(
        right_angles(&map.buildings[0].outer),
        "{:?}",
        map.buildings[0].outer
    );
    assert!(
        right_angles(&map.buildings[1].outer),
        "{:?}",
        map.buildings[1].outer
    );
    assert!(
        untagged
            .iter()
            .zip(&map.buildings[2].outer)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "здание без класса крупнее 250 м² выпрямлено"
    );
}

/// Дом, стеной заходящий на нарисованный тротуар улицы, отодвигается от неё
/// целиком и тянет за собой соседей по ряду; дом в стороне, дом с общей
/// вершиной и дом, сквозь который идёт улица, остаются на месте.
#[test]
fn a_house_on_the_sidewalk_is_pulled_back_into_the_block() {
    // residential 8 м: полоса с тротуаром и зазором — 4 + 1.76 + 2 от оси
    let reach = 4.0 + sidewalk_width(8.0).unwrap() + SIDEWALK_CLEARANCE;
    let street = vec![
        CENTER - Vec2::new(300.0, 0.0),
        CENTER + Vec2::new(300.0, 0.0),
    ];
    let house = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, gap),
            CENTER + Vec2::new(x + 12.0, gap + 10.0),
        )
    };
    let on_sidewalk = house(-200.0, 4.7);
    // сосед по ряду на той же линии — сдвигается на тот же сдвиг, хотя сам
    // не наезжает; сосед глубже в квартале — нет
    let in_row = house(-180.0, 6.5);
    let set_back = house(-160.0, 9.5);
    let clear = house(-120.0, 10.0);
    let crossed = house(-100.0, -5.0);
    let terraced = house(0.0, 4.7);
    let neighbour = house(12.0, 4.7);
    // уличный дом с юга: толкать надо в минус по y
    let south = rect(
        CENTER + Vec2::new(100.0, -14.0),
        CENTER + Vec2::new(112.0, -5.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential")], street)
        .area(&[("building", "yes")], on_sidewalk.clone())
        .area(&[("building", "yes")], clear.clone())
        .area(&[("building", "yes")], crossed.clone())
        .area(&[("building", "yes")], terraced.clone())
        .area(&[("building", "yes")], neighbour)
        .area(&[("building", "yes")], south.clone())
        .area(&[("building", "yes")], in_row.clone())
        .area(&[("building", "yes")], set_back.clone())
        .parse();

    let same =
        |ring: &[Vec2], got: &[Vec2]| ring.iter().zip(got).all(|(a, b)| a.distance(*b) < 0.01);
    let gap = |ring: &[Vec2]| {
        ring.iter()
            .map(|vertex| (vertex.y - CENTER.y).abs())
            .fold(f32::INFINITY, f32::min)
    };
    let pulled = &map.buildings[0].outer;
    assert!(
        gap(pulled) >= reach - 0.06,
        "дом остался на тротуаре: {pulled:?}"
    );
    assert!(gap(pulled) < reach + 0.1, "дом унесён дальше нужного");
    assert!(
        pulled
            .iter()
            .zip(&on_sidewalk)
            .all(|(a, b)| (a.x - b.x).abs() < 0.01 && a.y > b.y),
        "сдвиг не поперёк улицы от неё"
    );
    assert!(
        same(&clear, &map.buildings[1].outer),
        "дом в стороне тронут"
    );
    assert!(
        same(&crossed, &map.buildings[2].outer),
        "дом на оси улицы сдвинут"
    );
    assert!(
        same(&terraced, &map.buildings[3].outer),
        "дом с общей вершиной сдвинут"
    );
    let south_pulled = &map.buildings[5].outer;
    assert!(gap(south_pulled) >= reach - 0.06, "{south_pulled:?}");
    assert!(south_pulled.iter().zip(&south).all(|(a, b)| a.y < b.y));

    // ряд держит линию: сосед сдвинут ровно на сдвиг наезжающего дома
    let row_shift = pulled[0].y - on_sidewalk[0].y;
    let neighbour_shift = map.buildings[6].outer[0].y - in_row[0].y;
    assert!(
        (neighbour_shift - row_shift).abs() < 0.01,
        "сосед по ряду сдвинут на {neighbour_shift}, ряд — на {row_shift}"
    );
    assert!(
        same(&set_back, &map.buildings[7].outer),
        "дом в глубине квартала сдвинут вместе с рядом"
    );
}

/// Дому, которому нужно больше предела, не отказывают: он сдвигается на
/// предел и не держит на месте свой ряд; а сдвиг, упёршийся в забор или в
/// соседнее здание, укорачивается, пока между ними не останется зазор.
#[test]
fn a_pull_is_capped_and_stops_short_of_what_stands_behind() {
    let reach = 4.0 + sidewalk_width(8.0).unwrap() + SIDEWALK_CLEARANCE;
    let street = vec![
        CENTER - Vec2::new(300.0, 0.0),
        CENTER + Vec2::new(300.0, 0.0),
    ];
    let house = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, gap),
            CENTER + Vec2::new(x + 12.0, gap + 10.0),
        )
    };
    // стена в метре от оси: нужно 6.76 м, предел 6
    let deep = house(-200.0, 1.0);
    // сосед по той же стороне, в 8 м от глубокого: встал бы с ним в ряд
    let beside = house(-180.0, 4.7);
    // забор в 2.5 м за задней стеной: полный сдвиг 3.06 м его пересёк бы
    let fenced = house(0.0, 4.7);
    let fence = vec![
        CENTER + Vec2::new(-5.0, 17.2),
        CENTER + Vec2::new(17.0, 17.2),
    ];
    // здание в 1.3 м за задней стеной, само улицы не касается
    let crowded = house(150.0, 4.7);
    let behind = rect(
        CENTER + Vec2::new(150.0, 16.0),
        CENTER + Vec2::new(162.0, 26.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential")], street)
        .area(&[("building", "yes")], deep.clone())
        .area(&[("building", "yes")], beside.clone())
        .area(&[("building", "yes")], fenced.clone())
        .way(&[("barrier", "fence")], fence)
        .area(&[("building", "yes")], crowded.clone())
        .area(&[("building", "yes")], behind.clone())
        .parse();

    let shift = |index: usize, ring: &[Vec2]| map.buildings[index].outer[0].y - ring[0].y;
    assert!(
        (shift(0, &deep) - SIDEWALK_SHIFT_MAX).abs() < 0.01,
        "глубокий дом сдвинут на {}",
        shift(0, &deep)
    );
    assert!(
        (shift(1, &beside) - (reach - 4.7)).abs() < 0.06,
        "сосед глубокого дома сдвинут на {}",
        shift(1, &beside)
    );
    let full = reach - 4.7;
    assert!(
        (shift(2, &fenced) - full * 0.5).abs() < 0.01,
        "дом у забора сдвинут на {}",
        shift(2, &fenced)
    );
    assert!(
        (shift(3, &crowded) - full * 0.25).abs() < 0.01,
        "дом перед соседом сдвинут на {}",
        shift(3, &crowded)
    );
    assert!(
        map.buildings[4]
            .outer
            .iter()
            .zip(&behind)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "здание позади тронуто"
    );
}

/// Край квартала, которому до нарисованного полотна осталась пара метров,
/// дотягивается **под** асфальт; квартал в стороне остаётся на месте, и улица
/// внутри квартала его границу к себе не стягивает — зелень только растёт.
#[test]
fn a_block_edge_is_pulled_under_the_asphalt() {
    // residential 8 м: край полотна с тротуаром — 4 + 1.76 от оси
    let edge = 4.0 + sidewalk_width(8.0).unwrap();
    let street = vec![
        CENTER - Vec2::new(400.0, 0.0),
        CENTER + Vec2::new(400.0, 0.0),
    ];
    let block = |x: f32, gap: f32| {
        rect(
            CENTER + Vec2::new(x, -40.0),
            CENTER + Vec2::new(x + 80.0, -gap),
        )
    };
    let near = block(-300.0, 6.5);
    let far = block(-200.0, 12.0);
    // улица идёт внутри квартала, в 7 м от его южной границы
    let around = rect(
        CENTER + Vec2::new(-100.0, -7.0),
        CENTER + Vec2::new(-20.0, 60.0),
    );
    let map = Overpass::new(CITY)
        .way(&[("highway", "residential")], street)
        .area(&[("landuse", "residential")], near)
        .area(&[("landuse", "residential")], far.clone())
        .area(&[("landuse", "residential")], around)
        .parse();

    let edges = |ring: &[Vec2]| {
        ring.iter()
            .map(|vertex| vertex.y - CENTER.y)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
                (low.min(y), high.max(y))
            })
    };
    let (_, top) = edges(&map.landuse[0].outer);
    assert!(
        (top + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "край квартала не заведён под полотно: {top}"
    );
    assert!(
        map.landuse[1]
            .outer
            .iter()
            .zip(&far)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "квартал в стороне от улицы тронут"
    );
    let (low, high) = edges(&map.landuse[2].outer);
    assert!(
        (low + 7.0).abs() < 0.01 && (high - 60.0).abs() < 0.01,
        "улица внутри квартала стянула его границу к себе: {low}..{high}"
    );
}

/// Дырка в квартале, сквозь которую идёт улица, **сжимается** к ней: зелень у
/// дырки — тот же край двора, и подходить к полотну обязан он.
#[test]
fn a_street_in_a_courtyard_shrinks_the_hole_to_its_asphalt() {
    let edge = 4.0 + sidewalk_width(8.0).unwrap();
    let map = Overpass::new(CITY)
        .way(
            &[("highway", "residential")],
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
        )
        .relation(
            &[("landuse", "residential"), ("type", "multipolygon")],
            &[
                ("outer", closed(square(CENTER, 100.0))),
                (
                    "inner",
                    closed(rect(
                        CENTER + Vec2::new(-60.0, -7.0),
                        CENTER + Vec2::new(60.0, 7.0),
                    )),
                ),
            ],
        )
        .parse();

    let hole = &map.landuse[0].holes[0];
    let reach = hole
        .iter()
        .map(|vertex| (vertex.y - CENTER.y).abs())
        .fold(0.0, f32::max);
    assert!(
        (reach - (edge - LANDUSE_OVERLAP)).abs() < 0.02,
        "край дырки не подошёл к полотну: {reach}"
    );
}

/// Цвета из разметки доезжают до дома: hex — как есть, имя — краской карты,
/// незнакомое имя — пусто, а у воды цветов не бывает и с тегом.
#[test]
fn tagged_colours_reach_the_building() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "church"),
                ("building:colour", "#FFD700"),
                ("roof:colour", "Blue"),
            ],
            square(CENTER, 10.0),
        )
        .area(
            &[("building", "yes"), ("roof:colour", "#abc")],
            square(CENTER + Vec2::new(60.0, 0.0), 10.0),
        )
        .area(
            &[("building", "yes"), ("building:colour", "chartreuse-ish")],
            square(CENTER + Vec2::new(120.0, 0.0), 10.0),
        )
        .area(
            &[("natural", "water"), ("building:colour", "red")],
            square(CENTER + Vec2::new(0.0, 120.0), 10.0),
        )
        .parse();
    assert_eq!(
        map.buildings[0].colours,
        Colours {
            wall: Some([255, 215, 0]),
            roof: colour("blue"),
        }
    );
    assert_eq!(map.buildings[1].colours.roof, Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(map.buildings[2].colours, Colours::default());
    assert_eq!(map.water[0].colours, Colours::default());
    assert_eq!(colour("#GGG"), None);
    assert_eq!(colour("#12345"), None);
    assert_eq!(colour("dimgrey"), colour("dimgray"));
    assert_ne!(
        colour("blue"),
        Some([0, 0, 255]),
        "имя — краска карты, не CSS"
    );
}

// --- шов между чтением элементов и доводкой ---------------------------------
//
// Тесты, которые зовут **один проход**, а не конвейер целиком. До шва так никто
// не делал: `parse` была одним телом на сто двадцать пять строк, у стадии не
// было имени, и каждый тест шёл через `Overpass::…parse()`, адресуя дома по их
// месту в фикстуре.

/// Сырая карта из сцены — ровно то, что отдаёт элементный цикл, без доводки.
fn read(scene: &Overpass) -> (MapData, Vec<Vec2>, ReadReport) {
    let response: OverpassResponse =
        serde_json::from_str(&scene.json()).expect("фикстура строит валидный JSON");
    read_elements(&response, &GeoBounds::for_city(CITY))
}

/// Чтение элементов ничего не доводит: дом посреди пруда из него выходит
/// живым, и топит его отдельный проход.
#[test]
fn reading_the_elements_leaves_the_passes_undone() {
    let scene = Overpass::new(CITY)
        .area(&[("natural", "water")], square(CENTER, 100.0))
        .area(&[("building", "yes")], square(CENTER, 10.0));

    let (mut map, _, _) = read(&scene);
    assert_eq!(map.buildings.len(), 1, "сарай посреди пруда ещё стоит");

    let drowned = drop_buildings_in_water(&mut map);
    assert_eq!(drowned, 1);
    assert!(map.buildings.is_empty());
}

/// Вторая половина шва тоже отчитывается **значением**: что элементный цикл
/// узнал про сторону движения и сколько колец бросил, раньше можно было
/// прочесть только на stderr.
#[test]
fn reading_the_elements_reports_what_it_skipped() {
    let (sw, se, ..) = corners(HALF);
    let torn = Overpass::new(CITY)
        .relation(
            &[
                ("boundary", "administrative"),
                ("admin_level", "2"),
                ("driving_side", "left"),
            ],
            &[],
        )
        // кольцо из одного куска в две точки: соединять не с чем, замкнуть
        // нечем — такое бросается и считается
        .relation(&[("natural", "water")], &[("outer", vec![sw, se])]);

    let (map, _, report) = read(&torn);
    assert_eq!(report.unclosed_rings, 1);
    assert!(map.water.is_empty(), "порванное кольцо не стало водой");
    assert_eq!(report.traffic_side, Some(TrafficSide::Left));

    // без границы в ответе сторона движения неизвестна, и карта рисуется
    // правосторонней — отчёт говорит именно «неизвестна», а не «правая»
    let (map, _, report) = read(&Overpass::new(CITY));
    assert_eq!(report.unclosed_rings, 0);
    assert!(report.traffic_side.is_none());
    assert_eq!(map.traffic_side, TrafficSide::Right);
}

/// Проход зовётся сам по себе, на карте, собранной руками, — без JSON, без
/// `GeoBounds` и без остальных шести проходов.
#[test]
fn a_pass_runs_on_a_hand_built_map() {
    let mut map = MapData {
        buildings: vec![
            building(square(CENTER, 10.0), Vec::new()),
            building(square(CENTER + Vec2::new(500.0, 0.0), 10.0), Vec::new()),
        ],
        water: vec![water_area(square(CENTER, 100.0), Vec::new())],
        ..MapData::default()
    };

    assert_eq!(drop_buildings_in_water(&mut map), 1);
    assert_eq!(map.buildings.len(), 1, "дом вдали от воды остался");
}

/// Выпрямление косых домиков — тоже само по себе, и видно, что оно сделало.
/// Что именно оно сохраняет (площадь, центр, вход на вершине), проверяет
/// `a_skewed_small_house_is_squared_into_a_rectangle` через весь конвейер;
/// здесь важно только то, что проход зовётся в одиночку.
#[test]
fn squaring_runs_on_its_own() {
    let before = skewed_house(CENTER);
    let mut map = MapData {
        buildings: vec![building(before.clone(), Vec::new())],
        ..MapData::default()
    };

    assert_eq!(square_skewed_houses(&mut map), 1);
    let after = &map.buildings[0].outer;
    assert_eq!(after.len(), 4);
    assert_ne!(*after, before, "контур не тронут");

    assert!(right_angles(after), "углы не стали прямыми");
    for (from, to) in before.iter().zip(after) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
}

/// Порядок проходов — это их интерфейс, и вот доказательство, что он
/// load-bearing: те же два прохода в обратном порядке теряют дверь.
///
/// Разметанный вход держит координату **ноды**, а выпрямление уносит вершину
/// контура. Пока дверь приложена к дому, её уносит тот же сантиметровый ключ;
/// если же выпрямить сначала, ключ вершины уже другой, и приложить дверь
/// становится не к чему. Написать такой тест до шва было нечем: оба прохода
/// жили внутри одного тела `parse`, и переставить их местами было негде.
#[test]
fn squaring_before_attaching_loses_the_door() {
    let ring = skewed_house(CENTER);
    let door = ring[1];
    let house = || MapData {
        buildings: vec![building(ring.clone(), Vec::new())],
        ..MapData::default()
    };

    // как в `finish_parse`: сначала дверь, потом выпрямление
    let mut in_order = house();
    assert_eq!(attach_entrances(&mut in_order, &[door]), 0);
    assert_eq!(square_skewed_houses(&mut in_order), 1);
    let carried = in_order.buildings[0].entrances[0];
    assert!(
        in_order.buildings[0]
            .outer
            .iter()
            .any(|&vertex| vertex.distance(carried) < 0.01),
        "дверь съехала с выпрямленного контура: {carried}"
    );

    // наоборот: дом выпрямлен, ноду двери прикладывать уже не к чему
    let mut reversed = house();
    assert_eq!(square_skewed_houses(&mut reversed), 1);
    assert_eq!(
        attach_entrances(&mut reversed, &[door]),
        1,
        "дверь нашла дом на старой вершине"
    );
    assert!(reversed.buildings[0].entrances.is_empty());
}

/// Доводка целиком — одним вызовом, и она отчитывается значением: те же
/// счётчики, что уходили в лог восемью `eprintln!`, теперь можно сравнить.
#[test]
fn finishing_the_parse_reports_what_each_pass_did() {
    let scene = Overpass::new(CITY)
        .area(&[("natural", "water")], square(CENTER, 100.0))
        // тонет
        .area(&[("building", "yes")], square(CENTER, 10.0))
        // выпрямляется: косой домик вдали от воды и дорог
        .area(
            &[("building", "house")],
            skewed_house(CENTER + Vec2::new(800.0, 0.0)),
        )
        // сажается и собирается: одиночное дерево вдали от всего остального
        .node(
            &[("natural", "tree"), ("diameter_crown", "10")],
            CENTER + Vec2::new(-800.0, 0.0),
        );

    let (mut map, entrances, _) = read(&scene);
    let report = finish_parse(&mut map, &entrances);

    assert_eq!(report.drowned, 1);
    assert_eq!(report.squared, 1);
    assert_eq!(report.entrances_found, 0);
    assert_eq!(report.entrances_orphaned, 0);
    // и счётчики остальных проходов — тоже значением, а не строкой на stderr
    assert_eq!(
        report.pulled.moved, 0,
        "дорог в сцене нет, отодвигать не от чего"
    );
    assert_eq!(report.pulled.left, 0);
    assert_eq!(report.stretched, 0, "кварталов в сцене нет");
    assert_eq!(report.planted.tree_nodes, 1);
    assert_eq!(report.planted.standalone, 1);
    assert_eq!(map.buildings.len(), 1, "остался только косой домик");
    // и деревья собраны по составу по умолчанию, а не оставлены несобранными:
    // посаженная одиночка доехала до набора, который читает рендер
    assert_eq!(map.standalone_trees.len(), 1, "дерево посажено");
    assert_eq!(map.trees.len(), 1, "и собрано в набор рендера");
    assert_eq!(map.composed_for, Some(TreeCompose::default()));
}

/// Отодвигание домов от тротуаров — в одиночку, на карте, собранной руками:
/// улица и два дома, а не сцена Overpass. Что именно проход сохраняет и кого
/// оставляет на месте, проверяет `a_house_on_the_sidewalk_is_pulled_back_into_the_block`
/// через весь конвейер; здесь важно, что проход зовётся по имени и что
/// счётчики у него не нулевые.
#[test]
fn pulling_houses_off_the_sidewalks_runs_on_its_own() {
    // residential 8 м: полоса с тротуаром и зазором — 4 + 1.76 + 2 от оси
    let reach = 4.0 + sidewalk_width(8.0).unwrap() + SIDEWALK_CLEARANCE;
    let on_sidewalk = rect(
        CENTER + Vec2::new(-20.0, 4.7),
        CENTER + Vec2::new(-8.0, 14.7),
    );
    let clear = rect(
        CENTER + Vec2::new(20.0, 20.0),
        CENTER + Vec2::new(32.0, 30.0),
    );
    let mut map = MapData {
        roads: vec![street(
            vec![
                CENTER - Vec2::new(300.0, 0.0),
                CENTER + Vec2::new(300.0, 0.0),
            ],
            8.0,
        )],
        buildings: vec![
            building(on_sidewalk.clone(), Vec::new()),
            building(clear.clone(), Vec::new()),
        ],
        ..MapData::default()
    };

    let pulled = pull_houses_off_sidewalks(&mut map);
    assert_eq!(pulled.moved, 1, "наезжающий дом не сдвинут");
    assert_eq!(pulled.partly, 0, "сдвигу ничто не мешало");
    assert_eq!(pulled.left, 0);

    let gap = map.buildings[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        gap >= reach - 0.06,
        "дом остался на тротуаре: {gap} м от оси"
    );
    assert!(gap < reach + 0.1, "дом унесён дальше нужного: {gap}");
    assert!(
        map.buildings[1]
            .outer
            .iter()
            .zip(&clear)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "дом в стороне от улицы тронут"
    );
}

/// Дотягивание кварталов до дорог — тоже само по себе и тоже с ненулевым
/// счётчиком: у ближнего квартала под асфальт уходит весь его верхний край,
/// дальний не трогают.
#[test]
fn pulling_the_blocks_to_the_roads_runs_on_its_own() {
    // residential 8 м: край полотна с тротуаром — 4 + 1.76 от оси
    let edge = 4.0 + sidewalk_width(8.0).unwrap();
    let block = |ring: Vec<Vec2>| PolyArea {
        kind: AreaKind::Residential,
        ..building(ring, Vec::new())
    };
    let near = rect(
        CENTER + Vec2::new(-40.0, -40.0),
        CENTER + Vec2::new(40.0, -6.5),
    );
    let far = rect(
        CENTER + Vec2::new(60.0, -40.0),
        CENTER + Vec2::new(140.0, -14.0),
    );
    let mut map = MapData {
        roads: vec![street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            8.0,
        )],
        landuse: vec![block(near), block(far.clone())],
        ..MapData::default()
    };

    let stretched = pull_landuse_to_roads(&mut map);
    assert!(stretched >= 2, "дотянуто вершин: {stretched}");
    let top = map.landuse[0]
        .outer
        .iter()
        .map(|vertex| vertex.y - CENTER.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (top + edge - LANDUSE_OVERLAP).abs() < 0.02,
        "край квартала не заведён под полотно: {top}"
    );
    assert!(
        map.landuse[1]
            .outer
            .iter()
            .zip(&far)
            .all(|(a, b)| a.distance(*b) < 0.01),
        "квартал в стороне от улицы тронут"
    );
}

/// Сборка храмов — в одиночку, на трёх контурах: барабан внутри собора берёт
/// его веру и его посев, а одинокая церковь без разметки — веру большинства
/// города, и она же одна и попадает в счётчик угаданных.
#[test]
fn resolving_the_faiths_runs_on_its_own() {
    let church = |ring: Vec<Vec2>, faith: Faith, form: SacredForm| PolyArea {
        building_use: BuildingUse::Church(Sacred {
            faith,
            form,
            complex: 0,
            floor_dm: 0,
        }),
        ..building(ring, Vec::new())
    };
    let mut buildings = vec![
        church(square(CENTER, 20.0), Faith::Orthodox, SacredForm::Nave),
        // барабан стоит в контуре собора и своей веры не имеет
        church(square(CENTER, 5.0), Faith::Unknown, SacredForm::Dome),
        // а эта церковь стоит сама по себе, и веры у неё тоже нет
        church(
            square(CENTER + Vec2::new(500.0, 0.0), 15.0),
            Faith::Unknown,
            SacredForm::Nave,
        ),
    ];

    assert_eq!(
        resolve_faiths(&mut buildings),
        1,
        "по большинству города угадан ровно один храм"
    );

    let sacred = |building: &PolyArea| match building.building_use {
        BuildingUse::Church(sacred) => sacred,
        other => panic!("не храм: {other:?}"),
    };
    let (cathedral, drum, lone) = (
        sacred(&buildings[0]),
        sacred(&buildings[1]),
        sacred(&buildings[2]),
    );
    assert_ne!(cathedral.complex, 0, "посев храма не проставлен");
    assert_eq!(drum.faith, Faith::Orthodox, "барабан не взял веру собора");
    assert_eq!(drum.complex, cathedral.complex, "и красится сам по себе");
    assert_eq!(
        lone.faith,
        Faith::Orthodox,
        "одинокая церковь не взяла веру большинства"
    );
    assert_ne!(
        lone.complex, cathedral.complex,
        "чужой храм красится посевом собора"
    );
}

/// Прямоугольник по косому четырёхугольнику: углы прямые, площадь, центроид
/// и обход — те же, и вершина `i` встаёт рядом со своей.
#[test]
fn a_fitted_rectangle_keeps_the_area_the_centroid_and_the_winding() {
    let ring = skewed_house(CENTER);
    let quad = [ring[0], ring[1], ring[2], ring[3]];
    let fitted = fit_rectangle(&quad);

    assert!(right_angles(&fitted), "углы не прямые: {fitted:?}");
    let local = |ring: &[Vec2]| ring.iter().map(|point| *point - CENTER).collect::<Vec<_>>();
    let (before, after) = (
        signed_ring_area(&local(&quad)),
        signed_ring_area(&local(&fitted)),
    );
    assert!(
        (after / before - 1.0).abs() < 1e-3,
        "площадь {before} → {after}"
    );
    assert!(
        ring_area_centroid(&local(&quad)).distance(ring_area_centroid(&local(&fitted))) < 0.01,
        "центроид уехал"
    );
    for (from, to) in quad.iter().zip(&fitted) {
        assert!(from.distance(*to) < 2.5, "вершина {from} уехала в {to}");
    }
}

/// Г по косо обведённой Г: шесть прямых углов на своих местах. Контур, у
/// которого два соседних ребра лежат по одной оси, — не Г, и ответа нет.
#[test]
fn a_fitted_ell_squares_its_corners_and_refuses_what_is_not_an_ell() {
    // Тула, way 968378349: углы до 17° мимо прямого, 99 м²
    let ell = [
        (629.8, 3563.2),
        (640.4, 3569.9),
        (645.9, 3561.8),
        (642.2, 3557.9),
        (637.8, 3562.0),
        (631.6, 3558.6),
    ]
    .map(|(x, y)| Vec2::new(x - 638.0, y - 3563.0) + CENTER)
    .to_vec();

    let fitted = fit_ell(&ell).expect("Г не выпрямилась");
    assert_eq!(fitted.len(), ell.len());
    assert!(right_angles(&fitted), "углы не прямые: {fitted:?}");
    let area = |ring: &[Vec2]| {
        signed_ring_area(&ring.iter().map(|point| *point - CENTER).collect::<Vec<_>>())
    };
    assert!(
        (area(&fitted) / area(&ell) - 1.0).abs() < ELL_AREA_DRIFT,
        "площадь ушла: {} → {}",
        area(&ell),
        area(&fitted)
    );
    for (from, to) in ell.iter().zip(&fitted) {
        assert!(from.distance(*to) < 1.0, "вершина {from} уехала в {to}");
    }

    // прямоугольник с лишней вершиной посередине длинной стороны: рёбра по
    // осям не чередуются
    let not_an_ell = [
        (0.0, 0.0),
        (6.0, 0.0),
        (12.0, 0.0),
        (12.0, 8.0),
        (6.0, 8.0),
        (0.0, 8.0),
    ]
    .map(|(x, y)| CENTER + Vec2::new(x, y))
    .to_vec();
    assert!(fit_ell(&not_an_ell).is_none());
}

/// Ближайшая пара точек двух отрезков: у пересекающихся её нет, у
/// параллельных она поперёк, у разминувшихся торцами — концы.
#[test]
fn the_closest_pair_of_two_segments_is_none_only_when_they_cross() {
    let (a, b) = (CENTER, CENTER + Vec2::new(10.0, 0.0));
    assert!(
        closest_between_segments(
            a,
            b,
            CENTER + Vec2::new(5.0, -5.0),
            CENTER + Vec2::new(5.0, 5.0)
        )
        .is_none(),
        "у пересекающихся отрезков ближайшей пары не бывает"
    );

    let (near_a, near_c) = closest_between_segments(
        a,
        b,
        CENTER + Vec2::new(2.0, 3.0),
        CENTER + Vec2::new(8.0, 3.0),
    )
    .expect("параллельные отрезки не пересекаются");
    assert!(
        (near_a.distance(near_c) - 3.0).abs() < 1e-3,
        "зазор {} вместо 3",
        near_a.distance(near_c)
    );
    assert!(
        (near_a.y - CENTER.y).abs() < 1e-3 && (near_c.y - CENTER.y - 3.0).abs() < 1e-3,
        "пара взята не поперёк: {near_a} и {near_c}"
    );

    let (near_a, near_c) = closest_between_segments(
        a,
        b,
        CENTER + Vec2::new(14.0, 0.0),
        CENTER + Vec2::new(20.0, 0.0),
    )
    .expect("отрезки на одной прямой не пересекаются");
    assert!(near_a.distance(b) < 1e-3, "{near_a} вместо конца отрезка");
    assert!(near_c.distance(CENTER + Vec2::new(14.0, 0.0)) < 1e-3);
}
