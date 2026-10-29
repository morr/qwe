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
    AreaKind, FenceKind, Highway, KerbParking, LaneTurn, LotKind, Pavement, PitchKind, RailKind,
    RoadAreaKind, RoadClass, RoadNodeKind, ServiceTrack, SidewalkSide, StructureKind, WaterKind,
    polyline_length,
};
use crate::map::osm::overpass::Element;

mod buildings;

use self::buildings::is_fortification;
pub(super) use self::buildings::{area_colours, area_height, area_storeys, area_use};
#[cfg(test)]
pub(super) use self::buildings::{building_height, colour};

/// Границы правдоподобия высоты, м. В OSM попадаются и `height=0`, и опечатки
/// на порядок; всё за пределами трактуем как отсутствие тега — лучше дефолт
/// потребителя, чем километровый сарай.
const BUILDING_HEIGHT_RANGE: RangeInclusive<f32> = 2.0..=600.0;

/// Значения `entrance`, через которые человек не ходит: `no` — «это не вход»
/// (в Париже таких 604), `garage` — ворота для машины, `emergency` — запертая
/// пожарная дверь. Всё остальное (`yes`, `main`, `staircase`, `home`, `shop`,
/// `service`, `exit`) — дверь как дверь.
pub(super) const NON_WALKABLE_ENTRANCES: [&str; 3] = ["no", "garage", "emergency"];

/// Значения `parking=*`, которых на снимке сверху нет: асфальт лежит под землёй
/// или на крыше, а контур в OSM нарисован по двору, газону или самому дому над
/// ним. Нарисовать такой контур лентой с разметкой и машинами — ровно та
/// неправдоподобность, против которой заведено это направление. Считано по
/// Overpass: `underground` — Париж 11, Берлин 22, Лондон 10, NY 3, Токио 2;
/// `multi-storey` — Берлин 3, Тула 1; `rooftop` — Берлин 1. Парного
/// `location=underground` на таких контурах нет ни в одном из шести городов,
/// поэтому смотрится только `parking`.
const HIDDEN_PARKING: [&str; 3] = ["underground", "multi-storey", "rooftop"];

/// Границы правдоподобия шага посадки аллеи, м: ниже кроны сливаются в живую
/// изгородь, выше — это уже не ряд, а отдельные деревья.
const TREE_ROW_SPACING_RANGE: RangeInclusive<f32> = 2.0..=40.0;

/// Границы правдоподобия радиуса кроны из `diameter_crown`, м. Шире лесной
/// вилки (2.5..4): аллейный или одиночный тополь честно бывает крупнее, а вот
/// `diameter_crown=50` — опечатка.
const TREE_CROWN_RADIUS_RANGE: RangeInclusive<f32> = 1.5..=8.0;

/// Границы правдоподобия радиуса промышленного цилиндра, м. Меньше 0.75 (то
/// есть полтора метра в поперечнике) — столбик, а не сооружение; больше
/// полусотни (сто метров в поперечнике) не бывает даже у газгольдера, и такое
/// значение почти всегда означает, что в `width` записали габарит площадки.
const STRUCTURE_RADIUS_RANGE: RangeInclusive<f32> = 0.75..=50.0;

/// Ширина трубы в связке с её изоляцией, м.
const PIPE_SPACING: f32 = 0.7;

/// Границы ширины связки, м, — не отбраковка, а зажим: одиночная труба у́же
/// метра на снимке не читается, а `count=20` — это уже не теплотрасса, а
/// опечатка, и рисовать её двадцатью трубами незачем.
const PIPE_WIDTH_RANGE: RangeInclusive<f32> = 0.9..=4.0;

/// Границы правдоподобия ширины русла из тега `width`, м: уже полуметра — не
/// водоток, а разметочная линия; шире полусотни — либо опечатка, либо ширина
/// поймы, а не воды (такое место в OSM размечают полигоном, а не линией).
const WATER_WIDTH_RANGE: RangeInclusive<f32> = 0.5..=50.0;

/// Какой тег `width` дорожки вообще читается, м: `0` и `0.3` — не ширина, а
/// пометка, за дюжиной метров — ширина площади или всей аллеи с газонами.
/// Тула, кеш v15: у 100 из 3375 дорожек тег есть, три `0`, один `0.3`.
const PATH_WIDTH_READ: RangeInclusive<f32> = 0.5..=12.0;
/// Зажим прочитанной ширины дорожки, м: у́же метра тропа на карте — нитка,
/// которой не видно (восемь `0.5` в Туле), шире восьми — уже не дорожка.
const PATH_WIDTH_RANGE: RangeInclusive<f32> = 1.0..=8.0;

/// Границы правдоподобия `lanes`: ноль — не дорога, а за восемью полосами —
/// опечатка или сумма всей развязки, а не одной ленты.
const LANES_RANGE: RangeInclusive<f32> = 1.0..=8.0;

/// Классификация элемента по тегам → вид площадного объекта.
pub(super) fn area_kind(element: &Element) -> Option<AreaKind> {
    let tags = &element.tags;
    if tags.contains_key("building") {
        return Some(if is_fortification(tags) {
            AreaKind::Kremlin
        } else {
            AreaKind::Building
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
    // стоянка — после зелени и до кварталов: зелёный тег на том же контуре
    // выигрывает (сквер с парковкой по краю остаётся сквером), а вот двор
    // `landuse=residential` — нет, там асфальт главное. Парковочный дом
    // (`building=*` + `amenity=parking`) сюда не доходит: здание выше. Большая
    // ли она, скажет замощённый контур (`lots::pave_lots`), не тег
    if tags.get("amenity").map(String::as_str) == Some("parking") {
        match tags.get("parking").map(String::as_str) {
            Some(value) if HIDDEN_PARKING.contains(&value) => {}
            Some("street_side") => return Some(AreaKind::Parking(LotKind::Kerbside)),
            _ => return Some(AreaKind::Parking(LotKind::Yard)),
        }
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

/// Ширина по классу, род ленты и класс по значению highway; `None` — дорогу
/// не рисуем.
///
/// Ширина здесь — **номинальная**, до сечений: у дорожек её заменяет
/// [`path_width`] (кроме мостиков и арок), у прочих её
/// пересчитывает из числа полос проход сечений
/// (`map::roads::network::sections`), первый в доводке разбора. Съезды
/// (`*_link`) долго выбрасывались целиком — словарь их не знал, и въезд на
/// мост в Туле (22 way `primary_link`) обрывался пустым местом.
pub(in crate::map::osm) fn road_class(highway: &str) -> Option<(f32, RoadClass, Highway)> {
    let street = |width: f32, highway: Highway| (width, RoadClass::Street, highway);
    Some(match highway {
        "motorway" => street(16.0, Highway::Motorway),
        "trunk" => street(16.0, Highway::Trunk),
        "primary" => street(16.0, Highway::Primary),
        "secondary" => street(12.0, Highway::Secondary),
        "tertiary" => street(10.0, Highway::Tertiary),
        "motorway_link" => street(8.0, Highway::MotorwayLink),
        "trunk_link" => street(8.0, Highway::TrunkLink),
        "primary_link" => street(8.0, Highway::PrimaryLink),
        "secondary_link" => street(8.0, Highway::SecondaryLink),
        "tertiary_link" => street(8.0, Highway::TertiaryLink),
        "residential" => street(8.0, Highway::Residential),
        "unclassified" => street(8.0, Highway::Unclassified),
        "living_street" => street(8.0, Highway::LivingStreet),
        "service" => street(5.0, Highway::Service),
        "footway" | "path" | "pedestrian" | "cycleway" | "steps" | "track" => {
            (3.5, RoadClass::Alley, Highway::Path)
        }
        _ => return None,
    })
}

/// Ширина дорожки ([`RoadClass::Alley`]), м — вместо одной на всех 3.5 м по
/// классу. Правдоподобный `width` ([`PATH_WIDTH_READ`], зажатый в
/// [`PATH_WIDTH_RANGE`]) решает сам; без него — вид и покрытие: пешеходная
/// улица 5, полевая дорога 3, переход 3, лестница и тротуар 2.5, велодорожка
/// 2, тропа по земле (`surface=dirt|ground|earth|mud|grass|sand|woodchips`)
/// 1.5, прочая `path` 2, прочий `footway` 3 — аллея сквера и дорожка двора.
/// Тула, кеш v15: 1916 голых `footway`, 571 `footway=sidewalk`, 388
/// переходов, 285 `path`, 128 лестниц, 57 `track`.
pub(super) fn path_width(tags: &HashMap<String, String>) -> f32 {
    if let Some(width) = tags
        .get("width")
        .and_then(|value| parse_measure(value))
        .filter(|width| PATH_WIDTH_READ.contains(width))
    {
        return width.clamp(*PATH_WIDTH_RANGE.start(), *PATH_WIDTH_RANGE.end());
    }
    let tag = |key: &str| tags.get(key).map(String::as_str);
    let trail = matches!(
        tag("surface"),
        Some("dirt" | "ground" | "earth" | "mud" | "grass" | "sand" | "woodchips")
    );
    match (tag("highway"), tag("footway")) {
        (Some("pedestrian"), _) => 5.0,
        (Some("track"), _) => 3.0,
        (Some("steps"), _) => 2.5,
        (_, Some("crossing")) => 3.0,
        (_, Some("sidewalk")) => 2.5,
        (Some("cycleway"), _) => 2.0,
        _ if trail => 1.5,
        (Some("path"), _) => 2.0,
        _ => 3.0,
    }
}

/// Вид дорожного узла; `None` — нода не дорожный узел (вход, дерево, труба).
///
/// Белый список по той же причине, что [`rail_class`]: под `highway=*` на
/// нодах лежат ещё `bus_stop`, `street_lamp`, `speed_camera`, `milestone`, а
/// запрос их не просит — но нода с двумя тегами сразу прийти может.
pub(super) fn road_node_kind(tags: &HashMap<String, String>) -> Option<RoadNodeKind> {
    let tag = |key: &str| tags.get(key).map(String::as_str);
    Some(match tag("highway") {
        Some("crossing") => RoadNodeKind::Crossing {
            signals: tag("crossing") == Some("traffic_signals")
                || tag("crossing:signals") == Some("yes"),
            island: tag("crossing:island") == Some("yes") || tag("crossing") == Some("island"),
            marked: tag("crossing") != Some("unmarked") && tag("crossing:markings") != Some("no"),
        },
        Some("traffic_signals") => RoadNodeKind::TrafficSignals,
        Some("stop") => RoadNodeKind::Stop,
        Some("give_way") => RoadNodeKind::GiveWay,
        Some("mini_roundabout") => RoadNodeKind::MiniRoundabout,
        Some("turning_circle" | "turning_loop") => RoadNodeKind::TurningCircle,
        _ if tag("traffic_calming") == Some("island") => RoadNodeKind::Island,
        _ => return None,
    })
}

/// Вид площади дороги у контура; `None` — контур не площадь дороги.
///
/// `area:highway` несёт класс той дороги, чьё покрытие нарисовано, и он
/// читается тем же [`road_class`], что класс линии: проезжий класс —
/// проезжая часть, аллейный — пешеходное. `highway` + `area=yes` — то же
/// самое, только тег класса другой. Значения вне словаря дорог
/// (`emergency`, `yes`) пропускаются: что это за покрытие, не сказано.
pub(super) fn road_area_kind(tags: &HashMap<String, String>) -> Option<RoadAreaKind> {
    let tag = |key: &str| tags.get(key).map(String::as_str);
    if tag("traffic_calming") == Some("island") || tag("area:highway") == Some("traffic_island") {
        return Some(RoadAreaKind::Island);
    }
    let class = match (tag("area:highway"), tag("highway"), tag("area")) {
        (Some(value), _, _) => value,
        (None, Some(value), Some("yes")) => value,
        _ => return None,
    };
    road_class(class).map(|(_, class, _)| match class {
        RoadClass::Street => RoadAreaKind::Carriageway,
        RoadClass::Alley => RoadAreaKind::Walkway,
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

/// Служебный путь: `service=*` есть только у путей, не относящихся к
/// главному ходу. Белый список, а не «тег есть»: `service=crossover` — это
/// съезд между главными путями, состав на нём не бросают.
///
/// Тег приезжает в кеше и так (`out geom` отдаёт все теги элемента), поэтому
/// версию запроса поднимать не понадобилось.
pub(super) fn service_track(tags: &HashMap<String, String>) -> Option<ServiceTrack> {
    Some(match tags.get("service").map(String::as_str)? {
        "siding" => ServiceTrack::Siding,
        "yard" => ServiceTrack::Yard,
        "spur" => ServiceTrack::Spur,
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

/// Проезд стоянки: `highway=service` + `service=parking_aisle`. Ровно этот
/// тег, а не любой `service` — `driveway`, `alley` и `drive-through` ведут
/// **к** стоянке, а не вдоль её рядов, и раскладка мест (`map::parking`)
/// развернула бы по ним ряды поперёк.
pub(super) fn is_parking_aisle(tags: &HashMap<String, String>) -> bool {
    tags.get("service").map(String::as_str) == Some("parking_aisle")
}

/// Число полос из `lanes`, если оно правдоподобно. В OSM это сумма по обоим
/// направлениям; `2;3` и `2.5` попадаются и читаются как `2`. Только тег:
/// дефолт по ширине и правило кольца — у рендера (`roads::lane_count`).
pub(super) fn tagged_lanes(tags: &HashMap<String, String>) -> Option<u8> {
    let count = |key: &str| {
        tags.get(key)
            .and_then(|value| parse_measure(value))
            .map(f32::floor)
    };
    // без общего `lanes` — сумма по направлениям: в Туле так размечено 26 way,
    // и все они с `lanes` заодно, но в Европе бывает и одно без другого. Одно
    // направление без второго — не сумма: вторая сторона не нулевая, а неизвестная
    let lanes = count("lanes").or_else(|| {
        Some(
            count("lanes:forward")?
                + count("lanes:backward")?
                + count("lanes:both_ways").unwrap_or(0.0),
        )
    })?;
    LANES_RANGE.contains(&lanes).then_some(lanes as u8)
}

/// Полосы двусторонней дороги против хода точек: `lanes:backward`, иначе
/// `lanes` − `lanes:forward` (Ростов, Текучёва: `lanes=5`, `lanes:forward=3`
/// — назад две). Полоса `lanes:both_ways` — общая средняя, делить потоки по
/// ней некому, и ответа нет; у односторонней — тоже. Сходится ли число с
/// итоговым `lanes`, проверяет проход сечений.
pub(super) fn tagged_lanes_backward(tags: &HashMap<String, String>) -> Option<u8> {
    if is_oneway(tags) {
        return None;
    }
    let count = |key: &str| {
        tags.get(key)
            .and_then(|value| parse_measure(value))
            .map(f32::floor)
    };
    if count("lanes:both_ways").is_some_and(|both| both > 0.0) {
        return None;
    }
    let backward =
        count("lanes:backward").or_else(|| Some(count("lanes")? - count("lanes:forward")?))?;
    (0.0..=*LANES_RANGE.end())
        .contains(&backward)
        .then_some(backward as u8)
}

/// Манёвры полос `[по ходу точек, против]` из `turn:lanes`. Односторонней
/// годится и общий тег, и тег направления её потока (`oneway=-1` развёрнут
/// ниже, так что поток после разбора всегда по ходу точек); двусторонней —
/// только `turn:lanes:forward` / `:backward`. Полоса — значение между `|`,
/// её манёвры — через `;`.
pub(super) fn tagged_turns(tags: &HashMap<String, String>) -> [Vec<LaneTurn>; 2] {
    let read = |key: &str| tags.get(key).map(|value| lane_turns(value));
    if is_oneway(tags) {
        let flow = if is_oneway_backward(tags) {
            "turn:lanes:backward"
        } else {
            "turn:lanes:forward"
        };
        return [
            read("turn:lanes")
                .or_else(|| read(flow))
                .unwrap_or_default(),
            Vec::new(),
        ];
    }
    [
        read("turn:lanes:forward").unwrap_or_default(),
        read("turn:lanes:backward").unwrap_or_default(),
    ]
}

/// Тротуар `[слева, справа]` по ходу точек **после разбора**: `sidewalk=both|
/// left|right|no|separate`, уточнённый `sidewalk:both|left|right`. `no`,
/// `none` и `separate` — [`SidewalkSide::None`] (отдельный footway рисуется
/// сам), остальное — [`SidewalkSide::Tagged`]. `None` — ни одного ключа
/// `sidewalk*`: решает не тег, а класс и окружение ([`untagged_sidewalks`],
/// потом `parse::infer_sidewalks`). У `oneway=-1` точки разворачиваются, и
/// стороны меняются местами вместе с ними.
pub(super) fn tagged_sidewalks(tags: &HashMap<String, String>) -> Option<[SidewalkSide; 2]> {
    use SidewalkSide::{None as Bare, Tagged};
    if !tags.keys().any(|key| key.starts_with("sidewalk")) {
        return None;
    }
    let side_of = |value: &str| {
        if matches!(value, "no" | "none" | "separate") {
            Bare
        } else {
            Tagged
        }
    };
    let mut sides = match tags.get("sidewalk").map(String::as_str) {
        Some("left") => [Tagged, Bare],
        Some("right") => [Bare, Tagged],
        Some(value) => [side_of(value); 2],
        None => [Tagged; 2],
    };
    if let Some(value) = tags.get("sidewalk:both") {
        sides = [side_of(value); 2];
    }
    for (side, key) in ["sidewalk:left", "sidewalk:right"].into_iter().enumerate() {
        if let Some(value) = tags.get(key) {
            sides[side] = side_of(value);
        }
    }
    if is_oneway_backward(tags) {
        sides.reverse();
    }
    Some(sides)
}

/// Тротуар way без тега `sidewalk*` — до прохода по окружению: у грунтовой
/// улицы (`surface=gravel|unpaved|ground|dirt|compacted|…`) его нет никогда,
/// у прочих — пока с обеих сторон ([`SidewalkSide::Inferred`]), а жилую,
/// проезд без названия и жилую зону потом проверит застройка вокруг
/// (`parse::infer_sidewalks`).
pub(super) fn untagged_sidewalks(tags: &HashMap<String, String>) -> [SidewalkSide; 2] {
    if surface_pavement(tags) == Some(Pavement::Unpaved) {
        [SidewalkSide::None; 2]
    } else {
        [SidewalkSide::Inferred; 2]
    }
}

/// Есть ли на проезжей части разметка полос — [`RoadLine::lane_markings`]:
/// только явное `lane_markings=no` её снимает. Тула, кеш v15: `no` на 11
/// way (переулки, проезды, связки), `yes` на 3.
///
/// [`RoadLine::lane_markings`]: crate::map::osm::model::RoadLine::lane_markings
pub(super) fn has_lane_markings(tags: &HashMap<String, String>) -> bool {
    tags.get("lane_markings").map(String::as_str) != Some("no")
}

/// Что говорит о покрытии тег `surface`; `None` — тега нет или значение не
/// из словаря (`clay` корта, опечатка).
fn surface_pavement(tags: &HashMap<String, String>) -> Option<Pavement> {
    Some(match tags.get("surface")?.as_str() {
        "asphalt"
        | "paved"
        | "paving_stones"
        | "paving_stones:lanes"
        | "concrete"
        | "concrete:plates"
        | "concrete:lanes"
        | "sett"
        | "cobblestone"
        | "unhewn_cobblestone"
        | "bricks"
        | "metal"
        | "wood"
        | "rubber"
        | "tartan" => Pavement::Paved,
        "unpaved" | "gravel" | "fine_gravel" | "pebblestone" | "ground" | "dirt" | "earth"
        | "mud" | "sand" | "grass" | "compacted" | "woodchips" | "grass_paver" => Pavement::Unpaved,
        _ => return None,
    })
}

/// Покрытие дорожки по её тегам — [`RoadLine::pavement`] до прохода по
/// окружению. Решает `surface`; без него — вид дорожки: тротуар и переход
/// (`footway=sidewalk|crossing`), лестница, пешеходная улица и велодорожка в
/// городе мощёные, `path` и `track` — тропа и полевая дорога. Прочий
/// `footway` без тега — `None`: в сквере это тропинка, во дворе — асфальт,
/// и спросить надо окружение (`parse::infer_pavements`).
///
/// Тула, кеш v15 (дорожки-линии, без `area=yes`): мощёных по `surface` 1952,
/// грунтовых 146, без тега 1271 — из них 690 голых `footway`, 171
/// `footway=sidewalk`, 154 `path`, 126 `footway=crossing`, 93 `steps`, 36
/// `track`.
///
/// [`RoadLine::pavement`]: crate::map::osm::model::RoadLine::pavement
pub(super) fn tagged_pavement(tags: &HashMap<String, String>) -> Option<Pavement> {
    if let Some(pavement) = surface_pavement(tags) {
        return Some(pavement);
    }
    let tag = |key: &str| tags.get(key).map(String::as_str);
    match (tag("highway"), tag("footway")) {
        (Some("footway"), Some("sidewalk" | "crossing" | "access_aisle")) => Some(Pavement::Paved),
        (Some("steps" | "pedestrian" | "cycleway"), _) => Some(Pavement::Paved),
        (Some("path" | "track"), _) => Some(Pavement::Unpaved),
        _ => None,
    }
}

/// Стоянка у бордюра `[слева, справа]` по ходу точек после разбора:
/// `parking:<side>` (или `parking:both`) и запрет из
/// `parking:<side>:restriction`, который сильнее самого места — полоса под
/// знаком «остановка запрещена» пуста. Развёрнутый `oneway=-1` меняет стороны,
/// как у тротуара.
pub(super) fn tagged_parking(tags: &HashMap<String, String>) -> [KerbParking; 2] {
    let tag = |side: &str, key: &str| {
        tags.get(&format!("parking:{side}{key}"))
            .or_else(|| tags.get(&format!("parking:both{key}")))
            .map(String::as_str)
    };
    let mut sides = ["left", "right"].map(|side| {
        if matches!(
            tag(side, ":restriction"),
            Some("no_stopping" | "no_parking" | "no_standing")
        ) {
            return KerbParking::No;
        }
        match tag(side, "") {
            Some("street_side") => KerbParking::Pocket,
            Some("lane" | "on_kerb" | "half_on_kerb" | "shoulder" | "yes") => KerbParking::Lane,
            Some("no" | "separate") => KerbParking::No,
            _ => KerbParking::Untagged,
        }
    });
    if is_oneway_backward(tags) {
        sides.reverse();
    }
    sides
}

/// Одна строка `turn:lanes`: `left|through;right`. Неизвестное слово (в Туле
/// есть `throught`) — прямо: полоса есть, и прямо из неё едут чаще всего.
fn lane_turns(value: &str) -> Vec<LaneTurn> {
    value
        .split('|')
        .map(|lane| {
            let mut turn = LaneTurn::default();
            for word in lane.split(';').map(str::trim) {
                match word {
                    "left" | "slight_left" | "sharp_left" => turn.left = true,
                    "right" | "slight_right" | "sharp_right" => turn.right = true,
                    "reverse" => {}
                    _ => turn.through = true,
                }
            }
            turn
        })
        .collect()
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

/// Промышленный цилиндр: белый список, как у путей и водотоков.
/// `man_made=*` — самый разношёрстный ключ OSM: под ним и `surveillance`, и
/// `street_cabinet`, и `pipeline`, и `bridge` (обводка моста контуром), и
/// `works` на весь завод. Круглым светлым пятном на снимке из всего этого
/// читаются пятеро.
pub(super) fn structure_kind(tags: &HashMap<String, String>) -> Option<StructureKind> {
    match tags.get("man_made").map(String::as_str)? {
        "storage_tank" => Some(StructureKind::Tank),
        "silo" => Some(StructureKind::Silo),
        "chimney" => Some(StructureKind::Chimney),
        "water_tower" => Some(StructureKind::WaterTower),
        "gasometer" => Some(StructureKind::Gasometer),
        _ => None,
    }
}

/// Радиус цилиндра из тегов: `diameter` (он же `width` у трубы — её меряют
/// поперёк) пополам. Значение вне [`STRUCTURE_RADIUS_RANGE`] не зажимается, а
/// считается отсутствующим — как и всюду в этом файле, дальше берётся типовой
/// радиус рода ([`structure_size`]).
pub(super) fn structure_radius(tags: &HashMap<String, String>) -> Option<f32> {
    let diameter = ["diameter", "width"]
        .iter()
        .find_map(|key| tags.get(*key))
        .and_then(|value| parse_measure(value))?;
    let radius = diameter / 2.0;
    STRUCTURE_RADIUS_RANGE.contains(&radius).then_some(radius)
}

/// Высота цилиндра из тега `height`. `building:levels` тут не годится:
/// этажей у трубы не бывает, и [`building_height`] брать целиком незачем.
pub(super) fn structure_height(tags: &HashMap<String, String>) -> Option<f32> {
    let meters = tags.get("height").and_then(|value| parse_measure(value))?;
    BUILDING_HEIGHT_RANGE.contains(&meters).then_some(meters)
}

/// Ширина связки надземного трубопровода, м, или `None`, если он подземный.
///
/// Правило **обратное** тому, что у путей и водотоков: там подземное надо
/// доказать (`is_underground`), здесь — надземное. В OSM трубопровод без
/// `location` по умолчанию закопан, и таких большинство; провести через весь
/// город серебристую линию по закопанной трубе — враньё крупнее, чем потерять
/// эстакаду, у которой забыли тег.
///
/// Ширину даёт `count` — число труб в пучке. В Туле это почти всегда
/// теплотрасса: пара (подача и обратка) у двенадцати ways, четвёрка у шести.
pub(super) fn pipe_width(tags: &HashMap<String, String>) -> Option<f32> {
    let overground = matches!(
        tags.get("location").map(String::as_str),
        Some("overground" | "overhead" | "bridge")
    );
    if !overground {
        return None;
    }
    let count = tags
        .get("count")
        .and_then(|value| parse_measure(value))
        .unwrap_or(2.0);
    Some((count * PIPE_SPACING).clamp(*PIPE_WIDTH_RANGE.start(), *PIPE_WIDTH_RANGE.end()))
}

/// Радиус и высота по умолчанию, м. Нужны почти всегда: из десяти
/// сооружений Тулы размер указан **у одного**, и это высота трубы. Числа —
/// типовые для советской промзоны: заводская труба под шестьдесят метров,
/// водонапорная башня под тридцать, резервуар нефтебазы низкий и широкий.
///
/// Радиус из этой пары идёт в дело только у ноды: у way он считается по
/// контуру, который заведомо честнее.
pub(super) fn structure_size(kind: StructureKind) -> (f32, f32) {
    match kind {
        StructureKind::Tank => (8.0, 12.0),
        StructureKind::Silo => (4.0, 25.0),
        StructureKind::Chimney => (2.5, 60.0),
        StructureKind::WaterTower => (5.0, 28.0),
        StructureKind::Gasometer => (20.0, 30.0),
    }
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

/// Ограда участка: белый список, как у путей и водотоков. `barrier=*` несёт
/// ещё и `kerb`, `gate`, `bollard`, `block` — это точки и мелочь, а не линия,
/// и `city_wall`, который забирает ветка выше: кремлёвская стена
/// **непроходима**, а забор рисуется и только.
///
/// Забор, у которого **каждый** материал `fence_type` сквозной
/// ([`SEE_THROUGH_FENCES`]), — [`FenceKind::Railing`]: тени не отбрасывает.
/// Смешанный `barbed_wire;concrete` — сплошной: бетон тень даёт.
pub(super) fn fence_kind(tags: &HashMap<String, String>) -> Option<FenceKind> {
    match tags.get("barrier").map(String::as_str)? {
        "fence" => Some(match tags.get("fence_type") {
            Some(types)
                if types
                    .split(';')
                    .all(|kind| SEE_THROUGH_FENCES.contains(&kind.trim())) =>
            {
                FenceKind::Railing
            }
            _ => FenceKind::Fence,
        }),
        "wall" | "retaining_wall" => Some(FenceKind::Wall),
        "hedge" => Some(FenceKind::Hedge),
        _ => None,
    }
}

/// Сквозные `fence_type`: решётка, сетка, проволока, жерди. Тула, кеш v15:
/// `wire` 20, `metal` 18, `chain_link` 15, `bars` 5, `pole` 3 из 84 заборов с
/// тегом; `metal` в Туле — решётки (разделительная Советской), профнастил
/// ходит под `corrugated_metal`.
const SEE_THROUGH_FENCES: &[&str] = &[
    "metal",
    "metal_bars",
    "railing",
    "bars",
    "chain_link",
    "mesh",
    "wire",
    "barbed_wire",
    "electric",
    "pole",
    "split_rail",
];

#[cfg(test)]
mod tests;
