//! Швы между чанками: подразбиение общих кромок, компоненты связности слоя и
//! сшивка соседних слоёв в один меш с графом переходов.

use bevy::prelude::*;

use super::{ChunkComponents, ChunkGraph, GraphNode, from_poly, to_poly};

/// Шаг подразбиения кромки чанка, метры.
///
/// Без подразбиения на шве совпадают только углы чанка: CDT ставит вершины на
/// внешнем контуре лишь там, где его касается препятствие. Сшитые по двум
/// точкам полигоны становятся «соседями» через **точку**, а не через общее
/// ребро, и поиск идёт по мусорной смежности — замерено: 140 ГБ за считанные
/// секунды. Подразбиение даёт обеим сторонам совпадающие вершины (чанки
/// одинакового размера, точки кратны шагу от общего угла), то есть цепочку
/// настоящих общих рёбер. Интервал видимости переносится через ребро
/// целиком, поэтому шаг задаёт не точность пути, а лишь дробность цепочки;
/// геометрию проходов вдоль шва по-прежнему задают вершины препятствий.
/// `Triangulation::simplify` внешний контур не трогает (только `interiors`),
/// так что подразбиение переживает упрощение.
const SEAM_STEP_METERS: f32 = 20.0;

/// Шаг сетки, на которую сажаются координаты обрезанных контуров, метры.
///
/// Общую кромку два соседа режут порознь, и i_overlay выдаёт им точки,
/// различающиеся в младших разрядах f32. Такая пара не находит друг друга при
/// сшивке, и вершина остаётся на шве только с одной стороны. Переход через шов
/// тогда односторонний: из соседа внутрь общий полигон находится (он содержит
/// оба конца ребра), а обратно — нет, потому что лишняя вершина разбила ребро
/// надвое. На этой асимметрии воронка перестаёт сходиться, очередь растёт без
/// предела, процесс умирает от OOM. Округление на общую сетку убирает
/// расхождение в источнике; сантиметр — на два порядка меньше упрощения
/// (`SIMPLIFY_EPSILON`) и на порядок больше разрешения f32 на краю карты.
const SEAM_QUANTUM: f32 = 0.01;

pub(super) fn quantized(value: f32) -> f32 {
    (value / SEAM_QUANTUM).round() * SEAM_QUANTUM
}

/// Точка подразбиения кромки чанка — **функция от номера узла сетки и номера
/// шага**, а не от координат конкретного чанка.
///
/// Это принципиально. Общую кромку два соседа считают каждый со своей стороны,
/// и если бы точка бралась как `origin + lerp(...)`, у левого соседа вышло бы
/// `x*s + s`, а у правого `(x+1)*s` — в f32 это разные числа. Поиск же хранит
/// интервал воронки в мировых координатах и сверяет его концы с координатами
/// вершин, так что расхождение в младшем разряде на каждом шве не даёт воронке
/// сходиться: она обходит одну и ту же область по кругу, очередь растёт без
/// предела, и процесс умирает от OOM (замерено: минута на карте из двух стен).
fn seam_point(node: u32, step: u32, steps: u32, chunk_size: f32) -> f32 {
    node as f32 * chunk_size + step as f32 / steps as f32 * chunk_size
}

/// Сколько отрезков подразбиения приходится на одну сторону чанка.
fn seam_steps(side: f32) -> u32 {
    (side / SEAM_STEP_METERS).ceil().max(1.0) as u32
}

/// Точки, где контуры препятствий пересекают линии сетки чанков.
///
/// Считаются один раз на всю карту и раздаются обоим соседям, поэтому кромку
/// оба получают с одинаковым набором вершин. Без этого препятствие, упёршееся
/// в шов только с одной стороны, оставляет вершину лишь в своём чанке: у
/// соседа ребро шва цельное, у него — разбитое надвое. Переход тогда
/// односторонний (из соседа внутрь общий полигон находится, обратно нет), и на
/// этой асимметрии воронка перестаёт сходиться.
#[derive(Default)]
pub(super) struct SeamPoints {
    /// координаты вдоль линии `x = node * chunk_size.x`, по индексу узла
    vertical: Vec<Vec<f32>>,
    /// координаты вдоль линии `y = node * chunk_size.y`, по индексу узла
    horizontal: Vec<Vec<f32>>,
}

pub(super) fn seam_points(
    contours: &[(Vec2, Vec2, Vec<[f32; 2]>)],
    grid: UVec2,
    chunk_size: Vec2,
) -> SeamPoints {
    let mut seams = SeamPoints {
        vertical: vec![Vec::new(); grid.x as usize + 1],
        horizontal: vec![Vec::new(); grid.y as usize + 1],
    };
    let cross = |from: f32, to: f32, other_from: f32, other_to: f32, line: f32| -> Option<f32> {
        if (from < line) == (to < line) {
            return None;
        }
        let ratio = (line - from) / (to - from);
        Some(quantized(other_from + (other_to - other_from) * ratio))
    };
    for (min, max, contour) in contours {
        for index in 0..contour.len() {
            let a = Vec2::from(contour[index]);
            let b = Vec2::from(contour[(index + 1) % contour.len()]);
            for node in 0..=grid.x {
                let line = quantized(node as f32 * chunk_size.x);
                if min.x > line || max.x < line {
                    continue;
                }
                if let Some(value) = cross(a.x, b.x, a.y, b.y, line) {
                    seams.vertical[node as usize].push(value);
                }
            }
            for node in 0..=grid.y {
                let line = quantized(node as f32 * chunk_size.y);
                if min.y > line || max.y < line {
                    continue;
                }
                if let Some(value) = cross(a.y, b.y, a.x, b.x, line) {
                    seams.horizontal[node as usize].push(value);
                }
            }
        }
    }
    for line in seams.vertical.iter_mut().chain(seams.horizontal.iter_mut()) {
        line.sort_by(f32::total_cmp);
        line.dedup();
    }
    seams
}

/// Внутренние точки одной стороны чанка: подразбиение плюс пересечения
/// препятствий с этой линией сетки. Оба конца исключены — их ставит кольцо.
fn side_points(node: u32, span: f32, steps: u32, crossings: &[f32]) -> Vec<f32> {
    let (low, high) = (
        quantized(node as f32 * span),
        quantized((node + 1) as f32 * span),
    );
    let mut points: Vec<f32> = (1..steps)
        .map(|step| quantized(seam_point(node, step, steps, span)))
        .collect();
    points.extend(
        crossings
            .iter()
            .copied()
            .filter(|value| *value > low && *value < high),
    );
    points.sort_by(f32::total_cmp);
    points.dedup();
    points
}

/// Контур чанка в мировых координатах, обход против часовой. Каждая сторона
/// считается в каноническом направлении (по возрастанию координаты) и при
/// необходимости разворачивается — иначе у двух соседей одна и та же кромка
/// вышла бы разными числами.
pub(super) fn chunk_outline(
    cell: UVec2,
    chunk_size: Vec2,
    seams: &SeamPoints,
) -> Vec<polyanya_glam::Vec2> {
    let (steps_x, steps_y) = (seam_steps(chunk_size.x), seam_steps(chunk_size.y));
    let xs = |node_y: u32| {
        side_points(
            cell.x,
            chunk_size.x,
            steps_x,
            &seams.horizontal[node_y as usize],
        )
    };
    let ys = |node_x: u32| {
        side_points(
            cell.y,
            chunk_size.y,
            steps_y,
            &seams.vertical[node_x as usize],
        )
    };
    let corner = |node_x: u32, node_y: u32| {
        Vec2::new(
            quantized(node_x as f32 * chunk_size.x),
            quantized(node_y as f32 * chunk_size.y),
        )
    };

    let (low_x, low_y) = (cell.x, cell.y);
    let (high_x, high_y) = (cell.x + 1, cell.y + 1);
    let mut outline: Vec<Vec2> = vec![corner(low_x, low_y)];
    outline.extend(
        xs(low_y)
            .into_iter()
            .map(|x| Vec2::new(x, corner(0, low_y).y)),
    );
    outline.push(corner(high_x, low_y));
    outline.extend(
        ys(high_x)
            .into_iter()
            .map(|y| Vec2::new(corner(high_x, 0).x, y)),
    );
    outline.push(corner(high_x, high_y));
    outline.extend(
        xs(high_y)
            .into_iter()
            .rev()
            .map(|x| Vec2::new(x, corner(0, high_y).y)),
    );
    outline.push(corner(low_x, high_y));
    outline.extend(
        ys(low_x)
            .into_iter()
            .rev()
            .map(|y| Vec2::new(corner(low_x, 0).x, y)),
    );
    outline.into_iter().map(to_poly).collect()
}

/// Компоненты связности слоя: flood fill по соседству полигонов через общие
/// вершины. Считать можно только **до** сшивки — она метит индексы в
/// `vertex.polygons` номером слоя, и обход по ним уедет за границы массива.
/// Своё, а не `Layer::bake_islands_detection`: его результат лежит в
/// `pub(crate)` поле и наружу не отдаётся.
pub(super) fn components_of(layer: &polyanya::Layer) -> ChunkComponents {
    let count = layer.polygons.len();
    let mut of_polygon = vec![u32::MAX; count];
    let mut centers: Vec<Vec2> = Vec::new();

    for root in 0..count {
        if of_polygon[root] != u32::MAX {
            continue;
        }
        let id = centers.len() as u32;
        of_polygon[root] = id;
        let mut sum = Vec2::ZERO;
        let mut members = 0.0;
        let mut stack = vec![root];

        while let Some(current) = stack.pop() {
            let polygon = &layer.polygons[current];
            let mut center = Vec2::ZERO;
            let mut corners = 0.0;
            let count = polygon.vertices.len();
            for index in 0..count {
                let first = polygon.vertices[index] as usize;
                let Some(vertex) = layer.vertices.get(first) else {
                    continue;
                };
                center += from_poly(vertex.coords);
                corners += 1.0;
                // сосед — только через РЕБРО: polyanya переносит поиск через
                // общее ребро, и два полигона, соприкоснувшиеся одной
                // вершиной (защемление на углу препятствия), для неё не
                // связаны. Заливка по вершинам склеивала бы их в одну
                // компоненту, граф обещал бы проход, поиск бы его не нашёл —
                // и выжег бы весь бюджет итераций
                let second = polygon.vertices[(index + 1) % count] as usize;
                let Some(neighbour) = shared_polygon_excluding(layer, first, second, current)
                else {
                    continue;
                };
                if of_polygon.get(neighbour) == Some(&u32::MAX) {
                    of_polygon[neighbour] = id;
                    stack.push(neighbour);
                }
            }
            if corners > 0.0 {
                sum += center / corners;
                members += 1.0;
            }
        }
        centers.push(if members > 0.0 {
            sum / members
        } else {
            Vec2::ZERO
        });
    }

    ChunkComponents {
        of_polygon,
        centers,
    }
}

/// Полигон, которому принадлежат обе вершины, — то есть их общее ребро.
/// Считать можно только до сшивки: она метит индексы номером слоя.
fn shared_polygon(layer: &polyanya::Layer, first: usize, second: usize) -> Option<usize> {
    shared_polygon_excluding(layer, first, second, usize::MAX)
}

/// Есть ли между вершинами **ребро**, а не просто общий полигон: в кольце
/// полигона они должны стоять рядом. Ровно так ребро видит и поиск —
/// разделённые третьей вершиной концы никакого перехода не дают.
fn shared_edge(layer: &polyanya::Layer, first: usize, second: usize) -> bool {
    let Some(vertex) = layer.vertices.get(first) else {
        return false;
    };
    vertex.polygons.iter().any(|&polygon| {
        polygon != u32::MAX
            && layer.polygons.get(polygon as usize).is_some_and(|ring| {
                let ring = &ring.vertices;
                (0..ring.len()).any(|at| {
                    let pair = (ring[at] as usize, ring[(at + 1) % ring.len()] as usize);
                    pair == (first, second) || pair == (second, first)
                })
            })
    })
}

/// То же, но мимо заданного полигона — для обхода соседей: у ребра их два, и
/// нужен тот, с которого не начинали.
fn shared_polygon_excluding(
    layer: &polyanya::Layer,
    first: usize,
    second: usize,
    skip: usize,
) -> Option<usize> {
    let others = &layer.vertices.get(second)?.polygons;
    layer
        .vertices
        .get(first)?
        .polygons
        .iter()
        .find(|&&polygon| {
            polygon != u32::MAX && polygon as usize != skip && others.contains(&polygon)
        })
        .map(|&polygon| polygon as usize)
}

/// Допуск совпадения вершин на шве, метры. Оба соседа режут одни и те же
/// глобальные контуры одной и той же прямой, так что координаты обязаны
/// совпадать; допуск покрывает только потерю точности f32 при переносе в
/// локальные координаты чанка.
const SEAM_EPSILON: f32 = 1e-3;

/// Итог разбора непарных вершин одного шва.
struct Unstitched {
    /// номера пар в `pairs`, стежок которых пришлось отменить
    dropped: Vec<usize>,
    /// сколько непарных вершин оказались опасными
    dangerous: usize,
    /// первая из них: точка и слой, у которого её нет. Собирается **только в
    /// отладочной сборке** — читает её один `debug_assert!`, в релизе поле
    /// не существует и заполнять его незачем
    #[cfg(debug_assertions)]
    first: Option<(Vec2, usize)>,
}

/// Разбор непарных вершин одного шва: какие стежки придётся отменить.
///
/// Опасна не всякая непарная вершина, а лежащая **строго внутри отрезка,
/// который у соседа остался цельным ребром, притом что у нас концы этого
/// отрезка лежат в одном полигоне**. Сшивка склеивает списки полигонов в самих
/// вершинах, так что оба конца отдадут соседу этот общий полигон, и его цельное
/// ребро найдёт переход в наш слой — а наши две половинки того же ребра
/// обратного перехода не найдут, потому что лишняя вершина между ними ничего не
/// получила. Односторонний шов и есть то, на чём воронка перестаёт сходиться
/// (см. `verify_seams`).
///
/// Отменяется стежок обоих концов: сшивка адресует вершины, а не рёбра, и
/// «не переносить поиск через этот отрезок» выражается только так. Соседние
/// отрезки при этом тоже теряют стежок, поэтому проверка и сделана точной —
/// на Туле она не срабатывает ни разу ни на одном из девяти радиусов слайдера,
/// хотя непарных вершин там от нуля до девяти на карту.
#[allow(clippy::too_many_arguments)]
fn unstitchable(
    mesh: &polyanya::Mesh,
    chunk: usize,
    neighbour: usize,
    here: &[usize],
    there: &[usize],
    pairs: &[(usize, usize)],
    corners: &[Vec2; 2],
    lonely: &[(Vec2, usize)],
) -> Unstitched {
    let at_corner = |layer: usize, list: &[usize], corner: Vec2| -> Option<usize> {
        list.iter().copied().find(|&vertex| {
            from_poly(mesh.layers[layer].vertices[vertex].coords).distance_squared(corner)
                <= SEAM_EPSILON * SEAM_EPSILON
        })
    };
    let corner_pair = |corner: Vec2| -> Option<(usize, usize)> {
        Some((
            at_corner(chunk, here, corner)?,
            at_corner(neighbour, there, corner)?,
        ))
    };
    // цепочка вдоль шва: сшитые пары плюс его концы. Концы сшивает отдельный
    // проход по узлам сетки чанков, но отрезок от конца до первой пары — такое
    // же настоящее ребро, и лишняя вершина рвёт его так же
    let mut chain: Vec<((usize, usize), Option<usize>)> = Vec::with_capacity(pairs.len() + 2);
    chain.extend(corner_pair(corners[0]).map(|pair| (pair, None)));
    chain.extend(
        pairs
            .iter()
            .enumerate()
            .map(|(index, &pair)| (pair, Some(index))),
    );
    chain.extend(corner_pair(corners[1]).map(|pair| (pair, None)));

    let mut dropped = Vec::new();
    let mut dangerous = 0;
    #[cfg(debug_assertions)]
    let mut first = None;
    for &(point, blind) in lonely {
        // вершины идут вдоль шва по порядку, так что соседние в цепочке и есть
        // концы отрезка
        let inside = chain.windows(2).position(|window| {
            let [((first, _), _), ((second, _), _)] = window else {
                return false;
            };
            strictly_inside(
                from_poly(mesh.layers[chunk].vertices[*first].coords),
                from_poly(mesh.layers[chunk].vertices[*second].coords),
                point,
            )
        });
        let Some(at) = inside else {
            continue;
        };
        let rich = if blind == chunk { neighbour } else { chunk };
        let end = |layer: usize, index: usize| {
            let ((mine, other), _) = chain[index];
            if layer == chunk { mine } else { other }
        };
        // Опасность собирается из двух половин, и порознь каждая безобидна.
        // У слепой стороны отрезок должен быть **цельным ребром** — иначе там
        // стена, и переносить поиск некуда. У богатой концы отрезка должны
        // лежать в одном полигоне — тогда сшивка отдаст его обоим концам
        // чужого ребра, и оно найдёт переход, которого с нашей стороны нет:
        // лишняя вершина разбила дорогу к нему надвое. Обычно лишняя вершина
        // заодно разрезает полигон, общего у концов не остаётся, переход не
        // собирается — и шов честно молчит (на Туле так во всех замеренных
        // случаях, потому проверка и стоит из двух половин, а не из одной).
        if !shared_edge(&mesh.layers[blind], end(blind, at), end(blind, at + 1))
            || shared_polygon(&mesh.layers[rich], end(rich, at), end(rich, at + 1)).is_none()
        {
            continue;
        }
        dangerous += 1;
        #[cfg(debug_assertions)]
        if first.is_none() {
            first = Some((point, blind));
        }
        dropped.extend(chain[at].1);
        dropped.extend(chain[at + 1].1);
    }
    dropped.sort_unstable();
    dropped.dedup();
    Unstitched {
        dropped,
        dangerous,
        #[cfg(debug_assertions)]
        first,
    }
}

/// Лежит ли точка на отрезке, не совпадая с его концами.
fn strictly_inside(from: Vec2, to: Vec2, point: Vec2) -> bool {
    let along = to - from;
    let length = along.length();
    if length <= SEAM_EPSILON {
        return false;
    }
    let ratio = (point - from).dot(along) / (length * length);
    let margin = SEAM_EPSILON / length;
    ratio > margin && ratio < 1.0 - margin
}

/// Сшивка соседних чанков и граф уровня 1 одним проходом.
///
/// Пары вершин ищутся **по совпадению мировых координат**, а не `zip`'ом
/// отсортированных списков, как в примере из тестов polyanya: zip парует по
/// порядку вдоль кромки и молча спарит не то, если с одной стороны вершин
/// окажется больше.
///
/// `stitch_at_vertices` зовётся ровно один раз на весь меш: он метит индексы
/// номером слоя через `+=`, и второй вызов пометил бы их повторно.
pub(super) fn stitch_chunks(
    mesh: &mut polyanya::Mesh,
    grid: UVec2,
    chunk_size: Vec2,
    components: &[ChunkComponents],
) -> ChunkGraph {
    let mut node_of: Vec<Vec<u32>> = Vec::with_capacity(components.len());
    let mut nodes: Vec<GraphNode> = Vec::new();
    for (chunk, chunk_components) in components.iter().enumerate() {
        let offset = from_poly(mesh.layers[chunk].offset);
        let ids = chunk_components
            .centers
            .iter()
            .map(|&center| {
                nodes.push(GraphNode {
                    chunk: chunk as u8,
                    center: center + offset,
                });
                nodes.len() as u32 - 1
            })
            .collect();
        node_of.push(ids);
    }

    let mut edges: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nodes.len()];
    // сколько вершин шва связывает пару компонент. Одной мало: polyanya
    // переносит поиск через общее РЕБРО, а соприкосновение в точке она не
    // пройдёт — а граф на таком ребре пообещает проход, и запрос выжжет весь
    // бюджет итераций, разворачивая фронт по всему коридору
    let mut touching: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut stitches: Vec<((u8, u8), Vec<(usize, usize)>)> = Vec::new();
    let mut seam_vertices = 0;
    let mut weak_seams = 0;
    let mut blind_vertices = 0;
    let mut split_edges = 0;
    // первое опасное место целиком — только для сообщения ассерта: по нему
    // сразу видно, какой шов и какую точку смотреть. В релизе не собирается
    #[cfg(debug_assertions)]
    let mut first_split: Option<(Vec2, usize, usize)> = None;

    for y in 0..grid.y {
        for x in 0..grid.x {
            let chunk = (y * grid.x + x) as usize;
            // сосед справа и сосед сверху — каждый шов обходится один раз
            for (neighbour, along_y) in [
                (x + 1 < grid.x).then(|| (chunk + 1, true)),
                (y + 1 < grid.y).then(|| (chunk + grid.x as usize, false)),
            ]
            .into_iter()
            .flatten()
            {
                // общая кромка одна и та же для обоих соседей — слои живут в
                // мировых координатах, локальных больше нет
                let node = |node_x: u32, node_y: u32| {
                    Vec2::new(
                        quantized(node_x as f32 * chunk_size.x),
                        quantized(node_y as f32 * chunk_size.y),
                    )
                };
                let (start, end) = if along_y {
                    (node(x + 1, y), node(x + 1, y + 1))
                } else {
                    (node(x, y + 1), node(x + 1, y + 1))
                };

                let here = mesh.layers[chunk].get_vertices_on_segment(to_poly(start), to_poly(end));
                let there =
                    mesh.layers[neighbour].get_vertices_on_segment(to_poly(start), to_poly(end));
                if here.is_empty() || there.is_empty() {
                    continue;
                }

                // концы шва — узлы сетки чанков, общие сразу для двух или
                // четырёх слоёв. Попарный обход «правый сосед и верхний» их
                // связывает несимметрично, поэтому здесь они пропускаются, а
                // сшиваются отдельным проходом всеми парами сразу (см. ниже).
                let corners = [start, end];
                let on_corner = |world: Vec2| {
                    corners
                        .iter()
                        .any(|corner| corner.distance_squared(world) <= SEAM_EPSILON * SEAM_EPSILON)
                };
                let mut pairs = Vec::new();
                // вершины, оставшиеся без пары, — вместе со слоем, у которого
                // их нет. Считаются с обеих сторон: чей слой оказался богаче на
                // вершину, шву безразлично
                let mut lonely: Vec<(Vec2, usize)> = Vec::new();
                for &vertex in &here {
                    let world = from_poly(mesh.layers[chunk].vertices[vertex].coords);
                    if on_corner(world) {
                        continue;
                    }
                    let matched = there.iter().find(|&&other| {
                        from_poly(mesh.layers[neighbour].vertices[other].coords)
                            .distance_squared(world)
                            <= SEAM_EPSILON * SEAM_EPSILON
                    });
                    let Some(&other) = matched else {
                        lonely.push((world, neighbour));
                        continue;
                    };
                    // добить до побитового совпадения: поиск сверяет концы
                    // интервала воронки с координатами вершин точным
                    // равенством, и остаточное расхождение в младшем разряде
                    // мешает ей сходиться
                    let snapped = mesh.layers[chunk].vertices[vertex].coords;
                    mesh.layers[neighbour].vertices[other].coords = snapped;
                    pairs.push((vertex, other));
                }
                // спаренные вершины соседа теперь побитово равны нашим, так что
                // непарные у него ищутся простым «нет в парах»
                for &vertex in &there {
                    let world = from_poly(mesh.layers[neighbour].vertices[vertex].coords);
                    if !on_corner(world) && !pairs.iter().any(|&(_, other)| other == vertex) {
                        lonely.push((world, chunk));
                    }
                }
                let unstitched = unstitchable(
                    mesh, chunk, neighbour, &here, &there, &pairs, &corners, &lonely,
                );
                split_edges += unstitched.dangerous;
                blind_vertices += lonely.len() - unstitched.dangerous;
                #[cfg(debug_assertions)]
                if first_split.is_none()
                    && let Some((point, blind)) = unstitched.first
                {
                    let rich = if blind == chunk { neighbour } else { chunk };
                    first_split = Some((point, blind, rich));
                }
                if !unstitched.dropped.is_empty() {
                    pairs = pairs
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !unstitched.dropped.contains(index))
                        .map(|(_, &pair)| pair)
                        .collect();
                }
                // ребро графа — не «вершины совпали», а **общий отрезок шва**:
                // polyanya переносит поиск через ребро, и пара компонент,
                // соприкоснувшихся в точке (или в двух точках по разные
                // стороны препятствия), проход не даёт. Граф, пообещавший
                // такой проход, стоит очень дорого: коридор построен, а цели
                // в нём не достичь, и запрос выжигает весь бюджет итераций,
                // разворачивая фронт на гигабайты.
                //
                // Вершины идут вдоль шва по порядку (`get_vertices_on_segment`
                // сортирует), так что соседние в списке и есть концы отрезка;
                // отрезок свободен ровно тогда, когда обе его вершины лежат в
                // одном полигоне — то есть у него есть общее ребро.
                for window in pairs.windows(2) {
                    let [(here_from, there_from), (here_to, there_to)] = window else {
                        continue;
                    };
                    let (Some(polygon_here), Some(polygon_there)) = (
                        shared_polygon(&mesh.layers[chunk], *here_from, *here_to),
                        shared_polygon(&mesh.layers[neighbour], *there_from, *there_to),
                    ) else {
                        continue;
                    };
                    let (Some(&from), Some(&to)) = (
                        components[chunk].of_polygon.get(polygon_here),
                        components[neighbour].of_polygon.get(polygon_there),
                    ) else {
                        continue;
                    };
                    *touching
                        .entry((
                            node_of[chunk][from as usize],
                            node_of[neighbour][to as usize],
                        ))
                        .or_insert(0) += 1;
                }
                if pairs.len() < 2 {
                    weak_seams += 1;
                }
                if pairs.is_empty() {
                    continue;
                }
                seam_vertices += pairs.len();
                stitches.push(((chunk as u8, neighbour as u8), pairs));
            }
        }
    }

    // Узлы сетки чанков — точки, принадлежащие сразу двум или четырём слоям.
    // Попарный обход выше их пропускает: «правый сосед и верхний» такую точку
    // связывает несимметрично, и кольцо соседей выходит несогласованным.
    // Оставить её несшитой, однако, нельзя: примыкающий к ней отрезок шва
    // становится тупиковым — общего полигона у его концов в соседнем слое нет,
    // `successors` отбраковывает ребро как cul-de-sac, и поиск видит на месте
    // отрезка стену длиной в шаг подразбиения. Таких стен по четыре на каждый
    // узел сетки, и на них разваливались обходы длиной больше пары чанков.
    // Лечится не пропуском, а полнотой: сшиваем все пары сразу, и кольцо
    // вокруг точки снова замкнуто.
    for node_y in 0..=grid.y {
        for node_x in 0..=grid.x {
            let world = Vec2::new(
                quantized(node_x as f32 * chunk_size.x),
                quantized(node_y as f32 * chunk_size.y),
            );
            let mut sharing: Vec<(u8, usize)> = Vec::new();
            for (dx, dy) in [(-1i32, -1i32), (0, -1), (-1, 0), (0, 0)] {
                let (cell_x, cell_y) = (node_x as i32 + dx, node_y as i32 + dy);
                if cell_x < 0 || cell_y < 0 || cell_x >= grid.x as i32 || cell_y >= grid.y as i32 {
                    continue;
                }
                let chunk = (cell_y as u32 * grid.x + cell_x as u32) as usize;
                let found = mesh.layers[chunk].vertices.iter().position(|vertex| {
                    from_poly(vertex.coords).distance_squared(world) <= SEAM_EPSILON * SEAM_EPSILON
                });
                if let Some(vertex) = found {
                    // та же побитовая привязка, что и на самом шве
                    mesh.layers[chunk].vertices[vertex].coords = to_poly(world);
                    sharing.push((chunk as u8, vertex));
                }
            }
            for first in 0..sharing.len() {
                for second in (first + 1)..sharing.len() {
                    stitches.push((
                        (sharing[first].0, sharing[second].0),
                        vec![(sharing[first].1, sharing[second].1)],
                    ));
                }
            }
        }
    }

    // порядок обхода `touching` — это порядок рёбер в списках смежности, а он
    // разрешает ничьи в поиске: два равных по стоимости коридора выбираются
    // тем, чьё ребро легло раньше. `std::HashMap` перемешивает обход своим
    // случайным `RandomState`, то есть от запуска к запуску, — сортировка по
    // ключу делает граф (а с ним и найденные пути) воспроизводимым
    let mut touching: Vec<((u32, u32), u32)> = touching.into_iter().collect();
    touching.sort_unstable_by_key(|&(pair, _)| pair);

    for ((from, to), segments) in touching {
        debug_assert!(segments > 0);
        let weight = nodes[from as usize]
            .center
            .distance(nodes[to as usize].center);
        edges[from as usize].push((to, weight));
        edges[to as usize].push((from, weight));
    }

    if weak_seams > 0 {
        warn!("polymesh: {weak_seams} seams stitched by fewer than two vertices");
    }
    // Вершина шва без пары — не ошибка постройки, а её обычный остаток:
    // глобальным (`seam_points`) задан только контур чанка, а сами препятствия
    // режет и упрощает каждый чанк у себя, и щель шириной в полметра между
    // раздутыми контурами может остаться открытой у одного соседа и закрыться
    // у другого. Пары такой вершине нет, потому что у соседа в этом месте
    // стена, — сшивать нечего, и падать не на чем.
    if blind_vertices > 0 {
        debug!(
            "polymesh: {blind_vertices} seam vertices face a wall on the other side \
             (normal, not an error: the neighbour has no free space there to pair with)"
        );
    }
    // А вот отрезок, оставшийся у соседа цельным ребром, — уже не остаток, а
    // сорванный инвариант: сшитый, он даёт переход, который находится только с
    // одной стороны (см. `verify_seams`), и на этом воронка перестаёт сходиться.
    // Постройка себя спасает — такие отрезки не сшиваются вовсе, шов на них
    // становится стеной, — но в отладочной сборке это повод остановиться и
    // чинить геометрию, а не жить со стеной посреди шва.
    if split_edges > 0 {
        warn!("polymesh: {split_edges} seam segments left unstitched — see stitch_chunks");
    }
    // Сообщение длинное сознательно: его читает не только человек, но и агент,
    // которому скормили лог падения, — и по нему должно быть видно, что именно
    // сломано, чем это грозит, что чинить и чем воспроизвести без приложения.
    #[cfg(debug_assertions)]
    if let Some((point, blind, rich)) = first_split {
        let cell = |chunk: usize| UVec2::new(chunk as u32 % grid.x, chunk as u32 / grid.x);
        let (blind, rich) = (cell(blind), cell(rich));
        panic!(
            "polymesh: BROKEN CHUNK SEAM GEOMETRY — {split_edges} seam segment(s) had to be \
             left unstitched, first at {point} between chunks {rich} and {blind}.\n\
             WHAT HAPPENED: chunk {rich} has a vertex strictly inside a seam segment that chunk \
             {blind} keeps as one whole edge, and {rich} holds both ends of that segment in a \
             single polygon. Stitching those ends would give {blind}'s edge a crossing into \
             {rich} that {rich}'s own two halves cannot answer — a ONE-WAY SEAM (see \
             `verify_seams`), on which polyanya's funnel stops converging and one query eats \
             memory until the process is killed. `stitch_chunks` defended itself by not \
             stitching that segment: the seam is a wall there, the mesh is safe but poorer.\n\
             WHAT TO FIX: the per-chunk geometry, so both neighbours cut their shared border at \
             the same points — `chunk_layer` (i_overlay clip plus `Triangulation::simplify`, \
             both computed per chunk) and `seam_points` (the global crossings, the only part \
             that is shared today). Do NOT 'fix' this by deleting this assert, by widening \
             `SEAM_EPSILON`, or by stitching the segment anyway.\n\
             NOT THIS FAILURE: an unpaired seam vertex on its own is normal — it means the \
             neighbour has a wall there and nothing to pair with; those are counted separately \
             in the `seam vertices face a wall on the other side` debug line.\n\
             REPRODUCE OFFLINE, no app needed:\n\
             cargo run --release --example polymesh_seam_audit -- <agent radius>\n\
             It prints, per radius, every unpaired seam vertex with its coordinates and both \
             chunks, what each side has within 2 m of it, and where obstacle contours cross \
             that seam line."
        );
    }
    if !stitches.is_empty() {
        // ровно один вызов на весь меш: он метит индексы номером слоя через
        // `+=`, и второй вызов пометил бы их повторно
        mesh.stitch_at_vertices(stitches, false);
    }
    #[cfg(debug_assertions)]
    verify_seams(mesh);

    ChunkGraph {
        node_of,
        nodes,
        edges,
        seam_vertices,
    }
}

/// Проверка сшивки, только в отладочной сборке: **каждый шов обязан быть
/// двусторонним**.
///
/// Именно этот инвариант и ломался, причём молча. `successors` переходит в
/// соседний слой по ребру, у которого обе вершины лежат в одном полигоне того
/// слоя. Если два чанка разошлись в наборе вершин на общей кромке — у соседа
/// ребро цельное, у нас разбитое лишней вершиной надвое, — то переход
/// находится только в одну сторону. Меш при этом выглядит здоровым: путь
/// строится, короткие маршруты проходят, а воронка на длинных перестаёт
/// сходиться, очередь растёт без предела и процесс умирает от OOM. Дешевле
/// уронить постройку здесь с внятным сообщением, чем ловить это потом.
#[cfg(debug_assertions)]
fn verify_seams(mesh: &polyanya::Mesh) {
    let layer_of = |packed: u32| (packed >> 24) as usize;
    let index_of = |packed: u32| (packed & 0x00FF_FFFF) as usize;

    for (index, layer) in mesh.layers.iter().enumerate() {
        for (number, polygon) in layer.polygons.iter().enumerate() {
            let ring = &polygon.vertices;
            for position in 0..ring.len() {
                let here = &layer.vertices[ring[position] as usize];
                let next = &layer.vertices[ring[(position + 1) % ring.len()] as usize];
                // ребро ведёт в другой слой ровно тогда, когда обе его вершины
                // держат один и тот же чужой полигон — так его находит и
                // `successors`
                let Some(&across) = here.polygons.iter().find(|packed| {
                    **packed != u32::MAX
                        && layer_of(**packed) != index
                        && next.polygons.contains(packed)
                }) else {
                    continue;
                };
                let other = &mesh.layers[layer_of(across)];
                let ring_there = &other.polygons[index_of(across)].vertices;
                let mirrored = (0..ring_there.len()).any(|at| {
                    let one = other.vertices[ring_there[at] as usize].coords;
                    let two =
                        other.vertices[ring_there[(at + 1) % ring_there.len()] as usize].coords;
                    (one == here.coords && two == next.coords)
                        || (one == next.coords && two == here.coords)
                });
                assert!(
                    mirrored,
                    "polymesh seam is one-way: edge {:?}-{:?} of polygon {number} in layer {index} \
                     leads into polygon {} of layer {}, which has no matching edge — the two \
                     chunks cut their shared border differently (see SEAM_QUANTUM, seam_points)",
                    from_poly(here.coords),
                    from_poly(next.coords),
                    index_of(across),
                    layer_of(across),
                );
            }
        }
    }
}
