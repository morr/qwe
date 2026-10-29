use super::super::fixtures::{CENTER, CITY, HALF, corners, tags};
use super::*;
use crate::map::osm::fixture::{Overpass, closed, square};
use crate::map::osm::model::{
    AreaKind, BuildingUse, Colours, Faith, Sacred, SacredForm, is_big_box,
};
use bevy::math::Vec2;

/// Храм, чья вера досталась ему от города без размеченных храмов.
const WESTERN_CHURCH: BuildingUse = BuildingUse::Church(Sacred {
    faith: Faith::Western,
    form: SacredForm::Nave,
    complex: 0,
    floor_dm: 0,
});

#[test]
fn measures_parse_with_units_commas_and_feet() {
    assert_eq!(parse_measure("12"), Some(12.0));
    assert_eq!(parse_measure("12.5"), Some(12.5));
    assert_eq!(parse_measure("12,5"), Some(12.5));
    assert_eq!(parse_measure("12 m"), Some(12.0));
    assert_eq!(parse_measure("12.5 metres"), Some(12.5));
    // несколько значений — берём первое
    assert_eq!(parse_measure("3;4"), Some(3.0));
    assert_eq!(parse_measure("40'"), Some(12.192));
    let inches = parse_measure("40'6\"").unwrap();
    assert!((inches - 12.3444).abs() < 1e-3, "{inches}");
    assert_eq!(parse_measure("tall"), None);
    assert_eq!(parse_measure(""), None);
}

/// Ширина дорожки — по тегу `width` (зажатому, с отбраковкой пометок), без
/// него — по виду и покрытию; мостик держит ширину класса (Тула, витрина 19:
/// тропинка `width=1 surface=mud` была 3.5-метровым бульваром).
#[test]
fn a_path_takes_its_width_from_the_tag_or_its_kind() {
    let (sw, se, ..) = corners(HALF);
    let width = |pairs: &[(&str, &str)]| {
        Overpass::new(CITY)
            .way(pairs, vec![sw, se])
            .parse()
            .roads
            .remove(0)
            .width
    };
    let footway = ("highway", "footway");
    assert_eq!(width(&[footway, ("width", "1"), ("surface", "mud")]), 1.0);
    assert_eq!(width(&[footway, ("width", "2.55")]), 2.55);
    assert_eq!(width(&[footway, ("width", "0.5")]), 1.0, "зажата до метра");
    assert_eq!(width(&[footway, ("width", "0")]), 3.0, "пометка — по виду");
    assert_eq!(width(&[footway, ("width", "40")]), 3.0, "габарит площади");
    assert_eq!(width(&[footway]), 3.0);
    assert_eq!(width(&[footway, ("footway", "sidewalk")]), 2.5);
    assert_eq!(width(&[footway, ("surface", "ground")]), 1.5);
    assert_eq!(width(&[("highway", "path")]), 2.0);
    assert_eq!(width(&[("highway", "path"), ("surface", "dirt")]), 1.5);
    assert_eq!(width(&[("highway", "pedestrian")]), 5.0);
    assert_eq!(width(&[("highway", "steps")]), 2.5);
    assert_eq!(width(&[("highway", "track")]), 3.0);
    assert_eq!(
        width(&[footway, ("width", "1"), ("bridge", "yes")]),
        3.5,
        "мостик — коридор навмеша, его ширина — класса"
    );
}

/// Высота по одним тегам: дом без назначения и без контура — та ветка,
/// которая у [`building_height`] была единственной до торговой коробки.
fn height(pairs: &[(&str, &str)]) -> Option<f32> {
    building_height(&tags(pairs), BuildingUse::Other, &[])
}

/// Высота торгового здания с пятном: квадрат такой площади вокруг начала
/// координат. Назначение берётся из самих тегов, как в разборе.
fn retail_height(pairs: &[(&str, &str)], area: f32) -> Option<f32> {
    let side = area.sqrt() / 2.0;
    let ring = [
        Vec2::new(-side, -side),
        Vec2::new(side, -side),
        Vec2::new(side, side),
        Vec2::new(-side, side),
    ];
    let tags = tags(pairs);
    building_height(&tags, super::area_use(AreaKind::Building, &tags), &ring)
}

#[test]
fn height_prefers_the_metric_tag_then_falls_back_to_levels() {
    // ветка Нью-Йорка: метры из LiDAR-импорта
    assert_eq!(height(&[("height", "31.4")]), Some(31.4));
    // ветка Европы: этажи
    assert_eq!(height(&[("building:levels", "5")]), Some(15.0));
    // roof:levels по схеме S3DB в building:levels не входит
    assert_eq!(
        height(&[("building:levels", "5"), ("roof:levels", "1")]),
        Some(18.0)
    );
    // проставлены оба — верим метрам, а не пересчёту
    assert_eq!(
        height(&[("height", "20"), ("building:levels", "5")]),
        Some(20.0)
    );
    assert_eq!(height(&[("building", "yes")]), None);
}

#[test]
fn implausible_heights_are_treated_as_missing() {
    assert_eq!(height(&[("height", "0")]), None);
    assert_eq!(height(&[("height", "12000")]), None);
    assert_eq!(height(&[("building:levels", "0")]), None);
    assert_eq!(height(&[("building:levels", "-1")]), None);
    // мусор в метрах не должен глушить этажи
    assert_eq!(
        height(&[("height", "9999"), ("building:levels", "4")]),
        Some(12.0)
    );
    // и у торговой коробки тоже: прибавка оболочки подняла бы ноль уровней до
    // правдоподобных 3.5 м, то есть нарисовала бы гипермаркет плитой в один
    // рост вместо того, чтобы отдать его выводу этажности
    assert_eq!(
        retail_height(&[("building", "retail"), ("building:levels", "0")], 9055.0),
        None
    );
    assert_eq!(
        retail_height(&[("building", "retail"), ("building:levels", "-1")], 9055.0),
        None
    );
}

/// Торговый уровень выше жилого этажа, и сверх него идёт техэтаж с парапетом:
/// `building:levels=1` у гипермаркета — это не трёхметровый дом. Ровно из-за
/// этого стодвадцатиметровый «Магнит» (`building=commercial`, `shop=supermarket`,
/// 9055 м², один уровень) читался гигантским одноэтажным жилым домом.
#[test]
fn a_trading_level_is_taller_than_a_dwelling_storey() {
    let magnit = &[
        ("building", "commercial"),
        ("shop", "supermarket"),
        ("building:levels", "1"),
    ];
    assert_eq!(retail_height(magnit, 9055.0), Some(8.0));
    // ТРЦ «Макси»: `building=yes` + `shop=mall`, два уровня
    let maxi = &[
        ("building", "yes"),
        ("shop", "mall"),
        ("building:levels", "2"),
    ];
    assert_eq!(retail_height(maxi, 64257.0), Some(12.5));
    // та же разметка на мелком пятне — магазин во встройке, обычный этаж
    assert_eq!(retail_height(maxi, 295.0), Some(6.0));
}

/// `shop=*` на девятиэтажке описывает не здание, а магазин на его первом
/// этаже: тульская «Пятёрочка» размечена `building=retail` + `levels=9` на
/// весь дом, и торговым уровнем это вышло бы сорокапятиметровой башней.
#[test]
fn a_shop_on_a_tall_block_is_measured_in_dwelling_storeys() {
    let pyaterochka = &[
        ("building", "retail"),
        ("shop", "supermarket"),
        ("building:levels", "9"),
    ];
    assert_eq!(retail_height(pyaterochka, 1528.0), Some(27.0));
}

/// И этим дело не кончается: дом, которому разбор отказал в торговом уровне,
/// не коробка и в отрисовке. Порог этажности стоял только в высоте, а всё
/// остальное — глухая кассета с фризом, ярус 5.5 м, решётка зенитных фонарей и
/// входные группы через 55 м — висело на одном пятне, и девятиэтажная
/// «Пятёрочка» получала их все. Два ответа на один вопрос сходятся теперь по
/// построению, и проверяются они вместе — через весь разбор, а не по тегам.
#[test]
fn a_shop_on_a_tall_block_is_not_a_big_box_either() {
    let block = |levels: &str| {
        Overpass::new(CITY)
            .area(
                &[
                    ("building", "retail"),
                    ("shop", "supermarket"),
                    ("building:levels", levels),
                ],
                square(CENTER, HALF),
            )
            .parse()
            .buildings
            .remove(0)
    };

    let tall = block("9");
    assert_eq!(tall.height, Some(27.0));
    assert!(!is_big_box(&tall), "девятиэтажка с магазином — не коробка");

    // тот же дом в два торговых уровня — коробка, и мерена она оболочкой
    let shell = block("2");
    assert_eq!(shell.height, Some(12.5));
    assert!(is_big_box(&shell));
}

/// А высотой этого не решить, и потому этажность доезжает до модели отдельным
/// полем. ТЦ «Империя» (`building=yes` + `shop=mall`, 1781 м², четыре этажа)
/// меряется жилым этажом и стоит в двенадцати метрах; ТРЦ «Макси»
/// (`building=yes` + `shop=mall`, 64 257 м², два торговых уровня) — в
/// двенадцати с половиной. Любой потолок по `area.height` либо оставит
/// коробкой четырёхэтажный ТЦ, либо отнимет коробку у настоящей: между ними
/// полметра, и различает их только `building:levels`.
#[test]
fn four_storeys_of_mall_are_not_a_two_level_box() {
    // 42 × 42 м — 1781 м² «Империи», пятно крупноформата с запасом над 1200
    let mall = |levels: &str| {
        Overpass::new(CITY)
            .area(
                &[
                    ("building", "yes"),
                    ("shop", "mall"),
                    ("building:levels", levels),
                ],
                square(CENTER, 21.1),
            )
            .parse()
            .buildings
            .remove(0)
    };

    let imperia = mall("4");
    assert_eq!(imperia.storeys, Some(4.0));
    assert_eq!(imperia.height, Some(12.0));
    assert!(!is_big_box(&imperia), "четырёхэтажный ТЦ — не коробка");

    // «Макси»: те же двенадцать метров с небольшим, но это два торговых уровня
    let maxi = mall("2");
    assert_eq!(maxi.storeys, Some(2.0));
    assert_eq!(maxi.height, Some(12.5));
    assert!(is_big_box(&maxi), "двухуровневая коробка — коробка");

    // этажей не разметили — ответ прежний, по потолку высоты («Верный»,
    // ТЦ «Перспектива»: пятно есть, тегов высоты нет вовсе)
    let untagged = Overpass::new(CITY)
        .area(
            &[("building", "retail"), ("shop", "supermarket")],
            square(CENTER, 21.1),
        )
        .parse()
        .buildings
        .remove(0);
    assert_eq!(untagged.storeys, None);
    assert_eq!(untagged.height, None);
    assert!(is_big_box(&untagged));
}

/// Высота по одним тегам приходит без контура (`tagged_height` для
/// `is_fortification`), и пустое кольцо не должно ронять разбор: площадь
/// кольца считает `len() - 1` по `usize`. Раньше срез спасал только порядок
/// сомножителей в `&&`.
#[test]
fn an_empty_ring_is_not_a_big_box_and_does_not_panic() {
    let retail = tags(&[("building", "retail"), ("building:levels", "2")]);
    assert_eq!(
        building_height(&retail, BuildingUse::Retail, &[]),
        Some(6.0)
    );
}

/// Высота доезжает до `MapData` и из way, и из relation, а на воде её нет.
#[test]
fn parsed_areas_carry_building_height_only() {
    let (sw, se, ne, _) = corners(HALF);
    let map = Overpass::new(CITY)
        .area(
            &[("building", "yes"), ("building:levels", "9")],
            square(CENTER, HALF),
        )
        .relation(
            &[("building", "yes"), ("height", "42 m")],
            &[("outer", closed(vec![sw, se, ne]))],
        )
        .area(
            &[("natural", "water"), ("height", "5")],
            square(CENTER, HALF),
        )
        .parse();

    assert_eq!(map.buildings.len(), 2);
    let heights: Vec<Option<f32>> = map
        .buildings
        .iter()
        .map(|building| building.height)
        .collect();
    assert!(heights.contains(&Some(27.0)), "{heights:?}");
    assert!(heights.contains(&Some(42.0)), "{heights:?}");
    assert_eq!(map.water.len(), 1);
    assert_eq!(map.water[0].height, None);
}

/// Кремлёвские постройки — те же здания, но своего вида: их красят иначе.
#[test]
fn kremlin_buildings_classified_by_historic_tag() {
    let map = Overpass::new(CITY)
        .area(
            &[("building", "yes"), ("historic", "citywalls")],
            square(CENTER, HALF),
        )
        .parse();

    assert_eq!(map.buildings[0].kind, AreaKind::Kremlin);
}

/// Назначение здания — класс отрисовки: `building=*` напрямую, а когда его
/// значение вне словаря (безликое `yes`, но и `construction`) — по
/// `amenity=*` того же контура. Вода назначения не имеет, даже если тег
/// на ней стоит.
#[test]
fn building_use_comes_from_the_building_tag_or_amenity_outside_the_vocabulary() {
    let map = Overpass::new(CITY)
        .area(&[("building", "house")], square(CENTER, HALF))
        .area(&[("building", "apartments")], square(CENTER, HALF))
        .area(&[("building", "garages")], square(CENTER, HALF))
        .area(&[("building", "garage")], square(CENTER, HALF))
        .area(
            &[("building", "yes"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "construction"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("amenity", "place_of_worship")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "church"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        // в стороне: лёжа на храме, контур без назначения стал бы его пристройкой
        .area(
            &[("building", "yes")],
            square(CENTER + Vec2::new(400.0, 0.0), HALF),
        )
        .area(
            &[("natural", "water"), ("amenity", "school")],
            square(CENTER, HALF),
        )
        .parse();

    // посев храма зависит от вершин контура — здесь сравниваются только классы
    let uses: Vec<BuildingUse> = map
        .buildings
        .iter()
        .map(|b| match b.building_use {
            BuildingUse::Church(sacred) => BuildingUse::Church(Sacred {
                complex: 0,
                ..sacred
            }),
            other => other,
        })
        .collect();
    assert_eq!(
        uses,
        [
            BuildingUse::House,
            BuildingUse::Apartments,
            // множественное число — кооператив целиком, единственное — бокс
            BuildingUse::GarageBlock,
            BuildingUse::Garage,
            BuildingUse::Public,
            // значение вне словаря отдаёт слово `amenity` так же, как `yes`
            BuildingUse::Public,
            // вера не размечена ни у одного храма города — западная
            // (`parse::faiths::resolve_faiths`)
            WESTERN_CHURCH,
            // `building=*` со смыслом сильнее `amenity`
            WESTERN_CHURCH,
            BuildingUse::Other,
        ]
    );
    assert_eq!(map.water[0].building_use, BuildingUse::Other);
}

/// Здание, которое **и есть магазин**, узнаётся по `building=*` и, когда там
/// безликое `yes`, по крупноформатному `shop=*` того же контура. Мелкая
/// торговля по белому списку не проходит: булочная и «продукты» в OSM стоят
/// на чужом доме, и назначение у него своё.
///
/// Без ветки `shop` половина тульских ТЦ (ТРЦ «Макси» — `building=yes` +
/// `shop=mall`, 64 тысячи м²) разбиралась как «полгорода без назначения».
///
/// `building=shop` стоит в торговой ветке, а не в конторской: в OSM это прямой
/// синоним `building=retail`. Значение конкретное, поэтому мелкий `shop=*` на
/// том же контуре его уже не перебивает — уточнять белому списку нечего.
#[test]
fn a_shop_building_is_read_from_the_shop_tag_too() {
    let map = Overpass::new(CITY)
        .area(&[("building", "retail")], square(CENTER, HALF))
        .area(&[("building", "supermarket")], square(CENTER, HALF))
        // `building=shop` — то же самое здание-магазин, что и `retail`
        .area(&[("building", "shop")], square(CENTER, HALF))
        .area(
            &[("building", "shop"), ("shop", "convenience")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "mall")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "doityourself")],
            square(CENTER, HALF),
        )
        // `commercial` — «коммерческое здание вообще», и `shop` его уточняет:
        // так размечен «Магнит»
        .area(
            &[("building", "commercial"), ("shop", "supermarket")],
            square(CENTER, HALF),
        )
        // …а `apartments` называет другой дом, и супермаркет на его первом
        // этаже этого не отменяет: так размечена тульская «Пятёрочка»
        .area(
            &[("building", "apartments"), ("shop", "supermarket")],
            square(CENTER, HALF),
        )
        // булочная на чужом доме магазином его не делает
        .area(
            &[("building", "apartments"), ("shop", "bakery")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("shop", "convenience")],
            square(CENTER, HALF),
        )
        // контора и павильон остаются торговлей вообще, а не магазином
        .area(&[("building", "commercial")], square(CENTER, HALF))
        .area(&[("building", "office")], square(CENTER, HALF))
        .parse();

    let uses: Vec<BuildingUse> = map.buildings.iter().map(|b| b.building_use).collect();
    assert_eq!(
        uses,
        [
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Retail,
            BuildingUse::Apartments,
            BuildingUse::Apartments,
            BuildingUse::Other,
            BuildingUse::Commercial,
            BuildingUse::Commercial,
        ]
    );
}

/// Крепость узнаётся не только по `historic`: у Тульского кремля стена —
/// `building=wall`, башни — `man_made=tower` с `tower:type=defensive`. Низкая
/// `building=wall` — ограда, и кремлём не становится.
#[test]
fn a_fortress_is_told_by_its_wall_and_its_defensive_towers() {
    let map = Overpass::new(CITY)
        .area(
            &[("building", "wall"), ("height", "12.7")],
            square(CENTER, HALF),
        )
        .area(
            &[
                ("building", "yes"),
                ("man_made", "tower"),
                ("tower:type", "defensive"),
            ],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("historic", "citywalls")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "wall"), ("height", "2")],
            square(CENTER, HALF),
        )
        .area(
            &[("building", "yes"), ("man_made", "tower")],
            square(CENTER, HALF),
        )
        .parse();
    let kinds: Vec<AreaKind> = map.buildings.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            AreaKind::Kremlin,
            AreaKind::Kremlin,
            AreaKind::Kremlin,
            AreaKind::Building,
            AreaKind::Building,
        ]
    );
}

/// Цвета из разметки доезжают до дома: hex — как есть, имя — краской карты,
/// незнакомое имя — пусто, а у воды цветов не бывает и с тегом.
#[test]
fn tagged_colours_reach_the_building() {
    let map = Overpass::new(CITY)
        .area(
            &[
                ("building", "church"),
                ("building:colour", "#FFD700"),
                ("roof:colour", "Blue"),
            ],
            square(CENTER, 10.0),
        )
        .area(
            &[("building", "yes"), ("roof:colour", "#abc")],
            square(CENTER + Vec2::new(60.0, 0.0), 10.0),
        )
        .area(
            &[("building", "yes"), ("building:colour", "chartreuse-ish")],
            square(CENTER + Vec2::new(120.0, 0.0), 10.0),
        )
        .area(
            &[("natural", "water"), ("building:colour", "red")],
            square(CENTER + Vec2::new(0.0, 120.0), 10.0),
        )
        .parse();
    assert_eq!(
        map.buildings[0].colours,
        Colours {
            wall: Some([255, 215, 0]),
            roof: colour("blue"),
        }
    );
    assert_eq!(map.buildings[1].colours.roof, Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(map.buildings[2].colours, Colours::default());
    assert_eq!(map.water[0].colours, Colours::default());
    assert_eq!(colour("#GGG"), None);
    assert_eq!(colour("#12345"), None);
    assert_eq!(colour("dimgrey"), colour("dimgray"));
    assert_ne!(
        colour("blue"),
        Some([0, 0, 255]),
        "имя — краска карты, не CSS"
    );
}

// --- шов между чтением элементов и доводкой ---------------------------------
//
// Тесты, которые зовут **один проход**, а не конвейер целиком. До шва так никто
// не делал: `parse` была одним телом на сто двадцать пять строк, у стадии не
// было имени, и каждый тест шёл через `Overpass::…parse()`, адресуя дома по их
// месту в фикстуре.
