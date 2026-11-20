//! Фоновая подготовка мира: кеш-файл → сеть (с прогрессом) → парсинг →
//! растеризация navmesh → прунинг. Работает на выделенном потоке; прогресс —
//! через `Arc<Mutex<JobState>>`. Всё, что не требует ECS, живёт здесь, а не в
//! `OnEnter(Playing)`: там кадр не рисуется, и экран загрузки замирает.

use std::io::Read;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::city::City;
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
/// тем же потоком в первой волне лейнов (`build_overlapped_world`); куски
/// мира догоняет `rebuild_index` первым кадром.
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
        // мешинга покрывает `prepare::join`, но всё вокруг них — сеть,
        // снапшот, парс, заливка с калитками и прунингом — летит здесь, и
        // без поимки `JobState` застревает на последней фазе, а экран
        // загрузки молчит навсегда. `AssertUnwindSafe`: захваты треда
        // не обязаны быть unwind-safe, тред всё равно не переживает панику.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&job, city, knobs)
                .and_then(|map| prepare::build_overlapped_world(&job, map, &navmesh, city, &styles))
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

    // Тёплый путь json-кеша с самоизлечением: нечитаемый файл (усечённая
    // запись рвёт utf-8 посреди названия улицы, сбой прав) и ответ без
    // карты (валидный json таймаута или лимита зеркала) лечатся так же,
    // как битый json — файл удаляется, ответ качается заново. Без этого
    // Retry вечно жевал бы тот же мусор: провал чтения уезжал в `Failed`
    // мимо самоизлечения, а ответ без карты вообще не ловился.
    match read_cache(city) {
        Ok(Some(json)) => {
            let cached = parse_logged(&json, city, knobs)
                .and_then(|map| validate_answer(&map).map(|()| map));
            match cached {
                Ok(map) => {
                    snapshot::write(city, knobs, &map);
                    return Ok(map);
                }
                Err(error) => {
                    warn!("osm: unusable cache ({error}), re-downloading");
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            warn!("osm: unusable cache ({error}), re-downloading");
            let _ = std::fs::remove_file(&path);
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

/// Ответ обязан быть картой: настоящая выгрузка несёт улицы. Зеркало под
/// лимитом или таймаутом отвечает валидным json без карты (пустой
/// `elements`, служебный remark) — гейт «начинается с `{`» такой ответ
/// пропускает, разбор проходит, и без проверки пустой город закешировался
/// бы и заснапшотился навсегда: самоизлечение срабатывает только на
/// провале разбора.
fn validate_answer(map: &MapData) -> Result<(), String> {
    if map.roads.is_empty() {
        Err("answer holds no roads (mirror busy or rate-limited?)".to_string())
    } else {
        Ok(())
    }
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
    // валидный json без карты не кешируется и не снапшотится: пустой город
    // пережил бы все перезапуски, самоизлечение ловит только провал разбора
    validate_answer(&map)?;

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
    use crate::map::osm::fixture::{Overpass, square};
    use bevy::math::Vec2;

    /// Ответ без улиц — не город: таймаут или лимит зеркала приходит
    /// валидным json, и без гейта пустой ответ закешировался бы навсегда.
    #[test]
    fn an_answer_without_roads_is_not_a_city() {
        let knobs = ParseKnobs::DEFAULT;
        let roadless = Overpass::new(City::Tula)
            .area(
                &[("building", "yes")],
                square(Vec2::new(500.0, 500.0), 30.0),
            )
            .parse_with(knobs);
        assert!(validate_answer(&roadless).is_err());

        let city = Overpass::new(City::Tula)
            .way(
                &[("highway", "secondary")],
                vec![Vec2::ZERO, Vec2::new(200.0, 0.0)],
            )
            .parse_with(knobs);
        assert_eq!(validate_answer(&city), Ok(()));
    }

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
