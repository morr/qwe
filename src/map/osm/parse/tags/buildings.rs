//! Читатели тегов **зданий**: назначение, храм, крепость, высота, этажи,
//! цвета. Механизмы — `references/parse-buildings.md`.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{BUILDING_HEIGHT_RANGE, parse_measure};
use crate::map::osm::model::{
    AreaKind, BIG_BOX_MAX_HEIGHT, BIG_BOX_MAX_LEVELS, BuildingUse, Colours, Faith, Rgb, Sacred,
    SacredForm, is_big_box_shape,
};
use crate::settings::STOREY_HEIGHT;

/// Назначение здания по `building=*`. Когда значение вне словаря — прежде
/// всего `yes` (в Туле 4004 из 7465), но и любое другое, скажем
/// `construction`, — смотрится `amenity=*` того же контура: школа
/// или больница в OSM почти всегда `building=yes` + `amenity=school`. Всё,
/// что не в словаре, — [`BuildingUse::Other`]: словарь только для того, что
/// на карте встречается сотнями, а не для полноты OSM-вики.
pub(in crate::map::osm::parse) fn building_use(tags: &HashMap<String, String>) -> BuildingUse {
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
/// (`parse::faiths::resolve_faiths`), а не словарь.
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
pub(super) fn is_fortification(tags: &HashMap<String, String>) -> bool {
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
pub(in crate::map::osm::parse) fn area_use(
    kind: AreaKind,
    tags: &HashMap<String, String>,
) -> BuildingUse {
    if matches!(kind, AreaKind::Building | AreaKind::Kremlin) {
        building_use(tags)
    } else {
        BuildingUse::Other
    }
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
pub(in crate::map::osm::parse) fn building_height(
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
pub(in crate::map::osm::parse) fn area_height(
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
pub(in crate::map::osm::parse) fn area_storeys(
    kind: AreaKind,
    tags: &HashMap<String, String>,
) -> Option<f32> {
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
pub(in crate::map::osm::parse) fn area_colours(
    kind: AreaKind,
    tags: &HashMap<String, String>,
) -> Colours {
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
pub(in crate::map::osm::parse) fn colour(value: &str) -> Option<Rgb> {
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
