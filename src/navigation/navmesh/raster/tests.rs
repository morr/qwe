use super::*;
use crate::grid::navtile_size;
use crate::map::osm::fixture::{building, rect};
use crate::map::osm::model::point_in_polygon;

/// Тайлы строки `y`, залитые построчной заливкой. Отрезки обрезаются по
/// ширине проверяемой полосы — как `set_area` обрезает их по сетке.
fn scanline_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
    let mut scratch = RowScratch::default();
    row_spans(outer, holes, y, navtile_size(), &mut scratch);
    scratch
        .spans
        .iter()
        .flat_map(|&(from, to)| from.max(0)..=to.min(width - 1))
        .collect::<Vec<_>>()
}

/// Те же тайлы точечной проверкой — эталон, который заменила заливка.
fn point_test_row(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, width: i32) -> Vec<i32> {
    (0..width)
        .filter(|&x| {
            let center = (Vec2::new(x as f32, y as f32) + 0.5) * navtile_size();
            point_in_polygon(center, outer)
                && !holes.iter().any(|hole| point_in_polygon(center, hole))
        })
        .collect()
}

fn assert_same_fill(outer: &[Vec2], holes: &[Vec<Vec2>], rows: i32, width: i32) {
    for y in 0..rows {
        assert_eq!(
            scanline_row(outer, holes, y, width),
            point_test_row(outer, holes, y, width),
            "row {y}"
        );
    }
}

#[test]
fn scanline_matches_point_test_for_a_rect_with_a_hole() {
    let outer = rect(Vec2::new(3.0, 5.0), Vec2::new(41.0, 33.0));
    let holes = vec![rect(Vec2::new(11.0, 13.0), Vec2::new(25.0, 27.0))];
    assert_same_fill(&outer, &holes, 20, 25);
}

/// Вогнутый контур: строка пересекает его дважды, и заливка обязана дать
/// два отрезка, а не один сплошной.
#[test]
fn scanline_matches_point_test_for_a_concave_ring() {
    let outer = vec![
        Vec2::new(2.0, 2.0),
        Vec2::new(30.0, 2.0),
        Vec2::new(30.0, 30.0),
        Vec2::new(24.0, 30.0),
        Vec2::new(24.0, 9.0),
        Vec2::new(8.0, 9.0),
        Vec2::new(8.0, 30.0),
        Vec2::new(2.0, 30.0),
    ];
    assert_same_fill(&outer, &[], 18, 18);
}

/// Дырка, наполовину вылезшая за внешнее кольцо. Если сваливать её рёбра
/// в общий even-odd список, торчащий кусок не вычтется, а зальётся —
/// именно поэтому дырки вычитаются отрезками.
#[test]
fn scanline_matches_point_test_for_a_hole_sticking_out() {
    let outer = rect(Vec2::new(6.0, 6.0), Vec2::new(30.0, 30.0));
    let holes = vec![rect(Vec2::new(20.0, 12.0), Vec2::new(44.0, 22.0))];
    assert_same_fill(&outer, &holes, 18, 25);
}

/// Косые рёбра — единственное место, где заливка могла бы разъехаться с
/// точечной проверкой на полтайла.
#[test]
fn scanline_matches_point_test_for_a_diagonal_ring() {
    let outer = vec![
        Vec2::new(1.7, 0.3),
        Vec2::new(37.4, 11.9),
        Vec2::new(21.1, 34.6),
        Vec2::new(5.2, 20.8),
    ];
    assert_same_fill(&outer, &[], 20, 22);
}

/// Сетка с чужим размером тайла: снапшот вдвое мельче текущего атомика.
/// Заливка обязана считать по нему — иначе дом ляжет не туда, где он стоит.
fn half_scale_navmesh(side: i32) -> Navmesh {
    let grid_size = IVec2::splat(side);
    Navmesh {
        passable: vec![true; (grid_size.x * grid_size.y) as usize],
        grid_size,
        tile_size: navtile_size() / 2.0,
    }
}

/// Площадная заливка (вода и дома — основной объём) идёт по `tile_size`
/// **своего** снапшота, а не по процессному атомику: иначе `Navmesh`, залитый
/// при одном размере навтайла, растеризует дом в масштабе другого.
#[test]
fn set_area_rasterises_by_the_navmesh_own_tile_size() {
    let mut navmesh = half_scale_navmesh(64);
    let tile_size = navmesh.tile_size;
    let area = building(rect(Vec2::new(10.0, 10.0), Vec2::new(30.0, 20.0)), vec![]);

    navmesh.set_area(&area, false);

    for x in 0..navmesh.grid_size.x {
        for y in 0..navmesh.grid_size.y {
            let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
            assert_eq!(
                navmesh.is_passable(x, y),
                !point_in_polygon(center, &area.outer),
                "тайл ({x}, {y}), центр {center:?}"
            );
        }
    }
}

/// То же для ленты: границы перебора и — главное — стартовый тайл цепочки по
/// осевой (`visit_segment_tiles`) берутся из снапшота. По атомику стена уехала
/// бы в другую строку сетки целиком.
#[test]
fn set_polyline_rasterises_by_the_navmesh_own_tile_size() {
    let mut navmesh = half_scale_navmesh(64);
    let tile_size = navmesh.tile_size;
    let (from, to) = (Vec2::new(10.0, 10.0), Vec2::new(30.0, 10.0));

    navmesh.set_polyline(&[from, to], 2.0, false);

    let tile_at = |point: Vec2| (point / tile_size).floor().as_ivec2();
    for point in [Vec2::new(20.0, 9.5), Vec2::new(20.0, 10.5)] {
        let tile = tile_at(point);
        assert!(
            !navmesh.is_passable(tile.x, tile.y),
            "лента стены в точке {point:?}"
        );
    }
    for point in [Vec2::new(20.0, 5.5), Vec2::new(20.0, 14.5)] {
        let tile = tile_at(point);
        assert!(
            navmesh.is_passable(tile.x, tile.y),
            "земля в стороне от стены, {point:?}"
        );
    }
}
