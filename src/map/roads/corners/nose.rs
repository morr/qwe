use bevy::prelude::*;

use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::meshing::arc_steps;
use crate::map::osm::model::closest_on_segment;

use super::{Arm, MIN_RADIUS, NOSE_SETTLE, NOSE_STEP, NOSE_THIN, NOSE_WINDOW, OVERLAP};

/// Нос острой развилки от луча `first` против часовой стрелки до луча
/// `second` ([`NOSE_SHARE`]): асфальт (или тротуар) от острия, где сошлись
/// кромки, до дуги радиуса `radius`, касательной к обеим кромкам. Кромка —
/// полуширина луча ([`Arm::half_at`], с клином) плюс `extra` (тротуар:
/// левый у первого, правый у второго).
///
/// Кромки идут по **осевым лучей** ([`Arm::trail`]), а не по их начальным
/// направлениям: остриё лежит в десятках метров от узла, и прямая от узла
/// ушла бы с гнутой ленты — съезд с кольца, ветка по дуге — на метры. Шаг
/// [`NOSE_STEP`] вдоль первого луча: его кромка, ближайшая к ней точка оси
/// второго и просвет между кромками; остриё — где просвет стал положительным,
/// центр носа — середина просвета в два радиуса. Не нашлось (второй луч
/// кончился, просвет не дорос за [`NOSE_REACH`]) — `None`.
///
/// `gored` — перед носом ещё и штрихуемый клин развилки ([`fork_gore`]) по тем
/// же пробам.
pub(super) fn nose(
    first: &Arm,
    second: &Arm,
    extra: (f32, f32),
    radius: f32,
    gored: bool,
) -> Option<Nose> {
    if radius < MIN_RADIUS {
        return None;
    }
    let (first_trail, second_trail) = (first.trail(), second.trail());
    let (along_first, total_first) = arclengths(&first_trail);
    let (along_second, _) = arclengths(&second_trail);
    let links = second_trail
        .len()
        .checked_sub(1)
        .filter(|&links| links > 0)?;
    // звено второго, у которого нашлась прошлая проба: пробы идут вперёд, и
    // ближайшая точка ищется в окне от него, а не по всей осевой
    let mut hint: usize = 0;
    // кромка первого на `at`, лицом к ней — кромка второго, и просвет между
    // ними; в пробе — ещё полуширины обоих и нормали к кромкам внутрь развилки
    let mut facing = |at: f32| -> Option<(Vec2, Vec2, f32, Station)> {
        let (point, tangent) = place_on_path(&first_trail, &along_first, at)?;
        let near_half = first.half_at(0, at) + extra.0;
        let edge = point + tangent.perp() * near_half;
        let (link, onto) = (hint.saturating_sub(NOSE_WINDOW)..(hint + NOSE_WINDOW).min(links))
            .map(|link| {
                let onto = closest_on_segment(edge, second_trail[link], second_trail[link + 1]);
                (link, onto)
            })
            .min_by(|(_, a), (_, b)| {
                a.distance_squared(edge)
                    .total_cmp(&b.distance_squared(edge))
            })?;
        hint = link;
        // второй кончился: ближайшая точка — его конец, не перпендикуляр
        if link + 1 == links && onto == second_trail[links] {
            return None;
        }
        let reach = along_second[link] + second_trail[link].distance(onto);
        let direction = (second_trail[link + 1] - second_trail[link]).try_normalize()?;
        let out = -direction.perp();
        let half = second.half_at(1, reach) + extra.1;
        let gap = (edge - onto).dot(out) - half;
        let opposite = onto + out * half;
        let station = Station {
            at,
            edges: [edge, opposite],
            gap,
            halves: [near_half, half],
            inward: [tangent.perp(), out],
        };
        Some((edge, opposite, gap, station))
    };
    let mut tip = None;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut stations = Vec::new();
    let mut at = 0.0;
    // середина просвета в два радиуса и докуда ещё вести стороны: основания
    // перпендикуляров из центра лягут чуть дальше неё
    let mut found: Option<(Vec2, f32)> = None;
    // проба, на которой просвет дорос до двух радиусов, — там кончается клин
    let mut widest = None;
    while at <= total_first {
        let Some((edge, opposite, gap, station)) = facing(at) else {
            break;
        };
        if gored {
            stations.push(station);
        }
        if gap < 0.0 && found.is_none() {
            // остриё — середина скрещённых кромок последней пробы, под обеими
            // лентами
            tip = Some((edge + opposite) / 2.0);
        } else {
            let tip = tip?;
            if left.is_empty() {
                left.push(tip);
                right.push(tip);
            }
            // стороны заходят под ленты на `OVERLAP`; в узкой части клина —
            // вершина через [`NOSE_THIN`] проб: у развилки под 8° он тянется
            // на двадцать метров, и все его пробы были бы вершинами меша
            if found.is_some()
                || gap >= radius
                || ((at / NOSE_STEP) as usize).is_multiple_of(NOSE_THIN)
            {
                let inward = (opposite - edge).normalize_or_zero() * OVERLAP;
                left.push(edge - inward);
                right.push(opposite + inward);
            }
            match found {
                None if gap >= 2.0 * radius => {
                    found = Some(((edge + opposite) / 2.0, at + 2.0 * radius));
                    widest = stations.len().checked_sub(1);
                }
                Some((_, until)) if at >= until => break,
                _ => {}
            }
        }
        at += NOSE_STEP;
    }
    let (mut centre, _) = found?;
    // Дуга касается обеих кромок: центр сдвигается, пока не встанет на радиус
    // от каждой. Середина просвета — только начало: гнутая кромка (кольцо
    // выпукло в сторону островка) ближе к ней, чем прямая, и дуга в радиус до
    // ближней не доставала до дальней — у острия оставалась зазубрина.
    for _ in 0..NOSE_SETTLE {
        let (left_foot, _) = nearest_on_path(&left, centre)?;
        let (right_foot, _) = nearest_on_path(&right, centre)?;
        let (away_left, away_right) = (centre - left_foot, centre - right_foot);
        centre += away_left.normalize_or_zero() * (radius - away_left.length())
            + away_right.normalize_or_zero() * (radius - away_right.length());
    }
    let (foot_left, cut_left) = nearest_on_path(&left, centre)?;
    let (foot_right, cut_right) = nearest_on_path(&right, centre)?;
    let radius = centre.distance(foot_left).min(centre.distance(foot_right));
    if radius < MIN_RADIUS {
        return None;
    }
    let keep = |chain: &[Vec2], cut: f32| -> Vec<Vec2> {
        let (along, _) = arclengths(chain);
        chain
            .iter()
            .zip(&along)
            .take_while(|(_, along)| **along < cut)
            .map(|(point, _)| *point)
            .collect()
    };
    let mut outline = keep(&left, cut_left);
    let from = (foot_left - centre).normalize_or_zero() * radius;
    let to = (foot_right - centre).normalize_or_zero() * radius;
    // короткой стороной — она и смотрит на остриё
    let sweep = from.angle_to(to);
    let steps = arc_steps(radius, sweep.abs()).max(1);
    let arc: Vec<Vec2> = (0..=steps)
        .map(|step| centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from))
        .collect();
    outline.extend(&arc);
    // остриё — уже первая вершина левой стороны
    outline.extend(keep(&right, cut_right).into_iter().skip(1).rev());
    if outline.len() < 3 {
        return None;
    }
    let gore = widest.and_then(|widest| fork_gore(&stations[..=widest], &arc));
    Some(Nose { outline, gore })
}

/// Нос развилки ([`nose`]) и штрихуемый клин перед ним ([`fork_gore`]).
pub(super) struct Nose {
    pub(super) outline: Vec<Vec2>,
    pub(super) gore: Option<Vec<Vec2>>,
}

/// Проба носа ([`nose`]) в `at` метрах по первому лучу: кромки обоих лучей
/// лицом друг к другу, просвет между ними (меньше нуля — ленты
/// перекрываются), полуширины (с тротуаром, если он в кромке) и нормали к
/// кромкам внутрь развилки.
struct Station {
    at: f32,
    edges: [Vec2; 2],
    gap: f32,
    halves: [f32; 2],
    inward: [Vec2; 2],
}

/// Клин развилки короче этого, м, не штрихуется. У развилки острее 25° он
/// не короче 12 м (полоса и два радиуса носа на синус угла); короче выходит
/// между гнутыми лучами внутри сложного узла — треугольнички в 3–5 м
/// островками посреди его асфальта (Орёл, витрина 04).
const FORK_GORE_MIN: f32 = 8.0;

/// Штрихуемый клин перед носом острой развилки. Ленты двух улиц под 8–25°
/// перекрываются на десятки метров, и нос встаёт только там, где их кромки
/// разошлись, — до него между полотнами тянулся ровный асфальтовый язык
/// (Тула, витрина 14: 34 м от узла). На земле полосы расходятся раньше, а
/// между ними — штриховка. Клин начинается, где ось узкой улицы вышла из
/// полотна широкой (перекрытие кромок сошло до меньшей полуширины), и
/// расходится до ширины просвета у носа (`stations` кончаются пробой, где
/// просвет дорос до двух радиусов носа), кончаясь поперёк у вершины дуги
/// носа `arc`. Стороны
/// заходят в полотна на недостающую ширину — каждая пропорционально своей
/// полуширине, так что обе полосы сужаются заодно. `None` — клин короче
/// [`FORK_GORE_MIN`] или ось узкой не выходит из широкой до носа.
fn fork_gore(stations: &[Station], arc: &[Vec2]) -> Option<Vec<Vec2>> {
    let (widest, _) = stations.split_last()?;
    let start = stations
        .iter()
        .position(|station| station.gap >= -station.halves[0].min(station.halves[1]))?;
    let (from, to) = (stations[start].at, widest.at);
    if to - from < FORK_GORE_MIN {
        return None;
    }
    let mut sides = [Vec::new(), Vec::new()];
    for station in &stations[start..] {
        let width = widest.gap * (station.at - from) / (to - from);
        let cut = (width - station.gap).max(0.0);
        let total = station.halves[0] + station.halves[1];
        for (side, points) in sides.iter_mut().enumerate() {
            let into = cut * station.halves[side] / total;
            points.push(station.edges[side] - station.inward[side] * into);
        }
    }
    // Кончается клин поперёк, по касательной к вершине дуги носа: вдоль самой
    // дуги по бордюру обводка легла бы белой скобой с крючками у её концов.
    let tip = sides[0][0].midpoint(sides[1][0]);
    let apex = arc
        .iter()
        .copied()
        .min_by(|a, b| a.distance(tip).total_cmp(&b.distance(tip)))?;
    let along = (apex - tip).try_normalize()?;
    let reach = apex.distance(tip);
    let side = |points: &[Vec2]| -> Vec<Vec2> {
        let depth = |point: Vec2| (point - tip).dot(along);
        let mut kept = Vec::new();
        for (index, pair) in points.windows(2).enumerate() {
            // прямым сторонам вершина через [`NOSE_THIN`] проб не нужна
            if index % NOSE_THIN == 0 {
                kept.push(pair[0]);
            }
            let [near, far] = [depth(pair[0]), depth(pair[1])];
            if far >= reach {
                let share = ((reach - near) / (far - near)).clamp(0.0, 1.0);
                kept.push(pair[0].lerp(pair[1], share));
                return kept;
            }
        }
        kept.extend(points.last());
        kept
    };
    let mut outline = side(&sides[0]);
    // остриё — уже первая вершина первой стороны
    let back = side(&sides[1]);
    outline.extend(back.into_iter().skip(1).rev());
    (outline.len() >= 3).then_some(outline)
}
