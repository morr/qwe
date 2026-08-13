//! Обвязка офлайн-инструментов: то, чем начинается каждый бенч и каждый аудит.
//!
//! Не библиотека и не фикстура — чтение кеша с диска и паника с советом
//! запустить приложение. `qwe::map::osm::fixture` живёт в крейте, потому что им
//! пользуются и юнит-, и интеграционные тесты; это нужно только примерам,
//! поэтому лежит рядом с ними и подключается строкой
//! `#[path = "../common/mod.rs"] mod common;`.
//!
//! Копий [`load_map`] было семь, байт в байт, и ещё две почти таких же.

// Модуль включается в КАЖДЫЙ пример своей копией, а нужны им разные половины:
// аудиту швов navmesh не нужен вовсе. Без этого каждый пример ругался бы на
// то, чем не пользуется.
#![allow(dead_code)]

use qwe::city::City;
use qwe::grid::world_to_tile;
use qwe::map::osm::{MapData, overpass, parse};
use qwe::navigation::{Navmesh, snap_portal_position};

/// Карта — только из кеша: инструменты работают офлайн, выгрузку делает игра.
pub fn load_map(city: City) -> MapData {
    let path = overpass::cache_path(city);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no OSM cache at {}: {error}. run the app once to download it",
            path.display()
        )
    });
    parse::parse(&json, city).expect("failed to parse cached OSM json")
}

/// Сеточный navmesh — ровно как в игре, вместе с отсечением недостижимого от
/// портала: цели инструмента обязаны выбираться по той же проходимости, иначе
/// он меряет не ту карту.
pub fn build_navmesh(map: &MapData, city: City) -> Navmesh {
    let mut navmesh = Navmesh::default();
    navmesh.fill_from_mapdata(map);
    let portal =
        snap_portal_position(&navmesh, city.portal_hint()).expect("no clear spot for portal");
    navmesh.prune_unreachable(world_to_tile(portal));
    navmesh
}
