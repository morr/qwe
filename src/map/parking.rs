//! Стоянки: асфальт с расчерченными местами и машинами на них.
//!
//! На снимке города двор со стоянкой ни с чем не спутать — прямоугольник
//! асфальта, расчерченный белыми полосками, и на половине мест что-то стоит.
//! До сих пор `amenity=parking` не запрашивался вовсе, и все эти площадки
//! были просто землёй.
//!
//! Раскладка держится одного правила: **к каждому месту машина должна
//! доехать**. Откуда она едет, у большой стоянки в OSM нарисовано —
//! `service=parking_aisle`, проезды между рядами ([`RoadLine::parking_aisle`]).
//! Тогда места раскладываются **по карманам между соседними проездами**
//! ([`aisle_rows`]): в кармане пара рядов спинами по его середине, носами
//! наружу, остаток кармана обеим сторонам под проезд — и глубина места
//! подгоняется так, чтобы проезду осталось не меньше [`PAIR_AISLE`]. Сетка
//! проездов при этом **продолжается своим шагом до краёв контура**: в OSM они
//! обрываются, не доходя до края, и без этого вдоль одних сторон площадки
//! оставалась бы широкая полоса голого асфальта, а вдоль других мест не было
//! бы вовсе. Своего поперечного проезда такой раскладке не нужно: проезды в
//! OSM уже нарезаны кварталами, а концы рядов упираются в контур.
//!
//! Проездов в OSM нет у большинства дворовых площадок — им раскладка
//! **выдумывается** ([`generated_rows`]): ряды вдоль **самой длинной стороны
//! контура** (не вдоль оси описанного прямоугольника: у стоянки, дотянутой до
//! дороги, эта ось уезжает на десяток градусов от той стороны, по которой
//! стоянка читается). Поперёк это `ряд — проезд — пара рядов спинами — проезд
//! — пара рядов` ([`row_bands`]): пристенный ряд выезжает в свой проезд,
//! каждая пара — в проезды по обе стороны от себя. Вдоль — что ряд не тянется
//! через всю площадку: каждые [`ROW_BLOCK`] метров его рвёт поперечный проезд
//! ([`row_places`]), и разрывы всех рядов стоят на одной линии, так что
//! площадка читается кварталами мест с проездами между ними, а не сплошной
//! штриховкой.
//!
//! Каждое место проверяется на попадание в контур, поэтому Г-образная стоянка
//! не получает мест поверх газона, а площадка, в которую не встаёт ни одно
//! место, — вовсе никаких. Маленький двор места получает, но не разметку:
//! см. [`MIN_AREA`].
//!
//! Разметка и машины делят **один** список мест: раскладка считается один раз
//! на загрузку мира в [`ParkingLayout`] (`map::spawn::spawn_map`), а краска
//! здесь и машины (`map::cars::fill_lots`) её только читают. Две независимые
//! раскладки поставили бы машину мимо её полосы.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::map::osm::PolyArea;
use crate::map::osm::model::{RoadLine, point_in_area, ring_bounds, signed_ring_area};

/// Место, м: легковая машина плюс просвет по обе стороны.
const STALL_WIDTH: f32 = 2.6;
const STALL_DEPTH: f32 = 5.2;
/// До скольких метров место ужимается ради проезда. Ниже — легковая машина
/// начинает торчать в проезд, и выигрыш оборачивается тем же затором.
const STALL_DEPTH_MIN: f32 = 4.8;
/// Сколько метров обязано остаться на проезд между парой рядов. Меньше
/// [`AISLE`]: на 14.8 м между проездами (больничная стоянка) полного проезда
/// и пары рядов не бывает вместе, а выбор между «ряд, ряд и 5.2 м» и «один
/// ряд и 9.6 м» решается в пользу первого — так эти площадки и размечены.
const PAIR_AISLE: f32 = 5.0;
/// Проезд между рядами, м, — и продольный, и поперечный: это одна и та же
/// полоса асфальта, по которой машина подъезжает к месту.
const AISLE: f32 = 6.0;
/// Длина ряда между поперечными проездами, м. Ряд длиннее читается сплошной
/// штриховкой: у большой стоянки (Тула, ТРЦ «Макси», 671 × 255 м) ряд без
/// разрывов тянулся на сотни метров, тогда как на снимке такая площадка
/// разбита проездами на кварталы мест. Пятьдесят метров — это 19 мест подряд,
/// обычный квартал между проездами.
const ROW_BLOCK: f32 = 50.0;
/// Насколько звено проезда может отклониться от главного направления стоянки
/// и всё ещё считаться рядом, °. Тридцать — это заведомо больше разнобоя
/// самих проездов (у ТРЦ «Макси» 102–105°) и заведомо меньше отворота к
/// соседнему проезду и поперечного проезда (там 59–61°, то есть 40° в сторону).
const AISLE_SPREAD: f32 = 30.0;
/// Насколько далеко конец одного звена может отстоять от начала следующего,
/// чтобы они всё ещё считались одним ходом проезда, м. Звенья приходят из
/// одной ломаной и стыкуются точно; допуск — на счёт в `f32` по мировым
/// координатам в тысячи метров.
const JOIN_SLACK: f32 = 0.01;
/// Два проезда ближе этого друг к другу — один и тот же, м.
const LANE_MERGE: f32 = STALL_DEPTH;
/// Сколько мест подряд обязано быть в куске ряда, чтобы его размечали. У
/// скошенной кромки площадки ряд обрывается, и остаётся одно место: на
/// картинке это полоска на пустом асфальте, к которой никто не подъедет.
const MIN_ROW_RUN: usize = 2;
/// Самая узкая полоса, по которой машина всё-таки протиснется к месту, м.
/// Карман между проездами, где на пару рядов не хватило, размечается одним
/// рядом, только если по обе стороны от него остаётся хотя бы столько.
const MIN_AISLE: f32 = 3.0;
/// Клетка сетки пересечений, м, — шире диагонали места (5.8 м), чтобы сосед
/// наверняка лежал в 3 × 3 клетках вокруг.
const OVERLAP_CELL: f32 = STALL_WIDTH + STALL_DEPTH;
/// Насколько местам позволено налезть друг на друга, м: соседи по ряду стоят
/// вплотную, и проверка пересечения обязана считать это «не пересекаются».
const OVERLAP_SLACK: f32 = 0.05;
/// Отступ разметки от края площадки, м.
const EDGE_MARGIN: f32 = 1.2;
/// Ширина полосы разметки, м, и её цвет — та же белая краска, что на улице.
const LINE_WIDTH: f32 = 0.12;
/// Насколько полоса не доходит до спины места, м. Ряды пары стоят спинами
/// вплотную, и без этого зазора их полосы сливаются в одну черту во всю
/// глубину обоих мест.
const LINE_GAP: f32 = 0.5;
const LINE_COLOR: Color = Color::srgb(0.82, 0.82, 0.80);
/// Стоянка мельче этого пятна не получает **разметки**: две машины во дворе
/// никто не расчерчивает. Места на ней остаются — машины на них стоят
/// (`map::cars::fill_lots`), просто по неразмеченному асфальту.
const MIN_AREA: f32 = 120.0;

/// Одно место: центр, направление, в котором машина стоит, и его глубина.
///
/// Глубина — поле, а не константа, потому что её задаёт карман между
/// проездами: чтобы на проезд осталось не меньше [`PAIR_AISLE`], паре рядов в
/// тесном кармане приходится ужаться до [`STALL_DEPTH_MIN`]. Длиннее всех в
/// слое машин «Газель» (5.3 м) — она торчит из ужатого места, легковые (3.9 —
/// 4.6 м) помещаются.
#[derive(Debug)]
pub struct Stall {
    pub at: Vec2,
    pub along: Vec2,
    pub depth: f32,
}

/// Раскладка всех стоянок карты — по списку мест на контур `MapData::parking`,
/// в том же порядке.
///
/// Кеш здесь окупается там, где у разрывов разметки (`roads::junctions`) не
/// окупился: вход раскладки — только контуры стоянок, а они меняются лишь со
/// сменой мира, тогда как слой машин пересобирается на каждое деление ползунка
/// солнца, зума и стиля дорог. Считать одно и то же по кадру — ровно та работа,
/// которой быть не должно; плата — резидентные 16 байт на место (Тула ~0.17 МБ).
#[derive(Resource, Default)]
pub struct ParkingLayout(pub Vec<Vec<Stall>>);

impl ParkingLayout {
    /// Места всех стоянок. Из дорог читается **единственное** — проезды стоянок
    /// (`service=parking_aisle`): по ним ряды и разворачиваются. Остальная сеть
    /// раскладке по-прежнему безразлична, и это не упущение — стоянка лежит
    /// поверх дорог (`Z_PARKING`) и кроет любую ленту, что заходит на неё или
    /// идёт сквозь неё: асфальт стоянки и есть проезд.
    pub fn new(lots: &[PolyArea], roads: &[RoadLine]) -> Self {
        let aisles: Vec<&RoadLine> = roads.iter().filter(|road| road.parking_aisle).collect();
        Self(lots.iter().map(|lot| stalls(lot, &aisles)).collect())
    }
}

/// Места стоянки: по проездам из OSM, а если их на этой площадке нет — по её
/// самой длинной стороне. Пусто, если площадка мелкая или вырожденная.
pub fn stalls(area: &PolyArea, aisles: &[&RoadLine]) -> Vec<Stall> {
    let rows = aisle_rows(area, aisles);
    if rows.is_empty() {
        generated_rows(area)
    } else {
        rows
    }
}

/// Ряды по проездам стоянки. Пусто, если ни один проезд сюда не заходит.
///
/// Считается **не от осевой проезда, а по карману между соседними проездами**,
/// и это главное в раскладке. Отступ от осевой был фиксированным — половина
/// [`AISLE`] плюс место, — и такой паре рядов нужно [`STALL_DEPTH`] · 2 +
/// `AISLE` = 16.4 м между проездами. У ТРЦ «Макси» их 16–19, и пары вставали;
/// у больничной стоянки (way 344589378) — 14.8, второй ряд каждый раз
/// оказывался наезжающим и снимался, и площадка размечалась по одному ряду на
/// проезд. Так не паркуют: один ряд бывает только с краю. Карман же делится по
/// факту — два ряда спинами по его середине, остаток обеим сторонам под
/// проезд, — и 14.8 м читаются как ряд, ряд и 4.4 м проезда между ними.
fn aisle_rows(area: &PolyArea, aisles: &[&RoadLine]) -> Vec<Stall> {
    let (low, high) = ring_bounds(&area.outer);
    let mut segments: Vec<(Vec2, Vec2)> = Vec::new();
    for aisle in aisles {
        let (aisle_low, aisle_high) = ring_bounds(&aisle.points);
        if aisle_low.x > high.x
            || aisle_high.x < low.x
            || aisle_low.y > high.y
            || aisle_high.y < low.y
        {
            continue;
        }
        for pair in aisle.points.windows(2) {
            // проезд может выходить за контур — на площадке он тем куском, что
            // внутри; остальное отсеет `fits`
            if point_in_area(pair[0].midpoint(pair[1]), area)
                || point_in_area(pair[0], area)
                || point_in_area(pair[1], area)
            {
                segments.push((pair[0], pair[1]));
            }
        }
    }
    let Some(main) = main_of(&segments) else {
        return Vec::new();
    };
    let across = Vec2::new(-main.y, main.x);
    let (low, high) = axis_bounds(&area.outer, main, across);
    let lanes = lanes_of(&rows_of(&segments, main), across, (low.y, high.y));
    if lanes.len() < 2 {
        return Vec::new();
    }

    // продольная сетка одна на всю площадку: тогда места соседних рядов стоят
    // в одну линию, как на снимке, а не вразнобой на полместа. Считается она
    // **от контура, а не от проездов**: проезд в OSM обрывается, не доходя до
    // края площадки, и ряд, обрезанный по нему, оставлял бы вдоль одной
    // стороны полосу голого асфальта, а вдоль другой ничего
    let frame = Frame {
        area,
        main,
        across,
        origin: low.x,
    };
    let span = (low.x, high.x);

    let mut placed = Placed::default();
    for pair in lanes.windows(2) {
        let gap = pair[1] - pair[0];
        let centre = pair[0].midpoint(pair[1]);
        // глубину места задаёт карман: сперва проезд, остальное ряду
        let depth = ((gap - AISLE) / 2.0).clamp(STALL_DEPTH_MIN, STALL_DEPTH);
        if gap >= 2.0 * depth + PAIR_AISLE {
            frame.push_row(&mut placed, centre - depth / 2.0, -1.0, depth, span);
            frame.push_row(&mut placed, centre + depth / 2.0, 1.0, depth, span);
        } else if gap >= STALL_DEPTH + MIN_AISLE {
            // на пару не хватило — ряд по середине кармана, проезд по обе
            // стороны от него
            frame.push_row(&mut placed, centre, -1.0, STALL_DEPTH, span);
        }
    }
    placed.stalls
}

/// Проезды как смещения поперёк площадки, по порядку и **продолженные до её
/// краёв**.
///
/// Два правила, и оба про одно — площадка размечается целиком.
///
/// Полосы ближе [`LANE_MERGE`] сливаются: длинный ход в OSM сплошь и рядом
/// разрезан на два way с небольшим изломом (у ТРЦ «Макси» так лежат все сорок
/// четыре проезда — парами в полуметре друг от друга), и считать их двумя
/// полосами значило бы получить между ними карман в полметра.
///
/// Затем сетка продолжается своим же шагом за крайние проезды, пока не выйдет
/// за габарит контура. Проезды в OSM обрываются, не доходя до края площадки, и
/// без этого у больничной стоянки вдоль двух сторон оставалась широкая полоса
/// голого асфальта, а вдоль двух других мест не было вовсе — поля выходили
/// разными, чего у настоящей стоянки не бывает.
fn lanes_of(rows: &[(Vec2, Vec2)], across: Vec2, bounds: (f32, f32)) -> Vec<f32> {
    let mut lanes: Vec<f32> = rows
        .iter()
        .map(|(from, to)| across.dot(*from).midpoint(across.dot(*to)))
        .collect();
    lanes.sort_by(f32::total_cmp);

    let mut merged: Vec<f32> = Vec::new();
    for lane in lanes {
        match merged.last() {
            Some(last) if lane - last < LANE_MERGE => {}
            _ => merged.push(lane),
        }
    }
    let (Some(first), Some(last)) = (merged.first().copied(), merged.last().copied()) else {
        return merged;
    };
    // шаг сетки — по самим проездам, а одиночному брать неоткуда: тогда это
    // пара рядов спинами и проезд, то есть как размечают
    let step = if merged.len() > 1 {
        (last - first) / (merged.len() - 1) as f32
    } else {
        2.0 * STALL_DEPTH + AISLE
    };
    let mut lane = first - step;
    while lane + step / 2.0 >= bounds.0 {
        merged.insert(0, lane);
        lane -= step;
    }
    let mut lane = last + step;
    while lane - step / 2.0 <= bounds.1 {
        merged.push(lane);
        lane += step;
    }
    merged
}

/// Оси площадки и общая продольная сетка — всё, что нужно, чтобы поставить ряд
/// на заданной глубине.
struct Frame<'a> {
    area: &'a PolyArea,
    main: Vec2,
    across: Vec2,
    origin: f32,
}

impl Frame<'_> {
    /// Ряд, середина которого стоит на `band` поперёк площадки, носом в `nose`
    /// (±1 вдоль `across`), местами глубиной `depth`, от `span.0` до `span.1`
    /// вдоль площадки.
    fn push_row(&self, placed: &mut Placed, band: f32, nose: f32, depth: f32, span: (f32, f32)) {
        let along = self.across * nose;
        let mut row = Vec::new();
        // кусок ряда — места подряд по сетке; обрыв считается здесь, потому
        // что **обрывок короче [`MIN_ROW_RUN`] не размечается вовсе**: у
        // скошенной кромки в ряду остаётся одно место, и на картинке это
        // полоска на пустом асфальте, куда никто не встанет
        let mut run: Vec<Stall> = Vec::new();
        let mut index = ((span.0 - self.origin) / STALL_WIDTH).ceil().max(0.0);
        loop {
            let place = self.origin + index * STALL_WIDTH + STALL_WIDTH / 2.0;
            if place + STALL_WIDTH / 2.0 > span.1 + 0.01 {
                break;
            }
            let at = self.main * place + self.across * band;
            if fits_with(self.area, at, self.main, self.across, depth, EDGE_MARGIN)
                && reachable(self.area, at, along, depth)
            {
                run.push(Stall { at, along, depth });
            } else {
                if run.len() >= MIN_ROW_RUN {
                    row.append(&mut run);
                }
                run.clear();
            }
            index += 1.0;
        }
        if run.len() >= MIN_ROW_RUN {
            row.append(&mut run);
        }
        // порядок мест в списке — тот, в котором их ждёт разметка
        // (`push_markings`): вдоль `-perp(Stall::along)`
        if nose < 0.0 {
            row.reverse();
        }
        for stall in row {
            placed.push(stall);
        }
    }
}

/// Отрезки, вдоль которых и правда стоят ряды: подряд идущие звенья проезда,
/// держащиеся главного направления, склеены в один, всё прочее выброшено.
///
/// Проезд в OSM — ломаная, и её звенья не равноправны. У больничной стоянки
/// (way 344589378) каждый проезд начинается коротким отворотом к соседнему,
/// под девяносто градусов к собственному ряду: ряд по такому отвороту ложится
/// поперёк главных, [`Placed`] рубит его в лапшу, и на снимке это полосы
/// вразнобой. Отворот — дорога **к** ряду, а не ряд, и рядов не получает.
///
/// Склейка — вторая половина того же: у того же проезда главный ход разбит на
/// два звена с изломом в полтора градуса, и по звену на ряд дало бы два
/// куска мест со стыком посередине вместо одного ряда. Изломы внутри
/// [`AISLE_SPREAD`] спрямляются: на длиннейшем таком ходу (65 м, три звена)
/// промежуточная вершина отстоит от хорды на 14 см.
fn rows_of(segments: &[(Vec2, Vec2)], main: Vec2) -> Vec<(Vec2, Vec2)> {
    let spread = AISLE_SPREAD.to_radians().cos();
    let mut rows = Vec::new();
    let mut run: Option<(Vec2, Vec2)> = None;
    for (from, to) in segments {
        let straight = (*to - *from)
            .try_normalize()
            .is_some_and(|step| step.dot(main).abs() >= spread);
        // звенья одного проезда идут подряд, и цепочка рвётся, как только
        // следующее начинается не там, где кончилось прошлое
        let joined = run.is_some_and(|(_, end)| end.distance(*from) < JOIN_SLACK);
        run = match (straight, joined, run) {
            (true, true, Some((start, _))) => Some((start, *to)),
            (true, _, done) => {
                rows.extend(done);
                Some((*from, *to))
            }
            (false, _, done) => {
                rows.extend(done);
                None
            }
        };
    }
    rows.extend(run);
    rows
}

/// Главное направление площадки — в два прохода: грубая оценка по всем
/// звеньям, затем по одним лишь тем, что её держатся.
///
/// Второй проход не роскошь: отворот к соседнему проезду входит в первую
/// оценку наравне с рядом, и тридцать метров отворота против восьмидесяти
/// метров ряда уводят её на девять градусов — ряды встают косо к площадке.
fn main_of(segments: &[(Vec2, Vec2)]) -> Option<Vec2> {
    let rough = main_direction(segments)?;
    let spread = AISLE_SPREAD.to_radians().cos();
    let straight: Vec<(Vec2, Vec2)> = segments
        .iter()
        .copied()
        .filter(|(from, to)| {
            (*to - *from)
                .try_normalize()
                .is_some_and(|step| step.dot(rough).abs() >= spread)
        })
        .collect();
    main_direction(&straight).or(Some(rough))
}

/// Главное направление проездов площадки — сумма их направлений, взвешенная
/// длиной. Углы удваиваются, потому что ряд и его разворот — одно и то же
/// направление: без этого встречные звенья одного проезда гасили бы друг друга.
fn main_direction(segments: &[(Vec2, Vec2)]) -> Option<Vec2> {
    let mut doubled = Vec2::ZERO;
    for (from, to) in segments {
        let step = *to - *from;
        let Some(dir) = step.try_normalize() else {
            continue;
        };
        doubled += Vec2::new(dir.x * dir.x - dir.y * dir.y, 2.0 * dir.x * dir.y) * step.length();
    }
    Some(Vec2::from_angle(doubled.try_normalize()?.to_angle() / 2.0))
}

/// Выдуманная раскладка: ряды вдоль самой длинной стороны контура.
fn generated_rows(area: &PolyArea) -> Vec<Stall> {
    let Some(along) = longest_side(&area.outer) else {
        return Vec::new();
    };
    let across = Vec2::new(-along.y, along.x);
    let (low, high) = axis_bounds(&area.outer, along, across);
    let length = (high.x - low.x) - 2.0 * EDGE_MARGIN;
    let width = (high.y - low.y) - 2.0 * EDGE_MARGIN;
    if length < STALL_WIDTH || width < STALL_DEPTH {
        return Vec::new();
    }
    let origin = along * (low.x + EDGE_MARGIN) + across * (low.y + EDGE_MARGIN);

    let mut stalls = Vec::new();
    let places = row_places(length);
    for band in row_bands(width) {
        let middle = band + STALL_DEPTH / 2.0;
        for place in &places {
            let at = origin + along * *place + across * middle;
            // машина стоит поперёк ряда, носом в проезд
            if fits(area, at, along, across, STALL_DEPTH) {
                stalls.push(Stall {
                    at,
                    along: across,
                    depth: STALL_DEPTH,
                });
            }
        }
    }
    stalls
}

/// Направление самой длинной стороны контура. Именно стороны, а не оси
/// описанного прямоугольника: у площадки, дотянутой до дороги
/// (`osm::parse::pull_landuse_to_roads`), контур зубчатый, и минимальный
/// прямоугольник разворачивается по случайному зубцу.
fn longest_side(ring: &[Vec2]) -> Option<Vec2> {
    (0..ring.len())
        .map(|index| ring[(index + 1) % ring.len()] - ring[index])
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .and_then(|side| side.try_normalize())
}

/// Габариты кольца в осях `along`/`across`: (мин, макс) проекций.
fn axis_bounds(ring: &[Vec2], along: Vec2, across: Vec2) -> (Vec2, Vec2) {
    let mut low = Vec2::INFINITY;
    let mut high = Vec2::NEG_INFINITY;
    for point in ring {
        let projected = Vec2::new(point.dot(along), point.dot(across));
        low = low.min(projected);
        high = high.max(projected);
    }
    (low, high)
}

/// Уже поставленные места с сеткой по ним: место кладётся, только если не
/// наезжает на чужое.
///
/// Без этого поперечный проезд стелил бы свой ряд поперёк чужих мест: в OSM
/// проезды пересекаются, а у ТРЦ «Макси» полсотни проездов на одну площадку,
/// из них шесть — под другим углом.
#[derive(Default)]
struct Placed {
    stalls: Vec<Stall>,
    /// Клетка шире диагонали места, поэтому соседей хватает искать в 3 × 3.
    grid: HashMap<(i32, i32), Vec<usize>>,
}

impl Placed {
    fn push(&mut self, stall: Stall) {
        let cell = Self::cell(stall.at);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(near) = self.grid.get(&(cell.0 + dx, cell.1 + dy)) else {
                    continue;
                };
                if near
                    .iter()
                    .any(|index| overlaps(&self.stalls[*index], &stall))
                {
                    return;
                }
            }
        }
        self.grid.entry(cell).or_default().push(self.stalls.len());
        self.stalls.push(stall);
    }

    fn cell(at: Vec2) -> (i32, i32) {
        (
            (at.x / OVERLAP_CELL).floor() as i32,
            (at.y / OVERLAP_CELL).floor() as i32,
        )
    }
}

/// Пересекаются ли два места — по теореме о разделяющей оси, осей четыре.
/// Допуск в [`OVERLAP_SLACK`] обязателен: соседи по ряду стоят ровно вплотную,
/// и без него ошибка в последнем знаке снимала бы каждое второе место.
fn overlaps(a: &Stall, b: &Stall) -> bool {
    let axes = [
        a.along,
        Vec2::new(-a.along.y, a.along.x),
        b.along,
        Vec2::new(-b.along.y, b.along.x),
    ];
    axes.iter().all(|axis| {
        let reach = |stall: &Stall| {
            let across = Vec2::new(-stall.along.y, stall.along.x);
            (stall.along.dot(*axis) * stall.depth / 2.0).abs()
                + (across.dot(*axis) * STALL_WIDTH / 2.0).abs()
        };
        (b.at - a.at).dot(*axis).abs() < reach(a) + reach(b) - OVERLAP_SLACK
    })
}

/// Начала рядов поперёк площадки шириной `width` (уже без отступов от краёв):
/// `ряд — проезд — пара рядов — проезд — пара рядов`.
///
/// Пристенный ряд один, а не пара, и это и есть правило «к месту можно
/// подъехать». Парами с самого края (как было) первый ряд упирается спиной в
/// пару, а носом — в край площадки: заехать в него неоткуда. Сдвиг на один ряд
/// даёт каждому ряду проезд с одной из сторон: одиночному — тот, что идёт
/// следом, паре — проезды по обе стороны от неё.
///
/// Ряд кладётся, только если влезает целиком; хвост уже площадки ряда остаётся
/// просто асфальтом. **Вторым рядом пары площадка не кончается**: спина такого
/// ряда — в соседнем ряду, а перед носом метр-другой до края, и заехать в него
/// неоткуда, — поэтому он снимается, а его полоса остаётся частью проезда.
/// Одиночный ряд у края — другое дело: к нему подъезжают с улицы, и площадка
/// в одну полосу так и размечается.
fn row_bands(width: f32) -> Vec<f32> {
    let mut bands = Vec::new();
    let mut depth = 0.0;
    // первая группа у края — один ряд, дальше пары спинами
    let mut rows = 1;
    while depth + STALL_DEPTH <= width {
        for _ in 0..rows {
            if depth + STALL_DEPTH > width {
                break;
            }
            bands.push(depth);
            depth += STALL_DEPTH;
        }
        depth += AISLE;
        rows = 2;
    }
    let tail = bands.last().is_some_and(|band| {
        let before = bands.len() > 1 && band - bands[bands.len() - 2] - STALL_DEPTH < AISLE;
        before && width - band - STALL_DEPTH < AISLE
    });
    if tail {
        bands.pop();
    }
    bands
}

/// Центры мест вдоль ряда длиной `length` (уже без отступов от краёв):
/// [`ROW_BLOCK`] метров мест, поперечный проезд, снова места.
///
/// Считается один раз на площадку и одинаково для всех её рядов — разрывы
/// обязаны стоять на одной линии, иначе вместо проезда через всю стоянку
/// получится россыпь пустых мест.
fn row_places(length: f32) -> Vec<f32> {
    let mut places = Vec::new();
    let mut place = STALL_WIDTH / 2.0;
    // сколько метров ряда уже уложено от последнего поперечного проезда
    let mut block = 0.0;
    while place + STALL_WIDTH / 2.0 <= length {
        places.push(place);
        place += STALL_WIDTH;
        block += STALL_WIDTH;
        if block >= ROW_BLOCK {
            place += AISLE;
            block = 0.0;
        }
    }
    places
}

/// Есть ли перед носом места асфальт, с которого на него заезжают.
///
/// Влезать в контур мало: сетка полос продолжается за крайние проезды
/// ([`lanes_of`]), и у самой кромки ряд встаёт так, что его проезд остался
/// снаружи площадки. Места там размечались, хотя заехать на них неоткуда —
/// на снимке это одинокие полосы вдоль кромки, отчёт автора. Проба идёт по
/// обеим передним кромкам, а не по одной осевой: у скошенного угла площадки
/// середина ещё на асфальте, когда половина выезда уже за ним.
fn reachable(area: &PolyArea, at: Vec2, along: Vec2, depth: f32) -> bool {
    let across = Vec2::new(-along.y, along.x);
    let ahead = at + along * (depth / 2.0 + PAIR_AISLE / 2.0);
    [-1.0f32, 1.0]
        .iter()
        .all(|side| point_in_area(ahead + across * (side * STALL_WIDTH / 2.0), area))
}

/// Место целиком внутри контура — по четырём углам, как и коробки на кровле.
fn fits(area: &PolyArea, at: Vec2, along: Vec2, across: Vec2, depth: f32) -> bool {
    fits_with(area, at, along, across, depth, 0.0)
}

/// То же, но место раздуто на `margin` со всех сторон: так у площадки требуют
/// **запаса до кромки**, а не попадания впритык.
///
/// Кромка стоянки идёт наискось к рядам, и место, чей угол лежит ровно на ней,
/// на картинке читается половинкой: машины на нём не видно, а полосы по его
/// краям торчат из ряда в никуда — отчёт автора. Запас в [`EDGE_MARGIN`]
/// убирает ровно такой торец, оставляя ряд на метр короче.
fn fits_with(
    area: &PolyArea,
    at: Vec2,
    along: Vec2,
    across: Vec2,
    depth: f32,
    margin: f32,
) -> bool {
    let half_width = along * (STALL_WIDTH / 2.0 + margin);
    let half_depth = across * (depth / 2.0 + margin);
    [
        at - half_width - half_depth,
        at + half_width - half_depth,
        at + half_width + half_depth,
        at - half_width + half_depth,
    ]
    .iter()
    .all(|corner| point_in_area(*corner, area))
}

/// Разметка мест в меш: по полоске между соседними местами. Полоса, а не
/// прямоугольник места: расчерчивают именно границы.
///
/// Полоса кладётся слева от каждого места (у соседей они совпадают — это
/// дешевле, чем искать соседа) плюс **закрывающая** справа там, где кусок ряда
/// начинается: у поперечного проезда и у края контура ряд обязан быть
/// закрыт, иначе крайнее место квартала выглядит распахнутым в проезд.
///
/// «Сосед справа» — это требование к **порядку** списка: места ряда обязаны
/// идти в направлении `-perp(Stall::along)`. У выдуманной раскладки так выходит
/// само, у рядов по проезду ([`aisle_rows`]) дальняя от проезда сторона за этим
/// идёт задом наперёд — иначе полосы легли бы по две в одно место.
pub fn push_markings(builder: &mut MeshBuilder, area: &PolyArea, stalls: &[Stall]) {
    if signed_ring_area(&area.outer).abs() < MIN_AREA {
        return;
    }
    let color = LINE_COLOR.to_linear();
    // места одного ряда идут по порядку, так что сосед справа — предыдущее
    // место списка; у первого места куска его там нет
    let mut previous: Option<Vec2> = None;
    for stall in stalls {
        let along = stall.along;
        let across = Vec2::new(-along.y, along.x);
        let nose = along * (stall.depth / 2.0);
        // со стороны спины полоса не доходит до конца места: у пары рядов
        // спинами полосы встречаются там встык и читаются одной длинной
        // чертой через оба ряда, а не границей мест
        let tail = along * (stall.depth / 2.0 - LINE_GAP);
        let half_line = across * (LINE_WIDTH / 2.0);
        let mut bar = |edge: Vec2| {
            builder.push_quad(
                [
                    edge - tail - half_line,
                    edge + nose - half_line,
                    edge + nose + half_line,
                    edge - tail + half_line,
                ],
                color,
            );
        };
        bar(stall.at - across * (STALL_WIDTH / 2.0));
        let neighbour = stall.at + across * STALL_WIDTH;
        if previous.is_none_or(|at| at.distance(neighbour) > LINE_WIDTH) {
            bar(stall.at + across * (STALL_WIDTH / 2.0));
        }
        previous = Some(stall.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::{AreaKind, fixture};

    fn lot(outer: Vec<Vec2>) -> PolyArea {
        fixture::area(AreaKind::Parking, outer)
    }

    fn rect(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::ZERO,
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    #[test]
    fn a_lot_is_filled_with_rows_of_stalls() {
        let lot = lot(rect(20.0, 40.0));
        let stalls = stalls(&lot, &[]);
        // 40 × 20 держит ровно пару рядов спинами: на проезд и следующую пару
        // нужно 2 · 1.2 + 2 · 5.2 + 6.0 + 5.2 = 24 м поперёк
        assert!(stalls.len() > 20, "{}", stalls.len());
        for stall in &stalls {
            assert!(point_in_area(stall.at, &lot), "{:?}", stall.at);
        }
    }

    /// Поперёк: пристенный ряд, проезд, пара рядов спинами — и на 40 × 20 из
    /// теста выше пара не влезает целиком, поэтому площадка здесь шире.
    #[test]
    fn a_single_row_at_the_edge_then_pairs_behind_an_aisle() {
        let lot = lot(rect(30.0, 40.0));
        // полосы по глубине (проекция центра на `Stall::along`)
        let mut bands: Vec<f32> = Vec::new();
        for stall in stalls(&lot, &[]) {
            let depth = stall.at.dot(stall.along);
            if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
                bands.push(depth);
            }
        }
        bands.sort_by(f32::total_cmp);
        assert_eq!(bands.len(), 3, "{bands:?}");
        let gaps: Vec<f32> = bands.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!((gaps[0] - (STALL_DEPTH + AISLE)).abs() < 0.01, "{gaps:?}");
        assert!((gaps[1] - STALL_DEPTH).abs() < 0.01, "{gaps:?}");
    }

    /// Правило раскладки: у каждого ряда с одной из сторон есть проезд шириной
    /// [`AISLE`]. Исключение одно — площадка в один ряд: подъезжают к нему с
    /// улицы, своего проезда у такой полосы нет и быть не может.
    #[test]
    fn every_row_has_an_aisle_to_drive_in_from() {
        for width in [5.0, 6.0, 12.0, 17.0, 22.0, 28.0, 40.0, 61.5, 120.0] {
            let bands = row_bands(width);
            if bands.len() < 2 {
                continue;
            }
            for (index, band) in bands.iter().enumerate() {
                let before = index
                    .checked_sub(1)
                    .map_or(*band, |previous| band - bands[previous] - STALL_DEPTH);
                let after = bands
                    .get(index + 1)
                    .map_or(width - band - STALL_DEPTH, |next| next - band - STALL_DEPTH);
                assert!(
                    before >= AISLE - 0.01 || after >= AISLE - 0.01,
                    "width {width}, band {index} of {bands:?}: {before} / {after}"
                );
            }
        }
    }

    /// Вдоль ряда: [`ROW_BLOCK`] метров мест, поперечный проезд, снова места.
    #[test]
    fn a_long_row_is_broken_by_cross_aisles() {
        let places = row_places(200.0);
        let gaps: Vec<f32> = places
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .filter(|gap| *gap > STALL_WIDTH + 0.01)
            .collect();
        assert_eq!(gaps.len(), 3, "{places:?}");
        for gap in gaps {
            assert!((gap - (STALL_WIDTH + AISLE)).abs() < 0.01);
        }
        // а короткий ряд не рвётся вовсе
        let short = row_places(40.0);
        assert!(
            short
                .windows(2)
                .all(|pair| (pair[1] - pair[0] - STALL_WIDTH).abs() < 0.01),
            "{short:?}"
        );
    }

    /// Есть проезд — ряды ложатся по обе стороны от него, носом в проезд,
    /// и вдоль он не рвётся: OSM уже нарезал стоянку проездами.
    #[test]
    fn rows_lie_along_the_osm_aisle_on_both_sides() {
        // площадка 120 × 22 и проезд по её середине
        let lot = lot(rect(22.0, 120.0));
        let aisle = fixture::parking_aisle(vec![Vec2::new(2.0, 11.0), Vec2::new(118.0, 11.0)]);
        let stalls = stalls(&lot, &[&aisle]);
        assert!(stalls.len() > 80, "{}", stalls.len());
        for stall in &stalls {
            // место стоит поперёк проезда...
            assert!(stall.along.x.abs() < 0.01, "{:?}", stall.along);
            // ...и носом в него: центр по одну сторону, нос — по другую
            let side = (stall.at.y - 11.0).signum();
            assert!((stall.along.y + side).abs() < 0.01, "{stall:?}");
            let depth = (stall.at.y - 11.0).abs();
            assert!(
                (depth - (AISLE + STALL_DEPTH) / 2.0).abs() < 0.01,
                "{depth}"
            );
        }
        // вдоль ряда разрывов нет — места идут сплошняком по [`STALL_WIDTH`]
        let mut row: Vec<f32> = stalls
            .iter()
            .filter(|stall| stall.at.y > 11.0)
            .map(|stall| stall.at.x)
            .collect();
        row.sort_by(f32::total_cmp);
        for pair in row.windows(2) {
            assert!((pair[1] - pair[0] - STALL_WIDTH).abs() < 0.01, "{row:?}");
        }
    }

    /// Поперечный проезд рядов не получает: он ведёт **к** рядам, а не вдоль
    /// них, и его ряд лёг бы поперёк главных.
    #[test]
    fn a_cross_aisle_gets_no_rows_of_its_own() {
        let lot = lot(rect(60.0, 120.0));
        let main = fixture::parking_aisle(vec![Vec2::new(2.0, 30.0), Vec2::new(118.0, 30.0)]);
        let cross = fixture::parking_aisle(vec![Vec2::new(60.0, 2.0), Vec2::new(60.0, 58.0)]);
        let stalls = stalls(&lot, &[&main, &cross]);
        for stall in &stalls {
            assert!(stall.along.x.abs() < 0.01, "{stall:?}");
        }
        for (index, stall) in stalls.iter().enumerate() {
            for other in &stalls[index + 1..] {
                assert!(!overlaps(stall, other), "{stall:?} / {other:?}");
            }
        }
    }

    /// Колено проезда — отворот к соседнему проезду, а не ряд: у больничной
    /// стоянки (way 344589378) с него начинается каждый проезд.
    #[test]
    fn the_elbow_that_leads_to_an_aisle_gets_no_rows() {
        let lot = lot(rect(60.0, 120.0));
        let elbow = fixture::parking_aisle(vec![
            Vec2::new(20.0, 2.0),
            Vec2::new(30.0, 30.0),
            Vec2::new(110.0, 30.0),
        ]);
        for stall in &stalls(&lot, &[&elbow]) {
            assert!(stall.along.x.abs() < 0.01, "{stall:?}");
        }
    }

    /// Излом главного хода не рвёт ряд надвое: подряд идущие звенья проезда
    /// склеиваются в один отрезок.
    #[test]
    fn a_kink_in_the_aisle_does_not_break_the_row_in_two() {
        let lot = lot(rect(30.0, 120.0));
        let bent = fixture::parking_aisle(vec![
            Vec2::new(2.0, 14.0),
            Vec2::new(60.0, 15.0),
            Vec2::new(118.0, 16.0),
        ]);
        let stalls = stalls(&lot, &[&bent]);
        // ряд — места на одной глубине; проезд наклонный, так что по `y` их
        // не разобрать
        let mut rows: Vec<Vec<Vec2>> = Vec::new();
        for stall in &stalls {
            let band = stall.at.dot(stall.along);
            match rows
                .iter_mut()
                .find(|row| (row[0].dot(stall.along) - band).abs() < 0.01)
            {
                Some(row) => row.push(stall.at),
                None => rows.push(vec![stall.at]),
            }
        }
        let row = rows
            .iter_mut()
            .max_by_key(|row| row.len())
            .expect("мест нет вовсе");
        row.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert!(row.len() > 40, "{}", row.len());
        for pair in row.windows(2) {
            let step = pair[0].distance(pair[1]);
            assert!((step - STALL_WIDTH).abs() < 0.01, "{step} в {row:?}");
        }
    }

    /// Между двумя проездами — **пара рядов спинами**, а не по одному ряду на
    /// проезд: у больничной стоянки (way 344589378) проезды идут через 14.8 м,
    /// и на фиксированном отступе от осевой второй ряд каждый раз снимался.
    #[test]
    fn a_pocket_between_two_aisles_holds_a_pair_of_rows_back_to_back() {
        let lot = lot(rect(40.0, 100.0));
        let lanes = [
            fixture::parking_aisle(vec![Vec2::new(2.0, 12.0), Vec2::new(98.0, 12.0)]),
            fixture::parking_aisle(vec![Vec2::new(2.0, 26.8), Vec2::new(98.0, 26.8)]),
        ];
        let stalls = stalls(&lot, &[&lanes[0], &lanes[1]]);
        let mut bands: Vec<f32> = Vec::new();
        for stall in &stalls {
            let depth = stall.at.y;
            if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
                bands.push(depth);
            }
        }
        bands.sort_by(f32::total_cmp);
        // в кармане между двумя проездами — пара рядов спинами по его середине
        let pair: Vec<f32> = bands
            .iter()
            .copied()
            .filter(|band| (12.0..26.8).contains(band))
            .collect();
        assert_eq!(pair.len(), 2, "{bands:?}");
        assert!(
            (pair[0].midpoint(pair[1]) - 19.4).abs() < 0.01,
            "пара не по середине кармана: {bands:?}"
        );
        // места ужаты ради проезда: 14.8 = 4.8 + 4.8 + 5.2 проезда
        let depth = pair[1] - pair[0];
        assert!((depth - STALL_DEPTH_MIN).abs() < 0.01, "{depth}");
        assert!(14.8 - 2.0 * depth >= PAIR_AISLE - 0.01, "{depth}");
        // и каждое место носом в свой проезд
        let to_lane = |at: Vec2| (at.y - 12.0).abs().min((at.y - 26.8).abs());
        for stall in &stalls {
            if !(12.0..26.8).contains(&stall.at.y) {
                continue;
            }
            let nose = stall.at + stall.along * (stall.depth / 2.0);
            assert!(to_lane(nose) < to_lane(stall.at), "{stall:?}");
        }
    }

    /// Обрывок ряда в одно место не размечается: у скошенной кромки такое
    /// место читается полоской на пустом асфальте.
    #[test]
    fn a_run_of_one_stall_is_not_striped() {
        // треугольный клин: к острому углу ряды сходят на одно место
        let wedge = lot(vec![
            Vec2::ZERO,
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 40.0),
        ]);
        let aisle = fixture::parking_aisle(vec![Vec2::new(10.0, 12.0), Vec2::new(95.0, 12.0)]);
        let stalls = stalls(&wedge, &[&aisle]);
        assert!(!stalls.is_empty());
        let mut rows: Vec<(f32, usize)> = Vec::new();
        for stall in &stalls {
            let band = stall.at.dot(stall.along);
            match rows.iter_mut().find(|row| (row.0 - band).abs() < 0.01) {
                Some(row) => row.1 += 1,
                None => rows.push((band, 1)),
            }
        }
        for (band, count) in &rows {
            assert!(*count >= MIN_ROW_RUN, "обрывок в {count} место на {band}");
        }
    }

    /// Место у самой кромки не размечается, если заехать на него неоткуда:
    /// продолженная сетка полос ставит ряд так, что его проезд остаётся за
    /// пределами площадки.
    #[test]
    fn a_stall_with_no_asphalt_in_front_of_it_is_not_striped() {
        // площадка ровно в ряд мест плюс проезд с одной стороны
        let lot = lot(rect(14.0, 60.0));
        let aisle = fixture::parking_aisle(vec![Vec2::new(2.0, 11.4), Vec2::new(58.0, 11.4)]);
        for stall in &stalls(&lot, &[&aisle]) {
            let ahead = stall.at + stall.along * (stall.depth / 2.0 + PAIR_AISLE / 2.0);
            assert!(point_in_area(ahead, &lot), "заехать неоткуда: {stall:?}");
        }
    }

    /// Сетка проездов продолжается до краёв контура: в OSM проезд обрывается,
    /// не дойдя до края, и вдоль двух сторон площадки оставалась широкая
    /// полоса голого асфальта, а вдоль двух других мест не было вовсе.
    #[test]
    fn the_lane_grid_runs_out_to_the_edges_of_the_lot() {
        let lot = lot(rect(60.0, 100.0));
        // проезд в углу площадки, коротким куском
        let aisle = fixture::parking_aisle(vec![Vec2::new(4.0, 12.0), Vec2::new(40.0, 12.0)]);
        let stalls = stalls(&lot, &[&aisle]);
        let far = stalls
            .iter()
            .map(|stall| stall.at.y)
            .fold(f32::MIN, f32::max);
        assert!(far > 40.0, "раскладка не дошла до дальнего края: {far}");
        let end = stalls
            .iter()
            .map(|stall| stall.at.x)
            .fold(f32::MIN, f32::max);
        assert!(end > 90.0, "ряд оборван по длине проезда: {end}");
    }

    /// Ряды по обе стороны одного проезда стоят напротив друг друга: обе
    /// стороны считаются от начала отрезка.
    #[test]
    fn the_two_rows_of_an_aisle_line_up_with_each_other() {
        let lot = lot(rect(22.0, 100.7));
        // длина проезда нарочно не кратна месту
        let aisle = fixture::parking_aisle(vec![Vec2::new(2.3, 11.0), Vec2::new(98.4, 11.0)]);
        let stalls = stalls(&lot, &[&aisle]);
        let row = |above: bool| {
            let mut places: Vec<f32> = stalls
                .iter()
                .filter(|stall| (stall.at.y > 11.0) == above)
                .map(|stall| stall.at.x)
                .collect();
            places.sort_by(f32::total_cmp);
            places
        };
        let (near, far) = (row(false), row(true));
        assert_eq!(near.len(), far.len(), "{near:?} / {far:?}");
        for (left, right) in near.iter().zip(&far) {
            assert!((left - right).abs() < 0.01, "{left} / {right}");
        }
    }

    /// Нет проездов — раскладка выдумывается, и ряды идут вдоль самой длинной
    /// стороны контура, а не вдоль оси описанного прямоугольника.
    #[test]
    fn without_aisles_rows_follow_the_longest_side() {
        // трапеция: низ 80 м вдоль x, верх скошен — у минимального
        // прямоугольника ось уезжает от длинной стороны
        let lot = lot(vec![
            Vec2::ZERO,
            Vec2::new(80.0, 0.0),
            Vec2::new(66.0, 26.0),
            Vec2::new(6.0, 22.0),
        ]);
        let stalls = stalls(&lot, &[]);
        assert!(!stalls.is_empty());
        for stall in &stalls {
            // ряд вдоль нижней стороны — место стоит поперёк неё
            assert!(stall.along.x.abs() < 0.01, "{:?}", stall.along);
        }
    }

    #[test]
    fn a_yard_corner_gets_nothing() {
        // 4 × 6 — двор на пару машин, размечать нечего
        assert!(stalls(&lot(rect(4.0, 6.0)), &[]).is_empty());
    }

    #[test]
    fn stalls_stay_out_of_a_notch() {
        // Г-образная площадка: места не должны лечь в вырез
        let ell = lot(vec![
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 12.0),
            Vec2::new(18.0, 12.0),
            Vec2::new(18.0, 26.0),
            Vec2::new(0.0, 26.0),
        ]);
        for stall in stalls(&ell, &[]) {
            assert!(point_in_area(stall.at, &ell), "{:?}", stall.at);
        }
    }
}
