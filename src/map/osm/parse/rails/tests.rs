use super::super::fixtures::{CITY, HALF, corners};
use super::*;
use crate::map::osm::fixture::Overpass;
use crate::map::osm::model::{RailKind, ServiceTrack};

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
    assert!((map.roads[0].width - 7.6).abs() < 1e-4);
    assert_eq!(map.roads[0].points, map.rails[2].points);

    // рельсы существуют только для картинки — навмеша они не касаются
    assert!(map.walls.is_empty());
    assert!(map.buildings.is_empty());
}

/// Путь на мосту — путепровод (Орёл, 7245, 728: `bridge=yes`, `layer=1` над
/// Р-119); `bridge=no` — явное «нет», как и отсутствие тега.
#[test]
fn a_track_on_a_bridge_is_marked_as_one() {
    let (sw, se, ne, nw) = corners(HALF);
    let map = Overpass::new(CITY)
        .way(
            &[("railway", "rail"), ("bridge", "yes"), ("layer", "1")],
            vec![sw, ne],
        )
        .way(&[("railway", "rail"), ("bridge", "viaduct")], vec![se, nw])
        .way(&[("railway", "rail"), ("bridge", "no")], vec![sw, se])
        .way(&[("railway", "rail")], vec![nw, ne])
        .parse();

    let bridges: Vec<bool> = map.rails.iter().map(|rail| rail.bridge).collect();
    assert_eq!(bridges, [true, true, false, false]);
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

use crate::map::osm::fixture;

/// Улица поперёк оси x = 0 на высоте `y` и два пути с юга и севера,
/// оборванные в `gap` метрах друг от друга.
fn crossing(gap: f32, north_x: f32) -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(-60.0, 0.0), Vec2::new(60.0, 0.0)],
        8.0,
    ));
    map.rails.push(fixture::rail(
        vec![Vec2::new(0.0, -80.0), Vec2::new(0.0, -gap / 2.0)],
        5.0,
    ));
    map.rails.push(fixture::rail(
        vec![Vec2::new(north_x, gap / 2.0), Vec2::new(north_x, 80.0)],
        5.0,
    ));
    map
}

#[test]
fn a_track_broken_across_a_street_is_stitched() {
    let mut map = crossing(18.0, 0.0);
    assert_eq!(stitch_rail_gaps(&mut map), 1);
    assert_eq!(map.rails.len(), 1);
    let points = &map.rails[0].points;
    assert_eq!(points.first(), Some(&Vec2::new(0.0, -80.0)));
    assert_eq!(points.last(), Some(&Vec2::new(0.0, 80.0)));
}

#[test]
fn the_stitch_holds_whichever_way_the_points_run() {
    let mut map = crossing(18.0, 0.0);
    map.rails[0].points.reverse();
    map.rails[1].points.reverse();
    assert_eq!(stitch_rail_gaps(&mut map), 1);
    assert_eq!(map.rails[0].points.len(), 4);
}

#[test]
fn no_street_in_the_gap_leaves_it_open() {
    let mut map = crossing(18.0, 0.0);
    map.roads.clear();
    assert_eq!(stitch_rail_gaps(&mut map), 0);
    assert_eq!(map.rails.len(), 2);
}

#[test]
fn a_bridge_in_the_gap_is_not_a_level_crossing() {
    let mut map = crossing(18.0, 0.0);
    map.roads[0].bridge = true;
    assert_eq!(stitch_rail_gaps(&mut map), 0);
}

#[test]
fn a_service_drive_in_the_gap_is_a_level_crossing() {
    let mut map = crossing(18.0, 0.0);
    // уже 7 м — фикстура делает дворовый проезд
    map.roads[0] = fixture::street(vec![Vec2::new(-60.0, 0.0), Vec2::new(60.0, 0.0)], 5.0);
    assert_eq!(stitch_rail_gaps(&mut map), 1);
}

#[test]
fn a_gap_too_long_or_ends_looking_past_each_other_stay_open() {
    assert_eq!(stitch_rail_gaps(&mut crossing(36.0, 0.0)), 0);
    // 6 м вбок на 18 м разрыва — 18° мимо
    assert_eq!(stitch_rail_gaps(&mut crossing(18.0, 6.0)), 0);
}

#[test]
fn an_end_on_a_switch_is_not_free() {
    let mut map = crossing(18.0, 0.0);
    // ответвление от южного конца: он — узел стрелки, не обрыв
    map.rails.push(fixture::rail(
        vec![Vec2::new(0.0, -9.0), Vec2::new(-30.0, -40.0)],
        5.0,
    ));
    assert_eq!(stitch_rail_gaps(&mut map), 0);
}
