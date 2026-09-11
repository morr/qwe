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
//! Плюс два требования, которые замером не выводятся, а следуют из здравого
//! смысла и из того, что в OSM дома сплошь и рядом стоят вплотную: **дверь не
//! ставится в стену, к которой прижат сосед** ([`FootprintIndex`]), и **дверь
//! не ставится в арку** ([`PassageIndex`]) — проезд сквозь дом выедает кусок
//! стены на всю высоту, и подъезда в этом куске не бывает.
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

use self::cohorts::{cohort_of, entrance_count, equivalent_length, plan_sections};
use self::index::{FootprintIndex, PassageIndex, RoadIndex, ring_is_ccw};
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

/// Насколько дверь держится от края арки, м. Проезд сквозь дом
/// (`tunnel=building_passage`) выедает кусок стены целиком, и подъезда там не
/// бывает — ни в самом проёме, ни впритык к нему: полотно двери с откосами
/// это уже два метра, а вокруг проёма лежит заплата
/// (`buildings::arches::push_wall_with_openings`) — задетые не целиком клетки
/// помечены `Solid`, — так что глухая полоса без окон уходит от края проёма ещё
/// почти на панель. Три метра — примерно эта панель и есть.
///
/// Без правила дверь вставала посреди арки: `push_doors` полотна там не
/// рисует (проём во всю стену уже есть), а гизмо и пешка — по данным, так что
/// вход оставался, но на карте его было не видно, и пешка шла в дыру.
const ENTRANCE_ARCH_CLEARANCE: f32 = 3.0;

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
/// все**. Возвращает число дописанных точек входа.
///
/// Дописанная точка и придуманный подъезд — не одно и то же: дворовая створка
/// сквозного подъезда (`through_doors`) тоже входит в этот счёт, потому что
/// в `PolyArea::entrances` она отдельный элемент, но подъезда не прибавляет.
/// Поэтому близнецы считаются ещё и сами по себе и печатаются своей строкой —
/// иначе «N generated» в логе загрузки читалось бы как «придумано N подъездов».
///
/// Дом с размеченными дверями раньше пропускался целиком, и это было ошибкой
/// той же природы, что и порог «не меньше двух дверей» в замере когорт: маппер
/// сплошь и рядом ставит один вход и бросает. Замер от такой разметки
/// защищался, а генерация — нет, и лондонский квартал в четверть километра
/// оставался с единственной дверью. Настоящие двери по-прежнему **не двигаются**:
/// они идут первыми, а когорта лишь дописывает недостающее. Выбрасывается из них
/// ровно одна разновидность — дверь, пришедшаяся на арку: стены в проёме нет, и
/// полотна там не будет, как бы дверь ни была размечена ([`PassageIndex`]).
pub fn generate_entrances(map: &mut MapData) -> usize {
    let roads = RoadIndex::build(&map.roads);
    let footprints = FootprintIndex::build(&map.buildings);
    // арки: проезд сквозь дом — это дыра в стене, а не место для подъезда
    let passages = PassageIndex::build(&map.roads);

    // двери сначала считаются по неизменной карте — каждому зданию нужны
    // контуры соседей, — и только потом раскладываются по зданиям
    let mut filled: Vec<(usize, Vec<Vec2>)> = Vec::new();
    let mut walled_in = 0;
    let mut through = 0;
    for (index, building) in map.buildings.iter().enumerate() {
        // стены и башни кремля дверей не несут
        if building.kind != AreaKind::Building {
            continue;
        }
        let Some(doors) = fill_building(index, building, &roads, &footprints, &passages) else {
            continue;
        };
        walled_in += usize::from(doors.forced);
        through += doors.through;
        filled.push((index, doors.entrances));
    }

    if walled_in > 0 {
        // дом целиком накрыт соседями (в OSM это чаще всего корпус, обведённый
        // ещё раз общим контуром квартала) — дверь ему всё равно нужна
        eprintln!("osm parse: {walled_in} buildings have no free wall for a door");
    }

    let mut generated = 0;
    for (index, entrances) in filled {
        // размеченные в OSM двери входят в этот список первыми — придуманными
        // считаются только дописанные. Вычитание **насыщающее**: дверь,
        // выброшенную из арки, список не содержит, и у дома с двумя такими
        // дверями он короче исходного — это ноль придуманных, а не минус один
        generated += entrances
            .len()
            .saturating_sub(map.buildings[index].entrances.len());
        map.buildings[index].entrances = entrances;
    }

    if through > 0 {
        // дворовые створки входят в `generated` наравне с остальными — точка
        // входа это отдельная, — но подъездов на столько не прибавилось, и без
        // этой оговорки «N generated» читалось бы как «придумано N подъездов»
        eprintln!(
            "osm parse: {through} of them are the courtyard half of a through entrance, \
             not a new one"
        );
    }
    generated
}

/// Двери одного здания: их точки, сколько из них — дворовые створки сквозных
/// подъездов ([`through_doors`]), и признак того, что свободной стены у дома не
/// нашлось.
struct FilledBuilding {
    entrances: Vec<Vec2>,
    through: usize,
    forced: bool,
}

fn fill_building(
    index: usize,
    building: &PolyArea,
    roads: &RoadIndex,
    footprints: &FootprintIndex,
    passages: &PassageIndex,
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
    let sections = plan_sections(area, length, building.height, building.building_use);
    let wanted = entrance_count(&cohort, length, sections, &mut random);

    let mut facades = score_facades(ring, roads);
    // лучшая грань — первой; NaN сюда попасть не может, длина и расстояние
    // всегда конечны
    facades.sort_by(|a, b| a.score.total_cmp(&b.score));

    // размеченные двери — первыми и неподвижными; нормаль каждой берётся у
    // ближайшей к ней грани, чтобы и она могла пройти домом насквозь. Двигать
    // их нельзя, но пришедшуюся на арку приходится выбросить: стены там нет
    // вовсе, и в OSM узел `entrance=*` вполне может стоять на той же вершине
    // контура, где кончается проезд (`buildings::arches`)
    let mut doors: Vec<Door> = building
        .entrances
        .iter()
        .filter(|&&at| !passages.blocks(at))
        .map(|&at| Door {
            at,
            outward: facade_outward(ring, at),
        })
        .collect();
    let real = doors.len();
    place_along(
        &facades,
        wanted,
        Pass::Walls(index, footprints, passages),
        &mut doors,
    );

    if !doors.is_empty() {
        let mut entrances: Vec<Vec2> = doors.iter().map(|door| door.at).collect();
        // сквозной подъезд: у корпуса дверь выходит и во двор. Дом-свечка и
        // короткий дом сюда не попадают — см. [`THROUGH_MIN_LENGTH`]
        let mut through = 0;
        if through_entrances(building, length) {
            let twins = through_doors(ring, &doors, index, footprints, passages);
            through = twins.len();
            entrances.extend(twins);
        }
        return Some(FilledBuilding {
            entrances,
            through,
            forced: false,
        });
    }
    // ни одной свободной стены — или дом целиком из ступенек: без двери он
    // выпал бы из целей блуждания, так что ставим её на лучшую грань, не глядя
    // ни на соседей, ни на длину. Арку запасной проход всё же обходит:
    // перебрать есть что, а дверь в проёме не рисуется вовсе
    let mut forced = Vec::new();
    place_along(&facades, 1, Pass::LastResort(passages), &mut forced);
    if forced.is_empty() {
        // и только если свободной от арки точки не нашлось ни одной — любая:
        // дом, пробитый проездом насквозь и зажатый соседями, всё равно должен
        // остаться целью блуждания
        place_along(&facades, 1, Pass::Forced, &mut forced);
    }
    Some(FilledBuilding {
        entrances: forced.into_iter().map(|door| door.at).collect(),
        // единственная дверь на глухой стене насквозь не идёт
        through: 0,
        // дом с уцелевшей размеченной дверью сюда не доходит, так что счётчик
        // считает именно глухие дома, а не всякий бездверный случай — и дом,
        // у которого единственную размеченную дверь съела арка, он считает
        // глухим по праву: свободной стены у него и правда не нашлось
        forced: real == 0,
    })
}

/// Внешняя нормаль грани, идущей в направлении `along`: интерьер лежит слева
/// от направления обхода у CCW-кольца, значит наружу — направо.
fn outward_of(along: Vec2, ccw: bool) -> Vec2 {
    match ccw {
        true => Vec2::new(along.y, -along.x),
        false => Vec2::new(-along.y, along.x),
    }
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
            best = (distance, outward_of(along, ccw));
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
        let outward = outward_of(direction, ccw);

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

/// Короче этого грань — не фасад, а **ступенька контура**, м. Дом в OSM
/// обводят с уступами в два-четыре метра (выступ лестничной клетки, эркер,
/// стык секций), и такая грань нередко оказывается у самой дороги, то есть
/// первой по оценке. Подъезда на ней не бывает: полотно с откосами это уже два
/// метра, а сам подъезд — это кусок стены, а не торец уступа. Ровно так третий
/// подъезд тульской десятиэтажки и вставал на четырёхметровый огрызок, пока
/// длинное крыло оставалось без дверей.
const ENTRANCE_MIN_FACADE: f32 = 6.0;

/// Сколько дверей забирает себе одна грань. Сверху — [`ENTRANCE_SPACING`]:
/// длинный фасад не должен собрать все двери дома в кучу. Снизу — жёсткий
/// предел по [`ENTRANCE_MIN_SPACING`]: двери раскладываются равномерно с шагом
/// `length / (take + 1)`, поэтому `take` дверей помещаются только при
/// `length / (take + 1) >= ENTRANCE_MIN_SPACING`. Одну грань всегда берём хотя
/// бы под одну дверь — иначе у крошечного дома их не осталось бы вовсе.
///
/// `stubs` — брать ли в счёт грани короче [`ENTRANCE_MIN_FACADE`]. В обычном
/// проходе нет; в запасном, когда дому не досталось ни одной двери, да —
/// у киоска три на три метра других граней и не бывает.
fn facade_capacity(length: f32, stubs: bool) -> usize {
    if !stubs && length < ENTRANCE_MIN_FACADE {
        return 0;
    }
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
/// `placed` приходит **не пустым**, когда двери дому уже размечены в OSM:
/// `wanted` — это сколько дверей у дома должно быть всего, а не сколько
/// дописать, и размеченные занимают своё место в зазоре наравне с
/// придуманными.
fn place_along(facades: &[Facade], wanted: usize, pass: Pass<'_>, placed: &mut Vec<Door>) {
    let minimum_squared = ENTRANCE_MIN_SPACING * ENTRANCE_MIN_SPACING;
    let stubs = !matches!(pass, Pass::Walls(..));

    for facade in facades {
        if placed.len() >= wanted {
            break;
        }
        let Some(direction) = (facade.to - facade.from).try_normalize() else {
            continue;
        };

        let take = facade_capacity(facade.length, stubs).min(wanted - placed.len());
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
            let blocked = match pass {
                Pass::Walls(owner, footprints, passages) => {
                    footprints.is_covered(point + facade.outward * entrance_clearance(), owner)
                        || passages.blocks(point)
                }
                Pass::LastResort(passages) => passages.blocks(point),
                Pass::Forced => false,
            };
            if blocked {
                continue;
            }
            placed.push(Door {
                at: point,
                outward: facade.outward,
            });
        }
    }
}

/// Который это проход по граням. Их три, и это **два уровня отката**, а не
/// один: обычный смотрит на всё, запасной отпускает соседа и порог длины грани,
/// но арку держит, и только последний не запрещает ничего. Три признака —
/// сосед, арка, ступенька контура — поэтому и разошлись по вариантам.
#[derive(Clone, Copy)]
enum Pass<'a> {
    /// Обычный: свой номер, индекс контуров и индекс арок. Точка проверяется
    /// на [`entrance_clearance`] наружу, и место, накрытое чужим домом,
    /// пропускается: стена, к которой сосед стоит вплотную, — глухая, дверь на
    /// ней смотрит в чужой фасад. И на [`ENTRANCE_ARCH_CLEARANCE`] от проезда
    /// сквозь дом — там стены нет вовсе. Занятая грань просто не отдаёт
    /// дверей, и они достаются следующей по оценке. Ступенька контура двери не
    /// берёт — см. [`ENTRANCE_MIN_FACADE`].
    Walls(usize, &'a FootprintIndex<'a>, &'a PassageIndex),
    /// Запасной, для дома, у которого свободной стены не нашлось вовсе: ни
    /// оглядки на соседей, ни порога длины грани. Арку он всё же обходит —
    /// выбор здесь не двоичный: `facade_capacity(.., stubs = true)` даёт
    /// несколько точек на нескольких гранях, и пропустить среди них попавшие в
    /// проём ничего не стоит, а дверь в проёме не рисуется вовсе
    /// ([`ENTRANCE_ARCH_CLEARANCE`]).
    LastResort(&'a PassageIndex),
    /// Последний: не запрещено ничего. Сюда доходит дом, у которого в арке
    /// оказалась каждая точка каждой грани; без двери он выпал бы из целей
    /// блуждания, и это хуже двери в проёме.
    Forced,
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
/// Подъезд от неё один: когорты отвечают на вопрос «сколько подъездов», и
/// второй раз их здесь не спрашивают. А вот **точка входа — вторая, отдельная**,
/// и модель этих двух понятий не различает: дворовая створка ложится в
/// `PolyArea::entrances` самостоятельным элементом, и все, кто читает этот
/// список, читают его как список входов — пешка выбирает из него цель «в этот
/// дом» равновероятно (`human::building_target`), строка загрузки складывает
/// длины списков, гизмо `doors` рисует кружок на каждой точке, `push_doors`
/// вешает на стену свою створку. У длинного корпуса половина целей блуждания
/// поэтому во дворе — со двора в подъезд заходят не реже, чем с улицы, так что
/// цена принята сознательно. Развести створку и подъезд в самой модели (пара
/// `at` + `through` вместо точки) — отдельная работа, её здесь нет; пока
/// близнецы просто считаются своим слагаемым в [`generate_entrances`].
///
/// Луч идёт от двери внутрь дома по нормали её грани; годится первое
/// пересечение с контуром дальше [`THROUGH_DEPTH_RANGE`], и только если та
/// грань смотрит навстречу — у Г-образного корпуса луч упирается в собственное
/// крыло, и дверь там выходила бы в торец соседнего подъезда.
fn through_doors(
    ring: &[Vec2],
    doors: &[Door],
    index: usize,
    footprints: &FootprintIndex,
    passages: &PassageIndex,
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
        // дворовый конец арки — та же дыра в стене, что и уличный, и
        // проверять его надо отдельно: сюда луч приходит от двери, которая
        // сама через `place_along` уже прошла
        if passages.blocks(exit) {
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
            best = Some((t, from + direction * t, outward_of(along, ccw)));
        }
    }
    best.map(|(_, point, outward)| (point, outward))
}

#[cfg(test)]
mod tests;
