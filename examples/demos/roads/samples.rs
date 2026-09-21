//! Примеры пересечений города — манифест и сами вырезки OSM.
//!
//! Пример здесь — **не геометрия, выписанная руками**, а кусок настоящей
//! выгрузки Overpass: файл `data/<город>/NN_*.json` в том же формате, в каком
//! карту качает игра (`out geom`, все теги как есть), только вырезанный окном
//! вокруг одного узла (`tools/osm_crop`). Витрина читает его с диска, отдаёт
//! игровому `parse` и рисует игровыми `mesh_*` — так что на одном перекрёстке
//! виден весь путь «данные OSM → пиксели», и любую его стадию можно потрогать,
//! поправив файл и нажав `F5`.
//!
//! Манифест `data/<город>.json` — перечень примеров города: файл, вид
//! пересечения, что на нём смотреть, центр окна и его полуразмер. Центр хранится
//! **гео-координатами**, а не метрами карты: метры считаются от угла bbox и
//! уезжают вместе с `MAP_SIZE`, гео-точка — нет. Игровые координаты под
//! примером считаются из неё проекцией города, той же, что в разборе.
//!
//! Адрес тоже не выписан, а прочитан из самой вырезки: страна — с границы, на
//! которой OSM держит `driving_side`, улицы — `name` тех `highway`, что проходят
//! через центр окна. Данных о регионе и городе в выгрузке нет (запрос их не
//! просит), поэтому эти два слова — таблицей ниже.

use std::path::PathBuf;

use bevy::prelude::*;
use qwe::city::City;
use qwe::map::osm::overpass::GeoBounds;
use serde::Deserialize;

/// Дорога считается проходящей через узел, если её осевая ближе этого к центру
/// окна, м. Больше полуширины магистрали: у разделённого проспекта узлов два, и
/// центр окна стоит между половинами.
const ADDRESS_REACH: f32 = 14.0;

#[derive(Deserialize)]
struct Manifest {
    folder: String,
    samples: Vec<ManifestSample>,
}

#[derive(Deserialize)]
struct ManifestSample {
    file: String,
    title: String,
    note: String,
    /// `(широта, долгота)` центра окна.
    geo: [f64; 2],
    /// Полуразмер **видимого** окна, м. Вырезка шире (`tools/osm_crop
    /// --margin`): обрезанные концы дорог, половины теней и дома за краем
    /// остаются под маской.
    half: f32,
}

/// Один пример: вырезка OSM и всё, что про неё написано под окном.
pub(crate) struct Sample {
    pub(crate) title: String,
    pub(crate) note: String,
    pub(crate) address: String,
    /// Центр окна в игровых координатах — метрах карты города.
    pub(crate) at: Vec2,
    pub(crate) half: f32,
    /// Имя файла вырезки — в подписи, чтобы было что открыть.
    pub(crate) file: String,
    /// Сколько в вырезке нод, way и relation — первая стадия пути к пикселям.
    pub(crate) elements: [usize; 3],
    /// Ответ Overpass как есть: его и ест `parse`.
    pub(crate) osm: String,
}

/// Регион и город по-русски — то, чего в выгрузке нет.
fn place(city: City) -> (&'static str, &'static str) {
    match city {
        City::Tula => ("Тульская область", "Тула"),
        City::NewYork => ("штат Нью-Йорк", "Нью-Йорк"),
        City::Paris => ("Иль-де-Франс", "Париж"),
        City::Berlin => ("Берлин", "Берлин"),
        City::London => ("Англия", "Лондон"),
        City::Tokyo => ("Токио", "Токио"),
        City::DevilsLake => ("Северная Дакота", "Девилс-Лейк"),
    }
}

fn data_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/demos/roads/data"
    ))
}

/// Примеры города с диска. `Err` — причина словами: её показывает витрина.
pub(crate) fn load(city: City) -> Result<Vec<Sample>, String> {
    let manifest_path = data_dir().join(format!("{}.json", city.slug()));
    let text = std::fs::read_to_string(&manifest_path).map_err(|_| {
        format!(
            "для города {} примеры ещё не собраны: нет {}",
            place(city).1,
            manifest_path.display()
        )
    })?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let bounds = GeoBounds::for_city(city);
    manifest
        .samples
        .into_iter()
        .map(|sample| {
            let path = data_dir().join(&manifest.folder).join(&sample.file);
            let osm = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let at = bounds.project(sample.geo[0], sample.geo[1]);
            let (address, elements) = address(city, &osm, &bounds, at)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Ok(Sample {
                elements,
                title: sample.title,
                note: sample.note,
                address,
                at,
                half: sample.half,
                file: format!("{}/{}", manifest.folder, sample.file),
                osm,
            })
        })
        .collect()
}

/// Полный адрес узла: страна, регион, город и улицы, сошедшиеся в центре окна.
///
/// Читает сырой JSON, а не `MapData`: у `RoadLine` имени нет — карте оно ни к
/// чему, — а в вырезке оно лежит тегом на каждом way.
///
/// Вторым значением — счёт элементов вырезки по видам (node, way, relation):
/// JSON уже разобран, а второй раз ради трёх чисел его читать незачем.
fn address(
    city: City,
    osm: &str,
    bounds: &GeoBounds,
    at: Vec2,
) -> Result<(String, [usize; 3]), String> {
    let response: serde_json::Value =
        serde_json::from_str(osm).map_err(|error| error.to_string())?;
    let elements = response["elements"].as_array().ok_or("нет elements")?;
    let count = |kind: &str| {
        elements
            .iter()
            .filter(|element| element["type"] == kind)
            .count()
    };
    let counts = [count("node"), count("way"), count("relation")];

    let country = elements
        .iter()
        .filter(|element| element["type"] == "relation" && element["tags"]["admin_level"] == "2")
        .find_map(|element| element["tags"]["name"].as_str());

    // улицы — по близости осевой к центру, ближайшая первой
    let mut streets: Vec<(f32, &str)> = Vec::new();
    for element in elements {
        let tags = &element["tags"];
        let (Some(_), Some(name), Some(geometry)) = (
            tags["highway"].as_str(),
            tags["name"].as_str(),
            element["geometry"].as_array(),
        ) else {
            continue;
        };
        let points: Vec<Vec2> = geometry
            .iter()
            .filter_map(|point| {
                Some(bounds.project(point["lat"].as_f64()?, point["lon"].as_f64()?))
            })
            .collect();
        let distance = points
            .windows(2)
            .map(|link| distance_to_segment(at, link[0], link[1]))
            .fold(f32::INFINITY, f32::min);
        if distance > ADDRESS_REACH {
            continue;
        }
        match streets.iter_mut().find(|(_, known)| *known == name) {
            Some(street) => street.0 = street.0.min(distance),
            None => streets.push((distance, name)),
        }
    }
    streets.sort_by(|a, b| a.0.total_cmp(&b.0));

    let (region, town) = place(city);
    let mut parts: Vec<&str> = country.into_iter().collect();
    parts.extend([region, town]);
    let streets: Vec<&str> = streets.iter().map(|(_, name)| *name).collect();
    let crossing = if streets.is_empty() {
        "улица без имени".to_string()
    } else {
        streets.join(" × ")
    };
    Ok((format!("{}, {crossing}", parts.join(", ")), counts))
}

fn distance_to_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    let link = to - from;
    let t = if link.length_squared() > 0.0 {
        ((point - from).dot(link) / link.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(from + link * t)
}
