//! Чистая геометрия пути под лентами: miter-офсеты ([`miter_offsets`]),
//! склейка близких точек ([`merge_ribbon_points`], [`merge_close_points`],
//! [`ribbon_vertices`]), вставка вершин по длине дуги и торцы
//! ([`butt_direction`], [`butt_normal`]).

use bevy::prelude::*;

use crate::map::meshing::geometry::ARC_TOLERANCE;

/// Максимальное удлинение стыка ленты относительно полуширины. Контур кроны
/// полон почти встречных рёбер (впадины между фестонами), и там miter уходит
/// в длинный шип — при 1.5 стык вырождается в срез, шипов не видно.
const MITER_LIMIT: f32 = 1.5;

/// Вершина на длине дуги `at`, если та внутри какого-то сегмента и не ближе
/// `merge_distance` к его концам. Отвечает, нашёлся ли такой сегмент, — чтобы
/// звонящий знал, что остаётся замыкающее звено кольца ([`append_vertex_at`]).
pub(super) fn insert_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
) -> bool {
    let Some(index) = along
        .windows(2)
        .position(|pair| pair[0] < at && at < pair[1])
    else {
        return false;
    };
    if at - along[index] <= merge_distance || along[index + 1] - at <= merge_distance {
        return true;
    }
    let t = (at - along[index]) / (along[index + 1] - along[index]);
    let point = path[index].lerp(path[index + 1], t);
    path.insert(index + 1, point);
    along.insert(index + 1, at);
    true
}

/// То же на **замыкающем** звене кольца — от последней точки к первой: вершина
/// там не вставляется между соседями, а дописывается в хвост.
pub(super) fn append_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
    period: f32,
) {
    let (Some(&last), Some(&first)) = (along.last(), path.first()) else {
        return;
    };
    if at <= last || at >= period {
        return;
    }
    if at - last <= merge_distance || period - at <= merge_distance {
        return;
    }
    let t = (at - last) / (period - last);
    let tail = path[path.len() - 1];
    path.push(tail.lerp(first, t));
    along.push(at);
}

/// Miter-офсет вершины ломаной: вектор от точки пути до края ленты полуширины
/// `half_width`, по биссектрисе излома, с ограничением [`MITER_LIMIT`] на
/// острых стыках. Край ленты — `path[i] ± offsets[i]`. Общий и для отрисовки
/// ([`MeshBuilder::push_ribbon`]), и для навмеша (бордюры моста): полосы,
/// заблокированные в сетке, совпадают с нарисованными по построению.
pub fn miter_offsets(path: &[Vec2], closed: bool, half_width: f32) -> Vec<Vec2> {
    let count = path.len();
    (0..count)
        .map(|index| {
            let incoming = (index > 0 || closed).then(|| {
                let previous = path[(index + count - 1) % count];
                (path[index] - previous).normalize_or(Vec2::X).perp()
            });
            let outgoing = (index + 1 < count || closed).then(|| {
                let next = path[(index + 1) % count];
                (next - path[index]).normalize_or(Vec2::X).perp()
            });
            match (incoming, outgoing) {
                (Some(before), Some(after)) => {
                    let bisector = (before + after).normalize_or(before);
                    // на острых стыках длина miter уходит в бесконечность — режем
                    let cosine = bisector.dot(before).max(1.0 / MITER_LIMIT);
                    bisector * (half_width / cosine)
                }
                (Some(normal), None) | (None, Some(normal)) => normal * half_width,
                (None, None) => Vec2::ZERO,
            }
        })
        .collect()
}

/// Ломаная без точек ближе `merge_distance` к предыдущей: на такой дистанции
/// они не видны, но вырождают нормаль стыка. У замкнутой ленты так же
/// подрезается хвост, сошедшийся с началом.
///
/// Эпсилон выбирает потребитель, и они разные: рендеру важно «не видно»
/// (`width / 4`, [`MeshBuilder::push_ribbon`]), полигональному навмешу — только
/// вырожденность (`polymesh::build::DEGENERATE_SPAN`), потому что схлопывание
/// по ширине сдвинуло бы его футпринт относительно заливки сетки.
pub fn merge_close_points(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<Vec2> {
    let merge_distance_sq = merge_distance.powi(2);
    let mut path: Vec<Vec2> = Vec::with_capacity(points.len());
    for &point in points {
        if path
            .last()
            .is_none_or(|last| last.distance_squared(point) > merge_distance_sq)
        {
            path.push(point);
        }
    }
    if closed {
        while path.len() > 1 && path[0].distance_squared(path[path.len() - 1]) <= merge_distance_sq
        {
            path.pop();
        }
    }
    path
}

/// [`merge_close_points`] для ленты: у разомкнутой ломаной конец остаётся на
/// месте, а сливается с ним предпоследняя точка. Иначе лента, срезанная в
/// сантиметрах за вершиной, кончалась бы на вершине и не доходила до шва с
/// соседним куском (клин смены сечения и тело улицы, `roads/tapers.rs`) — щель
/// поперёк проезжей части.
pub fn merge_ribbon_points(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<Vec2> {
    ribbon_vertices(points, closed, merge_distance)
        .into_iter()
        .zip(points)
        .filter_map(|(kept, &point)| kept.then_some(point))
        .collect()
}

/// Какие точки `points` лента оставит вершинами ([`merge_ribbon_points`] —
/// ровно они, по порядку). По ним скругления бордюра в узле
/// (`roads/corners.rs`) меряют прямой край луча: край ленты идёт по этим
/// вершинам, а не по слитым, и дуга по слитой вершине садилась мимо края —
/// у широкой улицы, где слияние берёт четверть ширины, на десятки сантиметров.
pub fn ribbon_vertices(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<bool> {
    let merge_distance_sq = merge_distance.powi(2);
    let mut kept = vec![false; points.len()];
    let mut last: Option<usize> = None;
    let mut count = 0;
    for (index, &point) in points.iter().enumerate() {
        if last.is_none_or(|last| points[last].distance_squared(point) > merge_distance_sq) {
            kept[index] = true;
            last = Some(index);
            count += 1;
        }
    }
    let Some(mut last) = last else {
        return kept;
    };
    let first = kept.iter().position(|&kept| kept).unwrap_or_default();
    if closed {
        // хвост, сошедшийся с началом
        while count > 1 && points[first].distance_squared(points[last]) <= merge_distance_sq {
            kept[last] = false;
            count -= 1;
            last = kept[..last].iter().rposition(|&kept| kept).unwrap_or(first);
        }
    } else if count > 1 && points[last] != points[points.len() - 1] {
        // конец остаётся на месте, сливается с ним предпоследняя точка; путь,
        // схлопнутый в точку, так и остаётся точкой
        kept[last] = false;
        kept[points.len() - 1] = true;
    }
    kept
}

/// Расстояние, на котором лента шириной `width` сливает соседние точки оси
/// ([`merge_ribbon_points`]): на четверти ширины их не видно.
pub fn ribbon_merge_distance(width: f32) -> f32 {
    width / 4.0
}

/// Направление **исходного** крайнего звена `points` у торца `end` — а не
/// звена после слияния точек: у начала — от него вглубь ломаной, у конца —
/// на него (перпендикуляр к нему, [`butt_normal`], в обоих случаях смотрит
/// влево по ходу). Два куска, разрезанные на одном звене, так встают торцами
/// друг к другу без клиновидной щели, даже если у каждого слияние выпрямило
/// конец по-своему. Звено короче сантиметра не в счёт.
pub(crate) fn butt_direction(points: &[Vec2], end: bool) -> Option<Vec2> {
    let away = |tip: Vec2, point: &&Vec2| point.distance(tip) > 0.01;
    let direction = if end {
        let tip = *points.last()?;
        tip - *points.iter().rev().find(|point| away(tip, point))?
    } else {
        let tip = *points.first()?;
        *points.iter().find(|point| away(tip, point))? - tip
    };
    direction.try_normalize()
}

/// Нормаль прямого торца: поперёк исходного крайнего звена
/// ([`butt_direction`]).
pub(super) fn butt_normal(points: &[Vec2], end: bool) -> Option<Vec2> {
    butt_direction(points, end).map(Vec2::perp)
}

/// Не видна ли щель butt-квадов на изломе в `turn` радиан у ленты полуширины
/// `radius`: её ширина `radius · |turn|` мельче [`ARC_TOLERANCE`]. Одно правило
/// на обе стороны излома круглой ленты — такому веер не кладётся
/// ([`MeshBuilder::push_join_fan`]), и такой же квады проходят общими
/// вершинами; разойдись они, вернулась бы либо волосяная щель, либо веер поверх
/// общих вершин.
pub(super) fn join_gap_hidden(radius: f32, turn: f32) -> bool {
    radius * turn.abs() < ARC_TOLERANCE
}
