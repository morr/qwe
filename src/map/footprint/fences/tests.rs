use super::*;
use crate::map::osm::fixture;

fn length(piece: &[Vec2]) -> f32 {
    piece.windows(2).map(|pair| pair[0].distance(pair[1])).sum()
}

/// Тропинка поперёк забора: проём на пересечении осевых, забор рисуется
/// двумя кусками, обрезанными ровно по кругу проёма.
#[test]
fn a_footway_splits_the_drawn_fence_at_its_gap() {
    let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
    let roads = [fixture::footway(vec![
        Vec2::new(20.0, -10.0),
        Vec2::new(20.0, 10.0),
    ])];
    let gaps = &fence_gaps(std::slice::from_ref(&fence), &roads)[0];
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].at.distance(Vec2::new(20.0, 0.0)) < 1e-3);
    let pieces = fence_pieces(&fence, gaps, &BridgeDecks::build(&roads, 0.0));
    assert_eq!(pieces.len(), 2);
    let reach = gaps[0].reach;
    assert!((pieces[0].last().unwrap().x - (20.0 - reach)).abs() < 1e-3);
    assert!((pieces[1].first().unwrap().x - (20.0 + reach)).abs() < 1e-3);
}

/// Улица вдоль забора его не режет — ни в навмеше, ни на картинке.
#[test]
fn a_street_along_the_fence_leaves_no_gap() {
    let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
    let road = fixture::street(vec![Vec2::new(-10.0, 4.0), Vec2::new(50.0, 4.0)], 16.0);
    assert!(fence_gaps(std::slice::from_ref(&fence), &[road])[0].is_empty());
}

/// Въезд, брошенный OSM в нескольких метрах от ограды и смотрящий на неё,
/// открывает ворота: на месте он идёт сквозь них, и отрисовка дотягивает
/// его туда стежком. Дальше [`GAP_END_REACH`] — уже задуманный тупик.
#[test]
fn a_drive_dropped_short_of_the_fence_opens_a_gate() {
    let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
    let short =
        |gap: f32| fixture::street(vec![Vec2::new(20.0, -20.0), Vec2::new(20.0, -gap)], 5.0);
    let gaps = &fence_gaps(std::slice::from_ref(&fence), &[short(4.0)])[0];
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].at.distance(Vec2::new(20.0, 0.0)) < 1e-3);
    assert!((gaps[0].reach - 2.5).abs() < 1e-3, "{}", gaps[0].reach);
    assert!(fence_gaps(std::slice::from_ref(&fence), &[short(GAP_END_REACH + 1.0)])[0].is_empty());
}

/// Но только висячий торец и только смотрящий вперёд: улица, идущая вдоль
/// забора, и way, разрезанная картографом посреди проезда, ворот не
/// открывают.
#[test]
fn a_way_split_beside_the_fence_opens_nothing() {
    let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
    // торец в четырёх метрах сбоку от ограды: дорога идёт вдоль неё
    let along = fixture::street(vec![Vec2::new(0.0, -4.0), Vec2::new(20.0, -4.0)], 5.0);
    assert!(fence_gaps(std::slice::from_ref(&fence), &[along])[0].is_empty());

    // тот же торец, но полотно продолжает вторая way — торец не висячий
    let first = fixture::street(vec![Vec2::new(20.0, -20.0), Vec2::new(20.0, -4.0)], 5.0);
    let second = fixture::street(vec![Vec2::new(20.0, -4.0), Vec2::new(32.0, -8.0)], 5.0);
    assert!(fence_gaps(std::slice::from_ref(&fence), &[first, second])[0].is_empty());
}

/// Проём на изломе режет оба звена, а кольцо ограды с калиткой остаётся
/// ломаной без калитки: длина кусков — периметр минус проём. Огрызок короче
/// полуметра у края не рисуется.
#[test]
fn a_gate_at_a_corner_cuts_both_links_and_drops_the_stub() {
    let fence = fixture::fence(fixture::closed(fixture::square(Vec2::ZERO, 10.0)));
    let gate = FenceGap {
        at: Vec2::new(-10.0, -10.0),
        reach: 1.75,
    };
    let pieces = fence_pieces(&fence, &[gate], &BridgeDecks::build(&[], 0.0));
    let total: f32 = pieces.iter().map(|piece| length(piece)).sum();
    assert!((total - (80.0 - 2.0 * 1.75)).abs() < 1e-3, "{total}");

    // за проёмом остаётся 0.3 м забора — не рисуется
    let stub = FenceGap {
        at: Vec2::new(19.0, 0.0),
        reach: 0.7,
    };
    let line = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0)]);
    let pieces = fence_pieces(&line, &[stub], &BridgeDecks::build(&[], 0.0));
    assert_eq!(pieces.len(), 1);
    assert!((pieces[0][1].x - 18.3).abs() < 1e-3);
}

/// Мост над оградой: проходимости он не меняет (`fence_gaps` мостов не
/// знает), но нарисованный забор под настилом обрывается — ровно по
/// кромке нарисованного моста, то есть по бордюру, а не по краю полотна.
#[test]
fn a_bridge_hides_the_stretch_of_fence_it_covers() {
    let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)]);
    let roads = [fixture::bridge(
        vec![Vec2::new(50.0, -40.0), Vec2::new(50.0, 40.0)],
        16.0,
    )];

    let gaps = &fence_gaps(std::slice::from_ref(&fence), &roads)[0];
    assert!(gaps.is_empty(), "мост не открывает проём: {gaps:?}");

    let pieces = fence_pieces(&fence, gaps, &BridgeDecks::build(&roads, 0.0));
    assert_eq!(pieces.len(), 2);
    let reach = roads[0].curb_reach();
    assert!((pieces[0].last().unwrap().x - (50.0 - reach)).abs() < 1e-3);
    assert!((pieces[1].first().unwrap().x - (50.0 + reach)).abs() < 1e-3);
}

/// Забор **вдоль** моста накрыт лентой по всей длине, а не кругом в точке
/// пересечения осевых: под настилом его не видно целиком.
#[test]
fn a_fence_running_under_a_deck_is_hidden_along_its_whole_length() {
    let fence = fixture::fence(vec![Vec2::new(-20.0, 2.0), Vec2::new(60.0, 2.0)]);
    let roads = [fixture::bridge(
        vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)],
        16.0,
    )];

    let pieces = fence_pieces(&fence, &[], &BridgeDecks::build(&roads, 0.0));
    assert_eq!(pieces.len(), 2);
    // торец настила скруглён: капсула достаёт на полуширину за него
    let reach = (roads[0].curb_reach().powi(2) - 2.0f32.powi(2)).sqrt();
    assert!((pieces[0].last().unwrap().x - -reach).abs() < 1e-3);
    assert!((pieces[1].first().unwrap().x - (40.0 + reach)).abs() < 1e-3);
}
