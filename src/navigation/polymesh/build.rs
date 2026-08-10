//! Постройка меша: контуры препятствий → boolean-объединение → CDT polyanya,
//! чанк за чанком. Швами занимается [`super::seams`], поиском — [`super::path`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use super::seams::{SeamPoints, chunk_outline, quantized, seam_points};
use super::stitch::{components_of, stitch_chunks};
use super::{
    ChunkComponents, PolymeshBuild, PolymeshInput, SEARCH_DELTA, SEARCH_STEPS, chunk_grid,
};
use crate::map::miter_offsets;
use crate::map::osm::model::signed_ring_area;
use crate::settings::MAP_SIZE;

/// Упрощение контуров препятствий (Visvalingam–Whyatt внутри polyanya),
/// метры. Не косметика: boolean оставляет отрезки в доли миллиметра, CDT на
/// них вырождается, и поиск по такому мешу зацикливается — замерено на
/// `examples/polymesh_bench`, без упрощения прогон встаёт на 40-м запросе.
const SIMPLIFY_EPSILON: f32 = 0.05;

/// Насколько прямоугольник клипа ШИРЕ границы карты. Препятствия обрезаются
/// снаружи от внешней границы триангуляции, а не внутрь: втянутый клип
/// оставлял бы вдоль кромки карты проходимую щель шириной в отступ, и путь
/// обходил бы по ней реку, упирающуюся в границу. Пересечение препятствием
/// внешней границы polyanya переваривает: проходимость треугольника — точечный
/// `contains` центра (внутри exterior и вне колец препятствий), так что
/// треугольники за границей непроходимы сами по себе, а клип лишь не даёт
/// хвостам OSM-геометрии за bbox раздувать триангуляцию
/// (`polyanya::input/triangulation.rs::as_layer`).
const MAP_EDGE_MARGIN: f32 = 10.0;

/// Весь конвейер: контуры препятствий → boolean → CDT polyanya. `None` —
/// постройку отменили; проверки стоят перед каждым долгим шагом, внутрь
/// i_overlay и spade не заглянуть.
pub(super) fn build_polymesh(
    input: &PolymeshInput,
    agent_radius: f32,
    cancelled: Option<&AtomicBool>,
    chunk_meters: Option<f32>,
) -> Option<PolymeshBuild> {
    let is_cancelled = || cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed));

    let mut blockers: Vec<Vec<[f32; 2]>> = Vec::new();
    // дыры колец идут в объединение обратным обходом: NonZero вычитает их из
    // внешнего кольца ровно так же, как сеточный `row_spans` вычитает их
    // интервалы, а дом внутри дыры (сарай во дворе) снова даёт +1 и остаётся
    // сплошным. Выбросить дыру нельзя: остров в реке — это inner-кольцо
    // водного мультиполигона (Сите и Сен-Луи — дыры «La Seine»), и вся суша
    // под ним оказывалась препятствием, включая точку старта Парижа.
    //
    // Недостижимую полость это не открывает: двор без арки остаётся дырой
    // РЕЗУЛЬТАТА, а её ниже отбрасывает `shape.first()` — то же, что делает
    // сеточный `prune_unreachable`. Дыру открывает только прорез: настил
    // моста, ведущий на остров, разрезает водное кольцо, и остров вместе с
    // прорезом становится частью внешнего контура.
    for area in input.buildings.iter().chain(&input.water) {
        push_contour(&mut blockers, area.outer.clone());
        for hole in &area.holes {
            push_hole(&mut blockers, hole.clone());
        }
    }
    // трубы полосы не имеют — над культвертом земля (как в заливке сетки)
    for line in &input.water_lines {
        if let Some(band) = line.channel_band()
            && let Some(ring) = ribbon_outline(&band.line, band.width)
        {
            push_contour(&mut blockers, ring);
        }
    }
    for wall in &input.walls {
        let band = wall.band();
        if let Some(ring) = ribbon_outline(&band.line, band.width) {
            push_contour(&mut blockers, ring);
        }
    }
    // бордюры мостов — те же две полосы, что рисует рендер
    // (`RoadLine::curb_bands`, общие с заливкой сетки).
    //
    // Блокирует не всякая полоса. OSM режет один физический мост на несколько
    // ways (проезжая часть и тротуар — параллельные ленты), и бордюр на
    // внутреннем шве такой пары запер бы мост поперёк. Сетка решает это щупом
    // «что снаружи» по тайлам; вектору доступна прямая формулировка того же
    // намерения — **полоса минус ленты всех ОСТАЛЬНЫХ bridge-ways**: то, что
    // накрыто соседней лентой, и есть внутренний шов, остальное — внешняя
    // граница composite-моста. Мостов на карте десятки, так что N разностей
    // дешевле одного union зданий.
    let coverage = crate::map::footprint::CurbCoverage::build(&input.roads);
    let bridges = coverage.bridges();
    let bands: Vec<Vec<[f32; 2]>> = bridges
        .iter()
        .filter_map(|road| ribbon_outline(&road.points, 2.0 * road.curb_reach()))
        .map(oriented)
        .collect();
    for (index, road) in bridges.iter().enumerate() {
        let mut sides: Vec<Vec<[f32; 2]>> = Vec::with_capacity(2);
        for band in road.curb_bands() {
            if let Some(ring) = ribbon_outline(&band.line, band.width) {
                sides.push(oriented(ring));
            }
        }
        let others: Vec<Vec<[f32; 2]>> = bands
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, band)| band.clone())
            .collect();
        for shape in sides.overlay(&others, OverlayRule::Difference, FillRule::NonZero) {
            blockers.extend(shape);
        }
    }

    let mut carves: Vec<Vec<[f32; 2]>> = Vec::new();
    for road in bridges {
        // настил — ровно проезжая часть, как её рисует рендер
        // (`RoadLine::deck_band`). Сеточное `+curb − tile·√2` сюда не
        // переносится: обе поправки компенсируют блуждание центров тайлов, а
        // полная ширина вместе с бордюром съела бы половину полосы, которая
        // обязана остаться барьером.
        let band = road.deck_band();
        if let Some(ring) = ribbon_outline(&band.line, band.width) {
            push_contour(&mut carves, ring);
        }
    }
    // примыкающая дорога открывает бордюр, который накрывает её панель;
    // береговая тропа в паре метров ПОД пролётом узла не делит и не
    // открывает ничего (список — общий с заливкой сетки, `CurbCoverage`)
    for road in coverage.joining() {
        if let Some(ring) = ribbon_outline(&road.points, road.width) {
            push_contour(&mut carves, ring);
        }
    }
    for road in input.roads.iter().filter(|road| road.passage) {
        let band = road.passage_band();
        if let Some(ring) = ribbon_outline(&band.line, band.width) {
            push_contour(&mut carves, ring);
        }
    }

    if is_cancelled() {
        return None;
    }
    // union(blockers) − union(carves) одним difference: NonZero при единой
    // закрутке контуров объединяет внутри subject и внутри clip сам
    let shapes = blockers.overlay(&carves, OverlayRule::Difference, FillRule::NonZero);
    let margin = MAP_EDGE_MARGIN;
    let rect = vec![vec![
        [-margin, -margin],
        [MAP_SIZE.x + margin, -margin],
        [MAP_SIZE.x + margin, MAP_SIZE.y + margin],
        [-margin, MAP_SIZE.y + margin],
    ]];
    let shapes = shapes.overlay(&rect, OverlayRule::Intersect, FillRule::NonZero);
    if is_cancelled() {
        return None;
    }

    // Радиус агента отрабатывается ЗДЕСЬ, а не в polyanya.
    //
    // `Triangulation::set_agent_radius` раздувает каждое кольцо препятствия
    // независимо (`input/triangulation.rs::inflate_obstacles` — `.map()` по
    // interiors) и **не объединяет** результат. На карте города это всегда
    // невалидный вход: соседние дома стоят в 30–40 см, раздутые на 0.2 м
    // контуры пересекаются, и CDT получает самопересекающийся набор
    // ограничений. Смежность в таком меше перестаёт соответствовать
    // геометрии, и воронка поиска на нём зацикливается — замерено на
    // `examples/polymesh_bench`: с радиусом 0.2 запрос №131 висит вечно и
    // съедает всю память, с радиусом 0 те же 300 запросов проходят.
    //
    // Свой офсет плюс union даёт снова непересекающийся набор: union заодно
    // разрешает самопересечения, которые miter даёт на острых вогнутых углах.
    let inflated: Vec<Vec<[f32; 2]>> = shapes
        .iter()
        // контур 0 — внешний; дыры результата difference — карманы, см. выше
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let ring: Vec<Vec2> = contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect();
            if agent_radius > 0.0 {
                oriented(inflate_ring(&ring, agent_radius))
            } else {
                oriented(ring)
            }
        })
        .collect();
    let nothing: Vec<Vec<[f32; 2]>> = Vec::new();
    let shapes = inflated.overlay(&nothing, OverlayRule::Subject, FillRule::NonZero);
    if is_cancelled() {
        return None;
    }

    // контуры придержаны для оверлея: polyanya хранит только проходимые
    // полигоны, а закрасить надо непроходимое — и закрасить именно то, что
    // блокирует на самом деле, то есть уже раздутое
    let obstacles: Vec<Vec<Vec2>> = shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(|contour| contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect())
        .collect();

    let grid = chunk_grid(MAP_SIZE, chunk_meters);
    let chunk_size = MAP_SIZE / grid.as_vec2();
    let chunking = Instant::now();
    // внешние контуры со своими bbox — по ним чанк отбирает то, что его
    // вообще задевает
    let contours: Vec<(Vec2, Vec2, Vec<[f32; 2]>)> = shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let (min, max) = contour.iter().fold(
                (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
                |(min, max), &[x, y]| (min.min(Vec2::new(x, y)), max.max(Vec2::new(x, y))),
            );
            (min, max, contour.clone())
        })
        .collect();
    let seams = seam_points(&contours, grid, chunk_size);
    let mut layers = Vec::with_capacity((grid.x * grid.y) as usize);
    for index in 0..grid.x * grid.y {
        if is_cancelled() {
            return None;
        }
        layers.push(chunk_layer(
            &contours,
            UVec2::new(index % grid.x, index / grid.x),
            chunk_size,
            &seams,
        ));
    }
    let polygons: usize = layers.iter().map(|layer| layer.polygons.len()).sum();
    // пустые слои считаются вслух: чанк, целиком накрытый препятствием (река,
    // сплошная застройка), полигонов не даёт — это законно, но такой слой
    // раньше убивал процесс в `Layer::bake`, и на Нью-Йорке их четыре
    let empty = layers
        .iter()
        .filter(|layer| layer.polygons.is_empty())
        .count();
    info!(
        "polymesh chunked into {}x{} layers ({polygons} polygons, {empty} layers fully blocked) \
         in {:?}",
        grid.x,
        grid.y,
        chunking.elapsed()
    );
    if is_cancelled() {
        return None;
    }

    // компоненты считаются ДО сшивки: она метит индексы в `vertex.polygons`
    // номером слоя, и обход по ним после неё уедет за границы своего слоя
    let components: Vec<ChunkComponents> = layers.iter().map(components_of).collect();

    let mut mesh = polyanya::Mesh {
        layers,
        ..Default::default()
    };
    // строго после слияния и строго ДО сшивки: `Layer::bake` требует
    // несшитый слой, а `merge_polygons` начинается с `unbake`.
    //
    // Без bake меш только рисуется: поиску он даёт линейный скан по всем
    // полигонам на каждый конец запроса вместо BVH.
    let baking = Instant::now();
    mesh.bake();
    info!("polymesh baked in {:?}", baking.elapsed());
    if is_cancelled() {
        return None;
    }

    let stitching = Instant::now();
    let graph = stitch_chunks(&mut mesh, grid, chunk_size, &components);
    info!(
        "polymesh stitched: {} seam vertices, {} component nodes, {} edges, {:?}",
        graph.seam_vertices,
        graph.nodes.len(),
        graph.edges.iter().map(Vec::len).sum::<usize>(),
        stitching.elapsed()
    );

    // допуск посадки концов запроса на меш. Дефолт polyanya —
    // `search_delta 0.1 × search_steps 2` = 0.2 м, ровно вровень с радиусом
    // агента, и этого не хватает: цель прогулки — вершина контура здания
    // (`human::pick_building_ahead`), а сетка объявляет тайл проходимым, если
    // его центр вне полигона хоть на сантиметр. Раздутый на радиус контур
    // такой центр накрывает, и цель оказывается вне меша. Замерено на Туле:
    // с дефолтным допуском отказывало 96% запросов против 3.5% у сетки.
    mesh.set_search_delta(SEARCH_DELTA);
    mesh.set_search_steps(SEARCH_STEPS);
    Some(PolymeshBuild {
        mesh,
        obstacles,
        grid,
        chunk_size,
        components,
        graph,
    })
}

/// Один чанк: препятствия, обрезанные его прямоугольником, триангулированные в
/// **мировых** координатах, слой с нулевым `offset`.
///
/// Мировые, а не локальные (как в образце сшивки из тестов polyanya), потому
/// что поиск сравнивает координаты вершин с концами интервала воронки как
/// `coords + offset`, точным равенством. У одной и той же точки шва суммы в
/// двух слоях с разными `offset` расходятся на младший разряд f32 — и воронка
/// перестаёт сходиться. С нулевым `offset` сравнивать нечего.
fn chunk_layer(
    contours: &[(Vec2, Vec2, Vec<[f32; 2]>)],
    cell: UVec2,
    chunk_size: Vec2,
    seams: &SeamPoints,
) -> polyanya::Layer {
    let origin = Vec2::new(
        quantized(cell.x as f32 * chunk_size.x),
        quantized(cell.y as f32 * chunk_size.y),
    );
    let far = Vec2::new(
        quantized((cell.x + 1) as f32 * chunk_size.x),
        quantized((cell.y + 1) as f32 * chunk_size.y),
    );
    let rect = vec![vec![
        [origin.x, origin.y],
        [far.x, origin.y],
        [far.x, far.y],
        [origin.x, far.y],
    ]];
    // только те контуры, чей bbox задевает чанк. Без этого отбора каждый из
    // 140 чанков резал бы весь набор из 7178 контуров: работа квадратична, а
    // память i_overlay пропорциональна входу — на этом приложение съедало
    // десяток гигабайт ещё до первого поиска
    let nearby: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .filter(|(min, max, _)| {
            min.x <= far.x && max.x >= origin.x && min.y <= far.y && max.y >= origin.y
        })
        .map(|(_, _, contour)| contour.clone())
        .collect();
    let clipped = nearby.overlay(&rect, OverlayRule::Intersect, FillRule::NonZero);

    // радиус агента уже вшит в контуры (см. `build_polymesh`), поэтому
    // `set_agent_radius` здесь не зовётся: он раздувает кольца по отдельности
    // и снова столкнул бы их.
    let mut triangulation =
        polyanya::Triangulation::from_outer_edges(&chunk_outline(cell, chunk_size, seams));
    for shape in &clipped {
        let Some(contour) = shape.first() else {
            continue;
        };
        triangulation.add_obstacle(
            contour
                .iter()
                .map(|&point| polyanya_glam::Vec2::new(quantized(point[0]), quantized(point[1]))),
        );
    }
    // а вот упрощение обязательно, и это проверено: без него те же 300
    // запросов виснут на 40-м, с ним доезжают до конца. Boolean оставляет
    // микроотрезки в доли миллиметра, и CDT на них вырождается
    triangulation.simplify(SIMPLIFY_EPSILON);

    let mut layer = triangulation.as_layer();
    // до сходимости, а не один раз: `merge_polygons` возвращает «слил хоть
    // что-то», и каждый проход открывает следующие пары — слитый выпуклый
    // полигон становится соседом, которым не был
    while layer.merge_polygons() {}
    layer.remove_useless_vertices();
    layer
}

/// Кольцо, смещённое наружу на `distance`, тем же miter-офсетом, которым
/// строятся ленты дорог. Сторона выбирается по площади: у смещённого наружу
/// кольца она по модулю больше, и это не зависит от исходной закрутки.
fn inflate_ring(ring: &[Vec2], distance: f32) -> Vec<Vec2> {
    let offsets = miter_offsets(ring, true, distance);
    let shift = |sign: f32| -> Vec<Vec2> {
        ring.iter()
            .zip(&offsets)
            .map(|(&point, &offset)| point + offset * sign)
            .collect()
    };
    let (outward, inward) = (shift(1.0), shift(-1.0));
    if signed_ring_area(&outward).abs() >= signed_ring_area(&inward).abs() {
        outward
    } else {
        inward
    }
}

/// Кольцо → контур i_overlay, нормализованный CCW: NonZero гасит контуры
/// противоположного обхода, а обход source-колец OSM произволен (тот же
/// приём, что у `buildings::layers::shadow_builder`).
fn oriented(mut ring: Vec<Vec2>) -> Vec<[f32; 2]> {
    if signed_ring_area(&ring) < 0.0 {
        ring.reverse();
    }
    ring.into_iter().map(|point| [point.x, point.y]).collect()
}

/// То же с отбраковкой вырожденных колец, прямо в накопитель.
fn push_contour(target: &mut Vec<Vec<[f32; 2]>>, ring: Vec<Vec2>) {
    if ring.len() < 3 {
        return;
    }
    target.push(oriented(ring));
}

/// Дыра кольца — тот же контур обратным обходом: при NonZero он гасит
/// заливку внутреннего кармана, но не мешает вложенным контурам (дом во
/// дворе снова поднимает обмотку до +1).
fn push_hole(target: &mut Vec<Vec<[f32; 2]>>, ring: Vec<Vec2>) {
    if ring.len() < 3 {
        return;
    }
    let mut contour = oriented(ring);
    contour.reverse();
    target.push(contour);
}

/// Замкнутый контур ленты постоянной ширины вдоль открытой ломаной:
/// `p + o` вперёд, `p − o` в обратном порядке. Та же кромка, что у
/// `RoadJoin::Miter`-отрисовки и у бордюров в заливке сетки.
fn ribbon_outline(path: &[Vec2], width: f32) -> Option<Vec<Vec2>> {
    if path.len() < 2 {
        return None;
    }
    let offsets = miter_offsets(path, false, width / 2.0);
    let mut ring: Vec<Vec2> = path
        .iter()
        .zip(&offsets)
        .map(|(&point, &offset)| point + offset)
        .collect();
    ring.extend(
        path.iter()
            .zip(&offsets)
            .rev()
            .map(|(&point, &offset)| point - offset),
    );
    Some(ring)
}
