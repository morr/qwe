//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов. Деревья по разобранной карте сажает
//! `super::planting`.

use std::collections::{HashMap, HashSet};

use bevy::math::Vec2;

use super::planting::plant_trees;
use crate::city::City;
use crate::map::osm::entrances::generate_entrances;
use crate::map::osm::model::{
    AreaKind, BuildingUse, Faith, FenceLine, MapData, PipeLine, PolyArea, RailLine, RoadLine,
    Sacred, SacredForm, Structure, TrafficSide, TreeCompose, TreeNode, TreeRow, TreeRowLayout,
    WallLine, WaterLine, point_in_area, point_in_polygon, ring_area, ring_bounds, signed_ring_area,
};
use crate::map::osm::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};
use crate::map::seed::seed_from_point;

/// Ширина стены Кремля, м.
const WALL_WIDTH: f32 = 3.0;
/// Совпадение концов way при сборке колец, м (общие OSM-узлы дают
/// идентичные координаты; эпсилон страхует от шума проекции).
const RING_JOIN_EPSILON: f32 = 0.01;

/// Шаг квантования координат при привязке входа к зданию, 1/м. Вход в OSM —
/// как правило общий узел контура здания (Тула 82%, Берлин 79%, Париж 65%),
/// так что после одной и той же проекции координаты совпадают точно;
/// сантиметровая сетка — страховка от шума f32, а не поиск ближайшего.
const ENTRANCE_SNAP_SCALE: f32 = 100.0;

pub fn parse(json: &str, city: City) -> Result<MapData, String> {
    let response: OverpassResponse =
        serde_json::from_str(json).map_err(|error| format!("overpass json: {error}"))?;
    let bounds = GeoBounds::for_city(city);

    let mut map = MapData::default();
    match driving_side(&response.elements) {
        Some(side) => map.traffic_side = side,
        // не error: зеркало без областей отдаёт пустой `is_in`, а карта без
        // стороны движения всё равно рисуется
        None => eprintln!("osm parse: no driving_side in the answer, assuming right-hand traffic"),
    }
    let mut skipped_open_rings = 0usize;
    // Overpass отдаёт ноды раньше way, так что здания на этот момент ещё не
    // разобраны: копим входы и раскладываем по домам после цикла
    let mut entrances = Vec::new();

    for element in &response.elements {
        match element.kind.as_str() {
            "node" => {
                if let Some(position) = parse_entrance(element, &bounds) {
                    entrances.push(position);
                }
                if let Some(node) = parse_tree_node(element, &bounds) {
                    map.tree_nodes.push(node);
                }
                if let Some(structure) = parse_structure_node(element, &bounds) {
                    map.structures.push(structure);
                }
            }
            "way" => parse_way(element, &bounds, &mut map),
            "relation" => parse_relation(element, &bounds, &mut map, &mut skipped_open_rings),
            _ => {}
        }
    }

    if skipped_open_rings > 0 {
        // не error: кольца, порванные краем bbox, ожидаемы
        eprintln!("osm parse: {skipped_open_rings} unclosed relation rings skipped");
    }

    let drowned = drop_buildings_in_water(&mut map);
    if drowned > 0 {
        eprintln!("osm parse: {drowned} buildings dropped as standing entirely in water");
    }

    let guessed = resolve_faiths(&mut map.buildings);
    if guessed > 0 {
        eprintln!("osm parse: {guessed} places of worship took their faith from the city");
    }

    let orphaned = attach_entrances(&mut map, &entrances);
    if orphaned > 0 {
        // ожидаемо: вход бывает отдельной нодой у крыльца, а не узлом контура,
        // либо принадлежит зданию, которое не попало в bbox
        eprintln!(
            "osm parse: {orphaned} of {} entrances match no building",
            entrances.len()
        );
    }
    let started = std::time::Instant::now();
    let squared = square_skewed_houses(&mut map);
    if squared > 0 {
        eprintln!(
            "osm parse: {squared} skewed small houses squared into rectangles in {:?}",
            started.elapsed()
        );
    }
    // размеченных дверей в OSM единицы процентов — остальным дом получает свои
    // по замеру когорт, см. `entrances/`
    let started = std::time::Instant::now();
    let generated = generate_entrances(&mut map);
    eprintln!(
        "osm parse: {} entrances attached, {generated} generated in {:?}",
        entrances.len() - orphaned,
        started.elapsed()
    );

    let started = std::time::Instant::now();
    let (standalone, woods, rows, asked) = plant_trees(&map);
    // «посажено меньше, чем запрошено» — лес уперся в насыщение, потолок
    // плотности стоит выше достижимого (см. `planting::TREE_MIN_SPACING`).
    // Аллеи считаются отдельно и под обе политики: `kept` = `slid` означает,
    // что сдвигать было нечего, а `0 in K tree rows` при `K > 0` — что тег
    // доехал, а посадка по нему не встала никуда. Одиночные ноды выбывают
    // штатно — в лесу и у аллей дерево уже посажено процедурно
    let counts: Vec<String> = TreeRowLayout::ALL
        .iter()
        .map(|&layout| rows.get(layout).len().to_string())
        .collect();
    eprintln!(
        "osm parse: {} trees planted of {asked} asked, {} standalone of {} tree nodes, \
         {} in {} tree rows (keep/slide x osm/slider) in {:?}",
        woods.len(),
        standalone.len(),
        map.tree_nodes.len(),
        counts.join("/"),
        map.tree_rows.len(),
        started.elapsed()
    );
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;
    // сборка по составу **по умолчанию**: парсер о панелях ничего не знает,
    // но и отдавать `MapData` с пустым `trees` не должен — иначе каждый читатель
    // обязан помнить про отдельный шаг сборки. Выбранный игроком состав
    // доложит `map::trees::recompose_row_trees`, и только если он другой
    map.compose_trees(TreeCompose::default());
    Ok(map)
}

/// Дома, целиком стоящие в воде, выбрасываются. В OSM это плавучие рестораны и
/// дебаркадеры (`HMS Belfast` в Лондоне, `Café Barge` в Париже), а в Туле —
/// одинокий сарай посреди Верхнего пруда. Навмеш заливает воду непроходимой,
/// так что до дверей такого дома пешка всё равно не дойдёт, а коробка посреди
/// пруда читается как баг рендера.
///
/// Критерий — **все** вершины контура в воде: дом, зацепившийся за берег
/// (пирс, набережная, дом на сваях у кромки), остаётся. Выброшенных единицы:
/// Тула 1, Берлин 6, Нью-Йорк 17, Лондон и Париж по 28, Токио 0.
///
/// Порядок важен: до раскладки входов и посадки деревьев — иначе дом получит
/// двери, а деревья обойдут стороной пустое место.
fn drop_buildings_in_water(map: &mut MapData) -> usize {
    // AABB-прекомпьют: воды десятки полигонов, зданий десятки тысяч, и почти
    // каждое отсеивается на первой же вершине, не доходя до point-in-polygon
    let bounds: Vec<(Vec2, Vec2)> = map
        .water
        .iter()
        .map(|area| ring_bounds(&area.outer))
        .collect();

    let MapData {
        buildings, water, ..
    } = map;
    let before = buildings.len();
    buildings.retain(|building| {
        !building.outer.iter().all(|point| {
            water.iter().zip(&bounds).any(|(area, &(min, max))| {
                point.x >= min.x
                    && point.x <= max.x
                    && point.y >= min.y
                    && point.y <= max.y
                    && point_in_area(*point, area)
            })
        })
    });
    before - buildings.len()
}

/// Как далеко от храма ещё стоит его часть — колокольня рядом, придел, м.
const CHURCH_PART_REACH: f32 = 30.0;

/// Храмы собираются из частей: у каждой — свой **храм** ([`Sacred::complex`])
/// и вера. Возвращает, скольким храмам веру пришлось взять у города.
///
/// OSM рисует собор то одним контуром, то общим контуром и частями
/// (`building:part`, барабаны с `roof:shape=onion`, колокольня рядом), и у
/// частей своих тегов почти нет. В Туле так размечен Успенский собор кремля:
/// вера — на общем контуре, а три главы и колокольня — отдельными контурами
/// без неё, и каждая часть выбирала себе цвет сама — розовый барабан рядом с
/// белым и бирюзовая глава рядом с золотой.
///
/// * **Хозяин части** — самый крупный из храмов крупнее неё, в контуре
///   которого стоит её центр, а если такого нет — ближайший из них не дальше
///   [`CHURCH_PART_REACH`] (колокольня стоит рядом, а не внутри). Храм без
///   хозяина — сам себе храм. Хозяин берётся цепочкой до верха: колокольня,
///   чей ближайший крупный сосед — часть собора, принадлежит собору.
/// * **Посев храма** — от первой вершины верхнего хозяина: им красятся все части.
/// * **Вера** — своя, если размечена; иначе ближайшего по цепочке хозяина с
///   размеченной; иначе большинства храмов
///   города: христианский храм без деноминации в Туле православный, а в Берлине
///   кирха. При равенстве, и в городе без единого размеченного храма, —
///   [`Faith::Western`], самый распространённый в OSM вид церкви.
fn resolve_faiths(buildings: &mut [PolyArea]) -> usize {
    struct Church {
        index: usize,
        faith: Faith,
        area: f32,
        center: Vec2,
        bounds: (Vec2, Vec2),
    }
    let churches: Vec<Church> = buildings
        .iter()
        .enumerate()
        .filter_map(|(index, building)| match building.building_use {
            BuildingUse::Church(sacred) => {
                let bounds = ring_bounds(&building.outer);
                Some(Church {
                    index,
                    faith: sacred.faith,
                    area: ring_area(&building.outer),
                    center: (bounds.0 + bounds.1) * 0.5,
                    bounds,
                })
            }
            _ => None,
        })
        .collect();

    let (orthodox, western) = churches
        .iter()
        .fold((0, 0), |(o, w), church| match church.faith {
            Faith::Orthodox => (o + 1, w),
            Faith::Western => (o, w + 1),
            _ => (o, w),
        });
    let majority = if orthodox > western {
        Faith::Orthodox
    } else {
        Faith::Western
    };

    // хозяин — по снимку до записи: часть не должна стать хозяином по
    // собственной, только что выданной вере
    let hosts: Vec<Option<usize>> = churches
        .iter()
        .map(|part| {
            let larger = churches
                .iter()
                .enumerate()
                .filter(|(_, other)| other.index != part.index && other.area > part.area);
            let inside = larger
                .clone()
                .filter(|(_, other)| {
                    part.center.cmpge(other.bounds.0).all()
                        && part.center.cmple(other.bounds.1).all()
                        && point_in_polygon(part.center, &buildings[other.index].outer)
                })
                .max_by(|a, b| a.1.area.total_cmp(&b.1.area));
            inside.map(|(at, _)| at).or_else(|| {
                larger
                    .map(|(at, other)| {
                        let nearest = part.center.clamp(other.bounds.0, other.bounds.1);
                        (at, nearest.distance(part.center))
                    })
                    .filter(|(_, distance)| *distance <= CHURCH_PART_REACH)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(at, _)| at)
            })
        })
        .collect();

    // хозяин цепочкой: колокольня, ближе всего стоящая к части собора, берёт
    // посев и веру у собора, а не у части. Цепочка конечна — площадь хозяина
    // строго растёт
    let chain = |from: usize| std::iter::successors(Some(from), |&at| hosts[at]);
    let mut guessed = 0;
    for (at, church) in churches.iter().enumerate() {
        let faith = match chain(at)
            .map(|up| churches[up].faith)
            .find(|faith| *faith != Faith::Unknown)
        {
            Some(faith) => faith,
            None => {
                guessed += 1;
                majority
            }
        };
        let root = chain(at).last().unwrap_or(at);
        let anchor = churches[root].index;
        let complex = seed_from_point(buildings[anchor].outer.first().copied().unwrap_or_default());
        if let BuildingUse::Church(sacred) = &mut buildings[church.index].building_use {
            sacred.faith = faith;
            sacred.complex = complex;
        }
    }

    absorb_annexes(buildings);
    guessed
}

/// Во сколько раз пристройка может быть крупнее храма, на котором лежит. Музей
/// оружия в Богоявленском соборе крупнее собора в 1.35 раза — это тот же дом,
/// размеченный вторым контуром; квартал с домовой церковью во дворе крупнее
/// в десятки раз, и храмом он не становится.
const ANNEX_AREA_RATIO: f32 = 2.5;
/// Какую долю своего пятна контур обязан делить с храмом, чтобы стать
/// пристройкой, когда ни один из двух центров не лежит в другом. Алтарная
/// часть Богоявленского собора (20 × 24 м) заходит в собор на половину, а её
/// центр — за его стеной. Дом, лишь примыкающий к храму общей стеной, делит с
/// ним ноль площади и остаётся домом.
const ANNEX_OVERLAP_SHARE: f32 = 0.25;

/// Площадь пересечения двух колец, м².
fn overlap_area(a: &[Vec2], b: &[Vec2]) -> f32 {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;

    let ring = |points: &[Vec2]| -> Vec<[f32; 2]> {
        let mut ring: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
        // i_overlay ждёт единую закрутку, OSM её не гарантирует
        if signed_ring_area(points) < 0.0 {
            ring.reverse();
        }
        ring
    };
    let (a, b) = (vec![ring(a)], vec![ring(b)]);
    a.overlay(&b, OverlayRule::Intersect, FillRule::NonZero)
        .iter()
        .flat_map(|shape| shape.iter().enumerate())
        .map(|(index, contour)| {
            let points: Vec<Vec2> = contour.iter().map(|p| Vec2::new(p[0], p[1])).collect();
            // первый контур фигуры — внешний, остальные — дыры
            let area = ring_area(&points);
            if index == 0 { area } else { -area }
        })
        .sum()
}

/// Контуры без назначения, лежащие **на храме**, становятся его пристройками
/// ([`SacredForm::Annex`]): вера и посев храма, храмовые стены и кровля, своих
/// глав нет.
///
/// OSM рисует храм и тем, чем он стал: в Тульском кремле поверх Богоявленского
/// собора лежит контур «музей оружия» (`building=yes`, три этажа), а рядом —
/// пристройка без тегов, и оба рисовались жилыми коробками с окнами, из-за
/// которых торчали главы собора. Признак пристройки — взаимное наложение:
/// центр контура в храме, центр храма в контуре или общая с храмом доля пятна
/// не меньше [`ANNEX_OVERLAP_SHARE`], при площади не больше
/// [`ANNEX_AREA_RATIO`] храма. Храм, к которому она прирастает, — самый крупный
/// из подходящих.
fn absorb_annexes(buildings: &mut [PolyArea]) {
    // Раунды — потому что пристройка прирастает и к пристройке: алтарная часть
    // Богоявленского собора лежит на контуре музея, а с самим собором делит
    // меньше четверти своего пятна. Отдельно стоящий дом ни с чем не
    // перекрывается, так что цепочка сквозь квартал не растёт.
    for _ in 0..ANNEX_ROUNDS {
        if absorb_round(buildings) == 0 {
            break;
        }
    }
}

/// Сколько раундов прирастания пристроек, не больше.
const ANNEX_ROUNDS: usize = 3;

/// Один раунд [`absorb_annexes`]: хозяева — все храмы и пристройки на его
/// начало. Возвращает, сколько контуров стало пристройками.
fn absorb_round(buildings: &mut [PolyArea]) -> usize {
    let candidates: Vec<(usize, (Vec2, Vec2), f32)> = buildings
        .iter()
        .enumerate()
        .filter(|(_, building)| matches!(building.building_use, BuildingUse::Church(_)))
        .map(|(index, building)| {
            (
                index,
                ring_bounds(&building.outer),
                ring_area(&building.outer),
            )
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut absorbed = 0;
    for index in 0..buildings.len() {
        let building = &buildings[index];
        // только контур без назначения (`building=yes`, `building:part`): дом,
        // школа или магазин с названным классом храмом не становятся, как бы
        // ни лежали
        if building.kind != AreaKind::Building || building.building_use != BuildingUse::Other {
            continue;
        }
        let bounds = ring_bounds(&building.outer);
        let center = (bounds.0 + bounds.1) * 0.5;
        let area = ring_area(&building.outer);
        let host = candidates
            .iter()
            .filter(|(church, (lo, hi), church_area)| {
                let overlap = bounds.0.cmple(*hi).all() && bounds.1.cmpge(*lo).all();
                overlap
                    && area <= church_area * ANNEX_AREA_RATIO
                    && (point_in_polygon(center, &buildings[*church].outer)
                        || point_in_polygon((*lo + *hi) * 0.5, &building.outer)
                        || overlap_area(&building.outer, &buildings[*church].outer)
                            >= area * ANNEX_OVERLAP_SHARE)
            })
            .max_by(|a, b| a.2.total_cmp(&b.2));
        let Some(&(church, ..)) = host else {
            continue;
        };
        let BuildingUse::Church(sacred) = buildings[church].building_use else {
            continue;
        };
        buildings[index].building_use = BuildingUse::Church(Sacred {
            form: SacredForm::Annex,
            floor_dm: 0,
            ..sacred
        });
        absorbed += 1;
    }
    absorbed
}

/// Нода `entrance=*` → позиция на карте. Не вход или значение из
/// [`NON_WALKABLE_ENTRANCES`] — `None`.
fn parse_entrance(element: &Element, bounds: &GeoBounds) -> Option<Vec2> {
    let entrance = element.tags.get("entrance")?.as_str();
    if NON_WALKABLE_ENTRANCES.contains(&entrance) {
        return None;
    }
    Some(bounds.project(element.lat?, element.lon?))
}

/// Нода `man_made=*` → цилиндр промзоны. Контура у ноды нет, поэтому радиус
/// берётся из тега `diameter`, а если и его нет — типовой для рода
/// ([`structure_size`]). В Туле нодами размечены четыре заводские трубы из
/// восьми.
fn parse_structure_node(element: &Element, bounds: &GeoBounds) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let (radius, height) = structure_size(kind);
    Some(Structure {
        at: bounds.project(element.lat?, element.lon?),
        radius: structure_radius(&element.tags).unwrap_or(radius),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
    })
}

/// Way `man_made=*` → тот же цилиндр, но радиус считается по контуру: он
/// заведомо честнее тега, которого в данных обычно и нет.
///
/// Центр — среднее вершин, а не центроид площади: контур цилиндра в OSM
/// рисуют равномерным многоугольником, на нём это одно и то же, а у
/// вытянутого контура (силосный корпус, размеченный прямоугольником) среднее
/// вершин ближе к тому, что глаз считает серединой.
fn parse_structure_way(element: &Element, points: &[Vec2]) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let ring = as_ring(points)?;
    let at = ring.iter().sum::<Vec2>() / ring.len() as f32;
    let spread = ring.iter().map(|point| at.distance(*point)).sum::<f32>() / ring.len() as f32;
    let (_, height) = structure_size(kind);
    Some(Structure {
        at,
        radius: spread.max(f32::EPSILON),
        height: structure_height(&element.tags).unwrap_or(height),
        kind,
    })
}

/// Нода `natural=tree` → одиночное дерево. Сажает его (и отсеивает
/// продублированные процедурной посадкой) `planting::plant_standalone`.
fn parse_tree_node(element: &Element, bounds: &GeoBounds) -> Option<TreeNode> {
    if element.tags.get("natural").map(String::as_str) != Some("tree") {
        return None;
    }
    Some(TreeNode {
        pos: bounds.project(element.lat?, element.lon?),
        radius: crown_radius(&element.tags),
    })
}

/// Раскладка входов по зданиям: вход ищется среди вершин контуров, потому что
/// в OSM он и есть узел контура. Возвращает число входов, не нашедших дом.
///
/// Общий узел двух домов (сплошная застройка) попадает в таблицу один раз —
/// вход достанется одному из них, и это не важно: дверь всё равно там же.
///
/// Совпадающие входы схлопываются: в Париже встречаются две ноды `entrance` в
/// одной точке (замер по выгрузке — минимальный зазор 0.00 м), а две двери на
/// одном месте — это две одинаковых цели для пешек и лишний кружок в оверлее.
fn attach_entrances(map: &mut MapData, entrances: &[Vec2]) -> usize {
    let key = |point: Vec2| {
        (
            (point.x * ENTRANCE_SNAP_SCALE).round() as i32,
            (point.y * ENTRANCE_SNAP_SCALE).round() as i32,
        )
    };

    let mut by_vertex: HashMap<(i32, i32), usize> = HashMap::new();
    for (index, building) in map.buildings.iter().enumerate() {
        for &vertex in &building.outer {
            by_vertex.insert(key(vertex), index);
        }
    }

    let mut orphaned = 0;
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    for &entrance in entrances {
        let Some(&index) = by_vertex.get(&key(entrance)) else {
            orphaned += 1;
            continue;
        };
        // дубль считаем привязанным, а не сиротой: дом он нашёл
        if taken.insert(key(entrance)) {
            map.buildings[index].entrances.push(entrance);
        }
    }
    orphaned
}

/// Какой площади дом, м², ещё выпрямляется в прямоугольник. Тот же порог, что
/// у скатной когорты (`roofs::SMALL_FOOTPRINT_MAX`): частный дом.
const SQUARE_AREA_MAX: f32 = 250.0;
/// Перекос угла от прямого, градусы, с которого контур выпрямляется. Ниже —
/// обводка и так ровная, а сдвиг первой вершины сменил бы дому посев
/// (материал, этажность) ради сантиметров.
const SQUARE_SKEW_MIN: f32 = 2.0;
/// Перекос, выше которого четырёхугольник оставляется как есть: это уже не
/// криво обведённый прямоугольник, а трапеция по участку.
const SQUARE_SKEW_MAX: f32 = 20.0;
/// Дальше этого, м, ни одна вершина не сдвигается — иначе дом наедет на
/// соседа или на дорогу.
const SQUARE_SHIFT_MAX: f32 = 2.5;

/// Маленькие дома, обведённые в OSM **косым четырёхугольником**, выпрямляются в
/// прямоугольник. Сдвиг первой вершины меняет дому посев, поэтому материал и
/// этажность у выпрямленного дома выпадут заново. Возвращает, сколько домов
/// выпрямлено.
///
/// Частный сектор обводят по спутнику на глаз, и прямоугольный сруб выходит
/// ромбом с углами 79°–100° (Тула, way 968419942). В 2.5D такой дом читается
/// кривым: торцы стоят косо к фасаду, а двускатная крыша на нём не встаёт.
///
/// Прямоугольник сохраняет **центроид и площадь**: ось — средняя по
/// направлениям рёбер (угол ×4, взвешенный длиной, так что противоположные и
/// соседние рёбра голосуют за одну ось), стороны — средние длины
/// противоположных рёбер вдоль неё, подогнанные под площадь. Вершина `i`
/// переходит в угол `i`, обход сохраняется — вместе с ней переезжает и
/// размеченный на ней вход.
///
/// Не трогаются дома, у которых хоть одна вершина **общая** с другим контуром
/// или линией (сплошная застройка, забор по стене, арка): выпрямленный, такой
/// дом разошёлся бы с соседом щелью. Порядок в конвейере: после раскладки
/// входов (они ищут дом по точной вершине) и до генерации дверей и посадки
/// деревьев (те должны видеть уже выпрямленный контур).
fn square_skewed_houses(map: &mut MapData) -> usize {
    let key = |point: Vec2| {
        (
            (point.x * ENTRANCE_SNAP_SCALE).round() as i32,
            (point.y * ENTRANCE_SNAP_SCALE).round() as i32,
        )
    };
    let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
    let mut count = |points: &[Vec2]| {
        let unique: HashSet<(i32, i32)> = points.iter().map(|point| key(*point)).collect();
        for vertex in unique {
            *uses.entry(vertex).or_default() += 1;
        }
    };
    for building in &map.buildings {
        count(&building.outer);
        building.holes.iter().for_each(|hole| count(hole));
    }
    map.roads.iter().for_each(|line| count(&line.points));
    map.rails.iter().for_each(|line| count(&line.points));
    map.walls.iter().for_each(|line| count(&line.points));
    map.fences.iter().for_each(|line| count(&line.points));
    map.pipes.iter().for_each(|line| count(&line.points));
    map.water_lines.iter().for_each(|line| count(&line.points));

    let mut squared = 0;
    for building in &mut map.buildings {
        let small_house = building.kind == AreaKind::Building
            && building.holes.is_empty()
            && matches!(
                building.building_use,
                BuildingUse::House | BuildingUse::Other
            )
            && signed_ring_area(&building.outer).abs() <= SQUARE_AREA_MAX;
        let Ok(quad) = <[Vec2; 4]>::try_from(building.outer.as_slice()) else {
            continue;
        };
        if !small_house || quad.iter().any(|vertex| uses[&key(*vertex)] > 1) {
            continue;
        }
        let Some(skew) = quad_skew(&quad) else {
            continue;
        };
        if !(SQUARE_SKEW_MIN..=SQUARE_SKEW_MAX).contains(&skew) {
            continue;
        }
        let rect = fit_rectangle(&quad);
        if quad
            .iter()
            .zip(&rect)
            .any(|(from, to)| from.distance(*to) > SQUARE_SHIFT_MAX)
        {
            continue;
        }
        for entrance in &mut building.entrances {
            if let Some(index) = quad.iter().position(|vertex| vertex == entrance) {
                *entrance = rect[index];
            }
        }
        building.outer = rect.to_vec();
        squared += 1;
    }
    squared
}

/// Наибольшее отклонение угла выпуклого четырёхугольника от прямого, градусы;
/// `None` — четырёхугольник невыпуклый или вырожденный.
fn quad_skew(quad: &[Vec2; 4]) -> Option<f32> {
    let mut sign = 0.0;
    let mut skew = 0.0_f32;
    for index in 0..4 {
        let (prev, at, next) = (quad[(index + 3) % 4], quad[index], quad[(index + 1) % 4]);
        let (back, forth) = (
            (prev - at).normalize_or_zero(),
            (next - at).normalize_or_zero(),
        );
        let turn = (at - prev).perp_dot(next - at);
        if back == Vec2::ZERO || forth == Vec2::ZERO || turn == 0.0 || turn * sign < 0.0 {
            return None;
        }
        sign = turn;
        let angle = back.dot(forth).clamp(-1.0, 1.0).acos().to_degrees();
        skew = skew.max((angle - 90.0).abs());
    }
    Some(skew)
}

/// Прямоугольник с центроидом и площадью четырёхугольника `quad`; угол `i`
/// соответствует его вершине `i`, обход тот же.
fn fit_rectangle(quad: &[Vec2; 4]) -> [Vec2; 4] {
    let edge = |index: usize| quad[(index + 1) % 4] - quad[index];
    // направления с периодом 90°: угол ×4 сводит рёбра обеих осей в одно
    let vote = (0..4).fold(Vec2::ZERO, |sum, index| {
        let e = edge(index);
        sum + Vec2::from_angle(e.to_angle() * 4.0) * e.length()
    });
    let axis = Vec2::from_angle(vote.to_angle() / 4.0);
    // `along` — ось рёбер 0 и 2, `across` — рёбер 1 и 3
    let (along, across) = if edge(0).dot(axis).abs() >= edge(0).dot(axis.perp()).abs() {
        (axis, axis.perp())
    } else {
        (axis.perp(), axis)
    };
    let length = (edge(0).dot(along).abs() + edge(2).dot(along).abs()) / 2.0;
    let width = (edge(1).dot(across).abs() + edge(3).dot(across).abs()) / 2.0;
    // от первой вершины: в метрах карты (тысячи) произведения в f32 теряют
    // сантиметры, а центроиду и площади нужны именно они
    let local = quad.map(|vertex| vertex - quad[0]);
    let area = signed_ring_area(&local).abs();
    let scale = (area / (length * width)).sqrt();
    let side = along * edge(0).dot(along).signum() * length * scale;
    let up = across * edge(1).dot(across).signum() * width * scale;
    let first = quad[0] + ring_centroid(&local) - (side + up) / 2.0;
    [first, first + side, first + side + up, first + up]
}

/// Центроид площади простого кольца.
fn ring_centroid(ring: &[Vec2]) -> Vec2 {
    let mut sum = Vec2::ZERO;
    let mut twice_area = 0.0;
    for index in 0..ring.len() {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        let cross = a.perp_dot(b);
        sum += (a + b) * cross;
        twice_area += cross;
    }
    sum / (3.0 * twice_area)
}

fn project_points(points: &[LatLon], bounds: &GeoBounds) -> Vec<Vec2> {
    points
        .iter()
        .map(|point| bounds.project(point.lat, point.lon))
        .collect()
}

/// Закрытая полилиния way → открытое кольцо (без повторённой последней точки).
fn as_ring(points: &[Vec2]) -> Option<Vec<Vec2>> {
    if points.len() < 4 || points.first() != points.last() {
        return None;
    }
    Some(points[..points.len() - 1].to_vec())
}

fn push_area(map: &mut MapData, area: PolyArea) {
    match area.kind {
        AreaKind::Building | AreaKind::Kremlin => map.buildings.push(area),
        AreaKind::Water => map.water.push(area),
        AreaKind::Park => map.parks.push(area),
        AreaKind::Wood => map.woods.push(area),
        AreaKind::Grass => map.grass.push(area),
        AreaKind::Sand => map.sand.push(area),
        AreaKind::Residential | AreaKind::Industrial => map.landuse.push(area),
        AreaKind::Parking => map.parking.push(area),
        AreaKind::Pitch(_) => map.pitches.push(area),
    }
}

fn parse_way(element: &Element, bounds: &GeoBounds, map: &mut MapData) {
    let Some(geometry) = &element.geometry else {
        return;
    };
    let points = project_points(geometry, bounds);
    if points.len() < 2 {
        return;
    }

    // Рельсы проверяются до дорог и **не** прерывают разбор: трамвайный путь в
    // OSM сплошь и рядом висит на том же way, что и `highway=*`, и такой way
    // обязан стать и улицей, и путём.
    if let Some(railway) = element.tags.get("railway")
        && let Some((width, kind)) = rail_class(railway)
        && !is_underground(&element.tags)
    {
        map.rails.push(RailLine {
            points: points.clone(),
            width,
            kind,
            service: service_track(&element.tags),
        });
    }

    // Надземный трубопровод — теплотрасса на опорах. Тоже до дорог и тоже без
    // `return`: труба идёт эстакадой над улицей, и такой way в OSM носит оба
    // тега — стоя после дорожной ветки, она бы до него не добралась.
    if element.tags.get("man_made").map(String::as_str) == Some("pipeline")
        && let Some(width) = pipe_width(&element.tags)
    {
        map.pipes.push(PipeLine {
            points: points.clone(),
            width,
        });
    }

    // аллея — тоже до дорог и тоже без `return`, по той же причине, что рельсы:
    // ветка не должна затыкаться чужим ранним выходом
    if element.tags.get("natural").map(String::as_str) == Some("tree_row") {
        map.tree_rows.push(TreeRow {
            spacing: row_spacing(&element.tags, &points),
            radius: crown_radius(&element.tags),
            points: points.clone(),
        });
    }

    // водоток — снова до дорог и снова без `return`: ручей в трубе под улицей
    // размечен `waterway=*` на том же way, что и `highway=*`, и обе ветки обязаны
    // отработать. Замкнутый `waterway=riverbank` сюда не попадает — его нет в
    // белом списке, и площадью он остаётся ниже, в `area_kind`.
    if let Some(waterway) = element.tags.get("waterway")
        && let Some((default_width, kind)) = water_class(waterway)
    {
        map.water_lines.push(WaterLine {
            points: points.clone(),
            width: water_width(&element.tags).unwrap_or(default_width),
            kind,
            tunnel: is_underground(&element.tags),
        });
    }

    // ограда участка — снова до дорог и снова без `return`: обнесённый забором
    // квартал в OSM — это один way с `barrier=fence` **и** `landuse=residential`,
    // и он обязан стать и оградой, и кварталом (с `return` здесь Тула теряла три
    // площадки, три стоянки, квартал и парк), а забор вдоль тропы висит на том же
    // way, что и `highway=*`, и ниже дорожной ветки её `return` съедал бы его
    // целиком (в Париже и Лондоне по одному такому way, в остальных пяти городах
    // ни одного). Ограда — не стена: рисуется, но навмеша не касается.
    if let Some(kind) = fence_kind(&element.tags) {
        map.fences.push(FenceLine {
            points: points.clone(),
            kind,
            gates: Vec::new(),
        });
    }

    // подземное не рисуем — то же правило, что у рельсов и водотоков. У дорог
    // оно долго отсутствовало, и подземный переход выходил на карту обычной
    // дорожкой: в Токио так рисовались 1399 way из 12 859 (10.9%), в Лондоне
    // 1985, в Париже 1473, в Берлине 883, в Туле 35. Подавляющее большинство —
    // `highway=footway|steps` метрополитена. Что тег `tunnel` на этом way
    // может описывать вовсе не дорогу — вопрос [`is_road_underground`]
    if let Some(highway) = element.tags.get("highway") {
        let Some((width, class)) = road_class(highway) else {
            return;
        };
        if is_road_underground(&element.tags) {
            return;
        }
        let bridge = element
            .tags
            .get("bridge")
            .is_some_and(|value| value != "no");
        // `oneway=-1` — поток против порядка точек; разворачиваем здесь, чтобы
        // ниже по конвейеру «направление way» и «направление движения» были
        // одним и тем же. Рельс и водоток той же ноды это не касается: их
        // ветки отработали выше и взяли `points` в исходном порядке
        let mut points = points;
        if is_oneway_backward(&element.tags) {
            points.reverse();
        }
        map.roads.push(RoadLine {
            points,
            width,
            class,
            bridge,
            passage: is_building_passage(&element.tags),
            oneway: is_oneway(&element.tags),
            roundabout: is_roundabout(&element.tags),
            lanes: tagged_lanes(&element.tags),
        });
        return;
    }

    if element.tags.get("barrier").map(String::as_str) == Some("city_wall") {
        map.walls.push(WallLine {
            points,
            width: WALL_WIDTH,
        });
        return;
    }

    // Цилиндр промзоны — резервуар, силос, труба, башня. Эта ветка, в
    // отличие от рельсовой и трубопроводной, **прерывает разбор**: труба,
    // размеченная way с
    // `building=yes` (в Туле такая одна из восьми), — тот же самый объект, и
    // коробка под цилиндром была бы им обоим сразу. Заодно у неё не
    // заводятся ни двери, ни навмеш-препятствие, чего трубе и не нужно.
    if let Some(structure) = parse_structure_way(element, &points) {
        map.structures.push(structure);
        return;
    }

    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(outer) = as_ring(&points) else {
        return;
    };
    push_area(
        map,
        PolyArea {
            outer,
            holes: Vec::new(),
            kind,
            building_use: area_use(kind, &element.tags),
            height: area_height(kind, &element.tags),
            entrances: Vec::new(),
        },
    );
}

/// Сторона движения — `driving_side` на административной границе, внутри
/// которой лежит центр карты (запрос `is_in` в `overpass_query`). На дорогах
/// тег почти не ставят: в OSM он живёт на стране и наследуется всем внутри.
///
/// Границ с тегом может прийти несколько (страна и заморская территория,
/// регион-исключение внутри страны) — берётся самая мелкая, с наибольшим
/// `admin_level`. Непонятное значение пропускается, как отсутствие тега.
fn driving_side(elements: &[Element]) -> Option<TrafficSide> {
    elements
        .iter()
        .filter(|element| element.kind == "relation")
        .filter_map(|element| {
            let side = match element.tags.get("driving_side")?.as_str() {
                "right" => TrafficSide::Right,
                "left" => TrafficSide::Left,
                _ => return None,
            };
            let level = element
                .tags
                .get("admin_level")
                .and_then(|level| level.parse::<u8>().ok())
                .unwrap_or(0);
            Some((level, side))
        })
        .max_by_key(|(level, _)| *level)
        .map(|(_, side)| side)
}

fn parse_relation(
    element: &Element,
    bounds: &GeoBounds,
    map: &mut MapData,
    skipped_open_rings: &mut usize,
) {
    let Some(kind) = area_kind(element) else {
        return;
    };
    let Some(members) = &element.members else {
        return;
    };

    let outers = assemble_rings(members, "outer", bounds, skipped_open_rings);
    let inners = assemble_rings(members, "inner", bounds, skipped_open_rings);
    let height = area_height(kind, &element.tags);
    let building_use = area_use(kind, &element.tags);

    for outer in outers {
        let holes = inners
            .iter()
            .filter(|inner| point_in_polygon(inner[0], &outer))
            .cloned()
            .collect();
        push_area(
            map,
            PolyArea {
                outer,
                holes,
                kind,
                building_use,
                height,
                entrances: Vec::new(),
            },
        );
    }
}

/// Сборка замкнутых колец из way-членов relation с заданной ролью:
/// цепочки соединяются по совпадающим концам (с разворотом при
/// необходимости), пока не замкнутся.
fn assemble_rings(
    members: &[Member],
    role: &str,
    bounds: &GeoBounds,
    skipped_open_rings: &mut usize,
) -> Vec<Vec<Vec2>> {
    let mut segments: Vec<Vec<Vec2>> = members
        .iter()
        .filter(|member| member.kind == "way" && member.role == role)
        .filter_map(|member| member.geometry.as_ref())
        .map(|geometry| project_points(geometry, bounds))
        .filter(|points| points.len() >= 2)
        .collect();

    let close = |a: Vec2, b: Vec2| a.distance_squared(b) < RING_JOIN_EPSILON * RING_JOIN_EPSILON;
    let mut rings = Vec::new();

    while let Some(mut ring) = segments.pop() {
        loop {
            if close(ring[0], *ring.last().unwrap()) && ring.len() >= 4 {
                ring.pop();
                rings.push(ring);
                break;
            }

            let tail = *ring.last().unwrap();
            let Some(index) = segments.iter().position(|segment| {
                close(segment[0], tail) || close(*segment.last().unwrap(), tail)
            }) else {
                // не замкнулось (обычно порвано краем bbox): ≥3 точек —
                // насильно замыкаем, иначе выбрасываем
                if ring.len() >= 3 {
                    rings.push(ring);
                } else {
                    *skipped_open_rings += 1;
                }
                break;
            };

            let mut segment = segments.swap_remove(index);
            if !close(segment[0], tail) {
                segment.reverse();
            }
            ring.extend_from_slice(&segment[1..]);
        }
    }
    rings
}

mod tags;
#[cfg(test)]
mod tests;

// Приватный реэкспорт: снаружи модуль виден тем же набором имён, что и до
// разрезания, а `use super::*` в `tests.rs` продолжает доставать классификаторы.
use self::tags::{
    NON_WALKABLE_ENTRANCES, area_height, area_kind, area_use, crown_radius, fence_kind,
    is_building_passage, is_oneway, is_oneway_backward, is_road_underground, is_roundabout,
    is_underground, pipe_width, rail_class, road_class, row_spacing, service_track,
    structure_height, structure_kind, structure_radius, structure_size, tagged_lanes, water_class,
    water_width,
};
