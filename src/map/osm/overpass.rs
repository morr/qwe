//! Overpass API: границы выгрузки, проекция, QL-запрос и DTO ответа
//! (формат `[out:json]` + `out geom` — геометрия инлайн в way/relation).

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::math::{DVec2, Vec2};
use serde::Deserialize;

use crate::city::City;
use crate::settings::{MAP_SIZE, METERS_PER_DEG_LAT};

/// Гео-границы карты и локальная равнопромежуточная проекция:
/// метры от юго-западного угла bbox, y — на север.
pub struct GeoBounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
    /// Метров в градусе долготы на широте центра.
    lon_scale: f64,
}

impl GeoBounds {
    pub fn for_city(city: City) -> Self {
        let center = city.geo_center();
        let lon_scale = METERS_PER_DEG_LAT * center.x.to_radians().cos();
        let half = DVec2::new(
            MAP_SIZE.x as f64 / 2.0 / lon_scale,
            MAP_SIZE.y as f64 / 2.0 / METERS_PER_DEG_LAT,
        );
        Self {
            south: center.x - half.y,
            west: center.y - half.x,
            north: center.x + half.y,
            east: center.y + half.x,
            lon_scale,
        }
    }

    pub fn project(&self, lat: f64, lon: f64) -> Vec2 {
        Vec2::new(
            ((lon - self.west) * self.lon_scale) as f32,
            ((lat - self.south) * METERS_PER_DEG_LAT) as f32,
        )
    }
}

/// Версия запроса в имени кеша: расширили набор тегов — старая выгрузка
/// новых зон не содержит, и без этого её никто бы не перекачал.
const QUERY_VERSION: u32 = 2;

/// QL-запрос: здания, дороги, вода, парки/зелень, луга, песок, стены Кремля.
pub fn overpass_query(city: City) -> String {
    let GeoBounds {
        south,
        west,
        north,
        east,
        ..
    } = GeoBounds::for_city(city);
    let bbox = format!("{south},{west},{north},{east}");
    format!(
        r#"[out:json][timeout:120];
(
  way["building"]({bbox});
  relation["building"]({bbox});
  way["highway"]({bbox});
  way["natural"="water"]({bbox});
  relation["natural"="water"]({bbox});
  way["waterway"="riverbank"]({bbox});
  way["leisure"~"^(park|garden)$"]({bbox});
  relation["leisure"~"^(park|garden)$"]({bbox});
  way["landuse"~"^(recreation_ground|forest)$"]({bbox});
  relation["landuse"~"^(recreation_ground|forest)$"]({bbox});
  way["natural"="wood"]({bbox});
  way["landuse"~"^(grass|meadow)$"]({bbox});
  relation["landuse"~"^(grass|meadow)$"]({bbox});
  way["natural"~"^(grassland|meadow)$"]({bbox});
  way["natural"~"^(sand|beach)$"]({bbox});
  relation["natural"~"^(sand|beach)$"]({bbox});
  way["barrier"="city_wall"]({bbox});
);
out geom;
"#
    )
}

/// Файл кеша выгрузки: параметры в имени — смена настроек инвалидирует кеш,
/// а у каждого города свой файл, так что переключение туда-обратно не качает
/// заново.
pub fn cache_path(city: City) -> PathBuf {
    let center = city.geo_center();
    PathBuf::from(format!(
        "assets/osm/{}_{}_{}_{}x{}_v{QUERY_VERSION}.json",
        city.slug(),
        center.x,
        center.y,
        MAP_SIZE.x,
        MAP_SIZE.y
    ))
}

#[derive(Deserialize)]
pub struct OverpassResponse {
    pub elements: Vec<Element>,
}

#[derive(Deserialize)]
pub struct Element {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    /// Геометрия way (с `out geom`).
    #[serde(default)]
    pub geometry: Option<Vec<LatLon>>,
    /// Члены relation.
    #[serde(default)]
    pub members: Option<Vec<Member>>,
}

#[derive(Deserialize, Clone, Copy)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize)]
pub struct Member {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub geometry: Option<Vec<LatLon>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_projects_to_map_center() {
        for city in City::ALL {
            let bounds = GeoBounds::for_city(city);
            let geo = city.geo_center();
            let center = bounds.project(geo.x, geo.y);
            assert!(
                (center - MAP_SIZE / 2.0).length() < 1.0,
                "{city:?} {center:?}"
            );
        }
    }

    #[test]
    fn corners_project_to_map_corners() {
        for city in City::ALL {
            let bounds = GeoBounds::for_city(city);
            let sw = bounds.project(bounds.south, bounds.west);
            let ne = bounds.project(bounds.north, bounds.east);
            assert!(sw.length() < 1.0, "{city:?} {sw:?}");
            assert!((ne - MAP_SIZE).length() < 1.0, "{city:?} {ne:?}");
        }
    }
}
