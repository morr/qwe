use super::fixtures::{CENTER, CITY, HALF, WOOD_HALF, corners, fixture};
use super::*;
use crate::map::osm::fixture::{Overpass, closed, rect, square};
use crate::map::osm::model::{
    AreaKind, FenceKind, PitchKind, RoadAreaKind, RoadNodeKind, StructureKind, WaterKind,
    distance_to_segment,
};
use bevy::math::Vec2;

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

/// Игла в кольце площадки — возврат p → q → p — сворачивается при разборе.
/// Самокасание оставалось в полигоне, и кайма газона рисовала обе стороны
/// иглы: светлый шов «V» поверх газона обочины (Тула, сквер у Сойфера, N8).
#[test]
fn a_needle_in_an_area_ring_is_collapsed() {
    let map = Overpass::new(CITY)
        .area(
            &[("landuse", "grass")],
            vec![
                CENTER + Vec2::new(-20.0, -15.0),
                CENTER + Vec2::new(20.0, -15.0),
                CENTER + Vec2::new(20.0, 15.0),
                CENTER + Vec2::new(0.0, 15.0),
                CENTER + Vec2::new(0.0, 3.0),
                CENTER + Vec2::new(0.0, 15.0),
                CENTER + Vec2::new(-20.0, 15.0),
            ],
        )
        .parse();
    assert_eq!(map.grass.len(), 1);
    let outer = &map.grass[0].outer;
    let needle = outer
        .iter()
        .any(|p| Vec2::new(p[0], p[1]).distance(CENTER + Vec2::new(0.0, 3.0)) < 1.0);
    assert!(!needle, "игла осталась в кольце: {outer:?}");
    // четыре угла плюс основание иглы, схлопнувшееся в одну точку на кромке
    assert_eq!(outer.len(), 5, "{outer:?}");
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
        // решётка — сквозная, смешанный с бетоном — сплошной
        .way(
            &[("barrier", "fence"), ("fence_type", "metal")],
            vec![sw, ne],
        )
        .way(
            &[("barrier", "fence"), ("fence_type", "barbed_wire;concrete")],
            vec![sw, ne],
        )
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
            FenceKind::Hedge,
            FenceKind::Railing,
            FenceKind::Fence,
        ]
    );
    assert_eq!(map.walls.len(), 1);
}

/// Дорожные узлы доезжают до `MapData::road_nodes` со своим видом; переход
/// несёт регулируемость, островок и разметку. Нода с чужим `highway`
/// (остановка) узлом не становится.
#[test]
fn road_nodes_carry_their_kind() {
    let at = |dx: f32| CENTER + Vec2::new(dx, 0.0);
    let map = Overpass::new(CITY)
        .node(
            &[("highway", "crossing"), ("crossing", "traffic_signals")],
            at(0.0),
        )
        .node(
            &[
                ("highway", "crossing"),
                ("crossing", "unmarked"),
                ("crossing:island", "yes"),
            ],
            at(1.0),
        )
        .node(&[("highway", "crossing")], at(2.0))
        .node(&[("highway", "traffic_signals")], at(3.0))
        .node(&[("highway", "stop")], at(4.0))
        .node(&[("highway", "give_way")], at(5.0))
        .node(&[("highway", "mini_roundabout")], at(6.0))
        .node(&[("highway", "turning_loop")], at(7.0))
        .node(&[("traffic_calming", "island")], at(8.0))
        .node(&[("highway", "bus_stop")], at(9.0))
        .parse();

    let kinds: Vec<RoadNodeKind> = map.road_nodes.iter().map(|node| node.kind).collect();
    let crossing = |signals, island, marked| RoadNodeKind::Crossing {
        signals,
        island,
        marked,
    };
    assert_eq!(
        kinds,
        [
            crossing(true, false, true),
            crossing(false, true, false),
            crossing(false, false, true),
            RoadNodeKind::TrafficSignals,
            RoadNodeKind::Stop,
            RoadNodeKind::GiveWay,
            RoadNodeKind::MiniRoundabout,
            RoadNodeKind::TurningCircle,
            RoadNodeKind::Island,
        ]
    );
    assert!((map.road_nodes[4].pos - at(4.0)).length() < 0.01);
}

/// Площади дорог: покрытие `area:highway` по классу своей дороги, площадь
/// `highway` + `area=yes` — так же и **вдобавок** к линии в `roads`, как было до
/// v15; островок контуром. Незамкнутый way и значение вне словаря дорог
/// площадью не становятся.
#[test]
fn road_areas_are_read_by_the_class_of_their_road() {
    let (sw, se, ..) = corners(HALF);
    let map = Overpass::new(CITY)
        .area(&[("area:highway", "service")], square(CENTER, 5.0))
        .area(&[("area:highway", "footway")], square(CENTER, 4.0))
        .area(
            &[("highway", "pedestrian"), ("area", "yes")],
            square(CENTER, 3.0),
        )
        .area(&[("traffic_calming", "island")], square(CENTER, 2.0))
        .area(&[("area:highway", "emergency")], square(CENTER, 1.0))
        .way(&[("area:highway", "residential")], vec![sw, se])
        .parse();

    let kinds: Vec<RoadAreaKind> = map.road_areas.iter().map(|area| area.kind).collect();
    assert_eq!(
        kinds,
        [
            RoadAreaKind::Carriageway,
            RoadAreaKind::Walkway,
            RoadAreaKind::Walkway,
            RoadAreaKind::Island,
        ]
    );
    assert_eq!(map.road_areas[0].outline.len(), 4, "open ring");
    assert_eq!(map.roads.len(), 1, "the square is still a line as well");
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
