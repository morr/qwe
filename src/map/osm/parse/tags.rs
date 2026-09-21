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
    AreaKind, BIG_BOX_MAX_HEIGHT, BIG_BOX_MAX_LEVELS, BuildingUse, Colours, Faith, FenceKind,
    Highway, PitchKind, RailKind, Rgb, RoadAreaKind, RoadClass, RoadNodeKind, Sacred, SacredForm,
    ServiceTrack, StructureKind, WaterKind, is_big_box_shape, polyline_length,
};
use crate::map::osm::overpass::Element;
use crate::settings::STOREY_HEIGHT;

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
        // Само здание — магазин; контора и павильон остаются торговлей вообще.
        // `shop` здесь не по недосмотру: в OSM `building=shop` — прямой синоним
        // `building=retail` («здание магазина»), и белый список строкой ниже
        // сам считает торговыми `shop=mall` и `shop=department_store`. Конторой
        // такой контур рисовался бы витражом без кассеты, фриза и шага входов.
        Some("retail" | "supermarket" | "mall" | "department_store" | "shop") => {
            Some(BuildingUse::Retail)
        }
        Some("commercial" | "office" | "kiosk") => Some(BuildingUse::Commercial),
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
            | "religious" | "shrine" | "bell_tower" | "campanile" | "minaret",
        ) => Some(BuildingUse::Church(sacred(tags))),
        Some(
            "school" | "hospital" | "university" | "college" | "kindergarten" | "public" | "civic"
            | "government" | "train_station" | "museum" | "library" | "stadium" | "sports_hall"
            | "fire_station" | "theatre" | "town_hall" | "courthouse",
        ) => Some(BuildingUse::Public),
        _ => None,
    };
    // Крупноформатный `shop=*` **уточняет общее значение `building`, но не
    // спорит с конкретным**. В OSM торговая коробка приходит тремя способами:
    // `building=retail` (сказано всё), `building=yes` + `shop=mall` (ТРЦ
    // «Макси», 64 тысячи м²) и `building=commercial` + `shop=supermarket`
    // («Магнит», 9 тысяч) — и в двух последних назначение здания знает
    // **только** `shop`. При этом `commercial` значит «коммерческое здание
    // вообще», а `house`, `apartments`, `church`, `industrial` называют совсем
    // другой дом, и булочная или «Пятёрочка» на его первом этаже этого не
    // отменяют: над такими значениями `shop` не властен.
    let generic = matches!(by_building, None | Some(BuildingUse::Commercial));
    if generic && is_big_format_shop(tags) {
        return BuildingUse::Retail;
    }
    if let Some(class) = by_building {
        return class;
    }
    // колокольня в OSM — чаще `building=yes` + `man_made=tower`, и назначение у
    // неё видно только по типу башни
    if is_sacred_tower_type(tags) {
        return BuildingUse::Church(sacred(tags));
    }
    match tags.get("amenity").map(String::as_str) {
        Some("place_of_worship") => BuildingUse::Church(sacred(tags)),
        Some(
            "school" | "hospital" | "clinic" | "university" | "college" | "kindergarten" | "police"
            | "fire_station" | "townhall" | "courthouse" | "library" | "theatre"
            | "community_centre",
        ) => BuildingUse::Public,
        _ => BuildingUse::Other,
    }
}

/// Крупноформатная торговля по `shop=*` — белый список, как у путей, водотоков
/// и оград, и по той же причине: `shop` в OSM это добрая сотня значений, и
/// почти все они — **точка внутри чужого дома**. Булочная, цветы, табак и
/// «продукты» на первом этаже девятиэтажки не делают её магазином; ТЦ,
/// супермаркет, универмаг, строительный и мебельный — делают, там здание и
/// есть магазин.
///
/// Тула на кэше v14: 69 элементов с `shop=*` на контуре здания (67 way и 2
/// relation — «Дом Лента» и ТРЦ «Макси»), из них 35 проходят этот список (17
/// `mall`, 7 `supermarket`, 4 `doityourself`, 3 `department_store`, 2
/// `furniture`, `hardware`, `car`), остальные 34 — мелочь во встройке.
/// Бампить `QUERY_VERSION` не надо: `out geom` отдаёт все теги элемента, и
/// `shop` лежит в каждом кэше с первой версии.
fn is_big_format_shop(tags: &HashMap<String, String>) -> bool {
    matches!(
        tags.get("shop").map(String::as_str),
        Some(
            "mall"
                | "supermarket"
                | "department_store"
                | "wholesale"
                | "doityourself"
                | "hardware"
                | "trade"
                | "garden_centre"
                | "furniture"
                | "car"
        )
    )
}

/// Башня храма по `tower:type` — колокольня или минарет.
fn is_sacred_tower_type(tags: &HashMap<String, String>) -> bool {
    matches!(
        tags.get("tower:type").map(String::as_str),
        Some("bell_tower" | "minaret")
    )
}

/// Храм по тегам: вероисповедание и форма части.
fn sacred(tags: &HashMap<String, String>) -> Sacred {
    Sacred {
        faith: faith(tags),
        form: sacred_form(tags),
        complex: 0,
        floor_dm: (part_floor(tags) * 10.0).round() as u16,
    }
}

/// С какой высоты часть здания начинается, м: `min_height`, иначе
/// `building:min_level` × высота этажа; ноль — от земли. Зажато в пределы
/// правдоподобной высоты здания.
fn part_floor(tags: &HashMap<String, String>) -> f32 {
    let floor = tags
        .get("min_height")
        .and_then(|value| parse_measure(value))
        .or_else(|| {
            tags.get("building:min_level")
                .and_then(|value| parse_measure(value))
                .map(|levels| levels * STOREY_HEIGHT)
        })
        .unwrap_or(0.0);
    if floor.is_finite() {
        floor.clamp(0.0, *BUILDING_HEIGHT_RANGE.end())
    } else {
        0.0
    }
}

/// Вероисповедание по `religion` и `denomination`, а где их нет — по самому
/// `building`: мечеть и синагога называют себя тегом здания чаще, чем
/// религией. Христианский храм без деноминации остаётся [`Faith::Unknown`] —
/// в Туле это православный храм, в Берлине кирха, и решает за него город
/// (`parse::resolve_faiths`), а не словарь.
fn faith(tags: &HashMap<String, String>) -> Faith {
    let denomination = tags.get("denomination").map(String::as_str);
    match tags.get("religion").map(String::as_str) {
        Some("christian") => match denomination {
            Some(
                "orthodox" | "russian_orthodox" | "greek_orthodox" | "serbian_orthodox"
                | "romanian_orthodox" | "bulgarian_orthodox" | "georgian_orthodox"
                | "ukrainian_orthodox" | "old_believers" | "armenian_apostolic" | "coptic"
                | "syriac_orthodox" | "ethiopian_orthodox" | "oriental_orthodox",
            ) => Faith::Orthodox,
            None | Some("christian" | "unknown") => Faith::Unknown,
            // католики, протестанты всех ветвей и всё, чего нет в словаре
            // православных: с воздуха это кирпич и шпиль
            Some(_) => Faith::Western,
        },
        Some("muslim") => Faith::Muslim,
        Some("jewish") => Faith::Jewish,
        Some("buddhist" | "hindu" | "shinto" | "taoist" | "jain" | "sikh" | "confucian") => {
            Faith::Eastern
        }
        _ => match tags.get("building").map(String::as_str) {
            Some("mosque" | "minaret") => Faith::Muslim,
            Some("synagogue") => Faith::Jewish,
            Some("shrine" | "temple") => Faith::Eastern,
            Some("campanile") => Faith::Western,
            _ => Faith::Unknown,
        },
    }
}

/// Какая это часть храма: башня, барабан под главой или сам храм.
fn sacred_form(tags: &HashMap<String, String>) -> SacredForm {
    let tower = is_sacred_tower_type(tags)
        || matches!(
            tags.get("building").map(String::as_str),
            Some("bell_tower" | "campanile" | "minaret")
        );
    if tower {
        return SacredForm::Tower;
    }
    match tags.get("roof:shape").map(String::as_str) {
        Some("onion" | "dome") => SacredForm::Dome,
        _ => SacredForm::Nave,
    }
}

/// Ниже этого `building=wall` — ограда, а не крепостная стена, м.
const FORTRESS_WALL_MIN_HEIGHT: f32 = 6.0;

/// Крепостное сооружение: кремлёвская стена, её башни и ворота.
///
/// `historic=citywalls|castle|city_gate|fort` в OSM стоит далеко не всегда: у
/// Тульского кремля его нет ни на одном контуре. Стена там размечена
/// `building=wall` (12.7 м), а башни — `man_made=tower` +
/// `tower:type=defensive`, и по одному `historic` кремль рисовался бы
/// многоэтажками с окнами. `building=wall` пониже [`FORTRESS_WALL_MIN_HEIGHT`]
/// — садовая ограда, кремлём она не становится.
fn is_fortification(tags: &HashMap<String, String>) -> bool {
    let historic = tags.get("historic").map(String::as_str);
    if matches!(
        historic,
        Some("citywalls" | "castle" | "city_gate" | "fort")
    ) {
        return true;
    }
    if tags.get("barrier").map(String::as_str) == Some("city_wall") {
        return true;
    }
    if tags.get("man_made").map(String::as_str) == Some("tower")
        && tags.get("tower:type").map(String::as_str) == Some("defensive")
    {
        return true;
    }
    tags.get("building").map(String::as_str) == Some("wall")
        && tagged_height(tags).is_some_and(|height| height >= FORTRESS_WALL_MIN_HEIGHT)
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
    // (`building=*` + `amenity=parking`) сюда не доходит: здание выше
    if tags.get("amenity").map(String::as_str) == Some("parking")
        && !tags
            .get("parking")
            .is_some_and(|value| HIDDEN_PARKING.contains(&value.as_str()))
    {
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

/// Высота торгового зала, м, — вместо жилого [`STOREY_HEIGHT`], и прибавка на
/// техэтаж с парапетом над верхним залом. `building:levels=1` у гипермаркета
/// значит «один торговый уровень», а не «дом в три метра»: на фотографии у
/// одноуровневого «Магнита» до парапета все восемь, и именно поэтому
/// стотридцатиметровая коробка читалась гигантским одноэтажным жилым домом.
///
/// Прибавка — не «запас», а вещь: над залом лежит техэтаж с вентиляцией, а по
/// верху идёт парапет, за которым прячется оборудование, и на нём же висит
/// вывеска. С этими числами: 1 уровень → 8 м («Магнит»), 2 → 12.5 (ТРЦ
/// «Макси»), 3 → 17 (ТЦ «Сарафан»).
const BIG_BOX_LEVEL_HEIGHT: f32 = 4.5;
const BIG_BOX_SHELL_EXTRA: f32 = 3.5;

/// Ниже одного торгового уровня тега нет: `building:levels=0` — это брошенная
/// разметка, а не коробка, и без нижней границы прибавка оболочки
/// ([`BIG_BOX_SHELL_EXTRA`]) подняла бы этот ноль до правдоподобных 3.5 м и
/// отобрала бы у дома выведенную высоту (`heights.rs`, `BIG_BOX_HEIGHTS`) —
/// то есть нарисовала бы гипермаркет плитой в один рост.
///
/// **Верхней границы здесь нет, и это не пропуск.** Она одна на всю коробку и
/// стоит там же, где пятно, — [`BIG_BOX_MAX_LEVELS`]: четвёртый этаж делает
/// дом не коробкой, а многоэтажным ТЦ или жилым корпусом с магазином внизу, и
/// мерить его торговым уровнем нельзя. До этого порог этажности стоял только
/// тут, и девять этажей с `shop=supermarket` («Пятёрочка» на первом этаже
/// панельного дома, размеченная на весь дом) мерились жилым этажом — но
/// кассету, ярус и решётку фонарей всё равно получали.
const BIG_BOX_MIN_LEVELS: f32 = 1.0;

/// Высота здания в метрах: `height` как есть, иначе этажи
/// (`building:levels` + `roof:levels`, второй по схеме S3DB в первый не входит)
/// по [`STOREY_HEIGHT`]. Оба тега разом почти не встречаются, так что это не
/// «уточнение», а две независимые ветки данных.
///
/// **Метраж этажа — не константа, а свойство здания**: у торговой коробки
/// уровень выше жилого этажа ([`BIG_BOX_LEVEL_HEIGHT`]). Отсюда и контур в
/// аргументах — крупноформатность в OSM не размечают, её видно только по
/// пятну ([`is_big_box_shape`] — половина правила
/// [`crate::map::osm::model::is_big_box`], записанная один раз), и мерить
/// торговым уровнем «Дикси» во встройке было бы такой же ложью, как мерить
/// жилым этажом гипермаркет.
///
/// Вторая половина — этажность [`BIG_BOX_MAX_LEVELS`] и потолок
/// [`BIG_BOX_MAX_HEIGHT`], и тут они **те же**, что у предиката: дом, у
/// которого этажей больше трёх или оболочка вышла бы выше потолка, меряется
/// жилым этажом. Так ответы разбора и предиката сходятся по построению — то,
/// что посчитано торговым уровнем, и есть то, что рисуется коробкой.
pub(super) fn building_height(
    tags: &HashMap<String, String>,
    class: BuildingUse,
    outer: &[Vec2],
) -> Option<f32> {
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
    // оболочка: столько вышло бы у коробки с этими уровнями. Этажей больше
    // трёх или оболочка выше потолка — дом уже не коробка ([`is_big_box`]
    // скажет то же самое про этажи и посчитанную высоту), и меряется он
    // обычным жилым этажом. Потолок тут не лишний: он ловит `roof:levels`,
    // которого этажный порог не считает
    let shell = (levels + roof_levels) * BIG_BOX_LEVEL_HEIGHT + BIG_BOX_SHELL_EXTRA;
    if is_big_box_shape(class, outer)
        && (BIG_BOX_MIN_LEVELS..=BIG_BOX_MAX_LEVELS).contains(&levels)
        && shell <= BIG_BOX_MAX_HEIGHT
    {
        return plausible(shell);
    }
    plausible((levels + roof_levels) * STOREY_HEIGHT)
}

/// Высота по одним тегам — для [`is_fortification`], где контура ещё нет и
/// торговой коробки быть не может: `building=wall` в шесть метров. Пустой срез
/// здесь безопасен не по порядку вычисления, а по устройству
/// [`is_big_box_shape`], которая короткое кольцо отвергает сама.
fn tagged_height(tags: &HashMap<String, String>) -> Option<f32> {
    building_height(tags, BuildingUse::Other, &[])
}

/// Ширина по классу, род ленты и класс по значению highway; `None` — дорогу
/// не рисуем.
///
/// Ширина здесь — **номинальная**, до сечений: у всего, кроме дорожек, её
/// пересчитывает из числа полос проход сечений
/// (`map::roads::network::sections`), первый в доводке разбора. Съезды
/// (`*_link`) долго выбрасывались целиком — словарь их не знал, и въезд на
/// мост в Туле (22 way `primary_link`) обрывался пустым местом.
pub(super) fn road_class(highway: &str) -> Option<(f32, RoadClass, Highway)> {
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
pub(super) fn fence_kind(tags: &HashMap<String, String>) -> Option<FenceKind> {
    match tags.get("barrier").map(String::as_str)? {
        "fence" => Some(FenceKind::Fence),
        "wall" | "retaining_wall" => Some(FenceKind::Wall),
        "hedge" => Some(FenceKind::Hedge),
        _ => None,
    }
}

/// Высота имеет смысл только у зданий: у пруда и газона её не бывает даже при
/// случайно проставленном теге.
///
/// Контур нужен затем же, зачем назначение: этаж считается в метрах
/// по-разному, и у торговой коробки метраж этажа решает её размер
/// ([`building_height`]).
///
/// **Класс передаётся, а не выводится заново.** Вызывающий строит
/// `PolyArea::building_use` тем же [`area_use`] в том же выражении, а у
/// мультиполигона высота считается по каждому внешнему кольцу — так что
/// собственный вывод был бы вторым прогоном цепочки `building`/`shop`/`amenity`
/// на здание и третьим на кольцо. И, что важнее прогонов: класс, которым дом
/// **рисуется**, и класс, которым меряется его **высота**, обязаны быть одним
/// значением, а не двумя совпадающими.
pub(super) fn area_height(
    kind: AreaKind,
    tags: &HashMap<String, String>,
    building_use: BuildingUse,
    outer: &[Vec2],
) -> Option<f32> {
    matches!(kind, AreaKind::Building | AreaKind::Kremlin)
        .then(|| building_height(tags, building_use, outer))
        .flatten()
}

/// Этажи по разметке — `building:levels` как есть, только у зданий, как и
/// высота. В `PolyArea::storeys` они едут отдельно от высоты, потому что
/// высота их уже не помнит: и четырёхэтажный ТЦ, и двухуровневая коробка
/// выходят в двенадцать с небольшим метров, а коробка из них только вторая
/// (`model::is_big_box`).
///
/// `roof:levels` сюда **не** прибавляется: по схеме S3DB это этажи, спрятанные
/// в кровлю, и на вопрос «сколько этажей насчитал маппер» отвечает
/// `building:levels`. Высоте они нужны оба, и складывает их
/// [`building_height`] у себя.
///
/// Правдоподобность не проверяется — в отличие от высоты, где диапазон
/// отсекает `height=0` и опечатки на порядок. Здесь отсекать нечего: ноль
/// этажей это брошенная разметка, которая и так не мешает (высоту такому дому
/// выводит `buildings/heights.rs`), а нелепо большое число отвечает на вопрос
/// предиката ровно тем, чем надо, — «не коробка».
pub(super) fn area_storeys(kind: AreaKind, tags: &HashMap<String, String>) -> Option<f32> {
    matches!(kind, AreaKind::Building | AreaKind::Kremlin)
        .then(|| {
            tags.get("building:levels")
                .and_then(|value| parse_measure(value))
        })
        .flatten()
}

/// Цвета здания из `building:colour` и `roof:colour` — только у зданий, как и
/// высота. Тула: 30 и 91 тега на 7.7 тысячи домов, но среди них — золото глав
/// кремлёвского собора (`#FFD700`) и бирюза Всехсвятского (`#5D948F`), а это
/// ровно то, чем храм узнают.
pub(super) fn area_colours(kind: AreaKind, tags: &HashMap<String, String>) -> Colours {
    if !matches!(kind, AreaKind::Building | AreaKind::Kremlin) {
        return Colours::default();
    }
    let read = |key: &str| tags.get(key).and_then(|value| colour(value));
    Colours {
        wall: read("building:colour"),
        roof: read("roof:colour"),
    }
}

/// Цвет из значения тега: `#rgb`, `#rrggbb` или имя из CSS — те, что в OSM
/// пишут руками. Незнакомое имя — `None`, а не серый: лучше палитра по посеву,
/// чем угаданный цвет.
///
/// Hex берётся как есть — его мапер подбирал по фотографии. **Имя — краской
/// карты**, а не значением CSS: `blue` у мапера значит «синяя кровля», и
/// `#0000FF` на карте был бы маркером, а не краской; тона в [`CSS_COLOURS`]
/// взяты из палитр кровель и стен (`buildings/temples.rs`, `material.rs`).
pub(super) fn colour(value: &str) -> Option<Rgb> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let digit = |at: usize| {
            hex.as_bytes()
                .get(at)
                .and_then(|byte| (*byte as char).to_digit(16))
        };
        return match hex.len() {
            3 => Some([
                digit(0)? as u8 * 17,
                digit(1)? as u8 * 17,
                digit(2)? as u8 * 17,
            ]),
            6 => Some([
                (digit(0)? * 16 + digit(1)?) as u8,
                (digit(2)? * 16 + digit(3)?) as u8,
                (digit(4)? * 16 + digit(5)?) as u8,
            ]),
            _ => None,
        };
    }
    let name = value.to_ascii_lowercase();
    CSS_COLOURS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, rgb)| *rgb)
}

/// Имена цветов, встречающиеся в `building:colour` / `roof:colour`, краской
/// карты (см. [`colour`]): `white` — побелка, а не `#FFFFFF`, `blue` — синее
/// железо кровли, `darkgray` темнее `gray`, как его и пишут, а не светлее, как
/// в CSS.
const CSS_COLOURS: [(&str, Rgb); 37] = [
    ("white", [236, 234, 229]),
    ("black", [46, 46, 48]),
    ("red", [168, 58, 50]),
    ("darkred", [122, 40, 36]),
    ("maroon", [110, 34, 38]),
    ("green", [82, 133, 107]),
    ("darkgreen", [56, 96, 72]),
    ("lime", [120, 170, 80]),
    ("olive", [128, 124, 70]),
    ("blue", [87, 107, 138]),
    ("darkblue", [56, 72, 118]),
    ("navy", [50, 62, 100]),
    ("lightblue", [150, 180, 205]),
    ("skyblue", [130, 175, 215]),
    ("cyan", [90, 168, 172]),
    ("teal", [66, 132, 132]),
    ("yellow", [222, 196, 84]),
    ("gold", [219, 168, 61]),
    ("orange", [214, 140, 70]),
    ("brown", [140, 90, 62]),
    ("saddlebrown", [120, 72, 40]),
    ("tan", [200, 172, 132]),
    ("beige", [226, 216, 190]),
    ("ivory", [240, 238, 226]),
    ("cream", [240, 235, 210]),
    ("pink", [220, 170, 170]),
    ("purple", [120, 80, 130]),
    ("violet", [170, 130, 180]),
    ("gray", [128, 128, 128]),
    ("grey", [128, 128, 128]),
    ("darkgray", [100, 100, 102]),
    ("darkgrey", [100, 100, 102]),
    ("lightgray", [190, 190, 190]),
    ("lightgrey", [190, 190, 190]),
    ("silver", [176, 178, 180]),
    ("dimgray", [96, 96, 98]),
    ("dimgrey", [96, 96, 98]),
];
