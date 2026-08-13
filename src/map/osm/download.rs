//! Фоновая подготовка мира: кеш-файл → сеть (с прогрессом) → парсинг →
//! растеризация navmesh → прунинг. Работает на выделенном потоке; прогресс —
//! через `Arc<Mutex<JobState>>`. Всё, что не требует ECS, живёт здесь, а не в
//! `OnEnter(Playing)`: там кадр не рисуется, и экран загрузки замирает.

use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::city::City;
use crate::grid::world_to_tile;
use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{cache_path, overpass_query, prune_stale_caches};
use crate::map::osm::parse::parse;
use crate::navigation::{Navmesh, snap_portal_position};

/// Зеркала Overpass по порядку обхода. Основной инстанс на плотных городах
/// (Нью-Йорк, Лондон) регулярно отвечает 504 «server too busy» — или, того
/// хуже, HTML-страницей с runtime error под кодом 200; тогда идём к
/// следующему. Все четыре отдают один и тот же API и данные.
///
/// Первое — российское зеркало VK/Mail.ru: полная планета, свежий срез, и
/// ближайший канал отсюда, пока европейские инстансы залипают в 504.
pub const OVERPASS_URLS: [&str; 4] = [
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];
/// Сколько зеркал в обходе — для подписи «mirror 2/3» на экране загрузки.
pub const OVERPASS_MIRRORS: usize = OVERPASS_URLS.len();
const CHUNK_SIZE: usize = 64 * 1024;
/// Окно замера скорости: и шаг сглаживания, и частота обновления цифры на
/// экране загрузки. По чанку в 64 КБ мерить бессмысленно — цифра прыгает.
const SPEED_WINDOW: Duration = Duration::from_millis(250);

/// Готовый к спавну мир: разобранная карта и позиция портала (снап нужен
/// уже заполненному navmesh, а прунинг — уже снапнутому порталу).
pub struct LoadedWorld {
    pub map: MapData,
    pub portal: Vec2,
}

pub enum JobState {
    /// Ждём первый байт ответа: TCP/TLS плюс — почти всё время — счёт запроса
    /// на стороне Overpass. На плотном городе это минуты, поэтому экран
    /// загрузки тикает секундами (`poll_job`): поток здесь заблокирован внутри
    /// `send()` и сам о себе сообщить не может. `attempt` — номер зеркала,
    /// 1-based; по его смене UI перезапускает счётчик.
    Connecting {
        attempt: usize,
    },
    Downloading {
        bytes: u64,
        total: Option<u64>,
        /// Сглаженная скорость, байт/с. 0 — первое окно ещё не набралось.
        bytes_per_sec: f64,
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
        Self(Arc::new(Mutex::new(JobState::Connecting { attempt: 1 })))
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
    // ожидание write-лока меряется отдельно от растеризации: пока они шли
    // одной цифрой, чужой read-лок на десяток секунд читался как «медленная
    // заливка карты»
    let lock_since = std::time::Instant::now();
    let mut navmesh = arc_navmesh.write().unwrap();
    let waited = lock_since.elapsed();
    let started = std::time::Instant::now();
    navmesh.fill_from_mapdata(&map);
    info!(
        "navmesh filled in {:?} (write lock waited {:?})",
        started.elapsed(),
        waited
    );

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
    // до чтения, а не после записи: устаревшие файлы надо подмести и на
    // попадании в кеш, иначе они переживут все следующие запуски
    prune_stale_caches();

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
    for (index, url) in OVERPASS_URLS.iter().enumerate() {
        match download_from(job, url, &query, index + 1) {
            Ok(json) => return Ok(json),
            Err(error) => {
                warn!("osm: {url} failed ({error})");
                last_error = error;
            }
        }
    }
    Err(last_error)
}

fn download_from(
    job: &MapLoadJob,
    url: &str,
    query: &str,
    attempt: usize,
) -> Result<String, String> {
    job.set(JobState::Connecting { attempt });
    info!("osm: downloading {url}");

    let mut response = ureq::post(url)
        .send(query)
        .map_err(|error| format!("overpass request: {error}"))?;

    // с gzip длина обычно неизвестна (chunked) — тогда прогресс в байтах
    let total = response.body().content_length();
    let mut reader = response.body_mut().as_reader();
    let mut data = Vec::new();
    let mut chunk = vec![0u8; CHUNK_SIZE];
    // локальны для попытки: смена зеркала начинает замер с нуля, а не тащит
    // за собой скорость отвалившегося
    let mut window_started = Instant::now();
    let mut window_bytes = 0u64;
    let mut speed = 0.0f64;
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("overpass read: {error}"))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);

        window_bytes += read as u64;
        let elapsed = window_started.elapsed();
        if elapsed >= SPEED_WINDOW {
            let instant = window_bytes as f64 / elapsed.as_secs_f64();
            speed = if speed == 0.0 {
                instant
            } else {
                speed * 0.7 + instant * 0.3
            };
            window_started = Instant::now();
            window_bytes = 0;
        }

        job.set(JobState::Downloading {
            bytes: data.len() as u64,
            total,
            bytes_per_sec: speed,
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
