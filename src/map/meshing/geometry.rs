//! Свободная геометрия без состояния: описанный прямоугольник кольца,
//! площадь, периметр и короткая сторона эквивалентного прямоугольника,
//! длины дуги ломаной, проекция точки на путь, свип выпуклого контура и шаг
//! тесселяции дуги. Чистые функции над точками, общие для сборщика и
//! остальных модулей карты.

use std::f32::consts::PI;

use bevy::prelude::*;

/// Допуск на стрелку хорды дуги, м. Шаг тесселяции считается от радиуса, а не
/// берётся константой: у аллеи (полуширина 1.75 м) выходит ~28° на хорду, у
/// магистрали (8 м) ~13°, и обе дуги одинаково гладкие на глаз.
///
/// Тем же допуском отсекаются веера на почти прямых изломах — см.
/// [`MeshBuilder::push_join_fan`](super::MeshBuilder::push_join_fan).
pub(super) const ARC_TOLERANCE: f32 = 0.05;

/// Потолок числа хорд в дуге — страховка от вырожденного радиуса.
pub(super) const MAX_ARC_STEPS: usize = 12;

/// Минимальный по площади описанный прямоугольник кольца, CCW, первое ребро
/// вдоль длинной оси. У такого прямоугольника одна сторона лежит на ребре
/// выпуклой оболочки, а рёбра оболочки — подмножество рёбер контура, так что
/// перебор направлений всех рёбер находит оптимум без построения оболочки:
/// контуров тысячи, вершин в каждом — единицы.
pub fn min_area_rect(ring: &[Vec2]) -> Option<[Vec2; 4]> {
    if ring.len() < 3 {
        return None;
    }
    let mut best: Option<(f32, Vec2, Vec2, Vec2)> = None;
    for i in 0..ring.len() {
        let Some(u) = (ring[(i + 1) % ring.len()] - ring[i]).try_normalize() else {
            continue;
        };
        let v = Vec2::new(-u.y, u.x);
        let (mut u_min, mut u_max, mut v_min, mut v_max) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for point in ring {
            let (pu, pv) = (point.dot(u), point.dot(v));
            u_min = u_min.min(pu);
            u_max = u_max.max(pu);
            v_min = v_min.min(pv);
            v_max = v_max.max(pv);
        }
        let area = (u_max - u_min) * (v_max - v_min);
        if best.is_none_or(|(best_area, ..)| area < best_area) {
            best = Some((area, u, Vec2::new(u_min, v_min), Vec2::new(u_max, v_max)));
        }
    }
    let (_, u, low, high) = best?;
    let v = Vec2::new(-u.y, u.x);
    let corner = |pu: f32, pv: f32| u * pu + v * pv;
    let mut rect = [
        corner(low.x, low.y),
        corner(high.x, low.y),
        corner(high.x, high.y),
        corner(low.x, high.y),
    ];
    // длинная ось первой: у CCW-квадрата сдвиг на одну вершину сохраняет обход
    if high.x - low.x < high.y - low.y {
        rect.rotate_left(1);
    }
    Some(rect)
}

/// Площадь замкнутого контура по формуле шнурков, со знаком: положительна при
/// обходе против часовой стрелки. Делить на два обязательно — на «площадь /
/// периметр» как на толщину контура опирается зажим каймы
/// ([`MeshBuilder::push_inset_band`](super::MeshBuilder::push_inset_band)).
///
/// Та же величина, что даёт `osm::model::signed_ring_area`; копия здесь ровно
/// потому, что `meshing` не знает про OSM-модель и не должен узнать — это
/// чистая геометрия.
pub(super) fn signed_area(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].perp_dot(ring[(index + 1) % count]))
        .sum::<f32>()
        / 2.0
}

/// Периметр замкнутого кольца (последнее ребро — от конца к началу). Одна
/// копия на всю карту: здесь, в чистой геометрии, потому что `meshing` не
/// знает про OSM-модель, а модель, разбор и слои знают про `meshing`.
pub(crate) fn ring_perimeter(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].distance(ring[(index + 1) % count]))
        .sum()
}

/// Короткая сторона прямоугольника с той же площадью `area` и тем же
/// периметром `perimeter`, что у фигуры, — меньший корень
/// `w² − (P/2)·w + A = 0`. Если такого прямоугольника нет (фигура компактнее
/// квадрата и дискриминант отрицателен), сторона квадрата с тем же периметром.
///
/// Одна копия на обоих читателей — ширину полосы парковочного кармана
/// (`parking::strip_width`, где `2·A/P` занижает ширину на торцах полосы) и
/// короткую сторону плана дома с дворами (`buildings::material::equivalent_width`,
/// у каре и буквы Г — ширина крыла), — по той же причине, что и
/// [`ring_perimeter`]: чистая геометрия, одна на всю карту.
pub(crate) fn equivalent_width(perimeter: f32, area: f32) -> f32 {
    let semi = perimeter / 2.0;
    (semi - (semi * semi - 4.0 * area).max(0.0).sqrt()) / 2.0
}

/// Длина дуги в каждой точке ломаной и полная длина; у замкнутой — с
/// замыкающим сегментом.
pub(super) fn arclengths(path: &[Vec2], closed: bool) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(path.len());
    let mut total = 0.0;
    for (index, point) in path.iter().enumerate() {
        if index > 0 {
            total += point.distance(path[index - 1]);
        }
        along.push(total);
    }
    if closed && path.len() > 1 {
        total += path[path.len() - 1].distance(path[0]);
    }
    (along, total)
}

/// Длина дуги в ближайшей к `point` точке ломаной.
pub(super) fn project_onto_path(path: &[Vec2], along: &[f32], point: Vec2, closed: bool) -> f32 {
    let mut best = (f32::INFINITY, 0.0);
    // у замкнутого пути сегментов столько же, сколько точек: последний идёт
    // от хвоста к голове
    let count = path.len();
    let segments = if closed && count > 1 {
        count
    } else {
        count.saturating_sub(1)
    };
    for index in 0..segments {
        let (from, to) = (path[index], path[(index + 1) % count]);
        let span = to - from;
        let length_sq = span.length_squared();
        let t = if length_sq > 0.0 {
            ((point - from).dot(span) / length_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = point.distance_squared(from + span * t);
        if distance < best.0 {
            best = (distance, along[index] + t * length_sq.sqrt());
        }
    }
    best.1
}

/// Свип выпуклого контура по свету: оболочка контура и его копии, сдвинутой
/// на `offset`, — ровно то, что накрывает тень. Общий для машин
/// (`cars/body.rs`) и оград (`fences.rs`): тень начинается **под** объектом и
/// вытекает из-под него, а не лежит сдвинутой копией в стороне.
///
/// Строится обходом, без сортировки: ребро, чья внешняя нормаль смотрит по
/// свету, уезжает на `offset`, остальные остаются на месте, а в двух
/// вершинах, где одно сменяется другим, оболочка переходит из одной копии в
/// другую. Это сумма Минковского контура с отрезком `[0, offset]`, поэтому
/// на выпуклом контуре результат выпуклый — на это опирается `push_convex`.
/// Контур — против часовой. Нулевой `offset` (солнце в зените) даёт обратно
/// сам контур.
pub fn sweep_convex(outline: &[Vec2], offset: Vec2) -> Vec<Vec2> {
    let count = outline.len();
    // контур обходится против часовой, значит внешняя нормаль ребра смотрит
    // вправо от него; ребро отбрасывает тень наружу, когда она смотрит по свету
    let casts = |at: usize| {
        let edge = outline[(at + 1) % count] - outline[at];
        Vec2::new(edge.y, -edge.x).dot(offset) > 0.0
    };
    let mut hull = Vec::with_capacity(count + 2);
    for (at, &point) in outline.iter().enumerate() {
        match (casts((at + count - 1) % count), casts(at)) {
            (false, false) => hull.push(point),
            (true, true) => hull.push(point + offset),
            (false, true) => hull.extend([point, point + offset]),
            (true, false) => hull.extend([point + offset, point]),
        }
    }
    hull
}

/// Сколько хорд нужно дуге радиуса `radius` на `sweep` радиан, чтобы стрелка
/// хорды осталась в пределах [`ARC_TOLERANCE`].
pub(crate) fn arc_steps(radius: f32, sweep: f32) -> usize {
    let max_step = if radius > ARC_TOLERANCE {
        2.0 * (1.0 - ARC_TOLERANCE / radius).acos()
    } else {
        PI
    };
    ((sweep / max_step).ceil() as usize).clamp(1, MAX_ARC_STEPS)
}

/// Расстояние от точки до ломаной — мерка тестов на геометрию лент: ни одна
/// вершина не имеет права уйти от осевой дальше, чем обещает примитив. Одна
/// на тесты мешинга, дорог и рельсов.
#[cfg(test)]
pub(crate) fn distance_to_path(point: Vec2, path: &[Vec2]) -> f32 {
    path.windows(2)
        .map(|segment| {
            let span = segment[1] - segment[0];
            let t = (point - segment[0]).dot(span) / span.length_squared();
            point.distance(segment[0] + span * t.clamp(0.0, 1.0))
        })
        .fold(f32::INFINITY, f32::min)
}
