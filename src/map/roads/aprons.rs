//! **Фартуки у мостиков** (slit aprons) — заливка щели между настилом улицы и
//! мостиком, замапленным рядом с ним отдельным `bridge=yes` way, и её
//! продолжения на земле между тротуаром подхода и дорожкой, что ведёт на
//! мостик.
//!
//! OSM рисует тротуар моста своим way в метрах от проезжей части: у моста
//! 1-й Курской в Орле (88463507) мостики 375888718/719 идут в 13.5 м от оси
//! настила шириной 14.2 — между бортиком настила и бортиком мостика 2.15 м
//! пустоты, и сквозь неё видны пути под мостом (S2, R32). Мостик, замапленный
//! **внутри** настила, выносится к его кромке (`axis.rs::place_bridge_sidewalks`),
//! а этот стоит, где стоял. На деле между ними плита — и щель закрывается
//! **отрисовкой**: фартук в слое настилов цветом мостика, от наружной кромки
//! бортика настила до кромки заливки мостика (его внутренний бортик фартук
//! кроет). Навмеш не трогается: бортик настила — его граница, щель и так
//! непроходима.
//!
//! На земле то же: дорожка, продолжающая мостик, и полоса тротуара улицы,
//! продолжающей настил, сходятся к голове моста и оставляют между собой иглу
//! газона или клин голой земли (игла у северной головы 1-й Курской; клин у
//! Одоевского путепровода в Туле, S5). Промежуток уже [`SLIT_MAX`] от головы
//! назад, пока он такой, мостится в слое дорожки её цветом.

use std::borrow::Cow;

use bevy::prelude::*;

use super::drawn::Drawn;
use crate::map::along::densify;
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine};

/// Щель шире этого, м, — не щель, а земля между двумя мостами.
pub(super) const SLIT_MAX: f32 = 3.0;
/// Косинус угла между мостиком и настилом, при котором мостик идёт вдоль
/// (как у `axis.rs::FOOTBRIDGE_ALONG`).
const ALONG_MIN: f32 = 0.9;
/// Шаг проб вдоль мостика и дорожки, м.
const STEP: f32 = 1.0;
/// Как далеко от головы моста фартук идёт по земле, м: игла у 1-й Курской
/// и клин у Одоевского путепровода — метров по девять.
const GROUND_REACH: f32 = 15.0;
/// Щель уже этого, м, — стык, а не щель.
const CLOSED: f32 = 0.02;
/// На сколько фартук на земле заходит за голову под настил, м.
const GROUND_LIP: f32 = 1.0;

/// Фартуки: на настилах (в слой настилов) и на земле (в слой дорожки), с
/// индексом дорожки или мостика, чей цвет они берут.
#[derive(Default)]
pub(super) struct Aprons {
    pub decks: Vec<(usize, Vec<Vec2>)>,
    pub ground: Vec<(usize, Vec<Vec2>)>,
}

/// Фартуки всех мостиков у настилов улиц. `ribbon` — оси, как они
/// нарисованы.
pub(super) fn slit_aprons(
    drawn: &[&RoadLine],
    ribbon: &[Cow<'_, [Vec2]>],
    prepared: &Drawn,
) -> Aprons {
    let mut aprons = Aprons::default();
    let decks: Vec<usize> = (0..drawn.len())
        .filter(|&index| is_street_deck(drawn[index]) && ribbon[index].len() >= 2)
        .collect();
    let boxes: Vec<(Vec2, Vec2)> = decks
        .iter()
        .map(|&index| bounds(&ribbon[index], drawn[index].curb_reach() + SLIT_MAX + 4.0))
        .collect();
    // улицы на земле: у мостика, идущего берегом без настила, щель между
    // тротуаром улицы и им (R32) — берег моста длиннее настила, и тройка
    // «тротуар, щель грунта, мостик» читалась вдоль всего подхода
    let streets: Vec<(usize, (Vec2, Vec2))> = (0..drawn.len())
        .filter(|&index| {
            !drawn[index].bridge
                && drawn[index].class == RoadClass::Street
                && ribbon[index].len() >= 2
        })
        .map(|index| {
            let pad = drawn[index].curb_reach() + SLIT_MAX + drawn[index].width / 2.0 + 4.0;
            (index, bounds(&ribbon[index], pad))
        })
        .collect();
    for foot in (0..drawn.len()).filter(|&index| is_footbridge(drawn[index])) {
        let path: &[Vec2] = &ribbon[foot];
        if path.len() < 2 {
            continue;
        }
        let (low, high) = bounds(path, 0.0);
        for (&deck, &(deck_low, deck_high)) in decks.iter().zip(&boxes) {
            if high.cmplt(deck_low).any() || low.cmpgt(deck_high).any() {
                continue;
            }
            let inner = drawn[deck].curb_reach();
            let outer = drawn[foot].curb_reach();
            let fill = drawn[foot].width / 2.0;
            // вдоль настила ли мостик вообще — вплоть до вынесенного к его
            // кромке (бортики внахлёст, щели нет, а на земле клин есть: S5)
            let alongside = !beside(path, &ribbon[deck], |_, distance| {
                (distance - inner - outer <= SLIT_MAX).then_some([inner, inner])
            })
            .is_empty();
            if !alongside {
                continue;
            }
            let runs = beside(path, &ribbon[deck], |_, distance| {
                let clear = distance - inner - outer;
                (clear > CLOSED && clear <= SLIT_MAX).then_some([inner, distance - fill])
            });
            aprons
                .decks
                .extend(runs.into_iter().map(|(_, ring)| (foot, ring)));
            // и на земле — у обоих торцов мостика, что стоят на шве
            for end in [0, 1] {
                if let Some(ring) = ground_apron(drawn, ribbon, prepared, foot, deck, end) {
                    aprons.ground.push(ring);
                }
            }
        }
        // берегом, без настила: щель между тротуаром улицы и мостиком —
        // плиткой (в слое дорожки, цвет мостика), от кромки её полосы до
        // кромки настила мостика (R32)
        let fill = drawn[foot].width / 2.0;
        for &(street, (street_low, street_high)) in &streets {
            if high.cmplt(street_low).any() || low.cmpgt(street_high).any() {
                continue;
            }
            let runs = beside(path, &ribbon[street], |side, distance| {
                let band = prepared.band_half(street, side);
                let clear = distance - band - fill;
                (clear > CLOSED && clear <= SLIT_MAX).then_some([band, distance - fill])
            });
            aprons
                .ground
                .extend(runs.into_iter().map(|(_, ring)| (foot, ring)));
        }
    }
    aprons
}

/// Фартук на земле у торца `end` мостика `foot` рядом с настилом `deck`:
/// между дорожкой, продолжающей мостик, и полосой тротуара улицы,
/// продолжающей настил, — от головы назад, пока промежуток уже [`SLIT_MAX`].
fn ground_apron(
    drawn: &[&RoadLine],
    ribbon: &[Cow<'_, [Vec2]>],
    prepared: &Drawn,
    foot: usize,
    deck: usize,
    end: usize,
) -> Option<(usize, Vec<Vec2>)> {
    let osm_end = |road: usize, end: usize| {
        let points = &drawn[road].points;
        if end == 0 {
            points.first()
        } else {
            points.last()
        }
        .copied()
    };
    let node = osm_end(foot, end)?;
    let path_road = prepared
        .nodes()
        .roads_at(node)
        .iter()
        .copied()
        .find(|&other| {
            other != foot && !drawn[other].bridge && drawn[other].class == RoadClass::Alley
        })?;
    // улица у торца настила, ближнего к этому торцу мостика
    let deck_node = [0, 1]
        .into_iter()
        .filter_map(|deck_end| osm_end(deck, deck_end))
        .min_by(|a, b| a.distance(node).total_cmp(&b.distance(node)))?;
    let street = prepared
        .nodes()
        .roads_at(deck_node)
        .iter()
        .copied()
        .find(|&other| !drawn[other].bridge && drawn[other].class == RoadClass::Street)?;
    // дорожка — от торца у мостика назад
    let mut path: Vec<Vec2> = ribbon[path_road].to_vec();
    if path.len() < 2 {
        return None;
    }
    if path[path.len() - 1].distance(node) < path[0].distance(node) {
        path.reverse();
    }
    let length = polyline_length(&path).min(GROUND_REACH);
    let path = super::tapers::cut(&path, 0.0, length);
    let street_path: &[Vec2] = &ribbon[street];
    let half = drawn[path_road].width / 2.0;
    let asphalt = drawn[street].width / 2.0;
    // внахлёст с полосой (у самой головы, пока дорожка идёт за вынесенным
    // мостиком) — тоже фартук: он под ними, и клин за нахлёстом начинается
    // сразу от головы
    let mut runs = beside(&path, street_path, |side, distance| {
        let clear = distance - prepared.band_half(street, side) - half;
        (clear <= SLIT_MAX).then_some([asphalt, (distance - half).max(asphalt)])
    });
    // только кусок от самой головы: дальше промежуток — уже газон обочины
    let (start, _) = runs.first()?;
    if *start > 1 {
        return None;
    }
    let (_, mut ring) = runs.swap_remove(0);
    // и губой под фартук настила: его торец стоит поперёк мостика, торец
    // этого — поперёк дорожки, и между ними оставалась щепка газона
    let out = (path[0] - path[1]).try_normalize()?;
    let (near, far) = (ring[0], ring[ring.len() - 1]);
    ring.insert(0, near + out * GROUND_LIP);
    ring.push(far + out * GROUND_LIP);
    Some((path_road, ring))
}

/// Куски пути `path`, идущего вдоль `along` (не за его торцами): в каждой
/// пробе через [`STEP`] `span(сторона, расстояние)` говорит, от какого до
/// какого расстояния от оси `along` лежит заливка, или `None` — пробы нет.
/// Кольцо куска — ближняя к `along` кромка вперёд, дальняя назад; при нём —
/// номер его первой пробы.
fn beside(
    path: &[Vec2],
    along: &[Vec2],
    span: impl Fn(usize, f32) -> Option<[f32; 2]>,
) -> Vec<(usize, Vec<Vec2>)> {
    let dense = densify(path, STEP);
    let mut runs = Vec::new();
    let mut near: Vec<Vec2> = Vec::new();
    let mut far: Vec<Vec2> = Vec::new();
    let mut start = 0;
    let mut flush = |near: &mut Vec<Vec2>, far: &mut Vec<Vec2>, start: usize| {
        if near.len() >= 2 {
            let mut ring = std::mem::take(near);
            ring.extend(far.drain(..).rev());
            runs.push((start, ring));
        }
        near.clear();
        far.clear();
    };
    for (index, &point) in dense.iter().enumerate() {
        let heading = if index + 1 < dense.len() {
            dense[index + 1] - point
        } else {
            point - dense[index - 1]
        };
        let found = heading
            .try_normalize()
            .and_then(|heading| foot_on(along, point, heading))
            .and_then(|(foot, normal, side)| {
                let [from, to] = span(side, point.distance(foot))?;
                Some((foot + normal * from, foot + normal * to))
            });
        match found {
            Some((a, b)) => {
                if near.is_empty() {
                    start = index;
                }
                near.push(a);
                far.push(b);
            }
            None => flush(&mut near, &mut far, start),
        }
    }
    flush(&mut near, &mut far, start);
    runs
}

/// Основание перпендикуляра из `point` на `path`, если оно на звене (не за
/// торцом пути) и звено идёт вдоль `heading`: точка, нормаль к ней и сторона
/// (`0` — слева по ходу `path`).
fn foot_on(path: &[Vec2], point: Vec2, heading: Vec2) -> Option<(Vec2, Vec2, usize)> {
    let mut best: Option<(f32, Vec2, Vec2, usize)> = None;
    for pair in path.windows(2) {
        let link = pair[1] - pair[0];
        let length = link.length();
        let Some(direction) = link.try_normalize() else {
            continue;
        };
        if direction.dot(heading).abs() < ALONG_MIN {
            continue;
        }
        let t = (point - pair[0]).dot(direction);
        if !(0.0..=length).contains(&t) {
            continue;
        }
        let foot = pair[0] + direction * t;
        let offset = point - foot;
        let distance = offset.length();
        if best.is_some_and(|(best, ..)| best <= distance) {
            continue;
        }
        let Some(normal) = offset.try_normalize() else {
            continue;
        };
        let side = usize::from(direction.perp_dot(offset) < 0.0);
        best = Some((distance, foot, normal, side));
    }
    best.map(|(_, foot, normal, side)| (foot, normal, side))
}

fn is_street_deck(road: &RoadLine) -> bool {
    road.bridge && road.class == RoadClass::Street && !road.passage
}

fn is_footbridge(road: &RoadLine) -> bool {
    road.bridge && road.class == RoadClass::Alley && !road.passage
}

fn bounds(path: &[Vec2], pad: f32) -> (Vec2, Vec2) {
    let (low, high) = path.iter().fold((path[0], path[0]), |(low, high), &point| {
        (low.min(point), high.max(point))
    });
    (low - pad, high + pad)
}
