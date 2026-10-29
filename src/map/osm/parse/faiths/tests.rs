use super::super::fixtures::{CENTER, CITY, HALF};
use super::*;
use crate::map::osm::fixture::{Overpass, building, square};
use crate::map::osm::model::{BuildingUse, Faith, Sacred, SacredForm};
use bevy::math::Vec2;

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

/// Храм с заданной верой и формой.
fn church(ring: Vec<Vec2>, faith: Faith, form: SacredForm) -> PolyArea {
    PolyArea {
        building_use: BuildingUse::Church(Sacred {
            faith,
            form,
            complex: 0,
            floor_dm: 0,
        }),
        ..building(ring, Vec::new())
    }
}

/// Сборка храмов — в одиночку, на трёх контурах: барабан внутри собора
/// берёт его веру и его посев, а одинокая церковь без разметки — веру
/// большинства города, и она же одна и попадает в счётчик угаданных.
/// Тест строит [`ResolveFaithsCtx`] буквально, без карты.
#[test]
fn resolving_the_faiths_runs_on_its_own() {
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
        resolve_faiths(ResolveFaithsCtx {
            buildings: &mut buildings
        }),
        1,
        "по большинству города угадан ровно один храм"
    );

    let (cathedral, drum, lone) = (
        sacred_of(&buildings[0]),
        sacred_of(&buildings[1]),
        sacred_of(&buildings[2]),
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

/// Контур без назначения, лежащий на храме, становится его пристройкой:
/// вера — храма, форма — [`SacredForm::Annex`]. Контекст из одних домов —
/// сцена из двух контуров, карты не нужно.
#[test]
fn a_house_lying_on_a_church_becomes_its_annex() {
    let mut buildings = vec![
        church(square(CENTER, 20.0), Faith::Orthodox, SacredForm::Nave),
        building(square(CENTER, 5.0), Vec::new()),
    ];
    resolve_faiths(ResolveFaithsCtx {
        buildings: &mut buildings,
    });
    assert_eq!(
        buildings[1].building_use,
        BuildingUse::Church(Sacred {
            faith: Faith::Orthodox,
            form: SacredForm::Annex,
            complex: sacred_of(&buildings[0]).complex,
            floor_dm: 0,
        }),
        "пристройка не взяла веру и посев храма"
    );
}

/// Храм контура целиком — сверить посев пристройки с посевом хозяина.
fn sacred_of(building: &PolyArea) -> Sacred {
    match building.building_use {
        BuildingUse::Church(sacred) => sacred,
        other => panic!("не храм: {other:?}"),
    }
}
