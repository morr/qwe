//! Фоновая подготовка мира: кеш-файл → сеть (с прогрессом) → парсинг →
//! растеризация navmesh → прунинг. Работает на выделенном потоке; прогресс —
//! через `Arc<Mutex<JobState>>`. Всё, что не требует ECS, живёт здесь, а не в
//! `OnEnter(Playing)`: там кадр не рисуется, и экран загрузки замирает.

use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::city::City;
use crate::map::fences::entry_step_hidden;
use crate::map::inspect::InspectIndex;
use crate::map::osm::model::MapData;
use crate::map::osm::overpass::{cache_path, overpass_query, prune_stale_caches};
use crate::map::osm::parse::{ParseKnobs, parse};
use crate::map::osm::snapshot;
use crate::map::prepare::{self, StyleSnapshot, WorldMeshes};
use crate::navigation::Navmesh;

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

/// Готовый к спавну мир: разобранная карта, позиция портала (снап нужен
/// уже заполненному navmesh, а прунинг — уже снапнутому порталу), слои
/// карты, собранные тем же потоком (`map/prepare.rs`) — спавн на входе в
/// мир кладёт их в мир без пересборки — и база inspect-индекса, собранная
/// тем же потоком параллельно лейнам (`finish_world`); куски мира догоняет
/// `rebuild_index` первым кадром.
pub struct LoadedWorld {
    pub map: MapData,
    pub portal: Vec2,
    pub meshes: WorldMeshes,
    pub inspect: InspectIndex,
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
    /// Сборка слоёв карты: здания, дороги, поверхности, деревья, рельсы и
    /// полимеш — параллельными группами в самом потоке (`map/prepare.rs`).
    /// Одно состояние, а не по группе: группы идут разом, и «чем занят
    /// поток» отвечает не одна строка.
    BuildingMeshes,
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
    /// Толерантно к отравлению: паника треда при зажатом локе не должна
    /// превращать следующий `set` (или чтение `poll_job`) во вторую панику —
    /// статус джобы это перезаписываемое перечисление, чужой мусор в нём
    /// не переживёт и мгновения.
    pub(crate) fn set(&self, state: JobState) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
    }
}

/// Выделенный поток, а не пул задач: многосекундное блокирующее чтение
/// сети не должно занимать воркер `AsyncComputeTaskPool`. Входы разбора
/// (`knobs`) и снимок стилей (`styles`) приходят аргументом: ECS в потоке
/// нет, а глобалей разбор и сборка слоёв не читают — ширину полосы поток
/// ставит сам из снимка карты, солнце сеется на первом кадре задолго до
/// конца парса.
pub fn start_load_thread(
    job: MapLoadJob,
    navmesh: Arc<RwLock<Navmesh>>,
    city: City,
    knobs: ParseKnobs,
    styles: StyleSnapshot,
) {
    std::thread::spawn(move || {
        // Паника где угодно на треде — Failed, а не замерший экран. Лейны
        // мешинга покрывает `prepare::join`, но всё до и после них — сеть,
        // снапшот, парс, серийная заливка с калитками и прунингом — летит
        // здесь, и без поимки `JobState` застревает на последней фазе, а
        // экран загрузки молчит навсегда. `AssertUnwindSafe`: захваты треда
        // не обязаны быть unwind-safe, тред всё равно не переживает панику.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&job, city, knobs).and_then(|map| {
                if entry_step_hidden(styles.fence_zoom) {
                    // скрытая ступень заборов на входе — перекрытая загрузка:
                    // серийный префикс (заливка, калитки, прунинг) прячется под
                    // дорожки мешинга
                    prepare::build_overlapped_world(&job, map, &navmesh, city, &styles)
                } else {
                    let world = build_navmesh(&job, map, &navmesh, city);
                    finish_world(world, &styles, &job)
                }
            })
        }))
        .unwrap_or_else(|panic| Err(load_panic_message(panic.as_ref())));
        job.set(match result {
            Ok(world) => JobState::Done(Some(Box::new(world))),
            Err(message) => JobState::Failed(message),
        });
    });
}

/// Текст паники для `Failed`. Payload бывает `&str`, `String` и «никто не
/// положил сообщение» — все три сводятся к одной строке на экране загрузки.
fn load_panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    let payload = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no panic message".to_string());
    format!("load thread panicked: {payload}")
}

/// Слои карты — в тот же поток: карта после гейтов и прунинга финальна, и
/// параллельные группы собирают всё, что раньше стояло в `OnEnter(Playing)`
/// и замироживало кадр перехода на 15.7 с (запись 10).
///
/// База inspect-индекса — тоже в этом потоке, параллельно лейнам: четыре
/// её прохода с карманами читают только карту и форму (обе готовы), а
/// дорожный финал ступени входа — сама стена whole set, под ними есть
/// полсекунды. Куски дорог появляются со спавном мира, их догоняет
/// `rebuild_index` первым кадром (`map/inspect.rs`). Состав деревьев
/// поднимается сюда же, до клона: индексный поток мешит уже собранную
/// карту, а проверка внутри `build_world_meshes` увидит состав готовым
/// и не повторит.
fn finish_world(
    mut world: LoadedWorld,
    styles: &StyleSnapshot,
    job: &MapLoadJob,
) -> Result<LoadedWorld, String> {
    let compose = crate::map::trees::tree_compose(&styles.trees, &styles.tree_rows);
    if world.map.composed_for != Some(compose) {
        world.map.compose_trees(compose);
    }
    // та же глобаль, что `prepare.rs` ставит лейнам: карманные контуры
    // индекса отвечают той же узловой осью, что рисунок
    crate::map::roads::shape::set_lane_width(world.map.knobs.lane_width);

    let started = std::time::Instant::now();
    let index_map = world.map.clone();
    info!(
        "map meshes: inspect index clone in {:.1?}",
        started.elapsed()
    );
    let inspect = std::thread::scope(|scope| -> Result<InspectIndex, String> {
        let base = scope.spawn(|| {
            let started = std::time::Instant::now();
            let index = InspectIndex::prebuild(&index_map, &styles.road_shape);
            info!(
                "map meshes: inspect index base in {:.1?}",
                started.elapsed()
            );
            index
        });
        world.meshes = prepare::build_world_meshes(&mut world.map, styles, job)?;
        base.join()
            .map_err(|_| "inspect index panicked".to_string())
    })?;
    world.inspect = inspect;
    Ok(world)
}

/// Растеризация карты и прунинг — по шагу на состояние, чтобы экран
/// загрузки показывал, чем поток занят. Серийный путь видимой ступени
/// заборов входного зума: лейн заборов там дорог сам по себе (2.3–2.8 с
/// Парижа), и его старт не двигается гонкой дорожек за ядра (урок прогона
/// №12). Перекрытый путь скрытой ступени — [`prepare::build_overlapped_world`].
fn build_navmesh(
    job: &MapLoadJob,
    mut map: MapData,
    arc_navmesh: &RwLock<Navmesh>,
    city: City,
) -> LoadedWorld {
    job.set(JobState::BuildingNavmesh);
    // ожидание write-лока меряется отдельно от растеризации: пока они шли
    // одной цифрой, чужой read-лок на десяток секунд читался как «медленная
    // заливка карты»
    let lock_since = std::time::Instant::now();
    let mut navmesh = arc_navmesh
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let portal =
        prepare::fill_navmesh_and_snap_portal(&mut navmesh, &map, city, lock_since.elapsed());

    // калитки — до прунинга и со снапнутым порталом: ограда, отрезавшая
    // участок с дверями, открывается раньше, чем его выбросят. Пишутся в
    // `map`, чтобы полигональный меш позже прочитал те же калитки
    let done = navmesh.open_gates_and_prune(&mut map, portal, |gates, took| {
        info!("navmesh: opened {gates} fence gates in {took:?}");
        job.set(JobState::Pruning);
    });
    info!(
        "navmesh: pruned {} unreachable tiles in {:?}",
        done.pruned, done.prune_time
    );

    LoadedWorld {
        map,
        portal,
        meshes: WorldMeshes::default(),
        inspect: InspectIndex::default(),
    }
}

fn run(job: &MapLoadJob, city: City, knobs: ParseKnobs) -> Result<MapData, String> {
    let path = cache_path(city);
    // до чтения, а не после записи: устаревшие файлы надо подмести и на
    // попадании в кеш, иначе они переживут все следующие запуски
    prune_stale_caches(knobs);

    // тёплый путь: бинарный снапшот разобранной карты (`snapshot.rs`) —
    // разбор json-кеша целиком заменяется чтением его готовой формы
    job.set(JobState::Parsing);
    if let Some(map) = snapshot::read(city, knobs) {
        return Ok(map);
    }

    if let Some(json) = read_cache(city)? {
        match parse_logged(&json, city, knobs) {
            Ok(map) => {
                snapshot::write(city, knobs, &map);
                return Ok(map);
            }
            Err(error) => {
                // битый кеш самоизлечивается: удаляем и качаем заново
                warn!("osm: broken cache ({error}), re-downloading");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    download_and_cache(job, city, knobs).map(|(_, map)| map)
}

/// Выгрузка города как её видит игра — JSON ответа Overpass: из кеша, а нет
/// кеша — с зеркал, и тогда кеш пишется тем же путём, что при загрузке мира.
///
/// Дверь витрины дорог (`examples/demos/roads`): она режет свои окна из того
/// же файла, что читает игра, поэтому срез не может разойтись с игрой после
/// подъёма `QUERY_VERSION`. Кеш она не разбирает — только читает: битый файл
/// вернётся ошибкой десериализации у неё, а самоизлечение остаётся за игрой.
pub fn city_extract(city: City) -> Result<String, String> {
    if let Some(json) = read_cache(city)? {
        return Ok(json);
    }
    // разбор здесь — только проверка, что зеркало отдало годный ответ: карта
    // выбрасывается, так что входы разбора — любые
    download_and_cache(&MapLoadJob::default(), city, ParseKnobs::default()).map(|(json, _)| json)
}

/// Разбор выгрузки с одной цифрой в логе: записи проходов (`osm parse: …`)
/// суммарное время не называют, а оно — то, что снапшот экономит.
fn parse_logged(json: &str, city: City, knobs: ParseKnobs) -> Result<MapData, String> {
    let started = std::time::Instant::now();
    parse(json, city, knobs).inspect(|_| {
        info!(
            "osm: parsed {} MB of json in {:?}",
            json.len() / 1_048_576,
            started.elapsed()
        );
    })
}

fn read_cache(city: City) -> Result<Option<String>, String> {
    let path = cache_path(city);
    if !path.exists() {
        return Ok(None);
    }
    info!("osm: cache hit at {}", path.display());
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("cache read: {error}"))
}

/// Скачать, разобрать и только после успешного разбора записать кеш.
fn download_and_cache(
    job: &MapLoadJob,
    city: City,
    knobs: ParseKnobs,
) -> Result<(String, MapData), String> {
    let path = cache_path(city);
    let json = download(job, city)?;
    job.set(JobState::Parsing);
    let map = parse_logged(&json, city, knobs)?;

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &json) {
        Ok(()) => info!("osm: cached {} bytes at {}", json.len(), path.display()),
        Err(error) => warn!("osm: cache write failed: {error}"),
    }
    // снапшот — после записи кеша: штамп снимается с готового файла
    snapshot::write(city, knobs, &map);
    Ok((json, map))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_message_names_str_string_and_silent_payloads() {
        let text = std::panic::catch_unwind(|| panic!("boom")).unwrap_err();
        assert_eq!(
            load_panic_message(text.as_ref()),
            "load thread panicked: boom"
        );

        let owned = std::panic::catch_unwind(|| std::panic::panic_any(String::from("owned boom")))
            .unwrap_err();
        assert_eq!(
            load_panic_message(owned.as_ref()),
            "load thread panicked: owned boom"
        );

        let silent = std::panic::catch_unwind(|| std::panic::panic_any(7u32)).unwrap_err();
        assert_eq!(
            load_panic_message(silent.as_ref()),
            "load thread panicked: no panic message"
        );
    }

    /// Паника с зажатым локом травит мьютекс джобы навсегда; `set` после
    /// этого обязан перезаписать статус, а не паниковаться сам — иначе
    /// Retry мёртв, как и до толерантных локов.
    #[test]
    fn job_set_survives_a_poisoned_mutex() {
        let job = MapLoadJob::default();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = job.0.lock().unwrap();
            panic!("poisoning");
        }))
        .is_err();
        assert!(poisoned);

        job.set(JobState::Parsing);
        let state = job
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(*state, JobState::Parsing));
    }
}
