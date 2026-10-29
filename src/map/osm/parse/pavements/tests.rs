use super::super::fixtures::{CENTER, CITY};
use super::*;
use crate::map::osm::fixture::{Overpass, rect, street};
use crate::map::osm::model::Pavement;
use bevy::math::Vec2;

/// Покрытие дорожки: `surface` решает сам, без него — вид дорожки, а голый
/// `footway` — по окружению: в парке тропинка, во дворе асфальт. Улица поля
/// не получает — его читают только дорожки.
#[test]
fn a_footway_is_paved_by_its_tag_its_kind_or_the_greenery_around() {
    let path = |y: f32| vec![CENTER + Vec2::new(-80.0, y), CENTER + Vec2::new(80.0, y)];
    let map = Overpass::new(CITY)
        .area(
            &[("leisure", "park")],
            rect(
                CENTER + Vec2::new(-100.0, -50.0),
                CENTER + Vec2::new(100.0, 50.0),
            ),
        )
        // в парке
        .way(&[("highway", "footway")], path(0.0))
        .way(
            &[("highway", "footway"), ("surface", "paving_stones")],
            path(10.0),
        )
        .way(
            &[("highway", "footway"), ("footway", "sidewalk")],
            path(20.0),
        )
        // вне парка
        .way(&[("highway", "footway")], path(200.0))
        .way(&[("highway", "path")], path(210.0))
        .way(
            &[("highway", "footway"), ("surface", "ground")],
            path(220.0),
        )
        .way(&[("highway", "steps")], path(230.0))
        .way(&[("highway", "residential")], path(300.0))
        .parse();
    let found: Vec<Option<Pavement>> = map.roads.iter().map(|road| road.pavement).collect();
    use Pavement::{Paved, Unpaved};
    assert_eq!(
        found,
        [
            Some(Unpaved),
            Some(Paved),
            Some(Paved),
            Some(Paved),
            Some(Unpaved),
            Some(Unpaved),
            Some(Paved),
            None,
        ]
    );
    assert!(map.roads[1].is_paved_path());
    assert!(!map.roads[7].is_paved_path(), "улица — не дорожка");
}

/// Дорожка по газону между двумя мощёными — звено мощёной сети, а не
/// тропинка: лучи сквера на острове кольца от аллеи к лестницам площадки
/// ложились песком, лестницы торчали на нём обрубками (Тула, витрина 04, L7).
#[test]
fn a_footway_on_a_lawn_between_paved_paths_is_paved() {
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    let map = Overpass::new(CITY)
        .area(
            &[("landuse", "grass")],
            rect(at(-100.0, -100.0), at(100.0, 100.0)),
        )
        // аллея по краю газона — мощёная тегом
        .way(
            &[("highway", "footway"), ("surface", "asphalt")],
            vec![at(-80.0, -60.0), at(0.0, -60.0), at(80.0, -60.0)],
        )
        // лестница у площадки — мощёная родом
        .way(&[("highway", "steps")], vec![at(0.0, -10.0), at(0.0, -5.0)])
        // от вершины аллеи до лестницы
        .way(
            &[("highway", "footway")],
            vec![at(0.0, -60.0), at(0.0, -10.0)],
        )
        // от аллеи в никуда
        .way(
            &[("highway", "footway")],
            vec![at(80.0, -60.0), at(80.0, 20.0)],
        )
        .parse();
    let found: Vec<Option<Pavement>> = map.roads.iter().map(|road| road.pavement).collect();
    use Pavement::{Paved, Unpaved};
    assert_eq!(
        found,
        [Some(Paved), Some(Paved), Some(Paved), Some(Unpaved)]
    );
}

use crate::map::osm::fixture::area;
use crate::map::osm::model::AreaKind;

/// Дорожка без тега покрытия по точкам `points`.
fn bare_footway(points: Vec<Vec2>) -> RoadLine {
    RoadLine {
        class: RoadClass::Alley,
        ..street(points, 3.0)
    }
}

/// Контекст — и есть договор прохода: без зелени вокруг пробам неоткуда
/// ответить «по газону», и голый `footway` становится асфальтом двора.
/// Тест строит [`InferPavementsCtx`] буквально, без карты, — форма
/// контекста видна сама.
#[test]
fn a_bare_footway_in_an_empty_context_is_paved() {
    let mut roads = vec![bare_footway(vec![
        CENTER + Vec2::new(-80.0, 0.0),
        CENTER + Vec2::new(80.0, 0.0),
    ])];
    let report = infer_pavements(InferPavementsCtx {
        roads: &mut roads,
        parks: &[],
        woods: &[],
        grass: &[],
    });
    assert_eq!(
        (report.asked, report.unpaved, report.between_paved),
        (1, 0, 0)
    );
    assert_eq!(roads[0].pavement, Some(Pavement::Paved));
}

/// Дорожка через парк большей частью пути — тропинка: больше половины
/// проб в зелени.
#[test]
fn a_footway_through_the_park_stays_a_trail() {
    let park = area(
        AreaKind::Park,
        rect(
            CENTER + Vec2::new(-100.0, -50.0),
            CENTER + Vec2::new(100.0, 50.0),
        ),
    );
    let mut roads = vec![bare_footway(vec![
        CENTER + Vec2::new(-80.0, 0.0),
        CENTER + Vec2::new(80.0, 0.0),
    ])];
    let report = infer_pavements(InferPavementsCtx {
        roads: &mut roads,
        parks: &[park],
        woods: &[],
        grass: &[],
    });
    assert_eq!(
        (report.asked, report.unpaved, report.between_paved),
        (1, 1, 0)
    );
    assert_eq!(roads[0].pavement, Some(Pavement::Unpaved));
}

/// Тропа по газону, оба конца которой — точки мощёных дорожек, — звено
/// мощёной сети: лучи сквера от аллеи к лестницам (Тула, остров кольца,
/// витрина 04). Мощёные строятся тегом — поле `pavement` уже стоит.
#[test]
fn a_trail_between_paved_paths_is_paved_back() {
    let grass = area(
        AreaKind::Grass,
        rect(
            CENTER + Vec2::new(-50.0, -50.0),
            CENTER + Vec2::new(50.0, 50.0),
        ),
    );
    let alley = |points: Vec<Vec2>| RoadLine {
        class: RoadClass::Alley,
        pavement: Some(Pavement::Paved),
        ..street(points, 3.0)
    };
    let at = |x: f32, y: f32| CENTER + Vec2::new(x, y);
    let mut roads = vec![
        alley(vec![at(-40.0, 0.0), at(0.0, 0.0)]),
        bare_footway(vec![at(0.0, 0.0), at(0.0, 40.0)]),
        alley(vec![at(0.0, 40.0), at(40.0, 40.0)]),
    ];
    let report = infer_pavements(InferPavementsCtx {
        roads: &mut roads,
        parks: &[],
        woods: &[],
        grass: &[grass],
    });
    assert_eq!(
        (report.asked, report.unpaved, report.between_paved),
        (1, 0, 1)
    );
    assert_eq!(roads[1].pavement, Some(Pavement::Paved));
}
