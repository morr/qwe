//! Фоновая загрузка выгрузки Overpass: кеш-файл → сеть (с прогрессом).
//! Работает на выделенном потоке; прогресс — через `Arc<Mutex<JobState>>`.

use std::io::Read;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{cache_path, overpass_query};
use crate::map::osm::parse::parse;

const OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
const CHUNK_SIZE: usize = 64 * 1024;

pub enum JobState {
    Connecting,
    Downloading {
        bytes: u64,
        total: Option<u64>,
    },
    Parsing,
    /// `Option`, чтобы poll-система могла забрать данные через `take()`.
    Done(Option<Box<MapData>>),
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
pub fn start_load_thread(job: MapLoadJob) {
    std::thread::spawn(move || {
        let result = run(&job);
        job.set(match result {
            Ok(map) => JobState::Done(Some(Box::new(map))),
            Err(message) => JobState::Failed(message),
        });
    });
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
