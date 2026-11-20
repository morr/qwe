//! Бинарный снапшот [`MapData`] рядом с JSON-кешем выгрузки: разбор Парижа —
//! крупнейшая последовательная часть тёплого старта (5.5 с, запись 14), а
//! сериализованная форма разобранной карты читается за долю секунды.
//!
//! Жизненный цикл — как у JSON-кеша (`overpass.rs`): пишется **после**
//! успешного разбора (поток загрузки держит готовый `MapData` до записи),
//! читается до него; битый файл удаляется и карта разбирается заново
//! (самолечение) — считается битым и снапшот карты без улиц: валидная
//! форма, чужое содержимое. Инвалидация — именем файла: слаг города,
//! гео-центр, размер
//! карты и `QUERY_VERSION` наследуются от имени JSON-кеша ([`snapshot_path`]
//! строится из [`overpass::cache_path`]), плюс собственные слагаемые:
//!
//! - `s{SNAPSHOT_VERSION}` — форма снапшота. Бинарь формата чувствителен к
//!   любому изменению полей модели, и ошибка декодирования ловит далеко не
//!   всякое изменение (сдвинувшееся поле может дочитаться мусором). Пока
//!   меняется то, что разбор кладёт в `MapData`, номер растёт — дисциплина
//!   та же, что у `QUERY_VERSION`;
//! - `k{lane_width}_{navtile}_{raw}` — входы разбора [`ParseKnobs`]: снапшот
//!   — результат разбора с ними, а не только с выгрузкой.
//!
//! Сверх имени заголовок файла ([`Header`]) держит размер и mtime JSON-кеша,
//! под которым снапшот писался: выгрузка под тем же именем может смениться
//! (кеш удалён и перекачан — задокументированный способ обновить карту), и
//! имя такое снапшот не отличит.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use bevy::log::{info, warn};

use super::model::MapData;
use super::overpass::cache_path;
use super::parse::ParseKnobs;
use crate::city::City;

/// Форма снапшота; растёт при любом изменении того, что разбор кладёт в
/// [`MapData`] — см. модульную шапку.
///
/// 2 — обрезка покрытий карманов по окну плитки
/// (`parse/pockets.rs::clipped_to_rect`): набор карманов тот же, но сетка
/// целого движка `i_overlay` в плитке зависит от габарита её входа, и на
/// волосных зазорах кольца выходят с другой плотностью вершин (до 0.3 м на
/// одном кармане Парижа) — байты не совпадают со снапшотами до обрезки.
const SNAPSHOT_VERSION: u32 = 2;

/// Размер и mtime JSON-кеша, под которым писался снапшот.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JsonStamp {
    len: u64,
    mtime: u64,
}

/// Заголовок файла снапшота; за ним — бинарь `MapData`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Header {
    version: u32,
    knobs: ParseKnobs,
    json_len: u64,
    json_mtime: u64,
}

/// Чем снапшот не годится: устарел (чужая версия/входы/выгрузка) или бит.
#[derive(Debug)]
enum Rejected {
    Stale,
    Corrupt(String),
}

/// Файл снапшота города: имя JSON-кеша минус `.json` плюс версия формы и
/// входы разбора. Параметры выгрузки (гео-центр, размер, `QUERY_VERSION`)
/// приходят из родительского имени сами.
pub fn snapshot_path(city: City, knobs: ParseKnobs) -> PathBuf {
    let stem = cache_path(city).with_extension("");
    PathBuf::from(format!(
        "{}_s{SNAPSHOT_VERSION}_k{}_{}_{}.bin",
        stem.display(),
        knobs.lane_width,
        knobs.navtile,
        raw_word(knobs.raw),
    ))
}

fn raw_word(raw: super::parse::RawOsm) -> &'static str {
    match raw {
        super::parse::RawOsm::Off => "off",
        super::parse::RawOsm::Parse => "parse",
        super::parse::RawOsm::Draw => "draw",
    }
}

/// Штамп JSON-кеша города — `None`, если кеша нет: без выгрузки, под которой
/// снапшот писался, он не годится.
fn json_stamp(city: City) -> Option<JsonStamp> {
    let meta = std::fs::metadata(cache_path(city)).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Some(JsonStamp {
        len: meta.len(),
        mtime,
    })
}

/// Готовая карта из снапшота — тёплый путь загрузки. Файла или его выгрузки
/// нет, версия, входы или штамп кеша чужие — `None` (устаревший файл
/// удаляется); битый — тоже удаляется, разбор начнётся с JSON-кеша.
pub fn read(city: City, knobs: ParseKnobs) -> Option<MapData> {
    let path = snapshot_path(city, knobs);
    let stamp = json_stamp(city)?;
    let mut reader = match File::open(&path) {
        Ok(file) => BufReader::new(file),
        Err(_) => return None,
    };
    let started = std::time::Instant::now();
    match deserialize_from(&mut reader, knobs, stamp) {
        Ok(map) => {
            info!(
                "osm: snapshot hit at {} (read in {:?})",
                path.display(),
                started.elapsed()
            );
            Some(map)
        }
        Err(rejected) => {
            warn!(
                "osm: snapshot rejected ({reason}), falling back to json — removing {}",
                path.display(),
                reason = match &rejected {
                    Rejected::Stale => "stale".to_string(),
                    Rejected::Corrupt(error) => format!("corrupt: {error}"),
                },
            );
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// Записать снапшот разобранной карты. Молча не пишет без JSON-кеша (штамп
/// взять неоткуда) и не поднимает ошибку наверх: сбой записи не должен
/// ронять загрузку, следующий старт просто разберёт JSON.
pub fn write(city: City, knobs: ParseKnobs, map: &MapData) {
    let path = snapshot_path(city, knobs);
    let Some(stamp) = json_stamp(city) else {
        warn!("osm: json cache gone, snapshot not written");
        return;
    };
    let started = std::time::Instant::now();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = match File::create(&path) {
        Ok(file) => file,
        Err(error) => {
            warn!("osm: snapshot write failed: {error}");
            return;
        }
    };
    let mut writer = BufWriter::new(file);
    let written = serialize_into(&mut writer, knobs, stamp, map)
        .and_then(|()| writer.flush().map_err(|error| error.into()));
    match written {
        Ok(()) => info!(
            "osm: snapshot written {} ({} MB) in {:?}",
            path.display(),
            std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) / 1_048_576,
            started.elapsed()
        ),
        Err(error) => {
            warn!("osm: snapshot write failed: {error}");
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn serialize_into(
    writer: &mut impl Write,
    knobs: ParseKnobs,
    stamp: JsonStamp,
    map: &MapData,
) -> Result<(), Box<dyn std::error::Error>> {
    bincode::serialize_into(
        &mut *writer,
        &Header {
            version: SNAPSHOT_VERSION,
            knobs,
            json_len: stamp.len,
            json_mtime: stamp.mtime,
        },
    )?;
    bincode::serialize_into(writer, map)?;
    Ok(())
}

fn deserialize_from(
    reader: &mut impl Read,
    knobs: ParseKnobs,
    stamp: JsonStamp,
) -> Result<MapData, Rejected> {
    let header: Header = bincode::deserialize_from(&mut *reader)
        .map_err(|error| Rejected::Corrupt(error.to_string()))?;
    if header.version != SNAPSHOT_VERSION
        || header.knobs != knobs
        || header.json_len != stamp.len
        || header.json_mtime != stamp.mtime
    {
        return Err(Rejected::Stale);
    }
    let map: MapData = bincode::deserialize_from(&mut *reader)
        .map_err(|error| Rejected::Corrupt(error.to_string()))?;
    // валидная форма, чужое содержимое: снапшот, под которым нет улиц, —
    // не карта города (отравленный пустым ответом зеркала); лечится как
    // битый — удаляется, разбор начнётся с json-кеша
    if map.roads.is_empty() {
        return Err(Rejected::Corrupt("decoded map holds no roads".to_string()));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{Overpass, square};
    use bevy::math::Vec2;

    fn stamp() -> JsonStamp {
        JsonStamp {
            len: 1234,
            mtime: 1700000000,
        }
    }

    fn scene() -> Overpass {
        Overpass::new(City::Tula)
            .area(
                &[("building", "yes")],
                square(Vec2::new(500.0, 500.0), 30.0),
            )
            .way(
                &[("highway", "secondary")],
                vec![Vec2::ZERO, Vec2::new(200.0, 0.0)],
            )
            .node(&[("natural", "tree")], Vec2::new(600.0, 600.0))
    }

    #[test]
    fn snapshot_gives_the_same_map_as_parse() {
        let knobs = ParseKnobs::DEFAULT;
        let parsed = scene().parse_with(knobs);
        let mut bytes = Vec::new();
        serialize_into(&mut bytes, knobs, stamp(), &parsed).unwrap();

        let decoded = deserialize_from(&mut bytes.as_slice(), knobs, stamp()).unwrap();
        assert_eq!(parsed, decoded);
    }

    #[test]
    fn a_roadless_snapshot_is_corrupt_not_a_map() {
        let knobs = ParseKnobs::DEFAULT;
        // сцена без улиц — то, что разбор делает из пустого ответа зеркала
        let roadless = Overpass::new(City::Tula)
            .area(
                &[("building", "yes")],
                square(Vec2::new(500.0, 500.0), 30.0),
            )
            .parse_with(knobs);
        let mut bytes = Vec::new();
        serialize_into(&mut bytes, knobs, stamp(), &roadless).unwrap();

        assert!(matches!(
            deserialize_from(&mut bytes.as_slice(), knobs, stamp()),
            Err(Rejected::Corrupt(_))
        ));
    }

    #[test]
    fn snapshot_of_other_knobs_is_stale() {
        let knobs = ParseKnobs::DEFAULT;
        let parsed = scene().parse_with(knobs);
        let mut bytes = Vec::new();
        serialize_into(&mut bytes, knobs, stamp(), &parsed).unwrap();

        let other = ParseKnobs {
            lane_width: 4.2,
            ..ParseKnobs::DEFAULT
        };
        assert!(matches!(
            deserialize_from(&mut bytes.as_slice(), other, stamp()),
            Err(Rejected::Stale)
        ));
    }

    #[test]
    fn truncated_snapshot_is_corrupt_not_stale() {
        let knobs = ParseKnobs::DEFAULT;
        let parsed = scene().parse_with(knobs);
        let mut bytes = Vec::new();
        serialize_into(&mut bytes, knobs, stamp(), &parsed).unwrap();
        bytes.truncate(bytes.len() / 2);

        assert!(matches!(
            deserialize_from(&mut bytes.as_slice(), knobs, stamp()),
            Err(Rejected::Corrupt(_))
        ));
    }

    #[test]
    fn snapshot_name_covers_query_version_and_knobs() {
        let knobs = ParseKnobs::DEFAULT;
        let path = snapshot_path(City::Tula, knobs).display().to_string();
        // версия выгрузки наследуется от имени json-кеша
        assert!(path.contains("_v15_"), "no query version in {path}");
        // и входы разбора в имени
        assert!(path.contains("_k3.3_2_off.bin"), "no knobs in {path}");
    }
}
