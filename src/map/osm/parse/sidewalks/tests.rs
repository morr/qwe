use super::super::fixtures::{CENTER, CITY, inferred_street, paved_footway, tags};
use super::super::tags::{tagged_sidewalks, untagged_sidewalks};
use super::super::verges;
use super::*;
use crate::map::osm::fixture::{Overpass, building, rect, square};
use crate::map::osm::model::sidewalk_band;
use crate::map::osm::model::{Highway, Pavement, PolyArea};
use crate::map::roads::network::RoadNetwork;
use crate::settings::STOREY_HEIGHT;
use bevy::math::Vec2;

/// `sidewalk=*` — стороны по ходу точек после разбора: `separate` и `no` —
/// нет тротуара, уточнение по стороне сильнее общего тега, у `oneway=-1`
/// стороны меняются вместе с разворотом точек.
#[test]
fn sidewalk_tags_pick_the_sides() {
    let sides = |pairs: &[(&str, &str)]| {
        tagged_sidewalks(&tags(pairs)).map(|sides| sides.map(SidewalkSide::is_present))
    };
    assert_eq!(sides(&[]), None, "без тега решает не тег");
    assert_eq!(
        tagged_sidewalks(&tags(&[("sidewalk", "right")])),
        Some([SidewalkSide::None, SidewalkSide::Tagged]),
        "тротуар по тегу — `Tagged`, снятый тегом — `None`"
    );
    assert_eq!(sides(&[("sidewalk", "separate")]), Some([false, false]));
    assert_eq!(sides(&[("sidewalk", "no")]), Some([false, false]));
    assert_eq!(sides(&[("sidewalk", "right")]), Some([false, true]));
    assert_eq!(
        sides(&[("sidewalk", "both"), ("sidewalk:left", "separate")]),
        Some([false, true])
    );
    assert_eq!(
        sides(&[("sidewalk:both", "separate")]),
        Some([false, false])
    );
    assert_eq!(
        sides(&[("sidewalk:left", "no")]),
        Some([false, true]),
        "одна названная сторона — другая остаётся"
    );
    assert_eq!(
        sides(&[("sidewalk", "left"), ("oneway", "-1")]),
        Some([false, true]),
        "развёрнутый way — тротуар справа по новому ходу"
    );
    let bare = |pairs: &[(&str, &str)]| untagged_sidewalks(&tags(pairs));
    assert_eq!(bare(&[]), [SidewalkSide::Inferred; 2]);
    assert_eq!(bare(&[("surface", "asphalt")]), [SidewalkSide::Inferred; 2]);
    assert_eq!(
        bare(&[("surface", "gravel")]),
        [SidewalkSide::None; 2],
        "у грунтовой — никогда"
    );
}

/// Жилая улица без тега тротуара: в частном секторе обочина без полосы, среди
/// многоэтажек — тротуар с обеих сторон. Магистраль и улица с тегом в
/// частном секторе тротуар сохраняют, улица вдали от домов — нет.
#[test]
fn an_untagged_street_takes_its_sidewalks_from_the_blocks_around() {
    let street = |y: f32| vec![CENTER + Vec2::new(-150.0, y), CENTER + Vec2::new(150.0, y)];
    let row = |scene: Overpass, y: f32, tags: &'static [(&str, &str)], side: f32| {
        (0..6).fold(scene, |scene, i| {
            let x = -125.0 + 50.0 * i as f32;
            scene.area(
                tags,
                rect(
                    CENTER + Vec2::new(x, y + 12.0),
                    CENTER + Vec2::new(x + side, y + 12.0 + side),
                ),
            )
        })
    };
    const HOUSE: &[(&str, &str)] = &[("building", "house")];
    const SLAB: &[(&str, &str)] = &[("building", "apartments"), ("height", "27")];
    // частный сектор у y = 0, микрорайон у y = 1000, пустырь у y = -1000
    let scene = Overpass::new(CITY)
        .way(&[("highway", "residential")], street(0.0))
        .way(&[("highway", "tertiary")], street(20.0))
        .way(
            &[("highway", "residential"), ("sidewalk", "both")],
            street(-20.0),
        )
        .way(&[("highway", "residential")], street(1000.0))
        .way(&[("highway", "residential")], street(-1000.0));
    let scene = row(scene, 0.0, HOUSE, 10.0);
    let scene = row(scene, 1000.0, SLAB, 30.0);
    let map = scene.parse();

    let sides: Vec<[bool; 2]> = map
        .roads
        .iter()
        .map(|road| road.sidewalks.map(SidewalkSide::is_present))
        .collect();
    assert_eq!(sides[0], [false; 2], "частный сектор — без полосы");
    assert_eq!(sides[1], [true; 2], "tertiary — всегда");
    assert_eq!(sides[2], [true; 2], "тег сильнее окружения");
    assert_eq!(sides[3], [true; 2], "микрорайон — с тротуаром");
    assert_eq!(map.roads[2].sidewalks, [SidewalkSide::Tagged; 2]);
    assert_eq!(map.roads[3].sidewalks, [SidewalkSide::Inferred; 2]);
    assert_eq!(sides[4], [false; 2], "без домов вокруг — без полосы");
}

/// Дом со **своей** высотой посреди улицы: мера [`Districts`] читает высоту
/// тем же `height_or_default`, что и слой машин, так что тег снимает тест с
/// зависимости от вывода этажей. Высота `N` этажей — `N × STOREY_HEIGHT`.
fn district_house(storeys: f32) -> Vec<PolyArea> {
    vec![PolyArea {
        height: Some(storeys * STOREY_HEIGHT),
        ..building(square(Vec2::new(100.0, 0.0), 10.0), Vec::new())
    }]
}

/// Застройка ниже городской снимает тротуар без тега, городская оставляет —
/// мера строится прямо из домов сцены, без карты.
#[test]
fn infer_sidewalks_drops_the_band_only_among_low_blocks() {
    let mut roads = vec![inferred_street(Highway::Residential)];
    let low = infer_sidewalks(&mut roads, &Districts::new(&district_house(2.0)));
    assert_eq!((low.asked, low.dropped), (1, 1));
    assert_eq!(roads[0].sidewalks, [SidewalkSide::None; 2]);

    let mut roads = vec![inferred_street(Highway::Unclassified)];
    let tall = infer_sidewalks(
        &mut roads,
        &Districts::new(&district_house(SIDEWALK_STOREYS_MIN)),
    );
    assert_eq!((tall.asked, tall.dropped), (1, 0));
    assert_eq!(roads[0].sidewalks, [SidewalkSide::Inferred; 2]);
}

/// Пусто вокруг — тоже обочина: мера ни разу не отвечает этажностью.
#[test]
fn infer_sidewalks_drops_the_band_where_nothing_stands_around() {
    let mut roads = vec![inferred_street(Highway::LivingStreet)];
    let empty = infer_sidewalks(&mut roads, &Districts::new(&[]));
    assert_eq!((empty.asked, empty.dropped), (1, 1));
    assert_eq!(roads[0].sidewalks, [SidewalkSide::None; 2]);
}

/// Спрашивается только тротуар без тега на жилой улице: тег, магистраль и
/// уже снятый (грунтовка) не трогаются и в счёт не идут.
#[test]
fn infer_sidewalks_asks_only_inferred_residential_streets() {
    let tagged = RoadLine {
        sidewalks: [SidewalkSide::Tagged; 2],
        ..inferred_street(Highway::Residential)
    };
    let unpaved = RoadLine {
        sidewalks: [SidewalkSide::None; 2],
        ..inferred_street(Highway::Residential)
    };
    let mut roads = vec![tagged, inferred_street(Highway::Tertiary), unpaved];
    let report = infer_sidewalks(&mut roads, &Districts::new(&[]));
    assert_eq!((report.asked, report.dropped), (0, 0));
    assert_eq!(roads[0].sidewalks, [SidewalkSide::Tagged; 2]);
    assert_eq!(roads[1].sidewalks, [SidewalkSide::Inferred; 2]);
    assert_eq!(roads[2].sidewalks, [SidewalkSide::None; 2]);
}

/// Отдельная мощёная дорожка осью в двух метрах за полосой справа (газон до
/// её края — два метра без её полуширины) снимает полосу справа; левая
/// сторона, где дорожки нет, остаётся.
#[test]
fn a_footway_along_the_kerb_takes_the_inferred_sidewalk_of_its_side() {
    let street = inferred_street(Highway::Primary);
    let edge = street.width / 2.0 + sidewalk_band(street.width);
    let mut roads = vec![street, paved_footway(-(edge + 2.0), 0.0, 200.0)];
    let network = RoadNetwork::new(&roads);
    let report = verges::measure_footways_beside_streets(&mut roads, &network);
    assert_eq!((report.asked, report.dropped), (2, 1));
    assert_eq!(
        roads[0].sidewalks,
        [SidewalkSide::Inferred, SidewalkSide::None]
    );
}

/// Не отдаётся: тег, дорожка поперёк, дорожка на треть длины, грунтовая
/// тропинка, дорожка за домом (дальше [`verges::SEPARATE_REACH`] за полосой) и
/// дорожка у самой кромки — газона меньше [`SEPARATE_LAWN`] (Тула, витрины 15
/// и 21: снятая полоса оставляла между бордюром и дорожкой щель земли).
#[test]
fn a_sidewalk_stays_unless_a_paved_footway_runs_beside_it() {
    let street = inferred_street(Highway::Residential);
    let edge = street.width / 2.0 + sidewalk_band(street.width);
    let check = |mut roads: Vec<RoadLine>, expected: [SidewalkSide; 2]| {
        let network = RoadNetwork::new(&roads);
        verges::measure_footways_beside_streets(&mut roads, &network);
        assert_eq!(roads[0].sidewalks, expected);
    };
    // у кромки: ближний край дорожки в метре от бордюра
    let close = paved_footway(0.0, 0.0, 200.0);
    let close_axis = street.width / 2.0 + 1.0 + close.width / 2.0;
    check(
        vec![street.clone(), paved_footway(close_axis, 0.0, 200.0)],
        [SidewalkSide::Inferred; 2],
    );
    let check = |mut roads: Vec<RoadLine>, expected: [SidewalkSide; 2]| {
        let network = RoadNetwork::new(&roads);
        verges::measure_footways_beside_streets(&mut roads, &network);
        assert_eq!(roads[0].sidewalks, expected);
    };
    let inferred = [SidewalkSide::Inferred; 2];
    // тег ставил человек
    let tagged = RoadLine {
        sidewalks: [SidewalkSide::Tagged; 2],
        ..street.clone()
    };
    check(
        vec![tagged, paved_footway(edge + 1.0, 0.0, 200.0)],
        [SidewalkSide::Tagged; 2],
    );
    // тот же газон в два метра — настоящий, полоса уходит
    let lawn_axis = street.width / 2.0 + 2.0 + close.width / 2.0;
    check(
        vec![street.clone(), paved_footway(lawn_axis, 0.0, 200.0)],
        [SidewalkSide::None, SidewalkSide::Inferred],
    );
    // поперёк
    let across = RoadLine {
        pavement: Some(Pavement::Paved),
        ..crate::map::osm::fixture::footway(vec![Vec2::new(100.0, 2.0), Vec2::new(100.0, 30.0)])
    };
    check(vec![street.clone(), across], inferred);
    // треть длины
    check(
        vec![street.clone(), paved_footway(edge + 1.0, 0.0, 66.0)],
        inferred,
    );
    // грунтовая
    let trail = RoadLine {
        pavement: Some(Pavement::Unpaved),
        ..paved_footway(edge + 1.0, 0.0, 200.0)
    };
    check(vec![street.clone(), trail], inferred);
    // за домом
    check(
        vec![
            street.clone(),
            paved_footway(edge + verges::SEPARATE_REACH + 2.0, 0.0, 200.0),
        ],
        inferred,
    );
}
