use super::*;
use crate::grid::world_to_tile;
use crate::map::osm::fixture::{bridge, building, fence, footway, rect, street};
use crate::map::osm::model::FenceLine;

/// Центр участка в тестах оград.
const PLOT: Vec2 = Vec2::new(300.0, 300.0);

/// Точка участка, повёрнутого на `degrees` вокруг [`PLOT`]: косой забор — это
/// там, где 4-связная цепочка тайлов и проём в ней могут разойтись.
fn plot_point(local: Vec2, degrees: f32) -> Vec2 {
    PLOT + Vec2::from_angle(degrees.to_radians()).rotate(local)
}

/// Замкнутая ограда квадратного участка со стороной 40 м.
fn plot_fence(degrees: f32) -> FenceLine {
    let corners = [
        (-20.0, -20.0),
        (20.0, -20.0),
        (20.0, 20.0),
        (-20.0, 20.0),
        (-20.0, -20.0),
    ];
    fence(
        corners
            .iter()
            .map(|&(x, y)| plot_point(Vec2::new(x, y), degrees))
            .collect(),
    )
}

/// Достижим ли центр участка с улицы после прунинга от точки снаружи.
fn plot_reachable(map: &MapData, degrees: f32) -> bool {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    navmesh.prune_unreachable(plot_point(Vec2::new(-60.0, 0.0), degrees));
    let inside = world_to_tile(PLOT);
    navmesh.is_passable(inside.x, inside.y)
}

const PLOT_ANGLES: [f32; 5] = [0.0, 17.0, 30.0, 45.0, 71.0];

/// Замкнутая ограда без калитки запирает участок при любом угле: забор
/// растеризован цепочкой по осевой, и косая линия не рассыпается в шахматку.
#[test]
fn a_closed_fence_seals_the_plot() {
    for degrees in PLOT_ANGLES {
        let mut map = MapData::default();
        map.fences.push(plot_fence(degrees));
        assert!(!plot_reachable(&map, degrees), "участок под {degrees}°");
    }
}

/// Тропинка сквозь ограду делает калитку — и тоже при любом угле: вынутый из
/// косой цепочки один тайл щели не даёт, поэтому проём шире на диагональ тайла.
#[test]
fn a_footway_through_the_fence_opens_a_gate() {
    for degrees in PLOT_ANGLES {
        let mut map = MapData::default();
        map.fences.push(plot_fence(degrees));
        map.roads.push(footway(vec![
            plot_point(Vec2::new(-60.0, 3.0), degrees),
            plot_point(Vec2::new(0.0, 3.0), degrees),
        ]));
        assert!(plot_reachable(&map, degrees), "калитка под {degrees}°");
    }
}

/// Тропа, оборванная у самой калитки, осевой забора не пересекает, но
/// упирается в него — это тоже проём.
#[test]
fn a_footway_ending_at_the_fence_opens_a_gate() {
    let mut map = MapData::default();
    map.fences.push(plot_fence(0.0));
    map.roads.push(footway(vec![
        plot_point(Vec2::new(-60.0, 0.0), 0.0),
        plot_point(Vec2::new(-21.0, 0.0), 0.0),
    ]));
    assert!(plot_reachable(&map, 0.0));
}

/// Главная ловушка: улица вдоль забора накрывает его своей номинальной лентой
/// на всём протяжении, но осевой не пересекает — забор остаётся целым.
#[test]
fn a_street_along_the_fence_leaves_it_whole() {
    let mut map = MapData::default();
    map.fences.push(plot_fence(0.0));
    map.roads.push(street(
        vec![
            plot_point(Vec2::new(-80.0, -24.0), 0.0),
            plot_point(Vec2::new(80.0, -24.0), 0.0),
        ],
        16.0,
    ));
    assert!(!plot_reachable(&map, 0.0));
}

/// Мост идёт над оградой, а не сквозь неё.
#[test]
fn a_bridge_over_the_fence_opens_nothing() {
    let mut map = MapData::default();
    map.fences.push(plot_fence(0.0));
    map.roads.push(bridge(
        vec![
            plot_point(Vec2::new(-60.0, 0.0), 0.0),
            plot_point(Vec2::new(60.0, 0.0), 0.0),
        ],
        5.0,
    ));
    assert!(!plot_reachable(&map, 0.0));
}

/// Глухой участок с дверью получает калитку по умолчанию — и на той стороне,
/// что смотрит на улицу, а не на первом попавшемся краю: вход в огороженную
/// школу делают с большой дороги.
#[test]
fn a_sealed_plot_gets_its_gate_on_the_street_side() {
    let mut map = MapData::default();
    map.fences.push(plot_fence(0.0));
    let mut house = building(
        rect(
            Vec2::new(PLOT.x - 5.0, PLOT.y - 5.0),
            Vec2::new(PLOT.x + 5.0, PLOT.y + 5.0),
        ),
        vec![],
    );
    house.entrances.push(Vec2::new(PLOT.x, PLOT.y - 5.0));
    map.buildings.push(house);
    // улица вдоль северной стороны, в 6 м за оградой
    map.roads.push(street(
        vec![
            Vec2::new(PLOT.x - 80.0, PLOT.y + 30.0),
            Vec2::new(PLOT.x + 80.0, PLOT.y + 30.0),
        ],
        8.0,
    ));
    let portal = plot_point(Vec2::new(-60.0, 0.0), 0.0);
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    assert!(navmesh.open_sealed_fences(&mut map, portal) >= 1);
    let gates = &map.fences[0].gates;
    assert!(
        gates
            .iter()
            .all(|gate| (gate.y - (PLOT.y + 20.0)).abs() < 1.0),
        "калитки {gates:?} не на северной стороне"
    );
    navmesh.prune_unreachable(portal);
    let inside = world_to_tile(Vec2::new(PLOT.x, PLOT.y - 12.0));
    assert!(navmesh.is_passable(inside.x, inside.y), "двор открыт");
}

/// Проём вынимается из маски забора, а не прорезается по сетке: дом, в который
/// упирается тропа сразу за калиткой, остаётся непроходимым.
#[test]
fn a_gate_does_not_open_the_house_behind_it() {
    let mut map = MapData::default();
    map.fences.push(plot_fence(0.0));
    map.buildings.push(building(
        rect(
            Vec2::new(PLOT.x - 19.0, PLOT.y - 6.0),
            Vec2::new(PLOT.x - 5.0, PLOT.y + 6.0),
        ),
        vec![],
    ));
    map.roads.push(footway(vec![
        Vec2::new(PLOT.x - 60.0, PLOT.y),
        Vec2::new(PLOT.x - 10.0, PLOT.y),
    ]));
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let tile = world_to_tile(Vec2::new(PLOT.x - 17.0, PLOT.y));
    assert!(!navmesh.is_passable(tile.x, tile.y), "дом за калиткой");
}
