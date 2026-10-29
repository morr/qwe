use super::super::fixtures::CENTER;
use super::super::pockets;
use crate::map::osm::fixture::{building, rect, street};
use crate::map::osm::model::{
    AreaKind, LotKind, MapData, Pavement, PolyArea, RoadLine, SidewalkSide, sidewalk_band,
};
use bevy::math::Vec2;

/// Сцена кармана: квартал, чей северо-западный угол срезан диагональю, и две
/// дорожки по его краям — вдоль верхнего и вдоль левого, сходящиеся над
/// срезанным углом. Катет среза — `cut` м; между диагональю и дорожками
/// остаётся треугольник земли. `rough` — левая дорожка грунтовая.
fn pocket_scene(cut: f32, rough: bool) -> MapData {
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    let path = |points: Vec<Vec2>, pavement: Pavement| RoadLine {
        pavement: Some(pavement),
        ..crate::map::osm::fixture::footway(points)
    };
    let left = if rough {
        Pavement::Unpaved
    } else {
        Pavement::Paved
    };
    // край квартала — под дорожками: полоса дорожки кроет его на метр
    let block = PolyArea {
        kind: AreaKind::Residential,
        ..building(
            vec![
                at(-1.0, -1.0 - cut),
                at(-1.0, -80.0),
                at(80.0, -80.0),
                at(80.0, 1.0),
                at(cut - 1.0, 1.0),
            ],
            Vec::new(),
        )
    };
    MapData {
        roads: vec![
            path(vec![at(-2.0, 2.0), at(100.0, 2.0)], Pavement::Paved),
            path(vec![at(-2.0, 2.0), at(-2.0, -100.0)], left),
        ],
        landuse: vec![block],
        ..MapData::default()
    }
}

/// Засеян ли точкой `(x, y)` от центра хоть один карман.
fn sown_at(map: &MapData, x: f32, y: f32) -> bool {
    map.pockets
        .iter()
        .any(|area| crate::map::osm::model::point_in_polygon(CENTER + Vec2::new(x, y), &area.outer))
}

/// Треугольник земли между срезанным углом квартала и двумя мощёными
/// дорожками — карман: он засевается травой двора того же вида, и край его
/// заходит под дорожки. Тула, витрина 02: квартал 164045103 не доходит до
/// дорожки десяти метров.
#[test]
fn a_ground_pocket_between_a_block_and_two_footways_is_sown_as_yard() {
    let mut map = pocket_scene(16.0, false);
    let sown = pockets::fill_ground_pockets(&mut map);
    assert_eq!(sown, 1, "карманов засеяно: {sown}");
    // карман — не квартал: обочина спрашивает двор только у настоящих
    assert_eq!(map.landuse.len(), 1);
    let pocket = &map.pockets[0];
    assert_eq!(pocket.kind, AreaKind::Residential);
    assert!(sown_at(&map, 4.0, -4.0), "середина клина осталась землёй");
    // заведён под дорожку: край кармана заходит за край её полосы (0.25 м)
    let top = pocket
        .outer
        .iter()
        .map(|point| point.y - CENTER.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(top > 0.5, "край кармана не под дорожкой: {top}");
}

/// Карман без квартала рядом засевается тоже — по правилу газона обочины:
/// у замапленного газона или сквера он вливается в зелень (см.
/// `a_meadow_pocket_touching_a_lawn_is_merged_into_it`), у чего-то другого
/// (тут стоянка) сеется травой двора. Раньше без квартала его засеять было
/// нечем, и бежевый параллелограмм оставался между газонами обочин
/// (Калуга 01).
#[test]
fn a_pocket_with_no_block_beside_is_meadow_by_a_lawn_and_yard_elsewhere() {
    let scene_beside = |kind: AreaKind, cut: f32| {
        let mut map = pocket_scene(cut, false);
        let mut area = map.landuse.pop().expect("квартал сцены");
        area.kind = kind;
        match kind {
            AreaKind::Park => map.parks.push(area),
            AreaKind::Grass => map.grass.push(area),
            _ => map.parking.push(area),
        }
        map
    };
    // у зелени клин вливается в неё: середина клина — внутри зелени, не
    // отдельным карманом
    let merged_beside = |kind: AreaKind| {
        let mut map = scene_beside(kind, 16.0);
        pockets::fill_ground_pockets(&mut map);
        let green = if matches!(kind, AreaKind::Park) {
            &map.parks[0]
        } else {
            &map.grass[0]
        };
        assert!(
            crate::map::osm::model::point_in_polygon(CENTER + Vec2::new(4.0, -4.0), &green.outer),
            "середина клина у {kind:?} не влилась в зелень"
        );
        assert!(
            !sown_at(&map, 4.0, -4.0),
            "клин у {kind:?} сеется поверх зелени"
        );
    };
    merged_beside(AreaKind::Park);
    merged_beside(AreaKind::Grass);
    let mut by_lot = scene_beside(AreaKind::Parking(LotKind::Yard), 16.0);
    assert!(
        pockets::fill_ground_pockets(&mut by_lot) >= 1,
        "у стоянки не засеяно ничего"
    );
    assert!(
        sown_at(&by_lot, 4.0, -4.0),
        "середина клина у стоянки — землёй"
    );
    assert_eq!(
        by_lot.pockets[0].kind,
        AreaKind::Residential,
        "у стоянки — травой двора"
    );
    // щель в несколько м² без соседа — не земля, а зазор плитки: не засевается,
    // а у газона та же щель вливается в него
    let lot = AreaKind::Parking(LotKind::Yard);
    assert_eq!(
        pockets::fill_ground_pockets(&mut scene_beside(lot, 4.0)),
        0,
        "щель у стоянки"
    );
    let mut slit = scene_beside(AreaKind::Grass, 4.0);
    pockets::fill_ground_pockets(&mut slit);
    assert!(
        crate::map::osm::model::point_in_polygon(
            CENTER + Vec2::new(1.0, -1.0),
            &slit.grass[0].outer
        ),
        "щель у газона не влилась в него"
    );
}

/// Карман-луг, касающийся той самой зелени, у которой засевается, вливается в
/// неё одним полигоном, а не сеется отдельной площадкой поверх её каймы:
/// засеянный рядом, он кроет кайму газона полосой в полметра, и там, где
/// кончается, срезанная кайма читается швом «V» (Тула, сквер у Сойфера, N8 —
/// два кармана-луга вдоль клина газона). Влившийся карман растит газон до
/// кромки дорожек, и кайма у газона остаётся только по внешнему краю.
#[test]
fn a_meadow_pocket_touching_a_lawn_is_merged_into_it() {
    let mut map = pocket_scene(16.0, false);
    let mut area = map.landuse.pop().expect("квартал сцены");
    area.kind = AreaKind::Grass;
    map.grass.push(area);
    let sown = pockets::fill_ground_pockets(&mut map);
    assert!(
        !sown_at(&map, 4.0, -4.0),
        "клин у газона сеется отдельной площадкой ({sown} засеяно)"
    );
    let lawn = &map.grass[0];
    let inside = |x: f32, y: f32| {
        crate::map::osm::model::point_in_polygon(CENTER + Vec2::new(x, y), &lawn.outer)
    };
    assert!(inside(4.0, -4.0), "клин не влился в газон");
    assert!(
        !inside(30.0, 30.0),
        "газон вырос за пределы бывшего клина и своей земли"
    );
    // газон дошёл до дорожек: его край заводится под полосу дорожки
    let top = lawn
        .outer
        .iter()
        .map(|point| point.y - CENTER.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(top > 0.5, "край газона не под дорожкой: {top}");
}

/// Щель между кромкой мощёной дорожки и краем сквера — фартук зелени:
/// полоса голой земли не шире пары метров засевается лугом от края зелени
/// до кромки ленты, замкнута она или протекает на концах. У СЗ угла
/// Оружейной × Энгельса такая полоса между
/// велодорожкой и забором парка светилась тёмной линией голой земли
/// (R37-02): на концах она протекает, и замкнутым карманом её не видно.
#[test]
fn a_slit_between_a_paved_path_and_the_park_edge_is_sown_as_meadow() {
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    let scene = |gap: f32, rough: bool| {
        let mut road = crate::map::osm::fixture::footway(vec![at(0.0, -40.0), at(0.0, 40.0)]);
        road.sidewalks = [SidewalkSide::None; 2];
        road.pavement = Some(if rough {
            Pavement::Unpaved
        } else {
            Pavement::Paved
        });
        // сквер за дорожкой в `gap` метрах от кромки (полудорожка 1.75 м)
        MapData {
            roads: vec![road],
            parks: vec![PolyArea {
                kind: AreaKind::Park,
                ..building(
                    vec![
                        at(1.75 + gap, -20.0),
                        at(30.0, -20.0),
                        at(30.0, 20.0),
                        at(1.75 + gap, 20.0),
                    ],
                    Vec::new(),
                )
            }],
            ..MapData::default()
        }
    };
    let mut map = scene(0.4, false);
    let sown = pockets::fill_ground_pockets(&mut map);
    assert!(sown >= 1, "карманов засеяно: {sown}");
    assert!(
        map.pockets.iter().all(|area| area.kind == AreaKind::Grass),
        "у сквера — лугом: {:?}",
        map.pockets.iter().map(|area| area.kind).collect::<Vec<_>>()
    );
    assert!(sown_at(&map, 1.95, 0.0), "щель осталась землёй");
    // широкая полоса — уже не щель, а граница; у грунтовой тропы зелени
    // не подшивают
    assert_eq!(pockets::fill_ground_pockets(&mut scene(4.0, false)), 0);
    assert_eq!(pockets::fill_ground_pockets(&mut scene(0.4, true)), 0);
}

/// Полоса голой земли между кромкой газона и тротуарной лентой улицы — тоже
/// фартук зелени, если газон её обочины полосу не закрывает. Улицам зелень
/// не подшивалась вовсе («полосу вдоль них рисует газон обочины»), но обочина
/// есть не на каждом отрезке, а где есть — не всегда доходит до кромки
/// зелени: у сквера Сойфера в Туле между газоном и тротуарной лентой улицы
/// Фёдора Смирнова светилась полоса земли 1 × 7 м — с юга открытая в
/// перекрёсток, замкнутым карманом не видная (N8b). Вопрос фартука — не
/// «есть ли обочина», а «доходит ли нарисованный край улицы (тротуарная
/// лента или её газон обочины, кто дальше) до кромки зелени»: недошедшее —
/// заливается лугом.
#[test]
fn a_strip_by_a_street_band_is_sown_where_no_verge_lawn_covers_it() {
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    // улица 7.6 м с тротуарами (край ленты 3.8 + `sidewalk_band`), газон за
    // ней в `gap` метрах от края; `verge` — обочина на стороне газона
    let band = 3.8 + sidewalk_band(7.6);
    let scene = |gap: f32, verge: f32| {
        let mut road = street(vec![at(0.0, -40.0), at(0.0, 40.0)], 7.6);
        road.verges = [0.0, verge];
        MapData {
            roads: vec![road],
            grass: vec![PolyArea {
                kind: AreaKind::Grass,
                ..building(
                    vec![
                        at(band + gap, -20.0),
                        at(30.0, -20.0),
                        at(30.0, 20.0),
                        at(band + gap, 20.0),
                    ],
                    Vec::new(),
                )
            }],
            ..MapData::default()
        }
    };
    // обочины нет вовсе — как на отрезке у сквера Сойфера
    let mut map = scene(0.4, 0.0);
    let sown = pockets::fill_ground_pockets(&mut map);
    assert!(sown >= 1, "полоса у ленты улицы не подшита: {sown}");
    assert!(
        map.pockets.iter().all(|area| area.kind == AreaKind::Grass),
        "у газона — лугом: {:?}",
        map.pockets.iter().map(|area| area.kind).collect::<Vec<_>>()
    );
    assert!(sown_at(&map, band + 0.2, 0.0), "полоса осталась землёй");
    // обочина есть, но не доходит: полоса от ленты до кромки газона — лугом,
    // от края того, что улица рисует сама
    let mut map = scene(0.4, 1.0);
    let sown = pockets::fill_ground_pockets(&mut map);
    assert!(sown >= 1, "полоса у ленты улицы не подшита: {sown}");
    assert!(sown_at(&map, band + 0.2, 0.0), "полоса осталась землёй");
    // обочина доходит за кромку зелени — полосы нет, и фартук не ложится
    // поверх её газона чужим цветом (два «крыла» на торцах газона —
    // вырожденные, нулевой площади, как и у фартука дорожки)
    let mut covered = scene(0.4, 2.5);
    pockets::fill_ground_pockets(&mut covered);
    assert!(
        !sown_at(&covered, band + 0.2, 0.0),
        "фартук лёг на газон обочины"
    );
}

/// Фартук шьётся по кромке зелени **после** вливания карманов: влитой
/// карман растит зелень к ленте, и полоса улицы открыта именно от
/// выращенной кромки — спрашивать прежнюю значит оставить полосу
/// (сквер Сойфера, N8b: фартук шёл по кромке до вливания и полосу не взял).
#[test]
fn a_green_apron_follows_the_lawn_edge_a_merged_pocket_grew() {
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    // дорожки — явно мощёные: у собранной руками `pavement: None`, и она
    // читается грунтовой тропой, у которой кармана и фартука нет
    let path = |points: Vec<Vec2>| {
        let mut road = crate::map::osm::fixture::footway(points);
        road.pavement = Some(Pavement::Paved);
        road
    };
    // улица внизу (край ленты 3.8 + `sidewalk_band`), над ней дорожка, ещё
    // выше газон; между дорожкой и газоном замкнутый карман (боковые дорожки
    // замыкают), вливающийся в газон и растящий его кромку вниз
    let band = 3.8 + sidewalk_band(7.6);
    let mut map = MapData {
        roads: vec![
            street(vec![at(-40.0, 0.0), at(40.0, 0.0)], 7.6),
            path(vec![at(-20.0, 6.5), at(20.0, 6.5)]),
            path(vec![at(-10.0, 4.0), at(-10.0, 12.0)]),
            path(vec![at(10.0, 4.0), at(10.0, 12.0)]),
        ],
        grass: vec![PolyArea {
            kind: AreaKind::Grass,
            ..building(
                vec![
                    at(-10.0, 8.5),
                    at(10.0, 8.5),
                    at(10.0, 40.0),
                    at(-10.0, 40.0),
                ],
                Vec::new(),
            )
        }],
        ..MapData::default()
    };
    let sown = pockets::fill_ground_pockets(&mut map);
    // карман влился: газон дорос до кромки дорожки
    let bottom = map.grass[0]
        .outer
        .iter()
        .map(|point| point.y - CENTER.y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        bottom < 8.0,
        "карман не влился в газон: низ газона {bottom}"
    );
    // фартук пошёл от выращенной кромки до ленты улицы; от прежней кромки
    // (8.5 м, дальше `GREEN_GAP_MAX`) полосы не видно
    assert!(
        sown_at(&map, 0.0, (band + bottom) / 2.0),
        "полоса у ленты не подшита ({sown} засеяно)"
    );
}

/// Что должно остаться землёй, остаётся: карман у грунтовой тропы (пустырь с
/// тропинками), большой карман (площадка сама по себе) и дырка в самом
/// квартале, у которой нет дороги на краю.
#[test]
fn a_pocket_by_a_dirt_path_a_large_one_and_a_hole_in_the_block_stay_ground() {
    let mut rough = pocket_scene(16.0, true);
    assert_eq!(pockets::fill_ground_pockets(&mut rough), 0, "у тропы");

    // катет 40 м — 800 м² земли
    let mut large = pocket_scene(40.0, false);
    assert_eq!(pockets::fill_ground_pockets(&mut large), 0, "большой");

    let mut holed = pocket_scene(0.0, false);
    holed.landuse[0].holes = vec![rect(
        CENTER + Vec2::new(30.0, -40.0),
        CENTER + Vec2::new(36.0, -34.0),
    )];
    assert_eq!(
        pockets::fill_ground_pockets(&mut holed),
        0,
        "дырка квартала засеяна"
    );
}

/// Карман ищется плитками, и тот, что лёг на шов двух плиток, засевается
/// ровно один раз: серединой он принадлежит одной из них.
#[test]
fn a_pocket_on_a_tile_seam_is_sown_once() {
    // сдвиг сцены так, чтобы клин лёг поперёк границы плиток в 250 м
    let seam = (CENTER / 250.0).ceil() * 250.0;
    let shift = seam - (CENTER + Vec2::new(4.0, -4.0));
    let mut map = pocket_scene(16.0, false);
    for road in &mut map.roads {
        road.points.iter_mut().for_each(|point| *point += shift);
    }
    map.landuse[0]
        .outer
        .iter_mut()
        .for_each(|point| *point += shift);
    assert_eq!(pockets::fill_ground_pockets(&mut map), 1);
}

/// Остров кольца — не карман: газон острова (`roads.rs::ring_island_lawns`)
/// кроет его целиком, и трава двора, засеянная в дырку покрытия, лежала на
/// нём тёмным рваным пятном (Белгород, кольцо Чапаева × Кошарский, R28/S4).
/// Та же петля без хода по кольцу — не кольцо, и земля в ней засевается.
#[test]
fn a_ring_island_is_not_a_ground_pocket() {
    let scene = |roundabout: bool| {
        let points: Vec<Vec2> = (0..=24)
            .map(|step| {
                let angle = std::f32::consts::TAU * (step % 24) as f32 / 24.0;
                CENTER + 15.0 * Vec2::from_angle(angle)
            })
            .collect();
        let ring = RoadLine {
            oneway: roundabout,
            roundabout,
            ..street(points, 8.0)
        };
        MapData {
            roads: vec![ring],
            ..MapData::default()
        }
    };
    let mut loop_road = scene(false);
    assert_eq!(
        pockets::fill_ground_pockets(&mut loop_road),
        1,
        "земля в петле"
    );
    assert!(sown_at(&loop_road, 0.0, 0.0));

    let mut ring = scene(true);
    assert_eq!(
        pockets::fill_ground_pockets(&mut ring),
        0,
        "остров кольца засеян"
    );
}
