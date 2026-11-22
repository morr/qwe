//! Финишная половина разбора: доводочные проходы ([`finish_parse`]) и их
//! помощники — утопленники, привязка разметанных дверей, ключ вершины.

use bevy::math::Vec2;
use bevy::platform::collections::{HashMap, HashSet};

use crate::map::cars::district::Districts;
use crate::map::osm::entrances::generate_entrances;
use crate::map::osm::model::{MapData, TreeCompose, TreeRowLayout, point_in_area, ring_bounds};
use crate::map::osm::planting::plant_trees;
use crate::map::roads::network::sections;
use crate::map::roads::rings;

use super::{
    DrawBuildout, InferredSidewalks, ParseKnobs, PassReport, Pending, PlantedReport, PulledHouses,
    StretchedAreas, drop_untagged_sidewalks, faiths, infer_sidewalks, lots, pavements, pockets,
    pull_areas_to_roads, pull_houses_off_sidewalks, rails, square_skewed_houses, verges,
};

/// Шаг квантования координат при привязке входа к зданию, 1/м. Вход в OSM —
/// как правило общий узел контура здания (Тула 82%, Берлин 79%, Париж 65%),
/// так что после одной и той же проекции координаты совпадают точно;
/// сантиметровая сетка — страховка от шума f32, а не поиск ближайшего.
const ENTRANCE_SNAP_SCALE: f32 = 100.0;

/// Доводочные проходы по сырой карте — девять, плюс сборка деревьев в конце, —
/// **и этот порядок и есть их интерфейс**. До этой функции он жил заметками в
/// трёх doc-комментариях из восьми и не был записан целиком нигде.
///
/// Почему именно так, сверху вниз:
///
/// 0. **Улицы и сечения** (`map::roads::network::sections`) — самыми первыми:
///    ширина дороги выводится из числа полос, а её читают шаги 5 и 6 (тротуар
///    отодвигает дома, край дороги притягивает кварталы и стоянки). Сеть улиц,
///    собранная здесь, читается и тротуаром, отданным отдельной дорожке
///    (шаг 1), — по ней проба знает, чей след для неё чужой. Домов этот
///    шаг не касается, так что ставить его раньше утопленников ничему не мешает.
///    Ещё раньше, прямо перед ним, **выпрямляются Y-подходы к кольцам**
///    (`map::roads::rings::straighten_tails`): проход двигает узлы и режет way,
///    а сеть улиц собирается по готовым way, так что после сечений он бы её
///    сломал.
/// 1. **Утопленники** уходят первыми: дом, целиком стоящий в воде, не должен
///    получить ни веры, ни двери, ни выпрямленного контура — всё это работа
///    по дому, которого не будет.
///    **Пути, оборванные на дороге, сшиваются** сразу за ними
///    ([`rails::stitch_rail_gaps`]): проход читает только осевые улиц и
///    путей, а место — любое после сечений (ему нужен класс проезжей части);
///    дальше пути в разборе никто не двигает.
///    **Тротуары без тега** решаются сразу за ними ([`infer_sidewalks`]): мера
///    — этажность застройки вокруг, и утонувший дом в неё входить не должен;
///    а читают решение шаги 5 и 6 (дом отъезжает только от нарисованного
///    тротуара, квартал дотягивается под него же).
///    **Покрытие дорожек без тега** ([`pavements::infer_pavements`]) — там же, хотя
///    место ему любое: зелень, которую он спрашивает, дальше не двигается, а
///    читает решение только рендер.
///    **Тротуар, отданный отдельной дорожке** ([`verges::measure_footways_beside_streets`]),
///    — сразу за покрытием: он спрашивает, мощёная ли дорожка вдоль кромки, а
///    читают его, как и тротуар по застройке, шаги 5 и 6.
/// 2. **Вера** — до дверей и до выпрямления: она собирает храм из частей
///    (`faiths::resolve_faiths` зовёт `absorb_annexes` внутри себя), а часть,
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
/// 6. **Подтягивание кварталов и стоянок к дорогам** — где угодно в хвосте: ни
///    `landuse`, ни `parking` не трогают ни навмеш, ни двери, ни посадку.
///    Стоит здесь, потому что дома к этому моменту уже на своих местах, а
///    контуры стоянок — **после** выпрямления домиков (шаг 4): `vertex_uses`
///    считает общие вершины по сырым контурам стоянок, и дотянутый край не
///    должен менять то, какие дома выпрямились. Шаг — две строки: кварталы
///    (`pull_areas_to_roads`), а сразу за ними стоянки (`lots::pave_lots`) —
///    другим контекстом: узкий проход получает [`lots::PaveLotsCtx`] со
///    ссылками на то, что читает, а не `&mut MapData` целиком.
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
///
/// Входы вне ответа Overpass — только `knobs`: ширину полосы берёт шаг 0,
/// навтайл — шаг 7. Глобалей проходы не читают.
///
/// **Сырой OSM** (`knobs.raw`, [`super::RawOsm`]) пропускает все шаги, кроме
/// размеченных дверей (шаг 3) и посадки (шаг 8); сеть улиц склеивается без
/// вывода сечений (`sections::apply_raw`), а тротуар без тега снимается
/// ([`drop_untagged_sidewalks`]). На втором уровне посадка оставляет только
/// деревья-ноды OSM.
pub(in crate::map::osm::parse) fn finish_parse(
    map: &mut MapData,
    pending: &Pending,
    knobs: ParseKnobs,
) -> PassReport {
    map.knobs = knobs;
    let entrances = &pending.entrances;
    // сырой OSM (`RawOsm`): доводочные проходы пропускаются все, кроме
    // размеченных дверей — это данные, — и сборки сети улиц, которая ничего в
    // данных не меняет. Пропущенный проход отчитывается нулями
    let raw = knobs.raw.skips_passes();
    let straightened = if raw {
        Default::default()
    } else {
        rings::straighten_tails(&mut map.roads)
    };
    let sections = if raw {
        sections::apply_raw(map, knobs.lane_width)
    } else {
        sections::apply(map, knobs.lane_width)
    };
    let drowned = if raw { 0 } else { drop_buildings_in_water(map) };
    // переезд, которого нет в OSM: путь, оборванный на дороге, сшивается
    let stitched_rails = if raw { 0 } else { rails::stitch_rail_gaps(map) };
    let (sidewalks, pavements, separate, faiths_guessed) = if raw {
        // тротуар — только по тегу: достроенный без тега снимается
        let dropped = drop_untagged_sidewalks(&mut map.roads);
        (
            InferredSidewalks {
                asked: dropped,
                dropped,
            },
            Default::default(),
            Default::default(),
            0,
        )
    } else {
        // мера квартала — явный вход прохода: один `Districts` на все его
        // вопросы «сколько вокруг этажей», построенный по домам, оставшимся
        // после утопленников (утонувший дом в неё входить не должен)
        let sidewalks = {
            let districts = Districts::new(&map.buildings);
            infer_sidewalks(&mut map.roads, &districts)
        };
        let pavements = pavements::infer_pavements(pavements::InferPavementsCtx::of(map));
        let separate = verges::measure_footways_beside_streets(&mut map.roads, &map.network);
        let faiths_guessed = faiths::resolve_faiths(faiths::ResolveFaithsCtx::of(map));
        (sidewalks, pavements, separate, faiths_guessed)
    };
    let entrances_orphaned = attach_entrances(map, entrances);

    let started = std::time::Instant::now();
    let squared = if raw { 0 } else { square_skewed_houses(map) };
    let squaring = started.elapsed();

    let started = std::time::Instant::now();
    let pulled = if raw {
        PulledHouses::default()
    } else {
        pull_houses_off_sidewalks(map)
    };
    let pulling = started.elapsed();

    let started = std::time::Instant::now();
    let stretched = if raw {
        StretchedAreas::default()
    } else {
        pull_areas_to_roads(map)
    };
    let stretching = started.elapsed();

    // стоянки — второй строкой того же шага 6, сразу за кварталами и их
    // обочинами: дотянутые кварталы уже покрывают свою землю. Данные —
    // контекстом (`PaveLotsCtx`), а не картой целиком: узкий проход получает
    // ссылку на то, что читает
    let started = std::time::Instant::now();
    let lots = if raw {
        lots::PavedLots::default()
    } else {
        lots::pave_lots(lots::PaveLotsCtx::of(map))
    };
    let paving = started.elapsed();

    // после кварталов и стоянок: дотянутые, они уже покрывают свою землю
    let started = std::time::Instant::now();
    let sown = if raw {
        0
    } else {
        pockets::fill_ground_pockets(map)
    };
    let sowing = started.elapsed();

    // размеченных дверей в OSM единицы процентов — остальным дом получает свои
    // по замеру когорт, см. `entrances/`
    let started = std::time::Instant::now();
    let generated = if raw {
        0
    } else {
        generate_entrances(map, knobs.navtile)
    };
    let generating = started.elapsed();

    let started = std::time::Instant::now();
    let (standalone, mut woods, mut rows, asked) = plant_trees(map);
    // второй уровень: сгенерированных деревьев нет — лес и аллеи не
    // сажаются, остаются деревья-ноды OSM
    if knobs.raw.disables(DrawBuildout::PlantedTrees) {
        woods.clear();
        rows = Default::default();
    }
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
        raw: knobs.raw,
        straightened,
        sections,
        drowned,
        stitched_rails,
        sidewalks,
        pavements,
        separate,
        faiths_guessed,
        entrances_found: entrances.len(),
        entrances_orphaned,
        squared,
        squaring,
        pulled,
        pulling,
        stretched,
        stretching,
        lots,
        paving,
        sown,
        sowing,
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
pub(in crate::map::osm::parse) fn drop_buildings_in_water(map: &mut MapData) -> usize {
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

/// Раскладка входов по зданиям: вход ищется среди вершин контуров, потому что
/// в OSM он и есть узел контура. Возвращает число входов, не нашедших дом.
///
/// Общий узел двух домов (сплошная застройка) попадает в таблицу один раз —
/// вход достанется одному из них, и это не важно: дверь всё равно там же.
///
/// Совпадающие входы схлопываются: в Париже встречаются две ноды `entrance` в
/// одной точке (замер по выгрузке — минимальный зазор 0.00 м), а две двери на
/// одном месте — это две одинаковых цели для пешек и лишний кружок в оверлее.
pub(in crate::map::osm::parse) fn attach_entrances(map: &mut MapData, entrances: &[Vec2]) -> usize {
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

/// Ключ вершины на сантиметровой сетке: общий OSM-узел двух контуров, линий или
/// входа после одной проекции даёт один и тот же ключ. Одна привязка на всех —
/// [`attach_entrances`] ищет дом этим же ключом (см. [`ENTRANCE_SNAP_SCALE`]).
pub(in crate::map::osm::parse) fn vertex_key(point: Vec2) -> (i32, i32) {
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
pub(in crate::map::osm::parse) fn vertex_uses(map: &MapData) -> HashMap<(i32, i32), u32> {
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
