//! Паритет двух заполнений: сетка и полигональный меш строятся из одной
//! `MapData` ([`crate::map::osm::fixture::tiny_city`]) и обязаны дать
//! одинаковые вердикты — и по достижимости пар точек, и по проходимости
//! одиночных проб. Это исполняемая форма инварианта «одно правило для двух
//! заполнений»: до этих тестов он держался на одном общем предикате
//! (`ways_joined`) и прозе, а расхождения ловились только офлайн-аудитами на
//! скачанных городах.
//!
//! Сеточная сторона зеркалит игровой конвейер целиком: заливка + прунинг от
//! портала — меш отбрасывает недостижимые карманы сам, и без прунинга паритет
//! ложно расходился бы на них.

use bevy::prelude::*;

use super::{
    Navmesh, PathfindingAlgorithm, PolymeshBuild, build_polymesh_from_map, find_path,
    find_path_polymesh,
};
use crate::grid::world_to_tile;
use crate::map::osm::fixture::{TinyCity, tiny_city};

/// Радиус агента меша — маленький: пробные точки стоят от кромок дальше
/// инфляции, паритет сверяет правила заливки, а не толщину раздувания.
const AGENT_RADIUS: f32 = 0.2;

struct BuiltCity {
    city: TinyCity,
    navmesh: Navmesh,
    mesh: PolymeshBuild,
}

fn built_city() -> BuiltCity {
    let city = tiny_city();
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&city.map);
    navmesh.prune_unreachable(world_to_tile(city.portal));
    let mesh = build_polymesh_from_map(&city.map, AGENT_RADIUS).expect("постройка не отменялась");
    BuiltCity {
        city,
        navmesh,
        mesh,
    }
}

/// Оба заполнения находят путь между точками.
fn assert_both_reach(built: &BuiltCity, from: Vec2, to: Vec2, what: &str) {
    let grid = find_path(
        &built.navmesh,
        world_to_tile(from),
        world_to_tile(to),
        PathfindingAlgorithm::Astar,
    )
    .is_some();
    let mesh = find_path_polymesh(&built.mesh, from, to).is_some();
    assert!(grid, "{what}: сетка обязана находить путь {from} -> {to}");
    assert!(mesh, "{what}: меш обязан находить путь {from} -> {to}");
}

/// Оба заполнения считают точку непроходимой.
fn assert_both_blocked(built: &BuiltCity, point: Vec2, what: &str) {
    let tile = world_to_tile(point);
    assert!(
        !built.navmesh.is_passable(tile.x, tile.y),
        "{what}: сетка обязана блокировать {point}"
    );
    assert!(
        !built.mesh.contains(point),
        "{what}: меш обязан блокировать {point}"
    );
}

/// Оба заполнения считают точку проходимой.
fn assert_both_free(built: &BuiltCity, point: Vec2, what: &str) {
    let tile = world_to_tile(point);
    assert!(
        built.navmesh.is_passable(tile.x, tile.y),
        "{what}: сетка обязана пропускать {point}"
    );
    assert!(
        built.mesh.contains(point),
        "{what}: меш обязан пропускать {point}"
    );
}

#[test]
fn the_river_is_crossed_at_the_bridge_in_both_fills() {
    let built = built_city();
    assert_both_reach(&built, built.city.south_bank, built.city.north_bank, "мост");
    assert_both_blocked(&built, built.city.river_water, "русло реки");
}

#[test]
fn a_culvert_blocks_neither_fill() {
    let built = built_city();
    assert_both_reach(
        &built,
        built.city.culvert_south,
        built.city.culvert_north,
        "кульверт",
    );
    assert_both_free(&built, built.city.culvert_mouth, "линия трубы");
}

#[test]
fn the_arch_is_the_only_way_through_the_wall_building_in_both_fills() {
    let built = built_city();
    assert_both_reach(&built, built.city.west_gate, built.city.east_gate, "арка");
    assert_both_free(&built, built.city.arch_center, "коридор арки");
    assert_both_blocked(&built, built.city.beside_arch, "фасад за капом ширины арки");
    assert_both_blocked(
        &built,
        built.city.inside_wall_building,
        "толща здания-перегородки",
    );
}

#[test]
fn dry_bridge_curbs_block_the_deck_edges_in_both_fills() {
    let built = built_city();
    assert_both_free(&built, built.city.dry_deck, "настил эстакады");
    assert_both_blocked(&built, built.city.dry_curb, "бордюр эстакады");
    assert_both_free(&built, built.city.past_dry_curb, "земля за бордюром");
    assert_both_free(
        &built,
        built.city.joined_curb_gap,
        "бордюр в створе примыкающей дороги",
    );
}

#[test]
fn an_island_hole_is_opened_by_its_bridge_in_both_fills() {
    let built = built_city();
    assert_both_reach(&built, built.city.island_bank, built.city.island, "остров");
    assert_both_blocked(&built, built.city.island_water, "вода вокруг острова");
}

/// Сквозной маршрут через полкарты: мост, открытые зоны и арка одним путём.
#[test]
fn the_whole_city_is_one_component_in_both_fills() {
    let built = built_city();
    assert_both_reach(
        &built,
        built.city.portal,
        built.city.east_gate,
        "портал -> за арку",
    );
    assert_both_reach(
        &built,
        built.city.portal,
        built.city.south_bank,
        "портал -> южный берег",
    );
}
