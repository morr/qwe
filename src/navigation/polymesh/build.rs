//! Постройка меша: контуры препятствий → boolean-объединение → CDT polyanya,
//! чанк за чанком. Швами занимается [`super::seams`], поиском — [`super::path`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use super::seams::{SeamPoints, chunk_outline, node_world, quantized, seam_points};
use super::stitch::{components_of, stitch_chunks};
use super::{ChunkComponents, PolymeshBuild, PolymeshInput, chunk_cell, chunk_grid};
use crate::map::footprint::{CurbCoverage, JOIN_EPSILON, fence_gaps};
use crate::map::osm::model::{signed_ring_area, water_line_caps};
use crate::map::parallel::in_parallel;
use crate::map::shadow::{difference_of_shapes, union_of};
use crate::map::{merge_close_points, miter_offsets};
use crate::settings::{
    MAP_SIZE, POLYMESH_MAP_EDGE_MARGIN, POLYMESH_SEARCH_DELTA, POLYMESH_SEARCH_STEPS,
};

/// Упрощение контуров препятствий (Visvalingam–Whyatt внутри polyanya),
/// метры. Не косметика: boolean оставляет отрезки в доли миллиметра, CDT на
/// них вырождается, и поиск по такому мешу зацикливается — замерено на
/// `examples/polymesh_bench`, без упрощения прогон встаёт на 40-м запросе.
const SIMPLIFY_EPSILON: f32 = 0.05;

/// Шаг ломаной, ниже которого точка схлопывается с предыдущей перед
/// miter-офсетом, метры. На нулевом шаге `miter_offsets` подставляет запасную
/// ось `Vec2::X` вместо нормали, и офсет вершины разворачивается — до 180° и до
/// `MITER_LIMIT · half_width` длиной. Нулевой шаг здесь берётся не из OSM (на
/// семи кешах ни одного дубля подряд, минимальный шаг 1.3 см), а из f32: сетка
/// i_overlay мельче шага f32 на масштабе карты, поэтому микроребро boolean
/// (те самые «доли миллиметра» из [`SIMPLIFY_EPSILON`]) приходит парой
/// одинаковых вершин, а бордюрная полоса моста — производная от тех же
/// офсетов — на Париже уже даёт шаг 0.49 мм. Сантиметр: тот же порядок, что у
/// `SEAM_QUANTUM` («на порядок больше разрешения f32 на краю карты»), впятеро
/// мельче упрощения и на два порядка мельче навтайла, так что оба заполнения
/// остаются согласованными. Эпсилон рендера (`width / 4`) сюда не годится: на
/// primary это 4 м, и футпринт меша уехал бы от футпринта сетки.
const DEGENERATE_SPAN: f32 = 0.01;

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

    let coverage = CurbCoverage::build(&input.roads);
    let blockers = blocker_shapes(input, &coverage);
    let carves = carve_contours(input, &coverage);

    if is_cancelled() {
        return None;
    }
    // union(blockers) − union(carves) — разностью по клеткам
    // (`shadow::difference_of_shapes`, то же разложение, что у вычитания
    // настилов из теней заборов): фигуры едут в клетку целиком, дыры —
    // вместе со своим наружным кольцом. Результат — фигуры, как у прежнего
    // одного `Difference`-оверлея.
    let differed = difference_of_shapes(&blockers, &carves);
    let margin = POLYMESH_MAP_EDGE_MARGIN;
    let rect = vec![vec![
        [-margin, -margin],
        [MAP_SIZE.x + margin, -margin],
        [MAP_SIZE.x + margin, MAP_SIZE.y + margin],
        [-margin, MAP_SIZE.y + margin],
    ]];
    // Отсечка прямоугольником карты — точечно, не всем городом: пересечение
    // с прямоугольником дистрибутивно по объединению, поэтому фигура целиком
    // внутри проходит как есть, а оверлей платят те, чья рамка его задела
    // (полосы водотоков, стен и оград у кромки выгрузки).
    let rect_box = (
        Vec2::new(-margin, -margin),
        Vec2::new(MAP_SIZE.x + margin, MAP_SIZE.y + margin),
    );
    let (inside, crossing): (Vec<_>, Vec<_>) = differed.into_iter().partition(|shape| {
        let (min, max) = shape.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(min, max), contour| {
                let (cmin, cmax) = contour_bounds(contour);
                (min.min(cmin), max.max(cmax))
            },
        );
        min.x >= rect_box.0.x
            && min.y >= rect_box.0.y
            && max.x <= rect_box.1.x
            && max.y <= rect_box.1.y
    });
    let shapes = if crossing.is_empty() {
        inside
    } else {
        crossing
            .overlay(&rect, OverlayRule::Intersect, FillRule::NonZero)
            .into_iter()
            .chain(inside)
            .collect()
    };
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
        // контур 0 — внешний; дыры результата разности — карманы, см. выше
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let ring: Vec<Vec2> = contour.iter().map(|&p| Vec2::new(p[0], p[1])).collect();
            if agent_radius > 0.0 {
                oriented(inflate_ring(&ring, agent_radius))
            } else {
                oriented(ring)
            }
        })
        // после схлопывания обмылок boolean может не дотянуть до трёх вершин —
        // он и до этого ничего не блокировал, но в overlay ему делать нечего
        .filter(|contour| contour.len() >= 3)
        .collect();
    // Объединение надутых колец — по клеткам (`shadow::union_of`, примитив
    // `push_union`): тот же вход, что раньше шёл одним `Subject`-оверлеем.
    // Вход без дыр — надутые наружные кольца, поэтому клетка их не разлучает.
    let shapes = union_of(&inflated);
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
    let layers = chunk_layers(&shapes, grid, chunk_size, is_cancelled)?;
    if is_cancelled() {
        return None;
    }

    // компоненты считаются ДО сшивки: она метит индексы в `vertex.polygons`
    // номером слоя, и обход по ним после неё уедет за границы своего слоя.
    // Слой от слоя не зависит (`components_of` читает только свой слой) —
    // по потокам, порядок слоёв тот же
    let components: Vec<ChunkComponents> = in_parallel(&layers, components_of);

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
    // слои независимы (сшивка метит индексы позже), а bake слоя — только
    // чтение его вершин и полигонов: считаем по слоям в потоках, применяем
    // серийно (`compute_bake`/`apply_bake`) — порядок слоёв и результат те же
    let baked = in_parallel(&mesh.layers, |layer| layer.compute_bake());
    for (layer, parts) in mesh.layers.iter_mut().zip(baked) {
        layer.apply_bake(parts);
    }
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
    // `search_delta 0.1 × (search_steps 2 − 1)` = 0.1 м, вдвое меньше
    // минимального радиуса агента, и этого не хватает: цель прогулки — вершина контура здания
    // (`human::pick_building_ahead`), а сетка объявляет тайл проходимым, если
    // его центр вне полигона хоть на сантиметр. Раздутый на радиус контур
    // такой центр накрывает, и цель оказывается вне меша. Замерено на Туле:
    // с дефолтным допуском отказывало 96% запросов против 3.5% у сетки.
    mesh.set_search_delta(POLYMESH_SEARCH_DELTA);
    mesh.set_search_steps(POLYMESH_SEARCH_STEPS);
    Some(PolymeshBuild {
        mesh,
        obstacles,
        grid,
        chunk_size,
        components,
        graph,
    })
}

/// Фигуры, которые БЛОКИРУЮТ: здания и вода со своими дырами, русла
/// некульвертных водотоков, стены, ограды минус их проёмы и внешние бордюры
/// мостов (полоса минус ленты соседних мостов и полотна примыкающих дорог).
/// Фигуры, а не
/// плоские контуры: дыра обязана доехать до клеточной разности
/// (`shadow::difference_of_shapes`) в одной группе со своим наружным
/// кольцом — одинокая дыра в чужой клетке под NonZero заполнена, то есть
/// препятствие там, где был вырез (остров в реке — дыра внешнего кольца
/// воды). Порядок накопления — часть контракта: объединение и вычитание
/// прорезов идут одним `overlay` в [`build_polymesh`].
fn blocker_shapes(input: &PolymeshInput, coverage: &CurbCoverage<'_>) -> Vec<Vec<Vec<[f32; 2]>>> {
    let mut blockers: Vec<Vec<Vec<[f32; 2]>>> = Vec::new();
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
        let mut shape = vec![oriented(area.outer.clone())];
        shape.extend(
            area.holes
                .iter()
                .map(|hole| hole_contour(hole.clone()))
                .filter(|contour| !contour.is_empty()),
        );
        blockers.push(shape);
    }
    // трубы полосы не имеют — над культвертом земля (как в заливке сетки)
    for line in &input.water_lines {
        if let Some(band) = line.channel_band() {
            // торцы — те же, что у сеточной капсулы: свободный конец круглый,
            // у входа в трубу срезан (`water_line_caps`)
            let [start, end] = water_line_caps(line, &input.water_lines);
            let path = extended_ends_capped(&band.line, band.width / 2.0, [start, end]);
            if let Some(ring) = ribbon_outline(&path, band.width) {
                blockers.push(vec![oriented(ring)]);
            }
        }
    }
    for wall in &input.walls {
        let band = wall.band();
        // оба торца круглые — у сеточной капсулы (`set_polyline`) исключений нет
        let line = extended_ends(&band.line, band.width / 2.0);
        if let Some(ring) = ribbon_outline(&line, band.width) {
            blockers.push(vec![oriented(ring)]);
        }
    }
    // ограды — полоса минус проёмы дорог (`footprint::fence_gaps`), одной
    // разностью на все. Проём не уходит в общие прорезы: те вычитаются из
    // объединения всех препятствий и открыли бы дом или воду, которых тропа
    // касается; здесь он вынимается только из самого забора — маска сетки
    // (`Navmesh::fill_fences`) делает то же самое по тайлам
    let gaps = fence_gaps(&input.fences, &input.roads);
    let mut fences: Vec<Vec<Vec<[f32; 2]>>> = Vec::new();
    let mut openings: Vec<Vec<[f32; 2]>> = Vec::new();
    for (fence, gaps) in input.fences.iter().zip(&gaps) {
        let band = fence.band();
        if let Some(ring) = ribbon_outline(&band.line, band.width) {
            fences.push(vec![oriented(ring)]);
        }
        for gap in gaps {
            push_contour(&mut openings, gap_outline(gap.at, gap.reach));
        }
    }
    if openings.is_empty() {
        blockers.extend(fences);
    } else {
        // та же клеточная разность, что и главная
        // (`shadow::difference_of_shapes`): город оград против города
        // проёмов одним проходом на Париже — четверть секунды, по клеткам —
        // десятки миллисекунд
        blockers.extend(difference_of_shapes(&fences, &openings));
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
    // граница composite-моста.
    //
    // Вычитать стоит только ленты, чья рамка задевает бордюры моста:
    // разность с непересекающейся лентой — тождество. Без отбора разность
    // шла против лент всех мостов карты — на Париже 408 мостов, и N разностей
    // стоили дороже одного union зданий; с рамками платят пары, у которых
    // есть чем пересечься (тот же отбор, что `boxes_may_join` у
    // `CurbCoverage`, — десятки мостов против единиц пересечений).
    let bridges = coverage.bridges();
    let bands: Vec<Option<((Vec2, Vec2), Vec<[f32; 2]>)>> = bridges
        .iter()
        .map(|road| {
            ribbon_outline(&road.points, 2.0 * road.curb_reach()).map(|ring| {
                let ring = oriented(ring);
                (contour_bounds(&ring), ring)
            })
        })
        .collect();
    // полотно примыкающей дороги открывает только бордюр, который накрывает
    // его панель, — как в сетке, где оно лишь снимает блокировку с бордюрных
    // тайлов под собой (`fill_base`, `tile.road`). Общей прорезью лента
    // вычиталась из всех препятствий разом и проходила сквозь воду, дома и
    // стены вдоль дороги (под примыкающими стояло 1 973 тайла домов и 641
    // воды в Париже, 630 и 33 в Москве). Ширина без сеточного запаса на
    // диагональ тайла: блуждания тайловых цепочек у вектора нет. Береговая
    // тропа в паре метров ПОД пролётом узла не делит и не открывает ничего —
    // список общий с заливкой сетки, `CurbCoverage`
    let joining: Vec<((Vec2, Vec2), Vec<[f32; 2]>)> = coverage
        .joining()
        .iter()
        .filter_map(|road| {
            let ring = oriented(ribbon_outline(&road.points, road.width)?);
            Some((contour_bounds(&ring), ring))
        })
        .collect();
    for (index, road) in bridges.iter().enumerate() {
        let mut sides: Vec<Vec<[f32; 2]>> = Vec::with_capacity(2);
        for band in road.curb_bands() {
            if let Some(ring) = ribbon_outline(&band.line, band.width) {
                sides.push(oriented(ring));
            }
        }
        let sides_box = sides.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(min, max), ring| {
                let (ring_min, ring_max) = contour_bounds(ring);
                (min.min(ring_min), max.max(ring_max))
            },
        );
        let others: Vec<Vec<[f32; 2]>> = bands
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, band)| band.as_ref())
            .chain(joining.iter())
            .filter(|(box_, _)| bands_may_overlap(*box_, sides_box))
            .map(|(_, band)| band.clone())
            .collect();
        if others.is_empty() {
            blockers.extend(sides.iter().map(|ring| vec![ring.clone()]));
        } else {
            blockers.extend(sides.overlay(&others, OverlayRule::Difference, FillRule::NonZero));
        }
    }
    blockers
}

/// Рамки двух лент задевают друг друга с запасом [`JOIN_EPSILON`] — тем же,
/// которым `CurbCoverage` судит соседство ways одного физического моста.
fn bands_may_overlap(first: (Vec2, Vec2), second: (Vec2, Vec2)) -> bool {
    (first.0 - JOIN_EPSILON).cmple(second.1).all() && (second.0 - JOIN_EPSILON).cmple(first.1).all()
}

/// Габарит контура: минимум и максимум вершин.
fn contour_bounds(contour: &[[f32; 2]]) -> (Vec2, Vec2) {
    contour.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(min, max), &[x, y]| (min.min(Vec2::new(x, y)), max.max(Vec2::new(x, y))),
    )
}

/// Контуры, которые ПРОРЕЗАЮТ блокирующие: настилы мостов и проезды-арки.
/// Примыкающая дорога сюда не входит: она открывает только бордюр, и её
/// лента вычитается из полос бордюра в [`blocker_shapes`].
fn carve_contours(input: &PolymeshInput, coverage: &CurbCoverage<'_>) -> Vec<Vec<[f32; 2]>> {
    let bridges = coverage.bridges();
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
    for road in input.roads.iter().filter(|road| road.passage) {
        let band = road.passage_band();
        let line = extended_ends(&band.line, band.width / 2.0);
        if let Some(ring) = ribbon_outline(&line, band.width) {
            push_contour(&mut carves, ring);
        }
    }
    carves
}

/// Ломаная, продолженная за оба конца на `reach` вдоль своих крайних звеньев.
///
/// Так же заканчиваются стены и свободные торцы водотоков (`blocker_shapes`):
/// капсула сетки круглая с обоих концов, а срезанная лента меша оставляла за
/// торцом ручья клин суши в воде и проход за концом стены.
///
/// Арка сетки — капсула (`set_polyline`: тайлы в пределах полуширины от
/// отрезка), и за концы осевой она выходит на полуширину; лента меша
/// обрезана по концам ровно. Разница не косметическая: осевая арки в OSM
/// кончается на контуре здания, а у ворот Тульского кремля в том же месте
/// проходят и лента `barrier=city_wall`, и соседнее прясло `building=wall`.
/// Ровный срез оставлял у устья тонкую перемычку препятствия, коридор замыкался
/// в дыру объединения — и весь кремль уходил в препятствие (`shape.first()`
/// выбрасывает дыры), тогда как сетка его пропускала.
fn extended_ends(line: &[Vec2], reach: f32) -> Vec<Vec2> {
    extended_ends_capped(line, reach, [true; 2])
}

/// То же по концам раздельно: `false` — торец остаётся срезанным ровно.
fn extended_ends_capped(line: &[Vec2], reach: f32, round: [bool; 2]) -> Vec<Vec2> {
    let mut line = line.to_vec();
    let count = line.len();
    if count < 2 {
        return line;
    }
    if round[0]
        && let Some(lead) = (line[0] - line[1]).try_normalize()
    {
        line[0] += lead * reach;
    }
    if round[1]
        && let Some(tail) = (line[count - 1] - line[count - 2]).try_normalize()
    {
        line[count - 1] += tail * reach;
    }
    line
}

/// Нарезка на слои: внешние контуры со своими bbox, глобальные точки швов, по
/// слою на чанк. `None` — постройку отменили посреди цикла.
///
/// Чанки считаются по потокам (`map::parallel::in_parallel`): входы — только
/// чтение (`contours`, `seams`, прямоугольник), у каждого слоя свой выход, а
/// порядок результатов совпадает с порядком индексов чанков — слои в меше
/// обязаны идти в порядке индексов (8-битный индекс слоя полианьи, граф
/// компонентов, коридор). Отмена проверяется перед чанком, как у
/// последовательного цикла: работник видит флаг, взяв следующий чанк.
fn chunk_layers(
    shapes: &[Vec<Vec<[f32; 2]>>],
    grid: UVec2,
    chunk_size: Vec2,
    is_cancelled: impl Fn() -> bool + Sync,
) -> Option<Vec<polyanya::Layer>> {
    let chunking = Instant::now();
    // внешние контуры со своими bbox — по ним чанк отбирает то, что его
    // вообще задевает
    let contours: Vec<(Vec2, Vec2, Vec<[f32; 2]>)> = shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(|contour| {
            let (min, max) = contour_bounds(contour);
            (min, max, contour.clone())
        })
        .collect();
    let seams = seam_points(&contours, grid, chunk_size);
    let cells: Vec<UVec2> = (0..grid.x * grid.y)
        .map(|index| chunk_cell(grid, index as usize))
        .collect();
    let layers: Vec<Option<polyanya::Layer>> = in_parallel(&cells, |cell| {
        if is_cancelled() {
            None
        } else {
            Some(chunk_layer(&contours, *cell, chunk_size, &seams))
        }
    });
    if is_cancelled() {
        return None;
    }
    // флаг не горел — работники вернули все слои
    let layers = layers.into_iter().collect::<Option<Vec<_>>>()?;
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
    Some(layers)
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
    let origin = node_world(cell, chunk_size);
    let far = node_world(cell + UVec2::ONE, chunk_size);
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
                .map(|&[x, y]| Vec2::new(quantized(x), quantized(y))),
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
///
/// Кольцо приходит из boolean, а i_overlay отдаёт f32 с шага мельче, чем f32
/// различает на масштабе карты: микроребро становится парой одинаковых вершин,
/// и без схлопывания ([`DEGENERATE_SPAN`]) угол на ней не раздувается, а
/// срезается внутрь барьера на весь радиус агента.
pub(super) fn inflate_ring(ring: &[Vec2], distance: f32) -> Vec<Vec2> {
    let ring = merge_close_points(ring, true, DEGENERATE_SPAN);
    let offsets = miter_offsets(&ring, true, distance);
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
/// приём, что у `buildings::shadows::shadow_builder`).
fn oriented(mut ring: Vec<Vec2>) -> Vec<[f32; 2]> {
    if signed_ring_area(&ring) < 0.0 {
        ring.reverse();
    }
    ring.into_iter().map(|point| [point.x, point.y]).collect()
}

/// Сторон у многоугольника проёма в ограде.
const GAP_SIDES: usize = 16;

/// Проём в ограде — круг радиуса `reach` вокруг точки пересечения, тот же, по
/// которому сетка вынимает тайлы из маски заборов (без её запаса на
/// растеризацию). Многоугольник описан вокруг круга, а не вписан: вписанный
/// оставлял бы у забора волосок барьера по краю проёма.
fn gap_outline(at: Vec2, reach: f32) -> Vec<Vec2> {
    let circumscribed = reach / (std::f32::consts::PI / GAP_SIDES as f32).cos();
    (0..GAP_SIDES)
        .map(|side| {
            let angle = side as f32 * std::f32::consts::TAU / GAP_SIDES as f32;
            at + Vec2::from_angle(angle) * circumscribed
        })
        .collect()
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
fn hole_contour(ring: Vec<Vec2>) -> Vec<[f32; 2]> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let mut contour = oriented(ring);
    contour.reverse();
    contour
}

/// Замкнутый контур ленты постоянной ширины вдоль открытой ломаной:
/// `p + o` вперёд, `p − o` в обратном порядке. Те же офсеты и те же ширины
/// футпринта, что у `RoadJoin::Miter`-отрисовки и у бордюров в заливке сетки, —
/// но по сырой осевой: рендер рисует свою сглаженную копию (`map::footprint`,
/// док модуля), поэтому вершина в вершину лента совпадает только без
/// сглаживания.
///
/// Вырожденные шаги схлопываются ([`DEGENERATE_SPAN`]) — иначе нулевой сегмент
/// разворачивает офсет на стыке.
pub(super) fn ribbon_outline(path: &[Vec2], width: f32) -> Option<Vec<Vec2>> {
    let path = merge_close_points(path, false, DEGENERATE_SPAN);
    if path.len() < 2 {
        return None;
    }
    let offsets = miter_offsets(&path, false, width / 2.0);
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
