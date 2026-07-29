//! Фоновая подготовка мира: кеш-файл → сеть (с прогрессом) → парсинг →
//! растеризация navmesh → прунинг. Работает на выделенном потоке; прогресс —
//! через `Arc<Mutex<JobState>>`. Всё, что не требует ECS, живёт здесь, а не в
//! `OnEnter(Playing)`: там кадр не рисуется, и экран загрузки замирает.

use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};

use bevy::prelude::*;

use crate::grid::world_to_tile;
use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{cache_path, overpass_query};
use crate::map::osm::parse::parse;
use crate::navigation::{Navmesh, snap_portal_position};
use crate::settings::PORTAL_POS;

const OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
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
pub fn start_load_thread(job: MapLoadJob, navmesh: Arc<RwLock<Navmesh>>) {
    std::thread::spawn(move || {
        let result = run(&job).map(|map| build_navmesh(&job, map, &navmesh));
        job.set(match result {
            Ok(world) => JobState::Done(Some(Box::new(world))),
            Err(message) => JobState::Failed(message),
        });
    });
}

/// Растеризация карты и прунинг — по шагу на состояние, чтобы экран
/// загрузки показывал, чем поток занят.
fn build_navmesh(job: &MapLoadJob, map: MapData, arc_navmesh: &RwLock<Navmesh>) -> LoadedWorld {
    job.set(JobState::BuildingNavmesh);
    let started = std::time::Instant::now();
    let mut navmesh = arc_navmesh.write().unwrap();
    navmesh.fill_from_mapdata(&map);
    info!("navmesh filled in {:?}", started.elapsed());

    let portal = match snap_portal_position(&navmesh, PORTAL_POS) {
        Some(position) => {
            if position != PORTAL_POS {
                info!("portal snapped {PORTAL_POS:?} => {position:?}");
            }
            position
        }
        None => {
            warn!("no clear spot for portal near {PORTAL_POS:?}");
            PORTAL_POS
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

fn run(job: &MapLoadJob) -> Result<MapData, String> {
    let path = cache_path();

    if path.exists() {
        info!("osm: cache hit at {}", path.display());
        let json =
            std::fs::read_to_string(&path).map_err(|error| format!("cache read: {error}"))?;
        job.set(JobState::Parsing);
        match parse(&json) {
            Ok(map) => return Ok(map),
            Err(error) => {
                // битый кеш самоизлечивается: удаляем и качаем заново
                warn!("osm: broken cache ({error}), re-downloading");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let json = download(job)?;
    job.set(JobState::Parsing);
    let map = parse(&json)?;

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

fn download(job: &MapLoadJob) -> Result<String, String> {
    job.set(JobState::Connecting);
    info!("osm: downloading {OVERPASS_URL}");

    let mut response = ureq::post(OVERPASS_URL)
        .send(overpass_query().as_str())
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

    String::from_utf8(data).map_err(|error| format!("overpass utf8: {error}"))
}
