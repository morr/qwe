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

use bevy::math::Vec2;

use crate::map::osm::model::{
    AreaKind, MapData, PolyArea, RoadLine, distance_to_segment, point_in_area, ring_bounds,
};
use crate::settings::NAVTILE_SIZE;

/// Границы длины здания, м — главная ось когорт. Именно длина, а не площадь,
/// отвечает на вопрос «сколько подъездов»: внутри одной полосы площади длина
/// продолжает разделять (при 800–2500 м²: 1.90 входа на 40–70 м против 3.11 на
/// 120 м и длиннее). По остаточному разбросу длина одна не хуже площади одной
/// (1.073 против 1.078), а вместе с площадью и высотой дают лучший результат
/// из проверенных — 1.036.
const COHORT_LENGTH_BANDS: [f32; 5] = [20.0, 40.0, 70.0, 120.0, 200.0];
/// Высота, с которой здание считается многоэтажным, м. Значима только на
/// длинных: при 70–120 м замер расходится 1.86 (ниже) против 2.23 (выше), при
/// 120 м и длиннее — 2.64 против 3.07, то есть примерно ±10% от полосы. На
/// коротких разницы нет (1.27 против 1.22 — шум). Здание без высоты идёт по
/// низкой ветке.
const COHORT_TALL_HEIGHT: f32 = 12.0;
/// Площадь, ниже которой длинное здание всё равно остаётся мелким, м². Ряд
/// гаражей 100 × 4 м длинный, но подъездов у него нет: замер для полосы
/// 120–800 м² при длине 70–120 м даёт 1.46, а не 1.86–2.23 длинной когорты.
const COHORT_SMALL_AREA: f32 = 800.0;

/// Сколько входов давать зданию когорты: среднее замера и потолок по p90.
/// Нижняя граница везде 1 — p10 во всех когортах равен единице, дом без двери
/// не бывает.
struct EntranceCohort {
    mean: f32,
    max: usize,
}

/// «Длина» здания — длинная сторона прямоугольника с той же площадью и тем же
/// периметром. Считается из них, а не из AABB: не зависит от поворота дома
/// (диагональный корпус AABB раздувает вдвое) и честно ловит именно
/// вытянутость, а не размер.
fn equivalent_length(area: f32, perimeter: f32) -> f32 {
    // P = 2(L + W), A = LW  =>  L = (P + sqrt(P² − 16A)) / 4
    let discriminant = perimeter * perimeter - 16.0 * area;
    if discriminant <= 0.0 {
        // компактнее прямоугольника (круг, восьмиугольник) — вытянутости нет
        return perimeter / 4.0;
    }
    (perimeter + discriminant.sqrt()) / 4.0
}

/// Верхняя граница «ряда» — когорты, в которую понижается длинный, но мелкий
/// дом. Держится отдельной константой, потому что на неё ссылается и таблица
/// когорт, и ограничитель по площади.
const COHORT_ROW_MEAN: f32 = 2.0;

/// Когорта здания по длине, высоте и площади — таблица в `CONTEXT.md`.
///
/// Средние взяты по домам, где в OSM размечено **не меньше двух** дверей.
/// Считать по всем домам с хотя бы одной нельзя: маппер сплошь и рядом ставит
/// одну дверь и бросает, и такие дома роняют среднее до абсурда — по всей
/// выборке выходило 94 м длины на дверь при 120–200 м и 133 м при 200 м и
/// длиннее, то есть дверь раз в сотню метров. Порог в две двери убирает именно
/// брошенную разметку; он завышает средние коротких когорт (там дверь
/// действительно одна), поэтому до 40 м оставлены значения по полной выборке —
/// у сарая и дома разметка и так полная.
fn cohort_of(area: f32, length: f32, height: Option<f32>) -> EntranceCohort {
    let tall = height.is_some_and(|meters| meters >= COHORT_TALL_HEIGHT);
    let [hut, house, row, block, slab] = COHORT_LENGTH_BANDS;

    let cohort = match length {
        // сарай, гараж, киоск — дверь одна, и это не артефакт разметки
        _ if length < hut => EntranceCohort { mean: 1.2, max: 2 },
        // дом, секция таунхауса, магазин
        _ if length < house => EntranceCohort { mean: 1.35, max: 3 },
        // ряд: длинный магазин, школьное крыло (замер 2.70)
        _ if length < row => EntranceCohort {
            mean: COHORT_ROW_MEAN,
            max: 4,
        },
        // корпус (замер 3.35, высота даёт ±10%)
        _ if length < block && !tall => EntranceCohort { mean: 3.0, max: 6 },
        _ if length < block => EntranceCohort { mean: 3.7, max: 6 },
        // «дом-корабль» (замер 4.18)
        _ if length < slab && !tall => EntranceCohort { mean: 3.8, max: 8 },
        _ if length < slab => EntranceCohort { mean: 4.6, max: 8 },
        // квартал целиком: длиннее 200 м (замер 4.42)
        _ if !tall => EntranceCohort { mean: 4.0, max: 8 },
        _ => EntranceCohort { mean: 4.9, max: 8 },
    };

    // длинный, но мелкий — это ряд гаражей, а не жилой корпус
    if area < COHORT_SMALL_AREA && cohort.mean > COHORT_ROW_MEAN {
        return EntranceCohort {
            mean: COHORT_ROW_MEAN,
            max: 4,
        };
    }
    cohort
}

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
/// Сторона ячейки индекса дорог, м.
const ROAD_CELL: f32 = 60.0;
/// Сторона ячейки индекса контуров, м. Мельче дорожной: здание — пятно в
/// десятки метров, и в ячейку должно попадать несколько кандидатов, а не
/// полквартала.
const FOOTPRINT_CELL: f32 = 30.0;
/// Насколько колец ячеек вокруг точки просматривает индекс, прежде чем
/// сдаться. 4 кольца — 240 м; дальше от любой дороги в городе не бывает, а
/// здание в глубине квартала всё равно получит дверь по лучшей из граней.
const ROAD_SEARCH_RINGS: i32 = 4;

/// Равномерная сетка отрезков дорог. Генератору нужен ближайший участок улицы
/// для каждой грани каждого контура — в Париже это 27 000 зданий против
/// 20 000 дорог, и линейный проход тут не проходит по времени.
struct RoadIndex {
    cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
}

impl RoadIndex {
    fn build(roads: &[RoadLine]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>> =
            std::collections::HashMap::new();
        for road in roads {
            for segment in road.points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                let min = from.min(to);
                let max = from.max(to);
                // отрезок кладётся во все ячейки, которые пересекает его AABB
                for x in cell(min.x, ROAD_CELL)..=cell(max.x, ROAD_CELL) {
                    for y in cell(min.y, ROAD_CELL)..=cell(max.y, ROAD_CELL) {
                        cells.entry((x, y)).or_default().push((from, to));
                    }
                }
            }
        }
        Self { cells }
    }

    /// Ближайшая точка дорожной сети и расстояние до неё. Кольца
    /// просматриваются от центра наружу; поиск останавливается, как только
    /// найденное расстояние заведомо меньше, чем всё, что может лежать в
    /// следующем кольце.
    fn nearest(&self, point: Vec2) -> Option<(Vec2, f32)> {
        let (cx, cy) = (cell(point.x, ROAD_CELL), cell(point.y, ROAD_CELL));
        let mut best: Option<(Vec2, f32)> = None;

        for ring in 0..=ROAD_SEARCH_RINGS {
            for x in cx - ring..=cx + ring {
                for y in cy - ring..=cy + ring {
                    // только периметр кольца — внутренние ячейки уже пройдены
                    if (x - cx).abs() != ring && (y - cy).abs() != ring {
                        continue;
                    }
                    for &(from, to) in self.cells.get(&(x, y)).into_iter().flatten() {
                        let distance = distance_to_segment(point, from, to);
                        if best.is_none_or(|(_, best_distance)| distance < best_distance) {
                            best = Some((closest_on_segment(point, from, to), distance));
                        }
                    }
                }
            }
            // всё, что осталось снаружи, дальше этого рубежа
            if best.is_some_and(|(_, distance)| distance <= ring as f32 * ROAD_CELL) {
                break;
            }
        }
        best
    }
}

fn cell(value: f32, size: f32) -> i32 {
    (value / size).floor() as i32
}

/// Равномерная сетка контуров зданий. Нужна, чтобы ответить на вопрос «есть ли
/// перед этой стеной свободное место»: в плотной застройке дома в OSM стоят
/// вплотную и даже перекрываются, и дверь, поставленная на общую стену,
/// оказывается внутри соседа — снаружи её не видно, а изнутри к ней не пройти.
struct FootprintIndex<'a> {
    cells: std::collections::HashMap<(i32, i32), Vec<usize>>,
    buildings: &'a [PolyArea],
}

impl<'a> FootprintIndex<'a> {
    fn build(buildings: &'a [PolyArea]) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> =
            std::collections::HashMap::new();
        for (index, building) in buildings.iter().enumerate() {
            // загораживает дверь только дом; вода и парк — не преграда
            if building.kind != AreaKind::Building || building.outer.len() < 3 {
                continue;
            }
            let (min, max) = ring_bounds(&building.outer);
            for x in cell(min.x, FOOTPRINT_CELL)..=cell(max.x, FOOTPRINT_CELL) {
                for y in cell(min.y, FOOTPRINT_CELL)..=cell(max.y, FOOTPRINT_CELL) {
                    cells.entry((x, y)).or_default().push(index);
                }
            }
        }
        Self { cells, buildings }
    }

    /// Точка занята чужим домом? Свой дом (`owner`) не в счёт — дверь стоит на
    /// его собственной стене.
    fn is_covered(&self, point: Vec2, owner: usize) -> bool {
        let key = (cell(point.x, FOOTPRINT_CELL), cell(point.y, FOOTPRINT_CELL));
        self.cells
            .get(&key)
            .into_iter()
            .flatten()
            .any(|&index| index != owner && point_in_area(point, &self.buildings[index]))
    }
}

/// Ближайшая точка отрезка — как `distance_to_segment`, но нужна сама точка.
fn closest_on_segment(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return from;
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    from + segment * t
}

/// Обход контура против часовой стрелки? От этого зависит, в какую сторону
/// смотрит внешняя нормаль грани. Формула та же, что в `ring_area`, но со
/// знаком.
fn ring_is_ccw(ring: &[Vec2]) -> bool {
    let mut doubled = 0.0;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        doubled += (ring[j].x - ring[i].x) * (ring[j].y + ring[i].y);
        j = i;
    }
    doubled > 0.0
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

/// Число входов: **закон шага** — дверь на каждые [`ENTRANCE_SPACING`] длины,
/// но не выше потолка когорты; на коротких домах, где закон шага даёт ноль,
/// работает среднее когорты.
///
/// Закон шага главнее таблицы средних, потому что подтверждён двумя
/// независимыми замерами, а таблица средних им противоречила. Первый: медиана
/// зазора между соседними дверями — 26.7 м по пяти городам, 22.6 м в Туле.
/// Второй: у тульских домов с размеченными подъездами (`entrance=staircase` —
/// единственная выборка, где двери перечисляют исчерпывающе по соглашению)
/// метров длины на подъезд держится 21.8–27.4 **во всех полосах длины**, от
/// сорокаметрового дома до двухсотметрового. Это и есть закон: подъезд каждые
/// 25 м. Средние же когорт давали для 200-метрового дома 4 двери, то есть
/// дверь раз в 60 м, — с обоими замерами это несовместимо.
///
/// Потолок когорты оставлен: двухсотметровый завод — не жилой корабль, и
/// восемь дверей ему ни к чему.
fn entrance_count(cohort: &EntranceCohort, length: f32, random: &mut impl FnMut() -> f32) -> usize {
    let by_pitch = (length / ENTRANCE_SPACING).floor() as usize;

    // дробная часть среднего разыгрывается — так когорта воспроизводит замер
    // ровно, а не «как повезёт с округлением»
    let whole = cohort.mean.floor();
    let extra = if random() < cohort.mean - whole { 1 } else { 0 };
    let by_cohort = whole as usize + extra;

    by_pitch.max(by_cohort).clamp(1, cohort.max)
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
