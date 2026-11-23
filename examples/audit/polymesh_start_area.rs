//! Проверка стартовой области: стоит ли портал города на полигональном меше,
//! чиста ли кромка, по которой рождаются демоны, и что от портала достижимо.
//! Париж ловился именно здесь — центр карты на острове Сите, а остров был дырой
//! водного мультиполигона «La Seine» и целиком уходил в препятствие.
//!
//! Портал берётся тот, с которым играет загрузка: заливка сетки → снап по
//! сетке (`navigation::snap_portal_position`) → калитки и прунинг от него.
//! Сырой хинт города (`City::portal_hint`) печатается рядом: хинт может лежать
//! внутри препятствия, и по двум строкам видно, что снап увёл портал на чистое
//! место, а не оставил его там.
//!
//! На город — три строки:
//!
//! - **кромка** — точки окружности рождения демонов (`PORTAL_DIAMETER / 2 +
//!   `PORTAL_RIM_MARGIN`, `demon::systems`): ровно на меше и не на закрытом
//!   тайле; и квадрат клиренса снапа после калиток с прунингом — сам снап
//!   считает его до обоих;
//! - **решётка** шага [`LATTICE_STEP`] — что от портала достижимо, по центрам
//!   тайлов под её точками (на центрах стоят пешки). После прунинга всё
//!   свободное по сетке достижимо по сетке, поэтому расходятся заполнения
//!   тремя способами: центр свободного тайла глубже допуска посадки внутри
//!   препятствия меша; точка сидит на меше, но пути от портала по мешу нет —
//!   с тайла, свободного сеткой (компонента меша, отрезанная от стартальной,
//!   настоящее расхождение) или закрытого (карман, который исключила сетка);
//!   и достижимо по мешу с тайла, закрытого сеткой.
//! - **пробы** в километре-полутором — не отрезанный ли карман сама стартовая
//!   область.
//!
//! Печатает заодно число дыр во входных кольцах и время постройки: дыры идут
//! в то же boolean-объединение, и цена вопроса видна здесь.
//!
//! ```text
//! cargo run --release --example polymesh_start_area -- [radius] [city ...]
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::{IVec2, Vec2};
use qwe::city::City;
use qwe::map::osm::overpass;
use qwe::navigation::{
    Navmesh, PolymeshBuild, build_polymesh_from_map, find_path_polymesh, portal_clearance_tiles,
    snap_portal_position,
};
use qwe::settings::{MAP_SIZE, PORTAL_DIAMETER, PORTAL_RIM_MARGIN};

/// Пробы вокруг портала: за мостом с каждой стороны, если портал на острове.
const PROBES: [Vec2; 4] = [
    Vec2::new(1500.0, 0.0),
    Vec2::new(-1500.0, 0.0),
    Vec2::new(0.0, 1000.0),
    Vec2::new(0.0, -1000.0),
];

/// Шаг решётки достижимости, м.
const LATTICE_STEP: f32 = 200.0;
/// Точек на окружности рождения демонов.
const RIM_POINTS: usize = 72;
/// Сколько точек решётки печатать поимённо.
const LISTED: usize = 16;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let radius: f32 = args
        .first()
        .map(|a| a.parse().expect("radius"))
        .unwrap_or(0.4);
    let cities: Vec<City> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|name| {
                City::ALL
                    .into_iter()
                    .find(|city| city.slug() == name.to_lowercase())
                    .unwrap_or_else(|| panic!("unknown city {name}"))
            })
            .collect()
    } else {
        City::ALL.to_vec()
    };

    for city in cities {
        // город без скачанного кеша — строка, а не паника: полный список
        // обязан доезжать до последнего города
        let cache = overpass::cache_path(city);
        if !cache.exists() {
            println!("{city:?}: skipped, no OSM cache at {}", cache.display());
            continue;
        }
        audit(city, radius);
    }
}

fn audit(city: City, radius: f32) {
    let mut map = common::load_map(city);
    let hint = city.portal_hint();
    // как поток загрузки (`map/prepare.rs::fill_navmesh_and_snap_portal`):
    // заливка, снап по сетке, затем калитки и прунинг от портала
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(&map);
    let snapped = snap_portal_position(&navmesh, hint);
    let portal = snapped.unwrap_or(hint);
    navmesh.open_gates_and_prune(&mut map, portal, |_, _| {});

    let holes: usize = map
        .buildings
        .iter()
        .chain(&map.water)
        .map(|area| area.holes.len())
        .sum();
    let started = Instant::now();
    let build = build_polymesh_from_map(&map, radius).expect("not cancelled");
    let elapsed = started.elapsed();
    let empty = build
        .mesh
        .layers
        .iter()
        .filter(|layer| layer.polygons.is_empty())
        .count();
    let polygons: usize = build.mesh.layers.iter().map(|l| l.polygons.len()).sum();
    // точный ответ «на меше»: `contains` терпит кольцо проб в четверть метра
    let on_mesh = |point: Vec2| build.nearest_free_point(point) == Some(point);
    let where_to = match snapped {
        Some(position) if navmesh.to_tile(position) == navmesh.to_tile(hint) => {
            "in place".to_string()
        }
        Some(position) => format!("moved {} m", (position - hint).length()),
        None => "NO CLEAR SPOT, the raw hint kept".to_string(),
    };
    println!(
        "{city:?}: hint {hint:?} on mesh = {}; portal {portal:?} ({where_to}), \
         on mesh exactly = {}; {holes} ring holes in input, {polygons} polygons, \
         {empty} empty layers, {} obstacle contours, built in {elapsed:?}",
        build.contains(hint),
        on_mesh(portal),
        build.obstacles.len(),
    );
    if !build.contains(hint) {
        println!(
            "   nearest free point to the hint: {:?}",
            build.nearest_free_point(hint)
        );
    }

    rim_and_square(&navmesh, &build, portal);
    lattice(&navmesh, &build, portal);

    // стоять на меше мало: стартовая область могла оказаться отрезанным
    // карманом. Пробы вокруг портала — четыре точки в километре с
    // небольшим, каждая подсаженная на меш
    for offset in PROBES {
        let target = portal + offset;
        let verdict = match build.nearest_free_point(target) {
            None => "off the mesh".to_string(),
            Some(probe) => match find_path_polymesh(&build, portal, probe) {
                Some(path) => format!("reachable, {} waypoints", path.len()),
                None => "NO PATH".to_string(),
            },
        };
        println!("   probe {target:?}: {verdict}");
    }
}

/// Проходим ли тайл под мировой точкой.
fn on_grid(navmesh: &Navmesh, point: Vec2) -> bool {
    let tile = navmesh.to_tile(point);
    navmesh.is_passable(tile.x, tile.y)
}

/// Кромка рождения демонов и квадрат клиренса снапа — после калиток и прунинга:
/// снап проверяет квадрат до обоих, а прунинг умеет только закрывать.
fn rim_and_square(navmesh: &Navmesh, build: &PolymeshBuild, portal: Vec2) {
    let rim = PORTAL_DIAMETER / 2.0 + PORTAL_RIM_MARGIN;
    let (mut off_mesh, mut blocked) = (0usize, 0usize);
    for step in 0..RIM_POINTS {
        let angle = step as f32 / RIM_POINTS as f32 * std::f32::consts::TAU;
        let point = portal + Vec2::from_angle(angle) * rim;
        off_mesh += (build.nearest_free_point(point) != Some(point)) as usize;
        blocked += !on_grid(navmesh, point) as usize;
    }
    let clearance = portal_clearance_tiles(navmesh.tile_size);
    let centre = navmesh.to_tile(portal);
    let square: usize = (-clearance..=clearance)
        .flat_map(|dx| {
            (-clearance..=clearance)
                .map(move |dy| !on_grid(navmesh, navmesh.tile_center(centre + IVec2::new(dx, dy))))
        })
        .map(|blocked| blocked as usize)
        .sum();
    println!(
        "   demon rim: {off_mesh} of {RIM_POINTS} exactly off the mesh, {blocked} on blocked tiles; \
         clearance square after gates and prune: {square} of {} tiles blocked",
        (2 * clearance + 1) * (2 * clearance + 1)
    );
}

/// Что от портала достижимо: решётка по всей карте, меш против сетки. Точки
/// решётки — центры тайлов под ними: на центрах стоят пешки, центры просеивает
/// выбор цели.
fn lattice(navmesh: &Navmesh, build: &PolymeshBuild, portal: Vec2) {
    let columns = (MAP_SIZE.x / LATTICE_STEP) as usize;
    let rows = (MAP_SIZE.y / LATTICE_STEP) as usize;
    let (mut points, mut free, mut reachable, mut reachable_free) = (0usize, 0, 0, 0);
    let (mut off_mesh, mut cut_off) = (Vec::new(), Vec::new());
    let mut cut_off_blocked = 0usize;
    for ix in 0..columns {
        for iy in 0..rows {
            let spot = Vec2::new(
                LATTICE_STEP * (ix as f32 + 0.5),
                LATTICE_STEP * (iy as f32 + 0.5),
            );
            let point = navmesh.tile_center(navmesh.to_tile(spot));
            points += 1;
            let grid_free = on_grid(navmesh, point);
            free += grid_free as usize;
            let Some(seat) = build.nearest_free_point(point) else {
                if grid_free {
                    off_mesh.push(point);
                }
                continue;
            };
            if find_path_polymesh(build, portal, seat).is_some() {
                reachable += 1;
                reachable_free += grid_free as usize;
            } else if grid_free {
                cut_off.push(point);
            } else {
                cut_off_blocked += 1;
            }
        }
    }
    println!(
        "   lattice {LATTICE_STEP} m: {points} tile centres, {free} free by the grid, \
         {reachable} reachable from the portal on the mesh \
         ({} of them on a tile the grid blocks)",
        reachable - reachable_free
    );
    println!(
        "   free by the grid yet off the mesh: {}; on the mesh yet cut off from the portal: \
         {}, of them free by the grid: {}",
        listed(&off_mesh),
        cut_off.len() + cut_off_blocked,
        listed(&cut_off),
    );
}

/// Число точек и первые [`LISTED`] поимённо.
fn listed(points: &[Vec2]) -> String {
    let shown: Vec<String> = points
        .iter()
        .take(LISTED)
        .map(|point| format!("({:.0}, {:.0})", point.x, point.y))
        .collect();
    let more = if points.len() > LISTED { ", …" } else { "" };
    format!("{} [{}{more}]", points.len(), shown.join(", "))
}
