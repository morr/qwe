//! Парсинг ответа Overpass в [`MapData`]: проекция, классификация по тегам,
//! сборка колец мультиполигонов. Деревья по разобранной карте сажает
//! `super::planting`.

use std::collections::{HashMap, HashSet};

use bevy::math::Vec2;

use super::planting::plant_trees;
use crate::city::City;
use crate::map::grid::Grid;
use crate::map::osm::entrances::generate_entrances;
use crate::map::osm::model::{
    AreaKind, BuildingUse, Faith, FenceLine, MapData, PipeLine, PolyArea, RailLine, RoadLine,
    Sacred, SacredForm, Structure, TrafficSide, TreeCompose, TreeNode, TreeRow, TreeRowLayout,
    WallLine, WaterLine, closest_on_segment, point_in_area, point_in_polygon, ring_area,
    ring_bounds, ring_vertex_mean, signed_ring_area,
};
use crate::map::osm::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};
use crate::map::roads::{is_carriageway, sidewalk_width};
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

/// Разбор — две половины, и между ними шов.
///
/// Первая читает элементы Overpass в сырую [`MapData`] ([`read_elements`]),
/// вторая гоняет по ней доводочные проходы в их единственно верном порядке
/// ([`finish_parse`]). До шва обе лежали одним телом на сто двадцать пять
/// строк, из которых сорок пять были телеметрией, и позвать проход в одиночку
/// было не то чтобы нельзя — просто не за что было взяться: у стадии не было
/// имени. Все шестьдесят тестов разбора поэтому гоняли конвейер целиком и
/// адресовали дома по их месту в фикстуре.
pub fn parse(json: &str, city: City) -> Result<MapData, String> {
    let response: OverpassResponse =
        serde_json::from_str(json).map_err(|error| format!("overpass json: {error}"))?;
    let bounds = GeoBounds::for_city(city);

    let (mut map, entrances, read) = read_elements(&response, &bounds);
    eprint!("{read}");
    let passes = finish_parse(&mut map, &entrances);
    eprint!("{passes}");
    Ok(map)
}

/// Что сказал элементный цикл — значением, а не двумя `eprintln!`.
struct ReadReport {
    /// `None` — зеркало не отдало `is_in`; карта рисуется правосторонней.
    traffic_side: Option<TrafficSide>,
    unclosed_rings: usize,
}

impl std::fmt::Display for ReadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // разбор по полям, а не `self.…`: подстановка тогда читается по имени,
        // а не счётом позиций — как у всех прочих отчётов `map/*`
        let Self {
            traffic_side,
            unclosed_rings,
        } = self;
        if traffic_side.is_none() {
            // не error: зеркало без областей отдаёт пустой `is_in`, а карта без
            // стороны движения всё равно рисуется
            writeln!(
                f,
                "osm parse: no driving_side in the answer, assuming right-hand traffic"
            )?;
        }
        if *unclosed_rings > 0 {
            // не error: кольца, порванные краем bbox, ожидаемы
            writeln!(
                f,
                "osm parse: {unclosed_rings} unclosed relation rings skipped"
            )?;
        }
        Ok(())
    }
}

/// Элементы Overpass — в сырую `MapData` и список входов, которые ещё некуда
/// положить: Overpass отдаёт ноды раньше way, так что на момент разбора ноды
/// зданий ещё нет.
///
/// **Ничего не доводит.** Дома ещё стоят в воде, храмы без веры, контуры
/// косые, дверей нет, деревья не посажены — всё это [`finish_parse`].
fn read_elements(
    response: &OverpassResponse,
    bounds: &GeoBounds,
) -> (MapData, Vec<Vec2>, ReadReport) {
    let mut map = MapData::default();
    let traffic_side = driving_side(&response.elements);
    if let Some(side) = traffic_side {
        map.traffic_side = side;
    }
    let mut unclosed_rings = 0usize;
    let mut entrances = Vec::new();

    for element in &response.elements {
        match element.kind.as_str() {
            "node" => {
                if let Some(position) = parse_entrance(element, bounds) {
                    entrances.push(position);
                }
                if let Some(node) = parse_tree_node(element, bounds) {
                    map.tree_nodes.push(node);
                }
                if let Some(structure) = parse_structure_node(element, bounds) {
                    map.structures.push(structure);
                }
            }
            "way" => parse_way(element, bounds, &mut map),
            "relation" => parse_relation(element, bounds, &mut map, &mut unclosed_rings),
            _ => {}
        }
    }

    let report = ReadReport {
        traffic_side,
        unclosed_rings,
    };
    (map, entrances, report)
}

/// Что дала посадка — то, чем была самая длинная строка лога.
struct PlantedReport {
    woods: usize,
    standalone: usize,
    tree_nodes: usize,
    /// По одной на политику размещения аллей, в порядке [`TreeRowLayout::ALL`]
    /// — массивом той же длины, а не `Vec`: «столько же и в том же порядке»
    /// держит тип, а не эта строчка.
    rows: [usize; TreeRowLayout::ALL.len()],
    tree_rows: usize,
    asked: usize,
}

/// Что сделали доводочные проходы — значением, а не восемью `eprintln!`.
///
/// Те же счётчики, что уходили в лог, но теперь их можно сравнить в тесте: до
/// этого единственным способом узнать, сколько домов отодвинулось от
/// тротуаров, было прочесть строку на stderr.
struct PassReport {
    drowned: usize,
    faiths_guessed: usize,
    entrances_found: usize,
    entrances_orphaned: usize,
    squared: usize,
    squaring: std::time::Duration,
    pulled: PulledHouses,
    pulling: std::time::Duration,
    stretched: usize,
    stretching: std::time::Duration,
    generated: usize,
    generating: std::time::Duration,
    planted: PlantedReport,
    planting: std::time::Duration,
}

impl std::fmt::Display for PassReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // разбор по полям, а не `self.…`: в строке посадки шесть подстановок, и
        // по именам они читаются, а по позициям — только счётом
        let Self {
            drowned,
            faiths_guessed,
            entrances_found,
            entrances_orphaned,
            squared,
            squaring,
            pulled,
            pulling,
            stretched,
            stretching,
            generated,
            generating,
            planted,
            planting,
        } = self;
        if *drowned > 0 {
            writeln!(
                f,
                "osm parse: {drowned} buildings dropped as standing entirely in water"
            )?;
        }
        if *faiths_guessed > 0 {
            writeln!(
                f,
                "osm parse: {faiths_guessed} places of worship took their faith from the city"
            )?;
        }
        if *entrances_orphaned > 0 {
            // ожидаемо: вход бывает отдельной нодой у крыльца, а не узлом
            // контура, либо принадлежит зданию, которое не попало в bbox
            writeln!(
                f,
                "osm parse: {entrances_orphaned} of {entrances_found} entrances match no building"
            )?;
        }
        if *squared > 0 {
            writeln!(
                f,
                "osm parse: {squared} skewed small houses squared into rectangles and L shapes in {squaring:?}"
            )?;
        }
        let PulledHouses {
            moved,
            partly,
            left,
        } = pulled;
        writeln!(
            f,
            "osm parse: {moved} buildings pulled off the sidewalks ({partly} of them only part of the way), {left} left standing on them, in {pulling:?}"
        )?;
        writeln!(
            f,
            "osm parse: {stretched} block vertices pulled to the drawn road edge in {stretching:?}"
        )?;
        let attached = entrances_found - entrances_orphaned;
        writeln!(
            f,
            "osm parse: {attached} entrances attached, {generated} generated in {generating:?}"
        )?;
        // «посажено меньше, чем запрошено» — лес упёрся в насыщение, потолок
        // плотности стоит выше достижимого (см. `planting::TREE_MIN_SPACING`).
        // Аллеи считаются отдельно и под обе политики: `kept` = `slid`
        // означает, что сдвигать было нечего, а `0 in K tree rows` при `K > 0`
        // — что тег доехал, а посадка по нему не встала никуда. Одиночные ноды
        // выбывают штатно — в лесу и у аллей дерево уже посажено процедурно
        let PlantedReport {
            woods,
            standalone,
            tree_nodes,
            rows,
            tree_rows,
            asked,
        } = planted;
        let counts = rows
            .iter()
            .map(usize::to_string)
            .collect::<Vec<String>>()
            .join("/");
        writeln!(
            f,
            "osm parse: {woods} trees planted of {asked} asked, {standalone} standalone of \
             {tree_nodes} tree nodes, {counts} in {tree_rows} tree rows \
             (keep/slide x osm/slider) in {planting:?}"
        )
    }
}

/// Доводочные проходы по сырой карте — восемь, плюс сборка деревьев в конце, —
/// **и этот порядок и есть их интерфейс**. До этой функции он жил заметками в
/// трёх doc-комментариях из восьми и не был записан целиком нигде.
///
/// Почему именно так, сверху вниз:
///
/// 1. **Утопленники** уходят первыми: дом, целиком стоящий в воде, не должен
///    получить ни веры, ни двери, ни выпрямленного контура — всё это работа
///    по дому, которого не будет.
/// 2. **Вера** — до дверей и до выпрямления: она собирает храм из частей
///    (`resolve_faiths` зовёт `absorb_annexes` внутри себя), а часть,
///    ставшая приделом, дальше читается иначе.
/// 3. **Разметанные двери** прикладываются к контурам, пока те ещё сырые: у
///    входа координата **ноды**, а не вершины, и ищется он по тому же
///    сантиметровому ключу.
/// 4. **Выпрямление косых домиков** — после разметки дверей (уже
///    приложенную дверь перенос контура уносит с собой по тому же ключу; точное
///    `==` молча оставило бы её на месте) и до генерации дверей и посадки
///    деревьев (тем нужен уже выпрямленный контур).
/// 5. **Отодвигание домов от тротуаров** — после выпрямления (косой дом
///    сначала становится прямым, потом отъезжает) и до всего, что читает
///    контур.
/// 6. **Подтягивание кварталов к дорогам** — где угодно в хвосте: `landuse` не
///    трогает ни навмеш, ни двери, ни посадку, ни машины. Стоит здесь, потому
///    что дома к этому моменту уже на своих местах.
/// 7. **Генерация дверей** — по уже окончательным контурам: дверь ставится по
///    стене того дома, который останется на карте.
/// 8. **Посадка деревьев** — тоже по окончательным контурам: дерево обходит
///    дом там, где дом стоит после выпрямления и сдвига.
/// 9. **Сборка деревьев по составу по умолчанию** (`compose_trees`) — только
///    после посадки и после того, как её три набора легли в `map`: парсер о
///    панелях ничего не знает, но и отдавать `MapData` с пустым `trees` не
///    должен — иначе каждый читатель обязан помнить про отдельный шаг сборки.
///    Выбранный игроком состав доложит `map::trees::recompose_row_trees`, и
///    только если он другой.
///
/// **`vertex_uses` считается дважды, и это не расточительство.** Шаги 4 и 5
/// оба спрашивают «эта вершина общая?», и между ними контуры **двигаются**:
/// выпрямленный дом уносит свои вершины на новые места, и счёт, снятый до
/// него, отвечал бы про старую карту.
fn finish_parse(map: &mut MapData, entrances: &[Vec2]) -> PassReport {
    let drowned = drop_buildings_in_water(map);
    let faiths_guessed = resolve_faiths(&mut map.buildings);
    let entrances_orphaned = attach_entrances(map, entrances);

    let started = std::time::Instant::now();
    let squared = square_skewed_houses(map);
    let squaring = started.elapsed();

    let started = std::time::Instant::now();
    let pulled = pull_houses_off_sidewalks(map);
    let pulling = started.elapsed();

    let started = std::time::Instant::now();
    let stretched = pull_landuse_to_roads(map);
    let stretching = started.elapsed();

    // размеченных дверей в OSM единицы процентов — остальным дом получает свои
    // по замеру когорт, см. `entrances/`
    let started = std::time::Instant::now();
    let generated = generate_entrances(map);
    let generating = started.elapsed();

    let started = std::time::Instant::now();
    let (standalone, woods, rows, asked) = plant_trees(map);
    let planting = started.elapsed();
    let planted = PlantedReport {
        woods: woods.len(),
        standalone: standalone.len(),
        tree_nodes: map.tree_nodes.len(),
        rows: TreeRowLayout::ALL.map(|layout| rows.get(layout).len()),
        tree_rows: map.tree_rows.len(),
        asked,
    };
    map.standalone_trees = standalone;
    map.wood_trees = woods;
    map.row_trees = rows;
    // шаг 9, довод — в списке над функцией
    map.compose_trees(TreeCompose::default());

    PassReport {
        drowned,
        faiths_guessed,
        entrances_found: entrances.len(),
        entrances_orphaned,
        squared,
        squaring,
        pulled,
        pulling,
        stretched,
        stretching,
        generated,
        generating,
        planted,
        planting,
    }
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
/// Место в конвейере — **шаг 1** [`finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
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
/// Центр — [`ring_vertex_mean`], а не центроид площади
/// ([`ring_area_centroid`]): контур цилиндра в OSM рисуют равномерным
/// многоугольником, на нём это одно и то же, а у вытянутого контура (силосный
/// корпус, размеченный прямоугольником) среднее вершин ближе к тому, что глаз
/// считает серединой.
fn parse_structure_way(element: &Element, points: &[Vec2]) -> Option<Structure> {
    let kind = structure_kind(&element.tags)?;
    let ring = as_ring(points)?;
    let at = ring_vertex_mean(&ring)?;
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
    let mut by_vertex: HashMap<(i32, i32), usize> = HashMap::new();
    for (index, building) in map.buildings.iter().enumerate() {
        for &vertex in &building.outer {
            by_vertex.insert(vertex_key(vertex), index);
        }
    }

    let mut orphaned = 0;
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    for &entrance in entrances {
        let Some(&index) = by_vertex.get(&vertex_key(entrance)) else {
            orphaned += 1;
            continue;
        };
        // дубль считаем привязанным, а не сиротой: дом он нашёл
        if taken.insert(vertex_key(entrance)) {
            map.buildings[index].entrances.push(entrance);
        }
    }
    orphaned
}

/// Какой площади здание без класса, м², ещё выпрямляется в прямоугольник. Тот же
/// порог, что у скатной когорты (`roofs::SMALL_FOOTPRINT_MAX`): частный дом.
const SQUARE_AREA_MAX: f32 = 250.0;
/// Какой площади `building=house`, м², ещё выпрямляется. Тег уже сказал, что это
/// частный дом, и в скатную когорту он входит любого размера; большой дом частного
/// сектора обводят так же на глаз (Тула, way 968378335: 348 м², перекос 21°).
const SQUARE_HOUSE_AREA_MAX: f32 = 400.0;
/// Перекос угла от прямого, градусы, с которого контур выпрямляется. Ниже —
/// обводка и так ровная, а сдвиг первой вершины сменил бы дому посев
/// (материал, этажность) ради сантиметров.
const SQUARE_SKEW_MIN: f32 = 2.0;
/// Перекос, выше которого четырёхугольник оставляется как есть: это уже не
/// криво обведённый прямоугольник, а трапеция по участку. 20° не хватало: в
/// частном секторе Тулы (ways 968378337, 968166712, 968378341) обводки уходят на
/// 20–32°, и все одинокие маленькие дома с перекосом больше 20° — такие же
/// кривые срубы, а не трапеции; вершины при этом сдвигаются меньше 2 м.
const SQUARE_SKEW_MAX: f32 = 35.0;
/// Дальше этого, м, ни одна вершина не сдвигается — иначе дом наедет на
/// соседа или на дорогу. 2.5 м оставляли кривым один дом Тулы (way 968378327,
/// перекос 27°, сдвиг 2.84 м) и больше ни одного.
const SQUARE_SHIFT_MAX: f32 = 3.0;
/// На какую долю площадь выпрямленной Г может разойтись с обводкой. Уровни стен
/// площадь не держат; больше этого — стены разъехались так, что «Г» уже
/// другой дом (вырожденная полка, вывернутый угол).
const ELL_AREA_DRIFT: f32 = 0.15;

/// Маленькие дома, обведённые в OSM **косым четырёхугольником** или **кривой
/// буквой Г**, выпрямляются в прямоугольник или в Г из прямых углов. Сдвиг
/// первой вершины меняет дому посев, поэтому материал и этажность у
/// выпрямленного дома выпадут заново. Возвращает, сколько домов выпрямлено.
///
/// Частный сектор обводят по спутнику на глаз, и прямоугольный сруб выходит
/// ромбом с углами 79°–100° (Тула, way 968419942). В 2.5D такой дом читается
/// кривым: торцы стоят косо к фасаду, а двускатная крыша на нём не встаёт.
/// Дом с пристройкой обводят так же — шестиугольником с одним вогнутым углом и
/// косыми стенами (ways 968378349, 968378329).
///
/// Прямоугольник сохраняет **центроид и площадь**: ось — средняя по
/// направлениям рёбер (угол ×4, взвешенный длиной, так что противоположные и
/// соседние рёбра голосуют за одну ось), стороны — средние длины
/// противоположных рёбер вдоль неё, подогнанные под площадь. Г строится в той же
/// оси по уровням стен (`fit_ell`) и площадь держит лишь приблизительно
/// (`ELL_AREA_DRIFT`). Вершина `i` переходит в угол `i`, обход сохраняется —
/// вместе с ней переезжает и размеченный на ней вход.
///
/// Не трогаются дома, у которых хоть одна вершина **общая** с другим контуром
/// или линией (сплошная застройка, забор по стене, арка): выпрямленный, такой
/// дом разошёлся бы с соседом щелью.
///
/// Место в конвейере — **шаг 4** [`finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
fn square_skewed_houses(map: &mut MapData) -> usize {
    let uses = vertex_uses(map);

    let mut squared = 0;
    for building in &mut map.buildings {
        let small_house = building.kind == AreaKind::Building
            && building.holes.is_empty()
            && match building.building_use {
                BuildingUse::House => Some(SQUARE_HOUSE_AREA_MAX),
                BuildingUse::Other => Some(SQUARE_AREA_MAX),
                _ => None,
            }
            .is_some_and(|max| signed_ring_area(&building.outer).abs() <= max);
        let outline = &building.outer;
        if !small_house
            || !matches!(outline.len(), 4 | 6)
            || outline.iter().any(|vertex| uses[&vertex_key(*vertex)] > 1)
        {
            continue;
        }
        // прямоугольник — без вогнутых углов, Г — ровно с одним
        let Some((skew, reflex)) = corner_skew(outline) else {
            continue;
        };
        if reflex != (outline.len() - 4) / 2 || !(SQUARE_SKEW_MIN..=SQUARE_SKEW_MAX).contains(&skew)
        {
            continue;
        }
        let fitted = match <[Vec2; 4]>::try_from(outline.as_slice()) {
            Ok(quad) => fit_rectangle(&quad).to_vec(),
            Err(_) => {
                let Some(ell) = fit_ell(outline).filter(|ell| {
                    let area = |ring: &[Vec2]| {
                        signed_ring_area(&ring.iter().map(|p| *p - ring[0]).collect::<Vec<_>>())
                    };
                    corner_skew(ell).is_some_and(|(_, found)| found == 1)
                        && (area(ell) / area(outline) - 1.0).abs() <= ELL_AREA_DRIFT
                }) else {
                    continue;
                };
                ell
            }
        };
        if outline
            .iter()
            .zip(&fitted)
            .any(|(from, to)| from.distance(*to) > SQUARE_SHIFT_MAX)
        {
            continue;
        }
        // тем же ключом, что и привязка: в `entrances` лежит координата ноды
        // входа, а не вершины контура, и точное `==` теряло бы дверь ровно в
        // том случае, ради которого сетка и заведена, — дом уезжает, дверь нет
        for entrance in &mut building.entrances {
            let at = vertex_key(*entrance);
            if let Some(index) = outline.iter().position(|vertex| vertex_key(*vertex) == at) {
                *entrance = fitted[index];
            }
        }
        building.outer = fitted;
        squared += 1;
    }
    squared
}

/// Ключ вершины на сантиметровой сетке: общий OSM-узел двух контуров, линий или
/// входа после одной проекции даёт один и тот же ключ. Одна привязка на всех —
/// [`attach_entrances`] ищет дом этим же ключом (см. [`ENTRANCE_SNAP_SCALE`]).
fn vertex_key(point: Vec2) -> (i32, i32) {
    (
        (point.x * ENTRANCE_SNAP_SCALE).round() as i32,
        (point.y * ENTRANCE_SNAP_SCALE).round() as i32,
    )
}

/// Сколько контуров и линий карты проходит через каждую вершину: `> 1` —
/// вершина общая (сплошная застройка, забор по стене, арка, тропа до угла).
///
/// Считаются здания, все линейные слои и **три площадных из восьми** —
/// [`MapData::parking`], [`MapData::pitches`], [`MapData::water`]. Перечень
/// явный, а не «все контуры карты», потому что решает тут не модель, а кадр:
/// край этих трёх нарисован собственной поверхностью с разметкой, а на стоянке
/// ещё и машинами (`cars::fill_lots`), так что дом, отъехавший от него, виден.
/// `parks`, `grass`, `woods`, `sand` не считаются — дом лежит поверх заливки,
/// и шва под ним не видно; `landuse` не считается по решению коммита 174ab8a
/// (частные дома сплошь и рядом обведены по границе квартала), и правило выше
/// его расширяет, а не спорит с ним.
fn vertex_uses(map: &MapData) -> HashMap<(i32, i32), u32> {
    let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
    let mut count = |points: &[Vec2]| {
        let unique: HashSet<(i32, i32)> = points.iter().map(|point| vertex_key(*point)).collect();
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
    for layer in [&map.parking, &map.pitches, &map.water] {
        for area in layer {
            count(&area.outer);
            area.holes.iter().for_each(|hole| count(hole));
        }
    }
    uses
}

/// Зазор между стеной и внешним краем нарисованного тротуара, м. Треть метра
/// оставляла дом вплотную к тротуару, а в 2.5D крыша ещё и наклоняется к
/// дороге на полметра-метр.
const SIDEWALK_CLEARANCE: f32 = 2.0;
/// Дальше этого, м, дом от улицы не отодвигается: дом, которому нужно больше,
/// сдвигается на этот предел и остаётся краем на тротуаре. 4 м оставляли на
/// месте целые ряды Тулы (улица Громова: дому 17 не хватило сантиметра).
const SIDEWALK_SHIFT_MAX: f32 = 6.0;
/// Ближе этого, м, сдвинутый дом не подходит к другому объекту — зданию,
/// линии (от её края), водоёму, цилиндру, — если до сдвига стоял дальше.
const SHIFT_CLEARANCE: f32 = 0.5;
/// Доли сдвига по очереди: полный упёрся — дом сдвигается на меньший.
const SHIFT_FRACTIONS: [f32; 4] = [1.0, 0.75, 0.5, 0.25];
/// Разница отступов от улицы, м, при которой соседи ещё стоят «на одной линии»
/// и сдвигаются вместе.
const ROW_SETBACK_TOLERANCE: f32 = 2.0;
/// Промежуток между габаритами соседей по ряду, м.
const ROW_GAP: f32 = 20.0;
/// Раундов сдвига: угловой дом у двух улиц отталкивается от каждой по очереди.
const SIDEWALK_SHIFT_ROUNDS: usize = 4;
/// Остаток наезда, м, который после раундов считается нулём.
const SIDEWALK_SHIFT_TOLERANCE: f32 = 0.05;
/// Ячейка сетки отрезков улиц, м.
const SIDEWALK_CELL: f32 = 32.0;

/// Звено линии карты и то расстояние от его оси, дальше которого оно не
/// достаёт. Один тип на все проходы этого файла, потому что вся геометрия у
/// них одна — «отрезок плюс радиус вокруг него», — а вот **чем** измерена
/// `reach`, решает тот, кто звено сложил: полоса улицы с тротуаром и зазором
/// ([`pull_houses_off_sidewalks`]), внешний край нарисованного полотна
/// ([`pull_landuse_to_roads`]), радиус запрета ([`Obstacles`]).
#[derive(Clone, Copy)]
struct Link {
    from: Vec2,
    to: Vec2,
    reach: f32,
}

/// Дома, контур которых **наезжает на нарисованный тротуар** улицы, сдвигаются
/// от неё целиком — внутрь квартала. Возвращает, сколько домов сдвинуто и
/// сколько наезжающих оставлено как есть.
///
/// Ширина улицы в модели — константа класса, а тротуар добавляет рендер
/// (`roads::sidewalk_width`), и в старой застройке дом, стоящий в OSM у самой
/// кромки, выходит стеной на тротуар, а в 2.5D крышей — на асфальт (Тула,
/// way 179102449 у улицы Бундурина: стена в 4.7 м от оси при 5.76 м полосы).
/// Точность до метра игре не нужна, а дом на тротуаре читается как баг.
///
/// Сдвиг — перенос всего контура вместе с входами, по кратчайшему направлению
/// от оси улицы. **Двигается ряд, а не дом**: соседи по одной стороне одной
/// улицы (габариты ближе [`ROW_GAP`]), чей отступ не больше чем на
/// [`ROW_SETBACK_TOLERANCE`] отличается от отступа самого наезжающего дома
/// ряда, отодвигаются на тот же сдвиг — иначе линия фасадов ломается
/// ступенькой. Дом, которому нужно больше [`SIDEWALK_SHIFT_MAX`], в ряд не
/// встаёт — иначе он оставил бы на месте всех соседей — и сдвигается на
/// предел. Потом каждый дом добирает остаток наезда от других улиц раундами до
/// [`SIDEWALK_SHIFT_ROUNDS`], не дальше того же предела. Сдвиг, после
/// которого дом подходит к другому объекту ближе [`SHIFT_CLEARANCE`]
/// ([`Obstacles`]), укорачивается по [`SHIFT_FRACTIONS`]; не помогла и
/// четверть — дом остаётся. Дом остаётся на месте и тогда, когда улица
/// проходит сквозь контур. Не трогаются, как и в [`square_skewed_houses`],
/// дома с общей вершиной — сплошная застройка разошлась бы щелью, арка
/// потеряла бы проход, — крепость и храмы (части храма стоят друг на друге).
/// Сдвиг первой вершины меняет дому посев: материал и этажность выпадут заново.
///
/// Место в конвейере — **шаг 5** [`finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
fn pull_houses_off_sidewalks(map: &mut MapData) -> PulledHouses {
    // `reach` — полуширина улицы с тротуаром и зазором; рядом индекс улицы
    let mut segments: Vec<Link> = Vec::new();
    let mut segment_road: Vec<usize> = Vec::new();
    for (index, road) in map.roads.iter().enumerate() {
        if road.bridge || !is_carriageway(road) {
            continue;
        }
        let Some(sidewalk) = sidewalk_width(road.width) else {
            continue;
        };
        let reach = road.width / 2.0 + sidewalk + SIDEWALK_CLEARANCE;
        for link in road.points.windows(2) {
            segments.push(Link {
                from: link[0],
                to: link[1],
                reach,
            });
            segment_road.push(index);
        }
    }
    let widest = segments.iter().map(|link| link.reach).fold(0.0, f32::max);
    let mut lines: Grid<usize> = Grid::new(SIDEWALK_CELL);
    for (index, link) in segments.iter().enumerate() {
        // радиус здесь знает запрос (`widest` ниже), а не звено
        lines.insert_segment(link.from, link.to, 0.0, index);
    }

    let uses = vertex_uses(map);
    let obstacles = Obstacles::new(map);
    let mut fronts: Vec<Front> = Vec::new();
    let mut left = 0;
    for (index, building) in map.buildings.iter().enumerate() {
        if building.kind != AreaKind::Building
            || matches!(building.building_use, BuildingUse::Church(_))
            || building
                .outer
                .iter()
                .any(|vertex| uses[&vertex_key(*vertex)] > 1)
        {
            continue;
        }
        let (min, max) = ring_bounds(&building.outer);
        let margin = Vec2::splat(widest + SIDEWALK_SHIFT_MAX);
        let nearby = lines.near(min - margin, max + margin);
        if nearby.is_empty() {
            continue;
        }
        match Front::of(index, &building.outer, &nearby, &segments, &segment_road) {
            // улица сквозь дом: сдвигать некуда, и в ряд он не встаёт
            None => left += 1,
            Some(front) if front.need > -ROW_SETBACK_TOLERANCE => fronts.push(front),
            Some(_) => {}
        }
    }

    // ряды: соседи по одной стороне одной улицы, union-find
    let mut parent: Vec<usize> = (0..fronts.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut by_street: HashMap<(usize, bool), Vec<usize>> = HashMap::new();
    for (index, front) in fronts.iter().enumerate() {
        // дом, которому мало предела, — ряд из одного себя
        if front.need <= SIDEWALK_SHIFT_MAX {
            by_street
                .entry((front.road, front.left))
                .or_default()
                .push(index);
        }
    }
    for members in by_street.values() {
        for (position, &a) in members.iter().enumerate() {
            for &b in &members[position + 1..] {
                let (first, second) = (&fronts[a], &fronts[b]);
                let gap = (first.min - second.max)
                    .max(second.min - first.max)
                    .max(Vec2::ZERO)
                    .length();
                if gap <= ROW_GAP {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
    }
    // сдвиг ряда — наезд самого глубокого дома в нём
    let mut row_need: HashMap<usize, f32> = HashMap::new();
    for (index, front) in fronts.iter().enumerate() {
        let row = root(&mut parent, index);
        let need = row_need.entry(row).or_insert(f32::NEG_INFINITY);
        *need = need.max(front.need);
    }

    let mut moved = 0;
    let mut partly = 0;
    for (index, front) in fronts.iter().enumerate() {
        let need = row_need[&root(&mut parent, index)];
        if need <= SIDEWALK_SHIFT_TOLERANCE || front.need < need - ROW_SETBACK_TOLERANCE {
            continue;
        }
        let intruding = front.need > SIDEWALK_SHIFT_TOLERANCE;
        let building = &map.buildings[front.building];
        let nearby = local(&front.nearby, &segments);
        // сдвиг ряда, потом остаток наезда от других улиц — всё в пределе
        let mut shift = front.away * need.min(SIDEWALK_SHIFT_MAX);
        let Some(mut push) = sidewalk_push(&building.outer, shift, &nearby) else {
            left += usize::from(intruding);
            continue;
        };
        for _ in 0..SIDEWALK_SHIFT_ROUNDS {
            if push.length() <= SIDEWALK_SHIFT_TOLERANCE {
                break;
            }
            let next = (shift + push).clamp_length_max(SIDEWALK_SHIFT_MAX);
            // ось улицы вошла в контур или предел не пускает дальше
            let Some(next_push) = sidewalk_push(&building.outer, next, &nearby) else {
                break;
            };
            if next.distance(shift) <= SIDEWALK_SHIFT_TOLERANCE {
                break;
            }
            (shift, push) = (next, next_push);
        }
        let Some(shift) = SHIFT_FRACTIONS
            .iter()
            .map(|fraction| shift * fraction)
            .find(|&shift| !obstacles.blocks(map, front.building, shift))
        else {
            left += usize::from(intruding);
            continue;
        };
        let remains = sidewalk_push(&building.outer, shift, &nearby)
            .is_none_or(|push| push.length() > SIDEWALK_SHIFT_TOLERANCE);
        partly += usize::from(intruding && remains);
        let building = &mut map.buildings[front.building];
        building
            .outer
            .iter_mut()
            .for_each(|vertex| *vertex += shift);
        building
            .holes
            .iter_mut()
            .flatten()
            .for_each(|vertex| *vertex += shift);
        building
            .entrances
            .iter_mut()
            .for_each(|entrance| *entrance += shift);
        moved += 1;
    }
    PulledHouses {
        moved,
        partly,
        left,
    }
}

/// Итог [`pull_houses_off_sidewalks`].
struct PulledHouses {
    /// Сдвинуто домов, соседи по ряду включительно.
    moved: usize,
    /// Из них наезжавших, что после сдвига ещё задевают тротуар: упёрлись в
    /// предел или в соседний объект.
    partly: usize,
    /// Наезжающих, оставленных на месте.
    left: usize,
}

/// Зазор между краем квартала и внешним краем нарисованного полотна, м,
/// который ещё дотягивается. Больше — это уже не щель, а настоящий промежуток
/// (палисадник, обочина, полоса отвода), и зелень туда лезть не должна.
/// На Туле в предел попадает четверть вершин кварталов из 2756: по метрам
/// 261 / 174 / 130 / 88 / 99, и дальше пяти метров их ещё 200 с лишним. Было
/// 3 м, поднято по взгляду на кадр: на четвёртом и пятом метре полоска земли
/// вдоль улицы всё ещё читается швом, а не обочиной.
const LANDUSE_GAP_MAX: f32 = 5.0;
/// На сколько метров дотянутый край квартала заводится **под** полотно, м.
/// Лента рисуется по сглаженной оси (`roads::centerline`), а зазор меряется по
/// сырым точкам OSM — без запаса на повороте осталась бы щель в сантиметр.
const LANDUSE_OVERLAP: f32 = 0.5;
/// Длиннее этого, м, ребро квартала рядом с дорогой разбивается на части.
/// Между своими вершинами ребро прямое, а дорога гнётся, и на выпуклости
/// поворота щель осталась бы посреди ребра, где двигать нечего.
const LANDUSE_STEP: f32 = 8.0;

/// Квартал (`landuse`), край которого не доходит до дороги считаные метры,
/// **дотягивается под полотно**. Возвращает, сколько вершин сдвинуто.
///
/// Ширина улицы в модели — константа класса, тротуар добавляет рендер, а
/// границу квартала в OSM рисуют по красным линиям или по заборам участков,
/// и между двором и нарисованным тротуаром остаётся полоска голой земли в
/// метр-полтора (Тула, `landuse=residential` 185117817 вдоль улицы
/// Воздухофлотской: граница в 6.1 м от оси при 5.76 м полосы). На снимке это
/// читается как непрокрашенный шов, а не как обочина.
///
/// Двигается **вершина**, а не квартал целиком: край подтягивается к оси
/// ближайшей дороги до [`LANDUSE_OVERLAP`] внутрь её полотна. Квартал лежит
/// ниже всего, что на нём нарисовано (`Z_LANDUSE` 0.25 против `Z_SIDEWALK`
/// 1.2), так что заведённая под асфальт зелень не видна — видно только то,
/// что щель закрылась.
///
/// **Зелень только растёт**: вершина идёт к дороге, лишь если этот сдвиг ведёт
/// **наружу от заливки** ([`pull_ring`]). Иначе улица, проходящая внутри
/// квартала, сжала бы его границу к себе; а улица в дырке, наоборот, дырку
/// сжимает — зелени там нет, и подходить к полотну обязан её край.
///
/// Мосты и арки пропущены: под мостом квартал и так рисуется, а проезд сквозь
/// дом — это не край двора.
fn pull_landuse_to_roads(map: &mut MapData) -> usize {
    // `reach` — внешний край нарисованного полотна от оси
    let mut segments: Vec<Link> = Vec::new();
    for road in &map.roads {
        if road.bridge || road.passage {
            continue;
        }
        let sidewalk = if is_carriageway(road) {
            sidewalk_width(road.width).unwrap_or_default()
        } else {
            0.0
        };
        let edge = road.width / 2.0 + sidewalk;
        for link in road.points.windows(2) {
            segments.push(Link {
                from: link[0],
                to: link[1],
                reach: edge,
            });
        }
    }
    // звено кладётся в ячейки с запасом на своё полотно и предельный зазор,
    // так что спрашивающему хватает ячейки самой вершины
    let mut lines: Grid<usize> = Grid::new(SIDEWALK_CELL);
    for (index, link) in segments.iter().enumerate() {
        lines.insert_segment(link.from, link.to, link.reach + LANDUSE_GAP_MAX, index);
    }

    let mut pulled = 0;
    for area in &mut map.landuse {
        area.outer = pull_ring(&area.outer, false, &segments, &lines, &mut pulled);
        area.holes = area
            .holes
            .iter()
            .map(|hole| pull_ring(hole, true, &segments, &lines, &mut pulled))
            .collect();
    }
    pulled
}

/// Кольцо квартала с дотянутыми к дорогам вершинами. Ребро длиннее
/// [`LANDUSE_STEP`] рядом с дорогой разбивается, и вставленная точка остаётся
/// в кольце, только если ей нашлось куда сдвинуться, — иначе кольцо копило бы
/// лишние вершины на каждой перестройке геометрии.
///
/// «Наружу от зелени» считается **локально**, по самому кольцу: для внешнего
/// контура это прочь из квартала, для дырки — внутрь неё (зелень лежит снаружи
/// такого кольца). Локально, а не вопросом «лежит ли дорога вне квартала»: у
/// полосы газона между двумя улицами ближайшая улица бывает **за
/// противоположным** краем, и такой ответ сжал бы полосу вместо того, чтобы её
/// растянуть.
fn pull_ring(
    ring: &[Vec2],
    hole: bool,
    segments: &[Link],
    lines: &Grid<usize>,
    pulled: &mut usize,
) -> Vec<Vec2> {
    // ориентация колец в OSM произвольная, так что сторону задаёт знак площади
    let sign = if (signed_ring_area(ring) > 0.0) == hole {
        1.0
    } else {
        -1.0
    };
    let normal = |from: Vec2, to: Vec2| ((to - from).perp() * sign).normalize_or_zero();
    let mut out: Vec<Vec2> = Vec::with_capacity(ring.len());
    for (index, &point) in ring.iter().enumerate() {
        let mut push = |point: Vec2, outward: Vec2, inserted: bool| match pull_vertex(
            point, outward, segments, lines,
        ) {
            Some(shifted) => {
                out.push(shifted);
                *pulled += 1;
            }
            None if inserted => {}
            None => out.push(point),
        };
        let previous = ring[(index + ring.len() - 1) % ring.len()];
        let next = ring[(index + 1) % ring.len()];
        // у вершины — биссектриса её рёбер, у вставленной точки — нормаль
        // самого ребра
        let along = normal(point, next);
        push(
            point,
            (normal(previous, point) + along).normalize_or_zero(),
            false,
        );
        let length = point.distance(next);
        if length <= LANDUSE_STEP || lines.near(point.min(next), point.max(next)).is_empty() {
            continue;
        }
        let steps = (length / LANDUSE_STEP).ceil() as usize;
        for step in 1..steps {
            push(point.lerp(next, step as f32 / steps as f32), along, true);
        }
    }
    out
}

/// Куда встаёт вершина квартала, которой до полотна ближайшей дороги остался
/// зазор не больше [`LANDUSE_GAP_MAX`]; `None` — двигать нечего или некуда.
/// `outward` — куда от этой вершины прибывает зелень (см. [`pull_ring`]).
fn pull_vertex(point: Vec2, outward: Vec2, segments: &[Link], lines: &Grid<usize>) -> Option<Vec2> {
    // ближайшая по **зазору до края полотна**, а не по расстоянию до оси:
    // узкий проезд рядом ближе широкой улицы, а щель оставляет улица
    let mut best: Option<(f32, Vec2)> = None;
    for index in lines.near(point, point) {
        let Link { from, to, reach } = segments[index];
        let axis = closest_on_segment(point, from, to);
        let gap = point.distance(axis) - reach;
        if best.is_none_or(|(best_gap, _)| gap < best_gap) {
            best = Some((gap, axis));
        }
    }
    let (gap, axis) = best?;
    if gap <= 0.0 || gap > LANDUSE_GAP_MAX {
        return None;
    }
    // зелень только прибывает: сдвиг к дороге, уводящий край внутрь заливки,
    // не делается вовсе — так улица, идущая внутри квартала, его не сжимает
    let direction = (axis - point).try_normalize()?;
    (direction.dot(outward) > 0.0).then(|| point + direction * (gap + LANDUSE_OVERLAP))
}

/// Во что сдвигаемый дом не должен упереться: другие здания и отрезки всего
/// прочего — дорог и рельсов (от края полотна), стен, заборов, труб, открытых
/// водотоков, берегов водоёмов, цилиндров (отрезок нулевой длины радиусом).
///
/// Правило относительное: мешает только объект, к которому дом **подошёл**
/// ближе [`SHIFT_CLEARANCE`], — стоявший вплотную по данным (тропа у стены,
/// забор по участку) сдвиг от себя не запрещает.
struct Obstacles {
    /// Контуры зданий до сдвигов: «было» меряется по ним, «стало» — по
    /// текущей карте, где соседи могли уже сдвинуться.
    original: Vec<Vec<Vec2>>,
    /// Здания по ячейкам — габарит, раздутый на [`SIDEWALK_SHIFT_MAX`], так
    /// что сдвинутое здание не выходит из своих ячеек.
    buildings: Grid<usize>,
    /// Звенья всего прочего; `reach` — радиус запрета, полуширина плюс зазор.
    segments: Vec<Link>,
    lines: Grid<usize>,
}

impl Obstacles {
    fn new(map: &MapData) -> Self {
        let original: Vec<Vec<Vec2>> = map
            .buildings
            .iter()
            .map(|building| building.outer.clone())
            .collect();
        let mut buildings = Grid::new(SIDEWALK_CELL);
        let grow = Vec2::splat(SIDEWALK_SHIFT_MAX + SHIFT_CLEARANCE);
        for (index, ring) in original.iter().enumerate() {
            let (min, max) = ring_bounds(ring);
            buildings.insert(min - grow, max + grow, index);
        }

        let mut segments = Vec::new();
        let mut add = |points: &[Vec2], half_width: f32| {
            let reach = half_width + SHIFT_CLEARANCE;
            points.windows(2).for_each(|link| {
                segments.push(Link {
                    from: link[0],
                    to: link[1],
                    reach,
                })
            });
        };
        map.roads
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.rails
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.walls
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.fences.iter().for_each(|line| add(&line.points, 0.0));
        map.pipes
            .iter()
            .for_each(|line| add(&line.points, line.width / 2.0));
        map.water_lines
            .iter()
            .filter(|line| !line.tunnel)
            .for_each(|line| add(&line.points, line.width / 2.0));
        for area in &map.water {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                let closed: Vec<Vec2> = ring.iter().chain(ring.first()).copied().collect();
                add(&closed, 0.0);
            }
        }
        for structure in &map.structures {
            add(&[structure.at, structure.at], structure.radius);
        }
        let mut lines = Grid::new(SIDEWALK_CELL);
        for (index, link) in segments.iter().enumerate() {
            lines.insert_segment(link.from, link.to, link.reach, index);
        }
        Self {
            original,
            buildings,
            segments,
            lines,
        }
    }

    /// Упрётся ли здание `house`, перенесённое из исходного места на `shift`.
    fn blocks(&self, map: &MapData, house: usize, shift: Vec2) -> bool {
        let before = &self.original[house];
        let after: Vec<Vec2> = before.iter().map(|vertex| *vertex + shift).collect();
        let (min, max) = ring_bounds(&after);
        // стало ближе запрета и ближе, чем было
        let closer = |now: f32, reach: f32, was: f32| now < reach && now < was - 0.01;

        let buildings = self.buildings.near(min, max);
        let neighbours = buildings.iter().filter(|&&other| other != house);
        for &other in neighbours {
            let current = &map.buildings[other].outer;
            let (other_min, other_max) = ring_bounds(current);
            let apart = (min - other_max).max(other_min - max).max(Vec2::ZERO);
            if apart.length() >= SHIFT_CLEARANCE {
                continue;
            }
            let now = ring_distance(&after, current);
            if closer(
                now,
                SHIFT_CLEARANCE,
                ring_distance(before, &self.original[other]),
            ) {
                return true;
            }
        }
        self.lines.near(min, max).into_iter().any(|index| {
            let Link { from, to, reach } = self.segments[index];
            let now = ring_segment_distance(&after, from, to);
            now < reach && closer(now, reach, ring_segment_distance(before, from, to))
        })
    }
}

/// Расстояние от контура до отрезка: ноль, если отрезок пересекает контур или
/// лежит внутри.
fn ring_segment_distance(ring: &[Vec2], from: Vec2, to: Vec2) -> f32 {
    if point_in_polygon(from, ring) {
        return 0.0;
    }
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            closest_between_segments(a, b, from, to)
                .map_or(0.0, |(on_ring, on_segment)| on_ring.distance(on_segment))
        })
        .fold(f32::INFINITY, f32::min)
}

/// Расстояние между двумя контурами: ноль, если они пересекаются или один
/// внутри другого.
fn ring_distance(a: &[Vec2], b: &[Vec2]) -> f32 {
    if a.first().is_some_and(|vertex| point_in_polygon(*vertex, b)) {
        return 0.0;
    }
    (0..b.len())
        .map(|index| ring_segment_distance(a, b[index], b[(index + 1) % b.len()]))
        .fold(f32::INFINITY, f32::min)
}

/// Отрезки улиц по индексам.
fn local(indices: &[usize], segments: &[Link]) -> Vec<Link> {
    indices.iter().map(|&index| segments[index]).collect()
}

/// Дом, выходящий фасадом на улицу: ближайшая к контуру ось.
struct Front {
    building: usize,
    /// Индексы отрезков улиц рядом с домом.
    nearby: Vec<usize>,
    road: usize,
    /// С какой стороны оси улицы стоит дом (по порядку её точек).
    left: bool,
    /// Единичное направление от оси к стене.
    away: Vec2,
    /// Насколько стена заходит в полосу улицы, м; отрицательное — запас.
    need: f32,
    min: Vec2,
    max: Vec2,
}

impl Front {
    /// `None` — ось улицы проходит сквозь контур или кончается внутри него.
    fn of(
        building: usize,
        ring: &[Vec2],
        nearby: &[usize],
        segments: &[Link],
        segment_road: &[usize],
    ) -> Option<Self> {
        let mut best: Option<(f32, usize, Vec2, Vec2)> = None;
        for &segment in nearby {
            let Link { from, to, reach } = segments[segment];
            if point_in_polygon(from, ring) || point_in_polygon(to, ring) {
                return None;
            }
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                let (on_wall, on_axis) = closest_between_segments(a, b, from, to)?;
                let need = reach - on_wall.distance(on_axis);
                if best.is_none_or(|(deepest, ..)| need > deepest) {
                    best = Some((need, segment, on_wall, on_axis));
                }
            }
        }
        let (need, segment, on_wall, on_axis) = best?;
        let Link { from, to, .. } = segments[segment];
        let (min, max) = ring_bounds(ring);
        Some(Self {
            building,
            nearby: nearby.to_vec(),
            road: segment_road[segment],
            left: (to - from).perp_dot(on_wall - on_axis) > 0.0,
            away: (on_wall - on_axis).normalize_or_zero(),
            need,
            min,
            max,
        })
    }
}

/// Сдвиг, выводящий контур `ring`, перенесённый на `shift`, из самой глубоко
/// задетой полосы улиц `segments`: `Some(ZERO)` — не наезжает, `None` — ось
/// улицы проходит сквозь контур или кончается внутри него, и сдвигать некуда.
fn sidewalk_push(ring: &[Vec2], shift: Vec2, segments: &[Link]) -> Option<Vec2> {
    let inside = |point: Vec2| point_in_polygon(point - shift, ring);
    if segments
        .iter()
        .any(|link| inside(link.from) || inside(link.to))
    {
        return None;
    }
    let mut push = Vec2::ZERO;
    for index in 0..ring.len() {
        let (a, b) = (ring[index] + shift, ring[(index + 1) % ring.len()] + shift);
        for &Link { from, to, reach } in segments {
            let (on_wall, on_axis) = closest_between_segments(a, b, from, to)?;
            let away = on_wall - on_axis;
            let depth = reach - away.length();
            if depth > push.length() {
                push = away.normalize_or_zero() * depth;
            }
        }
    }
    Some(push)
}

/// Ближайшие точки двух отрезков `(на первом, на втором)`; `None` — отрезки
/// пересекаются.
fn closest_between_segments(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<(Vec2, Vec2)> {
    let (ab, cd, ac) = (b - a, d - c, c - a);
    let denominator = ab.perp_dot(cd);
    if denominator != 0.0 {
        let t = ac.perp_dot(cd) / denominator;
        let u = ac.perp_dot(ab) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return None;
        }
    }
    [
        (a, closest_on_segment(a, c, d)),
        (b, closest_on_segment(b, c, d)),
        (closest_on_segment(c, a, b), c),
        (closest_on_segment(d, a, b), d),
    ]
    .into_iter()
    .min_by(|x, y| {
        x.0.distance_squared(x.1)
            .total_cmp(&y.0.distance_squared(y.1))
    })
}

/// Углы контура: наибольшее отклонение от прямого, градусы (у вогнутого угла —
/// от 270°), и сколько углов вогнутых. `None` — у контура есть ребро нулевой
/// длины или разворот назад.
fn corner_skew(ring: &[Vec2]) -> Option<(f32, usize)> {
    let count = ring.len();
    // от первой вершины: в метрах карты (тысячи) произведения в f32 теряют сантиметры
    let local: Vec<Vec2> = ring.iter().map(|vertex| *vertex - ring[0]).collect();
    let winding = signed_ring_area(&local).signum();
    let mut skew = 0.0_f32;
    let mut reflex = 0;
    for index in 0..count {
        let (prev, at, next) = (
            local[(index + count - 1) % count],
            local[index],
            local[(index + 1) % count],
        );
        let (back, forth) = (at - prev, next - at);
        if back == Vec2::ZERO || forth == Vec2::ZERO {
            return None;
        }
        let turn = back.angle_to(forth).to_degrees() * winding;
        if turn.abs() >= 179.0 {
            return None;
        }
        if turn < 0.0 {
            reflex += 1;
        }
        skew = skew.max((turn.abs() - 90.0).abs());
    }
    Some((skew, reflex))
}

/// Ось, вдоль которой обведён контур: направления рёбер с периодом 90° — угол
/// ×4 сводит рёбра обеих осей в одно, — средние с весом длины.
fn outline_axis(ring: &[Vec2]) -> Vec2 {
    let vote = (0..ring.len()).fold(Vec2::ZERO, |sum, index| {
        let e = ring[(index + 1) % ring.len()] - ring[index];
        sum + Vec2::from_angle(e.to_angle() * 4.0) * e.length()
    });
    Vec2::from_angle(vote.to_angle() / 4.0)
}

/// Прямоугольник с центроидом и площадью четырёхугольника `quad`; угол `i`
/// соответствует его вершине `i`, обход тот же.
fn fit_rectangle(quad: &[Vec2; 4]) -> [Vec2; 4] {
    let edge = |index: usize| quad[(index + 1) % 4] - quad[index];
    let axis = outline_axis(quad);
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
    let first = quad[0] + ring_area_centroid(&local) - (side + up) / 2.0;
    [first, first + side, first + side + up, first + up]
}

/// Г-образный шестиугольник, прямоугольный, в осях `outline_axis` контура `ring`;
/// угол `i` соответствует его вершине `i`, обход тот же. `None` — рёбра не
/// чередуются по осям (это не Г).
///
/// Каждое ребро относится к ближней оси и получает свой **уровень** — среднее
/// своих концов поперёк этой оси; вершина встаёт на пересечение уровней двух
/// своих рёбер. Для прямоугольного контура это тождество, для криво обведённого —
/// каждая стена ложится посередине между своими концами.
fn fit_ell(ring: &[Vec2]) -> Option<Vec<Vec2>> {
    let count = ring.len();
    let local: Vec<Vec2> = ring.iter().map(|vertex| *vertex - ring[0]).collect();
    let axis = outline_axis(&local);
    let across = axis.perp();
    let edge = |index: usize| local[(index + 1) % count] - local[index];
    // «вдоль» — ребро ближе к `axis`, его уровень меряется поперёк
    let along: Vec<bool> = (0..count)
        .map(|index| edge(index).dot(axis).abs() >= edge(index).dot(across).abs())
        .collect();
    if (0..count).any(|index| along[index] == along[(index + 1) % count]) {
        return None;
    }
    let level = |index: usize| {
        let middle = (local[index] + local[(index + 1) % count]) / 2.0;
        if along[index] {
            middle.dot(across)
        } else {
            middle.dot(axis)
        }
    };
    let fitted = (0..count)
        .map(|index| {
            let (before, after) = ((index + count - 1) % count, index);
            let (lengthwise, crosswise) = if along[before] {
                (before, after)
            } else {
                (after, before)
            };
            ring[0] + axis * level(crosswise) + across * level(lengthwise)
        })
        .collect();
    Some(fitted)
}

/// Центроид **площади** простого кольца — центр масс, в отличие от среднего
/// вершин ([`ring_vertex_mean`]): на вытянутом или невыпуклом контуре эти две
/// точки расходятся на метры, поэтому у них разные имена, а не одно на двоих.
fn ring_area_centroid(ring: &[Vec2]) -> Vec2 {
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
            colours: area_colours(kind, &element.tags),
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
    let colours = area_colours(kind, &element.tags);

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
                colours,
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
    NON_WALKABLE_ENTRANCES, area_colours, area_height, area_kind, area_use, crown_radius,
    fence_kind, is_building_passage, is_oneway, is_oneway_backward, is_road_underground,
    is_roundabout, is_underground, pipe_width, rail_class, road_class, row_spacing, service_track,
    structure_height, structure_kind, structure_radius, structure_size, tagged_lanes, water_class,
    water_width,
};
