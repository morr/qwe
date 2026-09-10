use super::cohorts::COHORT_ROW_MEAN;
use super::*;
use crate::map::osm::fixture;
use crate::map::osm::model::{BuildingUse, RoadLine};

fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

fn building(outer: Vec<Vec2>, height: Option<f32>) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind: AreaKind::Building,
        building_use: BuildingUse::Other,
        height,
        entrances: Vec::new(),
    }
}

fn road(points: Vec<Vec2>) -> RoadLine {
    fixture::street(points, 8.0)
}

/// Двери первого дома, вышедшие на **одну** стену. Шаг подъездов меряется по
/// одной стороне: у корпуса подъезд сквозной, и его дворовая половина стоит
/// напротив уличной, то есть в тех же метрах длины дома.
fn doors_on(map: &MapData, y: f32) -> Vec<Vec2> {
    map.buildings[0]
        .entrances
        .iter()
        .copied()
        .filter(|door| (door.y - y).abs() < 0.01)
        .collect()
}

/// Дом стоит между двух улиц, но одна из них вплотную к южной грани, а
/// вторая далеко на севере: дверь обязана выйти на ближнюю.
#[test]
fn the_entrance_lands_on_the_facade_facing_the_nearest_road() {
    let mut map = MapData {
        buildings: vec![building(
            rect(Vec2::new(100.0, 100.0), Vec2::new(120.0, 120.0)),
            Some(9.0),
        )],
        roads: vec![
            road(vec![Vec2::new(0.0, 95.0), Vec2::new(400.0, 95.0)]),
            road(vec![Vec2::new(0.0, 300.0), Vec2::new(400.0, 300.0)]),
        ],
        ..Default::default()
    };

    assert!(generate_entrances(&mut map) >= 1);
    let entrances = &map.buildings[0].entrances;
    // первая дверь — на лучшей грани, то есть на южной, у самой улицы
    assert!(
        (entrances[0].y - 100.0).abs() < 0.01,
        "first entrance not on the south facade: {:?}",
        entrances[0]
    );
    assert!((100.0..=120.0).contains(&entrances[0].x), "{entrances:?}");
    // и ни одна не ушла на северную, отвёрнутую от ближней улицы
    for entrance in entrances {
        assert!(
            (entrance.y - 120.0).abs() > 0.01,
            "entrance on the facade turned away from the road: {entrance:?}"
        );
    }
}

/// Одна и та же карта — одни и те же двери, сколько ни парси.
#[test]
fn generation_is_deterministic() {
    let make = || MapData {
        buildings: vec![
            building(
                rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
                Some(20.0),
            ),
            building(
                rect(Vec2::new(200.0, 100.0), Vec2::new(230.0, 115.0)),
                Some(4.0),
            ),
            building(rect(Vec2::new(300.0, 100.0), Vec2::new(380.0, 160.0)), None),
        ],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(500.0, 95.0)])],
        ..Default::default()
    };

    let (mut first, mut second) = (make(), make());
    generate_entrances(&mut first);
    generate_entrances(&mut second);
    for (a, b) in first.buildings.iter().zip(&second.buildings) {
        assert_eq!(a.entrances, b.entrances);
        assert!(!a.entrances.is_empty());
    }
}

/// Позиция двери не зависит от порядка разбора: тот же дом в выгрузке, где
/// соседей перечислили иначе, получает ту же дверь. Соседи здесь стоят
/// далеко и стен не загораживают.
#[test]
fn a_building_keeps_its_entrances_regardless_of_its_neighbours() {
    let alone = building(
        rect(Vec2::new(100.0, 100.0), Vec2::new(160.0, 130.0)),
        Some(20.0),
    );
    let roads = vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(500.0, 95.0)])];

    let mut solo = MapData {
        buildings: vec![alone.clone()],
        roads: roads.clone(),
        ..Default::default()
    };
    let mut crowded = MapData {
        buildings: vec![
            building(rect(Vec2::new(300.0, 200.0), Vec2::new(340.0, 240.0)), None),
            alone,
            building(
                rect(Vec2::new(400.0, 200.0), Vec2::new(460.0, 260.0)),
                Some(30.0),
            ),
        ],
        roads,
        ..Default::default()
    };

    generate_entrances(&mut solo);
    generate_entrances(&mut crowded);
    assert_eq!(solo.buildings[0].entrances, crowded.buildings[1].entrances);
}

/// Размеченную в OSM дверь генератор не двигает и не выбрасывает: она стоит
/// на своём месте и идёт первой.
#[test]
fn a_real_entrance_stays_where_it_was_mapped() {
    let real = Vec2::new(110.0, 100.0);
    let mut house = building(rect(Vec2::new(100.0, 100.0), Vec2::new(120.0, 120.0)), None);
    house.entrances = vec![real];
    let mut map = MapData {
        buildings: vec![house],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(400.0, 95.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    assert_eq!(map.buildings[0].entrances[0], real);
}

/// Регресс на лондонский квартал: дом в четверть километра, а вход в нём
/// размечен **один** — и такой дом генератор пропускал целиком, оставляя дверь
/// одну на все 250 метров. Разметка в OSM бросается на полпути сплошь и рядом
/// (тот же порог «не меньше двух дверей» стоит и в замере когорт), так что
/// одна размеченная дверь — это не «дом размечен», а «дом размечен наполовину».
#[test]
fn a_block_mapped_with_one_door_gets_the_rest() {
    let real = Vec2::new(120.0, 100.0);
    // 250 × 16 = 4000 м², 9 этажей
    let mut block = building(
        rect(Vec2::new(100.0, 100.0), Vec2::new(350.0, 116.0)),
        Some(27.0),
    );
    block.entrances = vec![real];
    let mut map = MapData {
        buildings: vec![block],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(600.0, 95.0)])],
        ..Default::default()
    };

    assert!(generate_entrances(&mut map) >= 5);
    let street = doors_on(&map, 100.0);
    assert!(street.contains(&real), "{street:?}");
    assert!(street.len() >= 6, "250 m block got {} doors", street.len());
    for (index, &door) in street.iter().enumerate() {
        for &other in &street[index + 1..] {
            assert!(
                door.distance(other) >= ENTRANCE_MIN_SPACING,
                "doors too close: {door:?} vs {other:?}"
            );
        }
    }
}

/// Когорты: сарай получает одну дверь, многоэтажный корпус — несколько, и
/// они разнесены не ближе минимального зазора.
#[test]
fn cohorts_scale_the_door_count_with_the_building() {
    let mut map = MapData {
        buildings: vec![
            // 10 × 8 = 80 м², сарай
            building(
                rect(Vec2::new(100.0, 100.0), Vec2::new(110.0, 108.0)),
                Some(3.0),
            ),
            // 90 × 15 = 1350 м² при 27 м — корпус с подъездами
            building(
                rect(Vec2::new(200.0, 100.0), Vec2::new(290.0, 115.0)),
                Some(27.0),
            ),
        ],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(500.0, 95.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    assert_eq!(map.buildings[0].entrances.len(), 1);

    let block = &map.buildings[1].entrances;
    assert!(block.len() >= 2, "a tall block got {} doors", block.len());
    for (index, &entrance) in block.iter().enumerate() {
        for &other in &block[index + 1..] {
            assert!(
                entrance.distance(other) >= ENTRANCE_MIN_SPACING,
                "doors too close: {entrance:?} vs {other:?}"
            );
        }
    }
}

/// Длина — главная ось когорты: «дом-корабль» 200 × 14 м обязан получить
/// больше дверей, чем компактный корпус той же площади, и они должны лечь
/// вдоль уличного фасада, а не сбиться в углу.
#[test]
fn a_long_slab_gets_more_doors_than_a_compact_building_of_the_same_area() {
    let street = || road(vec![Vec2::new(0.0, 95.0), Vec2::new(600.0, 95.0)]);

    // 200 × 14 = 2800 м², длина ~200 м
    let mut slab = MapData {
        buildings: vec![building(
            rect(Vec2::new(100.0, 100.0), Vec2::new(300.0, 114.0)),
            Some(20.0),
        )],
        roads: vec![street()],
        ..Default::default()
    };
    // 53 × 53 = 2800 м², та же площадь, длина ~53 м
    let mut compact = MapData {
        buildings: vec![building(
            rect(Vec2::new(100.0, 100.0), Vec2::new(153.0, 153.0)),
            Some(20.0),
        )],
        roads: vec![street()],
        ..Default::default()
    };

    generate_entrances(&mut slab);
    generate_entrances(&mut compact);
    // считаем по уличной стене: у корпуса подъезд сквозной, и его дворовая
    // половина — та же дверь, а не лишняя
    let slab_doors = doors_on(&slab, 100.0);
    let compact_doors = doors_on(&compact, 100.0);

    assert!(
        slab_doors.len() > compact_doors.len(),
        "slab got {} doors, compact {} — length is not driving the cohort",
        slab_doors.len(),
        compact_doors.len()
    );
    assert!(slab_doors.len() >= 3, "{slab_doors:?}");
    // и ни одна дверь не ушла на торец: подъезды стоят вдоль длинных стен
    for door in &slab.buildings[0].entrances {
        assert!(
            (door.y - 100.0).abs() < 0.01 || (door.y - 114.0).abs() < 0.01,
            "slab door off the long facades: {door:?}"
        );
    }
}

/// Регресс на реальный дефект: 250-метровый корпус получал 3 двери, то
/// есть дверь раз в 80 м. Такого дома не бывает — замер по домам с
/// доведённой разметкой даёт при 200 м и длиннее 4.42 двери.
#[test]
fn a_giant_slab_does_not_get_a_door_once_per_hundred_metres() {
    // 250 × 16 = 4000 м², 9 этажей
    let mut map = MapData {
        buildings: vec![building(
            rect(Vec2::new(100.0, 100.0), Vec2::new(350.0, 116.0)),
            Some(27.0),
        )],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(600.0, 95.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    let doors = doors_on(&map, 100.0);
    assert!(
        doors.len() >= 6,
        "250 m slab got only {} doors",
        doors.len()
    );

    // и шаг между ними — человеческий, а не «раз в сотню метров»
    let mut sorted: Vec<f32> = doors.iter().map(|door| door.x).collect();
    sorted.sort_by(f32::total_cmp);
    for pair in sorted.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (ENTRANCE_MIN_SPACING..=45.0).contains(&gap),
            "gap between doors is {gap} m: {doors:?}"
        );
    }
}

/// Закон шага: метров длины на дверь держится около `ENTRANCE_SPACING` на
/// всём диапазоне размеров — как у тульских подъездов, где замер даёт
/// 21.8–27.4 м во всех полосах длины. Ломалось именно это: на длинных
/// домах шаг уезжал в сотню метров.
#[test]
fn the_pitch_between_doors_holds_across_building_sizes() {
    for length in [60.0f32, 100.0, 160.0, 240.0] {
        let mut map = MapData {
            buildings: vec![building(
                rect(Vec2::new(100.0, 100.0), Vec2::new(100.0 + length, 116.0)),
                Some(27.0),
            )],
            roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(600.0, 95.0)])],
            ..Default::default()
        };
        generate_entrances(&mut map);
        let doors = doors_on(&map, 100.0).len();
        let pitch = length / doors as f32;
        assert!(
            (15.0..=45.0).contains(&pitch),
            "{length} m building got {doors} doors — {pitch:.0} m per door"
        );
    }
}

/// Длинный, но мелкий — это ряд гаражей, а не жилой корпус: площадь
/// возвращает его в скромную когорту.
#[test]
fn a_long_but_tiny_building_stays_in_a_modest_cohort() {
    // 100 × 4 = 400 м² при длине ~100 м
    let long_and_thin = cohort_of(400.0, 100.0, Some(4.0));
    let proper_block = cohort_of(2000.0, 100.0, Some(4.0));
    assert!(long_and_thin.mean < proper_block.mean);
    assert_eq!(long_and_thin.mean, COHORT_ROW_MEAN);
}

/// «Длина» не зависит от поворота дома: у AABB диагональный корпус вдвое
/// длиннее, чем он есть.
#[test]
fn equivalent_length_reads_a_rectangle_regardless_of_its_rotation() {
    // 200 × 14: площадь 2800, периметр 428
    let length = equivalent_length(2800.0, 2.0 * (200.0 + 14.0));
    assert!((length - 200.0).abs() < 0.5, "{length}");
    // квадрат 53 × 53 той же площади — длина стороны, а не диагональ
    let square = equivalent_length(2809.0, 4.0 * 53.0);
    assert!((square - 53.0).abs() < 0.5, "{square}");
}

/// Регресс на реальный дефект: два дома стоят вплотную, и у правого дверь
/// оказывалась на общей стене — то есть внутри левого дома. Стена, за
/// которой сразу сосед, глухая, дверь обязана уйти на свободную грань.
#[test]
fn a_door_never_lands_on_a_wall_a_neighbour_stands_against() {
    // улица идёт с запада, так что для правого дома лучшей по оценке
    // грань оказывается западная — она же общая с левым домом
    let mut map = MapData {
        buildings: vec![
            building(rect(Vec2::new(100.0, 100.0), Vec2::new(140.0, 130.0)), None),
            building(rect(Vec2::new(140.0, 100.0), Vec2::new(180.0, 130.0)), None),
        ],
        roads: vec![road(vec![Vec2::new(95.0, 0.0), Vec2::new(95.0, 400.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    let right = &map.buildings[1].entrances;
    assert!(!right.is_empty(), "the right building got no door at all");
    for door in right {
        assert!(
            (door.x - 140.0).abs() > 0.01,
            "door on the wall shared with the neighbour: {door:?}"
        );
    }
    // левому дому общая стена мешает ровно так же
    for door in &map.buildings[0].entrances {
        assert!((door.x - 140.0).abs() > 0.01, "{door:?}");
    }
}

/// Дом, накрытый чужим контуром целиком (в OSM так обводят квартал поверх
/// корпусов), дверь всё равно получает — без неё он выпал бы из целей
/// блуждания.
#[test]
fn a_building_walled_in_on_every_side_still_gets_a_door() {
    let mut map = MapData {
        buildings: vec![
            building(rect(Vec2::new(0.0, 0.0), Vec2::new(300.0, 300.0)), None),
            building(rect(Vec2::new(100.0, 100.0), Vec2::new(130.0, 130.0)), None),
        ],
        roads: vec![road(vec![Vec2::new(0.0, 350.0), Vec2::new(400.0, 350.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    assert!(!map.buildings[1].entrances.is_empty());
}

/// Дом в глубине квартала, вокруг ни одной дороги, всё равно получает
/// дверь — иначе он выпал бы из целей блуждания.
#[test]
fn a_building_with_no_road_in_reach_still_gets_a_door() {
    let mut map = MapData {
        buildings: vec![building(
            rect(Vec2::new(100.0, 100.0), Vec2::new(130.0, 130.0)),
            None,
        )],
        roads: Vec::new(),
        ..Default::default()
    };

    assert!(generate_entrances(&mut map) >= 1);
    for entrance in &map.buildings[0].entrances {
        assert!(
            (99.9..=130.1).contains(&entrance.x) && (99.9..=130.1).contains(&entrance.y),
            "{entrance:?}"
        );
    }
}

/// Юго-западный угол дома во всех замерах ниже: улица идёт по `y = 95`, то
/// есть вдоль южной стены.
const LOW: Vec2 = Vec2::new(100.0, 100.0);

/// Дом заданного размера, назначения и высоты у улицы на юге, с уже
/// расставленными входами.
fn block_by_the_street(depth: f32, length: f32, height: f32, building_use: BuildingUse) -> MapData {
    let mut block = building(rect(LOW, LOW + Vec2::new(length, depth)), Some(height));
    block.building_use = building_use;
    let mut map = MapData {
        buildings: vec![block],
        roads: vec![road(vec![Vec2::new(0.0, 95.0), Vec2::new(600.0, 95.0)])],
        ..Default::default()
    };
    generate_entrances(&mut map);
    map
}

/// Сколько дверей получил такой дом.
fn doors_of(depth: f32, length: f32, height: f32, building_use: BuildingUse) -> usize {
    block_by_the_street(depth, length, height, building_use).buildings[0]
        .entrances
        .len()
}

/// Пятиэтажный дом с улицей на юге: сколько дверей вышло на каждую из длинных
/// стен.
fn doors_by_side(depth: f32, length: f32, building_use: BuildingUse) -> (usize, usize) {
    let map = block_by_the_street(depth, length, 15.0, building_use);
    (
        doors_on(&map, LOW.y).len(),
        doors_on(&map, LOW.y + depth).len(),
    )
}

/// Подъезд панельной секции **сквозной**: та же дверь выходит и во двор, и
/// стоит она ровно напротив уличной.
#[test]
fn a_section_opens_into_its_courtyard_too() {
    let (street, yard) = doors_by_side(14.0, 60.0, BuildingUse::Apartments);
    assert!(
        street > 0 && yard == street,
        "{street} на улицу, {yard} во двор"
    );
}

/// Дом-свечка: квадратная высотка обходится **одним** входом, и приделывать
/// ему второй с изнанки нельзя — на сорок метров вглубь подъезд не бывает
/// сквозным.
#[test]
fn a_tower_keeps_its_entrances_on_one_side() {
    let (street, yard) = doors_by_side(44.0, 44.0, BuildingUse::Apartments);
    assert!(street > 0 && yard == 0, "{street} на улицу, {yard} во двор");
}

/// Гаражный ряд длинный, но пройти его насквозь нечем: четыре метра глубины —
/// это бокс, а не подъезд.
#[test]
fn a_shallow_row_is_not_walked_through() {
    let (street, yard) = doors_by_side(5.0, 60.0, BuildingUse::Other);
    assert!(street > 0 && yard == 0, "{street} на улицу, {yard} во двор");
}

/// Регресс на тульские девятиэтажки у портала: дом 32 × 32 м и 27 м высоты
/// (тысяча метров плана на девяти этажах — сотня с лишним квартир) получал
/// **одну** дверь, потому что когорта видит только длину, а «длина» такого
/// дома — 32 метра, полоса «дом, магазин». Считать подъезды надо по плану.
#[test]
fn a_tall_compact_block_gets_a_door_per_section_of_its_plan() {
    let doors = doors_of(32.0, 32.0, 27.0, BuildingUse::Apartments);
    assert!(doors >= 3, "девятиэтажка на 1000 м² плана: {doors} дверей");
}

/// Число входов **не падает** с ростом дома — ни по площади, ни по высоте.
/// Именно это ломалось: у высокого компактного дома дверей выходило меньше,
/// чем у низкого длинного вдвое меньшей площади.
#[test]
fn the_door_count_never_falls_as_the_building_grows() {
    let ladder = [
        // сарай, частный дом, секция, корпус, дом-корабль, и он же выше
        (8.0, 10.0, 3.0),
        (10.0, 12.0, 6.0),
        (14.0, 60.0, 15.0),
        (16.0, 90.0, 27.0),
        (16.0, 200.0, 27.0),
        (16.0, 250.0, 33.0),
    ];
    let mut previous = 0;
    for (depth, length, height) in ladder {
        let doors = doors_of(depth, length, height, BuildingUse::Apartments);
        assert!(
            doors >= previous,
            "{length} × {depth} × {height} м: {doors} дверей после {previous}"
        );
        previous = doors;
    }
    assert!(previous >= 8, "дом-корабль получил всего {previous} дверей");
}

/// Один вход бывает **только** у дома-свечки: компактный план и 15 этажей.
/// Всё прочее крупное жильё носит несколько подъездов.
#[test]
fn only_a_tall_tower_may_keep_a_single_entrance() {
    // свечка: 25 × 25 м, 50 м высоты — вестибюль на весь дом, вход может быть один
    let tower = doors_of(25.0, 25.0, 50.0, BuildingUse::Apartments);
    assert!(tower <= 2, "свечке хватает одного входа, а вышло {tower}");

    // а девятиэтажка того же плана — уже не свечка
    let block = doors_of(25.0, 25.0, 27.0, BuildingUse::Apartments);
    assert!(block >= 2, "девятиэтажка получила {block} вход");
}

/// Правило секций — про **жильё**: у одноэтажного склада в 3000 м² подъездов
/// не бывает, сколько бы плана он ни занимал.
#[test]
fn a_warehouse_gets_doors_by_its_walls_not_by_its_floor_plan() {
    let shed = doors_of(50.0, 60.0, 6.0, BuildingUse::Industrial);
    assert!(
        (1..=4).contains(&shed),
        "складу насчитали {shed} дверей, как жилым секциям"
    );
}

/// Ступенька контура — не фасад: подъезда на четырёхметровом уступе не бывает,
/// даже если он ближе всех к дороге. Именно так третий подъезд десятиэтажки
/// вставал на выступ у угла, пока длинная стена оставалась глухой.
#[test]
fn a_step_in_the_outline_is_not_a_facade() {
    // корпус 60 × 14 с четырёхметровым выступом посреди уличной стены —
    // выступ ближе к дороге, чем сама стена
    let mut block = building(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(128.0, 100.0),
            Vec2::new(128.0, 96.0),
            Vec2::new(132.0, 96.0),
            Vec2::new(132.0, 100.0),
            Vec2::new(160.0, 100.0),
            Vec2::new(160.0, 114.0),
            Vec2::new(100.0, 114.0),
        ],
        Some(27.0),
    );
    block.building_use = BuildingUse::Apartments;
    let mut map = MapData {
        buildings: vec![block],
        roads: vec![road(vec![Vec2::new(0.0, 90.0), Vec2::new(400.0, 90.0)])],
        ..Default::default()
    };

    generate_entrances(&mut map);
    let doors = &map.buildings[0].entrances;
    assert!(!doors.is_empty());
    for door in doors {
        assert!(
            (door.y - 96.0).abs() > 0.01,
            "подъезд на торце уступа: {door:?} из {doors:?}"
        );
    }
}

/// Высота — не поправка, а второй множитель: тот же план, вдвое выше дом —
/// больше подъездов. Ровно этого не хватало когорте, которая видит длину.
#[test]
fn height_moves_the_door_count_on_the_same_plan() {
    let plan = (30.0, 40.0);
    let low = doors_of(plan.0, plan.1, 15.0, BuildingUse::Apartments);
    let tall = doors_of(plan.0, plan.1, 27.0, BuildingUse::Apartments);
    let higher = doors_of(plan.0, plan.1, 51.0, BuildingUse::Apartments);
    assert!(
        low < tall && tall < higher,
        "{low} → {tall} → {higher} дверей на 5, 9 и 17 этажах"
    );
}

/// Частный дом сквозным подъездом не обзаводится, какой бы длины ни был.
#[test]
fn a_private_house_keeps_its_single_side() {
    let (street, yard) = doors_by_side(12.0, 45.0, BuildingUse::House);
    assert!(street > 0 && yard == 0, "{street} на улицу, {yard} во двор");
}
