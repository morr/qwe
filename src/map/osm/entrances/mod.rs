//! Синтетические входы для зданий, у которых OSM их не разметил.
//!
//! Реальные входы (`entrance=*`) есть у ничтожной доли домов — 161 здание из
//! 7355 в Туле, 185 из 21 106 в Нью-Йорке. Остальным дверь приходится
//! придумывать, и делать это надо не наугад, а по тому, как двери размечены
//! там, где их размечали: сколько их на дом, как далеко они друг от друга и
//! куда смотрят. Замеры — по пяти городам (Тула, Нью-Йорк, Париж, Берлин,
//! Лондон), 14 941 привязанный вход; методика и таблицы — в `CONTEXT.md`,
//! раздел «Entrance cohorts».
//!
//! Два вывода замера определяют весь алгоритм:
//!
//! * **Дверь смотрит на улицу.** Медианный угол между внешней нормалью грани
//!   контура и азимутом на ближайшую дорогу — 0.0–0.9° по городам; 95.6%
//!   входов укладываются в 45°, 98% — в 90°. Расстояние до дороги: медиана
//!   0.7 м, p90 5.8 м. То есть вход ставится на ту грань, которая упирается в
//!   проезжую часть, а не на любую внешнюю.
//! * **Число дверей растёт с домом, но медленно.** На 100 м периметра
//!   приходится 4.4 входа у сарая и 0.65 у вокзала — линейная плотность не
//!   годится, нужны когорты.
//!
//! Плюс одно требование, которое замером не выводится, а следует из здравого
//! смысла и из того, что в OSM дома сплошь и рядом стоят вплотную: **дверь не
//! ставится в стену, к которой прижат сосед** — см. [`FootprintIndex`].
//!
//! Генерация детерминирована: LCG засеян геометрией самого здания, поэтому
//! дверь одного и того же дома оказывается на одном и том же месте при каждом
//! запуске и не зависит ни от порядка зданий в выгрузке, ни от того, какие
//! дома обработали раньше.
//!
//! Сколько дверей полагается дому — считает [`cohorts`] по замеру; где именно
//! стоит ближайшая дорога и не прижат ли к стене сосед — отвечает [`index`].

mod cohorts;
mod index;

use bevy::math::Vec2;

use self::cohorts::{cohort_of, entrance_count, equivalent_length};
use self::index::{FootprintIndex, RoadIndex, ring_is_ccw};
use crate::map::osm::model::{AreaKind, MapData, PolyArea};
use crate::settings::NAVTILE_SIZE;

/// Шаг расстановки входов вдоль одной грани, м — медиана зазора между
/// соседними входами в OSM (26.7 м по пяти городам; 22.6 в Туле, где
/// размечены подъезды, 29.0 в Париже).
const ENTRANCE_SPACING: f32 = 25.0;
/// Минимальный зазор между сгенерированными входами, м. p10 замера — 4.5 м, но
/// навтайл здесь 2 м: две двери ближе десятка метров ведут в одну и ту же
/// клетку и как отдельные цели бессмысленны.
const ENTRANCE_MIN_SPACING: f32 = 12.0;
/// Штраф за отворот грани от дороги, м на радиан. Замер: 95.6% входов смотрят
/// на дорогу в пределах 45°, поэтому грань, отвёрнутая на прямой угол,
/// обязана проигрывать вдвое более далёкой, но обращённой к улице (на 90°
/// штраф даёт 31 м).
const ENTRANCE_FACING_PENALTY: f32 = 20.0;
/// Насколько далеко от стены проверяется, свободно ли перед дверью, м. Ровно
/// навтайл: дверь имеет смысл только там, где перед ней есть куда встать, а
/// меньше тайла свободного места навмеш всё равно не разрешит. Заодно этот же
/// зазор съедает разнобой в координатах общей стены — соседние дома в OSM
/// обводят по одному и тому же ряду точек редко.
const ENTRANCE_CLEARANCE: f32 = NAVTILE_SIZE;

/// Грань контура, оценённая на пригодность под вход.
struct Facade {
    from: Vec2,
    to: Vec2,
    /// Внешняя нормаль — по ней проверяется, свободно ли перед стеной.
    outward: Vec2,
    length: f32,
    /// Меньше — лучше: расстояние до дороги плюс штраф за отворот от неё.
    score: f32,
}

/// Расстановка входов у зданий, где OSM их не дал. Возвращает число
/// сгенерированных входов.
pub fn generate_entrances(map: &mut MapData) -> usize {
    let roads = RoadIndex::build(&map.roads);
    let footprints = FootprintIndex::build(&map.buildings);

    // двери сначала считаются по неизменной карте — каждому зданию нужны
    // контуры соседей, — и только потом раскладываются по зданиям
    let mut filled: Vec<(usize, Vec<Vec2>)> = Vec::new();
    let mut walled_in = 0;
    for (index, building) in map.buildings.iter().enumerate() {
        // реальные двери всегда важнее придуманных; стены и башни кремля
        // дверей не несут
        if !building.entrances.is_empty() || building.kind != AreaKind::Building {
            continue;
        }
        let Some(doors) = fill_building(index, building, &roads, &footprints) else {
            continue;
        };
        walled_in += usize::from(doors.forced);
        filled.push((index, doors.entrances));
    }

    if walled_in > 0 {
        // дом целиком накрыт соседями (в OSM это чаще всего корпус, обведённый
        // ещё раз общим контуром квартала) — дверь ему всё равно нужна
        eprintln!("osm parse: {walled_in} buildings have no free wall for a door");
    }

    let mut generated = 0;
    for (index, entrances) in filled {
        generated += entrances.len();
        map.buildings[index].entrances = entrances;
    }
    generated
}

/// Двери одного здания и признак того, что свободной стены у него не нашлось.
struct FilledBuilding {
    entrances: Vec<Vec2>,
    forced: bool,
}

fn fill_building(
    index: usize,
    building: &PolyArea,
    roads: &RoadIndex,
    footprints: &FootprintIndex,
) -> Option<FilledBuilding> {
    let ring = &building.outer;
    if ring.len() < 3 {
        return None;
    }

    let area = crate::map::osm::model::ring_area(ring);
    let perimeter: f32 = (0..ring.len())
        .map(|index| ring[index].distance(ring[(index + 1) % ring.len()]))
        .sum();
    let length = equivalent_length(area, perimeter);
    let cohort = cohort_of(area, length, building.height);
    let mut random = lcg_seeded_by(ring[0]);
    let wanted = entrance_count(&cohort, length, &mut random);

    let mut facades = score_facades(ring, roads);
    // лучшая грань — первой; NaN сюда попасть не может, длина и расстояние
    // всегда конечны
    facades.sort_by(|a, b| a.score.total_cmp(&b.score));

    let free = place_along(&facades, wanted, Some((index, footprints)));
    if !free.is_empty() {
        return Some(FilledBuilding {
            entrances: free,
            forced: false,
        });
    }
    // ни одной свободной стены: дом без двери выпал бы из целей блуждания, так
    // что ставим её на лучшую грань как раньше
    Some(FilledBuilding {
        entrances: place_along(&facades, 1, None),
        forced: true,
    })
}

/// Оценка каждой грани контура: чем ближе к дороге и чем прямее смотрит на
/// неё, тем лучше.
fn score_facades(ring: &[Vec2], roads: &RoadIndex) -> Vec<Facade> {
    let ccw = ring_is_ccw(ring);
    let mut facades = Vec::with_capacity(ring.len());

    for index in 0..ring.len() {
        let from = ring[index];
        let to = ring[(index + 1) % ring.len()];
        let edge = to - from;
        let Some(direction) = edge.try_normalize() else {
            continue;
        };
        // интерьер слева от направления обхода у CCW-кольца, значит наружу —
        // направо
        let outward = if ccw {
            Vec2::new(direction.y, -direction.x)
        } else {
            Vec2::new(-direction.y, direction.x)
        };

        let middle = (from + to) / 2.0;
        // дороги рядом нет (двор в глубине квартала, карта без дорог в тесте):
        // выбирать грань не по чему, но дверь дом всё равно получит — все
        // грани оказываются равны, и берётся первая
        let score = match roads.nearest(middle) {
            Some((road, distance)) => {
                let angle = match (road - middle).try_normalize() {
                    Some(to_road) => outward.dot(to_road).clamp(-1.0, 1.0).acos(),
                    // грань стоит прямо на осевой — отворачиваться не от чего
                    None => 0.0,
                };
                distance + angle * ENTRANCE_FACING_PENALTY
            }
            None => f32::MAX,
        };

        facades.push(Facade {
            from,
            to,
            outward,
            length: edge.length(),
            score,
        });
    }
    facades
}

/// Сколько дверей забирает себе одна грань. Сверху — [`ENTRANCE_SPACING`]:
/// длинный фасад не должен собрать все двери дома в кучу. Снизу — жёсткий
/// предел по [`ENTRANCE_MIN_SPACING`]: двери раскладываются равномерно с шагом
/// `length / (take + 1)`, поэтому `take` дверей помещаются только при
/// `length / (take + 1) >= ENTRANCE_MIN_SPACING`. Одну грань всегда берём хотя
/// бы под одну дверь — иначе у крошечного дома их не осталось бы вовсе.
fn facade_capacity(length: f32) -> usize {
    let preferred = (length / ENTRANCE_SPACING).floor() as usize + 1;
    let limit = ((length / ENTRANCE_MIN_SPACING).floor() as usize).saturating_sub(1);
    preferred.min(limit).max(1)
}

/// Расстановка `wanted` точек по граням в порядке их оценки. Грани
/// перебираются от лучшей к худшей, каждая берёт столько дверей, сколько
/// вмещает ([`facade_capacity`]), и раскладывает их по себе равномерно. Вторая
/// дверь уходит на боковой фасад только тогда, когда на уличном ей уже не
/// хватило места.
///
/// `neighbours` — свой номер и индекс контуров; каждая точка проверяется на
/// [`ENTRANCE_CLEARANCE`] наружу, и место, накрытое чужим домом, пропускается:
/// стена, к которой сосед стоит вплотную, — глухая, дверь на ней смотрит в
/// чужой фасад. Занятая грань просто не отдаёт дверей, и они достаются
/// следующей по оценке. `None` — расставлять, не глядя на соседей (запасной
/// проход для дома, у которого свободной стены не нашлось вовсе).
fn place_along(
    facades: &[Facade],
    wanted: usize,
    neighbours: Option<(usize, &FootprintIndex)>,
) -> Vec<Vec2> {
    let mut placed: Vec<Vec2> = Vec::with_capacity(wanted);
    let minimum_squared = ENTRANCE_MIN_SPACING * ENTRANCE_MIN_SPACING;

    for facade in facades {
        if placed.len() == wanted {
            break;
        }
        let Some(direction) = (facade.to - facade.from).try_normalize() else {
            continue;
        };

        let take = facade_capacity(facade.length).min(wanted - placed.len());
        let step = facade.length / (take + 1) as f32;
        for slot in 1..=take {
            let point = facade.from + direction * (step * slot as f32);
            // соседняя грань могла уже занять угол
            if placed
                .iter()
                .any(|other| other.distance_squared(point) < minimum_squared)
            {
                continue;
            }
            if neighbours.is_some_and(|(owner, footprints)| {
                footprints.is_covered(point + facade.outward * ENTRANCE_CLEARANCE, owner)
            }) {
                continue;
            }
            placed.push(point);
        }
    }
    placed
}

/// Детерминированный LCG, засеянный координатами здания: одна и та же выгрузка
/// даёт одни и те же двери при каждом запуске, и дом не зависит от того, какие
/// здания разобрали до него. Тот же приём и то же семейство, что у посадки
/// деревьев в `parse::plant_trees`.
fn lcg_seeded_by(point: Vec2) -> impl FnMut() -> f32 {
    let mut state: u64 =
        0x9E37_79B9_7F4A_7C15 ^ (point.x.to_bits() as u64) ^ ((point.y.to_bits() as u64) << 32);
    move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as f32) / (u32::MAX >> 1) as f32
    }
}

#[cfg(test)]
mod tests;
