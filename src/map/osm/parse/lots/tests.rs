use super::super::fixtures::{CITY, HALF};
use super::*;
use crate::map::osm::fixture::{Overpass, building, fence, rect, residential_half, square, street};
use crate::map::osm::model::{SidewalkSide, sidewalk_band};
use crate::map::osm::parse::ParseKnobs;
use crate::settings::MAP_SIZE;

/// Центр карты, как у фикстур разбора: сцены собираются вокруг него, и в
/// тех же метрах пишутся проверки.
const CENTER: Vec2 = Vec2::new(MAP_SIZE.x / 2.0, MAP_SIZE.y / 2.0);

/// Прогон прохода на сцене из дорог, домов, заборов и стоянок: зелень и
/// вода — пустые срезы, их сцены мощения не строят. Сцену без домов и
/// заборов читай как «в контексте одни дороги» — тесты выше показывают,
/// что именно входит в [`PaveLotsCtx`].
fn pave(
    roads: &[RoadLine],
    buildings: &[PolyArea],
    fences: &[FenceLine],
    parking: &mut Vec<PolyArea>,
) -> PavedLots {
    pave_lots(PaveLotsCtx {
        roads,
        buildings,
        parks: &[],
        woods: &[],
        grass: &[],
        sand: &[],
        water: &[],
        fences,
        parking,
    })
}

/// Контекст — и есть договор прохода: на пустых срезах нет ни дорог для
/// роста, ни домов для отмостки, ни зелени с заборами для препятствий, и
/// стоянка остаётся тем же кольцом. Тест строит [`PaveLotsCtx`] буквально,
/// без карты, — форма контекста видна сама.
#[test]
fn an_empty_context_leaves_the_lots_alone() {
    let mut parking = vec![PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-20.0, -20.0),
                CENTER + Vec2::new(20.0, 20.0),
            ),
            Vec::new(),
        )
    }];
    let before = parking[0].outer.clone();
    let report = pave_lots(PaveLotsCtx {
        roads: &[],
        buildings: &[],
        parks: &[],
        woods: &[],
        grass: &[],
        sand: &[],
        water: &[],
        fences: &[],
        parking: &mut parking,
    });
    assert_eq!(report, PavedLots::default());
    assert_eq!(parking.len(), 1);
    assert_eq!(parking[0].outer, before, "контур тронут без дорог и домов");
}

/// Карман вдоль улицы отдельным контуром (`parking=street_side`, Тула,
/// Советская улица) стоял за полосой нарисованного тротуара, и въехать в
/// него было не с чего. Он дотягивается до бордюра проезжей части; обычная
/// стоянка на том же месте — по-прежнему до края тротуара.
#[test]
fn a_street_side_lot_reaches_the_kerb_across_the_sidewalk() {
    let sidewalk = sidewalk_band(8.0);
    let top = |street_side: bool| {
        let lot = PolyArea {
            kind: AreaKind::Parking(if street_side {
                LotKind::Kerbside
            } else {
                LotKind::Yard
            }),
            ..building(
                rect(
                    CENTER + Vec2::new(-20.0, -4.0 - sidewalk - 2.5),
                    CENTER + Vec2::new(20.0, -4.0 - sidewalk - 0.1),
                ),
                Vec::new(),
            )
        };
        let roads = vec![street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            8.0,
        )];
        let mut parking = vec![lot];
        pave(&roads, &[], &[], &mut parking);
        // посреди полосы тротуара и у самого бордюра
        [0.5, 0.1].map(|share| {
            let probe = CENTER + Vec2::new(0.0, -4.0 - sidewalk * share);
            point_in_polygon(probe, &parking[0].outer)
        })
    };
    assert_eq!(top(true), [true; 2], "карман не дошёл до бордюра");
    assert_eq!(top(false), [false; 2], "обычная стоянка вылезла на тротуар");
}

/// Бордюр кармана — только у улицы, вдоль которой он лежит. Поперечная
/// улица у его торца, в радиусе замыкания, остаётся с тротуаром: асфальт
/// кармана доходит до края её тротуара, а не заливает угол до бордюра.
#[test]
fn a_street_side_lot_leaves_a_cross_street_sidewalk() {
    let sidewalk = sidewalk_band(8.0);
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Kerbside),
        ..building(
            rect(
                CENTER + Vec2::new(-20.0, -4.0 - sidewalk - 2.5),
                CENTER + Vec2::new(20.0, -4.0 - sidewalk - 0.1),
            ),
            Vec::new(),
        )
    };
    let roads = vec![
        street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            8.0,
        ),
        // поперечная: бордюр при x = 24, тротуар в 2.2 м от торца кармана
        street(
            vec![
                CENTER + Vec2::new(28.0, -60.0),
                CENTER + Vec2::new(28.0, 60.0),
            ],
            8.0,
        ),
    ];
    let mut parking = vec![lot];
    pave(&roads, &[], &[], &mut parking);
    let paved = |probe: Vec2| point_in_polygon(CENTER + probe, &parking[0].outer);
    assert!(
        paved(Vec2::new(0.0, -4.0 - sidewalk * 0.5)),
        "карман не дошёл до бордюра своей улицы"
    );
    assert!(
        !paved(Vec2::new(24.0 - sidewalk * 0.5, -4.0 - sidewalk - 1.3)),
        "карман залил тротуар поперечной улицы"
    );
}

/// Большая ли стоянка, решает **замощённый** контур, а не нарисованный: у
/// площадки в 7800 м² за три метра от улицы полоса до тротуара — тоже её
/// асфальт, и вместе с ней она большая.
#[test]
fn a_lot_paved_past_the_threshold_is_a_big_lot() {
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-50.0, -edge - 3.0 - 78.0),
                CENTER + Vec2::new(50.0, -edge - 3.0),
            ),
            Vec::new(),
        )
    };
    assert!(!crate::map::parking::is_ground(&lot));
    let roads = vec![street(
        vec![
            CENTER - Vec2::new(400.0, 0.0),
            CENTER + Vec2::new(400.0, 0.0),
        ],
        2.0 * residential_half(ParseKnobs::DEFAULT),
    )];
    let mut parking = vec![lot];
    pave(&roads, &[], &[], &mut parking);
    assert!(crate::map::parking::is_ground(&parking[0]));
}

/// Тем же проходом дотягивается и стоянка — но по своему правилу: её край
/// пересекает проезд ряда, и вершина, стоящая на его полотне, обязана
/// уехать к улице вместе с соседями. По правилу квартала («лежишь под
/// лентой — тянуть некуда») она осталась бы на месте, и вдоль улицы вышла
/// бы пила.
#[test]
fn a_lot_reaches_the_road_across_its_own_aisle() {
    let edge = residential_half(ParseKnobs::DEFAULT)
        + sidewalk_band(2.0 * residential_half(ParseKnobs::DEFAULT));
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-40.0, -40.0),
                CENTER + Vec2::new(40.0, -12.0),
            ),
            Vec::new(),
        )
    };
    let roads = vec![
        street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            8.0,
        ),
        // проезд ряда: упирается в улицу, пересекая верхний край стоянки
        street(
            vec![
                CENTER + Vec2::new(0.0, -30.0),
                CENTER + Vec2::new(0.0, -6.0),
            ],
            5.0,
        ),
    ];
    let mut parking = vec![lot];

    // именно рост: площадка дотянулась до улицы, а домов, от которых можно
    // было бы отступить, в сцене нет
    assert_eq!(
        pave(&roads, &[], &[], &mut parking),
        PavedLots {
            grown: 1,
            trimmed: 0
        }
    );
    let lot = &parking[0];
    // вся полоса между стоянкой и улицей — асфальт, без зубцов у проезда
    // (под самим проездом асфальт его собственный: полотно в контур не
    // входит)…
    for x in (-35..=35).step_by(5).filter(|x| *x != 0) {
        let at = CENTER + Vec2::new(x as f32, -edge - 0.5);
        assert!(point_in_area(at, lot), "полоса у улицы не залита: x = {x}");
    }
    // …и край заведён под тротуар ровно на запас, не дальше
    for vertex in &lot.outer {
        let y = vertex.y - CENTER.y;
        assert!(
            y <= -edge + LANDUSE_OVERLAP + 0.05,
            "край стоянки вылез на тротуар: {y}"
        );
    }
}

/// Карман между торцами двух проездов, не доходящих до объездной, —
/// асфальт: замыкание затягивает его целиком, а не тянет к дороге каждую
/// вершину порознь, из-за чего край и выходил зубцами.
#[test]
fn the_pocket_between_two_aisle_stubs_is_paved() {
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-40.0, -60.0),
                CENTER + Vec2::new(40.0, -20.0),
            ),
            Vec::new(),
        )
    };
    let aisle = |x: f32| {
        street(
            vec![CENTER + Vec2::new(x, -50.0), CENTER + Vec2::new(x, 0.0)],
            5.0,
        )
    };
    let roads = vec![
        // объездная в двадцати метрах от кромки: повершинно не дотянуться
        street(
            vec![
                CENTER - Vec2::new(400.0, 0.0),
                CENTER + Vec2::new(400.0, 0.0),
            ],
            5.0,
        ),
        aisle(-8.5),
        aisle(8.5),
    ];
    let mut parking = vec![lot];

    // карман затянут ростом к дорогам, отмостке тут резать нечего
    assert_eq!(
        pave(&roads, &[], &[], &mut parking),
        PavedLots {
            grown: 1,
            trimmed: 0
        }
    );
    for y in [-18.0, -12.0, -6.0] {
        let at = CENTER + Vec2::new(0.0, y);
        assert!(point_in_area(at, &parking[0]), "карман не залит: y = {y}");
    }
}

/// Заливка тоньше остаточной полосы — не асфальт: зазор в 1.1 м между
/// кромкой площадки и полотном улицы замыкание наливает языком вдоль всей
/// кромки, и читается он обрубком асфальта поверх чужой плитки (Тула,
/// ≈4454,3855 — язык 1.1 м у торца стоянки, N3; там же семья — воротник
/// 2.3→0 м у устья проезда, N9). Честная полоса шире — растёт целиком.
#[test]
fn a_fill_thinner_than_the_remnant_stays_unpaved() {
    let half = residential_half(ParseKnobs::DEFAULT);
    let edge = half + sidewalk_band(2.0 * half);
    // кромка площадки в 1.1 м за полотном улицы (ось на нуле)
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(0.0, -edge - 1.1),
                CENTER + Vec2::new(40.0, -30.0),
            ),
            Vec::new(),
        )
    };
    let roads = vec![street(
        vec![
            CENTER - Vec2::new(400.0, 0.0),
            CENTER + Vec2::new(400.0, 0.0),
        ],
        8.0,
    )];
    let mut parking = vec![lot];

    pave(&roads, &[], &[], &mut parking);
    let tongue = CENTER + Vec2::new(20.0, -edge - 0.5);
    assert!(
        !point_in_area(tongue, &parking[0]),
        "язык в 1.1 м у кромки налит асфальтом",
    );
}

/// Выемка в самом контуре дороги не касается и асфальтом не становится:
/// так Г-образная стоянка не закрашивает газон в своём углу.
#[test]
fn a_notch_in_the_lot_is_not_paved() {
    let base = CENTER + Vec2::new(0.0, -60.0);
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            vec![
                base + Vec2::new(-40.0, 0.0),
                base + Vec2::new(40.0, 0.0),
                base + Vec2::new(40.0, 40.0),
                base + Vec2::new(5.0, 40.0),
                base + Vec2::new(5.0, 20.0),
                base + Vec2::new(-5.0, 20.0),
                base + Vec2::new(-5.0, 40.0),
                base + Vec2::new(-40.0, 40.0),
            ],
            Vec::new(),
        )
    };
    let roads = vec![street(
        vec![
            base + Vec2::new(-400.0, -10.0),
            base + Vec2::new(400.0, -10.0),
        ],
        5.0,
    )];
    let mut parking = vec![lot];

    pave(&roads, &[], &[], &mut parking);
    assert!(!point_in_area(base + Vec2::new(0.0, 30.0), &parking[0]));
}

/// Асфальт стоянки под дом не лезет, какой бы тот ни был: контур,
/// нарисованный внахлёст с домом, отступает от стены на отмостку, а будка
/// посреди площадки получает свой остров. Дороги для этого не нужно —
/// проход отступает и там, где дотягивать нечего.
#[test]
fn a_lot_steps_back_from_the_houses_on_it() {
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-40.0, -40.0),
                CENTER + Vec2::new(40.0, 40.0),
            ),
            Vec::new(),
        )
    };
    // корпус заходит на площадку с запада на десять метров, будка — посреди
    let buildings = vec![
        building(
            rect(
                CENTER + Vec2::new(-70.0, -10.0),
                CENTER + Vec2::new(-30.0, 10.0),
            ),
            Vec::new(),
        ),
        building(
            rect(
                CENTER + Vec2::new(10.0, 10.0),
                CENTER + Vec2::new(16.0, 17.0),
            ),
            Vec::new(),
        ),
    ];
    let mut parking = vec![lot];

    // счётчик разделён по событиям: дорог в сцене нет, так что расти
    // площадке некуда — её тронула одна отмостка, и попасть она обязана в
    // `trimmed`
    assert_eq!(
        pave(&[], &buildings, &[], &mut parking),
        PavedLots {
            grown: 0,
            trimmed: 1
        }
    );
    let lot = &parking[0];
    // под корпусом и в полуметре от его стены асфальта нет, в трёх метрах
    // — есть
    assert!(!point_in_area(CENTER + Vec2::new(-35.0, 0.0), lot));
    assert!(!point_in_area(CENTER + Vec2::new(-29.5, 0.0), lot));
    assert!(point_in_area(CENTER + Vec2::new(-27.0, 0.0), lot));
    // будка — дырка в площадке, с той же отмосткой
    assert!(!point_in_area(CENTER + Vec2::new(13.0, 13.0), lot));
    assert!(!point_in_area(CENTER + Vec2::new(9.5, 13.0), lot));
    assert!(point_in_area(CENTER + Vec2::new(7.0, 13.0), lot));
    // дальний от домов край остался где был
    assert!(point_in_area(CENTER + Vec2::new(39.5, -39.5), lot));
}

/// Обнесённая забором стоянка за него не выходит: в OSM её контур лежит
/// **по** забору, и вершина, стоящая на нём, с него не сходит — иначе
/// асфальт вылезает в соседний сквер.
#[test]
fn a_fenced_lot_stays_behind_its_fence() {
    let top = CENTER.y - 12.0;
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-40.0, -40.0),
                CENTER + Vec2::new(40.0, -12.0),
            ),
            Vec::new(),
        )
    };
    let roads = vec![street(
        vec![
            CENTER - Vec2::new(400.0, 0.0),
            CENTER + Vec2::new(400.0, 0.0),
        ],
        8.0,
    )];
    // забор по верхнему краю стоянки, между ней и улицей
    let fences = vec![fence(vec![
        Vec2::new(CENTER.x - 60.0, top),
        Vec2::new(CENTER.x + 60.0, top),
    ])];
    let mut parking = vec![lot];

    let before = parking[0].outer.len();
    assert_eq!(
        pave(&roads, &[], &fences, &mut parking),
        PavedLots::default(),
        "забор отрезал добавленное: ни роста, ни отмостки — прохода не было"
    );
    // добавлять нечего — контур остался тем же кольцом, и ни одна вершина
    // не оказалась за забором
    assert_eq!(parking[0].outer.len(), before);
    for vertex in &parking[0].outer {
        assert!(vertex.y <= top + 0.01, "вершина за забором: {vertex:?}");
    }
}

/// Вершина, стоящая **в стороне** от забора, тоже за него не уходит: она
/// тянется к улице на десяток метров и перешагивает ограду по дороге.
#[test]
fn a_lot_does_not_step_over_a_fence_it_was_not_standing_on() {
    let fence_y = CENTER.y - 16.0;
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            rect(
                CENTER + Vec2::new(-40.0, -40.0),
                CENTER + Vec2::new(40.0, -20.0),
            ),
            Vec::new(),
        )
    };
    let roads = vec![street(
        vec![
            CENTER - Vec2::new(400.0, 0.0),
            CENTER + Vec2::new(400.0, 0.0),
        ],
        8.0,
    )];
    // забор в четырёх метрах от края стоянки, между ней и улицей
    let fences = vec![fence(vec![
        Vec2::new(CENTER.x - 60.0, fence_y),
        Vec2::new(CENTER.x + 60.0, fence_y),
    ])];
    let mut parking = vec![lot];

    assert_eq!(
        pave(&roads, &[], &fences, &mut parking),
        PavedLots::default(),
        "забор отрезал добавленное: ни роста, ни отмостки — прохода не было"
    );
    for vertex in &parking[0].outer {
        assert!(vertex.y <= fence_y + 0.01, "вершина за забором: {vertex:?}");
    }
}

/// Клин в кармане забора у торца стоянки асфальтом не становится. Тула,
/// 441737398 у Кремлёвского сада: длинная кромка стоянки лежит на оси
/// Садового переулка, у северного торца забор обходит карман, а за углом
/// начинается улица Дзержинского. Замыкание затягивало карман, полоса
/// забора оставляла от него клин уже самой ограды, и тот торчал из угла
/// стоянки серым шипом на тротуар — отчёт автора со скриншота.
#[test]
fn a_fence_pocket_at_a_lot_end_leaves_no_asphalt_sliver() {
    // настоящие метры карты, сдвинутые к центру сцены
    let at = |x: f32, y: f32| CENTER + Vec2::new(x - 5818.0, y - 3240.0);
    let lot = PolyArea {
        kind: AreaKind::Parking(LotKind::Yard),
        ..building(
            vec![
                at(5775.27, 3192.40),
                at(5779.90, 3184.04),
                at(5811.05, 3220.77),
                at(5827.75, 3240.47),
                at(5864.04, 3283.25),
                at(5858.17, 3287.70),
            ],
            Vec::new(),
        )
    };
    let bare = |points: Vec<Vec2>, width: f32| RoadLine {
        sidewalks: [SidewalkSide::None; 2],
        ..street(points, width)
    };
    let roads = vec![
        // Садовый переулок — по кромке стоянки
        bare(
            vec![
                at(5864.04, 3283.25),
                at(5827.75, 3240.47),
                at(5811.05, 3220.77),
                at(5779.90, 3184.04),
                at(5774.87, 3178.77),
            ],
            7.6,
        ),
        // улица Дзержинского — от угла стоянки в обе стороны
        bare(
            vec![
                at(5879.56, 3271.45),
                at(5871.60, 3277.50),
                at(5868.61, 3279.77),
                at(5864.04, 3283.25),
                at(5871.12, 3287.65),
                at(5878.69, 3296.93),
            ],
            10.94,
        ),
    ];
    let fences = vec![fence(vec![
        at(5775.3, 3192.4),
        at(5858.2, 3287.7),
        at(5861.1, 3290.8),
        at(5865.1, 3287.3),
        at(5874.0, 3297.3),
        at(5870.6, 3299.8),
    ])];
    let mut parking = vec![lot];

    let original = parking[0].clone();
    pave(&roads, &[], &fences, &mut parking);
    // у северного торца контур остался своим: ни одной вершины в кармане
    let corner = at(5858.17, 3287.70);
    let off = |vertex: Vec2| {
        if point_in_area(vertex, &original) {
            return 0.0;
        }
        (0..original.outer.len())
            .map(|index| {
                let next = original.outer[(index + 1) % original.outer.len()];
                distance_to_segment(vertex, original.outer[index], next)
            })
            .fold(f32::INFINITY, f32::min)
    };
    let farthest = parking[0]
        .outer
        .iter()
        .filter(|vertex| vertex.distance(corner) < 8.0)
        .map(|&vertex| off(vertex))
        .fold(0.0, f32::max);
    assert!(
        farthest < 0.3,
        "асфальт в кармане забора, {farthest:.2} м от стоянки"
    );
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
    // 110 × 110 м — от `GROUND_MIN_AREA`, большая
    assert_eq!(map.parking[0].kind, AreaKind::Parking(LotKind::Ground));
    assert_eq!(map.buildings.len(), 1);
    assert_eq!(map.parks.len(), 1);
    // в кварталы стоянка не падает: до ветки `landuse` дело не доходит
    assert!(map.landuse.is_empty());
}

/// Вид стоянки решает разбор: `parking=street_side` — карман вдоль улицы,
/// прочая небольшая — двор, что бы ни стояло в `parking`.
#[test]
fn a_street_side_lot_is_kerbside_and_a_small_lot_is_a_yard() {
    let map = Overpass::new(CITY)
        .area(
            &[("amenity", "parking"), ("parking", "street_side")],
            rect(CENTER, CENTER + Vec2::new(40.0, 4.0)),
        )
        .area(
            &[("amenity", "parking"), ("parking", "surface")],
            square(CENTER + Vec2::new(0.0, 60.0), 10.0),
        )
        .parse();
    let kinds: Vec<AreaKind> = map.parking.iter().map(|lot| lot.kind).collect();
    assert_eq!(
        kinds,
        [
            AreaKind::Parking(LotKind::Kerbside),
            AreaKind::Parking(LotKind::Yard)
        ]
    );
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
