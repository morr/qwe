//! Классификаторы тегов: «такой тег с такой геометрией — это вот что».
//!
//! Без состояния и без обхода: каждая функция смотрит только на теги
//! (и иногда на точки) одного элемента. Это та единица, которой оперирует
//! аудит покрытия (`.claude/skills/osm-map/references/osm-coverage.md`), —
//! обход элементов остался в `parse.rs`.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use bevy::prelude::*;

use crate::map::osm::model::{
    AreaKind, BuildingUse, FenceKind, PitchKind, RailKind, RoadClass, WaterKind, polyline_length,
};
use crate::map::osm::overpass::Element;

/// Метров на этаж, когда в OSM есть только `building:levels`. Без этого
/// перевода высота была бы почти только у Нью-Йорка: `height` там проставлен у
/// 97% зданий (LiDAR-импорт), а в Европе его нет и у 2% — там маппят этажи
/// (Париж 64%, Берлин 59%, Лондон 50%, Тула 31%).
const METERS_PER_LEVEL: f32 = 3.0;

/// Границы правдоподобия высоты, м. В OSM попадаются и `height=0`, и опечатки
/// на порядок; всё за пределами трактуем как отсутствие тега — лучше дефолт
/// потребителя, чем километровый сарай.
const BUILDING_HEIGHT_RANGE: RangeInclusive<f32> = 2.0..=600.0;

/// Значения `entrance`, через которые человек не ходит: `no` — «это не вход»
/// (в Париже таких 604), `garage` — ворота для машины, `emergency` — запертая
/// пожарная дверь. Всё остальное (`yes`, `main`, `staircase`, `home`, `shop`,
/// `service`, `exit`) — дверь как дверь.
pub(super) const NON_WALKABLE_ENTRANCES: [&str; 3] = ["no", "garage", "emergency"];

/// Границы правдоподобия шага посадки аллеи, м: ниже кроны сливаются в живую
/// изгородь, выше — это уже не ряд, а отдельные деревья.
const TREE_ROW_SPACING_RANGE: RangeInclusive<f32> = 2.0..=40.0;

/// Границы правдоподобия радиуса кроны из `diameter_crown`, м. Шире лесной
/// вилки (2.5..4): аллейный или одиночный тополь честно бывает крупнее, а вот
/// `diameter_crown=50` — опечатка.
const TREE_CROWN_RADIUS_RANGE: RangeInclusive<f32> = 1.5..=8.0;

/// Границы правдоподобия ширины русла из тега `width`, м: уже полуметра — не
/// водоток, а разметочная линия; шире полусотни — либо опечатка, либо ширина
/// поймы, а не воды (такое место в OSM размечают полигоном, а не линией).
const WATER_WIDTH_RANGE: RangeInclusive<f32> = 0.5..=50.0;

/// Границы правдоподобия `lanes`: ноль — не дорога, а за восемью полосами —
/// опечатка или сумма всей развязки, а не одной ленты.
const LANES_RANGE: RangeInclusive<f32> = 1.0..=8.0;

/// Назначение здания по `building=*`. Когда значение вне словаря — прежде
/// всего `yes` (в Туле 4004 из 7465), но и любое другое, скажем
/// `construction`, — смотрится `amenity=*` того же контура: школа
/// или больница в OSM почти всегда `building=yes` + `amenity=school`. Всё,
/// что не в словаре, — [`BuildingUse::Other`]: словарь только для того, что
/// на карте встречается сотнями, а не для полноты OSM-вики.
pub(super) fn building_use(tags: &HashMap<String, String>) -> BuildingUse {
    let building = tags.get("building").map(String::as_str);
    let by_building = match building {
        Some(
            "house" | "detached" | "semidetached_house" | "terrace" | "bungalow" | "cabin" | "hut"
            | "farm" | "villa",
        ) => Some(BuildingUse::House),
        Some("apartments" | "residential" | "dormitory" | "hotel" | "hostel") => {
            Some(BuildingUse::Apartments)
        }
        Some("commercial" | "retail" | "office" | "supermarket" | "kiosk" | "shop" | "mall") => {
            Some(BuildingUse::Commercial)
        }
        Some(
            "industrial" | "warehouse" | "factory" | "hangar" | "manufacture" | "service"
            | "transportation" | "depot" | "storage_tank",
        ) => Some(BuildingUse::Industrial),
        // множественное число — это весь кооператив одним контуром, и
        // рисуется он рядами боксов, а не одной коробкой
        Some("garages") => Some(BuildingUse::GarageBlock),
        Some("garage" | "carport" | "shed" | "barn" | "roof") => Some(BuildingUse::Garage),
        Some(
            "church" | "cathedral" | "chapel" | "temple" | "mosque" | "synagogue" | "monastery"
            | "religious" | "shrine",
        ) => Some(BuildingUse::Church),
        Some(
            "school" | "hospital" | "university" | "college" | "kindergarten" | "public" | "civic"
            | "government" | "train_station" | "museum" | "library" | "stadium" | "sports_hall"
            | "fire_station" | "theatre" | "town_hall" | "courthouse",
        ) => Some(BuildingUse::Public),
        _ => None,
    };
    if let Some(class) = by_building {
        return class;
    }
    match tags.get("amenity").map(String::as_str) {
        Some("place_of_worship") => BuildingUse::Church,
        Some(
            "school" | "hospital" | "clinic" | "university" | "college" | "kindergarten" | "police"
            | "fire_station" | "townhall" | "courthouse" | "library" | "theatre"
            | "community_centre",
        ) => BuildingUse::Public,
        _ => BuildingUse::Other,
    }
}

/// [`building_use`] только для зданий: у воды и парков назначения нет.
pub(super) fn area_use(kind: AreaKind, tags: &HashMap<String, String>) -> BuildingUse {
    if matches!(kind, AreaKind::Building | AreaKind::Kremlin) {
        building_use(tags)
    } else {
        BuildingUse::Other
    }
}

/// Классификация элемента по тегам → вид площадного объекта.
pub(super) fn area_kind(element: &Element) -> Option<AreaKind> {
    let tags = &element.tags;
    if tags.contains_key("building") {
        // исторические стены/башни Кремля подкрашиваются отдельно
        let historic = tags.get("historic").map(String::as_str);
        return Some(match historic {
            Some("citywalls" | "castle" | "city_gate" | "fort") => AreaKind::Kremlin,
            _ => AreaKind::Building,
        });
    }
    let natural = tags.get("natural").map(String::as_str);
    let landuse = tags.get("landuse").map(String::as_str);
    if natural == Some("water") || tags.get("waterway").map(String::as_str) == Some("riverbank") {
        return Some(AreaKind::Water);
    }
    if matches!(natural, Some("sand" | "beach")) {
        return Some(AreaKind::Sand);
    }
    // луг проверяется до парка: газон внутри парка — отдельный светлый слой
    if matches!(landuse, Some("grass" | "meadow"))
        || matches!(natural, Some("grassland" | "meadow"))
    {
        return Some(AreaKind::Grass);
    }
    if natural == Some("wood") || landuse == Some("forest") {
        return Some(AreaKind::Wood);
    }
    if matches!(
        tags.get("leisure").map(String::as_str),
        Some("park" | "garden")
    ) || landuse == Some("recreation_ground")
    {
        return Some(AreaKind::Park);
    }
    // стоянка — после зелени и до кварталов: `amenity=parking` попадается и
    // на озеленённых дворах, но асфальт там всё-таки главное. Парковочный
    // дом (`building=*` + `amenity=parking`) сюда не доходит: здание выше
    if tags.get("amenity").map(String::as_str) == Some("parking") {
        return Some(AreaKind::Parking);
    }
    // площадка — после стоянки и до кварталов: `leisure=pitch` во дворе
    // сплошь и рядом лежит внутри `landuse=residential`, и покрытие поля
    // важнее подложки квартала. Парк проверен выше: `leisure=park` со
    // спортплощадкой на нём остаётся парком, а поле внутри придёт своим way
    if let Some(kind) = pitch_kind(tags) {
        return Some(AreaKind::Pitch(kind));
    }
    // кварталы — последними: у них нет ничего, что перекрыло бы зелень
    match landuse {
        Some("residential") => Some(AreaKind::Residential),
        Some("industrial" | "garages") => Some(AreaKind::Industrial),
        _ => None,
    }
}

/// Число из значения тега OSM. Единица измерения по умолчанию — метр, но
/// маппят и с суффиксом (`12 m`, `12.5 metres`), и с запятой (`12,5`), и через
/// точку с запятой, когда значений несколько (`3;4` — берём первое), и в футах
/// с дюймами (`40'`, `40'6"`). Не разобралось — `None`.
pub(super) fn parse_measure(value: &str) -> Option<f32> {
    let value = value.split(';').next()?.trim();

    if let Some((feet, inches)) = value.split_once('\'') {
        let feet: f32 = feet.trim().parse().ok()?;
        let inches: f32 = inches
            .trim()
            .trim_end_matches('"')
            .trim()
            .parse()
            .unwrap_or(0.0);
        return Some(feet * 0.3048 + inches * 0.0254);
    }

    // числовой префикс: всё, начиная с первого нецифрового символа, — единица
    let cleaned = value.replace(',', ".");
    let end = cleaned
        .find(|character: char| {
            !(character.is_ascii_digit() || character == '.' || character == '-')
        })
        .unwrap_or(cleaned.len());
    cleaned[..end].parse().ok()
}

/// Высота здания в метрах: `height` как есть, иначе этажи
/// (`building:levels` + `roof:levels`, второй по схеме S3DB в первый не входит)
/// по [`METERS_PER_LEVEL`]. Оба тега разом почти не встречаются, так что это не
/// «уточнение», а две независимые ветки данных.
pub(super) fn building_height(tags: &HashMap<String, String>) -> Option<f32> {
    let plausible = |meters: f32| BUILDING_HEIGHT_RANGE.contains(&meters).then_some(meters);

    if let Some(meters) = tags
        .get("height")
        .and_then(|value| parse_measure(value))
        .and_then(plausible)
    {
        return Some(meters);
    }

    let levels = tags
        .get("building:levels")
        .and_then(|value| parse_measure(value))?;
    let roof_levels = tags
        .get("roof:levels")
        .and_then(|value| parse_measure(value))
        .unwrap_or(0.0);
    plausible((levels + roof_levels) * METERS_PER_LEVEL)
}

/// Ширина и класс по значению highway; `None` — дорогу не рисуем.
pub(super) fn road_class(highway: &str) -> Option<(f32, RoadClass)> {
    Some(match highway {
        "motorway" | "trunk" | "primary" => (16.0, RoadClass::Street),
        "secondary" => (12.0, RoadClass::Street),
        "tertiary" => (10.0, RoadClass::Street),
        "residential" | "unclassified" | "living_street" => (8.0, RoadClass::Street),
        "service" => (5.0, RoadClass::Street),
        "footway" | "path" | "pedestrian" | "cycleway" | "steps" | "track" => {
            (3.5, RoadClass::Alley)
        }
        _ => return None,
    })
}

/// Ширина и состояние по значению `railway`; `None` — путь не рисуем.
///
/// Белый список, а не чёрный: под `railway=*` в OSM сидит весь словарь
/// станционного хозяйства (`platform`, `station`, `halt`, `switch`, `signal`,
/// `buffer_stop`, `turntable`, `construction`, `proposed`), и перечислять то,
/// что рисуем, короче и безопаснее, чем то, что выбрасываем.
pub(super) fn rail_class(railway: &str) -> Option<(f32, RailKind)> {
    Some(match railway {
        "rail" => (5.0, RailKind::Active),
        "light_rail" | "narrow_gauge" | "subway" => (4.0, RailKind::Active),
        // трамвай меряется не колеёй, а толщиной линии: он идёт по проезжей
        // части, и лента в ширину пути перекрыла бы саму улицу
        "tram" => (1.2, RailKind::Tram),
        "abandoned" | "disused" | "razed" | "dismantled" => (3.5, RailKind::Disused),
        _ => return None,
    })
}

/// Ширина по умолчанию и род по значению `waterway`; `None` — не водоток.
///
/// Белый список по той же причине, что у [`rail_class`]: под `waterway=*` лежит
/// не только русло, но и всё, что на нём стоит и линией не является —
/// `riverbank` (это площадь, её берёт [`area_kind`]), `dam`, `dock`, `lock_gate`,
/// `waterfall`, `fuel`, `water_point`.
///
/// Ширины — рисовальные, не гидрологические: OSM размечает линией то, что узко
/// для полигона, поэтому река здесь уже́ настоящей Упы (та размечена площадью).
/// Реальная ширина, если она есть в тегах, всё равно перебьёт эту в `parse_way`.
pub(super) fn water_class(waterway: &str) -> Option<(f32, WaterKind)> {
    Some(match waterway {
        "river" => (8.0, WaterKind::River),
        "canal" => (6.0, WaterKind::Canal),
        // водослив поперёк русла: своей ширины у него нет, лежит внутри реки
        "weir" => (4.0, WaterKind::Canal),
        "stream" | "brook" => (2.5, WaterKind::Stream),
        "ditch" | "drain" => (1.5, WaterKind::Ditch),
        _ => return None,
    })
}

/// Ширина русла из тега `width`, если она правдоподобна. Верхняя граница есть
/// не для красоты: линией размечают узкое, и `width=200` на ручье — это либо
/// опечатка, либо ширина всей поймы, а лента в 200 м накрыла бы полгорода
/// (и, поскольку водотоки блокируют навмеш, отрезала бы их друг от друга).
pub(super) fn water_width(tags: &HashMap<String, String>) -> Option<f32> {
    let width = tags.get("width").and_then(|value| parse_measure(value))?;
    WATER_WIDTH_RANGE.contains(&width).then_some(width)
}

/// Путь под землёй — метро в тоннеле, подземный перегон. Сверху его не видно,
/// значит и рисовать нечего.
///
/// Двух признаков мало по одному: в Туле из трёх подземных путей у двух стоит
/// `tunnel=yes` вместе с `layer=-1`, а у третьего только `layer=-1`. `layer`
/// читается дробным разбором, потому что в OSM попадается и `-1.5`; `tunnel=no`
/// — явное «нет», а не отсутствие тега.
pub(super) fn is_underground(tags: &HashMap<String, String>) -> bool {
    let tunnel = tags
        .get("tunnel")
        .is_some_and(|value| value != "no" && value != "building_passage");
    let below = tags
        .get("layer")
        .and_then(|value| value.parse::<f32>().ok())
        .is_some_and(|layer| layer < 0.0);
    tunnel || below
}

/// Подземна ли **сама дорога**.
///
/// Отдельный вопрос, а не [`is_underground`]: у дороги риск несимметричен.
/// Нарисовать лишнюю ленту — косметика; выбросить лишнюю — оторвать кусок
/// карты, потому что ровно дороги прорезают навмеш мостами и арками. Поэтому
/// правило отступает всюду, где тег мог описывать не дорогу:
///
/// - **мост** и **арка** существуют на уровне ходьбы по определению своей
///   роли. `layer` у них говорит не «под землёй», а «ниже того, что сверху
///   пересекает». Без этих двух исключений правило снесло бы в Токио 331
///   арку и 17 мостов, в Лондоне 177 арок — а с аркой закрывается двор,
///   в который другого входа нет, и `prune_unreachable` его ампутирует;
/// - **`culvert`** — труба **ручья** под этой улицей. В OSM ручей куда чаще
///   пускают трубой, чем строят улице мост, и оба тега висят на одном way;
///   для водотока `culvert` обязан значить «под землёй» (иначе труба
///   перегородит навмеш и отрежет квартал), а для улицы над ним — нет.
///   Явный `layer<0` поверх трубы — уже про сам way, и тогда правило
///   срабатывает.
pub(super) fn is_road_underground(tags: &HashMap<String, String>) -> bool {
    let bridge = tags.get("bridge").is_some_and(|value| value != "no");
    if bridge || is_building_passage(tags) {
        return false;
    }
    if tags.get("tunnel").map(String::as_str) == Some("culvert") {
        return tags
            .get("layer")
            .and_then(|value| value.parse::<f32>().ok())
            .is_some_and(|layer| layer < 0.0);
    }
    is_underground(tags)
}

/// Арка — дорога, проложенная сквозь здание. В Туле это `tunnel=building_passage`
/// (основной тег) и `covered` — часть таких проездов размечена только им.
/// `tunnel=yes` сюда не входит: это подземный туннель, поверху он ничего не
/// открывает.
pub(super) fn is_building_passage(tags: &HashMap<String, String>) -> bool {
    tags.get("tunnel").map(String::as_str) == Some("building_passage")
        || matches!(
            tags.get("covered").map(String::as_str),
            Some("yes" | "building_passage")
        )
}

/// Односторонняя ли улица: `oneway=yes|1|true|-1` и любое кольцо,
/// одностороннее по определению. `reversible` / `alternating` — нет: там едут
/// в обе стороны, пусть по очереди.
///
/// **Направление у такой улицы важно**: по нему ряд машин встаёт с одной
/// стороны, правой по ходу. `-1` означает «поток против порядка точек way», и
/// такой way разворачивается при разборе ([`is_oneway_backward`]), так что
/// дальше по конвейеру порядок точек всегда совпадает с направлением потока.
pub(super) fn is_oneway(tags: &HashMap<String, String>) -> bool {
    matches!(
        tags.get("oneway").map(String::as_str),
        Some("yes" | "1" | "true" | "-1")
    ) || is_roundabout(tags)
}

/// `oneway=-1` — поток идёт против порядка точек way. Нормализуется разворотом
/// точек в [`super::parse_way`], а не отдельным полем: одно понятие
/// «направление улицы» вместо двух, и ни один потребитель ниже не обязан о нём
/// помнить.
pub(super) fn is_oneway_backward(tags: &HashMap<String, String>) -> bool {
    tags.get("oneway").map(String::as_str) == Some("-1")
}

/// Кольцевая развязка: `junction=roundabout` (с приоритетом кольца) и
/// `junction=circular` (без него) — для картинки одно и то же кольцо.
pub(super) fn is_roundabout(tags: &HashMap<String, String>) -> bool {
    matches!(
        tags.get("junction").map(String::as_str),
        Some("roundabout" | "circular")
    )
}

/// Число полос из `lanes`, если оно правдоподобно. В OSM это сумма по обоим
/// направлениям; `2;3` и `2.5` попадаются и читаются как `2`. Только тег:
/// дефолт по ширине и правило кольца — у рендера (`roads::lane_count`).
pub(super) fn tagged_lanes(tags: &HashMap<String, String>) -> Option<u8> {
    let lanes = tags
        .get("lanes")
        .and_then(|value| parse_measure(value))?
        .floor();
    LANES_RANGE.contains(&lanes).then_some(lanes as u8)
}

/// Шаг посадки аллеи из тегов, м. `spacing` как есть, иначе `count` /
/// `tree:count` деревьев, растянутые на длину ряда.
///
/// Оба тега на `natural=tree_row` редки и полустандартны — подавляющее
/// большинство рядов вернёт `None` и получит шаг из ползунка плотности. Границы
/// нужны не для красоты: в OSM попадаются и `spacing=0.5`, и `count=1`.
pub(super) fn row_spacing(tags: &HashMap<String, String>, points: &[Vec2]) -> Option<f32> {
    let plausible = |meters: f32| TREE_ROW_SPACING_RANGE.contains(&meters).then_some(meters);

    if let Some(step) = tags
        .get("spacing")
        .and_then(|value| parse_measure(value))
        .and_then(plausible)
    {
        return Some(step);
    }

    let count = tags
        .get("count")
        .or_else(|| tags.get("tree:count"))
        .and_then(|value| parse_measure(value))?;
    if count < 2.0 {
        return None;
    }
    plausible(polyline_length(points) / (count - 1.0))
}

/// Радиус кроны из `diameter_crown`, м. Тег документирован на `natural=tree`
/// и переносится на ряд; `None` — радиус разыгрывается, как в лесу.
pub(super) fn crown_radius(tags: &HashMap<String, String>) -> Option<f32> {
    let diameter = tags
        .get("diameter_crown")
        .and_then(|value| parse_measure(value))?;
    let radius = diameter / 2.0;
    TREE_CROWN_RADIUS_RANGE.contains(&radius).then_some(radius)
}

/// Ограда участка: белый список, как у путей и водотоков. `barrier=*` несёт
/// ещё и `kerb`, `gate`, `bollard`, `block` — это точки и мелочь, а не линия,
/// и `city_wall`, который забирает ветка выше: кремлёвская стена
/// **непроходима**, а забор рисуется и только.
pub(super) fn fence_kind(tags: &HashMap<String, String>) -> Option<FenceKind> {
    match tags.get("barrier").map(String::as_str)? {
        "fence" => Some(FenceKind::Fence),
        "wall" | "retaining_wall" => Some(FenceKind::Wall),
        "hedge" => Some(FenceKind::Hedge),
        _ => None,
    }
}

/// Станционный путь: `service=*` есть только у путей, не относящихся к
/// главному ходу. Белый список, а не «тег есть»: `service=crossover` — это
/// съезд между главными путями, состав на нём не бросают.
///
/// Тег приезжает в кеше и так (`out geom` отдаёт все теги элемента), поэтому
/// версию запроса поднимать не понадобилось.
pub(super) fn is_service_track(tags: &HashMap<String, String>) -> bool {
    matches!(
        tags.get("service").map(String::as_str),
        Some("siding" | "yard" | "spur")
    )
}

/// Что за площадка — по `leisure`, а внутри `pitch` по `sport` и `surface`.
///
/// Белый список, как у путей и водотоков: `leisure=*` несёт ещё десяток
/// значений (`fitness_centre`, `dance`, `bandstand`, `marina`), которые
/// сверху не площадка, а здание или вовсе точка. `park` и `garden` сюда не
/// доходят — их забирает [`area_kind`] выше.
///
/// Вид спорта в OSM проставлен не всегда (в Туле у 37 из 48 полей), поэтому
/// решение трёхступенчатое: `sport`, потом `surface`, потом «твёрдая
/// площадка» — в русском дворе безымянное поле это чаще всего асфальтовая
/// коробка, а не газон.
pub(super) fn pitch_kind(tags: &HashMap<String, String>) -> Option<PitchKind> {
    let leisure = tags.get("leisure").map(String::as_str)?;
    match leisure {
        "track" => return Some(PitchKind::Track),
        "playground" => return Some(PitchKind::Playground),
        "sports_centre" | "stadium" => return Some(PitchKind::Ground),
        "pitch" => {}
        _ => return None,
    }
    // `sport=soccer;ice_hockey` — тоже поле: берём первое значение
    let sport = tags
        .get("sport")
        .map(String::as_str)
        .and_then(|value| value.split(';').next());
    if let Some(sport) = sport {
        return Some(match sport {
            "soccer" | "football" | "american_football" | "rugby" | "rugby_union" | "athletics"
            | "equestrian" | "baseball" | "cricket" | "field_hockey" | "multi" => PitchKind::Soccer,
            _ => PitchKind::Hard,
        });
    }
    Some(match tags.get("surface").map(String::as_str) {
        Some("grass" | "dirt" | "ground" | "earth") => PitchKind::Soccer,
        Some("sand") => PitchKind::Playground,
        _ => PitchKind::Hard,
    })
}

/// Высота имеет смысл только у зданий: у пруда и газона её не бывает даже при
/// случайно проставленном теге.
pub(super) fn area_height(kind: AreaKind, tags: &HashMap<String, String>) -> Option<f32> {
    matches!(kind, AreaKind::Building | AreaKind::Kremlin)
        .then(|| building_height(tags))
        .flatten()
}
