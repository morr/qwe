//! Поиск по готовому мешу: A* по графу компонент чанков, затем один запрос
//! polyanya внутри коридора из этих чанков.

use bevy::prelude::*;

use super::{PolymeshBuild, from_poly, to_poly};

/// Путь по полигональному мешу от точки к точке, **включая стартовую** —
/// таков контракт `movement::listen_for_pathfinding_tasks`, унаследованный от
/// сеточного поиска (первый waypoint отбрасывается, единственный означает
/// «уже на месте»). У polyanya `Path::path` старта не содержит.
///
/// Двухуровневый, по образцу `NorthstarGrid`: сперва A* по графу компонент
/// чанков, затем один запрос polyanya, которому оставлены незаблокированными
/// только чанки коридора. Плоский поиск по всему мешу разворачивал фронт по
/// всему городу — 85 млн узлов и смерть процесса от OOM; коридор держит его в
/// пределах пары тысяч полигонов.
///
/// `None` — конец не сажается на меш либо цель в другой компоненте связности.
/// Достижимость отвечает именно граф: у polyanya проверка островов отключена,
/// как только слоёв больше одного.
pub fn find_path_polymesh(build: &PolymeshBuild, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
    let (start, start_node) = build.locate(from, to)?;
    let (goal, goal_node) = build.locate(to, from)?;

    let blocked = if start_node == goal_node {
        // один и тот же кусок одного чанка — верхний уровень не нужен
        build.blocked_outside(std::iter::once(
            build.graph.nodes[start_node as usize].chunk,
        ))
    } else {
        let target = build.graph.nodes[goal_node as usize].center;
        let (route, _) = pathfinding::directed::astar::astar(
            &start_node,
            |&node| {
                build.graph.edges[node as usize]
                    .iter()
                    .map(|&(next, weight)| (next, (weight * COST_SCALE) as u32))
            },
            |&node| (build.graph.nodes[node as usize].center.distance(target) * COST_SCALE) as u32,
            |&node| node == goal_node,
        )?;
        let chunks: Vec<u8> = route
            .into_iter()
            .map(|node| build.graph.nodes[node as usize].chunk)
            .collect();
        build.blocked_outside(build.corridor(&chunks).into_iter())
    };

    let path = bounded_path(build, start, goal, blocked)?;
    let mut points = Vec::with_capacity(path.path.len() + 1);
    points.push(from);
    points.extend(path.path.into_iter().map(from_poly));
    Some(smoothed(&build.mesh, points))
}

/// Протяжка верёвки по готовой ломаной: точка выбрасывается, если прямая мимо
/// неё целиком лежит на меше.
///
/// Смысл в том, что коридор — ограничение поиска, а не геометрии: путь, обязанный
/// обогнуть угол закрытого чанка, обходит пустое место. Проверка идёт по **всему**
/// мешу, без закрытых слоёв, поэтому срез разрешается ровно там, где он и правда
/// свободен.
///
/// Работает вместе с добавкой четвёртого чанка (`corridor`), а не вместо неё, и
/// это замер, а не вкусовщина (Тула, `examples/polymesh_corner_audit`, 400
/// запросов, радиус 0.4; цена — `polymesh_bench`, 500 запросов):
///
/// | | путей с изломом в узле | длина/прямая | среднее | худший |
/// |---|---|---|---|---|
/// | ни того ни другого | 40.9% | 1.090 | 5.61 мс | 44.6 мс |
/// | только коридор | 16.2% | 1.061 | 6.23 мс | 74.5 мс |
/// | только сглаживание | 22.0% | 1.089 | 5.85 мс | 45.2 мс |
/// | оба | 5.1% | 1.061 | 6.50 мс | 79.5 мс |
///
/// Видно, что они лечат разное. Коридор укорачивает **маршрут** (1.090 → 1.061:
/// путь получает право сойти с лесенки), сглаживание убирает **излом** там, где
/// маршрут уже выбран, и почти ничего не даёт длине — изломы стоят 1600 м на 436
/// км. Вместе они снимают звезду до 20 изломов на 396 путей.
fn smoothed(mesh: &polyanya::Mesh, points: Vec<Vec2>) -> Vec<Vec2> {
    if points.len() < 3 {
        return points;
    }
    let mut result = Vec::with_capacity(points.len());
    result.push(points[0]);
    let mut anchor = 0;
    while anchor + 1 < points.len() {
        let mut next = anchor + 1;
        // до первого несвободного: дальше по ломаной проверять смысла нет,
        // за препятствием видимость не возвращается
        for candidate in anchor + 2..points.len() {
            if !segment_clear(mesh, points[anchor], points[candidate]) {
                break;
            }
            next = candidate;
        }
        result.push(points[next]);
        anchor = next;
    }
    result
}

/// Совпадение точки с вершиной и параметра — с концом отрезка.
const WALK_EPSILON: f32 = 1.0e-4;

/// Насколько отступить от начала отрезка, чтобы локализация дала полигон
/// **впереди**, а не любой из смежных вершине. Сантиметр — меньше любого
/// полигона меша и на порядок больше кванта шва.
const WALK_PROBE: f32 = 0.01;

/// Лежит ли точка внутри полигона (обход против часовой, полигоны выпуклые
/// после `merge_polygons`). Невыпуклый даст консервативное «нет».
fn inside(layer: &polyanya::Layer, polygon: &polyanya::Polygon, point: Vec2) -> bool {
    (0..polygon.vertices.len()).all(|index| {
        let (first, second) = (
            polygon.vertices[index],
            polygon.vertices[(index + 1) % polygon.vertices.len()],
        );
        match (
            layer.vertices.get(first as usize),
            layer.vertices.get(second as usize),
        ) {
            (Some(start), Some(end)) => {
                let (start, end) = (from_poly(start.coords), from_poly(end.coords));
                (end - start).perp_dot(point - start) >= -WALK_EPSILON
            }
            _ => false,
        }
    })
}

/// Лежит ли отрезок целиком на меше — проход по полигонам от начала к концу.
///
/// Не сэмплирование: точки через полметра пропустили бы щель между зданиями, а
/// щель — это ровно то, ради чего радиус агента вшит в контуры. Соседний полигон
/// ищется тем же приёмом, что и в `successors` самой polyanya — полигон, общий
/// обеим вершинам ребра, — поэтому шов проходится наравне с внутренним ребром:
/// сшивка дописала вершинам шва полигоны соседнего слоя.
///
/// Отказ консервативен: неоднозначность (отрезок уходит ровно через вершину,
/// начало не село на меш) считается «не свободно», и точка просто остаётся на
/// месте.
fn segment_clear(mesh: &polyanya::Mesh, from: Vec2, to: Vec2) -> bool {
    let direction = to - from;
    let length = direction.length();
    if length < WALK_EPSILON {
        return true;
    }
    // начальный полигон ищется не по самой точке, а по шагу вперёд: концы
    // среза — это **вершины меша** (иначе они не попали бы в путь), а
    // локализация по вершине честно возвращает любой из полигонов, ей
    // принадлежащих, в том числе лежащий позади. Проход в таком полигоне не
    // находит выхода — все рёбра сзади — и рапортует «свободно», не сделав ни
    // шага. Замерено: с этой ошибкой 10% сглаженных отрезков уходили сквозь
    // кварталы, один — на 3.2 км через полгорода.
    let step = (WALK_PROBE / length).min(0.5);
    let Some(mut current) = mesh
        .get_point_layer(to_poly(from.lerp(to, step)))
        .first()
        .map(polyanya::Coords::polygon)
    else {
        return false;
    };
    let mut travelled = 0.0f32;
    // потолок на случай зацикливания на вырожденной геометрии: полигонов на
    // пути не больше, чем их всего в меше
    let ceiling: usize = mesh.layers.iter().map(|layer| layer.polygons.len()).sum();
    for _ in 0..ceiling {
        let layer = &mesh.layers[(current >> 24) as usize];
        let Some(polygon) = layer.polygons.get((current & 0x00FF_FFFF) as usize) else {
            return false;
        };
        // ребро, через которое отрезок выходит: ближайшее пересечение дальше
        // текущего положения
        let mut exit: Option<(f32, u32, u32)> = None;
        for index in 0..polygon.vertices.len() {
            let (first, second) = (
                polygon.vertices[index],
                polygon.vertices[(index + 1) % polygon.vertices.len()],
            );
            let (Some(start), Some(end)) = (
                layer.vertices.get(first as usize),
                layer.vertices.get(second as usize),
            ) else {
                return false;
            };
            let (start, end) = (from_poly(start.coords), from_poly(end.coords));
            let edge = end - start;
            let denominator = direction.perp_dot(edge);
            if denominator.abs() < WALK_EPSILON {
                continue;
            }
            let offset = start - from;
            let along = offset.perp_dot(edge) / denominator;
            let across = offset.perp_dot(direction) / denominator;
            // конец отрезка — тоже вершина меша (иначе он не был бы точкой
            // пути), и рёбра, сходящиеся в ней, пересекаются ровно на `along`
            // = 1. Это не выход, а приезд
            if along <= travelled + WALK_EPSILON || along >= 1.0 - WALK_EPSILON {
                continue;
            }
            // пересечение лежит за концами ребра — это ребро отрезок не
            // пересекает вовсе (линии-то пересекаются всегда)
            if !(0.0..=1.0).contains(&across) {
                continue;
            }
            // ровно через вершину — неоднозначно, кольцо полигонов вокруг неё
            // здесь не разбирается
            if !(WALK_EPSILON..=1.0 - WALK_EPSILON).contains(&across) {
                return false;
            }
            if exit.is_none_or(|(best, _, _)| along < best) {
                exit = Some((along, first, second));
            }
        }
        // выхода нет — конец отрезка обязан лежать внутри этого полигона, и
        // это проверяется, а не предполагается: «выхода не нашлось» бывает и
        // когда проход стоит не в том полигоне
        let Some((along, first, second)) = exit else {
            return inside(layer, polygon, to);
        };
        let (Some(start), Some(end)) = (
            layer.vertices.get(first as usize),
            layer.vertices.get(second as usize),
        ) else {
            return false;
        };
        let Some(&other) = start
            .polygons
            .iter()
            .filter(|polygon| **polygon != u32::MAX && end.polygons.contains(polygon))
            .find(|polygon| **polygon != current)
        else {
            // по ту сторону ребра стены нет полигона — препятствие или край
            return false;
        };
        current = other;
        travelled = along;
    }
    false
}

/// Сколько извлечений из очереди на полигон открытого пространства поиск может
/// себе позволить, прежде чем считаться расходящимся.
///
/// Тот же порядок, что у собственного предела polyanya (`polygons * 10`), — но
/// с двумя отличиями, и оба существенны. Во-первых, предел polyanya считается
/// по всему мешу, а этот по **открытым** слоям: с коридором пространство
/// меньше на порядок, и потолок обязан ужиматься вместе с ним, иначе он ничего
/// не ограничивает. Во-вторых, предел polyanya стоит внутри блокирующего
/// `Mesh::path` и ограничивает только извлечения; вставки не ограничены ничем,
/// а прервать вызов снаружи нельзя. `Mesh::get_path` отдаёт будущее, которое
/// двигает поиск по три шага за опрос, — потолок ставится снаружи и режет
/// работу целиком.
///
/// Замер на Туле (плоский меш, 22 297 полигонов, 400 запросов): худший
/// успешный запрос — 20 370 опросов при бюджете 74 323. Медиана 423,
/// p99 — 12 673.
///
/// **Почему 40, а не 10.** «10 на полигон» было отмерено по плоскому мешу, где
/// открыт весь меш; у коридорного запроса открытых полигонов на порядок
/// меньше, а длинный маршрут тратит на полигон коридора больше десяти
/// извлечений — воронка на швах плодит узлы с равной стоимостью. Два живых
/// запроса (2.7 и 4.4 км через Тулу, радиус 0.4) исчерпывали бюджет ×1 и
/// **сходились на ×2** (`examples/audit/polymesh_budget_repro`), то есть паника
/// убивала здоровые поиски. 40 — измеренная потребность ×2 с двукратным
/// запасом; настоящую дивергенцию (цикл узлов) дедупликация в вендоренном
/// polyanya ловит раньше любого бюджета.
const SEARCH_POPS_PER_POLYGON: usize = 40;

/// `FuturePath::poll` продвигает поиск на три шага.
const SEARCH_STEPS_PER_POLL: usize = 3;

/// Нижняя граница бюджета: на крошечном меше (тесты, синтетика) доля от числа
/// полигонов вырождается в единицы опросов.
const MIN_SEARCH_POLLS: usize = 4096;

impl PolymeshBuild {
    /// Полигоны в слоях, открытых поиску.
    fn open_polygons(&self, blocked: &std::collections::HashSet<u8>) -> usize {
        self.mesh
            .layers
            .iter()
            .enumerate()
            .filter(|(index, _)| !blocked.contains(&(*index as u8)))
            .map(|(_, layer)| layer.polygons.len())
            .sum()
    }
}

/// Поиск под внешним потолком работы — единственная дверь к polyanya: и
/// коридорный запрос, и запрос без иерархии идут здесь. Блокирующий
/// `Mesh::path_on_layers` не годится ровно тем, что его не прервать: его
/// внутренний предел считается по **всему** мешу (сотни тысяч извлечений), а
/// вставки не ограничены вовсе — расходящийся поиск ел секунды процессора на
/// поток и 6.65 ГБ памяти за 64 000 опросов на одном битом шве.
///
/// `None` — пути нет. Исчерпанный бюджет — **паника всегда**, не только в
/// debug: расходящаяся воронка означает дефект геометрии меша либо вырожденную
/// точку старта/цели, и решение принято жёсткое — игра падает с координатами
/// обоих концов, чтобы дефект чинился сразу, а не жил месяцами в виде тихо
/// сжирающих пул «вечных» поисков (симптом: демоны замирают у портала,
/// конвейер пуст, CPU 400%+). Чинить по панике надо геометрию или источник
/// вырожденной точки — не поднимать потолок: запас и так 3.6× над худшим
/// замеренным здоровым запросом.
fn bounded_path(
    build: &PolymeshBuild,
    from: polyanya::Coords,
    to: polyanya::Coords,
    blocked: std::collections::HashSet<u8>,
) -> Option<polyanya::Path> {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    let open_polygons = build.open_polygons(&blocked);
    let budget = (open_polygons * SEARCH_POPS_PER_POLYGON)
        .div_ceil(SEARCH_STEPS_PER_POLL)
        .max(MIN_SEARCH_POLLS);

    // концы уже посажены `locate` и несут найденный полигон — `FuturePath`
    // использует его и повторную посадку не делает
    let blocked_chunks = blocked.len();
    let mut future = std::pin::pin!(build.mesh.get_path_on_layers(from, to, blocked));
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..budget {
        if let Poll::Ready(path) = future.as_mut().poll(&mut context) {
            return path;
        }
    }
    panic!(
        "polymesh search diverged: {budget} polls ({} steps) spent on {open_polygons} open \
         polygons ({blocked_chunks} chunks blocked) without an answer, {:?} -> {:?} \
         (tiles {:?} -> {:?}). The funnel is not converging: broken mesh geometry or a \
         degenerate start/goal point — fix that, do not raise the budget",
        budget * SEARCH_STEPS_PER_POLL,
        from_poly(from.position()),
        from_poly(to.position()),
        crate::grid::world_to_tile(from_poly(from.position())),
        crate::grid::world_to_tile(from_poly(to.position())),
    );
}

/// Веса графа целочисленные (у `astar` из `pathfinding` порядок на стоимости);
/// метры переводятся в сантиметры.
const COST_SCALE: f32 = 100.0;

impl PolymeshBuild {
    /// Коридор для нижнего поиска: чанки маршрута верхнего уровня плюс
    /// **четвёртый чанк на каждом его повороте**.
    ///
    /// Без добавки коридор ровно в один чанк шириной, а граф уровня 1 знает
    /// только четырёхсвязность (ребро — общий отрезок шва, см. `stitch_chunks`),
    /// поэтому диагональная поездка всегда идёт лесенкой A→B→C. Свободная
    /// область такой лесенки — невыпуклый угол, и кратчайший путь в ней обязан
    /// обогнуть его вершину. Вершина эта — узел сетки чанков, общая точка сразу
    /// четырёх слоёв, и воронка polyanya садится в неё **точно**: при непустом
    /// `blocked_layers` любая вершина, касающаяся закрытого слоя, считается
    /// углом (`vendor/polyanya/src/instance.rs`, ветки `*NonObservable`). На
    /// карте это видно как звезда — десятки путей сходятся в одну точку и
    /// расходятся из неё лучами.
    ///
    /// Замер до правки (Тула, `examples/polymesh_corner_audit`, 400 запросов,
    /// радиус 0.4): излом ровно в узле сетки был у 40.9% путей, 7.2 м на излом,
    /// и у 68% из них прямой срез мимо узла свободен на меше — то есть крюк не
    /// от препятствия, а от закрытого соседа. Общая длина пути против прямой —
    /// 1.090 при 1.037 у плоского меша.
    ///
    /// После правки: 16.2% путей с изломом, 3.15 м на излом, 218 м суммарно,
    /// длина против прямой 1.061. Стоит это открытой площади: коридор растёт с
    /// 9.6 до 12.9 чанков (+35%, максимум +78%), и на том же наборе
    /// (`polymesh_bench`, 500 запросов, радиус 0.4) среднее идёт с 5.61 до
    /// 6.23 мс, худший с 45 до 75 мс, промахи те же 5. Платят только маршруты с
    /// поворотом, то есть длинные: короткая прогулка внутри чанка или через один
    /// шов тройки не образует и коридор не расширяет.
    ///
    /// Дороже, чем прибавка площади, потому что добавленный чанк — не «где-то
    /// сбоку», а ровно на прямой к цели: у его полигонов эвристика A* самая
    /// маленькая, и разбираются они первыми, все.
    ///
    /// Добавляется ровно один чанк на поворот, а не всё кольцо соседей: кольцо
    /// раздуло бы открытую площадь (а с ней и фронт поиска, и потолок
    /// `SEARCH_POPS_PER_POLYGON`) в разы ради тех же срезанных углов.
    ///
    /// Отбирать повороты ещё жёстче — «открывать четвёртый чанк, только если
    /// прямая старт → цель его задевает» — пробовали и отвергли по замеру:
    /// поиск дешевеет на считанные проценты, а изломы возвращаются с 16.2% до
    /// 26.3%. Прямая старт → цель — плохой предиктор того, где путь захочет
    /// срезать: он огибает кварталы и отходит от неё на сотни метров.
    fn corridor(&self, route: &[u8]) -> Vec<u8> {
        let mut corridor = route.to_vec();
        let coords = |chunk: u8| {
            let chunk = chunk as u32;
            IVec2::new((chunk % self.grid.x) as i32, (chunk / self.grid.x) as i32)
        };
        for window in route.windows(3) {
            let (before, turn, after) = (coords(window[0]), coords(window[1]), coords(window[2]));
            // поворот — это когда концы тройки стоят по диагонали; на прямом
            // участке лесенки нет и срезать нечего
            if before.x == after.x || before.y == after.y {
                continue;
            }
            // четвёртый угол квадрата 2×2, в котором лежит тройка
            let fourth = before + after - turn;
            corridor.push((fourth.y as u32 * self.grid.x + fourth.x as u32) as u8);
        }
        corridor
    }

    /// Набор слоёв, которые поиску закрыты: всё, кроме коридора.
    fn blocked_outside(&self, corridor: impl Iterator<Item = u8>) -> std::collections::HashSet<u8> {
        let corridor: std::collections::HashSet<u8> = corridor.collect();
        (0..self.mesh.layers.len() as u8)
            .filter(|layer| !corridor.contains(layer))
            .collect()
    }
}
