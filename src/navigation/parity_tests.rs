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
use crate::map::osm::fixture::{TinyCity, building, rect, stream, tiny_city, wall};

/// Радиус агента меша — маленький: пробные точки стоят от кромок дальше
/// инфляции, паритет сверяет правила заливки, а не толщину раздувания.
const AGENT_RADIUS: f32 = 0.2;

struct BuiltCity {
    city: TinyCity,
    navmesh: Navmesh,
    mesh: PolymeshBuild,
}

fn built_city() -> BuiltCity {
    built_from(tiny_city())
}

/// Тот же конвейер поверх [`tiny_city`] с зонами, которых в ней нет.
fn built_from(city: TinyCity) -> BuiltCity {
    let mut city = city;
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&city.map);
    // калитки по умолчанию ложатся в карту до постройки меша — как в игре,
    // где меш строится из `MapData`, уже прошедшей поток загрузки
    navmesh.open_gates_and_prune(&mut city.map, city.portal, |_, _| {});
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

/// Правило примыкающей дороги — только проём в бордюре. Ручей поперёк улицы,
/// примыкающей к сухому мосту, и дом на той же улице: полотно улицы не может
/// их открыть. В меше оно было общей прорезью — вычиталось из всех препятствий
/// разом, и улица проходила сквозь воду и фасад (в Париже под такими дорогами
/// стояло 1 973 тайла домов и 641 воды). Теперь полотно вычитается только из
/// полос бордюра, как в сетке, где оно лишь снимает блокировку с бордюрных
/// тайлов.
#[test]
fn a_joining_road_opens_the_curb_and_nothing_else_in_both_fills() {
    let mut city = tiny_city();
    city.map.water_lines.push(stream(
        vec![(700.0, 2100.0).into(), (700.0, 2200.0).into()],
        4.0,
    ));
    city.map.buildings.push(building(
        rect((560.0, 2140.0).into(), (620.0, 2160.0).into()),
        Vec::new(),
    ));
    let built = built_from(city);
    assert_both_blocked(
        &built,
        (700.0, 2150.0).into(),
        "ручей поперёк примыкающей улицы",
    );
    assert_both_blocked(&built, (610.0, 2150.0).into(), "дом на примыкающей улице");
    assert_both_free(&built, (750.0, 2150.0).into(), "улица у моста");
    assert_both_reach(
        &built,
        (750.0, 2150.0).into(),
        built.city.dry_deck,
        "улица -> настил через проём в бордюре",
    );
}

/// Стена (`barrier=city_wall` и родственные) — полоса, блокирующая обе заливки,
/// с круглым торцом, как у капсулы сетки: за последним узлом стена ещё на
/// полширины. До этого пина правило сетки не держал ни один тест, а в меше
/// лента была срезана ровно.
#[test]
fn a_wall_blocks_in_both_fills() {
    let mut city = tiny_city();
    city.map.walls.push(wall(
        vec![(2400.0, 1201.0).into(), (2600.0, 1201.0).into()],
        3.0,
    ));
    let built = built_from(city);
    assert_both_blocked(&built, (2500.0, 1201.0).into(), "стена");
    assert_both_blocked(&built, (2601.0, 1201.0).into(), "торец стены");
    assert_both_free(&built, (2500.0, 1211.0).into(), "земля за стеной");
    assert_both_free(&built, (2500.0, 1191.0).into(), "земля перед стеной");
    assert_both_free(&built, (2604.0, 1201.0).into(), "проход за торцом стены");
    assert_both_reach(
        &built,
        (2500.0, 1211.0).into(),
        (2500.0, 1191.0).into(),
        "обход стены по торцу",
    );
}

/// Свободный торец водотока круглый в обеих заливках: за последним узлом вода
/// ещё на полширины русла (у сетки — капсула `set_polyline`). В меше лента
/// была срезана ровно, и за торцом ручья оставался свободный клин, которого
/// сетка не знает.
#[test]
fn a_free_stream_end_is_round_in_both_fills() {
    let mut city = tiny_city();
    city.map.water_lines.push(stream(
        vec![(1200.0, 1500.0).into(), (1200.0, 1600.0).into()],
        4.0,
    ));
    let built = built_from(city);
    assert_both_blocked(
        &built,
        (1200.0, 1601.0).into(),
        "вода за свободным торцом ручья",
    );
    assert_both_free(&built, (1200.0, 1605.0).into(), "земля за капом ручья");
}

#[test]
fn an_island_hole_is_opened_by_its_bridge_in_both_fills() {
    let built = built_city();
    assert_both_reach(&built, built.city.island_bank, built.city.island, "остров");
    assert_both_blocked(&built, built.city.island_water, "вода вокруг острова");
}

/// Ограда перекрывает оба заполнения, тропинка делает в ней калитку, глухой
/// участок с дверью открывает калитка по умолчанию, а щель без дверей остаётся
/// отрезанной — одними и теми же проёмами в сетке и в меше.
#[test]
fn fences_block_and_gates_open_in_both_fills() {
    let built = built_city();
    assert_both_blocked(&built, built.city.fence_line, "ограда");
    assert_both_reach(
        &built,
        built.city.fenced_street,
        built.city.fenced_yard,
        "калитка тропинки",
    );
    assert_both_reach(
        &built,
        built.city.portal,
        built.city.sealed_yard,
        "калитка по умолчанию",
    );
    assert_both_blocked(&built, built.city.sealed_sliver, "глухая щель");
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
