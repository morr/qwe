//! Храмы и крепость — здания, которые рисуются по смыслу, а не по правилам
//! дома: венец по вере над храмом, шатёр и зубцы над кремлём.
//!
//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе: каждый тест здесь строит освещённую геометрию, поэтому каждый
//! берёт `default_sun()` — гард, который держит солнце на дефолте и не пускает
//! к нему соседа (`map/sun.rs`).

use super::*;
use crate::map::buildings::clutter::merlons;
use crate::map::buildings::fixtures::{
    building, church, detail, extruded_mesh, is_solid, is_wall, oblong, square,
};
use crate::map::buildings::material::{DOOR_CODE, roof_look, wall_look};
use crate::map::buildings::roofs::landmark_roof;
use crate::map::buildings::{BuildingHeightMode, extrusion_lift};
use crate::map::meshing::unpack_material;
use crate::map::osm::{AreaKind, Colours};

/// Крепость — кладка без проёмов: **каждая** вершина её стен помечена как
/// стена без окон, и дверей на ней нет, даже когда вход у контура есть.
#[test]
fn a_fortress_wall_has_no_openings() {
    let _sun = crate::map::default_sun();
    let mut wall = building(oblong(3.0, 60.0), Some(12.7), AreaKind::Kremlin);
    wall.entrances = vec![Vec2::new(30.0, 0.0)];
    let mesh = extruded_mesh(std::slice::from_ref(&wall), &[], detail(false));
    let coords = mesh.roof_coords_for_test().unwrap();
    let walls: Vec<&[f32; 4]> = coords.iter().filter(|c| is_wall(c[2])).collect();
    assert!(!walls.is_empty());
    assert!(
        walls.iter().all(|c| is_solid(c[3])),
        "a kremlin wall got an opening"
    );
    assert!(
        coords.iter().all(|c| unpack_material(c[2]).0 != DOOR_CODE),
        "a kremlin wall got a door"
    );
}

/// Стена крепости кроется плоским ходом с зубцами, башня — шатром.
#[test]
fn a_fortress_tower_is_tented_and_its_wall_crenellated() {
    let _sun = crate::map::default_sun();
    let wall = building(oblong(3.0, 60.0), Some(12.0), AreaKind::Kremlin);
    let tower = building(square(), Some(30.0), AreaKind::Kremlin);
    assert_eq!(landmark_roof(&wall), Some(LandmarkRoof::Flat));
    assert!(matches!(
        landmark_roof(&tower),
        Some(LandmarkRoof::Tent { .. })
    ));
    let lift = extrusion_lift(&wall, BuildingHeightMode::Extrusion);
    // по зубцу на 2.6 м с каждой длинной стороны
    assert!(merlons(&wall, lift).len() >= 40);
    assert!(merlons(&tower, lift).is_empty());
}

/// Над храмом стоит венец его веры: у православного главы-луковицы, у
/// западного — башня со шпилем, у мечети — купол и минареты, а у восточного
/// — ничего сверх вальмы.
#[test]
fn a_church_is_crowned_by_its_faith() {
    let _sun = crate::map::default_sun();
    let white = Srgba::WHITE;
    let ship = || oblong(18.0, 44.0);
    let crowns = |area: &PolyArea| crowns_with(area, white, white, Own::default());

    let orthodox = crowns(&church(ship(), 14.0, Faith::Orthodox, SacredForm::Nave));
    assert!(orthodox.iter().any(|c| matches!(
        c,
        Crown::Dome {
            profile: Profile::Onion,
            ..
        }
    )));
    assert!(
        orthodox
            .iter()
            .any(|c| matches!(c, Crown::Tower { cap: Some(_), .. })),
        "a ship church has a bell tower with a cupola"
    );

    let western = crowns(&church(ship(), 14.0, Faith::Western, SacredForm::Nave));
    assert!(
        western
            .iter()
            .any(|c| matches!(c, Crown::Tower { cap: None, .. }))
    );
    assert!(!western.iter().any(|c| matches!(c, Crown::Dome { .. })));

    let mosque = crowns(&church(ship(), 14.0, Faith::Muslim, SacredForm::Nave));
    assert!(mosque.iter().any(|c| matches!(
        c,
        Crown::Dome {
            profile: Profile::Hemisphere,
            ..
        }
    )));
    assert!(mosque.iter().any(|c| matches!(c, Crown::Minaret { .. })));

    let eastern = crowns(&church(ship(), 14.0, Faith::Eastern, SacredForm::Nave));
    assert!(eastern.is_empty());

    // пристройка своих глав не несёт, не храм — тем более
    assert!(crowns(&church(ship(), 14.0, Faith::Orthodox, SacredForm::Annex)).is_empty());
    assert!(crowns(&building(ship(), Some(14.0), AreaKind::Building)).is_empty());
}

/// Колокольня стоит на храме всеми четырьмя углами. `min_area_rect` описывает
/// вместе с храмом и крыльцо, так что у торца с притвором прямоугольник длиннее
/// самого дома, — вровень с его концом башня повисла бы в воздухе.
#[test]
fn a_bell_tower_stands_on_the_church_and_not_on_its_porch() {
    let _sun = crate::map::default_sun();
    // «корабль» 18 × 44 м с крыльцом 3 × 3 м посреди западного торца
    let porched = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(44.0, 0.0),
        Vec2::new(44.0, 18.0),
        Vec2::new(0.0, 18.0),
        Vec2::new(0.0, 10.5),
        Vec2::new(-3.0, 10.5),
        Vec2::new(-3.0, 7.5),
        Vec2::new(0.0, 7.5),
    ];
    for faith in [Faith::Orthodox, Faith::Western] {
        let area = church(porched.clone(), 14.0, faith, SacredForm::Nave);
        let tower = crowns_with(&area, Srgba::WHITE, Srgba::WHITE, Own::default())
            .into_iter()
            .find_map(|crown| match crown {
                Crown::Tower { at, axis, size, .. } => Some((at, axis, size)),
                _ => None,
            })
            .expect("a ship church has a bell tower");
        let (at, axis, size) = tower;
        let perp = Vec2::new(-axis.y, axis.x);
        let (u, v) = (axis * (size.x / 2.0), perp * (size.y / 2.0));
        let corners = [at - u - v, at + u - v, at + u + v, at - u + v];
        for corner in corners {
            assert!(
                point_in_area(corner, &area),
                "{faith:?}: угол башни {corner:?} висит в воздухе"
            );
        }
        // и всё-таки у западного торца, а не посреди храма
        let west = corners.iter().fold(f32::MAX, |west, c| west.min(c.x));
        assert!(west < 6.0, "{faith:?}: башня уехала на восток");
    }
}

/// Главы тоже стоят на храме. У крестового плана восточная доля
/// `min_area_rect` приходится на апсиду, и пятиглавие, разложенное по ней,
/// вырастало барабанами из стен и висело над землёй за ней.
#[test]
fn every_cupola_stands_on_its_church_and_not_over_the_apse() {
    let _sun = crate::map::default_sun();
    // корабль 22 × 24 м с узкой апсидой 18 × 11 м на восточном конце
    let cross = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(22.0, 0.0),
        Vec2::new(22.0, 6.5),
        Vec2::new(40.0, 6.5),
        Vec2::new(40.0, 17.5),
        Vec2::new(22.0, 17.5),
        Vec2::new(22.0, 24.0),
        Vec2::new(0.0, 24.0),
    ];
    let area = church(cross, 18.0, Faith::Orthodox, SacredForm::Nave);
    let crowns = crowns_with(&area, Srgba::WHITE, Srgba::WHITE, Own::default());
    assert!(crowns.iter().any(|c| matches!(c, Crown::Dome { .. })));
    for crown in &crowns {
        let Crown::Dome { at, radius, .. } = *crown else {
            continue;
        };
        for step in 0..8 {
            let rim = at + Vec2::from_angle(step as f32 * std::f32::consts::FRAC_PI_4) * radius;
            assert!(
                point_in_area(rim, &area),
                "край главы {rim:?} висит в воздухе"
            );
        }
    }
}

/// Барабан, стоящий на крыше собора (`min_height`), рисуется барабаном с
/// главой с высоты начала — без коробки от земли, — а сам собор не ставит
/// поверх свою центральную главу.
#[test]
fn a_raised_drum_stands_on_its_church_instead_of_growing_from_the_ground() {
    let _sun = crate::map::default_sun();
    // не от начала координат: посев точки (0, 0) — ноль, а ноль у посева храма
    // значит «храм не собран»
    let origin = Vec2::new(100.0, 60.0);
    let mut cathedral = church(
        oblong(34.0, 40.0).into_iter().map(|p| p + origin).collect(),
        16.0,
        Faith::Orthodox,
        SacredForm::Nave,
    );
    let complex = building_seed(&cathedral);
    assert_ne!(complex, 0);
    let set = |area: &mut PolyArea, floor_dm: u16| {
        if let BuildingUse::Church(sacred) = &mut area.building_use {
            sacred.complex = complex;
            sacred.floor_dm = floor_dm;
        }
    };
    set(&mut cathedral, 0);
    let center = Vec2::new(20.0, 17.0) + origin;
    let mut drum = church(
        square()
            .iter()
            .map(|p| *p * 0.45 + center - Vec2::splat(4.5))
            .collect(),
        35.0,
        Faith::Orthodox,
        SacredForm::Dome,
    );
    set(&mut drum, 200);
    let list = [cathedral, drum];
    let sanctuary = Sanctuary::of(&list);

    assert_eq!(sanctuary.raised(0), None);
    assert_eq!(sanctuary.raised(1), Some(20.0));
    let white = Srgba::WHITE;
    let on_roof = sanctuary.crowns(1, &list[1], white, white, Vec2::new(3.0, 7.0));
    assert!(matches!(
        on_roof.as_slice(),
        [(Crown::Dome { base, .. }, eave)] if *base == 20.0 && *eave == Vec2::ZERO
    ));

    // у собора с главами-частями своих глав нет вовсе
    let own = sanctuary.crowns(0, &list[0], white, white, Vec2::ZERO);
    assert!(
        !own.iter()
            .any(|(crown, _)| matches!(crown, Crown::Dome { .. }))
    );

    // и коробки у барабана в меше нет: все его вершины — вершины главы, выше
    // высоты начала
    let mesh = extruded_mesh(&list[1..], &[], detail(false));
    let lone = Sanctuary::of(&list[1..]);
    assert_eq!(lone.raised(0), Some(20.0));
    let lowest = mesh
        .positions_for_test()
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min);
    assert!(
        lowest > origin.y + 17.0 - 4.5 + 20.0 * 0.35 * 0.9,
        "the drum reaches the ground: {lowest}"
    );
}

/// Глава на вальме стоит на её площадке, а не висит над ней и не тонет:
/// её основание — ровно подъём назначенной крыши.
#[test]
fn a_cupola_stands_on_the_ridge_it_is_given() {
    let _sun = crate::map::default_sun();
    let area = church(oblong(24.0, 26.0), 16.0, Faith::Orthodox, SacredForm::Nave);
    assert_eq!(landmark_roof(&area), Some(LandmarkRoof::Hip));
    let rise = landmark_rise(&area);
    assert!(rise > 0.0);
    for crown in crowns_with(&area, Srgba::WHITE, Srgba::WHITE, Own::default()) {
        if let Crown::Dome { base, .. } = crown {
            assert_eq!(base, rise);
        }
    }
}

/// Тень храма дотягивается до маковки: пятна венца уходят в развёртки выше
/// карниза самого храма.
#[test]
fn a_crown_casts_a_shadow_past_the_eaves() {
    let _sun = crate::map::default_sun();
    let area = church(oblong(24.0, 26.0), 16.0, Faith::Orthodox, SacredForm::Nave);
    let casters = Sanctuary::of(std::slice::from_ref(&area)).shadow_casters(0, &area);
    assert!(!casters.is_empty());
    assert!(
        casters
            .iter()
            .all(|(outline, top)| outline.len() >= 3 && *top > 16.0)
    );
}

/// Отдельно стоящая колокольня — весь дом венец: коробки у неё нет, столп
/// ярусами идёт от земли, шпиль — сверх столпа внутри высоты из OSM, и тень
/// дотягивается до её верха.
#[test]
fn a_standalone_bell_tower_is_all_crown_from_the_ground() {
    let _sun = crate::map::default_sun();
    let tower = church(oblong(12.0, 12.0), 70.0, Faith::Orthodox, SacredForm::Tower);
    let sanctuary = Sanctuary::of(std::slice::from_ref(&tower));
    assert!(sanctuary.boxless(0, &tower));
    assert_eq!(landmark_roof(&tower), Some(LandmarkRoof::Flat));
    let crowns = sanctuary.crowns(0, &tower, Srgba::WHITE, Srgba::WHITE, Vec2::new(3.0, 7.0));
    let [
        (
            Crown::Tower {
                base,
                height,
                spire,
                tiers,
                cap: Some(_),
                ..
            },
            eave,
        ),
    ] = crowns[..]
    else {
        panic!("one bell tower crown, got {crowns:?}");
    };
    assert_eq!(base, 0.0);
    assert_eq!(
        eave,
        Vec2::ZERO,
        "no box, no eave: the tower stands on the ground"
    );
    assert!(
        (height + spire - 70.0).abs() < 0.01,
        "OSM height is with the spire"
    );
    assert_eq!(tiers, 3, "a 12 m tower with a 50 m pillar is three tiers");
    let casters = sanctuary.shadow_casters(0, &tower);
    assert_eq!(casters.len(), 1);
    assert!(
        casters[0].1 > 70.0,
        "the shadow reaches the ball over the spire"
    );

    // минарет остаётся коробкой с венцом сверху
    let minaret = church(oblong(6.0, 6.0), 30.0, Faith::Muslim, SacredForm::Tower);
    assert!(!Sanctuary::of(std::slice::from_ref(&minaret)).boxless(0, &minaret));
}

/// Храм, у которого колокольня размечена своим контуром, корабельной башни от
/// себя не ставит: у кремлёвского собора Тулы она вставала в десяти метрах от
/// настоящей. Главы у него остаются.
#[test]
fn a_church_with_a_mapped_bell_tower_grows_none_of_its_own() {
    let _sun = crate::map::default_sun();
    // не в начале координат: посев от точки (0, 0) — ноль, «храм не собран»
    let shift =
        |ring: Vec<Vec2>, by: Vec2| -> Vec<Vec2> { ring.into_iter().map(|p| p + by).collect() };
    let mut ship = church(
        shift(oblong(18.0, 44.0), Vec2::new(100.0, 100.0)),
        14.0,
        Faith::Orthodox,
        SacredForm::Nave,
    );
    let complex = building_seed(&ship);
    assert_ne!(complex, 0);
    let BuildingUse::Church(sacred) = ship.building_use else {
        unreachable!()
    };
    ship.building_use = BuildingUse::Church(Sacred { complex, ..sacred });
    let mut tower = church(
        shift(oblong(10.0, 10.0), Vec2::new(80.0, 104.0)),
        40.0,
        Faith::Orthodox,
        SacredForm::Tower,
    );
    tower.building_use = BuildingUse::Church(Sacred {
        complex,
        form: SacredForm::Tower,
        ..sacred
    });
    let has_tower = |crowns: &[(Crown, Vec2)]| {
        crowns
            .iter()
            .any(|(crown, _)| matches!(crown, Crown::Tower { .. }))
    };
    let alone = Sanctuary::of(std::slice::from_ref(&ship));
    assert!(has_tower(&alone.crowns(
        0,
        &ship,
        Srgba::WHITE,
        Srgba::WHITE,
        Vec2::ZERO
    )));

    let both = [ship, tower];
    let sanctuary = Sanctuary::of(&both);
    let church_crowns = sanctuary.crowns(0, &both[0], Srgba::WHITE, Srgba::WHITE, Vec2::ZERO);
    assert!(!has_tower(&church_crowns), "the mapped tower stands beside");
    assert!(
        church_crowns
            .iter()
            .any(|(crown, _)| matches!(crown, Crown::Dome { .. })),
        "the cupolas stay"
    );
    assert!(has_tower(&sanctuary.crowns(
        1,
        &both[1],
        Srgba::WHITE,
        Srgba::WHITE,
        Vec2::ZERO
    )));
}

/// Цвет из разметки красит храм: `building:colour` — стены, `roof:colour` —
/// кровлю храма, но **главу** части с `roof:shape=onion` и шпиль колокольни;
/// без тега цвет идёт по посеву.
#[test]
fn a_tagged_colour_paints_the_church_where_the_tag_means_it() {
    let _sun = crate::map::default_sun();
    let gold = Srgba::rgb_u8(255, 215, 0);
    let white = Srgba::rgb_u8(236, 234, 229);
    let painted = |mut area: PolyArea| {
        area.colours = Colours {
            wall: Some([236, 234, 229]),
            roof: Some([255, 215, 0]),
        };
        area
    };
    let dome_of = |area: &PolyArea| {
        crowns_with(area, Srgba::WHITE, Srgba::WHITE, Own::default())
            .into_iter()
            .find_map(|crown| match crown {
                Crown::Dome { color, .. } => Some(color),
                _ => None,
            })
    };

    // барабан: тег — цвет главы, кровли у него нет
    let drum = painted(church(
        oblong(9.0, 9.0),
        20.0,
        Faith::Orthodox,
        SacredForm::Dome,
    ));
    assert_eq!(dome_of(&drum), Some(gold));
    assert_eq!(wall_look(&drum, 2.0).base, white);
    // до кровли барабана тег не доходит — её там и нет, палитра по посеву
    assert_ne!(roof_look(&drum).base, gold);

    // храм: тег — кровля, глава остаётся по посеву
    let nave = painted(church(
        oblong(24.0, 26.0),
        16.0,
        Faith::Orthodox,
        SacredForm::Nave,
    ));
    assert_eq!(roof_look(&nave).base, gold);
    assert_eq!(wall_look(&nave, 2.0).base, white);
    assert_ne!(dome_of(&nave), Some(gold));

    // колокольня: тег — шпиль и маковка
    let tower = painted(church(
        oblong(12.0, 12.0),
        70.0,
        Faith::Orthodox,
        SacredForm::Tower,
    ));
    assert!(
        crowns_with(&tower, Srgba::WHITE, Srgba::WHITE, Own::default())
            .iter()
            .any(|crown| matches!(crown, Crown::Tower { cap: Some(cap), .. } if *cap == gold))
    );

    // без тега — палитра по посеву, а не белый и не золото
    let plain = church(oblong(24.0, 26.0), 16.0, Faith::Orthodox, SacredForm::Nave);
    assert_ne!(wall_look(&plain, 2.0).base, white);
    assert_ne!(roof_look(&plain).base, gold);
}
