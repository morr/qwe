//! Примеры пересечений города — манифест и срезы OSM под ними.
//!
//! Пример здесь — **не геометрия, выписанная руками**, а кусок настоящей
//! выгрузки Overpass: окно вокруг одного узла, вырезанное из **того же кеша
//! города, что читает игра** (`assets/osm/<город>_…_vN.json`), в том же
//! формате (`out geom`, все теги как есть). Режет его при запуске и на `F5`
//! игровой `map::osm::crop`, разбирает игровой `parse`, рисуют игровые
//! `mesh_*` — так что на одном перекрёстке виден весь путь «данные OSM →
//! пиксели». Кеша нет — его качает игровой загрузчик
//! (`download::city_extract`), тот же файл и тот же путь.
//!
//! Срез поэтому **всегда равен игре**: поднялась `QUERY_VERSION`, пришли новые
//! теги и узлы — они в срезе с первого запуска. Цена — срез не неизменен:
//! перекачанный кеш может принести правку маппера в перекрёсток.
//!
//! **Замороженный срез — исключение, не правило.** Лежит рядом с манифестом
//! `data/<город>/<name>.json` — берётся он, а не кеш: так фиксируется репро
//! бага, и так работает «поправил тег в файле — `F5`». Файл получается
//! выгрузкой: `ROADS_DUMP=<папка>` пишет туда каждый срез, как его нарезала
//! витрина.
//!
//! Манифест `data/<город>.json` — перечень примеров города: имя, вид
//! пересечения, что на нём смотреть, центр окна и его полуразмер. Центр хранится
//! **гео-координатами**, а не метрами карты: метры считаются от угла bbox и
//! уезжают вместе с `MAP_SIZE`, гео-точка — нет. Игровые координаты под
//! примером считаются из неё проекцией города, той же, что в разборе.
//!
//! Адрес тоже не выписан, а прочитан из самого среза: страна — с границы, на
//! которой OSM держит `driving_side`, улицы — `name` тех `highway`, что проходят
//! через центр окна. Данных о регионе и городе в выгрузке нет (запрос их не
//! просит), поэтому эти два слова — таблицей ниже.
//!
//! # Критерии добавления примера
//!
//! Колонка — **перечень типов пересечения**, а не коллекция перекрёстков, и
//! каждое правило ниже — решение автора, принятое на живой витрине.
//!
//! 1. **Один пример — один тип пересечения, и каждый тип — один раз.** Тип —
//!    это то, чем узел отличается для конвейера: число и углы лучей, классы и
//!    ширины сошедшихся дорог, односторонность, кольцо, связка, арка, тупик.
//!    Другая улица того же устройства — не новый тип. Нашёлся дубль — лишний
//!    пример удаляется.
//! 2. **Числа примеров нет.** Типов в городе столько, сколько есть; добирать
//!    колонку до круглой цифры похожими узлами нельзя — меньше, но без повторов.
//! 3. **Тип читается в окне с первого взгляда.** Узел стоит в центре, а окно
//!    (`half`) взято ровно под него: лучи видны до первого прямого участка, и
//!    второго узла другого типа в кадре нет. Если в окне два разных
//!    пересечения, это два примера или ни одного. **Проверяется глазами, а не
//!    по данным**: узел, найденный в кеше как «Т» или «пять лучей», в кадре
//!    нередко оказывается обычной крестовиной (второй Т в пяти метрах, пятый
//!    луч — дворовый проезд) — так из тридцати найденных осталось пятнадцать.
//!    Заголовок называет то, что видно в окне.
//! 4. **Только одноуровневые пересечения дорог.** Переездов через рельсы и
//!    многоуровневых развязок (путепровод над улицей, петли съездов) здесь нет:
//!    колонка про то, как дороги сходятся в плоскости. Мост через реку с
//!    примыканием у его конца — одноуровневый узел и годится.
//! 5. **Пример про дорогу.** Место, где кадр занят другим слоем — стоянкой
//!    торгового центра, путевым развитием, — не годится, даже если узел под ним
//!    редкий: смотреть будут не на него.
//!    **Дорога в кадре видна целиком.** Арка сквозь дом
//!    (`tunnel=building_passage`) не годится: сверху улицу на этом участке
//!    закрывает крыша, и пример читается как «дом нарисован поверх дороги» —
//!    так его и прочёл автор. Проём арки лежит в стенах, которых в 2.5D не
//!    видно, так что показать тут нечего.
//! 6. **Типовой для города, а не курьёз.** Узел должен встречаться в городе не
//!    однажды (счёт даёт разбор кеша по общим нодам); единственный в своём
//!    роде берётся только тогда, когда он и есть тип — большое кольцо, пять
//!    лучей.
//! 7. **Только настоящие данные.** Пример — срез кеша города, не правленный
//!    руками; `title` называет тип, `note` — что на нём должен показать
//!    рендер. Новый пример добавляется точкой в метрах карты: узел находит
//!    `tools/osm_near`, в манифест пишется `"at": [x, y]`, витрина печатает в
//!    лог его `geo` — его и надо вписать вместо `at`.
//! 8. **Выглядит как в игре.** Новый пример сверяется с кадром того же места из
//!    игры (`OffscreenShotEvent` по игровым координатам из подписи). Расхождение
//!    — дефект витрины, а не особенность примера.
//!
//! # Эталон: снимок Яндекс Карт
//!
//! В `data/<город>/` лежит `<name>.yandex.png` — то же окно на схеме Яндекса;
//! витрина ставит его справа от рендера, в том же размере. Охват у снимка
//! обязан совпадать с окном, иначе сравнивать нечего, и получается он счётом,
//! а не на глаз:
//!
//! - карта открывается по центру примера на `z=19`:
//!   `https://yandex.ru/maps/15/tula/?ll=<lon>%2C<lat>&z=19`. У web-меркатора на
//!   этом зуме `156543.034 · cos(широты) / 2¹⁹` метров на CSS-пиксель — в Туле
//!   0.1747; окно примера — квадрат `2 · half / 0.1747` CSS-пикселей;
//! - **где в кадре центр карты, надо мерить, а не считать серединой окна.** У
//!   Яндекса `ll` — центр видимой части карты, правее боковой панели и чуть
//!   выше середины. Меряется меткой: тот же URL с `&pt=<lon>%2C<lat>` ставит в
//!   точку синий кружок; его место в кадре и есть центр для вырезания. На глаз
//!   по самому узлу выходила ошибка в девять метров;
//! - **место метки читается из DOM, а не со снимка:**
//!   `document.querySelector('.map-placemark').getBoundingClientRect()` даёт
//!   точку привязки в CSS-пикселях (кончик кружка, а не его середина). При
//!   том же размере окна браузера она одна для всех примеров — меряется один
//!   раз, дальше тот же URL снимается уже без `pt`, чтобы метки не было на
//!   эталоне. Область вырезания переводится из CSS-пикселей в кадр снимка
//!   множителем `ширина кадра / innerWidth`;
//! - сворачивать панель незачем и ненадёжно: клик по ней сразу после перехода
//!   не срабатывает, а центр после сворачивания другой;
//! - **снимать дважды.** Карта дорисовывает тайлы лениво: первый захват страницы
//!   приходит с пустой полосой с краю, второй через несколько секунд — полный;
//! - снимается живым браузером (Claude in Chrome): безголовому Chrome Яндекс
//!   отдаёт заглушку `limited`. Файл даёт `zoom` по области вырезания с
//!   `save_to_disk` — PNG. Больше 710 пикселей по стороне он не бывает, отсюда
//!   и размеры готовых снимков. С `scale` на диск ложится уменьшенная копия,
//!   так что 0.65 — это как раз около 710 px, а 0.3 — уже 330. `screencapture`
//!   macOS из сессии не работает («could not create image from display»).
//!
//! Подписи улиц и значки организаций на снимке остаются — на схеме Яндекса они
//! не отключаются.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bevy::prelude::*;
use qwe::city::City;
use qwe::map::osm::crop::{self, Cropper, GeoRect};
use qwe::map::osm::download::city_extract;
use qwe::map::osm::model::distance_to_segment;
use qwe::map::osm::overpass::{Element, GeoBounds, LatLon, OverpassResponse, cache_path};
use serde::Deserialize;

/// Дорога считается проходящей через узел, если её осевая ближе этого к центру
/// окна, м. Больше полуширины магистрали: у разделённого проспекта узлов два, и
/// центр окна стоит между половинами.
const ADDRESS_REACH: f32 = 14.0;

/// На сколько срез шире видимого окна, м. Витрина рисует только окно, так что
/// запас не виден — он для того, что конвейер *читает* вокруг точки: ряд, по
/// которому дом отодвигается от тротуара, дорогу, до которой мостится стоянка,
/// квартал, с которого машины берут плотность (`cars::district`, 120 м).
const CROP_MARGIN: f64 = 120.0;

/// Переменная окружения: папка, куда выгрузить каждый срез файлом — тем же,
/// что витрина берёт замороженным (`data/<город>/<name>.json`).
const DUMP_ENV: &str = "ROADS_DUMP";

#[derive(Deserialize)]
struct Manifest {
    samples: Vec<ManifestSample>,
}

#[derive(Deserialize)]
struct ManifestSample {
    /// Имя примера: по нему лежат снимок Яндекса `<name>.yandex.png` и, если
    /// есть, замороженный срез `<name>.json`.
    name: String,
    title: String,
    note: String,
    /// `(широта, долгота)` центра окна.
    geo: Option<[f64; 2]>,
    /// Центр окна в метрах карты — так пример добавляется; витрина печатает
    /// его `geo` в лог, и в манифест вписывается уже он.
    at: Option<[f32; 2]>,
    /// Полуразмер **видимого** окна, м. Срез шире на [`CROP_MARGIN`]:
    /// обрезанные концы дорог, половины теней и дома за краем остаются под
    /// маской.
    half: f32,
}

/// Один пример: срез OSM и всё, что про него написано под окном.
pub(crate) struct Sample {
    pub(crate) title: String,
    pub(crate) note: String,
    pub(crate) address: String,
    /// Центр окна в игровых координатах — метрах карты города.
    pub(crate) at: Vec2,
    pub(crate) half: f32,
    /// Откуда срез — в подписи: кеш города или замороженный файл.
    pub(crate) source: String,
    /// Сколько в срезе нод, way и relation — первая стадия пути к пикселям.
    pub(crate) elements: [usize; 3],
    /// Ответ Overpass: его и ест `parse_response`.
    pub(crate) osm: OverpassResponse,
    /// Снимок того же окна с Яндекс Карт (`data/<город>/<name>.yandex.png`),
    /// если он снят: эталон, с которым рендер сравнивают глазами. Охват у него
    /// тот же, что у окна, — квадрат `2 · half` метров вокруг того же центра.
    pub(crate) reference: Option<PathBuf>,
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

/// Примеры города: манифест с диска, срезы — из кеша города (или
/// замороженные файлы, где они есть). `Err` — причина словами: её показывает
/// витрина.
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
    let folder = data_dir().join(city.slug());
    let dump = std::env::var_os(DUMP_ENV).map(PathBuf::from);

    // кеш читается, только если хоть одному примеру он нужен: у города из одних
    // замороженных срезов незачем разбирать 18 МБ
    let needs_extract = manifest
        .samples
        .iter()
        .any(|sample| !frozen_path(&folder, &sample.name).exists());
    let extract = if needs_extract {
        Some(read_extract(city)?)
    } else {
        None
    };
    let cropper = extract.as_ref().map(Cropper::new);
    let cache_name = cache_path(city)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    manifest
        .samples
        .into_iter()
        .map(|sample| {
            let geo = match (sample.geo, sample.at) {
                (Some([lat, lon]), _) => LatLon { lat, lon },
                (None, Some([x, y])) => {
                    let geo = bounds.unproject(Vec2::new(x, y));
                    info!(
                        "roads: {} at [{x}, {y}] is \"geo\": [{:.6}, {:.6}]",
                        sample.name, geo.lat, geo.lon
                    );
                    geo
                }
                (None, None) => return Err(format!("{}: нет ни geo, ни at", sample.name)),
            };
            let frozen = frozen_path(&folder, &sample.name);
            let (osm, source) = match &cropper {
                Some(cropper) if !frozen.exists() => {
                    let window = GeoRect::around(&bounds, geo, sample.half as f64 + CROP_MARGIN);
                    (cropper.crop(window), format!("срез кеша {cache_name}"))
                }
                _ => (
                    read_frozen(&frozen)?,
                    format!("{} — замороженный", frozen.display()),
                ),
            };
            if let Some(dump) = &dump {
                write_dump(dump, &sample.name, &osm)?;
            }
            let at = bounds.project(geo.lat, geo.lon);
            let reference = Some(folder.join(format!("{}.yandex.png", sample.name)))
                .filter(|path| path.exists());
            Ok(Sample {
                reference,
                elements: counts(&osm),
                address: address(city, &osm, &bounds, at),
                title: sample.title,
                note: sample.note,
                at,
                half: sample.half,
                source,
                osm,
            })
        })
        .collect()
}

fn frozen_path(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.json"))
}

/// Выгрузка города целиком — из кеша игры, а нет его — с зеркал игровым же
/// загрузчиком.
fn read_extract(city: City) -> Result<OverpassResponse, String> {
    let started = Instant::now();
    let json = city_extract(city)?;
    let read = started.elapsed();
    let extract: OverpassResponse = serde_json::from_str(&json)
        .map_err(|error| format!("{}: {error}", cache_path(city).display()))?;
    info!(
        "roads: extract of {} MB read in {read:.0?}, deserialized in {:.0?} ({} elements)",
        json.len() / 1_000_000,
        started.elapsed() - read,
        extract.elements.len()
    );
    Ok(extract)
}

fn read_frozen(path: &Path) -> Result<OverpassResponse, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_dump(dir: &Path, name: &str, osm: &OverpassResponse) -> Result<(), String> {
    let path = dir.join(format!("{name}.json"));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&path, crop::to_json(osm)))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    info!("roads: dumped {}", path.display());
    Ok(())
}

/// Счёт элементов среза по видам: node, way, relation.
fn counts(osm: &OverpassResponse) -> [usize; 3] {
    let count = |kind: &str| {
        osm.elements
            .iter()
            .filter(|element| element.kind == kind)
            .count()
    };
    [count("node"), count("way"), count("relation")]
}

/// Полный адрес узла: страна, регион, город и улицы, сошедшиеся в центре окна.
///
/// Читает элементы Overpass, а не `MapData`: у `RoadLine` имени нет — карте
/// оно ни к чему, — а в срезе оно лежит тегом на каждом way.
fn address(city: City, osm: &OverpassResponse, bounds: &GeoBounds, at: Vec2) -> String {
    fn tag<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
        element.tags.get(key).map(String::as_str)
    }
    let country = osm
        .elements
        .iter()
        .filter(|element| element.kind == "relation" && tag(element, "admin_level") == Some("2"))
        .find_map(|element| tag(element, "name"));

    // улицы — по близости осевой к центру, ближайшая первой
    let mut streets: Vec<(f32, &str)> = Vec::new();
    for element in &osm.elements {
        let (Some(_), Some(name), Some(geometry)) = (
            tag(element, "highway"),
            tag(element, "name"),
            element.geometry.as_deref(),
        ) else {
            continue;
        };
        let points: Vec<Vec2> = geometry
            .iter()
            .map(|point| bounds.project(point.lat, point.lon))
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
    format!("{}, {crossing}", parts.join(", "))
}
