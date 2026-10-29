//! Выпрямление косых домиков: криво обведённый четырёхугольник становится
//! прямоугольником, кривая «Г» — «Г» из прямых углов ([`square_skewed_houses`]).

use bevy::math::Vec2;

use super::{vertex_key, vertex_uses};
use crate::map::osm::model::{AreaKind, BuildingUse, MapData, signed_ring_area};

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
pub(super) const ELL_AREA_DRIFT: f32 = 0.15;

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
/// Место в конвейере — **шаг 4** [`super::finish_parse`]: порядок проходов записан
/// там целиком и с доводом у каждого шага, и записан ровно в одном месте.
pub(super) fn square_skewed_houses(map: &mut MapData) -> usize {
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
pub(super) fn fit_rectangle(quad: &[Vec2; 4]) -> [Vec2; 4] {
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
pub(super) fn fit_ell(ring: &[Vec2]) -> Option<Vec<Vec2>> {
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
/// вершин ([`crate::map::osm::model::ring_vertex_mean`]): на вытянутом или невыпуклом контуре эти две
/// точки расходятся на метры, поэтому у них разные имена, а не одно на двоих.
pub(super) fn ring_area_centroid(ring: &[Vec2]) -> Vec2 {
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

#[cfg(test)]
mod tests;
