//! Фоновая подготовка мира: кеш-файл → сеть (с прогрессом) → парсинг →
//! растеризация navmesh → прунинг. Работает на выделенном потоке; прогресс —
//! через `Arc<Mutex<JobState>>`. Всё, что не требует ECS, живёт здесь, а не в
//! `OnEnter(Playing)`: там кадр не рисуется, и экран загрузки замирает.

use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};

use bevy::prelude::*;

use crate::city::City;
use crate::grid::world_to_tile;
use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{cache_path, overpass_query};
use crate::map::osm::parse::parse;
use crate::navigation::{Navmesh, snap_portal_position};

/// Зеркала Overpass по порядку обхода. Основной инстанс на плотных городах
/// (Нью-Йорк, Лондон) регулярно отвечает 504 «server too busy» — или, того
/// хуже, HTML-страницей с runtime error под кодом 200; тогда идём к
/// следующему. Все три отдают один и тот же API и данные.
const OVERPASS_URLS: [&str; 3] = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];
const CHUNK_SIZE: usize = 64 * 1024;

/// Готовый к спавну мир: разобранная карта и позиция портала (снап нужен
/// уже заполненному navmesh, а прунинг — уже снапнутому порталу).
pub struct LoadedWorld {
    pub map: MapData,
    pub portal: Vec2,
}

pub enum JobState {
    Connecting,
    Downloading {
        bytes: u64,
        total: Option<u64>,
    },
    Parsing,
    /// Растеризация карты в navmesh.
    BuildingNavmesh,
    /// Отсечение карманов, недостижимых от портала.
    Pruning,
    /// `Option`, чтобы poll-система могла забрать данные через `take()`.
    Done(Option<Box<LoadedWorld>>),
    Failed(String),
}

#[derive(Resource, Clone)]
pub struct MapLoadJob(pub Arc<Mutex<JobState>>);

impl Default for MapLoadJob {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(JobState::Connecting)))
    }
}

impl MapLoadJob {
    fn set(&self, state: JobState) {
        *self.0.lock().unwrap() = state;
    }
}

/// Выделенный поток, а не пул задач: многосекундное блокирующее чтение
/// сети не должно занимать воркер `AsyncComputeTaskPool`.
pub fn start_load_thread(job: MapLoadJob, navmesh: Arc<RwLock<Navmesh>>, city: City) {
    std::thread::spawn(move || {
        let result = run(&job, city).map(|map| build_navmesh(&job, map, &navmesh, city));
        job.set(match result {
            Ok(world) => JobState::Done(Some(Box::new(world))),
            Err(message) => JobState::Failed(message),
        });
    });
}

/// Растеризация карты и прунинг — по шагу на состояние, чтобы экран
/// загрузки показывал, чем поток занят.
fn build_navmesh(
    job: &MapLoadJob,
    map: MapData,
    arc_navmesh: &RwLock<Navmesh>,
    city: City,
) -> LoadedWorld {
    job.set(JobState::BuildingNavmesh);
    let started = std::time::Instant::now();
    let mut navmesh = arc_navmesh.write().unwrap();
    navmesh.fill_from_mapdata(&map);
    info!("navmesh filled in {:?}", started.elapsed());

    let hint = city.portal_hint();
    let portal = match snap_portal_position(&navmesh, hint) {
        Some(position) => {
            if position != hint {
                info!("portal snapped {hint:?} => {position:?}");
            }
            position
        }
        None => {
            warn!("no clear spot for portal near {hint:?}");
            hint
        }
    };

    job.set(JobState::Pruning);
    let started = std::time::Instant::now();
    let pruned = navmesh.prune_unreachable(world_to_tile(portal));
    info!(
        "navmesh: pruned {pruned} unreachable tiles in {:?}",
        started.elapsed()
    );

    LoadedWorld { map, portal }
}

fn run(job: &MapLoadJob, city: City) -> Result<MapData, String> {
    let path = cache_path(city);

    if path.exists() {
        info!("osm: cache hit at {}", path.display());
        let json =
            std::fs::read_to_string(&path).map_err(|error| format!("cache read: {error}"))?;
        job.set(JobState::Parsing);
        match parse(&json, city) {
            Ok(map) => return Ok(map),
            Err(error) => {
                // битый кеш самоизлечивается: удаляем и качаем заново
                warn!("osm: broken cache ({error}), re-downloading");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let json = download(job, city)?;
    job.set(JobState::Parsing);
    let map = parse(&json, city)?;

    // кеш пишется только после успешного парсинга
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &json) {
        Ok(()) => info!("osm: cached {} bytes at {}", json.len(), path.display()),
        Err(error) => warn!("osm: cache write failed: {error}"),
    }
    Ok(map)
}

/// Обход зеркал: первое, ответившее JSON'ом, выигрывает; иначе — ошибка
/// последнего.
fn download(job: &MapLoadJob, city: City) -> Result<String, String> {
    let query = overpass_query(city);
    let mut last_error = String::from("no overpass endpoints configured");
    for url in OVERPASS_URLS {
        match download_from(job, url, &query) {
            Ok(json) => return Ok(json),
            Err(error) => {
                warn!("osm: {url} failed ({error})");
                last_error = error;
            }
        }
    }
    Err(last_error)
}

fn download_from(job: &MapLoadJob, url: &str, query: &str) -> Result<String, String> {
    job.set(JobState::Connecting);
    info!("osm: downloading {url}");

    let mut response = ureq::post(url)
        .send(query)
        .map_err(|error| format!("overpass request: {error}"))?;

    // с gzip длина обычно неизвестна (chunked) — тогда прогресс в байтах
    let total = response.body().content_length();
    let mut reader = response.body_mut().as_reader();
    let mut data = Vec::new();
    let mut chunk = vec![0u8; CHUNK_SIZE];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("overpass read: {error}"))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
        job.set(JobState::Downloading {
            bytes: data.len() as u64,
            total,
        });
    }

    let json = String::from_utf8(data).map_err(|error| format!("overpass utf8: {error}"))?;
    // перегруженный инстанс отдаёт HTML-страницу с runtime error и статусом
    // 200 — для нас это отказ, а не карта
    if !json.trim_start().starts_with('{') {
        return Err("overpass returned a non-json body (server busy?)".to_string());
    }
    Ok(json)
}
