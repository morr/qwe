//! Что ограды делают со связностью сетки: сколько тайлов уходит в прунинг и
//! сколько домов остаётся без единой достижимой двери.
//!
//! Забор замыкает участок, а калитка в OSM размечена редко: если проём не
//! нашёлся, двор уходит в `prune_unreachable` вместе с дверями дома, к которым
//! ходят пешки. Поэтому одна и та же карта строится дважды — без оград и с
//! ними — и печатается разница.
//!
//! ```text
//! cargo run --release --example fence_prune_audit -- [city ...]
//! ```

#[path = "../common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::IVec2;
use qwe::city::City;
use qwe::grid::world_to_tile;
use qwe::map::footprint::fence_gaps;
use qwe::map::osm::MapData;
use qwe::navigation::{Navmesh, snap_portal_position};

/// Дверь достижима, если проходим её тайл или один из восьми соседей — тот же
/// круг, в котором ищет `find_passable_tile_near`.
fn door_reachable(navmesh: &Navmesh, tile: IVec2) -> bool {
    (-1..=1).any(|dx| (-1..=1).any(|dy| navmesh.is_passable(tile.x + dx, tile.y + dy)))
}

struct Report {
    pruned: usize,
    /// Калитки по умолчанию (`Navmesh::open_sealed_fences`) и их время.
    gates: usize,
    gate_time: std::time::Duration,
    /// Номера домов с дверями, у которых не достижима ни одна.
    doorless: Vec<usize>,
    /// Сетка до прунинга и после.
    filled: Navmesh,
    navmesh: Navmesh,
}

fn measure(map: &mut MapData, city: City) -> Report {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    let portal = snap_portal_position(&navmesh, city.portal_hint()).expect("portal");
    let started = Instant::now();
    let gates = navmesh.open_sealed_fences(map, portal);
    let gate_time = started.elapsed();
    let filled = navmesh.clone();
    let pruned = navmesh.prune_unreachable(portal);
    let doorless = map
        .buildings
        .iter()
        .enumerate()
        .filter(|(_, building)| !building.entrances.is_empty())
        .filter(|(_, building)| {
            !building
                .entrances
                .iter()
                .any(|&door| door_reachable(&navmesh, world_to_tile(door)))
        })
        .map(|(index, _)| index)
        .collect();
    Report {
        pruned,
        gates,
        gate_time,
        doorless,
        filled,
        navmesh,
    }
}

/// Карман, отрезанный оградами: тайлы, достижимые без оград и недостижимые с
/// ними (сами тайлы оград не в счёт), связная компонента.
struct Pocket {
    tiles: usize,
    min: IVec2,
    max: IVec2,
    /// Дома с дверями, у которых хоть одна дверь стоит в кармане.
    buildings: usize,
}

fn pockets(before: &Report, after: &Report, map: &MapData) -> Vec<Pocket> {
    let size = after.navmesh.grid_size;
    let cut = |x: i32, y: i32| {
        before.navmesh.is_passable(x, y)
            && after.filled.is_passable(x, y)
            && !after.navmesh.is_passable(x, y)
    };
    let mut seen = vec![false; (size.x * size.y) as usize];
    let mut door_tiles: std::collections::HashMap<IVec2, Vec<usize>> = Default::default();
    for (index, building) in map.buildings.iter().enumerate() {
        for &door in &building.entrances {
            let tile = world_to_tile(door);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    door_tiles
                        .entry(tile + IVec2::new(dx, dy))
                        .or_default()
                        .push(index);
                }
            }
        }
    }
    let mut result = Vec::new();
    for x in 0..size.x {
        for y in 0..size.y {
            let index = (x * size.y + y) as usize;
            if seen[index] || !cut(x, y) {
                continue;
            }
            seen[index] = true;
            let mut stack = vec![IVec2::new(x, y)];
            let mut pocket = Pocket {
                tiles: 0,
                min: IVec2::new(x, y),
                max: IVec2::new(x, y),
                buildings: 0,
            };
            let mut owners: Vec<usize> = Vec::new();
            while let Some(tile) = stack.pop() {
                pocket.tiles += 1;
                pocket.min = pocket.min.min(tile);
                pocket.max = pocket.max.max(tile);
                if let Some(found) = door_tiles.get(&tile) {
                    owners.extend(found);
                }
                for step in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                    let next = tile + step;
                    if next.x < 0 || next.y < 0 || next.x >= size.x || next.y >= size.y {
                        continue;
                    }
                    let next_index = (next.x * size.y + next.y) as usize;
                    if !seen[next_index] && cut(next.x, next.y) {
                        seen[next_index] = true;
                        stack.push(next);
                    }
                }
            }
            owners.sort_unstable();
            owners.dedup();
            pocket.buildings = owners.len();
            result.push(pocket);
        }
    }
    result.sort_by_key(|pocket| std::cmp::Reverse(pocket.tiles));
    result
}

fn main() {
    let cities: Vec<City> = std::env::args()
        .skip(1)
        .map(|name| {
            City::ALL
                .into_iter()
                .find(|city| city.slug() == name.to_lowercase())
                .unwrap_or_else(|| panic!("unknown city {name}"))
        })
        .collect();
    let cities = if cities.is_empty() {
        vec![City::Tula]
    } else {
        cities
    };

    for city in cities {
        let mut map = common::load_map(city);
        let started = Instant::now();
        let gaps = fence_gaps(&map.fences, &map.roads);
        let gap_time = started.elapsed();
        let gap_count: usize = gaps.iter().map(Vec::len).sum();
        let gapless = gaps.iter().filter(|gaps| gaps.is_empty()).count();
        let closed = map
            .fences
            .iter()
            .zip(&gaps)
            .filter(|(fence, gaps)| {
                gaps.is_empty()
                    && fence.points.len() > 2
                    && fence.points.first() == fence.points.last()
            })
            .count();

        let started = Instant::now();
        let after = measure(&mut map, city);
        let with_fences = started.elapsed();
        // `MapData` не `Clone`: карта без оград — та же карта с вынутым списком
        let fences = std::mem::take(&mut map.fences);
        let before = measure(&mut map, city);
        map.fences = fences;
        println!(
            "  default gates: {} opened in {:?}",
            after.gates, after.gate_time
        );
        // где посмотреть глазами: первые калитки по умолчанию и проёмы дорог
        let gates: Vec<String> = map
            .fences
            .iter()
            .flat_map(|fence| &fence.gates)
            .map(|at| format!("({:.0}, {:.0})", at.x, at.y))
            .collect();
        let road_gaps: Vec<String> = gaps
            .iter()
            .flatten()
            .take(6)
            .map(|gap| format!("({:.0}, {:.0})", gap.at.x, gap.at.y))
            .collect();
        println!("  gates at {}", gates.join(" "));
        println!("  road gaps at {}", road_gaps.join(" "));
        let with_doors = map
            .buildings
            .iter()
            .filter(|building| !building.entrances.is_empty())
            .count();
        let lost: Vec<usize> = after
            .doorless
            .iter()
            .filter(|index| !before.doorless.contains(index))
            .copied()
            .collect();

        println!(
            "{}: {} fences, {gap_count} gaps ({gapless} fences without one, {closed} of them \
             closed rings), gaps in {gap_time:?}",
            city.slug(),
            map.fences.len()
        );
        println!(
            "  pruned: {} without fences -> {} with ({:+}), fill+prune {:?}",
            before.pruned,
            after.pruned,
            after.pruned as i64 - before.pruned as i64,
            with_fences
        );
        println!(
            "  buildings with doors: {with_doors}; doorless {} -> {} ({} newly cut off)",
            before.doorless.len(),
            after.doorless.len(),
            lost.len()
        );
        let pockets = pockets(&before, &after, &map);
        let tile = after.navmesh.tile_size;
        println!(
            "  pockets cut off: {} ({} of them hold doors, {} tiles; {} under 25 tiles)",
            pockets.len(),
            pockets.iter().filter(|pocket| pocket.buildings > 0).count(),
            pockets
                .iter()
                .filter(|pocket| pocket.buildings > 0)
                .map(|pocket| pocket.tiles)
                .sum::<usize>(),
            pockets.iter().filter(|pocket| pocket.tiles < 25).count()
        );
        for pocket in pockets.iter().take(12) {
            println!(
                "    {} tiles, {} buildings with doors, ({:.0}, {:.0})..({:.0}, {:.0}) m",
                pocket.tiles,
                pocket.buildings,
                pocket.min.x as f32 * tile,
                pocket.min.y as f32 * tile,
                (pocket.max.x + 1) as f32 * tile,
                (pocket.max.y + 1) as f32 * tile
            );
        }
        for &index in lost.iter().take(15) {
            let building = &map.buildings[index];
            let door = building.entrances[0];
            println!(
                "    building #{index} ({:?}), door at ({:.0}, {:.0})",
                building.building_use, door.x, door.y
            );
        }
    }
}
