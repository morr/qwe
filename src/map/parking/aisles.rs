//! Раскладка по настоящим проездам стоянки: поля проездов и их кварталы,
//! сетка полос, продолженная до краёв контура, поперечные проезды и главное
//! направление площадки.

use bevy::prelude::*;

use super::{
    AISLE, AISLE_SPREAD, CROSS_DRIVE_SLACK, EDGE_MARGIN, JOIN_SLACK, LANE_MERGE, MIN_AISLE,
    MIN_ROW_RUN, Outline, PAIR_AISLE, Placed, SPAN_SLACK, STALL_DEPTH, STALL_DEPTH_MIN,
    STALL_WIDTH, Stall, Surroundings, axis_bounds, fits_with, reachable,
};
use crate::map::osm::model::{RoadLine, distance_to_segment, ring_bounds};

/// Ряды по проездам стоянки. Пусто, если ни один проезд сюда не заходит, полос
/// вышло меньше двух или ни одно место не прошло проверок, — во всех трёх
/// случаях раскладку выдумывает [`super::generated::generated_rows`].
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
pub(super) fn aisle_rows(
    outline: &Outline,
    aisles: &[&RoadLine],
    through: &Surroundings,
) -> Vec<Stall> {
    let area = outline.area;
    let (lot_low, lot_high) = ring_bounds(&area.outer);
    let mut segments: Vec<(Vec2, Vec2)> = Vec::new();
    for aisle in aisles {
        let (aisle_low, aisle_high) = ring_bounds(&aisle.points);
        if aisle_low.x > lot_high.x
            || aisle_high.x < lot_low.x
            || aisle_low.y > lot_high.y
            || aisle_high.y < lot_low.y
        {
            continue;
        }
        for pair in aisle.points.windows(2) {
            // проезд может выходить за контур — на площадке он тем куском, что
            // внутри; остальное отсеет `fits_with` при раскладке
            if outline.contains(pair[0].midpoint(pair[1]))
                || outline.contains(pair[0])
                || outline.contains(pair[1])
            {
                segments.push((pair[0], pair[1]));
            }
        }
    }
    let fields = fields_of(segments, through);
    let runs: Vec<(Vec2, Vec2, usize)> = fields
        .iter()
        .enumerate()
        .flat_map(|(index, field)| field.rows.iter().map(move |(from, to)| (*from, *to, index)))
        .collect();

    let mut placed = Placed::default();
    for (index, field) in fields.iter().enumerate() {
        let (main, rows) = (field.main, &field.rows);
        let across = Vec2::new(-main.y, main.x);
        let (low, high) = axis_bounds(&area.outer, main, across);
        let lanes = lanes_of(rows, across, (low.y, high.y));
        if lanes.len() < 2 {
            continue;
        }
        // продольная сетка одна на всё поле: тогда места соседних рядов стоят
        // в одну линию, как на снимке, а не вразнобой на полместа. Считается
        // она **от контура, а не от проездов**: проезд в OSM обрывается, не
        // доходя до края площадки, и ряд, обрезанный по нему, оставлял бы вдоль
        // одной стороны полосу голого асфальта, а вдоль другой ничего
        // квартал размечается у своих проездов, а не во всю площадку: чужую
        // землю у него всё равно отберёт `Frame::territory`, и незачем её
        // перебирать
        let reach = rows
            .iter()
            .flat_map(|(from, to)| [main.dot(*from), main.dot(*to)])
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(least, most), at| {
                (least.min(at), most.max(at))
            });
        let span = if fields.len() > 1 {
            (
                low.x.max(reach.0 - BLOCK_REACH),
                high.x.min(reach.1 + BLOCK_REACH),
            )
        } else {
            (low.x, high.x)
        };
        let frame = Frame {
            area: outline,
            main,
            across,
            span,
            drives: cross_drives_of(rows, main, across),
            through,
            // у единственного поля спрашивать не у кого: вся площадка его
            territory: (fields.len() > 1).then_some((runs.as_slice(), index)),
        };
        for pair in lanes.windows(2) {
            let gap = pair[1] - pair[0];
            let centre = pair[0].midpoint(pair[1]);
            // глубину места задаёт карман: сперва проезд, остальное ряду
            let depth = ((gap - AISLE) / 2.0).clamp(STALL_DEPTH_MIN, STALL_DEPTH);
            if gap >= 2.0 * depth + PAIR_AISLE {
                frame.push_pair(&mut placed, centre, depth);
            } else if gap >= STALL_DEPTH + MIN_AISLE {
                // на пару не хватило — ряд по середине кармана, проезд по обе
                // стороны от него
                frame.push_row(&mut placed, centre, -1.0, STALL_DEPTH);
            }
        }
    }
    placed.stalls
}

/// Сколько разных полос обязано быть у **второго** поля проездов, чтобы оно
/// считалось полем. Отвороты к соседнему проезду у больничной стоянки лежат
/// цепочкой на одной линии — это одна «полоса», и рядов по ней быть не должно;
/// у ТРЦ «Макси» восточное поле — шесть параллельных проездов под 40° к
/// главным.
const FIELD_MIN_LANES: usize = 3;

/// Поле проездов: общее направление и ходы, что его держатся.
struct Field {
    main: Vec2,
    rows: Vec<(Vec2, Vec2)>,
}

/// Поля проездов площадки — по убыванию длины: главное и те, чьи проезды идут
/// под своим углом.
///
/// Большая стоянка бывает размечена не одной сеткой: у ТРЦ «Макси» сорок
/// четыре проезда идут под 102–105°, а шесть в восточном крыле — под 59–61°,
/// вдоль своей кромки. Одно направление на всю площадку расчерчивало это крыло
/// рядами главного поля — поперёк собственных проездов крыла, и на снимке там
/// штриховка под углом к тому, как стоят машины. Поле отбирается тем же
/// правилом, что и главное направление ([`main_of`] по ещё не разобранным
/// звеньям), а **чья земля** — решает ближайший ход ([`Frame::territory`]).
fn fields_of(mut segments: Vec<(Vec2, Vec2)>, through: &Surroundings) -> Vec<Field> {
    let spread = AISLE_SPREAD.to_radians().cos();
    let mut fields: Vec<Field> = Vec::new();
    while let Some(main) = main_of(&segments) {
        let rows = rows_of(&segments, main);
        if rows.is_empty() {
            break;
        }
        let across = Vec2::new(-main.y, main.x);
        if fields.is_empty() || distinct_lanes(&rows, across).len() >= FIELD_MIN_LANES {
            fields.extend(
                blocks_of(rows, main, across, through)
                    .into_iter()
                    .map(|rows| Field { main, rows }),
            );
        }
        segments.retain(|(from, to)| {
            (*to - *from)
                .try_normalize()
                .is_some_and(|step| step.dot(main).abs() < spread)
        });
    }
    fields
}

/// Сколько мест один ряд пары может выступать за другой, прежде чем выступ
/// считается отдельным рядом у кромки, а не обрезком ([`Frame::push_pair`]).
const PAIR_OVERHANG: usize = 2;

/// Соседние полосы одного квартала стоят не дальше этого поперёк, м: два шага
/// сетки с запасом — через карман, где вместо проезда идёт дорожка.
const BLOCK_LANE_REACH: f32 = 40.0;
/// Какой долей короткого хода соседи обязаны перекрываться вдоль, чтобы стоять
/// в одном квартале.
const BLOCK_OVERLAP_SHARE: f32 = 0.5;
/// Насколько соосны два куска одной полосы, разрезанной поперечным проездом, м.
const BLOCK_COLLINEAR: f32 = 1.0;
/// На сколько квартал размечается дальше концов своих проездов, м: проезд в
/// OSM обрывается, не доходя до кромки, а замощённая площадка ушла ещё и за
/// неё — до объездной.
const BLOCK_REACH: f32 = 30.0;

/// Ходы одного направления, разобранные по **кварталам**: у каждого квартала
/// своя сетка полос.
///
/// Сетка — одно число на полосу, смещение поперёк площадки, и на всё поле она
/// была одна. У ТРЦ «Макси» проезды к северу и к югу от бульвара — разные ways,
/// и друг против друга они не стоят: у кольца северная полоса приходится между
/// двумя южными. В общем списке такие полосы делили нормальный карман в 17 м
/// на 6 и 11, в одиннадцать вставал **один** ряд, в шесть — ничего, и посреди
/// площадки шли одиночные ряды с двойным проездом (отчёт автора).
///
/// В один квартал ходы сводятся двумя правилами. Соседи — стоят рядом поперёк
/// ([`BLOCK_LANE_REACH`]) и перекрываются вдоль хотя бы на
/// [`BLOCK_OVERLAP_SHARE`] короткого: бульвар идёт к проездам наискось, и
/// концы северного и южного хода заходят друг за друга на пару метров — это не
/// соседство. Куски одной полосы — соосны ([`BLOCK_COLLINEAR`]) и разделены
/// поперечным проездом, **а не дорогой сквозь площадку**: разрыв на бульваре
/// той же ширины, что и на поперечном проезде, и отличает их только она.
fn blocks_of(
    rows: Vec<(Vec2, Vec2)>,
    main: Vec2,
    across: Vec2,
    through: &Surroundings,
) -> Vec<Vec<(Vec2, Vec2)>> {
    // ход в осях поля: смещение поперёк и отрезок вдоль
    let frames: Vec<(f32, f32, f32)> = rows
        .iter()
        .map(|(from, to)| {
            let (start, end) = (main.dot(*from), main.dot(*to));
            (
                across.dot(*from).midpoint(across.dot(*to)),
                start.min(end),
                start.max(end),
            )
        })
        .collect();
    let mut block: Vec<usize> = (0..rows.len()).collect();
    fn root(block: &mut [usize], mut index: usize) -> usize {
        while block[index] != index {
            block[index] = block[block[index]];
            index = block[index];
        }
        index
    }
    for left in 0..rows.len() {
        for right in left + 1..rows.len() {
            let (a, b) = (frames[left], frames[right]);
            let lateral = (a.0 - b.0).abs();
            let overlap = a.2.min(b.2) - a.1.max(b.1);
            let shorter = (a.2 - a.1).min(b.2 - b.1);
            let neighbours =
                lateral <= BLOCK_LANE_REACH && overlap >= BLOCK_OVERLAP_SHARE * shorter;
            let pieces = lateral <= BLOCK_COLLINEAR && -overlap <= 2.0 * AISLE && {
                // разрыв — между ближними концами двух кусков
                let ends = [rows[left].0, rows[left].1];
                let others = [rows[right].0, rows[right].1];
                let (from, to) = ends
                    .iter()
                    .flat_map(|end| others.iter().map(move |other| (*end, *other)))
                    .min_by(|x, y| x.0.distance(x.1).total_cmp(&y.0.distance(y.1)))
                    .unwrap_or((ends[0], others[0]));
                !through.crossed(from, to)
            };
            if neighbours || pieces {
                let (left_root, right_root) = (root(&mut block, left), root(&mut block, right));
                block[right_root.max(left_root)] = right_root.min(left_root);
            }
        }
    }
    // порядок кварталов — по первому ходу, порядок ходов внутри — как пришли
    let mut blocks: Vec<(usize, Vec<(Vec2, Vec2)>)> = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let root = root(&mut block, index);
        match blocks.iter_mut().find(|(key, _)| *key == root) {
            Some((_, rows)) => rows.push(row),
            None => blocks.push((root, vec![row])),
        }
    }
    blocks.into_iter().map(|(_, rows)| rows).collect()
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
    let mut merged = distinct_lanes(rows, across);
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

/// Полосы самих проездов, по порядку поперёк площадки: ближе [`LANE_MERGE`]
/// друг к другу — одна полоса.
fn distinct_lanes(rows: &[(Vec2, Vec2)], across: Vec2) -> Vec<f32> {
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
    merged
}

/// Оси площадки и общая продольная сетка — всё, что нужно, чтобы поставить ряд
/// на заданной глубине.
struct Frame<'a> {
    area: &'a Outline<'a>,
    main: Vec2,
    across: Vec2,
    /// Отрезок вдоль площадки, на котором стоят ряды: от него же отсчитывается
    /// продольная сетка, поэтому начало сетки и начало отрезка — одно число.
    span: (f32, f32),
    /// Поперечные проезды, прочитанные из OSM ([`cross_drives_of`]): под
    /// каждым ряд рвётся, оставляя полосу [`AISLE`].
    drives: Vec<f32>,
    /// Дороги сквозь площадку: под ними и их бордюром мест нет.
    through: &'a Surroundings,
    /// Ходы проездов **всех** полей площадки с номером поля и номер своего:
    /// место встаёт, только если ближайший к нему ход — своего поля. Сетка
    /// полос продолжается до краёв контура, то есть и на землю соседнего поля,
    /// и без этого два поля расчерчивали бы друг друга.
    territory: Option<(&'a [(Vec2, Vec2, usize)], usize)>,
}

impl Frame<'_> {
    /// Своя ли это земля — см. [`Frame::territory`].
    fn owns(&self, at: Vec2) -> bool {
        let Some((runs, own)) = self.territory else {
            return true;
        };
        runs.iter()
            .map(|(from, to, field)| (distance_to_segment(at, *from, *to), *field))
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .is_none_or(|(_, field)| field == own)
    }

    /// Ряд, середина которого стоит на `band` поперёк площадки, носом в `nose`
    /// (±1 вдоль `across`), местами глубиной `depth`, во всю длину площадки
    /// ([`Frame::span`]).
    fn push_row(&self, placed: &mut Placed, band: f32, nose: f32, depth: f32) {
        self.lay(placed, self.cells(band, nose, depth, band), nose);
    }

    /// Пара рядов спинами друг к другу — **прямоугольником**: место остаётся,
    /// только если за его спиной стоит место второго ряда.
    ///
    /// Ряды пары проверяются порознь, и в косом квартале (между двумя
    /// дорогами, идущими к проездам под углом) каждый обрезало по-своему:
    /// слева четыре места, справа пять и со сдвигом. Читалось это не как пара,
    /// а как ряд и огрызок второго рядом с ним (отчёт автора). Если второго
    /// ряда нет вовсе — у кромки, — первый остаётся как есть: одиночный ряд с
    /// краю законен.
    fn push_pair(&self, placed: &mut Placed, centre: f32, depth: f32) {
        let mut rows = [
            self.cells(centre - depth / 2.0, -1.0, depth, centre),
            self.cells(centre + depth / 2.0, 1.0, depth, centre),
        ];
        let paired: Vec<bool> = rows[0]
            .iter()
            .zip(&rows[1])
            .map(|(left, right)| left.is_some() && right.is_some())
            .collect();
        if paired.iter().any(|paired| *paired) {
            for row in &mut rows {
                // выступ — места подряд без соседа за спиной. Короткий
                // срезается; длинный — ряд вдоль косой кромки, где второму
                // встать негде, и он остаётся
                let mut start = 0;
                while start < row.len() {
                    if row[start].is_none() || paired[start] {
                        start += 1;
                        continue;
                    }
                    let mut end = start;
                    while end < row.len() && row[end].is_some() && !paired[end] {
                        end += 1;
                    }
                    if end - start <= PAIR_OVERHANG {
                        row[start..end].fill(None);
                    }
                    start = end;
                }
            }
        }
        let [left, right] = rows;
        self.lay(placed, left, -1.0);
        self.lay(placed, right, 1.0);
    }

    /// Уложить ряд из клеток сетки: обрывок короче [`MIN_ROW_RUN`] не
    /// размечается вовсе — у скошенной кромки в ряду остаётся одно место, и на
    /// картинке это полоска на пустом асфальте, куда никто не встанет.
    fn lay(&self, placed: &mut Placed, cells: Vec<Option<Stall>>, nose: f32) {
        let mut row: Vec<Stall> = Vec::new();
        let mut run: Vec<Stall> = Vec::new();
        for cell in cells {
            match cell {
                Some(stall) => run.push(stall),
                None => {
                    if run.len() >= MIN_ROW_RUN {
                        row.append(&mut run);
                    }
                    run.clear();
                }
            }
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

    /// Клетки продольной сетки ряда: место там, где оно прошло все проверки.
    ///
    /// `anchor` — полоса, по которой решается, **чья это земля**
    /// ([`Frame::territory`]): у одиночного ряда своя, у пары — середина между
    /// рядами. По месту её решали сперва, и на стыке двух кварталов левый ряд
    /// пары доставался одному, правый другому — три места против пяти, каждый
    /// от своей сетки (отчёт автора: «ряд и рядом неполный второй»).
    fn cells(&self, band: f32, nose: f32, depth: f32, anchor: f32) -> Vec<Option<Stall>> {
        let along = self.across * nose;
        let mut cells = Vec::new();
        let mut index = 0.0;
        loop {
            let place = self.span.0 + index * STALL_WIDTH + STALL_WIDTH / 2.0;
            if place + STALL_WIDTH / 2.0 > self.span.1 + SPAN_SLACK {
                break;
            }
            let at = self.main * place + self.across * band;
            // `(AISLE + STALL_WIDTH) / 2` — не новая константа, а ровно «между
            // кромками мест по обе стороны коридора остаётся [`AISLE`]»
            let clear = !self
                .drives
                .iter()
                .any(|drive| (place - drive).abs() < (AISLE + STALL_WIDTH) / 2.0);
            // `owns` перебирает все ходы площадки — спрашивается после контура
            let stands = clear
                && fits_with(self.area, at, self.main, self.across, depth, EDGE_MARGIN)
                && self.owns(self.main * place + self.across * anchor)
                && !self.through.cover(at, along, depth)
                && reachable(self.area, self.through, at, along, depth);
            cells.push(stands.then_some(Stall {
                at,
                along,
                depth,
                parallel: false,
            }));
            index += 1.0;
        }
        cells
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

/// Поперечные проезды, прочитанные из OSM, — как смещения вдоль площадки.
///
/// Рисуют их **разрывом, а не линией**: длинный ход проезда разрезан на два
/// соосных куска, и между ними оставлена полоса в проезд, по которой машина
/// переезжает из квартала в квартал. У ТРЦ «Макси» так нарисован один
/// поперечный проезд — он виден сразу на 21 полосе из 37, шириной 5.1–6.0 м
/// (кэш 7600 × 5700, v14); во всём городе разрывов между соосными ходами 22,
/// и двадцать второй — 2.8 м, то есть уже не проезд.
///
/// Разрыв на одной полосе — это обрыв рисования, а не проезд, поэтому
/// засчитывается только тот, что повторяется хотя бы на второй полосе в
/// пределах [`CROSS_DRIVE_SLACK`] (у «Макси» 21 середина уложилась в 1.5 м).
fn cross_drives_of(rows: &[(Vec2, Vec2)], main: Vec2, across: Vec2) -> Vec<f32> {
    // ходы по полосам — тем же правилом, что у `lanes_of`, и **до**
    // продолжения сетки: коридор читается по самим проездам
    let mut runs: Vec<(f32, f32, f32)> = rows
        .iter()
        .map(|(from, to)| {
            let lane = across.dot(*from).midpoint(across.dot(*to));
            let (start, end) = (main.dot(*from), main.dot(*to));
            (lane, start.min(end), start.max(end))
        })
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));

    // середины разрывов и номер полосы, на которой каждый нашёлся
    let mut gaps: Vec<(f32, usize)> = Vec::new();
    let mut spans: Vec<(f32, f32)> = Vec::new();
    let mut lane_index = 0;
    let mut anchor = f32::NEG_INFINITY;
    for (lane, start, end) in runs {
        if lane - anchor >= LANE_MERGE {
            push_lane_gaps(&mut gaps, &mut spans, lane_index);
            lane_index += 1;
            anchor = lane;
        }
        spans.push((start, end));
    }
    push_lane_gaps(&mut gaps, &mut spans, lane_index);

    gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut drives = Vec::new();
    let mut cluster: Vec<(f32, usize)> = Vec::new();
    for gap in gaps {
        if cluster
            .last()
            .is_some_and(|last| gap.0 - last.0 >= CROSS_DRIVE_SLACK)
        {
            push_cross_drive(&mut drives, &cluster);
            cluster.clear();
        }
        cluster.push(gap);
    }
    push_cross_drive(&mut drives, &cluster);
    drives
}

/// Разрывы одной полосы: перекрывающиеся ходы сливаются, и зазор между
/// соседними засчитывается, только если в него пролезет машина ([`MIN_AISLE`])
/// и он не шире двух проездов — тогда это уже не проезд между кварталами, а
/// дыра в разметке OSM.
fn push_lane_gaps(gaps: &mut Vec<(f32, usize)>, spans: &mut Vec<(f32, f32)>, lane: usize) {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for span in spans.drain(..) {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }
    for pair in merged.windows(2) {
        let width = pair[1].0 - pair[0].1;
        if (MIN_AISLE..=2.0 * AISLE).contains(&width) {
            gaps.push((pair[0].1.midpoint(pair[1].0), lane));
        }
    }
}

/// Коридор из склеенных разрывов: его смещение — среднее их середин, и берётся
/// он, только если разрыв повторился хотя бы на второй полосе.
fn push_cross_drive(drives: &mut Vec<f32>, cluster: &[(f32, usize)]) {
    let mut lanes: Vec<usize> = cluster.iter().map(|(_, lane)| *lane).collect();
    lanes.sort_unstable();
    lanes.dedup();
    if lanes.len() < 2 {
        return;
    }
    let sum: f32 = cluster.iter().map(|(at, _)| *at).sum();
    drives.push(sum / cluster.len() as f32);
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
