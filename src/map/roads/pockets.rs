//! Стоянка вдоль улицы: где машины встают у бордюра на проезжей части, где —
//! в **карман** (асфальт за кромкой, врезанный в тротуар), а где их нет.
//!
//! Один ответ на двух читателей: лента асфальта (`map::roads`) кладёт по нему
//! карман, ряд машин (`map::cars`) — ставит в него машины. Пока ответов было
//! бы два, машина вставала бы мимо кармана на тротуар.
//!
//! Тег — `parking:<side>` ([`KerbParking`]); без тега — правило
//! ([`kerb_parking`]): на магистрали (trunk, primary, secondary) на полосе не
//! стоят, и машины уходят в карманы, на остальных улицах стоят у бордюра.
//! Карман по тегу идёт вдоль всей стороны way между перекрёстками, по правилу
//! — редкий и короткий ([`sparse_pockets`]).

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::is_carriageway;
use super::junctions::{self, MarkingBreaks, node_key};
use super::network::RoadNodes;
use super::node_paint::ZEBRA_LENGTH;
use super::tapers::{self, Tapers};
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::{Break, miter_offsets};
use crate::map::osm::model::{
    Highway, KerbParking, distance_to_area, polyline_length, ring_bounds,
};
use crate::map::osm::{PolyArea, RoadLine, RoadNode, RoadNodeKind, TrafficSide};
use crate::map::seed::{Lcg, seed_from_point};

/// Ширина кармана за кромкой проезжей части, м: машина при параллельной
/// стоянке и полметра до бордюра.
pub const POCKET_WIDTH: f32 = 2.5;
/// Длина скоса на торце кармана, м.
pub const POCKET_TAPER: f32 = 6.0;
/// Отступ кармана от перекрёстка сверх его разрыва, м: у перехода и угла
/// кармана не делают. Шесть — скос начинается за зеброй по правилу, что
/// стоит от кромки узла на метр и тянется на четыре (`roads::node_paint`).
const POCKET_CLEARANCE: f32 = 6.0;
/// Самый короткий карман по полной ширине, м — две машины. Короче — не
/// карман, а зазубрина.
const POCKET_MIN: f32 = 10.0;
/// Доля кварталов (кусков стороны между перекрёстками), где правило ставит
/// карманы: в городе карман — у магазина, у остановки, у подъезда, а не вдоль
/// каждого квартала.
const RULE_BLOCK_SHARE: f32 = 0.4;
/// Длина кармана по правилу со скосами, м: три-пять машин.
const RULE_POCKET_LENGTH: (f32, f32) = (24.0, 42.0);
/// Просвет тротуара между карманами по правилу, м.
const RULE_POCKET_GAP: (f32, f32) = (30.0, 90.0);
/// Досягаемость стоянки от наружного края кармана, м: тротуар (до трёх
/// метров) и газон за ним. Стоянка ближе — машинам есть где встать и без
/// кармана, и асфальт кармана перед ней читался лишней полосой (Тула,
/// Советская у театра, 6040, 2900: стоянка в 4 м за краем кармана).
const LOT_REACH: f32 = 6.0;
/// Шаг, с которым край кармана проверяется на соседство со стоянкой, м.
const LOT_STEP: f32 = 2.0;
/// Ячейка индекса стоянок, м.
const LOT_CELL: f32 = 60.0;

/// Стоянки карты (`amenity=parking`), у которых карман не нужен: машины
/// встают на стоянку, а не у бордюра перед ней.
pub struct KerbLots<'a> {
    areas: &'a [PolyArea],
    grid: Grid<usize>,
}

impl<'a> KerbLots<'a> {
    pub fn new(areas: &'a [PolyArea]) -> Self {
        let mut grid = Grid::new(LOT_CELL);
        for (index, area) in areas.iter().enumerate() {
            let (min, max) = ring_bounds(&area.outer);
            if min.cmple(max).all() {
                grid.insert(min - LOT_REACH, max + LOT_REACH, index);
            }
        }
        Self { areas, grid }
    }

    /// Стоянка внутри или ближе [`LOT_REACH`] к точке.
    fn near(&self, point: Vec2) -> bool {
        self.grid
            .at(point)
            .iter()
            .any(|&index| distance_to_area(point, &self.areas[index]) <= LOT_REACH)
    }

    /// Куски осевой `path`, у которых за наружным краем кармана — на `edge`
    /// от оси со стороны `side` — стоянка: `(from, to)` по длине осевой, по
    /// возрастанию.
    fn frontage(&self, path: &[Vec2], side: f32, edge: f32) -> Vec<(f32, f32)> {
        let (along, total) = arclengths(path);
        let mut found: Vec<(f32, f32)> = Vec::new();
        let steps = (total / LOT_STEP).ceil() as usize;
        for step in 0..=steps {
            let at = (step as f32 * LOT_STEP).min(total);
            let Some((point, direction)) = place_on_path(path, &along, at) else {
                break;
            };
            if !self.near(point + direction.perp() * side * edge) {
                continue;
            }
            match found.last_mut() {
                Some(run) if at - run.1 <= LOT_STEP + 1e-3 => run.1 = at,
                _ => found.push((at, at)),
            }
        }
        found
    }
}

/// Одна сторона улицы, вдоль которой может стоять ряд.
#[derive(Clone, Debug)]
pub struct Kerbside {
    /// Знак поперечной `direction.perp()`: `-1` — правая сторона по ходу
    /// точек, `1` — левая.
    pub side: f32,
    /// Стоят ли машины у бордюра на самой проезжей части.
    pub lane: bool,
    /// Карманы вдоль стороны — по длине осевой.
    pub pockets: Vec<Pocket>,
}

/// Карман: `from..to` по длине осевой, со скосами на торцах, кроме торца
/// way, за которым карман той же стороны продолжается на следующем way
/// ([`join_way_ends`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pocket {
    pub from: f32,
    pub to: f32,
    pub tapers: [bool; 2],
}

impl Pocket {
    /// Часть кармана полной ширины — где в нём помещается машина.
    pub fn full(&self) -> (f32, f32) {
        let taper = |on: bool| if on { POCKET_TAPER } else { 0.0 };
        (
            self.from + taper(self.tapers[0]),
            self.to - taper(self.tapers[1]),
        )
    }
}

impl Kerbside {
    /// Карман, в полную ширину которого попадает длина `at`.
    pub fn pocket_at(&self, at: f32) -> Option<&Pocket> {
        self.pockets.iter().find(|pocket| {
            let (from, to) = pocket.full();
            (from..=to).contains(&at)
        })
    }
}

/// Улица, вдоль которой паркуются: настоящая проезжая часть — то же
/// [`is_carriageway`], которым отбирает тротуары и разметку `map::roads`, —
/// но не мост (на мосту не стоят) и не кольцо (по кольцу едут, а не
/// паркуются). Разметке мост и кольцо нужны, машинам нет, поэтому оба
/// условия здесь, а не внутри предиката.
pub fn parkable(road: &RoadLine) -> bool {
    is_carriageway(road) && !road.bridge && !road.is_roundabout()
}

/// Стоянка стороны `side` (`0` — слева по ходу точек, `1` — справа): тег или
/// правило. Правило — по классу: на магистрали машина на полосе мешает
/// движению, и её место в кармане, если есть тротуар, в который его врезать;
/// на автомагистрали и съездах развязок не стоят вовсе.
pub fn kerb_parking(road: &RoadLine, side: usize) -> KerbParking {
    match road.parking[side] {
        KerbParking::Untagged => match road.highway {
            Highway::Motorway => KerbParking::No,
            highway if highway.is_link() => KerbParking::No,
            Highway::Trunk | Highway::Primary | Highway::Secondary => {
                if road.sidewalk().sides()[side] {
                    KerbParking::Pocket
                } else {
                    KerbParking::No
                }
            }
            _ => KerbParking::Lane,
        },
        tagged => tagged,
    }
}

/// Стороны всех улиц карты — по индексу дороги, у непарковочной пусто:
/// [`kerbsides`] каждой, потом торцы карманов сшиты через узлы way
/// ([`join_way_ends`]). Один вызов на ленту и на ряд машин: `shared` — узлы
/// тех же `roads` (соседство way'ев, [`RoadNodes::next_way`]), `paths` — их
/// нарисованные осевые, `breaks` — [`row_breaks`], `lots` — стоянки, перед
/// которыми кармана нет.
pub fn all_kerbsides<P: AsRef<[Vec2]>>(
    roads: &[RoadLine],
    shared: &RoadNodes,
    paths: &[P],
    breaks: &RowBreaks,
    traffic: TrafficSide,
    lots: &KerbLots,
) -> Vec<Vec<Kerbside>> {
    let mut sides: Vec<Vec<Kerbside>> = roads
        .iter()
        .enumerate()
        .map(|(index, road)| {
            if parkable(road) {
                kerbsides(road, paths[index].as_ref(), breaks.of(index), traffic, lots)
            } else {
                Vec::new()
            }
        })
        .collect();
    join_way_ends(shared, &mut sides);
    sides
}

/// Карман через торцы way — один карман. [`pockets_along`] оставляет торец
/// кармана у торца way открытым в расчёте на соседа, и здесь это
/// проверяется: торец, за которым у соседа нет открытого кармана той же
/// стороны (запрет остановки, мост, конец улицы), получает скос, а
/// [`POCKET_MIN`] меряется по всей цепочке кусков, сшитых открытыми
/// торцами, а не по куску на каждом way. Иначе карман обрывался ступенькой:
/// OSM режет улицу на короткие way у переходов и проездов, и кусок на way
/// в 23 м у проезда во двор отбрасывался как короткий, а соседний кусок
/// оставался открытым в пустоту (ул. Дзержинского в Туле, 5994, 3185).
///
/// Кусок, в котором скос длиннее его самого, уходит — скос через торец way
/// не переносится; уход куска закрывает торец соседа, поэтому проход
/// повторяется, пока что-то меняется: скосы только добавляются, куски
/// только убывают, так что он кончается.
///
/// Сосед — way, продолжающий улицу за торцом ([`RoadNodes::next_way`]), а не
/// любой way узла: на Т-узле карман прямой улицы не сворачивает в
/// примыкающую. В городе такой узел рвёт карманы и сам — три участника ряда
/// дают разрыв перекрёстка ([`row_breaks`]), — так что выбор соседа решает
/// на шве двух way одной улицы или через границу улиц.
fn join_way_ends(shared: &RoadNodes, sides: &mut [Vec<Kerbside>]) {
    // кусок у соседа, открытый на торце `end` дороги `road` со стороны `side`:
    // (дорога, сторона в её списке, карман)
    let partner = |sides: &[Vec<Kerbside>], road: usize, end: usize, side: f32| {
        let (next, reversed) = shared.next_way(road, end)?;
        // way, сходящиеся торцами разного конца, идут в одну сторону, и
        // сторона та же; одноимёнными — навстречу, и она меняется
        let (next_end, next_side) = if reversed {
            (end, -side)
        } else {
            (1 - end, side)
        };
        sides[next]
            .iter()
            .enumerate()
            .filter(|(_, kerbside)| kerbside.side == next_side)
            .find_map(|(at, kerbside)| {
                let index = kerbside
                    .pockets
                    .iter()
                    .position(|pocket| !pocket.tapers[next_end])?;
                Some((next, at, index))
            })
    };
    loop {
        let mut closing = Vec::new();
        let mut links = Vec::new();
        for (road, kerbsides) in sides.iter().enumerate() {
            for (at, kerbside) in kerbsides.iter().enumerate() {
                for (index, pocket) in kerbside.pockets.iter().enumerate() {
                    for end in 0..2 {
                        if pocket.tapers[end] {
                            continue;
                        }
                        match partner(sides, road, end, kerbside.side) {
                            Some(next) => links.push(((road, at, index), next)),
                            None => closing.push((road, at, index, end)),
                        }
                    }
                }
            }
        }
        for &(road, at, index, end) in &closing {
            sides[road][at].pockets[index].tapers[end] = true;
        }
        // цепочки кусков по открытым торцам и полная длина каждой
        let mut chain: HashMap<(usize, usize, usize), (usize, usize, usize)> = HashMap::new();
        fn root(
            chain: &HashMap<(usize, usize, usize), (usize, usize, usize)>,
            mut id: (usize, usize, usize),
        ) -> (usize, usize, usize) {
            while let Some(&up) = chain.get(&id) {
                id = up;
            }
            id
        }
        for (a, b) in links {
            let (a, b) = (root(&chain, a), root(&chain, b));
            if a != b {
                chain.insert(a, b);
            }
        }
        let mut full: HashMap<(usize, usize, usize), f32> = HashMap::new();
        let mut short_piece = false;
        for (road, kerbsides) in sides.iter().enumerate() {
            for (at, kerbside) in kerbsides.iter().enumerate() {
                for (index, pocket) in kerbside.pockets.iter().enumerate() {
                    let (from, to) = pocket.full();
                    short_piece |= to < from;
                    *full.entry(root(&chain, (road, at, index))).or_default() += to - from;
                }
            }
        }
        let dropped = short_piece || full.values().any(|&length| length < POCKET_MIN);
        if dropped {
            for (road, kerbsides) in sides.iter_mut().enumerate() {
                for (at, kerbside) in kerbsides.iter_mut().enumerate() {
                    let mut index = 0;
                    kerbside.pockets.retain(|pocket| {
                        let (from, to) = pocket.full();
                        let keep =
                            to >= from && full[&root(&chain, (road, at, index))] >= POCKET_MIN;
                        index += 1;
                        keep
                    });
                }
            }
        }
        if closing.is_empty() && !dropped {
            break;
        }
    }
}

/// Стороны улицы, вдоль которых стоит ряд, в том порядке, в каком их
/// обходит `map::cars` (от него зависит поток ГПСЧ): односторонняя — одна,
/// у бордюра своей стороны движения; двусторонняя — правая, потом левая.
/// `path` — нарисованная осевая, `breaks` — разрывы ряда на перекрёстках
/// (`map::cars` зовёт их `junctions`), `lots` — стоянки, перед которыми
/// кармана нет: по правилу карман рядом с ними не ставится, по тегу —
/// прерывается на их длину. Торцы карманов у торцов way здесь ещё открыты —
/// сшивает их [`all_kerbsides`].
fn kerbsides(
    road: &RoadLine,
    path: &[Vec2],
    breaks: &[Break],
    traffic: TrafficSide,
    lots: &KerbLots,
) -> Vec<Kerbside> {
    let sides: &[f32] = if road.oneway {
        &[traffic.kerb()]
    } else {
        &[-1.0, 1.0]
    };
    sides
        .iter()
        .map(|&side| {
            let index = usize::from(side < 0.0);
            let parking = kerb_parking(road, index);
            let pockets = if parking != KerbParking::Pocket {
                Vec::new()
            } else {
                let frontage = lots.frontage(path, side, road.width / 2.0 + POCKET_WIDTH);
                if road.parking[index] == KerbParking::Untagged {
                    // бухты раскладываются как без стоянок и потом снимаются:
                    // поток ГПСЧ не сдвигается, и остальные бухты квартала
                    // стоят там же, где стояли. Куски короче кармана — только
                    // для тега: правило тянет ГПСЧ по куску, и лишний кусок
                    // сдвинул бы его бухты
                    let runs = pockets_along(path, breaks, &[])
                        .into_iter()
                        .filter(|run| {
                            let (from, to) = run.full();
                            to - from >= POCKET_MIN
                        })
                        .collect();
                    let mut pockets = sparse_pockets(runs, road, index);
                    pockets.retain(|pocket| {
                        frontage
                            .iter()
                            .all(|&(from, to)| to < pocket.from || from > pocket.to)
                    });
                    pockets
                } else {
                    pockets_along(path, breaks, &frontage)
                }
            };
            Kerbside {
                side,
                lane: parking == KerbParking::Lane,
                pockets,
            }
        })
        .collect()
}

/// Карманы по правилу — редкие и короткие: на [`RULE_BLOCK_SHARE`] кварталов,
/// по [`RULE_POCKET_LENGTH`] через [`RULE_POCKET_GAP`] внутри куска `runs`,
/// который оставили перекрёстки. Тег `parking=street_side` говорит о всей
/// стороне way, а правило ничего не знает и не выдумывает больше, чем бывает
/// на самом деле: карман во весь квартал без машин читался как лишняя полоса.
///
/// Посев — от первой точки улицы и стороны ([`seed_from_point`]): лента и ряд
/// машин спрашивают одно и то же и получают одно и то же, и пересборка ничего
/// не сдвигает.
fn sparse_pockets(runs: Vec<Pocket>, road: &RoadLine, side: usize) -> Vec<Pocket> {
    let Some(&start) = road.points.first() else {
        return Vec::new();
    };
    let side_salt = if side == 0 { 0 } else { 0x9e37_79b9 };
    let mut rng = Lcg::new(seed_from_point(start) ^ side_salt);
    let mut pockets = Vec::new();
    for run in runs {
        if rng.next_f32() >= RULE_BLOCK_SHARE {
            continue;
        }
        let mut cursor = run.from + rng.range(0.0, RULE_POCKET_GAP.0);
        loop {
            let length = rng.range(RULE_POCKET_LENGTH.0, RULE_POCKET_LENGTH.1);
            if cursor + length > run.to {
                break;
            }
            pockets.push(Pocket {
                from: cursor,
                to: cursor + length,
                tapers: [true, true],
            });
            cursor += length + rng.range(RULE_POCKET_GAP.0, RULE_POCKET_GAP.1);
        }
    }
    pockets
}

/// Разрывы ряда у бордюра по дорогам: перекрёстки (без стежков — ряд их не
/// видит), переходы и клинья между сечениями, где бордюр ближе к оси. Один
/// расчёт на ряд машин и на ленту с карманами; `tapers` — клинья карты (у
/// ленты — те, что у `roads::Drawn`, у машин — [`Tapers::of_map`]), `shared` —
/// узлы тех же `roads` (продолжение улицы за торцом way), `nodes` — точки
/// дорог карты, из которых берутся переходы.
///
/// В перекрёстках участвуют и проезды, не только улицы разметки: во двор, к
/// стоянке съезжают через ряд, и машина на съезде его перегораживала (Тула,
/// Ф. Энгельса у 3976, 1236).
pub fn row_breaks(
    roads: &[RoadLine],
    shared: &RoadNodes,
    tapers: &Tapers,
    nodes: &[RoadNode],
) -> RowBreaks {
    row_breaks_over(
        junctions::marking_breaks(roads, is_row_participant, &[]),
        roads,
        shared,
        tapers,
        nodes,
    )
}

/// [`row_breaks`] по уже найденным разрывам перекрёстков `found` — узлам
/// участников [`is_row_participant`] без стежков: у слоя дорог узлы
/// обходятся один раз на всё (`junctions::Junctions`).
pub(super) fn row_breaks_over(
    found: MarkingBreaks,
    roads: &[RoadLine],
    shared: &RoadNodes,
    tapers: &Tapers,
    nodes: &[RoadNode],
) -> RowBreaks {
    let MarkingBreaks {
        mut breaks,
        junctions,
    } = found;
    for (road, clearing) in tapers::car_clearings(roads, tapers) {
        breaks[road].push(clearing);
    }
    for (road, crossing) in crossing_breaks(roads, shared, nodes) {
        breaks[road].push(crossing);
    }
    RowBreaks { breaks, junctions }
}

/// Разрывы **ряда** у бордюра по дорогам — карманы ленты ([`all_kerbsides`])
/// и ряд машин (`map::cars`). Выходят только из [`row_breaks`]: перекрёстки
/// участников ряда ([`is_row_participant`]) без стежков, клинья между
/// сечениями, переходы OSM. Свой тип, а не `MarkingBreaks`, чтобы карман и
/// ряд не встали по базе или по разрывам краски: у тех участники — проезжие
/// части, стежки — узлы, а переходов и клиньев нет.
pub struct RowBreaks {
    breaks: Vec<Vec<Break>>,
    /// Сколько узлов оказались перекрёстками (строка `cars:`).
    pub junctions: usize,
}

impl RowBreaks {
    pub fn of(&self, road: usize) -> &[Break] {
        &self.breaks[road]
    }

    /// Разрывы теста как есть — без клиньев и переходов.
    #[cfg(test)]
    pub(super) fn for_test(breaks: Vec<Vec<Break>>) -> Self {
        Self {
            breaks,
            junctions: 0,
        }
    }
}

/// Дорога, что рвёт ряд у бордюра, встретившись с улицей: проезжая часть или
/// проезд — всё, по чему ездят, кроме дорожек. Проезжие части
/// (`roads::is_carriageway`) — все среди них: на этом `junctions::Junctions`
/// строит узлы разметки из тех же узлов.
pub(super) fn is_row_participant(road: &RoadLine) -> bool {
    road.highway != Highway::Path
}

/// До торца way ближе этого, м, — переход рвёт ряд и на продолжении улицы за
/// торцом: OSM режет улицу у перехода, и зебра у самого торца короткого way
/// (Первомайская в Туле, 3279, 2799) без этого стояла вплотную к карману
/// соседнего, а тот обрывался без скоса.
const CROSSING_SPILL: f32 = 10.0;

/// Разрывы на размеченных переходах OSM: полдлины зебры вокруг узла на его
/// дороге, а если до торца её way ближе [`CROSSING_SPILL`] — и на проезжей
/// части, что продолжает улицу за этим торцом ([`RoadNodes::next_way`]), от
/// торца на остаток. Примыкающий way Т-узла не продолжение: у него в узле
/// свой разрыв перекрёстка, шире полузебры.
fn crossing_breaks(
    roads: &[RoadLine],
    shared: &RoadNodes,
    nodes: &[RoadNode],
) -> Vec<(usize, Break)> {
    let reach = ZEBRA_LENGTH / 2.0;
    let crossings: HashMap<(i32, i32), Vec2> = nodes
        .iter()
        .filter(|node| matches!(node.kind, RoadNodeKind::Crossing { marked: true, .. }))
        .map(|node| (node_key(node.pos), node.pos))
        .collect();
    if crossings.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !is_carriageway(road) {
            continue;
        }
        for &point in &road.points {
            let Some(&at) = crossings.get(&node_key(point)) else {
                continue;
            };
            found.push((index, Break { at, reach }));
            let (along, total) = arclengths(&road.points);
            let Some(station) = road
                .points
                .iter()
                .position(|&vertex| vertex == point)
                .map(|vertex| along[vertex])
            else {
                continue;
            };
            for (end, (tip, left)) in [
                (road.points[0], station),
                (road.points[road.points.len() - 1], total - station),
            ]
            .into_iter()
            .enumerate()
            {
                if left <= 0.0 || left >= CROSSING_SPILL {
                    continue;
                }
                let Some((next, _)) = shared.next_way(index, end) else {
                    continue;
                };
                if is_carriageway(&roads[next]) {
                    found.push((
                        next,
                        // клиренс ряда отмеряется от разрыва, и торцу хватает
                        // остатка полузебры за ним
                        Break {
                            at: tip,
                            reach: (reach - left).max(0.0),
                        },
                    ));
                }
            }
        }
    }
    found
}

/// Карманы вдоль осевой: вся её длина, кроме окрестностей перекрёстков и
/// кусков `frontage` перед стоянками (по длине осевой; скосы — за ними).
fn pockets_along(path: &[Vec2], breaks: &[Break], frontage: &[(f32, f32)]) -> Vec<Pocket> {
    let total = polyline_length(path);
    let mut closed: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = nearest_on_path(path, found.at).map_or(0.0, |(_, along)| along);
            let reach = found.reach + POCKET_CLEARANCE;
            (at - reach, at + reach)
        })
        .chain(frontage.iter().copied())
        .collect();
    closed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut pockets = Vec::new();
    let mut cursor = 0.0;
    let mut push = |from: f32, to: f32| {
        let pocket = Pocket {
            from,
            to,
            tapers: [from > 0.0, to < total],
        };
        // кусок, открытый в торец way, меряет цепочка ([`join_way_ends`])
        let (start, end) = pocket.full();
        let open = pocket.tapers != [true, true];
        if end - start >= POCKET_MIN || open && end > start {
            pockets.push(pocket);
        }
    };
    for (from, to) in closed {
        if from > cursor {
            push(cursor, from.min(total));
        }
        cursor = cursor.max(to);
    }
    if cursor < total {
        push(cursor, total);
    }
    pockets
}

/// Контур кармана со стороны `side`: внутренний край на `inner` от оси по
/// всей длине, наружный на `outer` — между скосами. Годится и для асфальта,
/// и для тротуара, отодвинутого за карман.
pub fn outline(path: &[Vec2], pocket: &Pocket, side: f32, [inner, outer]: [f32; 2]) -> Vec<Vec2> {
    let (full_from, full_to) = pocket.full();
    let edge = |from: f32, to: f32, shift: f32| -> Vec<Vec2> {
        let piece = tapers::cut(path, from, to);
        let offsets = miter_offsets(&piece, false, side * shift);
        piece
            .iter()
            .zip(offsets)
            .map(|(point, offset)| *point + offset)
            .collect()
    };
    let mut contour = edge(pocket.from, pocket.to, inner);
    contour.extend(edge(full_from, full_to, outer).into_iter().rev());
    contour
}

#[cfg(test)]
mod tests;
