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
//! Сколько дверей полагается дому — считает `cohorts` по замеру; где именно
//! стоит ближайшая дорога и не прижат ли к стене сосед — отвечает `index`.

mod cohorts;
mod index;

use bevy::math::Vec2;

use self::cohorts::{cohort_of, entrance_count, equivalent_length};
use self::index::{FootprintIndex, RoadIndex, ring_is_ccw};
use crate::map::osm::model::{AreaKind, BuildingUse, MapData, PolyArea};
use crate::rng::lcg_seeded_by;
use crate::settings::navtile_size;

/// Шаг расстановки входов вдоль одной грани, м — медиана зазора между
/// соседними входами в OSM (26.7 м по пяти городам; 22.6 в Туле, где
/// размечены подъезды, 29.0 в Париже).
const ENTRANCE_SPACING: f32 = 25.0;
/// Минимальный зазор между сгенерированными входами, м. p10 замера — 4.5 м, но
/// навтайл здесь 2 м (по умолчанию): две двери ближе десятка метров ведут в
/// одну и ту же клетку и как отдельные цели бессмысленны.
const ENTRANCE_MIN_SPACING: f32 = 12.0;
/// Штраф за отворот грани от дороги, м на радиан. Замер: 95.6% входов смотрят
/// на дорогу в пределах 45°, поэтому грань, отвёрнутая на прямой угол,
/// обязана проигрывать вдвое более далёкой, но обращённой к улице (на 90°
/// штраф даёт 31 м).
const ENTRANCE_FACING_PENALTY: f32 = 20.0;
/// Насколько далеко от стены проверяется, свободно ли перед дверью — ровно
/// навтайл ([`navtile_size`]): дверь имеет смысл только там, где перед ней
/// есть куда встать, а меньше тайла свободного места навмеш всё равно не
/// разрешит. Заодно этот же зазор съедает разнобой в координатах общей
/// стены — соседние дома в OSM обводят по одному и тому же ряду точек редко.
fn entrance_clearance() -> f32 {
    navtile_size()
}

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

/// Расстановка входов там, где OSM их не дал, и **доводка там, где дал не
/// все**. Возвращает число придуманных входов.
///
/// Дом с размеченными дверями раньше пропускался целиком, и это было ошибкой
/// той же природы, что и порог «не меньше двух дверей» в замере когорт: маппер
/// сплошь и рядом ставит один вход и бросает. Замер от такой разметки
/// защищался, а генерация — нет, и лондонский квартал в четверть километра
/// оставался с единственной дверью. Настоящие двери по-прежнему **не двигаются
/// и не выбрасываются**: они идут первыми, а когорта лишь дописывает недостающее.
pub fn generate_entrances(map: &mut MapData) -> usize {
    let roads = RoadIndex::build(&map.roads);
    let footprints = FootprintIndex::build(&map.buildings);

    // двери сначала считаются по неизменной карте — каждому зданию нужны
    // контуры соседей, — и только потом раскладываются по зданиям
    let mut filled: Vec<(usize, Vec<Vec2>)> = Vec::new();
    let mut walled_in = 0;
    for (index, building) in map.buildings.iter().enumerate() {
        // стены и башни кремля дверей не несут
        if building.kind != AreaKind::Building {
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
        // размеченные в OSM двери входят в этот список первыми и никуда не
        // делись — придуманными считаются только дописанные
        generated += entrances.len() - map.buildings[index].entrances.len();
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

    // размеченные двери — первыми и неприкосновенными; нормаль каждой берётся
    // у ближайшей к ней грани, чтобы и она могла пройти домом насквозь
    let mut doors: Vec<Door> = building
        .entrances
        .iter()
        .map(|&at| Door {
            at,
            outward: facade_outward(ring, at),
        })
        .collect();
    let real = doors.len();
    place_along(&facades, wanted, Some((index, footprints)), &mut doors);

    if !doors.is_empty() {
        let mut entrances: Vec<Vec2> = doors.iter().map(|door| door.at).collect();
        // сквозной подъезд: у корпуса дверь выходит и во двор. Дом-свечка и
        // короткий дом сюда не попадают — см. [`THROUGH_MIN_LENGTH`]
        if through_entrances(building, length) {
            entrances.extend(through_doors(ring, &doors, index, footprints));
        }
        return Some(FilledBuilding {
            entrances,
            forced: false,
        });
    }
    // ни одной свободной стены: дом без двери выпал бы из целей блуждания, так
    // что ставим её на лучшую грань как раньше
    let mut forced = Vec::new();
    place_along(&facades, 1, None, &mut forced);
    Some(FilledBuilding {
        entrances: forced.into_iter().map(|door| door.at).collect(),
        // дом с размеченной дверью сюда не доходит, так что счётчик считает
        // именно глухие дома, а не всякий бездверный случай
        forced: real == 0,
    })
}

/// Внешняя нормаль ближайшей к точке грани кольца. Для размеченной в OSM
/// двери — а она стоит в **вершине** контура — годится любая из двух её
/// граней: сквозной луч пойдёт внутрь дома в обоих случаях.
fn facade_outward(ring: &[Vec2], at: Vec2) -> Vec2 {
    let ccw = ring_is_ccw(ring);
    let mut best = (f32::MAX, Vec2::Y);
    for index in 0..ring.len() {
        let from = ring[index];
        let to = ring[(index + 1) % ring.len()];
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let reach = (to - from).length();
        let at_edge = (at - from).dot(along).clamp(0.0, reach);
        let distance = at.distance(from + along * at_edge);
        if distance < best.0 {
            let outward = match ccw {
                true => Vec2::new(along.y, -along.x),
                false => Vec2::new(-along.y, along.x),
            };
            best = (distance, outward);
        }
    }
    best.1
}

/// Бывает ли у этого дома сквозной подъезд.
///
/// Назначение решает первым: сквозной подъезд — примета **многоквартирного**
/// дома. `Other` идёт вместе с `Apartments` не по недосмотру, а потому что это
/// половина города (`building=yes`), и панельные корпуса Тулы сидят именно в
/// ней. Частный дом, гараж, склад, храм отсеиваются здесь, длинный, но мелкий
/// сарай — порогом длины, а глубину каждой двери проверяет уже сам луч
/// ([`through_doors`]): у свечки противоположная стена дальше
/// [`THROUGH_DEPTH_RANGE`], и второй двери она не получит.
fn through_entrances(building: &PolyArea, length: f32) -> bool {
    matches!(
        building.building_use,
        BuildingUse::Apartments | BuildingUse::Other
    ) && length >= THROUGH_MIN_LENGTH
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
/// [`entrance_clearance`] наружу, и место, накрытое чужим домом, пропускается:
/// стена, к которой сосед стоит вплотную, — глухая, дверь на ней смотрит в
/// чужой фасад. Занятая грань просто не отдаёт дверей, и они достаются
/// следующей по оценке. `None` — расставлять, не глядя на соседей (запасной
/// проход для дома, у которого свободной стены не нашлось вовсе).
///
/// `placed` приходит **не пустым**, когда двери дому уже размечены в OSM:
/// `wanted` — это сколько дверей у дома должно быть всего, а не сколько
/// дописать, и размеченные занимают своё место в зазоре наравне с
/// придуманными.
fn place_along(
    facades: &[Facade],
    wanted: usize,
    neighbours: Option<(usize, &FootprintIndex)>,
    placed: &mut Vec<Door>,
) {
    let minimum_squared = ENTRANCE_MIN_SPACING * ENTRANCE_MIN_SPACING;

    for facade in facades {
        if placed.len() >= wanted {
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
                .any(|other| other.at.distance_squared(point) < minimum_squared)
            {
                continue;
            }
            if neighbours.is_some_and(|(owner, footprints)| {
                footprints.is_covered(point + facade.outward * entrance_clearance(), owner)
            }) {
                continue;
            }
            placed.push(Door {
                at: point,
                outward: facade.outward,
            });
        }
    }
}

/// Поставленный вход: где он и куда смотрит. Нормаль нужна ровно затем, чтобы
/// пройти подъездом насквозь ([`through_doors`]) — наружу по ней уже проверена
/// свобода перед дверью.
#[derive(Clone, Copy)]
struct Door {
    at: Vec2,
    outward: Vec2,
}

/// Длина и глубина корпуса, при которых подъезд считается **сквозным**, м.
///
/// Панельная секция — это дом с двумя выходами из одного подъезда, на улицу и
/// во двор, и на снимке это видно: двери стоят парами напротив друг друга.
/// Отсюда обе границы. Снизу по длине — секция короче сорока метров это уже не
/// секция, а отдельный дом; по глубине — сквозным подъезд бывает начиная
/// примерно с восьми метров, а гаражный ряд (сто метров в длину, четыре в
/// глубину) под правило попадать не должен. Сверху по глубине — **дом-свечка**:
/// у новой квадратной высотки вход часто **один**, и второй, приделанный с
/// изнанки, читался бы ошибкой.
const THROUGH_MIN_LENGTH: f32 = 40.0;
const THROUGH_DEPTH_RANGE: std::ops::RangeInclusive<f32> = 8.0..=20.0;

/// Вторая дверь того же подъезда — на противоположной стене.
///
/// Это **не лишний вход**, а другой конец уже посчитанного: когорты отвечают на
/// вопрос «сколько подъездов», и число их здесь не растёт. Луч идёт от двери
/// внутрь дома по нормали её грани; годится первое пересечение с контуром
/// дальше [`THROUGH_DEPTH_RANGE`], и только если та грань смотрит навстречу —
/// у Г-образного корпуса луч упирается в собственное крыло, и дверь там
/// выходила бы в торец соседнего подъезда.
fn through_doors(
    ring: &[Vec2],
    doors: &[Door],
    index: usize,
    footprints: &FootprintIndex,
) -> Vec<Vec2> {
    let mut twins = Vec::new();
    let minimum_squared = ENTRANCE_MIN_SPACING * ENTRANCE_MIN_SPACING;
    for door in doors {
        let inward = -door.outward;
        let Some((exit, outward)) = ring_exit(ring, door.at, inward) else {
            continue;
        };
        if !THROUGH_DEPTH_RANGE.contains(&door.at.distance(exit))
            || outward.dot(door.outward) > -0.7
        {
            continue;
        }
        if footprints.is_covered(exit + outward * entrance_clearance(), index) {
            continue;
        }
        if doors
            .iter()
            .map(|other| other.at)
            .chain(twins.iter().copied())
            .any(|other| other.distance_squared(exit) < minimum_squared)
        {
            continue;
        }
        twins.push(exit);
    }
    twins
}

/// Где луч из точки внутри контура выходит наружу: ближайшее пересечение с
/// гранью кольца плюс её внешняя нормаль.
fn ring_exit(ring: &[Vec2], from: Vec2, direction: Vec2) -> Option<(Vec2, Vec2)> {
    let ccw = ring_is_ccw(ring);
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        let edge = b - a;
        let denominator = direction.perp_dot(edge);
        if denominator.abs() < 1e-6 {
            continue;
        }
        let offset = a - from;
        // from + direction·t = a + edge·u
        let t = offset.perp_dot(edge) / denominator;
        let u = offset.perp_dot(direction) / denominator;
        if t <= 1e-3 || !(0.0..=1.0).contains(&u) {
            continue;
        }
        if best.is_none_or(|(nearest, ..)| t < nearest) {
            let Some(along) = edge.try_normalize() else {
                continue;
            };
            let outward = match ccw {
                true => Vec2::new(along.y, -along.x),
                false => Vec2::new(-along.y, along.x),
            };
            best = Some((t, from + direction * t, outward));
        }
    }
    best.map(|(_, point, outward)| (point, outward))
}

#[cfg(test)]
mod tests;
